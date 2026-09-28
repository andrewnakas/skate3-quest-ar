//! The per-player bus chain every grain voice sends into: built by `sub_824C8878`, torn down by
//! `sub_824C4D50`, and driven every frame by `sub_824C9058`.
//!
//! The board owner keeps one 24-byte record per grain player at `owner + 1192 + 24k` (`k = 2·truck +
//! which`): `+0` graph 1's module table, `+4` graph 1, `+8`/`+12` graph 2, `+16`/`+20` graph 3.
//! Here each record is a 24-byte guest allocation with the same layout.
//!
//! ```text
//! graph 1 (order 2, 1 ch):  Sub0 → HI20 → LI20 → FSS0(arg 0.0) → Sen0 ─┐ → Gai0 → Sen0 ──→ graph 2 Sub0
//!                                                   (to graph 3, level owner+1556)
//! graph 2 (order 5):        Sub0 → Sen0(→ [[manager+116]], level 0) → Sen0(→ [[manager+52]])
//!                                → Pan2D1 (1 → 6 ch) → Sen0 (6 ch, → bus eEQChain = 8, the default bus)
//! graph 3 (order 3, local player only, 1 ch):
//!                           Sub0 → DCl0(owner+1528) → Gai0 → HS20(owner+1548, owner+1552) → Sen0 → graph 2 Sub0
//! ```
//!
//! Each player's voices send to graph 1's `Sub0` (the binder's `r4` is `[[owner+1192+24k]]`).
//!
//! The owner values are vault constants: `+1528` = `0xE64C04ED542DABC8` (0.09), `+1548` =
//! `0x55BEB30353F244A9` (5000 Hz), `+1552` = `0x45516395725ED16B` (0.65), all class
//! `0x6E878344774A7999` (`sub_824CA938`, from the tuning holder's `+132` instance), and `+1556` =
//! 0.0 (`sub_824C59C8`, never written again). The final bus comes from the eEQChain attribute
//! `0xA5D3ADA63608617F` of the tuning holder's `+140` instance (class `0x42AFE160E647167C`, 8 in
//! the vault), through `sub_82491108` -- where index 8 is the default bus behind `BUS_ROOT`.
//!
//! **Teardown is not used by the host.** Retail runs `sub_824C4D50` (three deferred player stops)
//! and `sub_824C8878` again on every grain bind. The concrete owner's player-stop handler does not
//! run the `Sub0` destructor that unlinks its accumulator from the global list at `0x830BDEB8`,
//! so freeing a chain would leave that list pointing into freed memory. [`teardown`] is ported for
//! the record, but [`crate::grain::host::Grains`] builds each player's chain once and keeps it; the
//! only audible difference from a rebuild is that filter, shifter and pan histories are not reset
//! at a surface change.

use crate::classes::{self, TAG_POINTER, TAG_SINGLE};
use crate::device::{self, COMMAND_PLAYER_STOP};
use crate::fp::load_single;
use crate::mathlib::Trig;
use crate::modules::{self, ZERO};
use crate::patch::Heap;
use crate::{Error, Guest, Result};

use super::board::ChainValues;
use super::fss;

/// `stwu r1,-448(r1)`.
pub const BUILD_FRAME: u32 = 448;
/// Bytes of one chain record.
pub const RECORD_BYTES: u32 = 24;
pub const MODULES_1: u32 = 0;
pub const GRAPH_1: u32 = 4;
pub const MODULES_2: u32 = 8;
pub const GRAPH_2: u32 = 12;
pub const MODULES_3: u32 = 16;
pub const GRAPH_3: u32 = 20;

pub const SUBMIX_ID: u32 = 0x5375_6230; // "Sub0"
pub const HIGHPASS_ID: u32 = 0x4849_3230; // "HI20"
pub const LOWPASS_ID: u32 = 0x4C49_3230; // "LI20"
pub const SHIFT_ID: u32 = 0x4653_5330; // "FSS0"
pub const PAN_ID: u32 = 0x506E_3231; // "Pn21"
pub const SEND_ID: u32 = 0x5365_6E30; // "Sen0"
pub const GAIN_ID: u32 = 0x4761_6930; // "Gai0"
pub const SHELF_ID: u32 = 0x4853_3230; // "HS20"
pub const CLIP_ID: u32 = 0x4443_6C30; // "DCl0"

