//! Build mode: put skate pieces on the real floor, tables and furniture.
//!
//! Hold the left menu button for a second (without chording it) to enter or
//! leave. While building the pad is neutral and the controllers do this:
//!
//! - left stick left/right: choose a piece (the wrist palette above the left
//!   controller shows them; the chosen one is raised)
//! - left stick up/down: scale the piece pointed at (or the next one to drop)
//!   as a whole; nothing else changes size (the skater's size and the whole
//!   park's are in the menu: Build a park)
//! - left stick click: drop a starter park where the right controller points
//! - left trigger: undo
//! - right controller: point; a ghost of the piece shows where it would go
//! - right trigger: drop it there
//! - right stick left/right: turn it in 15° steps (it lines up with a table
//!   edge when within 10°); up/down: resize it (its length or height).
//!   Pointing at a placed piece, the stick turns and resizes that piece
//! - right stick click on a placed piece: copy it (kind, size, scale, turn)
//! - right grip on a placed piece: pick it up, release to put it down
//! - B on a placed piece: delete it
//! - A on a placed piece: make it invisible (it is still skated; build mode
//!   shows it as a ghost) or visible again. A pointing at nothing shows the
//!   detected planes
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
use super::link::{ArWorld, SkaterLink, TeleportSkater};
use super::spawn::ray_triangle;
use crate::input::ExternalPad;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy_mod_openxr::openxr_session_running;
use serde::{Deserialize, Serialize};
use crate::xr::touch::XboxState;

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
/// Park scale: every placed piece is this many times its authored size.
/// Wide, because "skater size" changes it the other way to keep every piece
/// its real size while the skater grows or shrinks (world scale 1..60).
pub(crate) const MIN_PARK_SCALE: f32 = 0.05;
pub(crate) const MAX_PARK_SCALE: f32 = 60.0;
/// A grip counts as held for the park-scale chord past this pull.
const PARK_CHORD_GRIP: f32 = 0.6;
/// One piece's own uniform scale, on top of the park scale.
pub(crate) const MIN_PIECE_SCALE: f32 = 0.25;
pub(crate) const MAX_PIECE_SCALE: f32 = 4.0;
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
    /// This piece's uniform scale ([`MIN_PIECE_SCALE`]..[`MAX_PIECE_SCALE`]);
    /// absent in layouts from before 0.0.3.
    #[serde(default = "unit_scale")]
    pub scale: f32,
    /// Not drawn (outside build mode), still skated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

fn unit_scale() -> f32 {
    1.0
}

impl Placed {
    /// Pose of the piece in the game world; `park_scale` and its own scale
    /// grow it about its base centre, which stays on the same real spot.
    pub(crate) fn world_transform(&self, root: &Transform, park_scale: f32) -> Transform {
        Transform {
            translation: root.transform_point(Vec3::from_array(self.position)),
            rotation: root.rotation * Quat::from_rotation_y(self.yaw),
            scale: Vec3::splat(park_scale * self.scale),
        }
    }

    pub(crate) fn name(&self) -> String {
        format!("{:?} {}", self.piece.kind, self.id)
    }
}

/// Everything the player has placed. `revision` changes on every committed
/// edit (collision rebuild and save key off it); `dragging` is the piece
/// following the controller, `resizing` the one being resized and
/// `rescaling` the world scale being changed: collision waits until they end.
#[derive(Resource)]
pub(crate) struct PlacedPieces {
    pub items: Vec<Placed>,
    /// Size of every piece relative to its authored size (the skater keeps
    /// the world scale), [`MIN_PARK_SCALE`]..[`MAX_PARK_SCALE`].
    pub park_scale: f32,
    pub revision: u64,
    pub dragging: Option<u64>,
    pub resizing: Option<u64>,
    pub rescaling: bool,
    /// The piece last pointed at or placed in build mode (the menu edits it).
    pub selected: Option<u64>,
    /// The piece the build ray is on right now.
    pub pointed: Option<u64>,
    /// Draw invisible pieces as ghosts outside build mode too.
    pub show_hidden: bool,
    /// Earlier layouts, newest last, for undo.
    history: Vec<Vec<Placed>>,
}

