//! Voice and handle lifecycle: release a voice, unlink its node, re-point a mix link, unlink through
//! a checked handle, and remove an object from its owner's handle array.
//!
//! Seven verified bodies, ported from their `recomp/src/audio_ports/sub_*.inc`:
//!
//! | function | guest | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|
//! | [`unlink_voice`] | `sub_82B34BD0` | 109 | 9,408 | 10,860 |
//! | [`release_voice`] | `sub_82B31480` | 193 | 12,908 | 14,922 |
//! | [`release_voice_alias`] | `sub_82B31368` | 6 | 3,206 | 3,688 |
//! | [`repoint_link`] | `sub_82B31680` | 89 | 3,248 | 3,773 |
//! | [`unlink_checked`] | `sub_828E30B8` | 81 | 143 | 115 |
//! | [`unlink_checked_gen8`] | `sub_828E2D78` | 81 | 22 | 45 — **thin** |
//! | [`remove_handle`] | `sub_82B49438` | 90 | 3,826 | 4,126 |
//!
//! Replayed against **8,000 recorded calls, 0 disagreements** (session `voices`); the same session
//! compared each live with zero divergence. The thin `sub_828E2D78` carries 14 of them.
//!
//! ## The intrusive node, and a naming trap in the C++
//!
//! A voice carries a list node at `+56`: `{+0 next, +4 prev, +8 value, +12 item, +16 owner, +20 u16,
//! +22 u8}`, and the owner keeps the list head at its own `+4`. Which link is which is settled by the
//! head insert in `sub_82B31680`: the new node's `+0` gets the old head and its `+4` gets zero, and the
//! old head's `+4` is pointed back at the new node. So `+0` is **next** and `+4` is **prev**.
//!
//! `recomp/src/audio_ports/sub_82B34BD0.inc` names them the other way round (`kPrev = 0`,
//! `kNext = 4`). The arithmetic there is right — it is a transcription — only the names are swapped.
//! Here the constants follow the insert, [`LINK_NEXT`] and [`LINK_PREV`].
//!
//! A node's `+12` is the "item" the owner put it on the list with; zero means detached, and every
//! unlink returns at once without writing anything when it is zero.
//!
//! ## Releasing a voice
//!
//! When the voice's `+78` gain count is non-zero, [`release_voice`] builds two eight-entry pointer
//! arrays in its own 208-byte frame — sources pointing at the voice's eight gain words, destinations
//! pointing at eight words of frame scratch — gathers the gains into the scratch with
//! [`crate::routing::gather_bank`], unlinks the node while **folding** the gathered gains into the
//! owner's accumulator, and then zeroes the voice's gains. With no gains it runs the same unlink
//! inline, with nothing folded.
//!
//! The frame is reproduced here, because the pointer arrays are guest memory the gather reads back.

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp, mem, routing};

/// `+0` — the next node. The list head's `+0` is the second node.
pub const LINK_NEXT: u32 = 0;
/// `+4` — the previous node; zero at the head.
pub const LINK_PREV: u32 = 4;
/// `+8` — a value mirrored from the owner, cleared on unlink.
pub const NODE_VALUE: u32 = 8;
/// `+12` — the item the node was listed with. Zero means detached.
pub const NODE_ITEM: u32 = 12;
/// `+16` — the list owner.
pub const NODE_OWNER: u32 = 16;
/// `+20` — a halfword, cleared on unlink.
pub const NODE_SHORT: u32 = 20;
/// `+22` — the node's gain count, a byte; re-read on every fold iteration.
pub const NODE_GAIN_COUNT: u32 = 22;
/// The owner's `+4` — the list head.
pub const OWNER_HEAD: u32 = 4;
/// The owner's `+12` — the gain accumulator the fold adds into, four bytes per gain.
pub const OWNER_GAINS: u32 = 12;
/// The owner's `+46` — set when a fold touched it.
pub const OWNER_DIRTY: u32 = 46;

/// Unlink `node` from its owner's list, folding `gains` into the owner if it is non-null
/// (`sub_82B34BD0`). A detached node — `+12` zero — returns with nothing written.
///
/// Every step is the original's, reloads included: the owner pointer and the gain count are both
/// **re-read on every fold iteration**, so a fold that wrote over either would change what follows.
/// The accumulate is `lfsx` of the gain, `lfs` of the current total, `fadds`, `stfs`.
pub fn unlink_voice(g: &mut Guest, node: u32, gains: u32) -> Result<()> {
    if g.u32(node + NODE_ITEM)? == 0 {
        return Ok(()); // beqlr -- writes nothing
    }
    let owner = g.u32(node + NODE_OWNER)?;
    let head = g.u32(owner.wrapping_add(OWNER_HEAD))?;
    if node == head {
        let second = g.u32(head.wrapping_add(LINK_NEXT))?;
        g.set_u32(owner.wrapping_add(OWNER_HEAD), second)?; // stw r11,4(r10)
    }
    let prev = g.u32(node + LINK_PREV)?;
    if prev as i32 != 0 {
        let next = g.u32(node + LINK_NEXT)?;
        g.set_u32(prev.wrapping_add(LINK_NEXT), next)?; // stw r10,0(r11)
    }
    let next = g.u32(node + LINK_NEXT)?;
    if next as i32 != 0 {
        let prev = g.u32(node + LINK_PREV)?;
        g.set_u32(next.wrapping_add(LINK_PREV), prev)?; // stw r10,4(r11)
    }

    if gains != 0 {
        let owner = g.u32(node + NODE_OWNER)?;
        g.set_u8(owner.wrapping_add(OWNER_DIRTY), 1)?; // stb r9,46(r11)
        if g.u8(node + NODE_GAIN_COUNT)? != 0 {
            let mut fpscr = Fpscr::capture();
            fpscr.disable_flush_mode_unconditional(); // emitted at every lfsx
            let mut offset = OWNER_GAINS;
            let mut folded = 0i32;
            loop {
                let target = g.u32(node + NODE_OWNER)?.wrapping_add(offset); // reloaded
                let increment = fp::load_single(g, gains.wrapping_add(offset - OWNER_GAINS))?;
                folded += 1;
                offset += 4;
                let current = fp::load_single(g, target)?; // lfs f13,0(r9)
                fp::store_single(g, target, fp::add_single(increment, current))?; // fadds ; stfs
                if folded >= i32::from(g.u8(node + NODE_GAIN_COUNT)?) {
                    break; // the count is reloaded too
                }
            }
        }
    }

    g.set_u32(node + NODE_ITEM, 0)?; // stw r7,12(r3)
    g.set_u32(node + NODE_OWNER, 0)?; // stw r7,16(r3)
    g.set_u32(node + NODE_VALUE, 0)?; // stw r7,8(r3)
    g.set_u8(node + NODE_GAIN_COUNT, 0)?; // stb r7,22(r3)
    g.set_u16(node + NODE_SHORT, 0) // sth r7,20(r3)
}

