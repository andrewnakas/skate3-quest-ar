//! Patch banks at run time: loading an `.abk`, posting a message to an object, and the listener that
//! turns a post into an evaluator instance.
//!
//! **Unverified new work.** None of this runs on the audio thread (the loader and game threads do),
//! so no C++ body exists and nothing was compared. It follows the lifted code line by line:
//!
//! | function | here |
//! |---|---|
//! | `sub_828DC660`, its `.abk` branch | [`load_bank`] |
//! | `sub_82B1DF50` | [`install_bank`] |
//! | `sub_828E2B48` | [`post`] |
//! | `sub_82B1DAD0` | the listener every input record registers |
//! | `sub_82B1D880` | [`allocate_instance`] |
//! | `sub_828E2FF8` | [`subscribe`] |
//! | `sub_82B1D7E8`, `sub_82B1D7F8`, `sub_82B1D808`, `sub_82B1D840` | the instance's callbacks |
//! | `sub_82B1C150` (evaluator slot 4), `sub_82B1BF98` | [`end_instance`]: unlink and free |
//! | `sub_828E2C78` | [`release_message`] |
//! | `sub_828E2D18` | [`redeliver`]: hand a held message's rewritten payload to its instances again |
//! | `sub_828E2E08`, `sub_828E2EA0` | the unregister helpers (`sub_828E30B8` and `sub_828E2D78` are the verified ports in `voices.rs`) |
//!
//! Evaluator slot 27, the voice op, is in [`crate::voice`]; [`PatchHost`] runs it and the two
//! host-bound slots 4 and 5.
//!
//! What is left out, and why: the audio system's critical section around the installer and the
//! listener, which only serialises threads; the installer's registration of an `AEMS` unload handler
//! (`sub_82B48260`); and, on the first bank, the registration of the interpreter `sub_82B1E290` with
//! the audio system's scheduler (`sub_82B395E8`). The host instead calls [`crate::eval::interp::tick`]
//! once per audio frame. [`install_bank`] returns whether this was the first bank, which is when the
//! original registers it. Guest allocations go through [`Heap`], standing in for the allocator object
//! the original calls through a vtable.
//!
//! Indirect calls are resolved by guest address. A listener or callback this module does not
//! implement is an error naming the address, never a silent skip.
//!
//! The layouts, verified on all 376 banks by `skate-audio-formats`' `verify_patch_records` and
//! `verify_patch_programs`, are in `docs/audio-banks.md`, "From a message to the evaluator".

use crate::eval::TABLE_BASE;
use crate::eval::interp::LIST_HEAD;
use crate::mem::memcpy;
use crate::symbols::{lookup_table0, lookup_table1, lookup_table2};
use crate::voice::{VoiceDevice, voice_op};
use crate::voices::{unlink_checked, unlink_checked_gen8};
use crate::{Error, Guest, Result};

/// `lis -31997` + 28500: the list of installed banks, linked through each bank's `+80`.
pub const BANK_LIST: u32 = 0x8303_6F54;
/// `lis -31987` + -56: the bank id counter.
pub const BANK_ID_COUNTER: u32 = 0x830C_FFC8;
/// The listener every input record registers.
pub const LISTENER: u32 = 0x82B1_DAD0;
/// Instance callback: the posted message was released.
pub const ON_RELEASE: u32 = 0x82B1_D7F8;
/// Instance callback: take a variable's value at subscription.
pub const ON_SUBSCRIBE: u32 = 0x82B1_D7E8;
/// Instance callback: copy the posted payload into the instance.
pub const ON_PAYLOAD: u32 = 0x82B1_D808;
/// Instance callback: copy a broadcast into the instance and flag its arrival.
pub const ON_BROADCAST: u32 = 0x82B1_D840;

/// The installer's stack frame (`stwu r1,-176(r1)`); its query lives at `+80`.
pub const INSTALL_FRAME: u32 = 176;

/// Guest memory the patch runtime allocates from.
pub trait Heap {
    /// Allocate `size` bytes aligned to `align`; 0 when out of memory, as the original's allocator.
    fn alloc(&mut self, g: &mut Guest, size: u32, align: u32) -> Result<u32>;
    /// Free an allocation (the allocator vtable's `+12`).
    fn free(&mut self, g: &mut Guest, at: u32) -> Result<()>;
}

/// A bump allocator over a guest span. Never frees, which is enough for a test or a session.
pub struct BumpHeap {
    pub next: u32,
    pub end: u32,
}

impl Heap for BumpHeap {
    fn alloc(&mut self, _g: &mut Guest, size: u32, align: u32) -> Result<u32> {
        let align = align.max(4);
        let at = (self.next + align - 1) & !(align - 1);
        match at.checked_add(size) {
            Some(end) if end <= self.end => {
                self.next = end;
                Ok(at)
            }
            _ => Ok(0),
        }
    }

    fn free(&mut self, _g: &mut Guest, _at: u32) -> Result<()> {
        Ok(())
    }
}

/// The runtime half of the evaluator table, for [`crate::eval::interp::tick_with`].
pub struct PatchHost<'a> {
    pub heap: &'a mut dyn Heap,
    pub device: &'a mut dyn VoiceDevice,
}

impl crate::eval::interp::Host for PatchHost<'_> {
    fn op(&mut self, g: &mut Guest, opcode: u8, block: u32) -> Option<Result<u64>> {
        match opcode {
            4 => Some(end_instance(g, self.heap, self.device, block)),
            5 => Some(clamp_and_broadcast(g, block)),
            27 => Some(voice_op(g, self.device, block)),
            39 => Some(latch_and_notify(g, block)),
            _ => None,
        }
    }
}

