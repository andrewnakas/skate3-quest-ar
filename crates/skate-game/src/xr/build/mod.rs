//! Build mode: put skate pieces on the real floor, tables and furniture.
//!
//! Hold the left menu button for a second (without chording it) to enter or
//! leave. While building the pad is neutral and the controllers do this:
//!
//! - left stick left/right: choose a piece (the wrist palette above the left
//!   controller shows them; the chosen one is raised)
//! - left stick up/down: grow or shrink the whole world (skater and pieces
//!   together) against the real room; the skater stays where it was
//! - left stick click: drop a starter park where the right controller points
//! - left trigger: undo
//! - right controller: point; a ghost of the piece shows where it would go
//! - right trigger: drop it there
//! - right stick left/right: turn it in 15° steps (it lines up with a table
//!   edge when within 10°); up/down: resize it. Pointing at a placed piece,
//!   the stick turns and resizes that piece instead
//! - right stick click on a placed piece: copy it (kind, size, turn)
//! - right grip on a placed piece: pick it up, release to put it down
//! - B on a placed piece: delete it
//! - Y: rescan the room (Quest Space Setup)
//! - hold X: clear every piece
//!
//! Pieces live in tracking (STAGE) space, so they stay on the same real table,
//! and are saved between launches ([`layout`]). Their collision and grind
//! rails are built by `xr::real_world`.
mod layout;
pub(crate) mod pieces;
mod snap;

use self::pieces::{MAX_SIZE, MIN_SIZE, Piece, PieceGeometry, PieceKind};
use super::actions::{Hand, LatestTouch, TouchRead};
use super::planes::{SceneCapture, SceneShapes, Shape};
use super::rig::{Rig, heading};
use super::spawn::{ray_triangle, spawn_matrix};
use crate::input::ExternalPad;
use crate::physics::{GamePhysics, SkaterRuntime};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy_mod_openxr::openxr_session_running;
use serde::{Deserialize, Serialize};
use skate_core::input::xbox::XboxState;
use skate_core::physics::board::BodyId;

const HOLD_TO_TOGGLE_SECONDS: f32 = 1.0;
/// Left stick deflection that makes a menu hold a D-pad chord instead.
const CHORD_STICK: f32 = 0.3;
const PRESS: f32 = 0.7;
const RELEASE: f32 = 0.4;
const FLICK: f32 = 0.6;
const ROTATE_STEP: f32 = std::f32::consts::PI / 12.0;
/// Size change per second at full stick deflection (multiplicative).
const RESIZE_RATE: f32 = 0.8;
/// Piece sizes are kept on this grid, so each kind has a bounded set of meshes.
const SIZE_STEP: f32 = 0.05;
/// World scale change per second at full stick deflection (multiplicative).
const SCALE_RATE: f32 = 1.0;
/// Game metres per real metre: 1 is life size, 60 is a tiny toy.
pub(crate) const MIN_SCALE: f32 = 1.0;
pub(crate) const MAX_SCALE: f32 = 60.0;
const STICK_DEADZONE: f32 = 0.25;
const UNDO_DEPTH: usize = 40;
const CLEAR_HOLD_SECONDS: f32 = 1.0;
/// Game-world units (metres at skater scale).
const MAX_RANGE: f32 = 2000.0;
const MISS_LENGTH: f32 = 40.0;
/// Pieces go on ground-like faces, not walls.
const MIN_UP: f32 = 0.6;
/// Wrist palette: real-world size of the largest icon, and spacing, metres.
const ICON_SIZE_M: f32 = 0.024;
const ICON_SPACING_M: f32 = 0.03;

