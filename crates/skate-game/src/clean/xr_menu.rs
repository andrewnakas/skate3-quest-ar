//! The clean tier's menus in XR. The headset menu (`xr::ui`, shared with the
//! retail tier) is the start screen; this tier adds its skater page
//! ([`super::skater_page`]) and lends the panel to its full SK8 sheet
//! ("Advanced tuning", or menu + both grips). This module drives that sheet
//! with Touch, hands the panel back and forth, carries out the requests the
//! sheet makes of the XR layer, and feeds the corner readout.
use super::UiCapture;
use super::menu::{HudLines, Menu, MenuNav, Nav, Page, Repeat};
use super::skater_page::SK8_SHEET;
use crate::xr::actions::LatestTouch;
use crate::xr::build::{BuildMode, PlacedPieces};
use crate::xr::rig::Rig;
use crate::xr::ui::hud::TrickHud;
use crate::xr::ui::{UiOwnsControllers, XrUi, pages};
use bevy::prelude::*;

const GRIP: f32 = 0.6;
const STICK: f32 = 0.6;

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
    // Menu + both grips = SK8 sheet (a tap is the headset menu's, `xr::ui`).
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
        (Some(_), false) => None,
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

/// Lend the panel to the SK8 sheet and take it back, and do what the sheet
/// asked of the XR layer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    mut menu: ResMut<Menu>,
    mut ui: ResMut<XrUi>,
    owns: Res<UiOwnsControllers>,
    mut capture: ResMut<UiCapture>,
    mut rig: ResMut<Rig>,
    mut placed: ResMut<PlacedPieces>,
    mut build: ResMut<BuildMode>,
    mut lent: Local<bool>,
) {
    // The sheet is the only page of this menu shown in the headset: its start
    // and How to play pages are the headset menu's.
    if *lent && ui.external.is_none() {
        // The headset menu took the panel back (a menu tap closes both).
        menu.open = false;
    }
    if ui.external == Some(SK8_SHEET) && !*lent && !menu.open {
        menu.open_sk8();
    }
    if menu.open && menu.page != Page::Sk8 {
        // B out of the sheet: back to the headset menu's start page.
        menu.open = false;
        if *lent {
            ui.request = Some(pages::START);
        }
    }
    if menu.open {
        ui.external = Some(SK8_SHEET);
        ui.open = false;
        build.active = false;
    } else if ui.external == Some(SK8_SHEET) && *lent {
        ui.external = None;
    }
    *lent = menu.open;
    capture.0 = menu.open || owns.0;
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
        ui.request_recenter = true;
    }
}

/// The clean tier's trick readout, to the headset's corner readout.
pub(crate) fn feed_hud(lines: Res<HudLines>, mut hud: ResMut<TrickHud>) {
    let next = TrickHud {
        name: lines.name.clone(),
        points: lines.points.clone(),
        line: lines.line.clone(),
    };
    if *hud != next {
        *hud = next;
    }
}