/// `lbz r6,41(r3)` — the voice's source-channel count, handed to the gather.
pub const VOICE_CHANNELS: u32 = 41;
/// `+56` — the voice's list node.
pub const VOICE_NODE: u32 = 56;
/// `lbz r5,78(r3)` — the voice's gain count; the node's own `+22`.
pub const VOICE_GAIN_COUNT: u32 = 78;
/// `+80` — eight gain words, the gather's sources, cleared after the fold.
pub const VOICE_GAINS: u32 = 80;
/// `stwu r1,-208(r1)` — the release's frame.
pub const RELEASE_FRAME_BYTES: u32 = 208;
/// `addi r4,r1,80` — eight source pointers.
pub const RELEASE_SOURCES: u32 = 80;
/// `addi r3,r1,112` — eight destination pointers.
pub const RELEASE_DESTS: u32 = 112;
/// `addi r4,r1,144` — eight words of scratch the gains are gathered into.
pub const RELEASE_SCRATCH: u32 = 144;

/// Release a voice (`sub_82B31480`). `r3` is the voice at full width and `sp` the guest `r1`.
///
/// Returns what the original leaves in `r3`: on the gain path, the node address `voice + 56` that the
/// unlink was called with and does not change; with no gains, the voice pointer as it arrived.
pub fn release_voice(g: &mut Guest, r3: u64, sp: u32) -> Result<u64> {
    let voice = r3 as u32;
    let gain_count = u32::from(g.u8(voice + VOICE_GAIN_COUNT)?); // lbz r5,78(r3)
    let frame = sp.wrapping_sub(RELEASE_FRAME_BYTES); // stwu r1,-208(r1)
    g.set_u32(frame, sp)?;

    if gain_count == 0 {
        // loc_82B31560: the unlink inlined with a null gain vector -- the same steps.
        unlink_voice(g, voice + VOICE_NODE, 0)?;
        return Ok(r3);
    }

    let channels = u32::from(g.u8(voice + VOICE_CHANNELS)?); // lbz r6,41(r3)
    for i in 0..8u32 {
        // 64-bit adds, each stored as its low word.
        g.set_u32(
            frame + RELEASE_SOURCES + 4 * i,
            (r3 + u64::from(VOICE_GAINS + 4 * i)) as u32,
        )?;
        g.set_u32(
            frame + RELEASE_DESTS + 4 * i,
            frame + RELEASE_SCRATCH + 4 * i,
        )?;
    }
    // bl 0x82b468c0 -- r7 is the literal 1: one gain word per entry.
    routing::gather_bank(
        g,
        frame + RELEASE_DESTS,
        frame + RELEASE_SOURCES,
        gain_count,
        channels,
        1,
    )?;
    // bl 0x82b34bd0 -- unlink, folding the gathered gains into the owner.
    unlink_voice(g, voice + VOICE_NODE, frame + RELEASE_SCRATCH)?;
    for i in 0..8u32 {
        g.set_u32(voice + VOICE_GAINS + 4 * i, 0)?; // stwu r9,4(r11) x8
    }
    Ok(r3 + u64::from(VOICE_NODE))
}

/// `sub_82B31368` — a six-line alias: `b 0x82b31480`, an unconditional tail branch, so the release
/// runs with this function's arguments and returns straight to its caller. The original's result
/// mask is none, so nothing is returned here.
pub fn release_voice_alias(g: &mut Guest, r3: u64, sp: u32) -> Result<()> {
    release_voice(g, r3, sp).map(|_| ())
}

/// `lwz r31,4(r3)` — the link's target, the voice whose node moves.
pub const LINK_TARGET: u32 = 4;
/// `lwz r10,12(r30)` — the source owning the list the node joins. Zero leaves it detached.
pub const LINK_SOURCE: u32 = 12;
/// `addi r9,r10,52` — the list owner inside the source.
pub const SOURCE_OWNER: u32 = 52;
/// `stwu r1,-112(r1)` — the re-point's frame, below which the release opens its own.
pub const REPOINT_FRAME_BYTES: u32 = 112;

