//! The MixMap controller (16 bytes, vtable `0x82316B54`) and the owner-side readers.
//!
//! Layout: `+0` vtable, `+4` key (masked `& 0xE0FFFFF0`), `+8` input block (16 × i32, the game
//! writes these), `+12` output block (16 words of packed int16 pairs, the MixMap writes these;
//! word 15 bit 0 is the block's enable flag).
//!
//! | slot | function | here |
//! |---|---|---|
//! | +0 | `sub_8294BBF0` dtor | not ported (teardown) |
//! | +4 | `sub_8294BC68` SetFloat | [`set_float`] |
//! | +8 | `sub_8294BC50` Set | [`set`] |
//! | +12 | `sub_8294BCA0` Get | [`get`] |
//! | +16 | `sub_8294BCC0` GetOutput | [`get_output`] |
//! | +20 | `sub_8294BD98` disable | [`disable`] |
//! | +24 | `sub_8294BDB0` enable | [`enable`] |
//! | +28 | `sub_8294BBE8` set inputs | [`set_inputs`] |
//! | +32 | `sub_828DE848` set outputs | [`set_outputs`] |
//!
//! The owner readers (`sub_824C2870` vfunc52, `sub_824C5910` vfunc56, `sub_824AF240` vfunc60/64 of
//! the SFX objects) read `[[owner+12]+12]`: `owner+12` is the owner's controller, bound by the
//! game's `sub_82485850`. They are exposed twice — on the owner exactly as retail, and on the
//! controller for a Rust owner that holds the controller address itself.

use super::tables::{K_4096, cents_to_ratio};
use crate::fp::{fctiwz_low_word, load_single, mul_single};
use crate::vmx::Fpscr;
use crate::{Guest, Result};

/// The controller vtable.
pub const CONTROLLER_VTABLE: u32 = 0x8231_6B54;
pub const CTRL_KEY: u32 = 4;
pub const CTRL_INPUTS: u32 = 8;
pub const CTRL_OUTPUTS: u32 = 12;

/// `sub_8294BC50` — `[ctrl+8][id] = v`, nothing when the controller has no input block.
pub fn set(g: &mut Guest, ctrl: u32, id: u32, value: u32) -> Result<()> {
    let inputs = g.u32(ctrl + CTRL_INPUTS)?;
    if inputs == 0 {
        return Ok(());
    }
    g.set_u32(inputs.wrapping_add(id << 2), value)
}

/// `sub_8294BC68` — `fctiwz` the float, then call slot +8 ([`set`]).
pub fn set_float(g: &mut Guest, ctrl: u32, id: u32, value: f64) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let word = fctiwz_low_word(value);
    set(g, ctrl, id, word)
}

/// `sub_8294BCA0` — `[ctrl+8][id]`, or 0 without an input block.
pub fn get(g: &Guest, ctrl: u32, id: u32) -> Result<u32> {
    let inputs = g.u32(ctrl + CTRL_INPUTS)?;
    if inputs == 0 {
        return Ok(0);
    }
    g.u32(inputs.wrapping_add(id << 2))
}

/// The packed half an output id lives in: word `id >> 1` (`srawi`), shifted arithmetically by
/// `(id & 1) × 16` (`sraw`).
fn output_half(g: &Guest, outputs: u32, id: u32) -> Result<i32> {
    let word = g.u32(outputs.wrapping_add((((id as i32) >> 1) as u32) << 2))? as i32;
    Ok(word >> ((id << 4) & 0x10))
}

/// Cents in the low half → `2^(c/1200) × 4096`, as `sub_8294BCC0` type 1 and `sub_824C5910` do:
/// sign-extend bit 15, `sub_8294B4D8`, `fmuls` by 4096, `fctiwz`.
fn cents_word(g: &Guest, half: i32) -> Result<u32> {
    let mut v = (half as u32) & 0xFFFF;
    if v & 0x8000 != 0 {
        v |= 0xFFFF_0000;
    }
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let ratio = cents_to_ratio(g, v as i32)?;
    Ok(fctiwz_low_word(mul_single(ratio, load_single(g, K_4096)?)))
}

