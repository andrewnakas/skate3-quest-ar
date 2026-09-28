//! Push, brake, body-spin, steering, kick-turn, crouch, powerslide and world-grab intents
//! from the TU3 ActionGraph input listener (`825999F0`).
use super::{
    angle::left_stick_angle,
    controller::{DerivedControllerInput, magnitude},
};

#[derive(Clone, Copy, Debug, Default)]
pub struct PushPreferences {
    /// Profile component+52, byte58. The host owns profile storage/selection.
    pub automatic_push_enabled: bool,
    /// Profile component+56, byte156 selects the automatic push foot.
    pub automatic_push_right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RidingIntent {
    pub name: &'static str,
    pub value: f32,
}

pub fn produce(
    controller: &DerivedControllerInput,
    actor_flags: u32,
    preferences: PushPreferences,
) -> Vec<RidingIntent> {
    let words = controller.words();
    let previous = words[6];
    let current = words[13];
    let left = [f32::from_bits(words[7]), f32::from_bits(words[8])];
    let allowed = actor_flags & (1 << 11) == 0;
    let held = |bit: u32| current & (1u32 << bit) != 0 && allowed;
    let new = |bit: u32, timer: usize| {
        held(bit) && (previous & (1u32 << bit) == 0 || f32::from_bits(words[timer]) < 0.3)
    };
    let mut right = held(21);
    let mut left_push = held(23);
    let mut new_right = new(21, 20);
    let mut new_left = new(23, 22);
    let angle = left_stick_angle(left[0], left[1]);
    let from_forward = if angle >= 0.0 {
        f32::from_bits(0x40490fdb) - angle
    } else {
        -f32::from_bits(0x40490fdb) - angle
    };
    let length = magnitude(left[1].mul_add(left[1], left[0] * left[0]));
    if preferences.automatic_push_enabled
        && allowed
        && !(right || left_push || new_right || new_left)
        && from_forward.abs() < 0.6
        && length > 0.8
    {
        if preferences.automatic_push_right {
            right = true;
            new_right = true;
        } else {
            left_push = true;
            new_left = true;
        }
    }
    if actor_flags & (1 << 6) != 0 && (right || new_right) && (left_push || new_left) {
        right = false;
        left_push = false;
        new_right = false;
        new_left = false;
    }
    let mut output = Vec::with_capacity(17);
    let mut emit = |name, value| output.push(RidingIntent { name, value });
    if current & (1 << 20) != 0 && actor_flags & (1 << 7) == 0 {
        emit("Brake", 1.0);
    }
    if new_right || new_left {
        emit("NewPush", 1.0);
    }
    if right {
        emit("RightPush", 1.0);
        emit("Pushing", 1.0);
    }
    if left_push {
        emit("LeftPush", 1.0);
        emit("Pushing", 1.0);
    }
    //8259AC1C..AC54 emits this ordered pair from current left X. Actor1908
    //bit1 gates both; steering's bit0 and the physical state do not gate it.
    //BodySpin's later animation-attribute handler owns its sign conversion.
    if left[0] != 0.0 && actor_flags & (1 << 1) == 0 {
        emit("BodySpin", left[0]);
        emit("PhysBodySpin", left[0]);
    }
    //8259ACDC..ACE8 selects KickTurn from current left X, not shaped Turn.
    if left[0] != 0.0 && actor_flags & 1 == 0 {
        emit("KickTurn", left[0]);
    }
    let steering = super::steering_intentions::produce(left, actor_flags);
    if let Some(value) = steering.hard_turn_crouch {
        emit("HardTurnCrouch", value);
    }
    if let Some(value) = steering.hard_turn {
        emit("HardTurn", value);
    }
    //8259A398..A424, emission key8259AE30..AE3C: the stick-based crouch ramp is
    //multiplied by literal zero (f24). The two fsel instructions select the
    //MAXIMUM of that result and the two trigger values, not a subtraction.
    //An active gate can therefore intentionally emit Crouch with value zero.
    let absolute = angle.abs();
    let start = f32::from_bits(0x3ff5_c28f); //1.92
    let end = f32::from_bits(0x4016_6666); //2.35
    let stick_crouch = if absolute <= start {
        0.0
    } else if absolute < end {
        ((absolute - start) * f32::from_bits(0x4014_d655)) * length
    } else {
        length
    } * 0.0;
    let [left_trigger, right_trigger] = [words[11], words[12]].map(f32::from_bits);
    if absolute > start || left_trigger > 0.0 || right_trigger != 0.0 {
        let trigger = if left_trigger - right_trigger >= -0.0 {
            left_trigger
        } else {
            right_trigger
        };
        emit(
            "Crouch",
            if stick_crouch - trigger >= -0.0 {
                stick_crouch
            } else {
                trigger
            },
        );
    }
    if let Some(value) = steering.turn {
        emit("Turn", value);
    }

    //8259A430..A50C: start queries and continuous values have distinct open
    //heading windows. Both continuous descriptors can be present together.
    //8259AECC..AF5C gates only actor bit8, independently of push inhibition.
    if actor_flags & (1 << 8) == 0 && length > 0.89999998 {
        const SLIDE_SCALE: f32 = 0.28004956;
        const HALF_PI: f32 = f32::from_bits(0x3fc90fdb);
        if angle > 0.0 && angle < 0.91000003 {
            emit("RightSlideStart", 1.0);
        }
        if angle > -0.91000003 && angle < 0.0 {
            emit("LeftSlideStart", 1.0);
        }
        if angle > -2.0 && angle < HALF_PI {
            emit("LeftSlide", -((angle + 2.0) * SLIDE_SCALE));
        }
        if angle > -HALF_PI && angle < 2.0 {
            emit("RightSlide", (2.0 - angle) * SLIDE_SCALE);
        }
    }
    //8259A54C/554 reads current raw flags bit28;8259AF68..7C emits
    //GrabWorld (830BE780, named by initializer82F84EB0) while held.
    //No actor inhibition or rising-edge gate applies to this intention.
    if current & (1 << 28) != 0 {
        emit("GrabWorld", 1.0);
    }
    output
}

#[cfg(test)]
#[path = "riding_intentions_tests.rs"]
mod tests;