/// Re-point a mix link (`sub_82B31680`): release the target's voice, then push its node onto the
/// source's list and copy the source's format fields into it. Returns 16 on every path.
///
/// Two reloads are the original's: the source's item pointer is loaded a second time for its channel
/// byte, and the list head is loaded again **after** the node's two link words are written, and it
/// is that second value that gets back-linked.
pub fn repoint_link(g: &mut Guest, link: u32, sp: u32) -> Result<u64> {
    let target = g.u32(link + LINK_TARGET)?; // lwz r31,4(r3)
    let frame = sp.wrapping_sub(REPOINT_FRAME_BYTES); // stwu r1,-112(r1)
    g.set_u32(frame, sp)?;
    release_voice(g, u64::from(target), frame)?; // mr r3,r31 ; bl 0x82b31480

    let source = g.u32(link + LINK_SOURCE)?; // lwz r10,12(r30)
    if source != 0 {
        let node = target.wrapping_add(VOICE_NODE);
        let owner = source.wrapping_add(SOURCE_OWNER);
        let item = g.u32(owner)?;
        g.set_u32(node + NODE_ITEM, item)?; // lwz r8,52(r10) ; stw r8,68(r31)
        let value = g.u32(owner.wrapping_add(8))?;
        g.set_u32(node + NODE_VALUE, value)?; // lwz r6,60(r10) ; stw r6,64(r31)
        let count = g.u16(owner.wrapping_add(44))?;
        g.set_u16(node + NODE_SHORT, count)?; // lhz r5,96(r10) ; sth r5,76(r31)
        let item_again = g.u32(owner)?; // lwz r4,52(r10) -- reloaded
        let channels = g.u8(item_again.wrapping_add(42))?;
        g.set_u8(node + NODE_GAIN_COUNT, channels)?; // lbz r3,42(r4) ; stb r3,78(r31)
        g.set_u32(node + NODE_OWNER, owner)?; // stw r9,72(r31)
        let head = g.u32(owner.wrapping_add(OWNER_HEAD))?;
        g.set_u32(node + LINK_NEXT, head)?; // lwz r8,56(r10) ; stw r8,56(r31)
        g.set_u32(node + LINK_PREV, 0)?; // stw r7,60(r31)
        let head_again = g.u32(owner.wrapping_add(OWNER_HEAD))?; // lwz r10,56(r10) -- reloaded
        if head_again != 0 {
            g.set_u32(head_again.wrapping_add(LINK_PREV), node)?; // stw r11,4(r10)
        }
        g.set_u32(owner.wrapping_add(OWNER_HEAD), node)?; // stw r11,4(r9)
    }
    Ok(16) // li r3,16
}

/// A handle: `{+0 owner, +4 generation}`. A negative generation means already invalid.
pub const HANDLE_OWNER: u32 = 0;
/// `+4` — the handle's generation.
pub const HANDLE_GENERATION: u32 = 4;
/// Returned for a handle with no owner.
pub const ERROR_NO_OWNER: i64 = -6;
/// Returned, and stored into the handle, when its generation is stale.
pub const ERROR_STALE: i64 = -3;

/// Validate `handle` against the generation at `owner + generation_at`, then unlink `node` from the
/// owner's list, whose head is the owner's `+0`. Returns the result `r3` carries, sign-extended.
fn unlink_with_generation(
    g: &mut Guest,
    handle: u32,
    node: u32,
    generation_at: u32,
) -> Result<u64> {
    let generation = g.u32(handle + HANDLE_GENERATION)? as i32;
    if generation < 0 {
        return Ok(i64::from(generation) as u64); // already invalid: returned as it is
    }
    let owner = g.u32(handle + HANDLE_OWNER)?;
    if owner == 0 {
        return Ok(ERROR_NO_OWNER as u64);
    }
    if generation != g.u32(owner.wrapping_add(generation_at))? as i32 {
        // Stale: invalidate the handle, +4 then +0 as lifted, and report.
        g.set_u32(handle + HANDLE_GENERATION, ERROR_STALE as u32)?;
        g.set_u32(handle + HANDLE_OWNER, 0)?;
        return Ok(ERROR_STALE as u64);
    }
    let head = g.u32(owner)?; // lwz r11,0(r10)
    if node == head {
        let second = g.u32(head.wrapping_add(LINK_NEXT))?;
        g.set_u32(owner, second)?;
    }
    let prev = g.u32(node.wrapping_add(LINK_PREV))?;
    if prev as i32 != 0 {
        let next = g.u32(node.wrapping_add(LINK_NEXT))?;
        g.set_u32(prev.wrapping_add(LINK_NEXT), next)?;
    }
    let next = g.u32(node.wrapping_add(LINK_NEXT))?;
    if next as i32 != 0 {
        let prev = g.u32(node.wrapping_add(LINK_PREV))?;
        g.set_u32(next.wrapping_add(LINK_PREV), prev)?;
    }
    Ok(0) // li r3,0
}

/// Unlink `node` through a checked handle whose owner keeps its generation at `+12` (`sub_828E30B8`).
pub fn unlink_checked(g: &mut Guest, handle: u32, node: u32) -> Result<u64> {
    unlink_with_generation(g, handle, node, 12)
}

/// The same, for an owner that keeps its generation at `+8` (`sub_828E2D78`, **thin**).
pub fn unlink_checked_gen8(g: &mut Guest, handle: u32, node: u32) -> Result<u64> {
    unlink_with_generation(g, handle, node, 8)
}

