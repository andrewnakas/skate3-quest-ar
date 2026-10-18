//! The clean tier's menus in XR: Touch navigation (a tap of the left menu
//! button opens the start screen, menu + both grips the SK8 menu), the panel
//! that shows them, and the requests they make of the XR layer.
use super::menu::{Menu, MenuNav, Nav, Repeat};
use super::sim::CleanSkate;
use crate::xr::actions::LatestTouch;
use crate::xr::build::{BuildMode, PlacedPieces};
use crate::xr::link::{ArWorld, TeleportSkater};
use crate::xr::panel::XrPanel;
use crate::xr::rig::Rig;
use crate::xr::spawn::ray_triangle;
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;

/// A menu press shorter than this (with nothing else held) is a tap.
const TAP_SECONDS: f32 = 0.35;
const GRIP: f32 = 0.6;
const STICK: f32 = 0.6;
/// Where "put the skater in front of me" aims, real metres ahead of the head.
const AHEAD_M: f32 = 0.6;

#[derive(Default)]
pub(crate) struct TouchNavState {
    held: Option<(f32, bool)>,
    previous: crate::xr::touch::TouchState,
    repeat: Repeat,
}

pub(crate) fn touch_nav(
    touch: Res<LatestTouch>,
    time: Res<Time<Real>>,
    menu: Res<Menu>,
    mut state: Local<TouchNavState>,
    mut nav: ResMut<MenuNav>,
) {
    let t = touch.0;
    let p = state.previous;
    let dt = time.delta_secs();
    let mut n = Nav::default();
    // Menu button: tap = start screen, + both grips = SK8 menu.
    let grips = t.left_grip > GRIP && t.right_grip > GRIP;
    let chorded = |t: &crate::xr::touch::TouchState| {
        t.x || t.left_grip > 0.5 || t.right_grip > 0.5 || t.left_stick[0].abs() > 0.3 || t.left_stick[1].abs() > 0.3
    };
    state.held = match (state.held, t.menu) {
        (None, true) => Some((0.0, chorded(&t))),
        (Some((s, c)), true) => {
            if grips && !c {
                n.toggle_sk8 = true;
            }
            Some((s + dt, c || chorded(&t)))
        }
        (Some((s, c)), false) => {
            if !c && s < TAP_SECONDS {
                n.toggle_start = true;
            }
            None
        }
        (None, false) => None,
    };
    if menu.open && !t.menu {
        let y = if t.right_stick[1].abs() > t.left_stick[1].abs() { t.right_stick[1] } else { t.left_stick[1] };
        let x = if t.right_stick[0].abs() > t.left_stick[0].abs() { t.right_stick[0] } else { t.left_stick[0] };
        let [up, down, left, right] = state.repeat.directions(y > STICK, y < -STICK, x < -STICK, x > STICK, dt);
        n.up = up;
        n.down = down;
        n.left = left;
        n.right = right;
        n.select = t.a && !p.a;
        n.back = t.b && !p.b;
        n.alt = t.x && !p.x;
        n.tab_prev = t.left_grip > GRIP && p.left_grip <= GRIP;
        n.tab_next = t.right_grip > GRIP && p.right_grip <= GRIP;
        n.fine = t.left_trigger > 0.5;
        n.coarse = t.right_trigger > 0.5;
    }
    state.previous = t;
    nav.0 = n;
}

/// Show the panel with the menu, and do what the menu asked of the XR layer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    mut menu: ResMut<Menu>,
    mut panel: ResMut<XrPanel>,
    mut rig: ResMut<Rig>,
    mut placed: ResMut<PlacedPieces>,
    mut build: ResMut<BuildMode>,
    world: Res<ArWorld>,
    sim: Res<CleanSkate>,
    eyes: Query<&Transform, With<XrCamera>>,
    mut teleport: MessageWriter<TeleportSkater>,
    mut was_open: Local<bool>,
) {
    if menu.open && !*was_open {
        panel.recenter = true;
        // The menu and build mode both own the controllers.
        build.active = false;
    }
    *was_open = menu.open;
    panel.visible = menu.open;
    menu.park_scale = placed.park_scale;
    menu.world_scale = rig.scale;
    let requests = std::mem::take(&mut menu.xr_requests);
    if let Some(v) = requests.park_scale {
        placed.park_scale = v;
        placed.revision += 1;
    }
    if let Some(v) = requests.world_scale {
        rig.scale = v;
        placed.revision += 1; // saves the layout (it carries the scale)
    }
    if requests.build_mode {
        build.active = true;
    }
    if requests.recenter {
        let (Some(root), Some(eye)) = (rig.root_transform(), eyes.iter().next()) else { return };
        let forward = (eye.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
        let ahead = root.transform_point(eye.translation + forward * AHEAD_M);
        let from = ahead + Vec3::Y * root.scale.y * 0.5;
        let ground = world
            .all_triangles()
            .filter(|[a, b, c]| (*b - *a).cross(*c - *a).normalize_or_zero().y.abs() > 0.6)
            .filter_map(|[a, b, c]| ray_triangle(from, Vec3::NEG_Y, a, b, c))
            .min_by(f32::total_cmp)
            .map(|t| from - Vec3::Y * t);
        let point = ground.unwrap_or(Vec3::from_array(sim.spawn.0));
        teleport.write(TeleportSkater {
            point,
            forward: root.rotation * forward,
        });
    }
}
