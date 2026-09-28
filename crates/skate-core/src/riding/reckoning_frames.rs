//! Reckoning transform, dynamic lean and lateral tilt, TU3
//!82D8D688/82D8D930/82D8C4F8. State scheduling and frame producers are separate.
//! Numerical primitives retain their documented hardware-validation boundary.
use crate::{
    math::Vector3,
    physics::{
        board_motion_output::inverse_length_squared,
        native_arithmetic::dot3,
        skeleton_animation_record::{AnimationPartTransform, IDENTITY, compose_affine},
    },
    point_graph::PointGraph,
    riding::collision_response::signed_angle,
    trigonometry,
};

#[derive(Clone, Debug)]
pub struct ReckoningFrames {
    ///752: ground frame, whose translation is retained by CalculateTransform.
    pub ground: AnimationPartTransform,
    ///816: system frame after application of the body-flip transform.
    pub system: AnimationPartTransform,
    ///880: same frame before body flip, used by CalculateTilt.
    pub unflipped: AnimationPartTransform,
    ///944: rigid inverse of the final system frame.
    pub inverse_system: AnimationPartTransform,
    ///1008: body-flip transform, produced by UpdateBodyFlip.
    pub body_flip: AnimationPartTransform,
    ///1200: heading, reprojected onto the current up plane on each calculation.
    pub heading: [f32; 4],
    ///1576, published to PhysOutSystemReckoning160 with the stance sign.
    pub target_lean_angle: f32,
    ///1248: first lane consumed by the body-tilt animation handler.
    pub lateral_tilt: [f32; 4],
}

impl ReckoningFrames {
    ///82D33178 initializes each matrix independently to identity and seeds
    ///heading to world X. Reset82D8C3A8 later preserves the matrices.
    pub fn new() -> Self {
        Self {
            ground: IDENTITY,
            system: IDENTITY,
            unflipped: IDENTITY,
            inverse_system: IDENTITY,
            body_flip: IDENTITY,
            heading: [1.0, 0.0, 0.0, 0.0],
            target_lean_angle: 0.0,
            lateral_tilt: [0.0; 4],
        }
    }

    ///82D8D688. Cross products use the unnormalized intermediate axis;
    ///normalization has two refinements and no epsilon fallback in this leaf.
    pub fn calculate_transform(&mut self, up: [f32; 4], ground_normal: [f32; 4]) {
        // `up` parallel to `heading` -- a vertical board, which University's
        // walls produce readily -- collapses `right` and then `forward` to zero.
        // `normalize` has no epsilon fallback in this leaf, and
        // `inverse_length_squared(0, 2)` is NaN rather than infinity (the first
        // Newton step computes `fma(-0, inf, 1)`). `heading` is *persistent*, so
        // one degenerate frame poisons it for the rest of the session: that NaN
        // reaches `animation_to_world`, then the deck's straighten/heading
        // displacements, and lands as "Non-finite torque_acceleration before
        // shared solve" on some later touchdown -- the crash the owner sees.
        //
        // Retaining the previous axis is the standard degenerate handling and is
        // what the off-board frames already do (`surface_frame::math::normalize_or`).
        // Every non-degenerate input keeps its exact former value.
        let right = cross(up, self.heading);
        let forward = cross(right, up);
        self.heading = normalize_or(forward, self.heading);
        self.system[0] = normalize_or(right, self.system[0]);
        self.system[1] = up;
        self.system[2] = self.heading;
        self.unflipped = self.system;
        self.system = compose_affine(&self.body_flip, &self.system);
        self.inverse_system = inverse(&self.system);

        let ground_right = cross(ground_normal, self.heading);
        let ground_forward = cross(ground_right, ground_normal);
        self.ground[0] = normalize_or(ground_right, self.ground[0]);
        self.ground[1] = ground_normal;
        self.ground[2] = normalize_or(ground_forward, self.ground[2]);
    }