/// `lwz r10,16(r3)` — the object's owner, **reloaded** before nearly every access.
pub const HANDLE_ARRAY_OWNER: u32 = 16;
/// The owner's `+108` — an array of 8-byte entries `{u32 object, u32 payload}`.
pub const HANDLE_ARRAY: u32 = 108;
/// The owner's `+280` — the entry count, a halfword.
pub const HANDLE_COUNT: u32 = 280;
/// One entry.
pub const HANDLE_ENTRY_BYTES: u32 = 8;

/// `rlwinm rX,rY,3,0,28`: the low word shifted left three, in 32 bits.
fn times_eight(value: u64) -> u64 {
    u64::from((value as u32).wrapping_shl(3))
}

/// Remove `object` from its owner's handle array: find its entry, drop the count, and memmove the
/// tail down one entry (`sub_82B49438`). Returns 1 when it was found, 0 when not.
///
/// The count is decremented **before** the array and the count are re-read for the memmove, so the
/// length is computed from the new count — `8 * ((count - 1) - index)`, exactly the entries after the
/// removed one. The owner pointer is reloaded twice more on the way.
///
/// The 96-byte frame the original opens is not reproduced: it exists only so the memcpy leaf's spill
/// lands below it, and [`mem::memmove`] keeps its bytes in a buffer.
pub fn remove_handle(g: &mut Guest, object: u32) -> Result<u64> {
    let owner = g.u32(object + HANDLE_ARRAY_OWNER)?; // lwz r10,16(r3)
    let count = i32::from(g.u16(owner.wrapping_add(HANDLE_COUNT))?); // lhz r9,280(r10)
    let mut index = 0u32;
    let mut found = false;
    if count > 0 {
        let mut entry = g.u32(owner.wrapping_add(HANDLE_ARRAY))?; // lwz r10,108(r10)
        loop {
            if g.u32(entry)? == object {
                found = true;
                break;
            }
            index += 1;
            entry = entry.wrapping_add(HANDLE_ENTRY_BYTES);
            if index as i32 >= count {
                break;
            }
        }
    }
    if !found {
        return Ok(0); // li r3,0
    }

    let owner2 = g.u32(object + HANDLE_ARRAY_OWNER)?; // reloaded
    let offset = times_eight(u64::from(index)); // rlwinm r9,r11,3,0,28
    let used = u32::from(g.u16(owner2.wrapping_add(HANDLE_COUNT))?);
    g.set_u16(
        owner2.wrapping_add(HANDLE_COUNT),
        used.wrapping_add(0xFFFF) as u16,
    )?; // addis ; addi -1 ; sth
    let owner3 = g.u32(object + HANDLE_ARRAY_OWNER)?; // reloaded again
    let array = u64::from(g.u32(owner3.wrapping_add(HANDLE_ARRAY))?);
    let remaining = u64::from(g.u16(owner3.wrapping_add(HANDLE_COUNT))?); // the NEW count
    let tail = remaining.wrapping_sub(u64::from(index)); // subf r11,r11,r4 -- 64-bit
    let dst = array + offset; // add r3,r10,r9
    mem::memmove(g, dst as u32, (dst + 8) as u32, times_eight(tail))?; // bl 0x82f4dc60
    Ok(1) // li r3,1
}

// ---------------------------------------------------------------- sub_82B49100: retirement

/// `addi r11,r3,28` — the object's list node: `+0` next, `+4` prev.
pub const RETIRE_NODE: u32 = 28;
/// `stw r30,60(r31)` — zeroed; meaning unknown.
pub const RETIRE_FIELD_60: u32 = 60;
/// `lbz r9,68(r31)` — the handle count, reloaded every loop iteration.
pub const RETIRE_HANDLE_COUNT: u32 = 68;
/// `lbz r11,71(r3)` — 1 linked on the owner's `+20` list, 2 retired.
pub const RETIRE_STATE: u32 = 71;
/// `stw r4,76(r31)` — the argument is parked here; the handles start one word on.
pub const RETIRE_ARG_SLOT: u32 = 76;
/// `stw r30,36(r9)` — zeroed on each handle.
pub const RETIRE_HANDLE_FIELD: u32 = 36;
/// The owner's list the object leaves.
pub const OWNER_LINKED_HEAD: u32 = 20;
/// The owner's list it joins.
pub const OWNER_RETIRED_HEAD: u32 = 16;