const _: () = {
    assert!(SUBMIX_ID == (21365 << 16 | 25136) && HIGHPASS_ID == (18505 << 16 | 12848));
    assert!(LOWPASS_ID == (19529 << 16 | 12848) && SHIFT_ID == (18003 << 16 | 21296));
    assert!(PAN_ID == (20590 << 16 | 12849) && SEND_ID == (21349 << 16 | 28208));
    assert!(GAIN_ID == (18273 << 16 | 26928) && SHELF_ID == (18515 << 16 | 12848));
    assert!(CLIP_ID == (17475 << 16 | 27696));
};

/// The owner-side inputs of `sub_824C8878`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainConfig {
    /// `[[owner+16]+72]`: the owner is the local player; builds graph 3.
    pub local: bool,
    /// `[[owner+28]+72]`, `sub_82491108`'s create byte (irrelevant for bus 8).
    pub create: u8,
    /// The eEQChain bus index (vault: 8).
    pub eq_chain: u32,
    /// `owner+1528`, `+1548`, `+1552`, `+1556`.
    pub clip: f32,
    pub shelf_corner: f32,
    pub shelf_gain: f32,
    pub local_send: f32,
}

impl ChainConfig {
    /// The retail vault values (see the module note).
    pub fn retail(local: bool) -> Self {
        Self {
            local,
            create: 0,
            eq_chain: 8,
            clip: f32::from_bits(0x3DB8_51EC),
            shelf_corner: 5000.0,
            shelf_gain: f32::from_bits(0x3F26_6666),
            local_send: 0.0,
        }
    }
}

/// The registry walk `sub_824C8878` repeats for every class: 0 when absent.
pub(crate) fn find(g: &Guest, id: u32) -> Result<u32> {
    let root = g.u32(classes::BUS_ROOT)?;
    let registry = g.u32(root + 12)?;
    let mut link = g.u32(registry)?;
    while link != 0 {
        let class = link.wrapping_sub(32);
        link = g.u32(link)?;
        if g.u32(class + 36)? == id {
            return Ok(class);
        }
    }
    Ok(0)
}

pub(crate) fn descriptor(g: &mut Guest, at: u32, arg: u32, class: u32, channels: u8) -> Result<()> {
    g.set_u32(at, arg)?;
    g.set_u32(at + 4, class)?;
    g.set_u8(at + 8, channels)
}

/// Point a `Sen0` at `target`: `{TAG_POINTER, target}` through its configure entry.
pub(crate) fn route(g: &mut Guest, block: u32, send: u32, target: u32) -> Result<()> {
    g.set_u32(block, TAG_POINTER)?;
    g.set_u32(block + 4, target)?;
    device::configure(g, send, 0, block)
}

