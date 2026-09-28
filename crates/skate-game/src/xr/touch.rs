//! Quest Touch controllers presented as the Xbox pad the input pipeline was
//! built for. Everything downstream (the TU3 conversion in
//! `skate_core::input::xbox`, gesture mapping, flick-it) sees an ordinary
//! `XboxState`, so skating on Touch uses the exact same code as on a pad.
//!
//! Face buttons, sticks, triggers and stick clicks map one-to-one (the Touch
//! labels A/B/X/Y match the Xbox ones); the grips are the bumpers. Touch has
//! no D-pad, Back or Start, so the left menu button supplies them: a short
//! press is Start, and while it is held the left stick is the D-pad and X is
//! Back (neither leaks through as stick/X input during the chord).
use skate_core::input::xbox::XboxState;

pub(crate) const DPAD_UP: u16 = 0x0001;
pub(crate) const DPAD_DOWN: u16 = 0x0002;
pub(crate) const DPAD_LEFT: u16 = 0x0004;
pub(crate) const DPAD_RIGHT: u16 = 0x0008;
pub(crate) const START: u16 = 0x0010;
pub(crate) const BACK: u16 = 0x0020;
pub(crate) const LEFT_THUMB: u16 = 0x0040;
pub(crate) const RIGHT_THUMB: u16 = 0x0080;
pub(crate) const LEFT_SHOULDER: u16 = 0x0100;
pub(crate) const RIGHT_SHOULDER: u16 = 0x0200;
pub(crate) const A: u16 = 0x1000;
pub(crate) const B: u16 = 0x2000;
pub(crate) const X: u16 = 0x4000;
pub(crate) const Y: u16 = 0x8000;

/// A grip counts as a held bumper past this pull.
const GRIP_PRESSED: f32 = 0.5;
/// The chord's left stick must pass this before it counts as a D-pad press.
const DPAD_THRESHOLD: f32 = 0.6;
/// A menu press held longer than this is a chord, not Start.
const START_TAP_SECONDS: f32 = 0.35;
/// How long Start is held after a tap, so polling at any frame rate sees it.
const START_PULSE_SECONDS: f32 = 0.1;

/// One frame of raw Touch state, OpenXR ranges: sticks -1..1 (+y forward),
/// triggers and grips 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TouchState {
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub left_grip: f32,
    pub right_grip: f32,
    pub a: bool,
    pub b: bool,
    pub x: bool,
    pub y: bool,
    pub left_stick_click: bool,
    pub right_stick_click: bool,
    pub menu: bool,
}

/// The only state is the menu-button chord timing.
#[derive(Default, Debug)]
pub(crate) struct TouchMapper {
    menu_held: Option<MenuHold>,
    start_pulse: f32,
}

#[derive(Debug)]
struct MenuHold {
    seconds: f32,
    chorded: bool,
}

impl TouchMapper {
    pub(crate) fn map(&mut self, touch: &TouchState, dt: f32) -> XboxState {
        let mut buttons = 0u16;
        let mut left = touch.left_stick;
        let mut x = touch.x;

        self.start_pulse = (self.start_pulse - dt).max(0.0);
        match (&mut self.menu_held, touch.menu) {
            (None, true) => {
                self.menu_held = Some(MenuHold {
                    seconds: 0.0,
                    chorded: false,
                })
            }
            (Some(hold), true) => hold.seconds += dt,
            (Some(hold), false) => {
                if !hold.chorded && hold.seconds < START_TAP_SECONDS {
                    self.start_pulse = START_PULSE_SECONDS;
                }
                self.menu_held = None;
            }
            (None, false) => {}
        }
        if let Some(hold) = &mut self.menu_held {
            let [sx, sy] = touch.left_stick;
            for (active, bit) in [
                (sy > DPAD_THRESHOLD, DPAD_UP),
                (sy < -DPAD_THRESHOLD, DPAD_DOWN),
                (sx < -DPAD_THRESHOLD, DPAD_LEFT),
                (sx > DPAD_THRESHOLD, DPAD_RIGHT),
            ] {
                if active {
                    buttons |= bit;
                    hold.chorded = true;
                }
            }
            if touch.x {
                buttons |= BACK;
                hold.chorded = true;
            }
            left = [0.0, 0.0];
            x = false;
        }
        if self.start_pulse > 0.0 {
            buttons |= START;
        }

        for (pressed, bit) in [
            (touch.a, A),
            (touch.b, B),
            (x, X),
            (touch.y, Y),
            (touch.left_stick_click, LEFT_THUMB),
            (touch.right_stick_click, RIGHT_THUMB),
            (touch.left_grip >= GRIP_PRESSED, LEFT_SHOULDER),
            (touch.right_grip >= GRIP_PRESSED, RIGHT_SHOULDER),
        ] {
            if pressed {
                buttons |= bit;
            }
        }
        XboxState {
            buttons,
            triggers: [trigger(touch.left_trigger), trigger(touch.right_trigger)],
            left: stick(left),
            right: stick(touch.right_stick),
        }
    }
}

