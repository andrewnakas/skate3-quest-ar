//! Roomtone's light in Bevy's terms. Pure maths, no ECS.
//!
//! Roomtone answers in relative units: irradiance over pi, scaled so an
//! up-facing surface where the head stood gets 1, with a `level` for how
//! bright passthrough shows that room. Bevy's lights are photometric (lux,
//! lumens, cd/m^2) and its cameras multiply by an exposure, so everything
//! here goes through one [`Scale`]: the scene-referred value that makes a
//! white surface come out of the camera at the room's brightness.
use bevy::math::{Quat, UVec3, Vec3};
use roomtone_sys::{Light, Lighting};
use std::f32::consts::PI;

/// A white surface lit by 1 (Roomtone's units, at level 1) is shown this
/// bright (0-1, display): Roomtone's own reference, tuned on a Quest 3
/// against the passthrough image beside it.
const REFERENCE_WHITE: f32 = 0.7;
/// Display to linear.
const GAMMA: f32 = 2.2;

/// Roomtone's units to Bevy's, for one made light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scale {
    /// cd/m^2 per unit of irradiance over pi.
    pub luminance: f32,
}

impl Scale {
    /// `exposure` is the camera's (`Exposure::exposure()`: scene luminance
    /// times it is the linear colour that reaches tonemapping); `strength`
    /// the player's own trim (1: as tuned).
    pub(crate) fn new(light: &Lighting, exposure: f32, strength: f32) -> Self {
        let white = (REFERENCE_WHITE * light.level).powf(GAMMA) / light.mean.max(1e-3);
        Self { luminance: white * strength / exposure.max(1e-9) }
    }
}

/// The soft light at one point as an ambient cube: the light on surfaces
/// facing +X, -X, +Y, -Y, +Z and -Z of the *world* (Bevy's irradiance
/// volumes look the cube up by the world normal), in cd/m^2.
/// `tracking_from_world` turns a world direction into Roomtone's space.
pub(crate) fn ambient_cube(light: &Lighting, tracking_from_world: Quat, scale: Scale) -> [Vec3; 6] {
    [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z].map(|axis| {
        let n = tracking_from_world * axis;
        Vec3::from_array(light.irradiance(n.to_array())) * scale.luminance
    })
}

/// A key light for Bevy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Key {
    /// Unlit, or too dim to matter.
    None,
    /// No place in the room: a direction only.
    Directional {
        /// Linear RGB, its largest channel 1.
        color: Vec3,
        /// Lux on a surface facing the light.
        illuminance: f32,
        /// Toward the light, world space.
        toward: Vec3,
    },
    /// Depth gave it a place: a point light, so what is nearer it is lit more.
    Point {
        color: Vec3,
        /// Lumens (Bevy's `PointLight::intensity`).
        intensity: f32,
        /// World space.
        position: Vec3,
    },
}

/// `world_from_tracking` is the XR rig's root: a rotation, a uniform scale
/// (the miniature's) and a translation.
pub(crate) fn key(light: &Light, world_from_tracking: &bevy::transform::components::Transform, scale: Scale) -> Key {
    let rgb = Vec3::from_array(light.color);
    let peak = rgb.max_element();
    if !(peak > 1e-4) {
        return Key::None;
    }
    let color = rgb / peak;
    // Roomtone's colour is irradiance over pi on a surface facing the light.
    let illuminance = PI * peak * scale.luminance;
    if light.placed() && light.range > 0.0 {
        // The colour was measured `range` away: an inverse-square light of
        // I candela gives I / d^2 lux, and Bevy's lumens are 4 pi candela.
        // World distances are the room's times the rig's scale.
        let range = light.range * world_from_tracking.scale.x;
        Key::Point {
            color,
            intensity: 4.0 * PI * illuminance * range * range,
            position: world_from_tracking.transform_point(Vec3::from_array(light.pos.into())),
        }
    } else {
        let toward = world_from_tracking.rotation * Vec3::from_array(light.dir.into());
        Key::Directional { color, illuminance, toward: toward.normalize_or(Vec3::Y) }
    }
}