/// `sub_824C8878`: build one player's chain into the 24-byte `record`. `sp` is the caller's stack.
pub fn build<H: Heap + ?Sized, T: Trig + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    record: u32,
    config: &ChainConfig,
    sp: u32,
) -> Result<()> {
    let frame = sp.wrapping_sub(BUILD_FRAME);
    let submix = find(g, SUBMIX_ID)?; // r20
    let highpass = find(g, HIGHPASS_ID)?; // r24
    let lowpass = find(g, LOWPASS_ID)?; // r27
    let shift = find(g, SHIFT_ID)?; // r29
    let pan = find(g, PAN_ID)?; // r28
    let send = find(g, SEND_ID)?; // r26
    let gain = find(g, GAIN_ID)?; // r23
    let shelf = find(g, SHELF_ID)?; // r21
    let root = g.u32(classes::BUS_ROOT)?;
    classes::register_class(g, g.u32(root + 12)?, device::CLIP_DESCRIPTOR)?; // sub_82B46770
    let clip = find(g, CLIP_ID)?; // r22
    for (class, name) in [
        (submix, "Sub0"),
        (highpass, "HI20"),
        (lowpass, "LI20"),
        (shift, "FSS0"),
        (pan, "Pn21"),
        (send, "Sen0"),
        (gain, "Gai0"),
    ] {
        if class == 0 {
            return Err(Error::new(
                0,
                format!("grain chain class {name} is not registered"),
            ));
        }
    }

    // FSS0's argument: its leading rows' slots, then row 0 = {single, 0.0}.
    let rows = u32::from(g.u8(shift + 41)?);
    let table = g.u32(shift + 20)?;
    for k in 0..rows {
        let value = g.u64(table.wrapping_add(8 + 40 * k))?;
        g.set_u64(frame + 88 + 8 * k, value)?;
    }
    let zero = load_single(g, ZERO)?;
    g.set_u32(frame + 88, TAG_SINGLE)?;
    crate::fp::store_single(g, frame + 92, zero)?;
    let one = 1u8;
    descriptor(g, frame + 224, 0, submix, one)?;
    descriptor(g, frame + 236, 0, highpass, one)?;
    descriptor(g, frame + 248, 0, lowpass, one)?;
    descriptor(g, frame + 260, frame + 88, shift, one)?;
    descriptor(g, frame + 272, 0, send, one)?;
    descriptor(g, frame + 284, 0, gain, one)?;
    descriptor(g, frame + 296, 0, send, one)?;
    let system = g.u32(root + 8)?;
    let graph1 = modules::build_graph(g, heap, trig, system, 2, 7, frame + 224)?;
    if graph1 == 0 {
        return Err(Error::new(
            0x824C_8878,
            "grain chain graph 1 allocation failed",
        ));
    }
    descriptor(g, frame + 96, 0, submix, one)?;
    descriptor(g, frame + 108, 0, send, one)?;
    descriptor(g, frame + 120, 0, send, one)?;
    descriptor(g, frame + 132, 0, pan, 6)?;
    descriptor(g, frame + 144, 0, send, 6)?;
    g.set_u32(record + GRAPH_1, graph1)?;
    g.set_u32(record + MODULES_1, graph1 + 80)?;
    let graph2 = modules::build_graph(g, heap, trig, system, 5, 5, frame + 96)?;
    if graph2 == 0 {
        return Err(Error::new(
            0x824C_8878,
            "grain chain graph 2 allocation failed",
        ));
    }
    g.set_u32(record + GRAPH_2, graph2)?;
    g.set_u32(record + MODULES_2, graph2 + 80)?;
    let block = frame + 80;
    classes::class_defaults(g, send, 0, block)?;
    let mods1 = g.u32(record + MODULES_1)?;
    let mods2 = g.u32(record + MODULES_2)?;
    route(g, block, g.u32(mods1 + 24)?, g.u32(mods2)?)?;

    let manager = g.u32(device::BUS_MANAGER)?;
    let bus = device::bus_for(
        g,
        &mut device::NoBuses,
        manager,
        config.eq_chain,
        config.create,
    )?;
    route(g, block, g.u32(mods2 + 16)?, bus)?;
    let environment = g.u32(g.u32(manager + 52)?)?;
    route(g, block, g.u32(mods2 + 8)?, environment)?;
    let secondary = g.u32(g.u32(manager + 116)?)?;
    route(g, block, g.u32(mods2 + 4)?, secondary)?;
    device::post_property(g, g.u32(mods2 + 4)?, 0, zero)?;

    if config.local {
        if clip == 0 || shelf == 0 {
            return Err(Error::new(
                0,
                "grain chain classes DCl0/HS20 are not registered",
            ));
        }
        descriptor(g, frame + 160, 0, submix, one)?;
        descriptor(g, frame + 172, 0, clip, one)?;
        descriptor(g, frame + 184, 0, gain, one)?;
        descriptor(g, frame + 196, 0, shelf, one)?;
        descriptor(g, frame + 208, 0, send, one)?;
        let graph3 = modules::build_graph(g, heap, trig, system, 3, 5, frame + 160)?;
        if graph3 == 0 {
            return Err(Error::new(
                0x824C_8878,
                "grain chain graph 3 allocation failed",
            ));
        }
        g.set_u32(record + GRAPH_3, graph3)?;
        g.set_u32(record + MODULES_3, graph3 + 80)?;
        let mods3 = graph3 + 80;
        route(g, block, g.u32(mods1 + 16)?, g.u32(mods3)?)?;
        device::post_property(g, g.u32(mods1 + 16)?, 0, f64::from(config.local_send))?;
        route(g, block, g.u32(mods3 + 16)?, g.u32(mods2)?)?;
        device::post_property(g, g.u32(mods3 + 4)?, 0, f64::from(config.clip))?;
        device::post_property(g, g.u32(mods3 + 12)?, 0, f64::from(config.shelf_corner))?;
        device::post_property(g, g.u32(mods3 + 12)?, 1, f64::from(config.shelf_gain))?;
    }
    Ok(())
}