/// `sub_8294BCC0(ctrl, id, type)` — slot +16. Type 0, 2 and 4: `& 0x7FFF`; 1: cents → ×4096;
/// 3: `& 0xFFFF`; above 4 (unsigned) or no output block: 0.
pub fn get_output(g: &Guest, ctrl: u32, id: u32, kind: u32) -> Result<u32> {
    let outputs = g.u32(ctrl + CTRL_OUTPUTS)?;
    if outputs == 0 || kind > 4 {
        return Ok(0);
    }
    let half = output_half(g, outputs, id)?;
    match kind {
        1 => cents_word(g, half),
        3 => Ok((half as u32) & 0xFFFF),
        _ => Ok((half as u32) & 0x7FFF),
    }
}

/// `sub_8294BD98` — `[[ctrl+12]+60] = 0` when there is an output block.
pub fn disable(g: &mut Guest, ctrl: u32) -> Result<()> {
    let outputs = g.u32(ctrl + CTRL_OUTPUTS)?;
    if outputs != 0 {
        g.set_u32(outputs + 60, 0)?;
    }
    Ok(())
}

/// `sub_8294BDB0` — `[[ctrl+12]+60] = 1` when there is an output block.
pub fn enable(g: &mut Guest, ctrl: u32) -> Result<()> {
    let outputs = g.u32(ctrl + CTRL_OUTPUTS)?;
    if outputs != 0 {
        g.set_u32(outputs + 60, 1)?;
    }
    Ok(())
}

/// `sub_8294BBE8` — `ctrl+8 = inputs`.
pub fn set_inputs(g: &mut Guest, ctrl: u32, inputs: u32) -> Result<()> {
    g.set_u32(ctrl + CTRL_INPUTS, inputs)
}

/// `sub_828DE848` — `ctrl+12 = outputs`.
pub fn set_outputs(g: &mut Guest, ctrl: u32, outputs: u32) -> Result<()> {
    g.set_u32(ctrl + CTRL_OUTPUTS, outputs)
}

/// `sub_824C2870` on a controller: the output half `& 0xFFFF`, 0 without an output block.
pub fn read_u16(g: &Guest, ctrl: u32, id: u32) -> Result<u32> {
    if ctrl == 0 {
        return Ok(0);
    }
    let outputs = g.u32(ctrl + CTRL_OUTPUTS)?;
    if outputs == 0 {
        return Ok(0);
    }
    Ok((output_half(g, outputs, id)? as u32) & 0xFFFF)
}

/// `sub_824C5910` on a controller: cents → `2^(c/1200) × 4096`.
pub fn read_pitch(g: &Guest, ctrl: u32, id: u32) -> Result<u32> {
    if ctrl == 0 {
        return Ok(0);
    }
    let outputs = g.u32(ctrl + CTRL_OUTPUTS)?;
    if outputs == 0 {
        return Ok(0);
    }
    cents_word(g, output_half(g, outputs, id)?)
}

/// `sub_824AF240` on a controller: the output half `& 0x7FFF`.
pub fn read_gain(g: &Guest, ctrl: u32, id: u32) -> Result<u32> {
    if ctrl == 0 {
        return Ok(0);
    }
    let outputs = g.u32(ctrl + CTRL_OUTPUTS)?;
    if outputs == 0 {
        return Ok(0);
    }
    Ok((output_half(g, outputs, id)? as u32) & 0x7FFF)
}

/// `sub_824C2870(owner, id)` exactly: `[owner+12]` is the controller.
pub fn owner_read_u16(g: &Guest, owner: u32, id: u32) -> Result<u32> {
    read_u16(g, g.u32(owner + 12)?, id)
}

/// `sub_824C5910(owner, id)` exactly.
pub fn owner_read_pitch(g: &Guest, owner: u32, id: u32) -> Result<u32> {
    read_pitch(g, g.u32(owner + 12)?, id)
}

/// `sub_824AF240(owner, id)` exactly.
pub fn owner_read_gain(g: &Guest, owner: u32, id: u32) -> Result<u32> {
    read_gain(g, g.u32(owner + 12)?, id)
}
