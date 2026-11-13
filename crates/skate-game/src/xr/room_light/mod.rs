//! Room lighting: the skater and the park's pieces lit by the player's real
//! room instead of a fixed sun.
//!
//! Quest has no light estimation API. [Roomtone](https://github.com/rhythmerc/roomtone)
//! (`crates/roomtone-sys`) makes the estimate from the passthrough camera's
//! frames, each reduced to a 32x32 grid of colour with its exposure and the
//! head's pose. This module asks it for the light where the content is and
//! hands that to Bevy's own lighting, so every `StandardMaterial` in either
//! tier takes it with no shader of its own:
//!
//! - the soft light from every direction (L2 spherical harmonics) becomes an
//!   irradiance volume: one ambient cube without depth, and a grid of them
//!   over the room once Roomtone has seen where the room's surfaces are, so
//!   a piece beside a bright wall is lit by it and one across the room isn't;
//! - the room's two brightest lights become a point light each when depth
//!   gave them a place (what is nearer the lamp is lit more), a directional
//!   light when it didn't;
//! - the brightness follows passthrough's own.
//!
//! While it is on (and a light has been made) the tier's own sun and ambient
//! fade out; off, or before the first light, nothing changes.
//!
//! `SK8_ROOM_LIGHT=lamp|lamp-room|window|<recording>` shows it on the desktop park
//! without a headset: a synthetic room, or a session recorded with
//! Roomtone's `Recorder` (`--clean`, no `--xr`).
use super::ar_settings::ArSettings;
use super::build::PlacedPieces;
use super::link::ArWorld;
use super::rig::Rig;
use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::image::ImageSampler;
use bevy::light::{IrradianceVolume, LightProbe};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use roomtone_sys::{Lighting, Room};
use feed::{CameraState, Feed};
use shade::{Grid, Key, Scale};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "android")]
mod camera;
mod depth;
mod feed;
mod reduce;
mod shade;

/// Seconds for the room's light to fade in over the tier's own (and out).
const FADE_S: f32 = 1.0;
/// Seconds for the shown light to settle on a newly made one.
const SETTLE_S: f32 = 0.4;
/// The content's point, above the surface the park stands on, m.
const CONTENT_HEIGHT_M: f32 = 0.3;
/// The grid over a room seen with depth: a point about this far apart...
const GRID_SPACING_M: f32 = 1.5;
/// ...and at most this many along each axis (a make costs one build a point).
const GRID_LIMIT: UVec3 = UVec3::new(3, 2, 3);
const GRID_MARGIN_M: f32 = 0.5;
/// A point light's reach, real metres (Bevy fades it out there).
const POINT_REACH_M: f32 = 30.0;

/// The desktop preview's room, around the park.
const PREVIEW_ROOM: (Vec3, Vec3) = (Vec3::new(-4.0, 0.0, -4.0), Vec3::new(4.0, 2.5, 4.0));

/// The capture, shared with whatever feeds it frames.
#[derive(Resource, Clone)]
pub(crate) struct RoomCapture(pub Arc<Mutex<Room>>);

/// What the game shares with the capture's thread.
#[derive(Resource, Clone)]
struct RoomFeed(Arc<Feed>);

/// Where the light comes from.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// The headset's passthrough camera (the AR settings turn it on).
    Headset,
    /// `SK8_ROOM_LIGHT`: `lamp`, `window`, or a recording's path.
    Preview(String),
}

pub(crate) struct RoomLightPlugin(pub Source);

impl Plugin for RoomLightPlugin {
    fn build(&self, app: &mut App) {
        let mut room = Room::new();
        if let Source::Preview(what) = &self.0 {
            preview(&mut room, what);
        }
        let capture = RoomCapture(Arc::new(Mutex::new(room)));
        let feed = RoomFeed(Arc::new(Feed::default()));
        if matches!(self.0, Source::Headset) {
            #[cfg(target_os = "android")]
            camera::spawn(feed.0.clone(), capture.0.clone());
            app.add_systems(Update, (feed_head, feed_surfaces).before(place_points));
        }
        app.insert_resource(capture)
            .insert_resource(feed)
            .insert_resource(RoomLight { preview: matches!(self.0, Source::Preview(_)), ..default() })
            .add_systems(Startup, spawn)
            .add_systems(Update, (place_points, read, show).chain());
    }
}