impl Default for PlacedPieces {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            park_scale: 1.0,
            revision: 0,
            dragging: None,
            resizing: None,
            rescaling: false,
            selected: None,
            pointed: None,
            show_hidden: false,
            history: Vec::new(),
        }
    }
}

impl PlacedPieces {
    pub(crate) fn busy(&self) -> bool {
        self.dragging.is_some() || self.resizing.is_some() || self.rescaling
    }

    /// Remember the current layout before an edit.
    pub(crate) fn checkpoint(&mut self) {
        if self.history.len() == UNDO_DEPTH {
            self.history.remove(0);
        }
        self.history.push(self.items.clone());
    }

    pub(crate) fn undo(&mut self) -> bool {
        let Some(items) = self.history.pop() else {
            return false;
        };
        self.items = items;
        self.dragging = None;
        self.resizing = None;
        self.revision += 1;
        true
    }

    fn add(&mut self, piece: Piece, position: Vec3, yaw: f32, scale: f32) -> u64 {
        self.checkpoint();
        self.insert(piece, position, yaw, scale)
    }

    fn insert(&mut self, piece: Piece, position: Vec3, yaw: f32, scale: f32) -> u64 {
        let id = self.items.iter().map(|p| p.id).max().map_or(1, |m| m + 1);
        self.items.push(Placed {
            id,
            piece,
            position: position.to_array(),
            yaw,
            scale,
            hidden: false,
        });
        self.revision += 1;
        self.selected = Some(id);
        id
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Placed> {
        self.items.iter_mut().find(|p| p.id == id)
    }

    /// Every piece removed (undoable).
    pub(crate) fn clear(&mut self) {
        self.checkpoint();
        self.items.clear();
        self.dragging = None;
        self.resizing = None;
        self.selected = None;
        self.revision += 1;
    }

    pub(crate) fn remove(&mut self, id: u64) -> Option<Placed> {
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
    /// The rescale in progress is the park's, not the world's.
    park_rescale: bool,
    /// The park was resized: once collision is rebuilt, drop the skater onto
    /// whatever is now under this real (tracking) spot.
    drop_after_rebuild: Option<(Vec3, Vec3, u64)>,
    /// Unsnapped size (or scale) of the placed piece being resized.
    resize_size: f32,
    /// The resize in progress is the piece's uniform scale.
    resize_scale: bool,
    /// Uniform scale of the piece to drop.
    scale: f32,
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
            park_rescale: false,
            drop_after_rebuild: None,
            resize_size: 1.0,
            resize_scale: false,
            scale: 1.0,
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
    a: bool,
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
            .add_systems(Update, (sync_pieces, show_build, layout::save, drop_after_park_scale));
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
    // Menu + grips is the SK8 menu chord, not a build-mode hold.
    let chording = touch.x
        || touch.left_grip > 0.5
        || touch.right_grip > 0.5
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
    world: Res<ArWorld>,
    link: Res<SkaterLink>,
    mut teleport: MessageWriter<TeleportSkater>,
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
        placed.pointed = None;
        if placed.rescaling {
            finish_rescale(&rig, &mut teleport, &mut state, &mut placed);
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
        a: touch.a,
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
            placed.clear();
            info!("BUILD cleared");
        }
    } else {
        state.clear_hold = 0.0;
    }

    // (the skater's size and the whole park's are the menu's: Build a park; the sticks here only
    // ever change one piece)
    if placed.rescaling {
        finish_rescale(&rig, &mut teleport, &mut state, &mut placed);
    }

    let Some(root) = rig.root_transform() else {
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
    let hit = cast(&world, &placed, &root, origin, direction);
    let to_tracking = root.compute_affine().inverse();
    // A piece being resized keeps the stick until it is let go.
    state.target = placed.resizing.or(hit.and_then(|h| h.piece));
    placed.pointed = state.target;
    let valid = hit.filter(|h| h.normal.y >= MIN_UP);
    state.ray = Some(match hit {
        Some(h) => (origin, h.point, valid.is_some()),
        None => (origin, origin + direction * MISS_LENGTH, false),
    });
    state.ghost = valid.map(|h| (to_tracking.transform_point3(h.point), h.point));

    // Right stick: turn and resize (length / height) the piece pointed at, or else the ghost; left
    // stick up/down: scale that one piece as a whole.
    let [scale_x, scale_y] = touch.left_stick;
    let scaling = scale_y.abs() > STICK_DEADZONE && scale_y.abs() > scale_x.abs();
    let resize = if scaling { scale_y } else { touch.right_stick[1] };
    let resizing = scaling || (resize.abs() > STICK_DEADZONE && resize.abs() > touch.right_stick[0].abs());
    let turn = (now.right_flick != 0 && now.right_flick != previous.right_flick)
        .then(|| -ROTATE_STEP * now.right_flick as f32);
    let factor = (resize * RESIZE_RATE * time.delta_secs()).exp();
    if let Some(id) = state.target {
        placed.selected = Some(id);
    }
    match state.target.filter(|_| placed.dragging.is_none()) {
        Some(id) => {
            if let Some(turn) = turn {
                placed.checkpoint();
                if let Some(item) = placed.get_mut(id) {
                    item.yaw += turn;
                }
                placed.revision += 1;
                info!("BUILD turned piece {id}");
            }
            if resizing {
                if placed.resizing.is_none() {
                    placed.checkpoint();
                    placed.resizing = Some(id);
                    state.resize_scale = scaling;
                    let item = placed.items.iter().find(|p| p.id == id);
                    state.resize_size = item.map_or(1.0, |p| if scaling { p.scale } else { p.piece.size });
                }
                let scale_mode = state.resize_scale;
                let (lo, hi) = if scale_mode { (MIN_PIECE_SCALE, MAX_PIECE_SCALE) } else { (MIN_SIZE, MAX_SIZE) };
                state.resize_size = (state.resize_size * factor).clamp(lo, hi);
                let value = state.resize_size;
                if let Some(item) = placed.get_mut(id) {
                    if scale_mode {
                        item.scale = snap_scale(value);
                    } else {
                        item.piece.size = snap_size(value);
                    }
                }
            }
        }
        None => {
            if let Some(turn) = turn {
                state.yaw += turn;
            }
            if resizing && scaling {
                state.scale = (state.scale * factor).clamp(MIN_PIECE_SCALE, MAX_PIECE_SCALE);
            } else if resizing {
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
            state.scale = item.scale;
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
    if pressed(now.a, previous.a) {
        if let Some(id) = state.target {
            placed.checkpoint();
            if let Some(item) = placed.get_mut(id) {
                item.hidden = !item.hidden;
                info!("BUILD piece {id} hidden={}", item.hidden);
            }
            placed.revision += 1;
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
                let at = position + Quat::from_rotation_y(yaw) * Vec3::from_array(offset) * placed.park_scale / rig.scale;
                placed.insert(Piece { kind, size }, at, yaw + turn, 1.0);
            }
            info!("BUILD starter park at {position:?} ({} pieces)", placed.items.len());
        }
        return;
    }
    if pressed(now.trigger, previous.trigger) {
        if let Some((position, _)) = state.ghost {
            let piece = state.piece();
            let yaw = state.placed_yaw;
            let scale = snap_scale(state.scale);
            let id = placed.add(piece, position, yaw, scale);
            info!("BUILD placed {:?} size {:.2} scale {scale:.2} as piece {id} at {position:?}", piece.kind, piece.size);
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

pub(crate) fn snap_scale(scale: f32) -> f32 {
    ((scale / SIZE_STEP).round() * SIZE_STEP).clamp(MIN_PIECE_SCALE, MAX_PIECE_SCALE)
}

/// The scale stick was let go: put the skater back on the same real spot at
/// the new scale, and rebuild collision (and save) for it. After a park
/// rescale the skater is dropped onto whatever is under it once the new
/// collision exists ([`drop_after_park_scale`]).
fn finish_rescale(
    rig: &Rig,
    teleport: &mut MessageWriter<TeleportSkater>,
    state: &mut BuildState,
    placed: &mut PlacedPieces,
) {
    placed.rescaling = false;
    placed.revision += 1;
    let park = std::mem::take(&mut state.park_rescale);
    let Some((deck, forward)) = state.rescale_from.take() else {
        return;
    };
    if park {
        info!("BUILD park scale {:.2}", placed.park_scale);
        state.drop_after_rebuild = Some((deck, forward, placed.revision));
        return;
    }
    info!("BUILD world scale {:.2} (game metres per real metre)", rig.scale);
    let Some(root) = rig.root_transform() else {
        return;
    };
    teleport.write(TeleportSkater {
        point: root.transform_point(deck),
        forward: root.rotation * forward,
    });
}

/// Height above the skater's spot to search down from after a park rescale,
/// game metres.
const DROP_SEARCH: f32 = 200.0;

fn drop_after_park_scale(
    rig: Res<Rig>,
    world: Res<ArWorld>,
    placed: Res<PlacedPieces>,
    mut state: ResMut<BuildState>,
    mut teleport: MessageWriter<TeleportSkater>,
    mut seen: Local<u64>,
) {
    // Wait for the rebuild that follows the rescale's revision.
    let Some((deck, forward, revision)) = state.drop_after_rebuild else {
        *seen = world.revision;
        return;
    };
    if placed.revision != revision || world.revision == *seen {
        return;
    }
    state.drop_after_rebuild = None;
    let Some(root) = rig.root_transform() else {
        return;
    };
    let point = root.transform_point(deck);
    let from = point + Vec3::Y * DROP_SEARCH;
    let top = world
        .all_triangles()
        .filter_map(|[a, b, c]| ray_triangle(from, Vec3::NEG_Y, a, b, c))
        .min_by(f32::total_cmp)
        .map_or(point, |t| from - Vec3::Y * t);
    teleport.write(TeleportSkater {
        point: top,
        forward: root.rotation * forward,
    });
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
    world: &ArWorld,
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
    for [a, b, c] in world.room() {
        consider(ray_triangle(origin, direction, a, b, c), (b - a).cross(c - a), None);
    }
    for item in placed.items.iter().filter(|p| Some(p.id) != placed.dragging) {
        let at = item.world_transform(root, placed.park_scale);
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
    /// Invisible pieces, when shown.
    hidden: Option<Handle<StandardMaterial>>,
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

/// A placed piece's entity: its id, shape and whether it is drawn as a ghost.
#[derive(Component)]
struct PlacedVisual(u64, Piece, bool);
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
    assets.hidden = Some(materials.add(StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        ..unlit(Color::srgba(0.75, 0.85, 1.0, 0.22))
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

/// One entity per placed piece, kept at its world pose. Invisible pieces are
/// ghosts in build mode (or with "show invisible pieces") and not drawn
/// otherwise.
#[allow(clippy::too_many_arguments)]
fn sync_pieces(
    mut commands: Commands,
    placed: Res<PlacedPieces>,
    mode: Res<BuildMode>,
    rig: Res<Rig>,
    mut assets: ResMut<PieceAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut visuals: Query<(Entity, &PlacedVisual, &mut Transform, &mut Visibility)>,
) {
    let Some(root) = rig.root_transform() else {
        return;
    };
    let see_hidden = mode.active || placed.show_hidden;
    let mut shown = Vec::with_capacity(placed.items.len());
    for (entity, visual, mut transform, mut visibility) in &mut visuals {
        match placed.items.iter().find(|p| p.id == visual.0) {
            Some(item) if item.piece == visual.1 && item.hidden == visual.2 => {
                let target = item.world_transform(&root, placed.park_scale);
                if *transform != target {
                    *transform = target;
                }
                visibility.set_if_neq(if item.hidden && !see_hidden { Visibility::Hidden } else { Visibility::Inherited });
                shown.push(item.id);
            }
            _ => commands.entity(entity).despawn(),
        }
    }
    for item in placed.items.iter().filter(|p| !shown.contains(&p.id)) {
        let material = if item.hidden {
            assets.hidden.clone().unwrap_or_default()
        } else {
            assets.solid(item.piece.kind, &mut materials)
        };
        commands.spawn((
            PlacedVisual(item.id, item.piece, item.hidden),
            Mesh3d(assets.mesh(item.piece, &mut meshes)),
            MeshMaterial3d(material),
            item.world_transform(&root, placed.park_scale),
            if item.hidden && !see_hidden { Visibility::Hidden } else { Visibility::Inherited },
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
                scale: Vec3::splat(placed.park_scale * snap_scale(state.scale)),
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