/// Packs linear RGB as `Rgb9e5Ufloat` (shared 5-bit exponent, 9-bit
/// mantissas): the unsigned HDR format every GPU can filter.
pub(crate) fn rgb9e5(rgb: Vec3) -> u32 {
    const MANTISSA_BITS: i32 = 9;
    const BIAS: i32 = 15;
    const MAX_EXPONENT: i32 = 31;
    let max_value = ((1 << MANTISSA_BITS) - 1) as f32 / (1 << MANTISSA_BITS) as f32 * 2f32.powi(MAX_EXPONENT - BIAS);
    let c = rgb.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, max_value) });
    let peak = c.max_element();
    let mut exponent = (-BIAS - 1).max(peak.max(f32::MIN_POSITIVE).log2().floor() as i32) + 1 + BIAS;
    let mut denominator = 2f32.powi(exponent - BIAS - MANTISSA_BITS);
    if (peak / denominator + 0.5).floor() as i32 == 1 << MANTISSA_BITS {
        denominator *= 2.0;
        exponent += 1;
    }
    let m = c.map(|v| (v / denominator + 0.5).floor());
    (exponent as u32) << 27 | (m.z as u32) << 18 | (m.y as u32) << 9 | m.x as u32
}

#[cfg(test)]
pub(crate) fn rgb9e5_decode(v: u32) -> Vec3 {
    let scale = 2f32.powi((v >> 27) as i32 - 15 - 9);
    Vec3::new((v & 0x1ff) as f32, ((v >> 9) & 0x1ff) as f32, ((v >> 18) & 0x1ff) as f32) * scale
}

/// A Bevy irradiance volume's texture for a grid of `resolution` ambient
/// cubes (`cubes[x + rx * (y + ry * z)]`, sides as [`ambient_cube`]):
/// `Rgb9e5Ufloat` texels of a 3D texture (rx, 2 ry, 3 rz), where the cube
/// side along axis `a` (0 x, 1 y, 2 z) of voxel (x, y, z) is the texel
/// (x, y + ry if the side is the negative one, z + a rz), as
/// `irradiance_volume.wgsl` reads it.
pub(crate) fn volume_texels(resolution: UVec3, cubes: &[[Vec3; 6]]) -> Vec<u8> {
    let (rx, ry, rz) = (resolution.x as usize, resolution.y as usize, resolution.z as usize);
    assert_eq!(cubes.len(), rx * ry * rz);
    let mut out = vec![0u8; rx * 2 * ry * 3 * rz * 4];
    for z in 0..rz {
        for y in 0..ry {
            for x in 0..rx {
                let cube = &cubes[x + rx * (y + ry * z)];
                for axis in 0..3 {
                    for negative in 0..2 {
                        let (s, t, p) = (x, y + negative * ry, z + axis * rz);
                        let at = (s + rx * (t + 2 * ry * p)) * 4;
                        out[at..at + 4].copy_from_slice(&rgb9e5(cube[axis * 2 + negative]).to_le_bytes());
                    }
                }
            }
        }
    }
    out
}

/// Cells added around a grid of more than one point, each repeating the
/// nearest point's light: Bevy lights nothing outside an irradiance
/// volume's box, and the skater can leave the scanned room (a doorway).
const PAD: u32 = 6;

/// Where the light is made, in tracking space: a box cut into
/// `resolution` cells, a point at each cell's middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    pub min: Vec3,
    pub max: Vec3,
    pub resolution: UVec3,
}

impl Grid {
    /// One point, lighting everything: all a capture without depth can tell
    /// (the soft light is then the same wherever it's asked for).
    pub(crate) fn single(at: Vec3) -> Self {
        // Large enough to hold any room at any miniature scale.
        let half = Vec3::splat(500.0);
        Self { min: at - half, max: at + half, resolution: UVec3::ONE }
    }

