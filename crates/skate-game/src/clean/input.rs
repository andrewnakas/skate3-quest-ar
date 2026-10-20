//! Controls for the clean-room engine.
//!
//! Quest Touch (skating):
//! - left stick: steer; in the air, spin; in a manual, balance (up/down)
//! - right stick: hold down to crouch, flick to pop and flip (see How to play); part-way down /
//!   up and held: manual / nose manual (how far balances it)
//! - A: push (hold to keep pushing), B: mongo push, Y: brake
//! - X held while steering: powerslide
//! - triggers in the air: grab (left / right hand)
//! - right grip: manual, left grip: nose manual
//! - left menu: tap for the menu, hold for build mode (see `xr::build`)
//!
//! Desktop (`--clean` without a headset): A/D steer, W/S balance, Space push,
//! Left Alt mongo push, Left Shift brake, arrow keys = right stick, Q/E grabs, Z manual, X nose
//! manual, C powerslide; or any gamepad (left stick, right stick, South push, East mongo push,
//! North brake, West powerslide, triggers grab, bumpers manual).
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
        b |= buttons::PUSH_MONGO;
    }
    if touch.y {
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

/// `SK8_DEMO=grabs`: push, then an ollie into each of the fifteen grab styles in turn (trigger(s)
/// plus the left stick's direction at the start of the grab).
fn demo_grabs(t: f32) -> Input {
    const START: f32 = 3.0;
    const EACH: f32 = 2.6;
    let mut i = Input::default();
    if t < START {
        i.buttons |= buttons::PUSH;
        return i;
    }
    let n = ((t - START) / EACH) as usize % 15;
    let d = (t - START) % EACH;
    // A short push between airs keeps the speed up.
    if d > 2.0 {
        i.buttons |= buttons::PUSH;
    }
    if d < 0.35 {
        i.flick = [0.0, -1.0];
    } else if d < 0.4 {
        i.flick = [0.0, 0.0];
    } else if d < 0.5 {
        i.flick = [0.0, 1.0];
    }
    // twelve hand x direction styles, then the stick-left ones (Christ air, method, rocket)
    let (hand, dir) = if n < 12 { (n / 4, n % 4) } else { (n - 12, 4) };
    if (0.55..1.15).contains(&d) {
        i.buttons |= [buttons::GRAB_RIGHT, buttons::GRAB_LEFT, buttons::GRAB_LEFT | buttons::GRAB_RIGHT][hand];
        if d < 0.58 {
            i.steer = [[0.0, 0.0], [0.0, 1.0], [0.0, -1.0], [1.0, 0.0], [-1.0, 0.0]][dir];
        }
    }
    i
}

/// `SK8_DEMO=flips`: every trick in the engine's table in turn, using its own sample stick path
/// (the crouch held a third of a second), with a push between them.
fn demo_flips(t: f32) -> Input {
    const START: f32 = 2.0;
    const EACH: f32 = 3.0;
    const HOLD: usize = 20;
    let mut i = Input::default();
    if t < START {
        i.buttons |= buttons::PUSH;
        return i;
    }
    let names = skate_lite_sys::trick_names();
    let n = ((t - START) / EACH) as usize % names.len().max(1);
    let d = (t - START) % EACH;
    if d > 2.2 {
        i.buttons |= buttons::PUSH;
    }
    let path = skate_lite_sys::trick_demo_path(n);
    let k = (d * 60.0) as usize;
    let k = if k < 4 { k } else if k < 4 + HOLD { 4 } else { k - HOLD + 1 };
    if let Some(p) = path.get(k) {
        i.flick = *p;
    }
    i
}

/// `SK8_DEMO=push`: from a standstill, pushing (step off, chained strokes), a coast and the step
/// back on, a single tap, then a pop straight out of a stroke; regular in even slots, mongo in odd.
const PUSH_EACH: f32 = 9.0;
fn demo_push(t: f32) -> Input {
    let d = t % PUSH_EACH;
    let mut i = Input::default();
    if d < 3.6 || (5.0..5.05).contains(&d) || (6.8..7.6).contains(&d) {
        i.buttons |= buttons::PUSH;
    }
    // an ollie fired mid-stroke
    if (7.3..7.6).contains(&d) {
        i.flick = [0.0, -1.0];
    } else if (7.6..7.7).contains(&d) {
        i.flick = [0.0, 1.0];
    }
    i
}

/// `SK8_DEMO=manual`: rolling, a manual (right stick part-way down), a nose manual (part-way up),
/// then an ollie landed with the stick already part-way down (straight into a manual).
const MANUAL_EACH: f32 = 7.0;
fn demo_manual(t: f32) -> Input {
    let d = t % MANUAL_EACH;
    let mut i = Input::default();
    if (0.4..2.2).contains(&d) {
        i.flick = [0.0, -0.55];
    } else if (2.8..4.6).contains(&d) {
        i.flick = [0.0, 0.55];
    } else if (5.0..5.3).contains(&d) {
        i.flick = [0.0, -1.0];
    } else if (5.3..5.4).contains(&d) {
        i.flick = [0.0, 1.0];
    } else if (5.75..6.9).contains(&d) {
        i.flick = [0.0, -0.55];
    }
    i
}

/// `SK8_DEMO=1`: a scripted run through the move set (looping), for checking animation by eye or
/// with `SK8_ANIM_TRACE` without hand-driving the game.
fn demo_input(t: f32) -> Input {
    const LOOP: f32 = 26.0;
    let t = t % LOOP;
    let mut i = Input::default();
    let (down, centre, up) = ([0.0, -1.0], [0.0, 0.0], [0.0, 1.0]);
    let (up_left, up_right, down_left, left) = ([-0.7, 0.7], [0.7, 0.7], [-0.7, -0.7], [-1.0, 0.0]);
    // A flick starting at `at`: a crouch, then each token held for three engine steps.
    let flick = |at: f32, crouch: f32, tokens: &[[f32; 2]]| -> Option<[f32; 2]> {
        let d = t - at;
        if !(0.0..crouch + tokens.len() as f32 * 0.05).contains(&d) {
            return None;
        }
        if d < crouch { Some(down) } else { Some(tokens[((d - crouch) / 0.05) as usize]) }
    };
    let flicks = [
        flick(6.0, 0.35, &[centre, up]),
        flick(8.5, 0.35, &[centre, up_left]),
        flick(11.0, 0.35, &[centre, up_right]),
        flick(13.55, 0.3, &[down_left, left, centre]),
        flick(16.5, 0.35, &[centre, up]),
    ];
    if let Some(f) = flicks.into_iter().flatten().next() {
        i.flick = f;
    }
    let push = t < 3.0 || (14.8..16.0).contains(&t) || t > 23.45;
    if push {
        i.buttons |= buttons::PUSH;
    }
    if (3.0..4.5).contains(&t) || (20.5..22.5).contains(&t) {
        i.steer[0] = 1.0;
    } else if (4.5..6.0).contains(&t) {
        i.steer[0] = -1.0;
    }
    if (16.95..17.6).contains(&t) {
        i.buttons |= buttons::GRAB_RIGHT;
    }
    // a manual on the right stick, part-way down
    if (18.5..20.0).contains(&t) {
        i.flick = [0.0, -0.55];
    }
    if (20.5..22.5).contains(&t) {
        i.buttons |= buttons::SLIDE;
    }
    i
}

/// Log controllers as they connect, so "my pad does nothing" can be told apart from "not seen".
pub(crate) fn log_gamepads(pads: Query<(Entity, Option<&Name>), Added<Gamepad>>) {
    for (e, name) in &pads {
        info!("SK8 gamepad connected: {} ({e:?})", name.map_or("unnamed", |n| n.as_str()));
    }
}

/// The slot (one trick) a slotted demo is in, so each starts again at the spawn.
fn demo_slot(t: f32) -> Option<usize> {
    match std::env::var("SK8_DEMO").as_deref() {
        Ok("flips") if t >= 2.0 => Some(((t - 2.0) / 3.0) as usize),
        Ok("grabs") if t >= 3.0 => Some(((t - 3.0) / 2.6) as usize),
        Ok("push") => Some((t / PUSH_EACH) as usize),
        Ok("manual") => Some((t / MANUAL_EACH) as usize),
        _ => None,
    }
}

pub(crate) fn read_desktop(
    time: Res<Time<Real>>,
    mut demo: Local<Option<f32>>,
    mut demo_at: Local<Option<usize>>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    ui: Res<UiCapture>,
    mut sim: ResMut<CleanSkate>,
) {
    if ui.0 {
        sim.input = Input::default();
        return;
    }
    if demo.is_none() && std::env::var_os("SK8_DEMO").is_some() {
        // SK8_DEMO_FROM=<s>: start the script part-way through (e.g. at one slot)
        *demo = Some(std::env::var("SK8_DEMO_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0));
        // Face away from the test park: open floor, so no run ends in a box.
        sim.place(Vec3::ZERO, std::f32::consts::PI);
    }
    if let Some(t) = demo.as_mut() {
        if !sim.paused {
            *t += time.delta_secs().min(0.1) * sim.time_scale;
        }
        let slot = demo_slot(*t);
        if slot.is_some() && slot != *demo_at {
            *demo_at = slot;
            if std::env::var("SK8_DEMO").as_deref() == Ok("push") {
                // From a standstill, mongo every other slot.
                sim.place_rolling(Vec3::ZERO, std::f32::consts::PI, 0.0);
                if let Some(p) = skate_lite_sys::params().iter().find(|p| p.key == "push_mongo") {
                    sim.rider.set_param(p.index, (slot.unwrap_or(0) % 2) as f32);
                }
            } else {
                // Back to the spawn, rolling, for the next trick (open floor ahead every time).
                sim.place_rolling(Vec3::ZERO, std::f32::consts::PI, 4.0);
            }
        }
        sim.input = match std::env::var("SK8_DEMO").as_deref() {
            Ok("grabs") => demo_grabs(*t),
            Ok("flips") => demo_flips(*t),
            Ok("push") => demo_push(*t),
            Ok("manual") => demo_manual(*t),
            _ => demo_input(*t),
        };
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
        (KeyCode::AltLeft, buttons::PUSH_MONGO),
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
            (GamepadButton::East, buttons::PUSH_MONGO),
            (GamepadButton::North, buttons::BRAKE),
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