/// `SK8_ROOM_LIGHT` on a desktop build, if set.
pub(crate) fn preview_source() -> Option<Source> {
    std::env::var("SK8_ROOM_LIGHT").ok().filter(|v| !v.is_empty()).map(Source::Preview)
}

fn preview(room: &mut Room, what: &str) {
    let lamp = Vec3::new(0.35, 0.8, -0.5).normalize();
    match what {
        // A dim warm room, a bare bulb up and ahead to the right, 2.5 m away.
        "lamp" => {
            roomtone_sys::synthetic::sweep(room, |d| {
                let d = Vec3::from_array(d);
                if d.dot(lamp) > 0.1f32.cos() { [1.0, 1.0, 1.0] } else { (Vec3::new(0.07, 0.055, 0.04) * (1.0 + 0.3 * d.y)).to_array() }
            });
            for _ in 0..3 {
                room.add_hit([0.0; 3].into(), (lamp * 2.5).to_array().into());
            }
            room.make();
        }
        // The same bulb in a room whose walls, floor and ceiling are known
        // (as Space Setup's are on the headset): the capture places what it
        // saw, and the light is made on a grid over the room.
        "lamp-room" => {
            let (min, max) = PREVIEW_ROOM;
            let feed = Feed::default();
            feed.set_surfaces(Some(Arc::new(depth::Surfaces::new(depth::box_room(min, max)))));
            let eye = Vec3::new(0.0, 1.5, 0.0);
            roomtone_sys::synthetic::sweep_with(
                room,
                |d| {
                    let d = Vec3::from_array(d);
                    if d.dot(lamp) > 0.1f32.cos() { [1.0, 1.0, 1.0] } else { (Vec3::new(0.07, 0.055, 0.04) * (1.0 + 0.3 * d.y)).to_array() }
                },
                |room, frame| {
                    let mut frame = *frame;
                    frame.head.position = eye.to_array().into();
                    feed.add_frame(room, &frame);
                },
            );
        }
        // Daylight from a window filling the wall on the left, the room
        // behind it in its bounce.
        "window" => roomtone_sys::synthetic::sweep(room, |d| {
            let d = Vec3::from_array(d);
            if d.x < -0.6 && d.y.abs() < 0.45 { [0.8, 0.9, 1.0] } else { (Vec3::new(0.12, 0.12, 0.13) * (1.0 - 0.5 * d.x)).to_array() }
        }),
        path => {
            if !room.replay(std::path::Path::new(path)) {
                warn!("Room light: couldn't read the recording {path}");
            }
            room.make();
        }
    }
    info!("Room light preview ({what}): {}", room.summary());
}

/// What was last read from the capture, and what is shown of it.
#[derive(Resource, Default)]
pub(crate) struct RoomLight {
    preview: bool,
    /// The points the capture makes the light for: the content's, then the grid's.
    grid: Option<Grid>,
    points: Vec<Vec3>,
    /// The points are the capture's (it was free to take them).
    points_set: bool,
    /// The light at the content's point, and at each of the grid's.
    made: Option<(Lighting, Vec<Lighting>)>,
    /// Whether the capture has placed the room's surfaces (depth).
    pub voxels: bool,
    camera: CameraState,
    /// 0: the tier's own lights; 1: the room's.
    mix: f32,
    cubes: Vec<[Vec3; 6]>,
    resolution: UVec3,
    uploaded: Vec<[Vec3; 6]>,
    keys: [Key; 2],
}

impl Default for CameraState {
    fn default() -> Self {
        CameraState::Off
    }
}

impl Default for Key {
    fn default() -> Self {
        Key::None
    }
}

