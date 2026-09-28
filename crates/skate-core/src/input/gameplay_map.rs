//! Specialization of the 18 stock gameplay expressions registered by82697740.
//! Config identity and VM operand semantics are recorded in STEERING_INPUT.md.
use super::{controller::ActionMap, pad::Pad};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameplayActions {
    values: [f32; 18],
}

impl GameplayActions {
    pub fn from_values(values: [f32; 18]) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &[f32; 18] {
        &self.values
    }
}
impl GameplayActions {
    /// The native pad retains storage even while count is zero. With a nonzero
    /// count, stock Button operands read their fixed index, without a per-index
    /// count check; the caller must provide the native 24-button storage.
    pub fn from_pad(pad: &Pad) -> Self {
        let value = |slot: usize| {
            if pad.count() == 0 {
                0.0
            } else {
                f32::from_bits(pad.records()[slot][0])
            }
        };
        Self {
            values: [
                value(16) - value(17),
                value(18) - value(19),
                value(6),
                value(20) - value(21),
                value(22) - value(23),
                value(7),
                value(10),
                value(11),
                value(8),
                value(9),
                value(0),
                value(1),
                value(2),
                value(3),
                value(14),
                value(15),
                value(12),
                value(13),
            ],
        }
    }
}
impl ActionMap for GameplayActions {
    fn value(&mut self, action: u32) -> f32 {
        self.values
            [usize::try_from(action.checked_sub(64).expect("gameplay action below 64")).unwrap()]
    }
    fn state(&mut self, action: u32) -> u8 {
        u8::from(self.value(action) != 0.0)
    }
}