    /// Over a room's bounds (grown by `margin`): a point about every
    /// `spacing` metres, at most `limit` along each axis.
    pub(crate) fn over(min: Vec3, max: Vec3, margin: f32, spacing: f32, limit: UVec3) -> Self {
        let (min, max) = (min - margin, max + margin);
        let cells = ((max - min) / spacing).round().as_uvec3();
        Self { min, max, resolution: cells.clamp(UVec3::ONE, limit) }
    }

    pub(crate) fn len(&self) -> usize {
        (self.resolution.x * self.resolution.y * self.resolution.z) as usize
    }

    /// `points()[x + rx * (y + ry * z)]`.
    pub(crate) fn points(&self) -> Vec<Vec3> {
        let r = self.resolution;
        let cell = (self.max - self.min) / r.as_vec3();
        let mut out = Vec::with_capacity(self.len());
        for z in 0..r.z {
            for y in 0..r.y {
                for x in 0..r.x {
                    out.push(self.min + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * cell);
                }
            }
        }
        out
    }

    fn pad(&self) -> u32 {
        if self.len() > 1 { PAD } else { 0 }
    }

    /// The irradiance volume's resolution and cubes for this grid's
    /// (`cubes[x + rx * (y + ry * z)]`): the grid's own, and around a grid
    /// of several points [`PAD`] more cells each way that repeat its edge.
    pub(crate) fn volume(&self, cubes: &[[Vec3; 6]]) -> (UVec3, Vec<[Vec3; 6]>) {
        let (r, pad) = (self.resolution, self.pad());
        let padded = r + 2 * pad;
        let mut out = Vec::with_capacity((padded.x * padded.y * padded.z) as usize);
        let inner = |v: u32, r: u32| (v.saturating_sub(pad)).min(r - 1);
        for z in 0..padded.z {
            for y in 0..padded.y {
                for x in 0..padded.x {
                    out.push(cubes[(inner(x, r.x) + r.x * (inner(y, r.y) + r.y * inner(z, r.z))) as usize]);
                }
            }
        }
        (padded, out)
    }