impl RoomLight {
    /// A line for the settings page, when the setting is on and can't work.
    pub(crate) fn problem(&self, on: bool) -> Option<&'static str> {
        if !on || self.made.is_some() {
            return None;
        }
        match self.camera {
            CameraState::NoPermission => Some("Roomtone needs the headset's cameras: allow them, or turn it off."),
            CameraState::NoCamera => Some("Roomtone: this headset's cameras can't be used (Quest 3 or 3S, system v74 or later)."),
            _ => None,
        }
    }
}

#[derive(Component)]
struct RoomProbe;
/// The room's `n`th light without a place.
#[derive(Component)]
struct RoomSun(usize);
/// The room's `n`th light with one.
#[derive(Component)]
struct RoomLamp(usize);
/// One of the tier's own lights, and how bright the tier made it.
#[derive(Component)]
struct OwnLight(f32);

fn volume_image(resolution: UVec3, cubes: &[[Vec3; 6]]) -> Image {
    let mut image = Image::new(
        Extent3d { width: resolution.x, height: 2 * resolution.y, depth_or_array_layers: 3 * resolution.z },
        TextureDimension::D3,
        shade::volume_texels(resolution, cubes),
        TextureFormat::Rgb9e5Ufloat,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    image
}

fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let voxels = images.add(volume_image(UVec3::ONE, &[[Vec3::ZERO; 6]]));
    commands.spawn((
        RoomProbe,
        LightProbe,
        IrradianceVolume { voxels, intensity: 0.0, ..default() },
        Transform::from_scale(Vec3::splat(1000.0)),
        Visibility::Hidden,
    ));
    for n in 0..2 {
        commands.spawn((
            RoomSun(n),
            DirectionalLight { illuminance: 0.0, shadows_enabled: false, ..default() },
            Transform::default(),
            Visibility::Hidden,
        ));
        commands.spawn((
            RoomLamp(n),
            PointLight { intensity: 0.0, shadows_enabled: false, ..default() },
            Transform::default(),
            Visibility::Hidden,
        ));
    }
}

fn root(rig: Option<&Rig>) -> Transform {
    rig.and_then(Rig::root_transform).unwrap_or_default()
}