    ///82D8D930. Signed angle is wrapped by fraction/floor, not scalar atan2.
    pub fn calculate_dynamic_lean(&mut self, up: [f32; 4], dynamic_up: [f32; 4]) {
        let axis = self.system[2];
        let project = |value| {
            let amount = dot3(value, axis);
            std::array::from_fn(|i| value[i] - axis[i] * amount)
        };
        let from = project(up);
        let to = project(dynamic_up);
        let threshold = f32::from_bits(0x3727_c5ac); //8219B100
        self.target_lean_angle = if dot3(from, from) > threshold && dot3(to, to) > threshold {
            wrap_fraction(signed_angle(xyz(from), xyz(to), xyz(axis)))
        } else {
            0.0
        };
    }

    ///82D8C4F8. Degenerate projection preserves the preceding lateral tilt.
    ///The stock source uses the unflipped up/forward axes and two authored
    ///curves, including its absolute wrapped angle and stance sign change.
    pub fn calculate_tilt(
        &mut self,
        reverse_stance: bool,
        tilt_vs_angle: &PointGraph<8>,
        tilt_vs_up: &PointGraph<8>,
    ) {
        let axis = self.unflipped[1];
        let world_up = [0.0, 1.0, 0.0, 0.0];
        let dot = dot3(axis, world_up);
        let parallel = axis.map(|v| v * dot);
        let projected = std::array::from_fn(|i| world_up[i] - parallel[i]);
        let squared = dot3(projected, projected);
        if squared <= f32::from_bits(0x3a03_126f) {
            return;
        }
        let angle = if world_up == parallel {
            0.0
        } else {
            signed_angle(xyz(self.unflipped[2]), xyz(normalize(projected)), xyz(axis))
        };
        let half_pi = f32::from_bits(0x3fc9_0fdb);
        let scale = f32::from_bits(0x3f22_f983);
        let mut normalized_angle =
            ((wrap_fraction(angle - half_pi).abs() - half_pi) * -1.0) * scale;
        if reverse_stance {
            normalized_angle = -normalized_angle;
        }
        //Inline native asin polynomial82D8C704..7E0 is the same operation
        //tree retained by trigonometry::asin, including one rsqrt refinement.
        let normalized_up_angle = (half_pi - trigonometry::asin(axis[1])) * scale;
        let angle_tilt = tilt_vs_angle.evaluate(normalized_angle.abs());
        let up_tilt = tilt_vs_up.evaluate(normalized_up_angle.abs());
        let signed = if normalized_angle >= -0.0 {
            angle_tilt
        } else {
            -angle_tilt
        };
        self.lateral_tilt = [up_tilt * signed, 0.0, 0.0, 0.0];
    }
}

fn normalize(value: [f32; 4]) -> [f32; 4] {
    let reciprocal = inverse_length_squared(dot3(value, value), 2);
    value.map(|v| v * reciprocal)
}