/// Retire an object (`sub_82B49100`): unlink it from its owner's `+20` list if it is on it, reset
/// the three gains and every handle's field, drop it from the owner's handle array through
/// [`remove_handle`], then push it onto the owner's `+16` list. `object` is `r3`, `arg` `r4`.
///
/// An object already retired is left untouched. The handle count is read before the four reset
/// stores and **reloaded every iteration**; the owner and the `+16` head are reloaded after
/// [`remove_handle`], and the head a second time after the node's own two stores. The original's
/// 112-byte frame is not reproduced: [`remove_handle`] needs none here.
pub fn retire_object(g: &mut Guest, object: u32, arg: u32) -> Result<()> {
    let state = g.u8(object.wrapping_add(RETIRE_STATE))?; // lbz r11,71(r3)
    if state == 2 {
        return Ok(()); // beq cr6,0x82b491f4 -- already retired
    }
    let node = object.wrapping_add(RETIRE_NODE); // addi r11,r3,28
    if state == 1 {
        let owner = g.u32(object.wrapping_add(HANDLE_ARRAY_OWNER))?; // lwz r9,16(r3)
        let head = g.u32(owner.wrapping_add(OWNER_LINKED_HEAD))?; // lwz r10,20(r9)
        if node == head {
            let next = g.u32(head)?; // lwz r10,0(r10)
            g.set_u32(owner.wrapping_add(OWNER_LINKED_HEAD), next)?; // stw r10,20(r9)
        }
        let prev = g.u32(node + 4)?; // lwz r10,4(r11)
        if prev != 0 {
            let next = g.u32(node)?; // lwz r9,0(r11)
            g.set_u32(prev, next)?; // stw r9,0(r10)
        }
        let next = g.u32(node)?; // lwz r10,0(r11) -- reloaded after the store above
        if next != 0 {
            let prev = g.u32(node + 4)?;
            g.set_u32(next.wrapping_add(4), prev)?; // stw r11,4(r10)
        }
    }
    // loc_82B49170
    let count = g.u8(object.wrapping_add(RETIRE_HANDLE_COUNT))?; // lbz r9,68(r31)
    g.set_u32(object.wrapping_add(RETIRE_ARG_SLOT), arg)?; // stw r4,76(r31)
    g.set_u32(object.wrapping_add(RETIRE_FIELD_60), 0)?; // stw r30,60(r31)
    g.set_u8(object.wrapping_add(RETIRE_STATE), 2)?; // stb r8,71(r31)
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let zero = crate::fp::load_single(g, crate::leaves::ZERO_CELL)?; // lfs f0,23056(r10)
    for k in 0..3 {
        crate::fp::store_single(g, object.wrapping_add(4 * k), zero)?; // stfs f0,0/4/8(r31)
    }
    drop(fpscr);
    if count != 0 {
        let mut cursor = object.wrapping_add(RETIRE_ARG_SLOT); // addi r10,r31,76
        let mut index = 0u32;
        loop {
            cursor = cursor.wrapping_add(4); // lwzu r9,4(r10)
            let handle = g.u32(cursor)?;
            index += 1;
            g.set_u32(handle.wrapping_add(RETIRE_HANDLE_FIELD), 0)?; // stw r30,36(r9)
            if index >= u32::from(g.u8(object.wrapping_add(RETIRE_HANDLE_COUNT))?) {
                break; // lbz r9,68(r31) ; cmplw ; blt
            }
        }
    }
    remove_handle(g, object)?; // bl 0x82b49438
    let owner = g.u32(object.wrapping_add(HANDLE_ARRAY_OWNER))?; // lwz r10,16(r31) -- reloaded
    let first = g.u32(owner.wrapping_add(OWNER_RETIRED_HEAD))?; // lwz r9,16(r10)
    g.set_u32(node + 4, 0)?; // stw r30,32(r31)
    g.set_u32(node, first)?; // stw r9,28(r31)
    let first_again = g.u32(owner.wrapping_add(OWNER_RETIRED_HEAD))?; // lwz r9,16(r10) -- again
    if first_again != 0 {
        g.set_u32(first_again.wrapping_add(4), node)?; // stw r11,4(r9)
    }
    g.set_u32(owner.wrapping_add(OWNER_RETIRED_HEAD), node)?; // stw r11,16(r10)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OWNER: u32 = BASE + 0x100;
    const A: u32 = BASE + 0x200;
    const B: u32 = BASE + 0x300;
    const C: u32 = BASE + 0x400;

    /// Three nodes on one list, A -> B -> C, each listed (a non-zero item) and owned by OWNER.
    fn listed() -> Guest {
        let mut g = Guest::single(BASE, 0x2000);
        g.put(routing::UNITY_GAIN, 1.0f32.to_bits().to_be_bytes().to_vec());
        for (node, next, prev) in [(A, B, 0), (B, C, A), (C, 0, B)] {
            g.set_u32(node + LINK_NEXT, next).unwrap();
            g.set_u32(node + LINK_PREV, prev).unwrap();
            g.set_u32(node + NODE_VALUE, 0x1111).unwrap();
            g.set_u32(node + NODE_ITEM, 0x2222).unwrap();
            g.set_u32(node + NODE_OWNER, OWNER).unwrap();
            g.set_u16(node + NODE_SHORT, 0x3333).unwrap();
        }
        g.set_u32(OWNER + OWNER_HEAD, A).unwrap();
        g
    }

    fn cleared(g: &Guest, node: u32) -> bool {
        g.u32(node + NODE_VALUE).unwrap() == 0
            && g.u32(node + NODE_ITEM).unwrap() == 0
            && g.u32(node + NODE_OWNER).unwrap() == 0
            && g.u16(node + NODE_SHORT).unwrap() == 0
            && g.u8(node + NODE_GAIN_COUNT).unwrap() == 0
    }

    #[test]
    fn unlinking_the_middle_node_joins_its_neighbours() {
        let mut g = listed();
        unlink_voice(&mut g, B, 0).unwrap();
        assert_eq!(g.u32(A + LINK_NEXT).unwrap(), C, "A now points past B");
        assert_eq!(g.u32(C + LINK_PREV).unwrap(), A, "and C back to A");
        assert_eq!(
            g.u32(OWNER + OWNER_HEAD).unwrap(),
            A,
            "the head is untouched"
        );
        assert!(cleared(&g, B), "B's fields are cleared");
        assert_eq!(
            g.u8(OWNER + OWNER_DIRTY).unwrap(),
            0,
            "no gains, so no fold and no dirty flag"
        );
    }

    #[test]
    fn unlinking_the_head_moves_the_head_on() {
        let mut g = listed();
        unlink_voice(&mut g, A, 0).unwrap();
        assert_eq!(g.u32(OWNER + OWNER_HEAD).unwrap(), B);
        assert_eq!(
            g.u32(B + LINK_PREV).unwrap(),
            0,
            "B's prev was A's, which is zero"
        );
    }

    #[test]
    fn a_detached_node_is_left_exactly_as_it_is() {
        let mut g = listed();
        g.set_u32(B + NODE_ITEM, 0).unwrap();
        unlink_voice(&mut g, B, BASE + 0x800).unwrap();
        assert_eq!(g.u32(A + LINK_NEXT).unwrap(), B, "still linked");
        assert_eq!(
            g.u32(B + NODE_VALUE).unwrap(),
            0x1111,
            "and nothing cleared"
        );
        assert_eq!(g.u8(OWNER + OWNER_DIRTY).unwrap(), 0);
    }

    #[test]
    fn a_fold_adds_the_gains_into_the_owner_and_marks_it() {
        let mut g = listed();
        let gains = BASE + 0x800;
        for (i, v) in [1.5f32, 2.25, 4.0].iter().enumerate() {
            g.set_u32(gains + 4 * i as u32, v.to_bits()).unwrap();
            g.set_u32(OWNER + OWNER_GAINS + 4 * i as u32, 10.0f32.to_bits())
                .unwrap();
        }
        g.set_u8(B + NODE_GAIN_COUNT, 2).unwrap(); // two gains, not three
        unlink_voice(&mut g, B, gains).unwrap();
        assert_eq!(g.f32(OWNER + OWNER_GAINS).unwrap(), 11.5);
        assert_eq!(g.f32(OWNER + OWNER_GAINS + 4).unwrap(), 12.25);
        assert_eq!(
            g.f32(OWNER + OWNER_GAINS + 8).unwrap(),
            10.0,
            "the count was two"
        );
        assert_eq!(g.u8(OWNER + OWNER_DIRTY).unwrap(), 1);
    }

    const VOICE: u32 = BASE + 0x1000;
    const SP: u32 = BASE + 0x1F00;

    /// A voice whose node is B's twin: listed on OWNER between A and C.
    fn voice(gain_count: u8, channels: u8) -> Guest {
        let mut g = listed();
        let node = VOICE + VOICE_NODE;
        g.set_u32(node + LINK_NEXT, C).unwrap();
        g.set_u32(node + LINK_PREV, A).unwrap();
        g.set_u32(node + NODE_ITEM, 0x2222).unwrap();
        g.set_u32(node + NODE_OWNER, OWNER).unwrap();
        g.set_u32(A + LINK_NEXT, node).unwrap();
        g.set_u32(C + LINK_PREV, node).unwrap();
        g.set_u8(VOICE + VOICE_GAIN_COUNT, gain_count).unwrap();
        g.set_u8(VOICE + VOICE_CHANNELS, channels).unwrap();
        for i in 0..8u32 {
            g.set_u32(
                VOICE + VOICE_GAINS + 4 * i,
                (0.5 * (i as f32 + 1.0)).to_bits(),
            )
            .unwrap();
            g.set_u32(OWNER + OWNER_GAINS + 4 * i, 100.0f32.to_bits())
                .unwrap();
        }
        g
    }

    #[test]
    fn releasing_a_voice_with_no_gains_unlinks_inline_and_returns_r3() {
        let mut g = voice(0, 0);
        let r3 = u64::from(VOICE);
        assert_eq!(release_voice(&mut g, r3, SP).unwrap(), r3);
        assert_eq!(g.u32(A + LINK_NEXT).unwrap(), C, "unlinked");
        assert!(cleared(&g, VOICE + VOICE_NODE));
        assert_eq!(
            g.f32(VOICE + VOICE_GAINS).unwrap(),
            0.5,
            "the gains are not touched"
        );
        assert_eq!(
            g.f32(OWNER + OWNER_GAINS).unwrap(),
            100.0,
            "and nothing was folded"
        );
    }

    #[test]
    fn releasing_a_voice_gathers_folds_and_clears_its_gains() {
        // Three gains over three channels: not a standard layout, so the gather copies each gain at
        // unity into scratch, and the fold adds those three into the owner's accumulator.
        let mut g = voice(3, 3);
        g.set_u8(VOICE + VOICE_NODE + NODE_GAIN_COUNT, 3).unwrap();
        let r3 = u64::from(VOICE);
        assert_eq!(
            release_voice(&mut g, r3, SP).unwrap(),
            r3 + u64::from(VOICE_NODE)
        );
        for i in 0..3u32 {
            assert_eq!(
                g.f32(OWNER + OWNER_GAINS + 4 * i).unwrap(),
                100.0 + 0.5 * (i as f32 + 1.0)
            );
        }
        assert_eq!(
            g.f32(OWNER + OWNER_GAINS + 12).unwrap(),
            100.0,
            "only three were folded"
        );
        for i in 0..8u32 {
            assert_eq!(
                g.u32(VOICE + VOICE_GAINS + 4 * i).unwrap(),
                0,
                "gain {i} cleared"
            );
        }
        assert_eq!(g.u32(A + LINK_NEXT).unwrap(), C, "and unlinked");
        let frame = SP - RELEASE_FRAME_BYTES;
        assert_eq!(
            g.u32(frame + RELEASE_SOURCES).unwrap(),
            VOICE + VOICE_GAINS,
            "sources point at the voice"
        );
        assert_eq!(
            g.u32(frame + RELEASE_DESTS + 4).unwrap(),
            frame + RELEASE_SCRATCH + 4
        );
    }

    #[test]
    fn repointing_a_link_pushes_the_node_onto_the_source_list() {
        let mut g = voice(0, 0);
        let (link, source, item, other_head) =
            (BASE + 0x40, BASE + 0x1400, BASE + 0x1600, BASE + 0x1800);
        g.set_u32(link + LINK_TARGET, VOICE).unwrap();
        g.set_u32(link + LINK_SOURCE, source).unwrap();
        let owner = source + SOURCE_OWNER;
        g.set_u32(owner, item).unwrap();
        g.set_u32(owner + 8, 0xABCD).unwrap();
        g.set_u16(owner + 44, 0x77).unwrap();
        g.set_u8(item + 42, 6).unwrap();
        g.set_u32(owner + OWNER_HEAD, other_head).unwrap();

        assert_eq!(repoint_link(&mut g, link, SP).unwrap(), 16);

        let node = VOICE + VOICE_NODE;
        assert_eq!(
            g.u32(A + LINK_NEXT).unwrap(),
            C,
            "first released from the old list"
        );
        assert_eq!(g.u32(node + NODE_ITEM).unwrap(), item);
        assert_eq!(g.u32(node + NODE_VALUE).unwrap(), 0xABCD);
        assert_eq!(g.u16(node + NODE_SHORT).unwrap(), 0x77);
        assert_eq!(
            g.u8(node + NODE_GAIN_COUNT).unwrap(),
            6,
            "the item's channel byte"
        );
        assert_eq!(g.u32(node + NODE_OWNER).unwrap(), owner);
        assert_eq!(
            g.u32(node + LINK_NEXT).unwrap(),
            other_head,
            "next is the old head"
        );
        assert_eq!(g.u32(node + LINK_PREV).unwrap(), 0);
        assert_eq!(
            g.u32(other_head + LINK_PREV).unwrap(),
            node,
            "the old head points back"
        );
        assert_eq!(
            g.u32(owner + OWNER_HEAD).unwrap(),
            node,
            "and the node is the new head"
        );
    }

    #[test]
    fn a_checked_unlink_reports_invalid_missing_and_stale_handles() {
        let handle = BASE + 0x40;
        // Already invalid: the generation comes back, sign-extended, and nothing is written.
        let mut g = listed();
        g.set_u32(handle + HANDLE_GENERATION, (-9i32) as u32)
            .unwrap();
        assert_eq!(unlink_checked(&mut g, handle, B).unwrap() as i64, -9);
        // No owner.
        g.set_u32(handle + HANDLE_GENERATION, 5).unwrap();
        g.set_u32(handle + HANDLE_OWNER, 0).unwrap();
        assert_eq!(
            unlink_checked(&mut g, handle, B).unwrap() as i64,
            ERROR_NO_OWNER
        );
        // Stale: the owner's generation at +12 differs, so the handle is invalidated.
        g.set_u32(handle + HANDLE_OWNER, OWNER).unwrap();
        g.set_u32(OWNER + 12, 6).unwrap();
        assert_eq!(
            unlink_checked(&mut g, handle, B).unwrap() as i64,
            ERROR_STALE
        );
        assert_eq!(g.u32(handle + HANDLE_GENERATION).unwrap() as i32, -3);
        assert_eq!(g.u32(handle + HANDLE_OWNER).unwrap(), 0);
        assert_eq!(g.u32(A + LINK_NEXT).unwrap(), B, "nothing unlinked");
    }

    #[test]
    fn a_live_checked_unlink_uses_the_owners_plus_zero_head_and_its_own_generation_offset() {
        let handle = BASE + 0x40;
        for (offset, gen8) in [(12u32, false), (8u32, true)] {
            let mut g = listed();
            g.set_u32(OWNER, A).unwrap(); // this list's head is the owner's +0
            g.set_u32(handle + HANDLE_OWNER, OWNER).unwrap();
            g.set_u32(handle + HANDLE_GENERATION, 5).unwrap();
            g.set_u32(OWNER + offset, 5).unwrap();
            let r = if gen8 {
                unlink_checked_gen8(&mut g, handle, A)
            } else {
                unlink_checked(&mut g, handle, A)
            };
            assert_eq!(r.unwrap(), 0, "generation at +{offset}");
            assert_eq!(g.u32(OWNER).unwrap(), B, "the head moved on");
            assert_eq!(g.u32(B + LINK_PREV).unwrap(), 0);
        }
    }

    #[test]
    fn removing_a_handle_shifts_the_tail_down_and_drops_the_count() {
        let mut g = Guest::single(BASE, 0x1000);
        let (object, owner, array) = (BASE + 0x10, BASE + 0x100, BASE + 0x400);
        g.set_u32(object + HANDLE_ARRAY_OWNER, owner).unwrap();
        g.set_u32(owner + HANDLE_ARRAY, array).unwrap();
        g.set_u16(owner + HANDLE_COUNT, 4).unwrap();
        let entries = [(0xA, 1), (object, 2), (0xC, 3), (0xD, 4)];
        for (i, (who, payload)) in entries.iter().enumerate() {
            g.set_u32(array + 8 * i as u32, *who).unwrap();
            g.set_u32(array + 8 * i as u32 + 4, *payload).unwrap();
        }
        assert_eq!(remove_handle(&mut g, object).unwrap(), 1);
        assert_eq!(g.u16(owner + HANDLE_COUNT).unwrap(), 3);
        let after: Vec<(u32, u32)> = (0..4u32)
            .map(|i| {
                (
                    g.u32(array + 8 * i).unwrap(),
                    g.u32(array + 8 * i + 4).unwrap(),
                )
            })
            .collect();
        // Entries after the removed one move down; the last slot keeps its old bytes.
        assert_eq!(after, vec![(0xA, 1), (0xC, 3), (0xD, 4), (0xD, 4)]);

        // Not found: an object with the same owner that is not in the array returns 0 and writes
        // nothing. (It has to be a real object: the owner is read through it first.)
        let stranger = BASE + 0x20;
        g.set_u32(stranger + HANDLE_ARRAY_OWNER, owner).unwrap();
        assert_eq!(remove_handle(&mut g, stranger).unwrap(), 0);
        assert_eq!(g.u16(owner + HANDLE_COUNT).unwrap(), 3);
    }
}