/// Best-effort, non-failing read of a short NUL-terminated guest string, for diagnostics only.
fn read_cstr(g: &Guest, ea: u32, max: u32) -> String {
    let mut out = Vec::new();
    for i in 0..max {
        match g.u8(ea + i) {
            Ok(0) | Err(_) => break,
            Ok(b) => out.push(b),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `ea..ea+len` falls inside any mapped guest segment, without dereferencing it.
fn is_mapped(g: &Guest, ea: u32, len: u32) -> bool {
    g.segments()
        .iter()
        .any(|s| ea >= s.base && (ea - s.base) as u64 + len as u64 <= s.bytes.len() as u64)
}

/// `sub_828E29C0`: validate a table-1 broadcast handle, then call every subscriber with `values`.
///
/// The handle is `{symbol, generation}`. The symbol owns the subscriber-list head at `+0` and its
/// current generation at `+8`; subscriber nodes are `{next, prev, function, context}`. The recovered
/// player banks use [`ON_BROADCAST`], whose callback copies the words and sets the arrival flag.
/// Expected handle failures are returned as the guest's negative status rather than promoted to a
/// host error. An unknown callback remains an explicit error.
///
/// `symbol` is either a table-1 export written by [`install_bank`]'s export loop
/// (`lookup_table1`), or, in at least one confirmed shipped record (`Rolling_Rattle_Class` in
/// `Rolling_Rattles.abk`), a small compiled placeholder (observed value `1`) that stays in place
/// until some other, not-yet-identified retail object broadcasts a real value through it. Real
/// retail traces (`D:\skate3-audio-captures\...\retail-audio*.log`) show the live Xbox 360
/// process carries this exact same placeholder across dozens of captured deliveries of the same
/// object while rattle audio keeps playing, which is not a mappable guest/process address either
/// — so retail's own `sub_828E29C0` must already tolerate it as gracefully as it tolerates
/// `symbol == 0` below. A guest lookup failure would also leave the same kind of raw literal in
/// place. Either way, an unmapped `symbol` is treated as "no holder yet" (status `-6`, silently
/// skipping the broadcast) instead of being dereferenced, which previously surfaced only as an
/// opaque "no segment covers this address" host fault that killed the whole audio worker. Set
/// `SKATE_AUDIO_BROADCAST_TRACE=1` to log every time this path is taken, for tracking down the
/// real broadcaster later (see docs/wheel-audio-handoff-2026-09-17.md).
fn broadcast(g: &mut Guest, binding: u32, values: u32) -> Result<i32> {
    let generation = g.u32(binding + 4)?;
    if (generation as i32) < 0 {
        return Ok(generation as i32);
    }
    let symbol = g.u32(binding)?;
    if symbol == 0 {
        return Ok(-6);
    }
    if !is_mapped(g, symbol, 12) {
        if std::env::var_os("SKATE_AUDIO_BROADCAST_TRACE").is_some() {
            eprintln!(
                "SKATE_AUDIO_BROADCAST skipped: binding {binding:#010x} has unresolved symbol \
                 {symbol:#010x} (generation {generation:#010x})"
            );
        }
        return Ok(-6);
    }
    if generation != g.u32(symbol + 8)? {
        g.set_u32(binding + 4, (-3i32) as u32)?;
        g.set_u32(binding, 0)?;
        return Ok(-3);
    }
    let mut subscriber = g.u32(symbol)?;
    if subscriber == 0 {
        return Ok(-4);
    }
    while subscriber != 0 {
        let function = g.u32(subscriber + 8)?;
        let context = g.u32(subscriber + 12)?;
        match function {
            ON_BROADCAST => on_broadcast(g, values, context)?,
            other => {
                return Err(Error::new(
                    other,
                    format!("broadcast callback {other:#010x} is not implemented"),
                ));
            }
        }
        subscriber = g.u32(subscriber)?;
    }
    Ok(0)
}

/// Evaluator slot 5, `sub_82B1C210`: optionally clamp each outgoing word to its signed range,
/// then broadcast the words through the generation-checked table-1 handle. Always returns zero;
/// the guest deliberately ignores the broadcast helper's ordinary negative status codes.
fn clamp_and_broadcast(g: &mut Guest, block: u32) -> Result<u64> {
    let ranged = g.u8(block + 8)? != 0;
    let count = g.u8(block + 9)? as u32;
    let binding = if ranged {
        block + 12 + count * 8
    } else {
        block + 12
    };
    if ranged {
        for i in 0..count {
            let range = block + 12 + i * 8;
            let minimum = g.u32(range)? as i32;
            let maximum = g.u32(range + 4)? as i32;
            let at = binding + 4 + i * 4;
            let value = g.u32(at)? as i32;
            let value = if value < minimum {
                minimum
            } else if value > maximum {
                maximum
            } else {
                value
            };
            g.set_u32(at, value as u32)?;
        }
    }
    if g.u32(binding)? != 0 {
        let _ = broadcast(g, binding, binding + 4)?;
    }
    Ok(0)
}

/// Evaluator slot 39, `sub_82B1C450`: publish a changed requested setpoint.
///
/// The operand is both a `{holder, generation}` handle at `+0/+4` and the
/// `{low, high, applied, requested}` control at `+8..+20`. The retail helper
/// (`sub_828E2F38`) gives each registered table-2 listener the address of the
/// holder's value cell. Player banks use the known `ON_SUBSCRIBE` callback for
/// this path, which copies that word into the listener state at `+24`. An unmapped `holder` is
/// treated as "no holder yet" rather than dereferenced; see [`broadcast`]'s doc comment for the
/// retail evidence behind that choice.
fn latch_and_notify(g: &mut Guest, block: u32) -> Result<u64> {
    let requested = g.u32(block + 20)? as i32;
    if g.u32(block + 16)? as i32 == requested {
        return Ok(0);
    }
    let low = g.u32(block + 8)? as i32;
    let high = g.u32(block + 12)? as i32;
    // The applied word retains the raw request; only the published value is clamped.
    g.set_u32(block + 16, requested as u32)?;
    let published = requested.clamp(low, high) as u32;

    let generation = g.u32(block + 4)?;
    if (generation as i32) < 0 {
        return Ok(0);
    }
    let holder = g.u32(block)?;
    if holder == 0 {
        return Ok(0);
    }
    if !is_mapped(g, holder, 16) {
        // Same reasoning as broadcast()'s symbol check above: an unmapped holder is treated as
        // "no holder yet" rather than dereferenced and crashed on.
        if std::env::var_os("SKATE_AUDIO_BROADCAST_TRACE").is_some() {
            eprintln!(
                "SKATE_AUDIO_BROADCAST skipped: slot-39 handle at {block:#010x} has unresolved \
                 holder {holder:#010x} (generation {generation:#010x})"
            );
        }
        return Ok(0);
    }
    if generation as i32 != g.u32(holder + 12)? as i32 {
        g.set_u32(block + 4, (-3i32) as u32)?;
        g.set_u32(block, 0)?;
        return Ok(0);
    }
    if g.u32(holder + 4)? == published {
        return Ok(0);
    }
    let mut listener = g.u32(holder)?;
    g.set_u32(holder + 4, published)?;
    while listener != 0 {
        let function = g.u32(listener + 8)?;
        let context = g.u32(listener + 12)?;
        match function {
            ON_SUBSCRIBE => g.set_u32(context + 24, published)?,
            other => {
                return Err(Error::new(
                    other,
                    format!("slot-39 listener callback {other:#010x} is not implemented"),
                ));
            }
        }
        listener = g.u32(listener)?;
    }
    Ok(0)
}

/// Unlink a `{next, prev}` item from its neighbours, the way every helper here does: the previous
/// item's next first, then the next item's prev, each tested (signed, equality only) for null.
fn unlink(g: &mut Guest, item: u32) -> Result<()> {
    let prev = g.u32(item + 4)?;
    if prev as i32 != 0 {
        let next = g.u32(item)?;
        g.set_u32(prev, next)?;
    }
    let next = g.u32(item)?;
    if next as i32 != 0 {
        let prev = g.u32(item + 4)?;
        g.set_u32(next + 4, prev)?;
    }
    Ok(())
}

/// Evaluator slot 4, `sub_82B1C150`: when `triple+12` is set, unlink the instance from its record's
/// live list and from the interpreter's list, then free it. `triple` is the instance's
/// `{record, instance, post node}` back-pointer block. Always returns 0.
pub fn end_instance(
    g: &mut Guest,
    heap: &mut dyn Heap,
    device: &mut dyn VoiceDevice,
    triple: u32,
) -> Result<u64> {
    if g.u32(triple + 12)? as i32 == 0 {
        return Ok(0);
    }
    let record = g.u32(triple)?;
    let instance = g.u32(triple + 4)?;
    let head = g.u32(record + 56)?;
    if instance == head {
        let next = g.u32(head)?;
        g.set_u32(record + 56, next)?;
    }
    unlink(g, instance)?;
    let node = g.u32(triple + 4)?.wrapping_add(8);
    let head = g.u32(LIST_HEAD)?;
    if node == head {
        let next = g.u32(head)?;
        g.set_u32(LIST_HEAD, next)?;
    }
    unlink(g, node)?;
    free_instance(g, heap, device, triple)?;
    Ok(0)
}

/// `sub_82B1BF98`: take an instance's entries off every list they joined, release what it owns,
/// drop the record's live count and free the instance.
fn free_instance(
    g: &mut Guest,
    heap: &mut dyn Heap,
    device: &mut dyn VoiceDevice,
    triple: u32,
) -> Result<()> {
    let record = g.u32(triple)?;
    let mut entry = g.u32(triple + 4)?.wrapping_add(24);
    if g.u8(record + 37)? != 0 {
        let node = g.u32(triple + 8)?;
        unregister_node_callback(g, heap, node, entry, 12)?; // sub_828E2E08
        entry = entry.wrapping_add(20);
    }
    let mut record = g.u32(triple)?;
    if g.u16(record + 32)? != 0 {
        let mut i = 0i32;
        loop {
            unlink_checked(g, entry, entry + 8)?; // sub_828E30B8, verified in voices.rs
            record = g.u32(triple)?;
            i += 1;
            entry = entry.wrapping_add(28);
            if !(i < g.u16(record + 32)? as i32) {
                break;
            }
        }
    }
    let mut next = entry;
    if g.u8(record + 38)? != 0 {
        let node = g.u32(triple + 8)?;
        unregister_node_callback(g, heap, node, entry, 8)?; // sub_828E2EA0
        next = (((g.u8(entry + 16)? as u32 + 5) << 2) & !3).wrapping_add(entry);
    }
    record = g.u32(triple)?;
    if g.u16(record + 34)? != 0 {
        let mut i = 0i32;
        loop {
            unlink_checked_gen8(g, next, next + 8)?; // sub_828E2D78, verified in voices.rs
            let words = g.u8(next + 24)? as u32;
            record = g.u32(triple)?;
            i += 1;
            let count = g.u16(record + 34)? as i32;
            next = (((words + 7) << 2) & !3).wrapping_add(next);
            if !(i < count) {
                break;
            }
        }
    }
    let mut cursor = record + 60;
    if g.u8(record + 36)? != 0 {
        let mut i = 0i32;
        loop {
            let off = g.u32(cursor)?;
            cursor += 4;
            let object = off.wrapping_add(g.u32(triple + 4)?);
            let voice = g.u32(object + 8)?;
            if voice != 0 {
                device.release(g, voice)?; // the voice's vtable slot 0
                g.set_u32(object + 8, 0)?; // stw r28,8(r30)
            }
            record = g.u32(triple)?;
            i += 1;
            if !(i < g.u8(record + 36)? as i32) {
                break;
            }
        }
    }
    if g.u8(record + 39)? != 0 {
        let mut i = 0i32;
        let mut at = cursor.wrapping_sub(4);
        loop {
            at = at.wrapping_add(4);
            let object = g.u32(at)?.wrapping_add(g.u32(triple + 4)?);
            let message = g.u32(object + 8)?;
            if message != 0 {
                release_message(g, heap, message)?; // sub_828E2C78
            }
            record = g.u32(triple)?;
            i += 1;
            if !(i < g.u8(record + 39)? as i32) {
                break;
            }
        }
    }
    let live = g.u16(record + 28)?.wrapping_sub(1);
    g.set_u16(record + 28, live)?;
    let instance = g.u32(triple + 4)?;
    heap.free(g, instance) // allocator vtable +12
}

/// `sub_828E2E08` (`list_at` 12) and `sub_828E2EA0` (`list_at` 8): take `item` off one of a post
/// node's callback lists, drop the node's reference and free it at zero.
fn unregister_node_callback(
    g: &mut Guest,
    heap: &mut dyn Heap,
    node: u32,
    item: u32,
    list_at: u32,
) -> Result<()> {
    let head = g.u32(node + list_at)?;
    if item == head {
        let next = g.u32(head)?;
        g.set_u32(node + list_at, next)?;
    }
    unlink(g, item)?;
    let refs = g.u32(node + 4)?.wrapping_sub(1); // addic. r11,r11,-1
    g.set_u32(node + 4, refs)?;
    if refs == 0 {
        heap.free(g, node)?;
    }
    Ok(())
}

/// `sub_828E2C78`: release a post node. Run its release callbacks, drop its reference, and free it
/// at zero. Always returns 0.
pub fn release_message(g: &mut Guest, heap: &mut dyn Heap, node: u32) -> Result<i32> {
    let mut callback = g.u32(node + 12)?;
    while callback != 0 {
        let function = g.u32(callback + 8)?;
        let next = g.u32(callback)?;
        let ctx = g.u32(callback + 12)?;
        match function {
            ON_RELEASE => on_release(g, ctx)?,
            other => {
                return Err(Error::new(
                    other,
                    format!("release callback {other:#010x} is not implemented"),
                ));
            }
        }
        callback = next;
    }
    let refs = g.u32(node + 4)?.wrapping_sub(1);
    g.set_u32(node + 4, refs)?;
    if refs == 0 {
        heap.free(g, node)?;
    }
    Ok(0)
}

fn link_head(g: &mut Guest, head_cell: u32, node: u32) -> Result<()> {
    // The shape every list insert here takes: node.next = head; node.prev = 0; head.prev = node;
    // head = node, with the head reloaded before the back link.
    let head = g.u32(head_cell)?;
    g.set_u32(node + 4, 0)?;
    g.set_u32(node, head)?;
    let head = g.u32(head_cell)?;
    if head != 0 {
        g.set_u32(head + 4, node)?;
    }
    g.set_u32(head_cell, node)
}

/// The `.abk` branch of `sub_828DC660`: give the bank an id, point `+64` at its sample bank, and
/// install it. Returns the id and whether this was the first bank.
pub fn load_bank(g: &mut Guest, bank: u32, sp: u32) -> Result<(u32, bool)> {
    let mut id = g.u32(BANK_ID_COUNTER)?.wrapping_add(1); // addic. r11,r11,1
    g.set_u32(BANK_ID_COUNTER, id)?;
    if (id as i32) < 0 {
        id = 1;
        g.set_u32(BANK_ID_COUNTER, 1)?;
    }
    let samples = g.u32(bank + 32)?; // lwz r10,32(r31)
    g.set_u32(bank + 60, id)?; // stw r11,60(r31)
    if samples != 0 {
        g.set_u32(bank + 64, samples.wrapping_add(bank))?;
    }
    let fixups = g.u32(bank + 48)?.wrapping_add(bank); // lwz r11,48(r31) ; add r4,r11,r31
    let first = install_bank(g, bank, fixups, sp)?;
    Ok((g.u32(bank + 60)?, first))
}

/// `sub_82B1DF50`: install the bank at `bank`, whose fixup lists start at `fixups`. `sp` is the
/// caller's stack pointer. Returns whether no bank was installed before, which is when the original
/// registers the interpreter.
pub fn install_bank(g: &mut Guest, bank: u32, fixups: u32, sp: u32) -> Result<bool> {
    let node = bank + 80;
    let head = g.u32(BANK_LIST)?; // lwz r10,8(r26)
    g.set_u32(bank + 84, 0)?; // stw r25,84(r31)
    let first = head == 0; // cntlzw ; rlwinm r24,r9,27,31,31
    g.set_u32(bank + 80, head)?; // stw r10,80(r31)
    let head = g.u32(BANK_LIST)?;
    if head != 0 {
        g.set_u32(head + 4, node)?;
    }
    g.set_u32(BANK_LIST, node)?;
    g.set_u32(bank + 68, 0)?; // stw r25,68(r31)

    // Code references: each word, an opcode index, becomes a branch offset to its op. Empty on
    // every shipped bank (`verify_bank_fixups`), reproduced anyway.
    if (g.u32(fixups)? as i32) > 0 {
        let (mut i, mut at) = (0i32, fixups);
        loop {
            at += 4;
            let off = g.u32(at)?;
            i += 1;
            let site = off.wrapping_add(bank);
            let index = g.u32(site)?;
            let target = g.u32(((index << 2) & !3).wrapping_add(TABLE_BASE))?;
            g.set_u32(site, target.wrapping_sub(site).wrapping_sub(4))?;
            if !(i < g.u32(fixups)? as i32) {
                break;
            }
        }
    }

    // Rebases: 4,656 words across the shipped banks, all into the first section.
    let base = fixups.wrapping_sub(g.u32(bank + 48)?); // subf r27,r9,r30
    let rebases = g.u32(bank + 52)?.wrapping_add(base);
    if (g.u32(rebases)? as i32) > 0 {
        let (mut i, mut at) = (0i32, rebases);
        loop {
            at += 4;
            let off = g.u32(at)?;
            i += 1;
            let site = off.wrapping_add(bank);
            let word = g.u32(site)?;
            g.set_u32(site, word.wrapping_add(bank))?;
            if !(i < g.u32(rebases)? as i32) {
                break;
            }
        }
    }

    // Exports: resolve each into its slot, choosing the table by the kind word's top byte.
    let exports = g.u32(bank + 56)?.wrapping_add(base);
    if (g.u32(exports)? as i32) > 0 {
        let query = sp.wrapping_sub(INSTALL_FRAME) + 80;
        let (mut j, mut entry) = (0i32, exports + 4);
        loop {
            let record = g.u32(entry + 4)?.wrapping_add(base);
            let kind = g.u8(entry + 8)?;
            g.set_u32(query, record + 4)?; // stw r9,80(r1)
            let project = g.u16(record)?;
            let name_id = g.u16(record + 2)?;
            let slot = g.u32(entry)?.wrapping_add(bank);
            g.set_u16(query + 4, project)?;
            g.set_u16(query + 6, name_id)?;
            let resolved = match kind {
                0 => lookup_table2(g, slot, query)?,
                1 => lookup_table1(g, slot, query)?,
                _ => lookup_table0(g, slot, query)?,
            };
            if resolved.status != 0 && std::env::var_os("SKATE_AUDIO_EXPORT_TRACE").is_some() {
                eprintln!(
                    "SKATE_AUDIO_EXPORT unresolved bank={bank:#010x} slot={slot:#010x} kind={kind} \
                     project={project:#06x} name_id={name_id:#06x} name={:?}",
                    read_cstr(g, record + 4, 64)
                );
            }
            j += 1;
            entry += 12;
            if !(j < g.u32(exports)? as i32) {
                break;
            }
        }
    }

    // Input records: relocate, register the listener on the bound symbol, and give each instance
    // template its back-pointers to the bank.
    let mut record = g.u32(bank + 28)?.wrapping_add(bank);
    if g.u16(bank + 10)? != 0 {
        let mut r = 0i32;
        loop {
            let program = g.u32(record + 40)?;
            let template = g.u32(record + 44)?;
            g.set_u32(record + 24, record)?;
            g.set_u32(record + 40, program.wrapping_add(bank))?;
            g.set_u32(record + 20, LISTENER)?;
            g.set_u32(record + 44, template.wrapping_add(bank))?;
            let id = g.u32(record + 8)?;
            if (id as i32) >= 0 {
                let symbol = g.u32(record + 4)?;
                if symbol != 0 {
                    if id as i32 != g.u32(symbol + 8)? as i32 {
                        g.set_u32(record + 8, (-3i32) as u32)?;
                        g.set_u32(record + 4, 0)?;
                    } else {
                        link_head(g, symbol, record + 12)?;
                    }
                }
            }
            let (mut i, mut at) = (0i32, record + 56);
            if g.u8(record + 36)? != 0 {
                loop {
                    at += 4;
                    let off = g.u32(at)?;
                    i += 1;
                    let template = g.u32(record + 44)?;
                    g.set_u32(off.wrapping_add(template), bank)?; // stwx r31,r8,r4
                    if !(i < g.u8(record + 36)? as i32) {
                        break;
                    }
                }
            }
            let entries = g.u8(record + 39)? as u32 + g.u8(record + 36)? as u32;
            r += 1;
            let count = g.u16(bank + 10)? as i32;
            record = ((entries << 2) & !3).wrapping_add(record + 60);
            if !(r < count) {
                break;
            }
        }
    }
    g.set_u32(bank + 72, 0)?; // stw r25,72(r31)
    Ok(first)
}

/// `sub_828E2B48`: post the message whose payload is at `payload` to the object in `slot`. Returns
/// 0, the slot's own negative id, -6 for an empty slot, -3 for a stale one, or -1 when the heap is
/// out of memory. On success `message` receives the post's node.
pub fn post(
    g: &mut Guest,
    heap: &mut dyn Heap,
    slot: u32,
    payload: u32,
    message: u32,
) -> Result<i32> {
    g.set_u32(message, 0)?; // stw r29,0(r5)
    let id = g.u32(slot + 4)?;
    if (id as i32) < 0 {
        return Ok(id as i32);
    }
    let symbol = g.u32(slot)?;
    if symbol == 0 {
        return Ok(-6);
    }
    if id as i32 != g.u32(symbol + 8)? as i32 {
        g.set_u32(slot, 0)?;
        g.set_u32(slot + 4, (-3i32) as u32)?;
        return Ok(-3);
    }
    let node = heap.alloc(g, 16, 4)?; // allocator vtable +8: 16 bytes, tag 0x8217474C
    if node == 0 {
        return Ok(-1);
    }
    g.set_u32(node + 12, 0)?;
    g.set_u32(node + 8, 0)?;
    g.set_u32(node + 4, 1)?;
    let symbol = g.u32(slot)?;
    g.set_u32(node, symbol)?;
    let mut listener = g.u32(symbol)?;
    while listener != 0 {
        let function = g.u32(listener + 8)?;
        let ctx = g.u32(listener + 12)?;
        call_listener(g, heap, function, node, payload, ctx)?;
        listener = g.u32(listener)?;
    }
    let mut callback = g.u32(node + 8)?;
    while callback != 0 {
        let function = g.u32(callback + 8)?;
        let ctx = g.u32(callback + 12)?;
        call_payload_callback(g, function, payload, ctx)?;
        callback = g.u32(callback)?;
    }
    g.set_u32(message, node)?;
    Ok(0)
}

/// `sub_828E2D18`: re-deliver a held message. The game rewrites the payload of a message it keeps
/// (a grind's volume, cutoffs and speed, every frame) and calls this, which runs each payload
/// callback on the post node's `+8` list again. Returns 0.
pub fn redeliver(g: &mut Guest, node: u32, payload: u32) -> Result<i32> {
    let mut callback = g.u32(node + 8)?;
    while callback != 0 {
        let function = g.u32(callback + 8)?;
        let ctx = g.u32(callback + 12)?;
        call_payload_callback(g, function, payload, ctx)?;
        callback = g.u32(callback)?; // reloaded after the call
    }
    Ok(0)
}

fn call_listener(
    g: &mut Guest,
    heap: &mut dyn Heap,
    function: u32,
    node: u32,
    payload: u32,
    ctx: u32,
) -> Result<()> {
    match function {
        LISTENER => listener(g, heap, node, payload, ctx),
        other => Err(Error::new(
            other,
            format!("listener {other:#010x} is not implemented"),
        )),
    }
}

fn call_payload_callback(g: &mut Guest, function: u32, payload: u32, ctx: u32) -> Result<()> {
    match function {
        ON_PAYLOAD => copy_words(g, payload, ctx, 16, 20, None),
        other => Err(Error::new(
            other,
            format!("payload callback {other:#010x} is not implemented"),
        )),
    }
}

/// `sub_82B1DAD0`: a post reached an input record; spawn an instance if there is capacity.
fn listener(
    g: &mut Guest,
    heap: &mut dyn Heap,
    node: u32,
    _payload: u32,
    record: u32,
) -> Result<()> {
    let capacity = g.u16(record + 30)? as i16;
    let live = g.u16(record + 28)? as i16;
    if live < capacity {
        let instance = allocate_instance(g, heap, node, record)?;
        if instance != 0 {
            // The instance's interpreter node is its +8: {next, prev, program, block}.
            let head = g.u32(LIST_HEAD)?;
            g.set_u32(instance + 12, 0)?;
            g.set_u32(instance + 8, head)?;
            let head = g.u32(LIST_HEAD)?;
            if head != 0 {
                g.set_u32(head + 4, instance + 8)?;
            }
            g.set_u32(LIST_HEAD, instance + 8)?;
        }
    }
    Ok(())
}

/// `sub_82B1D880`: copy `record`'s template into a new instance and wire its entries to the post
/// `node` and to the symbols they subscribe to. Returns the instance, or 0.
pub fn allocate_instance(
    g: &mut Guest,
    heap: &mut dyn Heap,
    node: u32,
    record: u32,
) -> Result<u32> {
    let size = g.u32(record + 48)?;
    let instance = heap.alloc(g, size, 16)?; // allocator vtable +4: size, tag 0x82124690, align 16
    if instance == 0 {
        return Ok(0);
    }
    let len = g.u32(record + 48)?;
    let template = g.u32(record + 44)?;
    memcpy(g, instance, template, len as u64)?;
    let back = g.u32(record + 52)?.wrapping_add(instance);
    g.set_u32(back, record)?;
    g.set_u32(back + 4, instance)?;
    g.set_u32(back + 8, node)?;
    let live = g.u32(record + 56)?;
    g.set_u32(instance, live)?;
    g.set_u32(instance + 4, 0)?;
    let live = g.u32(record + 56)?;
    if live != 0 {
        g.set_u32(live + 4, instance)?;
    }
    g.set_u32(record + 56, instance)?;
    let mut entry = instance + 24;
    let program = g.u32(record + 40)?;
    g.set_u32(instance + 16, program)?;
    g.set_u32(instance + 20, entry)?;

    if g.u8(record + 37)? != 0 {
        // A release callback on the post node's +12 list.
        g.set_u32(entry + 12, entry)?;
        g.set_u32(entry + 8, ON_RELEASE)?;
        let head = g.u32(node + 12)?;
        g.set_u32(entry, head)?;
        g.set_u32(entry + 4, 0)?;
        let head = g.u32(node + 12)?;
        if head != 0 {
            g.set_u32(head + 4, entry)?;
        }
        g.set_u32(node + 12, entry)?;
        entry += 20;
        let refs = g.u32(node + 4)?.wrapping_add(1);
        g.set_u32(node + 4, refs)?;
    }

    if g.u16(record + 32)? != 0 {
        // 28-byte variable subscriptions.
        let mut i = 0i32;
        loop {
            g.set_u32(entry + 20, entry)?;
            g.set_u32(entry + 16, ON_SUBSCRIBE)?;
            subscribe(g, entry, entry + 8)?;
            i += 1;
            entry += 28;
            if !(i < g.u16(record + 32)? as i32) {
                break;
            }
        }
    }

    let mut next = entry;
    if g.u8(record + 38)? != 0 {
        // A payload copy on the post node's +8 list, followed by the words it fills.
        g.set_u32(entry + 12, entry)?;
        g.set_u32(entry + 8, ON_PAYLOAD)?;
        let head = g.u32(node + 8)?;
        g.set_u32(entry + 4, 0)?;
        g.set_u32(entry, head)?;
        let head = g.u32(node + 8)?;
        if head != 0 {
            g.set_u32(head + 4, entry)?;
        }
        g.set_u32(node + 8, entry)?;
        let refs = g.u32(node + 4)?.wrapping_add(1);
        g.set_u32(node + 4, refs)?;
        next = (((g.u8(entry + 16)? as u32 + 5) << 2) & !3).wrapping_add(entry);
    }

    if g.u16(record + 34)? != 0 {
        // Broadcast subscriptions: {slot, node, fn, ctx, count, words...}.
        let mut i = 0i32;
        let mut e = next;
        loop {
            let id = g.u32(e + 4)?;
            g.set_u32(e + 20, e)?;
            g.set_u32(e + 16, ON_BROADCAST)?;
            if (id as i32) >= 0 {
                let symbol = g.u32(e)?;
                if symbol != 0 {
                    if id as i32 != g.u32(symbol + 8)? as i32 {
                        g.set_u32(e + 4, (-3i32) as u32)?;
                        g.set_u32(e, 0)?;
                    } else {
                        link_head(g, symbol, e + 8)?;
                    }
                }
            }
            let words = g.u8(e + 24)? as u32;
            i += 1;
            let count = g.u16(record + 34)? as i32;
            e = (((words + 7) << 2) & !3).wrapping_add(e);
            if !(i < count) {
                break;
            }
        }
    }

    let live = g.u16(record + 28)?.wrapping_add(1);
    g.set_u16(record + 28, live)?;
    Ok(instance)
}

/// `sub_828E2FF8`: subscribe `node` to the table-2 symbol bound in `binding`, then hand its callback
/// the variable's value. Returns 0, the binding's own negative id, -6 or -3.
pub fn subscribe(g: &mut Guest, binding: u32, node: u32) -> Result<i32> {
    let id = g.u32(binding + 4)?;
    if (id as i32) < 0 {
        return Ok(id as i32);
    }
    let symbol = g.u32(binding)?;
    if symbol == 0 {
        return Ok(-6);
    }
    if id as i32 != g.u32(symbol + 12)? as i32 {
        g.set_u32(binding + 4, (-3i32) as u32)?;
        g.set_u32(binding, 0)?;
        return Ok(-3);
    }
    let head = g.u32(symbol)?;
    g.set_u32(node + 4, 0)?;
    g.set_u32(node, head)?;
    let head = g.u32(symbol)?;
    if head != 0 {
        g.set_u32(head + 4, node)?;
    }
    g.set_u32(symbol, node)?;
    let ctx = g.u32(node + 12)?;
    let function = g.u32(node + 8)?;
    match function {
        // sub_82B1D7E8: lwz r11,0(r3) ; stw r11,24(r4), with r3 = the symbol's +4 value word.
        ON_SUBSCRIBE => {
            let value = g.u32(symbol + 4)?;
            g.set_u32(ctx + 24, value)?;
        }
        other => {
            return Err(Error::new(
                other,
                format!("subscription callback {other:#010x} is not implemented"),
            ));
        }
    }
    Ok(0)
}

/// `sub_82B1D808` (count at `+16`, words from `+20`) and `sub_82B1D840` (count at `+24`, words from
/// `+28`, then the arrival flag at `+25`): copy `count` words from `src` into the entry. The count
/// is reloaded on every trip.
fn copy_words(
    g: &mut Guest,
    src: u32,
    ctx: u32,
    count_at: u32,
    dst_at: u32,
    flag_at: Option<u32>,
) -> Result<()> {
    let mut i = 0i32;
    if g.u8(ctx + count_at)? != 0 {
        loop {
            let word = g.u32(src.wrapping_add(4 * i as u32))?; // lwzu r8,4(r10)
            g.set_u32(ctx + dst_at + 4 * i as u32, word)?; // stwu r8,4(r9)
            i += 1;
            if !(i < g.u8(ctx + count_at)? as i32) {
                break;
            }
        }
    }
    if let Some(flag) = flag_at {
        g.set_u8(ctx + flag, 1)?;
    }
    Ok(())
}

/// `sub_82B1D840`, for a broadcaster: copy the broadcast values into the entry and flag arrival.
pub fn on_broadcast(g: &mut Guest, values: u32, ctx: u32) -> Result<()> {
    copy_words(g, values, ctx, 24, 28, Some(25))
}

/// `sub_82B1D7F8`, for a releaser: `stw 1,16(ctx)`.
pub fn on_release(g: &mut Guest, ctx: u32) -> Result<()> {
    g.set_u32(ctx + 16, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::{GENERATION, PROJECT_LIST_HEAD, install_project};
    use crate::voice::NoDevice;

    const MEM: u32 = 0x5000_0000;
    const CSI: u32 = MEM;
    const BANK: u32 = MEM + 0x1000;
    const HEAP: u32 = MEM + 0x4000;
    const STACK: u32 = MEM + 0x8000;
    const SLOT: u32 = MEM + 0x900;
    const MSG: u32 = MEM + 0x920;

    fn guest() -> Guest {
        let mut g = Guest::single(MEM, 0x9000);
        g.put(PROJECT_LIST_HEAD, vec![0; 4]);
        g.put(GENERATION, vec![0; 2]);
        g.put(0x8303_6F40, vec![0; 0x40]); // LIST_HEAD and BANK_LIST
        g.put(BANK_ID_COUNTER, vec![0; 4]);
        g
    }

    fn w(g: &mut Guest, at: u32, words: &[u32]) {
        for (i, v) in words.iter().enumerate() {
            g.set_u32(at + 4 * i as u32, *v).unwrap();
        }
    }

    /// A project with one table-1 symbol, `Class_grind`, id 0x09C5.
    fn project(g: &mut Guest) {
        g.set_u16(CSI + 12, 1).unwrap();
        g.set_u16(CSI + 16, 0x64BD).unwrap();
        g.set_u32(CSI + 40 + 4, 64).unwrap();
        g.set_u16(CSI + 40 + 8, 0x09C5).unwrap();
        g.set_span(CSI + 64, b"Class_grind\0").unwrap();
        install_project(g, CSI).unwrap();
    }

    /// A bank with one input record at 0x5C: capacity 2, a 64-byte template at 0x200 with one
    /// back-pointer at +8, a payload copy of two words, one rebased word at 0x300, and one export
    /// binding the record's slot to `Class_grind` under an unshipped project.
    fn bank(g: &mut Guest) {
        g.set_u16(BANK + 10, 1).unwrap();
        w(g, BANK + 28, &[0x5C]);
        w(g, BANK + 48, &[0x400, 0x404, 0x410]);
        let rec = BANK + 0x5C;
        g.set_u16(rec + 30, 2).unwrap();
        g.set_u8(rec + 36, 1).unwrap(); // one back-pointer entry
        g.set_u8(rec + 38, 1).unwrap(); // a payload copy
        // program, template, size 96, back-pointer triple at +72, no live instances, one object at +56
        w(g, rec + 40, &[0x180, 0x200, 96, 72, 0, 56]);
        g.set_u8(BANK + 0x200 + 24 + 16, 2).unwrap(); // the payload copy takes two words
        w(g, BANK + 0x300, &[0x123]);
        w(g, BANK + 0x400, &[0]); // no code references
        w(g, BANK + 0x404, &[1, 0x300]); // one rebase
        w(g, BANK + 0x410, &[1, 0x60, 0x500, 0x0100_0000]); // export: slot at 0x60, table 1
        g.set_u16(BANK + 0x500, 0x63D9).unwrap();
        g.set_u16(BANK + 0x502, 0x09C5).unwrap();
        g.set_span(BANK + 0x504, b"Class_grind\0").unwrap();
    }

    fn installed() -> (Guest, BumpHeap) {
        let mut g = guest();
        project(&mut g);
        bank(&mut g);
        let (id, first) = load_bank(&mut g, BANK, STACK).unwrap();
        assert_eq!((id, first), (1, true));
        (
            g,
            BumpHeap {
                next: HEAP,
                end: HEAP + 0x1000,
            },
        )
    }

    #[test]
    fn installing_rebases_binds_relocates_and_registers_the_listener() {
        let (g, _) = installed();
        let rec = BANK + 0x5C;
        assert_eq!(g.u32(BANK + 0x300).unwrap(), 0x123 + BANK, "the rebase");
        assert_eq!(
            g.u32(rec + 4).unwrap(),
            CSI + 40,
            "the export bound by name on the second pass"
        );
        assert_eq!(g.u32(rec + 40).unwrap(), BANK + 0x180);
        assert_eq!(g.u32(rec + 44).unwrap(), BANK + 0x200);
        assert_eq!(g.u32(rec + 20).unwrap(), LISTENER);
        assert_eq!(
            g.u32(rec + 24).unwrap(),
            rec,
            "the listener's context is the record"
        );
        assert_eq!(
            g.u32(CSI + 40).unwrap(),
            rec + 12,
            "the listener node heads the symbol's list"
        );
        assert_eq!(
            g.u32(BANK + 0x200 + 56).unwrap(),
            BANK,
            "the template's back-pointer"
        );
        assert_eq!(g.u32(BANK_LIST).unwrap(), BANK + 80);
    }

    fn resolve_game_slot(g: &mut Guest) {
        let q = MEM + 0x940;
        g.set_span(q + 8, b"Class_grind\0").unwrap();
        w(g, q, &[q + 8, 0x64BD_09C5]);
        assert_eq!(lookup_table1(g, SLOT, q).unwrap().status, 0);
    }

    #[test]
    fn a_post_spawns_an_instance_copies_the_payload_and_queues_its_node() {
        let (mut g, mut heap) = installed();
        resolve_game_slot(&mut g);
        w(&mut g, MSG + 4, &[0xAAAA, 0xBBBB]);
        assert_eq!(post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap(), 0);
        let rec = BANK + 0x5C;
        let instance = g.u32(rec + 56).unwrap();
        assert_ne!(instance, 0);
        assert_eq!(g.u16(rec + 28).unwrap(), 1, "one live instance");
        assert_eq!(
            g.u32(LIST_HEAD).unwrap(),
            instance + 8,
            "queued for the interpreter"
        );
        assert_eq!(g.u32(instance + 16).unwrap(), BANK + 0x180, "node program");
        assert_eq!(g.u32(instance + 20).unwrap(), instance + 24, "node block");
        assert_eq!(
            g.u32(instance + 72).unwrap(),
            rec,
            "back-pointer triple at +72"
        );
        assert_eq!(g.u32(instance + 24 + 20).unwrap(), 0xAAAA, "payload word 0");
        assert_eq!(g.u32(instance + 24 + 24).unwrap(), 0xBBBB, "payload word 1");
        let node = g.u32(MSG).unwrap();
        assert_eq!(
            g.u32(node + 4).unwrap(),
            2,
            "the payload callback took a reference"
        );
    }

    #[test]
    fn a_redelivery_rewrites_the_instance_copy() {
        let (mut g, mut heap) = installed();
        resolve_game_slot(&mut g);
        w(&mut g, MSG + 4, &[0, 1]);
        post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap();
        let instance = g.u32(BANK + 0x5C + 56).unwrap();
        w(&mut g, MSG + 4, &[32767, 7]);
        let node = g.u32(MSG).unwrap();
        assert_eq!(redeliver(&mut g, node, MSG + 4).unwrap(), 0);
        assert_eq!(
            (g.u32(instance + 44).unwrap(), g.u32(instance + 48).unwrap()),
            (32767, 7)
        );
    }

    #[test]
    fn capacity_bounds_the_instances() {
        let (mut g, mut heap) = installed();
        resolve_game_slot(&mut g);
        for _ in 0..3 {
            assert_eq!(post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap(), 0);
        }
        assert_eq!(g.u16(BANK + 0x5C + 28).unwrap(), 2);
    }

    #[test]
    fn a_stale_slot_is_cleared_and_reported() {
        let (mut g, mut heap) = installed();
        resolve_game_slot(&mut g);
        g.set_u32(SLOT + 4, 0x09C5_7777).unwrap();
        assert_eq!(post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap(), -3);
        assert_eq!(
            (g.u32(SLOT).unwrap(), g.u32(SLOT + 4).unwrap()),
            (0, (-3i32) as u32)
        );
        assert_eq!(
            post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap(),
            -3,
            "now its own negative id"
        );
        g.set_u32(SLOT + 4, 0).unwrap();
        assert_eq!(post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap(), -6);
    }

    #[test]
    fn slot_four_unlinks_and_frees_only_when_flagged() {
        let (mut g, mut heap) = installed();
        resolve_game_slot(&mut g);
        assert_eq!(post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap(), 0);
        let rec = BANK + 0x5C;
        let instance = g.u32(rec + 56).unwrap();
        let node = g.u32(MSG).unwrap();
        let triple = instance + 72;
        assert_eq!(
            end_instance(&mut g, &mut heap, &mut NoDevice, triple).unwrap(),
            0
        );
        assert_eq!(
            g.u16(rec + 28).unwrap(),
            1,
            "an unflagged instance is left alone"
        );
        g.set_u32(triple + 12, 1).unwrap();
        end_instance(&mut g, &mut heap, &mut NoDevice, triple).unwrap();
        assert_eq!(g.u32(rec + 56).unwrap(), 0, "off the record's live list");
        assert_eq!(g.u32(LIST_HEAD).unwrap(), 0, "off the interpreter's list");
        assert_eq!(g.u16(rec + 28).unwrap(), 0);
        assert_eq!(
            g.u32(node + 8).unwrap(),
            0,
            "the payload callback was unregistered"
        );
        assert_eq!(g.u32(node + 4).unwrap(), 1, "and gave back its reference");
    }

    #[test]
    fn the_interpreter_hands_slot_four_to_the_host() {
        use crate::eval::interp::{self, tick_with};
        let (mut g, mut heap) = installed();
        resolve_game_slot(&mut g);
        post(&mut g, &mut heap, SLOT, MSG + 4, MSG).unwrap();
        let instance = g.u32(BANK + 0x5C + 56).unwrap();
        // A program at the bank's 0x180 that moves the block to the triple (+48 from +24) and ends
        // the instance.
        w(
            &mut g,
            BANK + 0x180,
            &[0x0000_0000, 48, 0x0400_0000, 0, 0xFF00_0000],
        );
        g.set_u32(instance + 72 + 12, 1).unwrap();
        for cell in [
            interp::PERIOD_NUMER,
            interp::PERIOD_DENOM,
            interp::SCALE_UNIT,
        ] {
            g.put(cell, 1.0f32.to_bits().to_be_bytes().to_vec());
        }
        g.put(interp::ZERO_SINGLE, vec![0; 4]);
        g.put(interp::SCALE_GLOBAL, vec![0; 16]);
        let t = tick_with(
            &mut g,
            1.0,
            &mut PatchHost {
                heap: &mut heap,
                device: &mut NoDevice,
            },
        )
        .unwrap();
        assert_eq!((t.walked, t.ops), (true, 2));
        assert_eq!(
            g.u32(LIST_HEAD).unwrap(),
            0,
            "slot 4 took the instance off the list"
        );
    }

    #[test]
    fn broadcast_copies_and_flags() {
        let mut g = guest();
        w(&mut g, MEM + 0x10, &[7, 8, 9]);
        g.set_u8(MEM + 0x100 + 24, 2).unwrap();
        on_broadcast(&mut g, MEM + 0x10, MEM + 0x100).unwrap();
        assert_eq!(
            (
                g.u32(MEM + 0x100 + 28).unwrap(),
                g.u32(MEM + 0x100 + 32).unwrap()
            ),
            (7, 8)
        );
        assert_eq!(g.u8(MEM + 0x100 + 25).unwrap(), 1);
    }

    #[test]
    fn slot_five_clamps_and_runs_the_registered_broadcast_callback() {
        let mut g = guest();
        let block = MEM + 0x500;
        let symbol = MEM + 0x600;
        let subscriber = MEM + 0x640;
        let context = MEM + 0x700;
        let binding = block + 12 + 2 * 8;

        g.set_u8(block + 8, 1).unwrap();
        g.set_u8(block + 9, 2).unwrap();
        w(&mut g, block + 12, &[10, 20, (-10i32) as u32, 40]);
        g.set_u32(binding, symbol).unwrap();
        g.set_u32(binding + 4, 25).unwrap();
        g.set_u32(binding + 8, 50).unwrap();
        g.set_u32(symbol, subscriber).unwrap();
        g.set_u32(symbol + 8, 20).unwrap();
        w(&mut g, subscriber, &[0, 0, ON_BROADCAST, context]);
        g.set_u8(context + 24, 2).unwrap();

        assert_eq!(clamp_and_broadcast(&mut g, block).unwrap(), 0);
        assert_eq!(
            (g.u32(binding + 4).unwrap(), g.u32(binding + 8).unwrap()),
            (20, 40)
        );
        assert_eq!(
            (g.u32(context + 28).unwrap(), g.u32(context + 32).unwrap()),
            (20, 40)
        );
        assert_eq!(g.u8(context + 25).unwrap(), 1);
    }

    #[test]
    fn slot_five_invalidates_a_stale_handle_and_still_returns_zero() {
        let mut g = guest();
        let block = MEM + 0x500;
        let symbol = MEM + 0x600;
        let binding = block + 12;
        g.set_u32(binding, symbol).unwrap();
        g.set_u32(binding + 4, 7).unwrap();
        g.set_u32(symbol + 8, 8).unwrap();

        assert_eq!(clamp_and_broadcast(&mut g, block).unwrap(), 0);
        assert_eq!(g.u32(binding).unwrap(), 0);
        assert_eq!(g.u32(binding + 4).unwrap(), (-3i32) as u32);
    }

    #[test]
    fn slot_five_skips_an_unrelocated_symbol_instead_of_faulting_blind() {
        // An unresolved table-1 export, or a retail broadcaster that hasn't run yet, leaves a
        // small compiled literal (e.g. `9`, or the confirmed real-bank value `1`) in place
        // instead of a guest pointer. Retail's own traces show this exact literal surviving in
        // the live Xbox 360 process across dozens of deliveries of the same object while its
        // audio keeps playing, so this must be tolerated the same way `symbol == 0` already is
        // (status -6), not promoted to a fatal error that kills the whole audio worker.
        // Regression case for the retail wheel-audio crash logged in
        // docs/wheel-audio-handoff-2026-09-17.md (symbol `0x00000009` / `0x00000001`).
        let mut g = guest();
        let block = MEM + 0x500;
        let binding = block + 12;
        g.set_u32(binding, 9).unwrap(); // never a mapped address
        g.set_u32(binding + 4, 0).unwrap();

        assert_eq!(clamp_and_broadcast(&mut g, block).unwrap(), 0);
        // The bogus symbol/generation are left untouched: unlike a stale-generation handle, this
        // is not known to be permanently dead, so nothing is cleared.
        assert_eq!(g.u32(binding).unwrap(), 9);
    }

    #[test]
    fn slot_thirty_nine_skips_an_unrelocated_holder_instead_of_faulting_blind() {
        // The table-2 counterpart of the slot-five case above: an unresolved holder must be
        // skipped, not dereferenced.
        let mut g = guest();
        let block = MEM + 0x500;
        w(&mut g, block + 16, &[0, 1]); // applied=0, requested=1: forces the changed-value path
        g.set_u32(block, 9).unwrap(); // holder: never a mapped address
        g.set_u32(block + 4, 0).unwrap(); // generation: not negative, so the holder is reached

        assert_eq!(latch_and_notify(&mut g, block).unwrap(), 0);
        // The requested value is still latched into "applied" (that store happens before the
        // holder is ever read); only the broadcast itself is skipped.
        assert_eq!(g.u32(block + 16).unwrap(), 1);
    }
}
