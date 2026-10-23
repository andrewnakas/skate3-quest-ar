//! Controls for the clean-room engine.
//!
//! Quest Touch (skating):
//! - left stick: steer; in the air, spin; in a manual, balance (up/down)
//! - right stick: hold down to crouch, flick to pop and flip (see How to play); part-way down /
//!   up and held: manual / nose manual (how far balances it)
//! - A: push (hold to keep pushing), X: mongo push, B: brake, Y: get off / on the board (walk)
//! - left stick click held while flicking: old school (no comply, boneless, no comply 180)
//! - right grip (pad: RB) held while steering: powerslide; held into a landing: revert
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
    // the port's layout: A push, X mongo push, B brake, Y off the board
    if touch.x {
        b |= buttons::PUSH_MONGO;
    }
    if touch.b {
        b |= buttons::BRAKE;
    }
    if touch.y {
        b |= buttons::OFF_BOARD;
    }
    if touch.left_stick_click {
        b |= buttons::FOOT_OFF;
    }
    if touch.left_trigger > PRESSED {
        b |= buttons::GRAB_LEFT;
    }
    if touch.right_trigger > PRESSED {
        b |= buttons::GRAB_RIGHT;
    }
    if touch.right_grip > PRESSED {
        b |= buttons::SLIDE;
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

/// `SK8_DEMO=grabs`: push, then an ollie into each of the twenty-one grab styles in turn, the port's
/// way: the right stick held first, then the trigger(s); the last four tweaked (trigger first, then
/// the stick pushed out while held).
fn demo_grabs(t: f32) -> Input {
    const START: f32 = 3.0;
    const EACH: f32 = 2.6;
    const L: u32 = buttons::GRAB_LEFT;
    const R: u32 = buttons::GRAB_RIGHT;
    // (triggers, right stick held before them, right stick pushed out while held: a tweak)
    const GRABS: [(u32, [f32; 2], [f32; 2]); 21] = [
        (R, [0.0, 0.0], [0.0, 0.0]), // indy
        (L, [0.0, -1.0], [0.0, 0.0]), // crail
        (L, [0.0, 1.0], [0.0, 0.0]), // tail
        (L, [0.7, -0.7], [0.0, 0.0]), // stalefish
        (L, [0.0, 0.0], [0.0, 0.0]), // melon
        (R, [0.0, -1.0], [0.0, 0.0]), // nose
        (R, [-1.0, 0.0], [0.0, 0.0]), // tuck knee
        (L, [-0.7, 0.7], [0.0, 0.0]), // judo
        (L | R, [0.0, 0.0], [0.0, 0.0]), // double
        (L | R, [-0.7, 0.7], [0.0, 0.0]), // superman
        (L | R, [-0.7, -0.7], [0.0, 0.0]), // no foot
        (L, [-1.0, 0.0], [0.0, 0.0]), // stiffy
        (R, [-0.7, 0.7], [0.0, 0.0]), // christ air
        (R, [0.7, -0.7], [0.0, 0.0]), // method
        (L | R, [0.0, -1.0], [0.0, 0.0]), // rocket air
        (R, [0.0, 1.0], [0.0, 0.0]), // seatbelt
        (R, [1.0, 0.0], [0.0, 0.0]), // mute
        (L, [0.0, 0.0], [0.0, 1.0]), // nosebone (melon tweaked up)
        (L, [0.0, 0.0], [0.0, -1.0]), // tailbone (melon tweaked down)
        (R, [0.0, 0.0], [0.0, -1.0]), // japan (indy tweaked down)
        (R, [0.0, 0.0], [-1.0, 0.0]), // cross bone (indy tweaked left)
    ];
    let mut i = Input::default();
    if t < START {
        i.buttons |= buttons::PUSH;
        return i;
    }
    let n = ((t - START) / EACH) as usize % GRABS.len();
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
    let (hands, stick, tweak) = GRABS[n];
    if (0.56..0.7).contains(&d) {
        i.flick = stick;
    } else if (0.8..1.1).contains(&d) {
        i.flick = tweak;
    }
    if (0.62..1.15).contains(&d) {
        i.buttons |= hands;
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

/// `SK8_DEMO=oldschool`: the foot-off button held through each plain gesture in turn (ollie,
/// nollie, pop shove-it, frontside shove-it): no comply, boneless and the two no comply 180s.
const OLD_EACH: f32 = 3.0;
fn demo_oldschool(t: f32) -> Input {
    let d = t % OLD_EACH;
    let n = (t / OLD_EACH) as usize % 5;
    let mut i = Input::default();
    if d > 2.3 {
        i.buttons |= buttons::PUSH;
    }
    // the fifth: a hippie jump (crouch with the button held, let go, no flick)
    if n == 4 {
        if (0.1..0.5).contains(&d) {
            i.flick = [0.0, -1.0];
        }
        if d < 1.5 {
            i.buttons |= buttons::FOOT_OFF;
        }
        return i;
    }
    let base = ["Ollie", "Nollie", "Pop Shove-it", "Frontside Shove-it"][n];
    let names = skate_lite_sys::trick_names();
    let Some(k) = names.iter().position(|x| *x == base) else { return i };
    let path = skate_lite_sys::trick_demo_path(k);
    let f = (d * 60.0) as usize;
    let f = if f < 4 { f } else if f < 24 { 4 } else { f - 19 };
    if let Some(p) = path.get(f) {
        i.flick = *p;
        i.buttons |= buttons::FOOT_OFF;
    }
    i
}

/// `SK8_DEMO=grinds`: one rail per slot (along the path for grinds, across it at 70 degrees for
/// slides), an ollie onto it with the left stick set as the board comes down: every grind kind.
const GRIND_EACH: f32 = 4.5;
/// (slide, stick x, stick y) per slot, in sk8_grind_kind order.
const GRIND_SLOTS: [(bool, f32, f32); 17] = [
    // grinds by the right stick at the catch, as the port: 50-50, nosegrind, 5-0, crooked (held on
    // into a boardslide), overcrook, lipslide, willy
    (false, 0.0, 0.0), (false, 0.0, 1.0), (false, 0.0, -1.0), (false, 0.8, 0.8), (false, -0.8, 0.8),
    (false, -1.0, 0.0), (false, -0.8, -0.8),
    // slides (the rail across the path): boardslide, noseslide, tailslide, lipslide, noseblunt, bluntslide
    (true, 0.0, 0.0), (true, 0.0, 1.0), (true, 0.0, -1.0), (true, 1.0, 0.0), (true, 0.8, 0.8), (true, 0.8, -0.8),
    // with the left stick held back: smith, feeble, salad, suski
    (false, -1.0, 0.0), (false, 1.0, 0.0), (false, 0.8, -0.8), (false, 0.8, 0.8),
];
fn demo_grinds(t: f32, s: &skate_lite_sys::State, rail: Option<[Vec3; 2]>) -> Input {
    let d = t % GRIND_EACH;
    let slot = (t / GRIND_EACH) as usize % GRIND_SLOTS.len();
    let (_, x, y) = GRIND_SLOTS[slot];
    let mut i = Input::default();
    if (0.15..0.5).contains(&d) {
        i.flick = [0.0, -1.0];
    } else if (0.5..0.6).contains(&d) {
        i.flick = [0.0, 1.0];
    }
    // the right stick set as the board comes down to the rail and kept there along it
    let near = rail.is_some_and(|[a, b]| {
        let p = Vec3::from_array(s.position);
        let ab = b - a;
        let k = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        (a + ab * k).distance(p) < 0.9
    });
    let on = s.mode == skate_lite_sys::Mode::Grind || (s.mode == skate_lite_sys::Mode::Air && near && d > 0.6);
    if on {
        i.flick = [x, y];
        if slot >= 13 {
            i.steer = [0.0, -1.0];
        }
    }
    i
}

/// `SK8_DEMO=extras`: the port's smaller moves, one per slot: a kickflip underflip (the opposite
/// flick just after the pop), a one-foot ollie (A held in the air), an indy one foot (a trigger, then
/// A), a board adjust (the right stick held up then sideways in the air), a ground grab (the right
/// trigger rolling along) and a grab on a rail (the right trigger on a 50-50).
const EXTRA_EACH: f32 = 3.5;
const EXTRA_SLOTS: usize = 6;
fn demo_extras(t: f32, s: &skate_lite_sys::State) -> Input {
    let d = t % EXTRA_EACH;
    let slot = (t / EXTRA_EACH) as usize % EXTRA_SLOTS;
    let mut i = Input::default();
    let ollie = |i: &mut Input| {
        if (0.15..0.5).contains(&d) {
            i.flick = [0.0, -1.0];
        } else if (0.5..0.6).contains(&d) {
            i.flick = [0.0, 1.0];
        }
    };
    match slot {
        0 => {
            // the kickflip's own stick path (as SK8_DEMO=flips plays it), then up-left at once
            let kick = skate_lite_sys::trick_names().iter().position(|n| *n == "Kickflip").unwrap_or(0);
            let path = skate_lite_sys::trick_demo_path(kick);
            let k = ((d - 0.15).max(0.0) * 60.0) as usize;
            let k = if k < 4 { k } else if k < 24 { 4 } else { k - 19 };
            if let Some(p) = path.get(k) {
                i.flick = *p;
            } else if k < path.len() + 6 {
                i.flick = [-0.7071, 0.7071];
            }
        }
        1 => {
            ollie(&mut i);
            if (0.75..1.3).contains(&d) {
                i.buttons |= buttons::PUSH;
            }
        }
        2 => {
            ollie(&mut i);
            if (0.7..1.4).contains(&d) {
                i.buttons |= buttons::GRAB_RIGHT;
            }
            if (0.85..1.35).contains(&d) {
                i.buttons |= buttons::PUSH;
            }
        }
        3 => {
            ollie(&mut i);
            // (held from the centre: one way only, or the stick's path reads as a late flick)
            if (0.8..1.35).contains(&d) {
                i.flick = [0.0, 1.0];
            }
        }
        4 => {
            if (0.3..2.4).contains(&d) {
                i.buttons |= buttons::GRAB_RIGHT;
            }
        }
        _ => {
            ollie(&mut i);
            if s.mode == skate_lite_sys::Mode::Grind {
                i.buttons |= buttons::GRAB_RIGHT;
            }
        }
    }
    i
}

/// The demo's rail for slot `n`, from the spawn (straight ahead along `fwd`).
fn demo_rail(n: usize, at: Vec3, fwd: Vec3) -> [Vec3; 2] {
    let (slide, ..) = GRIND_SLOTS[n % GRIND_SLOTS.len()];
    let up = Vec3::Y * 0.45;
    if slide {
        let across = Quat::from_rotation_y(70f32.to_radians()) * fwd;
        let c = at + fwd * 9.2 + up;
        [c - across * 3.0, c + across * 3.0]
    } else {
        [at + fwd * 1.5 + up, at + fwd * 17.0 + up]
    }
}

/// Draws the scripted demo's rail (it has no mesh).
pub(crate) fn demo_rail_gizmo(sim: Res<CleanSkate>, mut gizmos: Gizmos) {
    if let Some([a, b]) = sim.demo_rail {
        for k in 0..3 {
            let o = Vec3::Y * (k as f32 * 0.01 - 0.01);
            gizmos.line(a + o, b + o, Color::srgb(0.85, 0.85, 0.9));
        }
    }
}

/// `SK8_DEMO=revert`: an ollie with X held into the landing, rolling away fakie.
const REVERT_EACH: f32 = 3.0;
fn demo_revert(t: f32) -> Input {
    let d = t % REVERT_EACH;
    let mut i = Input::default();
    if (0.15..0.5).contains(&d) {
        i.flick = [0.0, -1.0];
    } else if (0.5..0.6).contains(&d) {
        i.flick = [0.0, 1.0];
    }
    if (0.7..1.9).contains(&d) {
        i.buttons |= buttons::SLIDE;
    }
    i
}

/// `SK8_DEMO=switch`: a revert to roll away fakie, then per slot: left alone it shuffles round to
/// switch and pops a switch ollie; held crouched it stays fakie for a fakie ollie; a push from fakie
/// switches at once, then a switch kickflip.
const SWITCH_EACH: f32 = 6.0;
fn demo_switch(t: f32) -> Input {
    let slot = (t / SWITCH_EACH) as usize % 3;
    let mut i = demo_revert(t.min((t / SWITCH_EACH).floor() * SWITCH_EACH + 2.0));
    let d = t % SWITCH_EACH;
    if d < 2.0 {
        return i;
    }
    i = Input::default();
    let pop = |at: f32, end: [f32; 2], i: &mut Input| {
        if (at..at + 0.35).contains(&d) {
            i.flick = [0.0, -1.0];
        } else if (at + 0.35..at + 0.4).contains(&d) {
            i.flick = [0.0, 0.0];
        } else if (at + 0.4..at + 0.55).contains(&d) {
            i.flick = end;
        }
    };
    match slot {
        0 => pop(4.0, [0.0, 1.0], &mut i),
        1 => {
            if (2.0..3.6).contains(&d) {
                i.flick = [0.0, -1.0];
            } else if (3.6..3.75).contains(&d) {
                i.flick = [0.0, 1.0];
            }
        }
        _ => {
            if (2.0..2.1).contains(&d) {
                i.buttons |= buttons::PUSH;
            }
            pop(4.3, [0.7, 0.7], &mut i);
        }
    }
    i
}

/// `SK8_DEMO=wallride`: beside the desktop park's wall, heading a little into it, an ollie onto it;
/// every other slot flicks again on the wall to pop off it (a wallie).
const WALL_EACH: f32 = 3.5;
fn demo_wallride(t: f32, s: &skate_lite_sys::State) -> Input {
    let d = t % WALL_EACH;
    let pop_off = (t / WALL_EACH) as usize % 2 == 1;
    let mut i = Input::default();
    if (0.1..0.35).contains(&d) {
        i.flick = [0.0, -1.0];
    } else if (0.35..0.45).contains(&d) {
        i.flick = [0.0, 1.0];
    }
    if pop_off && s.mode == skate_lite_sys::Mode::Wallride {
        i.flick = if d < 0.9 { [0.0, -1.0] } else { [0.0, 1.0] };
    }
    i
}

/// `SK8_DEMO=bail`: the bail's stages, both ways: an ollie with a kickflip flicked late, just before
/// it comes down (under-flipped: thrown forward), then rolling hard straight into the desktop park's
/// wall (slipped out backward). Each slot lasts long enough to see it get up and back on.
const BAIL_EACH: f32 = 7.0;
fn demo_bail(t: f32) -> Input {
    let d = t % BAIL_EACH;
    let mut i = Input::default();
    if (t / BAIL_EACH) as usize % 2 == 0 {
        if (0.2..0.45).contains(&d) {
            i.flick = [0.0, -1.0];
        } else if (0.45..0.5).contains(&d) {
            i.flick = [0.0, 1.0];
        } else if (0.95..1.0).contains(&d) {
            i.flick = [0.0, -1.0];
        } else if (1.0..1.05).contains(&d) {
            i.flick = [0.0, 0.0];
        } else if (1.05..1.15).contains(&d) {
            i.flick = [0.7, 0.7];
        }
    }
    i
}

/// `SK8_DEMO=held`: flip tricks flicked and held at the end of the flick (more turns: double ...),
/// off the desktop park's kicker for the air.
const HELD_EACH: f32 = 3.0;
fn demo_held(t: f32) -> Input {
    const HOLD: usize = 20;
    let names = skate_lite_sys::trick_names();
    let pick = ["Kickflip", "Heelflip", "Pop Shove-it", "Tre Flip", "Impossible"];
    let n = (t / HELD_EACH) as usize % pick.len();
    let d = t % HELD_EACH;
    let mut i = Input::default();
    let Some(k) = names.iter().position(|x| *x == pick[n]) else { return i };
    let path = skate_lite_sys::trick_demo_path(k);
    let f = (d * 60.0) as usize;
    let f = if f < 4 { f } else if f < 4 + HOLD { 4 } else { f - HOLD + 1 };
    // the last sample of the flick, then kept there through the air
    if let Some(p) = path.get(f).or(if d < 1.6 { path.last() } else { None }) {
        i.flick = *p;
    }
    i
}

/// `SK8_DEMO=walk`: rolling, off the board (Y), standing, walking, running, back on, pushing away.
const WALK_EACH: f32 = 8.0;
fn demo_walk(t: f32) -> Input {
    let d = t % WALK_EACH;
    let mut i = Input::default();
    if (1.0..1.1).contains(&d) || (6.0..6.1).contains(&d) {
        i.buttons |= buttons::OFF_BOARD;
    }
    if (2.0..4.0).contains(&d) {
        i.steer[1] = 0.55;
    } else if (4.0..5.6).contains(&d) {
        i.steer = [0.25, 1.0];
    }
    if d > 6.3 || (3.0..3.05).contains(&d) {
        i.buttons |= buttons::PUSH; // (on foot: a jump)
    }
    i
}

/// `SK8_DEMO=plants`: an ollie at an edge across the path with the foot-off button held near it:
/// a footplant, then (a grab held too) a handplant; then the port's way to a boneless (a trigger
/// held, push pressed: the wind-up, then the pop) and a fastplant (X instead of A); then RB held up
/// the desktop park's quarter pipe: a handplant on its coping, X taking the front foot off.
const PLANT_EACH: f32 = 4.5;
const PLANT_SLOTS: usize = 5;
fn demo_plants(t: f32, s: &skate_lite_sys::State, rail: Option<[Vec3; 2]>) -> Input {
    let d = t % PLANT_EACH;
    let slot = (t / PLANT_EACH) as usize % PLANT_SLOTS;
    let hand = slot == 1;
    let mut i = Input::default();
    if slot == 4 {
        i.buttons |= buttons::SLIDE;
        if s.mode == skate_lite_sys::Mode::Grind {
            i.buttons |= buttons::PUSH_MONGO;
        }
        return i;
    }
    if slot >= 2 {
        if (0.1..1.2).contains(&d) {
            i.buttons |= buttons::GRAB_RIGHT;
        }
        if (0.2..0.9).contains(&d) {
            i.buttons |= if slot == 3 { buttons::PUSH_MONGO } else { buttons::PUSH };
        }
        return i;
    }
    if (0.15..0.5).contains(&d) {
        i.flick = [0.0, -1.0];
    } else if (0.5..0.6).contains(&d) {
        i.flick = [0.0, 1.0];
    }
    let near = rail.is_some_and(|[a, b]| {
        let p = Vec3::from_array(s.position);
        let ab = b - a;
        let k = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
        (a + ab * k).distance(p) < 1.2
    });
    if (s.mode == skate_lite_sys::Mode::Air && near) || s.mode == skate_lite_sys::Mode::Grind {
        i.buttons |= buttons::FOOT_OFF | if hand { buttons::GRAB_RIGHT } else { 0 };
    }
    i
}

/// `SK8_DEMO=varial`: an ollie, then the foot-off button and the left stick: a body varial.
const VARIAL_EACH: f32 = 3.0;
fn demo_varial(t: f32, s: &skate_lite_sys::State) -> Input {
    let d = t % VARIAL_EACH;
    let mut i = Input::default();
    if (0.15..0.5).contains(&d) {
        i.flick = [0.0, -1.0];
    } else if (0.5..0.6).contains(&d) {
        i.flick = [0.0, 1.0];
    }
    if s.mode == skate_lite_sys::Mode::Air && d > 0.72 {
        i.buttons |= buttons::FOOT_OFF;
        // half a turn at the spin rate (the stick let go after it)
        if d < 0.72 + std::f32::consts::PI / 6.6 {
            i.steer[0] = 1.0;
        }
    }
    i
}

/// `SK8_DEMO=primo`: the foot-off button with the right stick part-way down (a primo), then up
/// (a casper).
const PRIMO_EACH: f32 = 3.0;
fn demo_primo(t: f32) -> Input {
    let d = t % PRIMO_EACH;
    let casper = (t / PRIMO_EACH) as usize % 2 == 1;
    let mut i = Input::default();
    if (0.3..2.2).contains(&d) {
        i.buttons |= buttons::FOOT_OFF;
        i.flick = [0.0, if casper { 0.6 } else { -0.6 }];
    }
    i
}

/// `SK8_DEMO=1`: a scripted run through the move set (looping), for checking animation by eye or
/// with `SK8_ANIM_TRACE` without hand-driving the game.
fn demo_input(t: f32, s: &skate_lite_sys::State) -> Input {
    const LOOP: f32 = 26.0;
    let t = t % LOOP;
    let mut i = Input::default();
    let (down, centre, up) = ([0.0, -1.0], [0.0, 0.0], [0.0, 1.0]);
    let (up_left, up_right, down_right, right) = ([-0.7, 0.7], [0.7, 0.7], [0.7, -0.7], [1.0, 0.0]);
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
        flick(8.5, 0.35, &[centre, up_right]),
        flick(11.0, 0.35, &[centre, up_left]),
        flick(13.55, 0.3, &[down_right, right, centre]),
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
    } else if (6.6..20.0).contains(&t) && s.mode == skate_lite_sys::Mode::Ride {
        // a gentle carve between the tricks (on the ground: in the air it would spin): round in a
        // wide circle, not off the edge of the park
        i.steer[0] = 0.35;
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
        Ok("oldschool") => Some((t / OLD_EACH) as usize),
        Ok("grinds") => Some((t / GRIND_EACH) as usize),
        Ok("extras") => Some((t / EXTRA_EACH) as usize),
        Ok("revert") => Some((t / REVERT_EACH) as usize),
        Ok("switch") => Some((t / SWITCH_EACH) as usize),
        Ok("wallride") => Some((t / WALL_EACH) as usize),
        Ok("held") => Some((t / HELD_EACH) as usize),
        Ok("walk") => Some((t / WALK_EACH) as usize),
        Ok("plants") => Some((t / PLANT_EACH) as usize),
        Ok("varial") => Some((t / VARIAL_EACH) as usize),
        Ok("primo") => Some((t / PRIMO_EACH) as usize),
        Ok("bail") => Some((t / BAIL_EACH) as usize),
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
            let demo_kind = std::env::var("SK8_DEMO").unwrap_or_default();
            if demo_kind == "plants" && slot.unwrap_or(0) % PLANT_SLOTS == 4 {
                // up the desktop park's quarter pipe (at z = 40, facing +z; its coping 2.4 m up)
                sim.place_rolling(Vec3::new(0.0, 0.0, 52.0), std::f32::consts::PI, 8.5);
                sim.world.clear_rails();
                sim.world.add_rail(&[[-2.0, 2.4, 39.3], [2.0, 2.4, 39.3]]);
                sim.demo_rail = None;
            } else if demo_kind == "plants" {
                sim.place_rolling(Vec3::ZERO, std::f32::consts::PI, 7.5);
                let s = sim.current;
                let fwd = Quat::from_array(s.rider_rot) * Vec3::Z;
                // the slide rail (across the path): slot 7 in GRIND_SLOTS
                let rail = demo_rail(7, Vec3::from_array(s.position), fwd);
                sim.world.clear_rails();
                // (the boneless slots, 2 and 3, on open ground)
                if slot.unwrap_or(0) % PLANT_SLOTS < 2 {
                    sim.world.add_rail(&[rail[0].to_array(), rail[1].to_array()]);
                }
                sim.demo_rail = Some(rail);
            } else if demo_kind == "wallride" {
                // the desktop park's wall runs along z at x = 16 (z 8..20)
                sim.place_rolling(Vec3::new(15.05, 0.0, 6.0), 0.16, 7.5);
            } else if demo_kind == "bail" && slot.unwrap_or(0) % 2 == 1 {
                // straight at the desktop park's wall (x = 16, z 8..20), fast enough to slam
                sim.place_rolling(Vec3::new(10.0, 0.0, 14.0), std::f32::consts::FRAC_PI_2, 13.0);
            } else if demo_kind == "bail" {
                sim.place_rolling(Vec3::ZERO, std::f32::consts::PI, 7.0);
            } else if demo_kind == "extras" {
                sim.place_rolling(Vec3::ZERO, std::f32::consts::PI, 6.5);
                sim.world.clear_rails();
                sim.demo_rail = None;
                if slot.unwrap_or(0) % EXTRA_SLOTS == EXTRA_SLOTS - 1 {
                    let s = sim.current;
                    let fwd = Quat::from_array(s.rider_rot) * Vec3::Z;
                    let rail = demo_rail(0, Vec3::from_array(s.position), fwd);
                    sim.world.add_rail(&[rail[0].to_array(), rail[1].to_array()]);
                    sim.demo_rail = Some(rail);
                }
            } else if demo_kind == "grinds" {
                let (slide, ..) = GRIND_SLOTS[slot.unwrap_or(0) % GRIND_SLOTS.len()];
                sim.place_rolling(Vec3::ZERO, std::f32::consts::PI, if slide { 7.5 } else { 6.0 });
                let s = sim.current;
                let fwd = Quat::from_array(s.rider_rot) * Vec3::Z;
                let rail = demo_rail(slot.unwrap_or(0), Vec3::from_array(s.position), fwd);
                sim.world.clear_rails();
                sim.world.add_rail(&[rail[0].to_array(), rail[1].to_array()]);
                sim.demo_rail = Some(rail);
            } else if demo_kind == "push" {
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
            Ok("oldschool") => demo_oldschool(*t),
            Ok("grinds") => demo_grinds(*t, &sim.current, sim.demo_rail),
            Ok("extras") => demo_extras(*t, &sim.current),
            Ok("revert") => demo_revert(*t),
            Ok("switch") => demo_switch(*t),
            Ok("wallride") => demo_wallride(*t, &sim.current),
            Ok("held") => demo_held(*t),
            Ok("walk") => demo_walk(*t),
            Ok("plants") => demo_plants(*t, &sim.current, sim.demo_rail),
            Ok("varial") => demo_varial(*t, &sim.current),
            Ok("primo") => demo_primo(*t),
            Ok("bail") => demo_bail(*t),
            _ => demo_input(*t, &sim.current),
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
        (KeyCode::KeyF, buttons::FOOT_OFF),
        (KeyCode::ShiftLeft, buttons::BRAKE),
        (KeyCode::KeyG, buttons::OFF_BOARD),
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
            // the port's layout: A push, X mongo push, B brake, Y off the board; manuals are on the
            // right stick part-way (as in the port), so RB is the powerslide (and the revert)
            (GamepadButton::South, buttons::PUSH),
            (GamepadButton::West, buttons::PUSH_MONGO),
            (GamepadButton::East, buttons::BRAKE),
            (GamepadButton::North, buttons::OFF_BOARD),
            (GamepadButton::LeftThumb, buttons::FOOT_OFF),
            (GamepadButton::RightTrigger, buttons::SLIDE),
            (GamepadButton::LeftTrigger2, buttons::GRAB_LEFT),
            (GamepadButton::RightTrigger2, buttons::GRAB_RIGHT),
        ] {
            if pad.pressed(b) {
                input.buttons |= bit;
            }
        }
    }
    sim.input = input;
}