/// [`normalize`], but a degenerate vector keeps `fallback` instead of becoming
/// NaN. The squared length is tested against zero *after* the f32 multiply, so
/// this also catches a vector whose components are small enough that `dot3`
/// underflows to exact zero while the vector itself is not zero.
fn normalize_or(value: [f32; 4], fallback: [f32; 4]) -> [f32; 4] {
    let squared = dot3(value, value);
    if squared > 0.0 && squared.is_finite() {
        let reciprocal = inverse_length_squared(squared, 2);
        let scaled = value.map(|v| v * reciprocal);
        if scaled.iter().all(|v| v.is_finite()) {
            return scaled;
        }
    }
    fallback
}
fn cross(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        (-a[2]).mul_add(b[1], a[1] * b[2]),
        (-a[0]).mul_add(b[2], a[2] * b[0]),
        (-a[1]).mul_add(b[0], a[0] * b[1]),
        (-a[3]).mul_add(b[3], a[3] * b[3]),
    ]
}
fn inverse(frame: &AnimationPartTransform) -> AnimationPartTransform {
    let axes: [[f32; 4]; 3] = std::array::from_fn(|i| [frame[0][i], frame[1][i], frame[2][i], 0.0]);
    let p = frame[3].map(|v| 0.0 - v);
    let position = std::array::from_fn(|i| {
        let z = axes[2][i] * p[2];
        let y = axes[1][i].mul_add(p[1], z);
        axes[0][i].mul_add(p[0], y)
    });
    [axes[0], axes[1], axes[2], position]
}
fn wrap_fraction(angle: f32) -> f32 {
    let turns = angle * f32::from_bits(0x3e22_f983);
    let fraction = turns - turns.floor();
    let fraction = fraction - if fraction > 0.5 { 1.0 } else { 0.0 };
    fraction * f32::from_bits(0x40c9_0fdb)
}
fn xyz(v: [f32; 4]) -> Vector3 {
    Vector3::new(v[0], v[1], v[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_flip_changes_system_but_retains_ground_and_preflip_frames() {
        let mut state = ReckoningFrames::new();
        state.heading = [0.0, 0.0, 1.0, 0.0];
        state.body_flip = [
            [0., 0., -1., 0.],
            [0., 1., 0., 0.],
            [1., 0., 0., 0.],
            [3., 0., 0., 0.],
        ];
        state.calculate_transform([0., 1., 0., 0.], [0., 1., 0., 0.]);
        assert_eq!(state.unflipped, IDENTITY);
        assert_eq!(state.ground, IDENTITY);
        assert_eq!(state.system, state.body_flip);
        assert_eq!(
            compose_affine(&state.system, &state.inverse_system),
            IDENTITY
        );
        state.calculate_dynamic_lean([0., 1., 0., 0.], [0., 1., 0., 0.]);
        assert!(state.target_lean_angle.abs() < 0.001);
        state.lateral_tilt = [0.3, 0., 0., 0.];
        let curve = PointGraph {
            x: [0.; 8],
            y: [0.; 8],
        };
        state.calculate_tilt(false, &curve, &curve);
        assert_eq!(state.lateral_tilt, [0.3, 0., 0., 0.]);
    }
}

#[cfg(test)]
mod degenerate_tests {
    use super::*;

    /// A vertical board makes `up` parallel to `heading`, which collapses the
    /// cross products to zero. `inverse_length_squared(0, 2)` is NaN, and
    /// `heading` persists, so this used to poison the reckoning frame for the
    /// rest of the session and surface later as a NaN deck torque on a landing.
    #[test]
    fn a_vertical_board_cannot_poison_the_persistent_heading() {
        let mut frames = ReckoningFrames::new();
        frames.heading = [0.0, 1.0, 0.0, 0.0];
        let up = [0.0, 1.0, 0.0, 0.0];
        let before = frames.heading;
        frames.calculate_transform(up, [0.0, 1.0, 0.0, 0.0]);
        assert!(
            frames.heading.iter().all(|v| v.is_finite()),
            "heading went non-finite: {:?}",
            frames.heading
        );
        assert_eq!(
            frames.heading, before,
            "a degenerate frame should retain the last heading"
        );
        assert!(
            frames.system.iter().flatten().all(|v| v.is_finite())
                && frames.ground.iter().flatten().all(|v| v.is_finite()),
            "a derived frame went non-finite"
        );
        // And the frame still works normally afterwards.
        frames.calculate_transform([0.0, 1.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]);
        assert!(frames.heading.iter().all(|v| v.is_finite()));
    }

    /// An ordinary frame must be bit-identical to the unguarded leaf.
    #[test]
    fn a_healthy_frame_is_unchanged_by_the_guard() {
        let v = [0.3, 0.0, 0.9, 0.0];
        assert_eq!(normalize_or(v, [9.0; 4]), normalize(v));
    }
}