#[derive(Resource, Default)]
pub(crate) struct BuildMode {
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Placed {
    pub id: u64,
    pub piece: Piece,
    /// Tracking-space (STAGE) position of the piece's base centre.
    pub position: [f32; 3],
    /// Turn about +Y in tracking space.
    pub yaw: f32,
}

impl Placed {
    pub(crate) fn world_transform(&self, root: &Transform) -> Transform {
        Transform {
            translation: root.transform_point(Vec3::from_array(self.position)),
            rotation: root.rotation * Quat::from_rotation_y(self.yaw),
            scale: Vec3::ONE,
        }
    }
}

/// Everything the player has placed. `revision` changes on every committed
/// edit (collision rebuild and save key off it); `dragging` is the piece
/// following the controller, `resizing` the one being resized and
/// `rescaling` the world scale being changed: collision waits until they end.
#[derive(Resource, Default)]
pub(crate) struct PlacedPieces {
    pub items: Vec<Placed>,
    pub revision: u64,
    pub dragging: Option<u64>,
    pub resizing: Option<u64>,
    pub rescaling: bool,
    /// Earlier layouts, newest last, for undo.
    history: Vec<Vec<Placed>>,
}

impl PlacedPieces {
    pub(crate) fn busy(&self) -> bool {
        self.dragging.is_some() || self.resizing.is_some() || self.rescaling
    }

    /// Remember the current layout before an edit.
    fn checkpoint(&mut self) {
        if self.history.len() == UNDO_DEPTH {
            self.history.remove(0);
        }
        self.history.push(self.items.clone());
    }

    fn undo(&mut self) -> bool {
        let Some(items) = self.history.pop() else {
            return false;
        };
        self.items = items;
        self.dragging = None;
        self.resizing = None;
        self.revision += 1;
        true
    }

    fn add(&mut self, piece: Piece, position: Vec3, yaw: f32) -> u64 {
        self.checkpoint();
        self.insert(piece, position, yaw)
    }

    fn insert(&mut self, piece: Piece, position: Vec3, yaw: f32) -> u64 {
        let id = self.items.iter().map(|p| p.id).max().map_or(1, |m| m + 1);
        self.items.push(Placed {
            id,
            piece,
            position: position.to_array(),
            yaw,
        });
        self.revision += 1;
        id
    }

