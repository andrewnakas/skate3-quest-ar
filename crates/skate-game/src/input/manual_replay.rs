//! Opt-in verification device packets, never a gameplay state/force override.
//! The same packets run in production-frame tests and the normal rendered game.
use skate_core::input::xbox::XboxState;

pub(crate) const END_TICK: u64 = 330;

pub(crate) fn sample(tick: u64, sign: i16) -> XboxState {
    XboxState {
        buttons: if (12..40).contains(&tick) { 0x1000 } else { 0 },
        triggers: [0; 2],
        left: [0; 2],
        right: if (45..195).contains(&tick) {
            [0, sign * 22000]
        } else {
            [0; 2]
        },
    }
}
