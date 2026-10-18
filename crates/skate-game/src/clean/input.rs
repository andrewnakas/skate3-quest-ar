//! Controls for the clean-room engine.
//!
//! Quest Touch (skating):
//! - left stick: steer; in the air, spin; in a manual, balance (up/down)
//! - right stick: hold down to crouch, flick to pop and flip (see How to play)
//! - A: push (hold to keep pushing), B: brake
//! - X held while steering: powerslide
//! - triggers in the air: grab (left / right hand)
//! - right grip: manual, left grip: nose manual
//! - left menu: tap for the menu, hold for build mode (see `xr::build`)
//!
//! Desktop (`--clean` without a headset): A/D steer, W/S balance, Space push,
//! Left Shift brake, arrow keys = right stick, Q/E grabs, Z manual, X nose
//! manual, C powerslide; or any gamepad (left stick, right stick, South push,
//! East brake, West powerslide, triggers grab, bumpers manual).
use super::sim::CleanSkate;
use super::UiCapture;
use bevy::prelude::*;
use skate_lite_sys::{Input, buttons};

#[cfg(feature = "xr")]
const PRESSED: f32 = 0.5;

#[cfg(feature = "xr")]
pub(crate) fn from_touch(touch: &crate::xr::touch::TouchState) -> Input {
    let mut b = 0;
    if touch.a {
        b |= buttons::PUSH;
    }
    if touch.b {
        b |= buttons::BRAKE;
    }
    if touch.x {
        b |= buttons::SLIDE;
    }
    if touch.left_trigger > PRESSED {
        b |= buttons::GRAB_LEFT;
    }
    if touch.right_trigger > PRESSED {
        b |= buttons::GRAB_RIGHT;
    }
    if touch.right_grip > PRESSED {
        b |= buttons::MANUAL;
    }
    if touch.left_grip > PRESSED {
        b |= buttons::NOSE_MANUAL;
    }
    Input {
        steer: touch.left_stick,
        flick: touch.right_stick,
        buttons: b,
    }
}

/// Both left grip and trigger squeezed: the spawn aim owns the controls.
#[cfg(feature = "xr")]
fn spawn_aiming(touch: &crate::xr::touch::TouchState) -> bool {
    touch.left_grip >= 0.7 && touch.left_trigger >= 0.7
}

#[cfg(feature = "xr")]
pub(crate) fn read_touch(
    touch: Res<crate::xr::actions::LatestTouch>,
    build: Res<crate::xr::build::BuildMode>,
    ui: Res<UiCapture>,
    mut sim: ResMut<CleanSkate>,
) {
    let t = &touch.0;
    sim.input = if build.active || ui.0 || t.menu || spawn_aiming(t) {
        Input::default()
    } else {
        from_touch(t)
    };
}

pub(crate) fn read_desktop(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    ui: Res<UiCapture>,
    mut sim: ResMut<CleanSkate>,
) {
    if ui.0 {
        sim.input = Input::default();
        return;
    }
    let key = |k| if keys.pressed(k) { 1.0 } else { 0.0 };
    let mut input = Input {
        steer: [
            key(KeyCode::KeyD) - key(KeyCode::KeyA),
            key(KeyCode::KeyW) - key(KeyCode::KeyS),
        ],
        flick: [
            key(KeyCode::ArrowRight) - key(KeyCode::ArrowLeft),
            key(KeyCode::ArrowUp) - key(KeyCode::ArrowDown),
        ],
        buttons: 0,
    };
    for (k, bit) in [
        (KeyCode::Space, buttons::PUSH),
        (KeyCode::ShiftLeft, buttons::BRAKE),
        (KeyCode::KeyQ, buttons::GRAB_LEFT),
        (KeyCode::KeyE, buttons::GRAB_RIGHT),
        (KeyCode::KeyZ, buttons::MANUAL),
        (KeyCode::KeyX, buttons::NOSE_MANUAL),
        (KeyCode::KeyC, buttons::SLIDE),
    ] {
        if keys.pressed(k) {
            input.buttons |= bit;
        }
    }
    // Arrow keys: a diagonal is two keys; normalise so it reaches the rim.
    let f = Vec2::from_array(input.flick);
    if f.length() > 1.0 {
        input.flick = f.normalize().to_array();
    }
    for pad in &pads {
        let l = pad.left_stick();
        let r = pad.right_stick();
        if l.length() > 0.15 {
            input.steer = l.to_array();
        }
        if r.length() > 0.15 {
            input.flick = r.to_array();
        }
        for (b, bit) in [
            (GamepadButton::South, buttons::PUSH),
            (GamepadButton::East, buttons::BRAKE),
            (GamepadButton::West, buttons::SLIDE),
            (GamepadButton::LeftTrigger2, buttons::GRAB_LEFT),
            (GamepadButton::RightTrigger2, buttons::GRAB_RIGHT),
            (GamepadButton::RightTrigger, buttons::MANUAL),
            (GamepadButton::LeftTrigger, buttons::NOSE_MANUAL),
        ] {
            if pad.pressed(b) {
                input.buttons |= bit;
            }
        }
    }
    sim.input = input;
}