    fn remove(&mut self, id: u64) -> Option<Placed> {
        let at = self.items.iter().position(|p| p.id == id)?;
        self.checkpoint();
        self.revision += 1;
        Some(self.items.remove(at))
    }
}

#[derive(Resource)]
struct BuildState {
    selected: usize,
    sizes: [f32; PieceKind::ALL.len()],
    yaw: f32,
    /// `yaw`, lined up with a table edge when the ghost is on one.
    placed_yaw: f32,
    /// World-space ray (origin, end, lands on a ground-like face).
    ray: Option<(Vec3, Vec3, bool)>,
    /// Where the selected piece would go (tracking position, world point).
    ghost: Option<(Vec3, Vec3)>,
    /// The placed piece the ray is on.
    target: Option<u64>,
    menu_hold: Option<f32>,
    toggled: bool,
    clear_hold: f32,
    /// Where the skater's deck was in the room when a rescale began.
    rescale_from: Option<(Vec3, Vec3)>,
    /// Unsnapped size of the placed piece being resized.
    resize_size: f32,
    previous: Buttons,
}

impl Default for BuildState {
    fn default() -> Self {
        Self {
            selected: 0,
            sizes: [1.0; PieceKind::ALL.len()],
            yaw: 0.0,
            placed_yaw: 0.0,
            ray: None,
            ghost: None,
            target: None,
            menu_hold: None,
            toggled: false,
            clear_hold: 0.0,
            rescale_from: None,
            resize_size: 1.0,
            previous: Buttons::default(),
        }
    }
}

impl BuildState {
    fn piece(&self) -> Piece {
        Piece {
            kind: PieceKind::ALL[self.selected],
            size: snap_size(self.sizes[self.selected]),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Buttons {
    trigger: bool,
    grip: bool,
    b: bool,
    y: bool,
    undo: bool,
    park: bool,
    copy: bool,
    left_flick: i8,
    right_flick: i8,
}

pub(crate) struct BuildPlugin;

impl Plugin for BuildPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BuildMode>()
            .init_resource::<PlacedPieces>()
            .init_resource::<BuildState>()
            .init_resource::<PieceAssets>()
            .add_systems(Startup, (layout::load, spawn_visuals))
            .add_systems(
                PreUpdate,
                (toggle_build, build_controls)
                    .chain()
                    .after(TouchRead)
                    .before(crate::input::poll_controllers)
                    .run_if(openxr_session_running),
            )
            .add_systems(Update, (sync_pieces, show_build, layout::save));
    }
}

fn toggle_build(
    touch: Res<LatestTouch>,
    time: Res<Time<Real>>,
    mut mode: ResMut<BuildMode>,
    mut state: ResMut<BuildState>,
    mut placed: ResMut<PlacedPieces>,
) {
    let touch = &touch.0;
    let chording = touch.x
        || touch.left_stick[0].abs() > CHORD_STICK
        || touch.left_stick[1].abs() > CHORD_STICK;
    state.menu_hold = match (state.menu_hold, touch.menu && !chording) {
        (_, false) => {
            state.toggled = false;
            None
        }
        (None, true) => Some(0.0),
        (Some(t), true) => Some(t + time.delta_secs()),
    };
    if state.menu_hold.is_some_and(|t| t >= HOLD_TO_TOGGLE_SECONDS) && !state.toggled {
        state.toggled = true;
        mode.active = !mode.active;
        if !mode.active && (placed.dragging.take().is_some() | placed.resizing.take().is_some()) {
            placed.revision += 1;
        }
        info!("BUILD mode: {}", mode.active);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_controls(
    touch: Res<LatestTouch>,
    time: Res<Time<Real>>,
    mode: Res<BuildMode>,
    mut rig: ResMut<Rig>,
    hands: Query<(&Hand, &Transform)>,
    physics: Option<Res<GamePhysics>>,
    skater: Option<ResMut<SkaterRuntime>>,
    shapes: Res<SceneShapes>,
    mut pad: ResMut<ExternalPad>,
    mut state: ResMut<BuildState>,
    mut placed: ResMut<PlacedPieces>,
    mut capture: ResMut<SceneCapture>,
) {
    if !mode.active {
        state.ray = None;
        state.ghost = None;
        state.target = None;
        if placed.rescaling {
            finish_rescale(&rig, skater, &mut state, &mut placed);
        }
        return;
    }
    pad.set(XboxState {
        buttons: 0,
        triggers: [0, 0],
        left: [0, 0],
        right: [0, 0],
    });
    let touch = &touch.0;
    let flick = |x: f32, was: i8| match x {
        x if x > FLICK => 1,
        x if x < -FLICK => -1,
        x if x.abs() < FLICK * 0.5 => 0,
        _ => was,
    };
    let previous = state.previous;
    let now = Buttons {
        trigger: if previous.trigger { touch.right_trigger > RELEASE } else { touch.right_trigger > PRESS },
        grip: if previous.grip { touch.right_grip > RELEASE } else { touch.right_grip > PRESS },
        b: touch.b,
        y: touch.y,
        undo: if previous.undo { touch.left_trigger > RELEASE } else { touch.left_trigger > PRESS },
        park: touch.left_stick_click,
        copy: touch.right_stick_click,
        left_flick: flick(touch.left_stick[0], previous.left_flick),
        right_flick: flick(touch.right_stick[0], previous.right_flick),
    };
    state.previous = now;
    let pressed = |now: bool, was: bool| now && !was;

    if now.left_flick != 0 && now.left_flick != previous.left_flick {
        let n = PieceKind::ALL.len() as i32;
        state.selected = (state.selected as i32 + now.left_flick as i32).rem_euclid(n) as usize;
        info!("BUILD selected {:?}", PieceKind::ALL[state.selected]);
    }
    if pressed(now.y, previous.y) {
        capture.requested = true;
        info!("BUILD rescan requested");
    }
    if pressed(now.undo, previous.undo) {
        if placed.undo() {
            info!("BUILD undo ({} pieces)", placed.items.len());
        }
        return;
    }
    if touch.x {
        state.clear_hold += time.delta_secs();
        if state.clear_hold >= CLEAR_HOLD_SECONDS && !placed.items.is_empty() {
            placed.checkpoint();
            placed.items.clear();
            placed.dragging = None;
            placed.resizing = None;
            placed.revision += 1;
            info!("BUILD cleared");
        }
    } else {
        state.clear_hold = 0.0;
    }

    // World scale: up grows the skater and pieces in the room, down shrinks
    // them. Collision and the skater follow once the stick is let go.
    let [scale_x, scale_y] = touch.left_stick;
    if scale_y.abs() > STICK_DEADZONE && scale_y.abs() > scale_x.abs() {
        if !placed.rescaling {
            placed.rescaling = true;
            state.rescale_from = rig.root_transform().zip(physics.as_deref()).map(|(root, physics)| {
                let deck = physics.board.part_transforms()[BodyId::Deck.index()];
                let to_tracking = root.compute_affine().inverse();
                let t = deck.translation;
                let f = deck.basis.columns[2];
                (
                    to_tracking.transform_point3(Vec3::new(t.x, t.y, t.z)),
                    to_tracking.transform_vector3(Vec3::new(f[0], f[1], f[2])),
                )
            });
        }
        let scale = rig.scale * (-scale_y * SCALE_RATE * time.delta_secs()).exp();
        rig.scale = scale.clamp(MIN_SCALE, MAX_SCALE);
    } else if placed.rescaling {
        finish_rescale(&rig, skater, &mut state, &mut placed);
    }

    let (Some(root), Some(physics)) = (rig.root_transform(), physics) else {
        return;
    };
    let Some((_, controller)) = hands.iter().find(|(hand, _)| matches!(hand, Hand::Right)) else {
        return;
    };
    let origin = root.transform_point(controller.translation);
    let direction = (root.rotation * (controller.rotation * Vec3::NEG_Z)).normalize();
    if state.ghost.is_none() && state.target.is_none() {
        // Face pieces away from the player until they are turned.
        state.yaw = heading(controller.rotation);
    }
    let hit = cast(&physics, &placed, &root, origin, direction);
    let to_tracking = root.compute_affine().inverse();
    // A piece being resized keeps the stick until it is let go.
    state.target = placed.resizing.or(hit.and_then(|h| h.piece));
    let valid = hit.filter(|h| h.normal.y >= MIN_UP);
    state.ray = Some(match hit {
        Some(h) => (origin, h.point, valid.is_some()),
        None => (origin, origin + direction * MISS_LENGTH, false),
    });
    state.ghost = valid.map(|h| (to_tracking.transform_point3(h.point), h.point));

    // Right stick: turn and resize the piece pointed at, or else the ghost.
    let resize = touch.right_stick[1];
    let resizing = resize.abs() > STICK_DEADZONE && resize.abs() > touch.right_stick[0].abs();
    let turn = (now.right_flick != 0 && now.right_flick != previous.right_flick)
        .then(|| -ROTATE_STEP * now.right_flick as f32);
    let factor = (resize * RESIZE_RATE * time.delta_secs()).exp();
    match state.target.filter(|_| placed.dragging.is_none()) {
        Some(id) => {
            if let Some(turn) = turn {
                placed.checkpoint();
                if let Some(item) = placed.items.iter_mut().find(|p| p.id == id) {
                    item.yaw += turn;
                }
                placed.revision += 1;
                info!("BUILD turned piece {id}");
            }
            if resizing {
                if placed.resizing.is_none() {
                    placed.checkpoint();
                    placed.resizing = Some(id);
                    state.resize_size = placed.items.iter().find(|p| p.id == id).map_or(1.0, |p| p.piece.size);
                }
                state.resize_size = (state.resize_size * factor).clamp(MIN_SIZE, MAX_SIZE);
                let size = snap_size(state.resize_size);
                if let Some(item) = placed.items.iter_mut().find(|p| p.id == id) {
                    item.piece.size = size;
                }
            }
        }
        None => {
            if let Some(turn) = turn {
                state.yaw += turn;
            }
            if resizing {
                let selected = state.selected;
                state.sizes[selected] = (state.sizes[selected] * factor).clamp(MIN_SIZE, MAX_SIZE);
            }
        }
    }
    if !resizing {
        if let Some(id) = placed.resizing.take() {
            placed.revision += 1;
            info!("BUILD resized piece {id}");
        }
    }

    state.placed_yaw = match state.ghost {
        Some((position, _)) => snap::snap_yaw(
            shapes.0.iter().filter_map(|s| match *s {
                Shape::Box { pose, half } => Some((pose.rotation, pose.translation, half)),
                Shape::Plane { .. } => None,
            }),
            position,
            state.yaw,
        ),
        None => state.yaw,
    };

    if let Some(id) = placed.dragging {
        if let (Some((position, _)), Some(item)) =
            (state.ghost, placed.items.iter_mut().find(|p| p.id == id))
        {
            item.position = position.to_array();
            item.yaw = state.placed_yaw;
        }
        if !now.grip {
            placed.dragging = None;
            placed.revision += 1;
            info!("BUILD moved piece {id}");
        }
        return;
    }
    if placed.resizing.is_some() {
        return;
    }
    if pressed(now.grip, previous.grip) {
        if let Some(id) = state.target {
            placed.checkpoint();
            placed.dragging = Some(id);
            if let Some(item) = placed.items.iter().find(|p| p.id == id) {
                state.yaw = item.yaw;
            }
            info!("BUILD picked up piece {id}");
        }
        return;
    }
    if pressed(now.copy, previous.copy) {
        if let Some(item) = state.target.and_then(|id| placed.items.iter().find(|p| p.id == id)) {
            let selected = PieceKind::ALL.iter().position(|&k| k == item.piece.kind).unwrap_or(0);
            state.selected = selected;
            state.sizes[selected] = item.piece.size;
            state.yaw = item.yaw;
            info!("BUILD copied piece {} ({:?})", item.id, item.piece.kind);
        }
        return;
    }
    if pressed(now.b, previous.b) {
        if let Some(removed) = state.target.and_then(|id| placed.remove(id)) {
            info!("BUILD deleted {:?} (piece {})", removed.piece.kind, removed.id);
        }
        return;
    }
    if pressed(now.park, previous.park) {
        if let Some((position, _)) = state.ghost {
            placed.checkpoint();
            let yaw = state.placed_yaw;
            for (kind, size, offset, turn) in STARTER_PARK {
                // Offsets are game metres in the park's frame; positions are
                // tracking metres.
                let at = position + Quat::from_rotation_y(yaw) * Vec3::from_array(offset) / rig.scale;
                placed.insert(Piece { kind, size }, at, yaw + turn);
            }
            info!("BUILD starter park at {position:?} ({} pieces)", placed.items.len());
        }
        return;
    }
    if pressed(now.trigger, previous.trigger) {
        if let Some((position, _)) = state.ghost {
            let piece = state.piece();
            let yaw = state.placed_yaw;
            let id = placed.add(piece, position, yaw);
            info!("BUILD placed {:?} size {:.2} as piece {id} at {position:?}", piece.kind, piece.size);
        }
    }
}

/// A small line to skate away from the player: kicker and rail side by side,
/// a fun box, a ledge and a quarter pipe to turn on. (kind, size, offset in
/// game metres with -z away from the player, extra turn).
const STARTER_PARK: [(PieceKind, f32, [f32; 3], f32); 5] = [
    (PieceKind::Kicker, 0.8, [-4.0, 0.0, -5.0], 0.0),
    (PieceKind::Rail, 0.8, [4.0, 0.0, -6.0], std::f32::consts::FRAC_PI_2),
    (PieceKind::FunBox, 1.0, [0.0, 0.0, -12.0], 0.0),
    (PieceKind::Ledge, 0.8, [6.0, 0.0, -14.0], std::f32::consts::FRAC_PI_2),
    (PieceKind::QuarterPipe, 1.0, [0.0, 0.0, -20.0], 0.0),
];

fn snap_size(size: f32) -> f32 {
    ((size / SIZE_STEP).round() * SIZE_STEP).clamp(MIN_SIZE, MAX_SIZE)
}

/// The scale stick was let go: put the skater back on the same real spot at
/// the new scale, and rebuild collision (and save) for it.
fn finish_rescale(
    rig: &Rig,
    skater: Option<ResMut<SkaterRuntime>>,
    state: &mut BuildState,
    placed: &mut PlacedPieces,
) {
    placed.rescaling = false;
    placed.revision += 1;
    info!("BUILD world scale {:.2} (game metres per real metre)", rig.scale);
    let (Some((deck, forward)), Some(root), Some(mut skater)) =
        (state.rescale_from.take(), rig.root_transform(), skater)
    else {
        return;
    };
    let point = root.transform_point(deck);
    if let Err(e) = skater.travel_to(spawn_matrix(point, root.rotation * forward)) {
        warn!("BUILD rescale teleport: {e}");
    }
}

#[derive(Clone, Copy)]
struct Hit {
    point: Vec3,
    normal: Vec3,
    piece: Option<u64>,
}

/// Nearest face along the ray: the real room (collision tag 0) or a placed
/// piece. The piece being dragged is ignored so it can't land on itself.
fn cast(
    physics: &GamePhysics,
    placed: &PlacedPieces,
    root: &Transform,
    origin: Vec3,
    direction: Vec3,
) -> Option<Hit> {
    let mut best: Option<(f32, Hit)> = None;
    let mut consider = |t: Option<f32>, normal: Vec3, piece: Option<u64>| {
        if let Some(t) = t.filter(|&t| t <= MAX_RANGE) {
            if best.is_none_or(|(b, _)| t < b) {
                let normal = normal.normalize_or_zero();
                let normal = if normal.dot(direction) > 0.0 { -normal } else { normal };
                best = Some((t, Hit { point: origin + direction * t, normal, piece }));
            }
        }
    };
    for t in physics.world_triangles().iter().filter(|t| t.tag == 0) {
        let [a, b, c] = t.triangle.vertices.map(|v| Vec3::new(v.x, v.y, v.z));
        consider(ray_triangle(origin, direction, a, b, c), (b - a).cross(c - a), None);
    }
    for item in placed.items.iter().filter(|p| Some(p.id) != placed.dragging) {
        let at = item.world_transform(root);
        for q in item.piece.geometry().all_quads() {
            let [a, b, c, d] = q.map(|p| at.transform_point(p));
            let normal = (c - a).cross(b - a);
            consider(ray_triangle(origin, direction, a, b, c), normal, Some(item.id));
            consider(ray_triangle(origin, direction, a, c, d), normal, Some(item.id));
        }
    }
    best.map(|(_, hit)| hit)
}

/// Meshes and materials per piece shape, shared by placed pieces, the ghost
/// and the palette.
#[derive(Resource, Default)]
struct PieceAssets {
    meshes: HashMap<(PieceKind, u32), Handle<Mesh>>,
    solid: HashMap<PieceKind, Handle<StandardMaterial>>,
    ghost: Option<Handle<StandardMaterial>>,
}

impl PieceAssets {
    fn mesh(&mut self, piece: Piece, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
        self.meshes
            .entry((piece.kind, piece.size.to_bits()))
            .or_insert_with(|| meshes.add(piece_mesh(&piece.geometry())))
            .clone()
    }

    fn solid(&mut self, kind: PieceKind, materials: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
        self.solid
            .entry(kind)
            .or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: kind.color(),
                    perceptual_roughness: 0.85,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })
            })
            .clone()
    }
}