#[cfg(test)]
mod retire_tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const OBJECT: u32 = BASE + 0x100;
    const OTHER: u32 = BASE + 0x200;
    const FIRST: u32 = BASE + 0x300;
    const OWNER: u32 = BASE + 0x400;
    const H0: u32 = BASE + 0x800;
    const H1: u32 = BASE + 0x840;
    const ARG: u32 = 0x1234_5678;

    /// The object heads its owner's +20 list with OTHER behind it, FIRST alone on the +16 list, two
    /// handles, and an owner handle array that is empty so the removal finds nothing.
    fn guest(state: u8) -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        g.put(
            crate::leaves::ZERO_CELL,
            0.0f32.to_bits().to_be_bytes().to_vec(),
        );
        g.set_u32(OBJECT + HANDLE_ARRAY_OWNER, OWNER).unwrap();
        g.set_u8(OBJECT + RETIRE_STATE, state).unwrap();
        for k in 0..3 {
            g.set_u32(OBJECT + 4 * k, 1.5f32.to_bits()).unwrap();
        }
        let (node, other) = (OBJECT + RETIRE_NODE, OTHER + RETIRE_NODE);
        g.set_u32(OWNER + OWNER_LINKED_HEAD, node).unwrap();
        g.set_u32(node, other).unwrap();
        g.set_u32(other + 4, node).unwrap();
        g.set_u32(OWNER + OWNER_RETIRED_HEAD, FIRST + RETIRE_NODE)
            .unwrap();
        g.set_u8(OBJECT + RETIRE_HANDLE_COUNT, 2).unwrap();
        g.set_u32(OBJECT + RETIRE_ARG_SLOT + 4, H0).unwrap();
        g.set_u32(OBJECT + RETIRE_ARG_SLOT + 8, H1).unwrap();
        g.set_u32(H0 + RETIRE_HANDLE_FIELD, 0x77).unwrap();
        g.set_u32(H1 + RETIRE_HANDLE_FIELD, 0x77).unwrap();
        g
    }

    #[test]
    fn a_retired_object_is_left_alone() {
        let mut g = guest(2);
        retire_object(&mut g, OBJECT, ARG).unwrap();
        assert_eq!(g.f32(OBJECT).unwrap(), 1.5);
        assert_eq!(g.u32(H0 + RETIRE_HANDLE_FIELD).unwrap(), 0x77);
        assert_eq!(
            g.u32(OWNER + OWNER_RETIRED_HEAD).unwrap(),
            FIRST + RETIRE_NODE
        );
    }

    #[test]
    fn a_linked_object_moves_from_one_list_to_the_other_and_is_reset() {
        let mut g = guest(1);
        retire_object(&mut g, OBJECT, ARG).unwrap();
        let node = OBJECT + RETIRE_NODE;
        assert_eq!(
            g.u32(OWNER + OWNER_LINKED_HEAD).unwrap(),
            OTHER + RETIRE_NODE,
            "the head moved on"
        );
        assert_eq!(
            g.u32(OTHER + RETIRE_NODE + 4).unwrap(),
            0,
            "and lost its prev"
        );
        assert_eq!(g.u32(OBJECT + RETIRE_ARG_SLOT).unwrap(), ARG);
        assert_eq!(g.u32(OBJECT + RETIRE_FIELD_60).unwrap(), 0);
        assert_eq!(g.u8(OBJECT + RETIRE_STATE).unwrap(), 2);
        for k in 0..3 {
            assert_eq!(g.f32(OBJECT + 4 * k).unwrap(), 0.0, "gain {k}");
        }
        assert_eq!(g.u32(H0 + RETIRE_HANDLE_FIELD).unwrap(), 0);
        assert_eq!(g.u32(H1 + RETIRE_HANDLE_FIELD).unwrap(), 0);
        assert_eq!(
            g.u32(node).unwrap(),
            FIRST + RETIRE_NODE,
            "pushed in front of the old first"
        );
        assert_eq!(g.u32(node + 4).unwrap(), 0);
        assert_eq!(g.u32(FIRST + RETIRE_NODE + 4).unwrap(), node);
        assert_eq!(g.u32(OWNER + OWNER_RETIRED_HEAD).unwrap(), node);
    }

    #[test]
    fn an_unlinked_object_leaves_the_first_list_alone() {
        let mut g = guest(0);
        retire_object(&mut g, OBJECT, ARG).unwrap();
        assert_eq!(
            g.u32(OWNER + OWNER_LINKED_HEAD).unwrap(),
            OBJECT + RETIRE_NODE
        );
        assert_eq!(
            g.u32(OWNER + OWNER_RETIRED_HEAD).unwrap(),
            OBJECT + RETIRE_NODE
        );
    }
}