/// The room's bounds in tracking space, from the surfaces the skater rides.
fn room_bounds(world: &ArWorld, tracking_from_world: &GlobalTransform) -> Option<(Vec3, Vec3)> {
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for p in world.room().flatten() {
        let p = tracking_from_world.transform_point(p);
        // A free-roam floor has no end: leave such points out.
        if p.abs().max_element() > 50.0 {
            continue;
        }
        bounds = Some(bounds.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
    }
    bounds.filter(|(lo, hi)| (*hi - *lo).min_element() > 0.5)
}

/// The head's pose for the capture's frames, and whether it should run.
fn feed_head(
    feed: Res<RoomFeed>,
    settings: Res<ArSettings>,
    mut light: ResMut<RoomLight>,
    eyes: Query<(&Transform, &Camera), With<bevy_mod_xr::camera::XrCamera>>,
) {
    feed.0.set_wanted(settings.room_light);
    let state = feed.0.state();
    if light.camera != state {
        light.camera = state;
    }
    // The eye cameras' own transforms are tracking space (the rig scales their
    // parent). They render only while the runtime shows the game: off the
    // face or in the system menu they don't, their pose is the last tracked
    // one, and the camera's frames aren't the room as played in (often black).
    let mut eyes = eyes.iter().filter(|(_, camera)| camera.is_active).map(|(t, _)| t);
    let head = eyes.next().map(|first| {
        let (sum, count) = eyes.fold((first.translation, 1.0), |(sum, count), eye| (sum + eye.translation, count + 1.0));
        let (p, q) = (sum / count, first.rotation);
        roomtone_sys::Pose {
            orientation: roomtone_sys::Quat { x: q.x, y: q.y, z: q.z, w: q.w },
            position: p.to_array().into(),
        }
    });
    feed.0.set_head(head);
}

/// The room's surfaces, for the capture's depth: Space Setup's, as ridden.
fn feed_surfaces(
    feed: Res<RoomFeed>,
    settings: Res<ArSettings>,
    rig: Res<Rig>,
    world: Res<ArWorld>,
    mut seen: Local<Option<(u64, Transform, bool)>>,
) {
    let root = root(Some(&rig));
    let key = (world.revision, root, settings.room_light && !settings.free_roam());
    if seen.as_ref() == Some(&key) {
        return;
    }
    *seen = Some(key);
    if !key.2 {
        feed.0.set_surfaces(None);
        return;
    }
    let tracking_from_world = GlobalTransform::from(root).affine().inverse();
    let triangles: Vec<[Vec3; 3]> = world
        .room()
        .map(|t| t.map(|p| tracking_from_world.transform_point3(p)))
        // (A fallback floor has no end.)
        .filter(|t| t.iter().all(|p| p.abs().max_element() < 50.0))
        .collect();
    let surfaces = depth::Surfaces::new(triangles);
    info!("Room light: depth from {} room triangles", surfaces.len());
    feed.0.set_surfaces((surfaces.len() > 0).then(|| Arc::new(surfaces)));
}

/// Chooses the points the light is made for, and gives them to the capture.
fn place_points(
    capture: Res<RoomCapture>,
    mut light: ResMut<RoomLight>,
    rig: Option<Res<Rig>>,
    world: Option<Res<ArWorld>>,
    pieces: Option<Res<PlacedPieces>>,
    mut seen: Local<Option<(u64, u64, Transform, bool)>>,
) {
    let root = root(rig.as_deref());
    let key = (
        world.as_ref().map_or(0, |w| w.revision),
        pieces.as_ref().map_or(0, |p| p.revision),
        root,
        light.voxels,
    );
    if seen.as_ref() != Some(&key) || light.grid.is_none() {
        *seen = Some(key);
        let bounds = if light.preview {
            Some(PREVIEW_ROOM)
        } else {
            world.as_ref().and_then(|w| room_bounds(w, &GlobalTransform::from(root).affine().inverse().into()))
        };
        // The content: the park's middle, or the room's, a little above the
        // floor (the frames that looked there set the brightness).
        let placed: Vec<Vec3> =
            pieces.iter().flat_map(|p| p.items.iter().map(|i| Vec3::from_array(i.position))).collect();
        let content = if !placed.is_empty() {
            placed.iter().sum::<Vec3>() / placed.len() as f32
        } else if let Some((lo, hi)) = bounds {
            ((lo + hi) * 0.5).with_y(lo.y)
        } else {
            rig.as_ref().and_then(|r| r.anchor).map_or(Vec3::ZERO, |(p, _)| p)
        } + Vec3::Y * CONTENT_HEIGHT_M;
        let grid = match bounds {
            Some((lo, hi)) if light.voxels => Grid::over(lo, hi, GRID_MARGIN_M, GRID_SPACING_M, GRID_LIMIT),
            _ => Grid::single(content),
        };
        let mut points = vec![content];
        points.extend(grid.points());
        if light.grid != Some(grid) || light.points != points {
            light.grid = Some(grid);
            light.points = points;
            light.points_set = false;
        }
    }
    if !light.points_set {
        // The capture may be busy making a light on its own thread: next frame.
        if let Ok(mut room) = capture.0.try_lock() {
            let points: Vec<roomtone_sys::Vec3> = light.points.iter().map(|p| p.to_array().into()).collect();
            room.set_points(&points);
            if light.preview {
                room.make();
            }
            light.points_set = true;
            // What was made before was for other points.
            light.made = None;
        }
    }
}

/// Reads the light the capture last made.
fn read(capture: Res<RoomCapture>, mut light: ResMut<RoomLight>) {
    if !light.points_set {
        return;
    }
    let Ok(room) = capture.0.try_lock() else { return };
    light.voxels = room.voxels() > 0;
    if !room.made() || room.stale() {
        return;
    }
    let Some(content) = room.light_at(0).filter(finite) else { return };
    let grid: Option<Vec<Lighting>> = (1..light.points.len()).map(|i| room.light_at(i).filter(finite)).collect();
    let Some(grid) = grid else { return };
    // A level that falls (or rises) several times over is what a wrong light
    // looks like: say what the capture saw, for the log.
    let was = light.made.as_ref().map_or(content.level, |(l, _)| l.level);
    if content.level < was / LEVEL_JUMP || content.level > was * LEVEL_JUMP {
        info!("Room light: level {was:.2} -> {:.2}: {}", content.level, room.summary());
    }
    light.made = Some((content, grid));
}

/// A level change this many times over is logged.
const LEVEL_JUMP: f32 = 2.0;

/// No NaN or infinity anywhere: one would stick in the shown light's
/// smoothing and blacken everything it lights.
fn finite(l: &Lighting) -> bool {
    l.sh.iter().flatten().all(|v| v.is_finite())
        && l.lights.iter().all(|k| {
            k.color.iter().all(|v| v.is_finite())
                && [k.dir, k.pos].iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
                && k.range.is_finite()
        })
        && l.mean.is_finite()
        && l.level.is_finite()
}

fn show(
    time: Res<Time>,
    mut light: ResMut<RoomLight>,
    settings: Option<Res<ArSettings>>,
    rig: Option<Res<Rig>>,
    exposures: Query<&Exposure, With<Camera3d>>,
    mut images: ResMut<Assets<Image>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut own_ambient: Local<Option<f32>>,
    mut commands: Commands,
    new_lights: Query<(Entity, &DirectionalLight), (Without<RoomSun>, Without<OwnLight>)>,
    mut own_lights: Query<(&mut DirectionalLight, &OwnLight), Without<RoomSun>>,
    mut probe: Query<(&mut IrradianceVolume, &mut Transform, &mut Visibility), With<RoomProbe>>,
    mut suns: Query<
        (&RoomSun, &mut DirectionalLight, &mut Transform, &mut Visibility),
        (Without<RoomProbe>, Without<RoomLamp>),
    >,
    mut lamps: Query<
        (&RoomLamp, &mut PointLight, &mut Transform, &mut Visibility),
        (Without<RoomProbe>, Without<RoomSun>),
    >,
) {
    let on = light.preview || settings.as_ref().is_some_and(|s| s.room_light);
    let dt = time.delta_secs().min(0.1);
    let target = if on && light.made.is_some() { 1.0 } else { 0.0 };
    let step = dt / FADE_S;
    light.mix = if light.mix < target { (light.mix + step).min(target) } else { (light.mix - step).max(target) };
    let mix = light.mix;

    // The tier's own lights give way.
    for (entity, own) in &new_lights {
        commands.entity(entity).insert(OwnLight(own.illuminance));
    }
    for (mut own, OwnLight(full)) in &mut own_lights {
        let shown = full * (1.0 - mix);
        if own.illuminance != shown {
            own.illuminance = shown;
        }
    }
    let full_ambient = *own_ambient.get_or_insert(ambient.brightness);
    let shown_ambient = full_ambient * (1.0 - mix);
    if ambient.brightness != shown_ambient {
        ambient.brightness = shown_ambient;
    }

    let Ok((mut volume, mut probe_transform, mut probe_visibility)) = probe.single_mut() else { return };
    let (Some((content, grid_light)), Some(grid)) = (light.made.clone(), light.grid) else {
        set_visibility(&mut probe_visibility, false);
        return;
    };
    let root = root(rig.as_deref());
    let exposure = exposures.iter().next().copied().unwrap_or_default().exposure();
    let strength = settings.as_ref().map_or(1.0, |s| s.room_light_strength);
    let scale = Scale::new(&content, exposure, strength);
    if !scale.luminance.is_finite() {
        return;
    }

    // The soft light: settle on the newly made cubes, and upload when they've moved.
    let tracking_from_world = root.rotation.inverse();
    let made: Vec<[Vec3; 6]> = grid_light.iter().map(|l| shade::ambient_cube(l, tracking_from_world, scale)).collect();
    let weight = 1.0 - (-dt / SETTLE_S).exp();
    let broken = light.cubes.iter().flatten().any(|side| !side.is_finite());
    if light.resolution != grid.resolution || light.cubes.len() != made.len() || broken {
        light.resolution = grid.resolution;
        light.cubes = made;
        light.uploaded.clear();
    } else {
        for (shown, made) in light.cubes.iter_mut().zip(&made) {
            for side in 0..6 {
                shown[side] = shown[side].lerp(made[side], weight);
            }
        }
    }
    let moved = light.uploaded.len() != light.cubes.len()
        || light.cubes.iter().zip(&light.uploaded).any(|(a, b)| {
            (0..6).any(|s| (a[s] - b[s]).abs().max_element() > 0.004 * a[s].max_element().max(b[s].max_element()))
        });
    if moved {
        if let Some(image) = images.get_mut(&volume.voxels) {
            let (resolution, cubes) = grid.volume(&light.cubes);
            *image = volume_image(resolution, &cubes);
        }
        light.uploaded = light.cubes.clone();
    }
    if volume.intensity != mix {
        volume.intensity = mix;
    }
    let probe_target = grid.probe(&root);
    if *probe_transform != probe_target {
        *probe_transform = probe_target;
    }
    set_visibility(&mut probe_visibility, mix > 0.0);

    // The key lights.
    for n in 0..2 {
        let made = shade::key(&content.lights[n], &root, scale);
        light.keys[n] = match (light.keys[n], made) {
            (Key::Directional { color, illuminance, toward }, Key::Directional { color: c, illuminance: i, toward: t }) => {
                Key::Directional {
                    color: color.lerp(c, weight),
                    illuminance: illuminance + (i - illuminance) * weight,
                    toward: toward.lerp(t, weight).normalize_or(t),
                }
            }
            (Key::Point { color, intensity, position }, Key::Point { color: c, intensity: i, position: p }) => Key::Point {
                color: color.lerp(c, weight),
                intensity: intensity + (i - intensity) * weight,
                position: position.lerp(p, weight),
            },
            (_, made) => made,
        };
        if !key_finite(&light.keys[n]) {
            light.keys[n] = made;
        }
    }
    for (sun, mut directional, mut transform, mut visibility) in &mut suns {
        let Key::Directional { color, illuminance, toward } = light.keys[sun.0] else {
            set_visibility(&mut visibility, false);
            continue;
        };
        directional.color = Color::linear_rgb(color.x, color.y, color.z);
        directional.illuminance = illuminance * mix;
        *transform = Transform::default().looking_to(-toward, if toward.y.abs() > 0.99 { Vec3::X } else { Vec3::Y });
        set_visibility(&mut visibility, mix > 0.0);
    }
    for (lamp, mut point, mut transform, mut visibility) in &mut lamps {
        let Key::Point { color, intensity, position } = light.keys[lamp.0] else {
            set_visibility(&mut visibility, false);
            continue;
        };
        point.color = Color::linear_rgb(color.x, color.y, color.z);
        point.intensity = intensity * mix;
        point.range = POINT_REACH_M * root.scale.x;
        transform.translation = position;
        set_visibility(&mut visibility, mix > 0.0);
    }
}

fn key_finite(key: &Key) -> bool {
    match *key {
        Key::None => true,
        Key::Directional { color, illuminance, toward } => color.is_finite() && illuminance.is_finite() && toward.is_finite(),
        Key::Point { color, intensity, position } => color.is_finite() && intensity.is_finite() && position.is_finite(),
    }
}

fn set_visibility(visibility: &mut Visibility, shown: bool) {
    let target = if shown { Visibility::Visible } else { Visibility::Hidden };
    if *visibility != target {
        *visibility = target;
    }
}