fn piece_mesh(geometry: &PieceGeometry) -> Mesh {
    let positions: Vec<[f32; 3]> = geometry
        .all_quads()
        .flat_map(|q| [0, 2, 1, 0, 3, 2].map(|i| q[i].to_array()))
        .collect();
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.compute_flat_normals();
    mesh
}

#[derive(Component)]
struct PlacedVisual(u64, Piece);
#[derive(Component)]
struct GhostVisual(Option<Piece>);
#[derive(Component)]
struct BuildRay;
#[derive(Component)]
struct BuildMarker;
#[derive(Component)]
struct PaletteIcon(usize);

fn spawn_visuals(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut assets: ResMut<PieceAssets>,
) {
    let unlit = |color: Color| StandardMaterial {
        base_color: color,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    assets.ghost = Some(materials.add(StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        ..unlit(Color::srgba(0.3, 1.0, 0.5, 0.45))
    }));
    commands.spawn((
        BuildRay,
        Mesh3d(meshes.add(Cylinder::new(0.04, 1.0))),
        MeshMaterial3d(materials.add(unlit(Color::srgb(1.0, 0.8, 0.2)))),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.spawn((
        BuildMarker,
        Mesh3d(meshes.add(Annulus::new(0.5, 0.8))),
        MeshMaterial3d(materials.add(unlit(Color::srgb(1.0, 0.35, 0.3)))),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.spawn((GhostVisual(None), Transform::default(), Visibility::Hidden));
    for (i, kind) in PieceKind::ALL.into_iter().enumerate() {
        let piece = Piece::new(kind);
        let mesh = assets.mesh(piece, &mut meshes);
        let material = assets.solid(kind, &mut materials);
        commands.spawn((
            PaletteIcon(i),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
}

/// One entity per placed piece, kept at its world pose.
fn sync_pieces(
    mut commands: Commands,
    placed: Res<PlacedPieces>,
    rig: Res<Rig>,
    mut assets: ResMut<PieceAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut visuals: Query<(Entity, &PlacedVisual, &mut Transform)>,
) {
    let Some(root) = rig.root_transform() else {
        return;
    };
    let mut shown = Vec::with_capacity(placed.items.len());
    for (entity, visual, mut transform) in &mut visuals {
        match placed.items.iter().find(|p| p.id == visual.0) {
            Some(item) if item.piece == visual.1 => {
                let target = item.world_transform(&root);
                if *transform != target {
                    *transform = target;
                }
                shown.push(item.id);
            }
            _ => commands.entity(entity).despawn(),
        }
    }
    for item in placed.items.iter().filter(|p| !shown.contains(&p.id)) {
        commands.spawn((
            PlacedVisual(item.id, item.piece),
            Mesh3d(assets.mesh(item.piece, &mut meshes)),
            MeshMaterial3d(assets.solid(item.piece.kind, &mut materials)),
            item.world_transform(&root),
        ));
    }
}

#[allow(clippy::type_complexity)]
fn show_build(
    mode: Res<BuildMode>,
    state: Res<BuildState>,
    placed: Res<PlacedPieces>,
    rig: Res<Rig>,
    hands: Query<(&Hand, &Transform), Without<PaletteIcon>>,
    mut assets: ResMut<PieceAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    mut ray: Query<(&mut Transform, &mut Visibility), (With<BuildRay>, Without<BuildMarker>, Without<GhostVisual>, Without<PaletteIcon>, Without<Hand>)>,
    mut marker: Query<(&mut Transform, &mut Visibility), (With<BuildMarker>, Without<BuildRay>, Without<GhostVisual>, Without<PaletteIcon>, Without<Hand>)>,
    mut ghost: Query<(Entity, &mut GhostVisual, &mut Transform, &mut Visibility), (Without<BuildRay>, Without<BuildMarker>, Without<PaletteIcon>, Without<Hand>)>,
    mut icons: Query<(&PaletteIcon, &mut Transform, &mut Visibility), (Without<BuildRay>, Without<BuildMarker>, Without<GhostVisual>, Without<Hand>)>,
) {
    let (Ok((mut ray_tf, mut ray_vis)), Ok((mut marker_tf, mut marker_vis))) =
        (ray.single_mut(), marker.single_mut())
    else {
        return;
    };
    let Ok((ghost_entity, mut ghost_piece, mut ghost_tf, mut ghost_vis)) = ghost.single_mut() else {
        return;
    };
    let root = rig.root_transform();
    // Ray.
    match state.ray.filter(|_| mode.active) {
        Some((start, end, _)) => {
            let length = start.distance(end).max(1e-3);
            *ray_tf = Transform {
                translation: (start + end) * 0.5,
                rotation: Quat::from_rotation_arc(Vec3::Y, (end - start) / length),
                scale: Vec3::new(1.0, length, 1.0),
            };
            *ray_vis = Visibility::Visible;
        }
        None => *ray_vis = Visibility::Hidden,
    }
    // A ring on the piece the ray is on: grip moves it, B deletes it.
    match state.target.and(state.ray).filter(|_| mode.active && placed.dragging.is_none()) {
        Some((_, end, _)) => {
            *marker_tf = Transform::from_translation(end + Vec3::Y * 0.05)
                .with_rotation(Quat::from_rotation_arc(Vec3::Z, Vec3::Y));
            *marker_vis = Visibility::Visible;
        }
        None => *marker_vis = Visibility::Hidden,
    }
    // Ghost of the piece to drop.
    let piece = state.piece();
    match state.ghost.zip(root).filter(|_| {
        mode.active && placed.dragging.is_none() && state.target.is_none()
    }) {
        Some(((_, point), root)) => {
            if ghost_piece.0 != Some(piece) {
                ghost_piece.0 = Some(piece);
                commands.entity(ghost_entity).insert((
                    Mesh3d(assets.mesh(piece, &mut meshes)),
                    MeshMaterial3d(assets.ghost.clone().unwrap_or_default()),
                ));
            }
            *ghost_tf = Transform {
                translation: point,
                rotation: root.rotation * Quat::from_rotation_y(state.placed_yaw),
                scale: Vec3::ONE,
            };
            *ghost_vis = Visibility::Visible;
        }
        None => *ghost_vis = Visibility::Hidden,
    }
    // Wrist palette above the left controller.
    let left = hands.iter().find(|(hand, _)| matches!(hand, Hand::Left)).map(|(_, t)| *t);
    for (icon, mut transform, mut visibility) in &mut icons {
        let (Some(root), Some(left), true) = (root, left, mode.active) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let kind = PieceKind::ALL[icon.0];
        let extent = icon_extent(kind);
        let selected = icon.0 == state.selected;
        let offset = Vec3::new(
            (icon.0 as f32 - (PieceKind::ALL.len() as f32 - 1.0) * 0.5) * ICON_SPACING_M,
            if selected { 0.075 } else { 0.06 },
            -0.02,
        );
        let scale = ICON_SIZE_M / extent * if selected { 1.4 } else { 1.0 };
        *transform = Transform {
            translation: root.transform_point(left.transform_point(offset)),
            rotation: root.rotation * left.rotation,
            scale: Vec3::splat(root.scale.x * scale),
        };
        *visibility = Visibility::Visible;
    }
}

/// Largest dimension of a default-size piece, game metres.
fn icon_extent(kind: PieceKind) -> f32 {
    let geometry = Piece::new(kind).geometry();
    let (min, max) = geometry
        .all_quads()
        .flatten()
        .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    (max - min).max_element().max(0.1)
}