/// The send target a player's voices use: graph 1's `Sub0`.
pub fn voice_bus(g: &Guest, record: u32) -> Result<u32> {
    g.u32(g.u32(record + MODULES_1)?)
}

/// `sub_824C4D50`: defer the stop of every built graph in `record`, then clear it.
pub fn teardown(g: &mut Guest, record: u32) -> Result<()> {
    for at in [GRAPH_1, GRAPH_2, GRAPH_3] {
        let graph = g.u32(record + at)?;
        if graph != 0 {
            let system = g.u32(graph + 16)?;
            let offset = g.u32(system + 204)?;
            let ring = g.u32(system + 48)?;
            g.set_u32(system + 204, offset.wrapping_add(8))?;
            g.set_u32(ring.wrapping_add(offset), COMMAND_PLAYER_STOP)?;
            g.set_u32(ring.wrapping_add(offset) + 4, graph)?;
        }
    }
    for k in 0..6 {
        g.set_u32(record + 4 * k, 0)?;
    }
    Ok(())
}

/// `sub_824C9058` for one truck: post `values` into the chains of its players A (`a`) and B
/// (`b`), in the retail order. The caller runs it for a truck whose grains are active
/// (`owner+1328+t` set and `owner+1320+4t == 1`).
pub fn push(g: &mut Guest, a: u32, b: u32, values: &ChainValues) -> Result<()> {
    let m = |g: &Guest, record: u32, table: u32, index: u32| -> Result<u32> {
        g.u32(g.u32(record + table)? + 4 * index)
    };
    let post = |g: &mut Guest, module: u32, value: f32| {
        device::post_property(g, module, 0, f64::from(value))
    };
    let hp = f32::from_bits(values.highpass.to_bits());
    let module = m(g, a, MODULES_1, 1)?;
    post(g, module, hp)?;
    let module = m(g, a, MODULES_1, 2)?;
    post(g, module, values.lowpass)?;
    let module = m(g, b, MODULES_1, 1)?;
    post(g, module, hp)?;
    let module = m(g, b, MODULES_1, 2)?;
    post(g, module, values.lowpass)?;
    let module = m(g, a, MODULES_1, 3)?;
    post(g, module, values.shift_a)?;
    let module = m(g, b, MODULES_1, 3)?;
    post(g, module, values.shift_b)?;
    let module = m(g, a, MODULES_2, 3)?;
    post(g, module, values.pan)?;
    let module = m(g, b, MODULES_2, 3)?;
    post(g, module, values.pan)?;
    let module = m(g, a, MODULES_2, 2)?;
    post(g, module, values.environment)?;
    let module = m(g, b, MODULES_2, 2)?;
    post(g, module, values.environment)?;
    if let Some((level_a, level_b)) = values.local {
        let module = m(g, a, MODULES_2, 1)?;
        post(g, module, level_a)?;
        let module = m(g, b, MODULES_2, 1)?;
        post(g, module, level_b)?;
    }
    Ok(())
}

/// Registrations the chain relies on, retail's: `FSS0` by the board owner (`sub_824C8798`) and
/// `HS20` by the audio setup (`sub_828DD590`).
pub fn register_classes<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H) -> Result<()> {
    let system = g.u32(modules::SYSTEM)?;
    let registry = classes::class_registry(g, heap, system)?;
    classes::register_class(g, registry, fss::FSS_DESCRIPTOR)?;
    classes::register_class(g, registry, fss::HS_DESCRIPTOR)?;
    Ok(())
}