    /// The light probe's transform (Bevy's are unit cubes about the
    /// origin): the grid's box and its padding.
    pub(crate) fn probe(&self, world_from_tracking: &bevy::transform::components::Transform) -> bevy::transform::components::Transform {
        let cell = (self.max - self.min) / self.resolution.as_vec3();
        world_from_tracking.mul_transform(bevy::transform::components::Transform {
            translation: (self.min + self.max) * 0.5,
            scale: self.max - self.min + 2.0 * self.pad() as f32 * cell,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::transform::components::Transform;

    fn lighting() -> Lighting {
        // 0.5 from every side, 0.3 more from above and less from below, red from +X.
        let mut l = Lighting { mean: 0.5, level: 1.0, ..Default::default() };
        l.sh[0] = [0.5; 3];
        l.sh[1] = [0.3; 3];
        l.sh[3] = [0.2, 0.0, 0.0];
        l
    }

    #[test]
    fn scale_puts_the_reference_at_the_rooms_brightness() {
        let l = lighting();
        let exposure = 1.0 / 1000.0;
        let s = Scale::new(&l, exposure, 1.0);
        // A surface lit by the room's mean comes out of the camera at 0.7 (display).
        let shown = l.mean * s.luminance * exposure;
        assert!((shown.powf(1.0 / GAMMA) - 0.7).abs() < 1e-4, "{shown}");
        // A darker room (level) is shown darker; strength trims it.
        let dark = Lighting { level: 0.5, ..l };
        assert!(Scale::new(&dark, exposure, 1.0).luminance < s.luminance * 0.3);
        assert!((Scale::new(&l, exposure, 2.0).luminance / s.luminance - 2.0).abs() < 1e-5);
    }

    #[test]
    fn ambient_cube_follows_the_world_axes() {
        let l = lighting();
        let s = Scale { luminance: 1.0 };
        let cube = ambient_cube(&l, Quat::IDENTITY, s);
        assert!((cube[2] - Vec3::splat(0.8)).length() < 1e-5, "up {:?}", cube[2]);
        assert!((cube[3] - Vec3::splat(0.2)).length() < 1e-5, "down {:?}", cube[3]);
        assert!((cube[0] - Vec3::new(0.7, 0.5, 0.5)).length() < 1e-5, "+x {:?}", cube[0]);
        assert!((cube[1] - Vec3::new(0.3, 0.5, 0.5)).length() < 1e-5, "-x {:?}", cube[1]);
        // The rig turned a quarter: world +Z is the room's +X (the red side).
        let world_from_tracking = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let turned = ambient_cube(&l, world_from_tracking.inverse(), s);
        assert!((turned[4] - Vec3::new(0.7, 0.5, 0.5)).length() < 1e-5, "+z {:?}", turned[4]);
        assert!((turned[2] - cube[2]).length() < 1e-5, "up is up");
    }

    #[test]
    fn an_unplaced_key_is_directional() {
        let mut light = Light::default();
        light.color = [0.4, 0.2, 0.1];
        light.dir = [0.0, 1.0, 0.0].into();
        let root = Transform::from_rotation(Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2)).with_scale(Vec3::splat(4.0));
        let Key::Directional { color, illuminance, toward } = key(&light, &root, Scale { luminance: 100.0 }) else {
            panic!("directional");
        };
        assert!((color - Vec3::new(1.0, 0.5, 0.25)).length() < 1e-5);
        // A white surface facing it has luminance E / pi: 0.4 * 100.
        assert!((illuminance / PI - 40.0).abs() < 1e-3);
        assert!((toward - Vec3::X).length() < 1e-5, "{toward:?}");
        assert_eq!(key(&Light::default(), &root, Scale { luminance: 100.0 }), Key::None);
    }

    #[test]
    fn rgb9e5_round_trips() {
        for rgb in [Vec3::ZERO, Vec3::ONE, Vec3::new(0.001, 0.5, 0.02), Vec3::new(900.0, 30.0, 4000.0), Vec3::splat(1e-7)] {
            let back = rgb9e5_decode(rgb9e5(rgb));
            let tolerance = rgb.max_element() / 256.0 + 1e-6;
            assert!((back - rgb).abs().max_element() <= tolerance, "{rgb:?} -> {back:?}");
        }
        assert_eq!(rgb9e5(Vec3::new(-1.0, f32::NAN, 0.0)), rgb9e5(Vec3::ZERO));
    }

    #[test]
    fn volume_texels_are_laid_out_as_the_shader_reads_them() {
        let resolution = UVec3::new(2, 1, 3);
        // Each voxel's sides: (index, side, 0).
        let cubes: Vec<[Vec3; 6]> =
            (0..6).map(|i| std::array::from_fn(|side| Vec3::new(i as f32, side as f32, 0.0))).collect();
        let texels = volume_texels(resolution, &cubes);
        assert_eq!(texels.len(), 2 * 2 * 9 * 4);
        let texel = |s: usize, t: usize, p: usize| {
            let at = (s + 2 * (t + 2 * p)) * 4;
            rgb9e5_decode(u32::from_le_bytes(texels[at..at + 4].try_into().unwrap()))
        };
        // Voxel (1, 0, 2) is index 1 + 2 * (0 + 1 * 2) = 5.
        assert_eq!(texel(1, 0, 2), Vec3::new(5.0, 0.0, 0.0), "+x");
        assert_eq!(texel(1, 1, 2), Vec3::new(5.0, 1.0, 0.0), "-x");
        assert_eq!(texel(1, 0, 2 + 3), Vec3::new(5.0, 2.0, 0.0), "+y");
        assert_eq!(texel(1, 1, 2 + 6), Vec3::new(5.0, 5.0, 0.0), "-z");
        assert_eq!(texel(0, 0, 0), Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn grid_points_are_cell_middles() {
        let g = Grid::over(Vec3::new(-2.0, 0.0, -1.0), Vec3::new(2.0, 2.0, 1.0), 0.5, 1.5, UVec3::new(4, 2, 4));
        assert_eq!(g.resolution, UVec3::new(3, 2, 2));
        let points = g.points();
        assert_eq!(points.len(), g.len());
        assert!((points[0] - Vec3::new(-2.5 + 5.0 / 6.0, -0.5 + 0.75, -1.5 + 0.75)).length() < 1e-5);
        // The probe's unit cube maps onto the grid's box and its padding, through the rig.
        let root = Transform::from_xyz(10.0, 0.0, 0.0).with_scale(Vec3::splat(2.0));
        let probe = g.probe(&root);
        let cell = (g.max - g.min) / g.resolution.as_vec3();
        let corner = Vec3::new(10.0, 0.0, 0.0) + (g.min - PAD as f32 * cell) * 2.0;
        assert!((probe.transform_point(Vec3::splat(-0.5)) - corner).length() < 1e-3);
        // The volume repeats the edge points' cubes into the padding.
        let cubes: Vec<[Vec3; 6]> = (0..g.len()).map(|i| [Vec3::splat(i as f32); 6]).collect();
        let (resolution, padded) = g.volume(&cubes);
        assert_eq!(resolution, g.resolution + 2 * PAD);
        let at = |x: u32, y: u32, z: u32| padded[(x + resolution.x * (y + resolution.y * z)) as usize][0].x;
        assert_eq!(at(0, 0, 0), 0.0);
        assert_eq!(at(PAD + 1, PAD, PAD), 1.0);
        assert_eq!(at(resolution.x - 1, resolution.y - 1, resolution.z - 1), (g.len() - 1) as f32);
        assert_eq!(at(PAD + 2, 0, resolution.z - 1), (2 + 3 * (0 + 2 * 1)) as f32);
        // One point needs none: its box already holds everything.
        let single = Grid::single(Vec3::ZERO);
        assert_eq!(single.len(), 1);
        assert_eq!(single.volume(&cubes[..1]).0, UVec3::ONE);
    }

    #[test]
    fn a_placed_key_is_a_point_light_with_the_rooms_falloff() {
        // Measured 2 m away; the miniature is 4 game metres to the room's one.
        let mut room = roomtone_sys::Room::new();
        let lamp = Vec3::new(0.3, 0.8, -0.5).normalize();
        room.set_points(&[[0.0, 0.0, 0.0].into()]);
        roomtone_sys::synthetic::sweep(&mut room, |d| {
            let v = if Vec3::from_array(d).dot(lamp) > 0.12f32.cos() { 1.0 } else { 0.1 };
            [v, v, v]
        });
        for _ in 0..3 {
            room.add_hit(Vec3::ZERO.to_array().into(), (lamp * 2.0).to_array().into());
        }
        room.make();
        let made = room.light_at(0).unwrap();
        let root = Transform::from_xyz(1.0, 0.0, 0.0).with_scale(Vec3::splat(4.0));
        let scale = Scale { luminance: 100.0 };
        let Key::Point { intensity, position, .. } = key(&made.lights[0], &root, scale) else {
            panic!("placed: {}", room.summary());
        };
        assert!((position - (Vec3::X + lamp * 8.0)).length() < 0.2, "{position:?}");
        // At the point it was measured from (8 game metres away), it gives
        // the illuminance the directional light would.
        let lux = intensity / (4.0 * PI) / 64.0;
        let expected = PI * made.lights[0].color[0] * scale.luminance;
        assert!((lux / expected - 1.0).abs() < 0.05, "{lux} vs {expected}");
    }
}