fn trigger(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn stick([x, y]: [f32; 2]) -> [i16; 2] {
    // XInput's range is asymmetric (-32768..=32767); full deflection either
    // way lands on the ends the TU3 conversion normalises by 1/32768.
    let axis = |v: f32| {
        let v = v.clamp(-1.0, 1.0);
        if v < 0.0 {
            (v * 32768.0).round() as i16
        } else {
            (v * 32767.0).round() as i16
        }
    };
    [axis(x), axis(y)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 72.0;

    fn run(mapper: &mut TouchMapper, touch: TouchState, frames: usize) -> XboxState {
        let mut state = mapper.map(&touch, DT);
        for _ in 1..frames {
            state = mapper.map(&touch, DT);
        }
        state
    }

    #[test]
    fn buttons_sticks_and_triggers_map_one_to_one() {
        let mut mapper = TouchMapper::default();
        let state = mapper.map(
            &TouchState {
                left_stick: [-1.0, 1.0],
                right_stick: [0.5, -0.25],
                left_trigger: 1.0,
                right_trigger: 0.5,
                left_grip: 0.9,
                right_grip: 0.2,
                a: true,
                y: true,
                right_stick_click: true,
                ..default()
            },
            DT,
        );
        assert_eq!(state.buttons, A | Y | RIGHT_THUMB | LEFT_SHOULDER);
        assert_eq!(state.left, [-32768, 32767]);
        assert_eq!(state.right, [16384, -8192]);
        assert_eq!(state.triggers, [255, 128]);
    }

    #[test]
    fn out_of_range_runtime_values_are_clamped() {
        let mut mapper = TouchMapper::default();
        let state = mapper.map(
            &TouchState {
                left_stick: [1.2, -3.0],
                left_trigger: -0.1,
                right_trigger: 1.5,
                ..default()
            },
            DT,
        );
        assert_eq!(state.left, [32767, -32768]);
        assert_eq!(state.triggers, [0, 255]);
    }

    #[test]
    fn menu_tap_pulses_start_after_release() {
        let mut mapper = TouchMapper::default();
        let pressed = run(&mut mapper, TouchState { menu: true, ..default() }, 5);
        assert_eq!(pressed.buttons & START, 0, "no Start while still held");
        let released = mapper.map(&TouchState::default(), DT);
        assert_ne!(released.buttons & START, 0);
        // The pulse ends on its own.
        let later = run(&mut mapper, TouchState::default(), 20);
        assert_eq!(later.buttons & START, 0);
    }

    #[test]
    fn long_menu_hold_is_not_start() {
        let mut mapper = TouchMapper::default();
        run(&mut mapper, TouchState { menu: true, ..default() }, 60);
        let released = mapper.map(&TouchState::default(), DT);
        assert_eq!(released.buttons & START, 0);
    }

    #[test]
    fn menu_chord_turns_left_stick_into_dpad_and_x_into_back() {
        let mut mapper = TouchMapper::default();
        let held = run(
            &mut mapper,
            TouchState {
                menu: true,
                left_stick: [0.9, 0.0],
                x: true,
                ..default()
            },
            2,
        );
        assert_eq!(held.buttons, DPAD_RIGHT | BACK);
        assert_eq!(held.left, [0, 0], "stick does not also steer");
        // A chorded press never becomes Start, even if short.
        let released = mapper.map(&TouchState::default(), DT);
        assert_eq!(released.buttons & START, 0);
    }

    fn default() -> TouchState {
        TouchState::default()
    }
}
