//! The resident-stream voices of `SFXObj_Wheels` (`.11.cpp`): a per-owner bus graph and one
//! `SndPlayer1` voice graph per spin, playing a `.snr` member of `wheels.big` from a start offset
//! through its `.sek` seek table.
//!
//! ```text
//! bus (sub_824CE108, order 2):  Sub0 → PI20 → HS20 → Sen0 (→ [[manager+116]], level 0)
//!                                    → Sen0 (→ [[manager+52]]) → Pn21 (1 → 6 ch)
//!                                    → Sen0 (6 ch, → eEQChain bus 0x55E6488906A2E330)
//! voice (sub_824CEAF0, order 1): SnP1 → Rch0 → Rsp0 → Gai0 → Sen0 (→ the bus's Sub0)
//! ```
//!
//! The play request is `SndPlayer1` parameter 5, the same block the grain voice uses: no name, the
//! `.snr` resource as the stream, the `.sek` buffer as the detail (seek table), and the start in
//! seconds. [`super::host`] serves it: the `.snr` is registered as a resident source keyed by its
//! address ([`super::host::Grains::load_resident`]) and the retail seek through the `.sek` table
//! runs on the pending request, exactly as for a grain.
//!
//! Module properties are posted with the property stamp (`0x82B463A8`, [`device::post_property`]):
//! a voice's gain goes to `Gai0` (module 3) and its pitch ratio to `Rsp0` (module 2)
//! (`sub_824CEE00`). A voice stops through the deferred player-stop command (`sub_824CEF60`).

use crate::classes::{self, TAG_POINTER, TAG_SINGLE, TAG_STRING};
use crate::device::{self, COMMAND_PLAYER_STOP};
use crate::fp::load_single;
use crate::mathlib::Trig;
use crate::modules::{self, ZERO};
use crate::patch::Heap;
use crate::{Error, Guest, Result};

use super::chain::{PAN_ID, SEND_ID, SHELF_ID, SUBMIX_ID, descriptor, find, route};
use super::player::{GAIN_ID, RESAMPLE_ID, SNDPLAYER_ID};

/// `lis 20553 ; ori 12848`: "PI20", the peaking equaliser.
pub const PEAK_ID: u32 = 0x5049_3230;
/// `lis 21091 ; ori 26672`: "Rch0", the rechannel stage.
pub const RECHANNEL_ID: u32 = 0x5263_6830;
/// `0x822F8700`: the pool's zero double.
const ZERO_DOUBLE: u32 = 0x822F_8700;
/// `0x8231A844`: 1.0.
const ONE: u32 = 0x8231_A844;
/// `stwu r1,-240(r1)` / `-336`: the frames the two builders use.
pub const BUS_FRAME: u32 = 240;
pub const VOICE_FRAME: u32 = 336;

/// The bus graph `sub_824CE108` builds, as the owner keeps it: `+68` the graph, `+64` its module
/// table (`graph + 80`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bus {
    pub graph: u32,
    pub modules: u32,
}

/// One spin voice: `+84+4i` the graph, `+72+4i` its module table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Voice {
    pub graph: u32,
    pub modules: u32,
}

fn class(g: &Guest, id: u32, name: &str) -> Result<u32> {
    let found = find(g, id)?;
    if found == 0 {
        return Err(Error::new(
            id,
            format!("stream voice class {name} is not registered"),
        ));
    }
    Ok(found)
}

/// `sub_824CE108`: build the owner's bus graph and route its three sends; the first send (to the
/// manager's `+116` target) starts at level 0. `eq_chain` is the eEQChain value of
/// `0x55E6488906A2E330` (6 in the vault).
pub fn build_bus<H: Heap + ?Sized, T: Trig + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    eq_chain: u32,
    sp: u32,
) -> Result<Bus> {
    let frame = sp.wrapping_sub(BUS_FRAME);
    let submix = class(g, SUBMIX_ID, "Sub0")?;
    let peak = class(g, PEAK_ID, "PI20")?;
    let shelf = class(g, SHELF_ID, "HS20")?;
    let pan = class(g, PAN_ID, "Pn21")?;
    let send = class(g, SEND_ID, "Sen0")?;
    descriptor(g, frame + 96, 0, submix, 1)?;
    descriptor(g, frame + 108, 0, peak, 1)?;
    descriptor(g, frame + 120, 0, shelf, 1)?;
    descriptor(g, frame + 132, 0, send, 1)?;
    descriptor(g, frame + 144, 0, send, 1)?;
    descriptor(g, frame + 156, 0, pan, 6)?;
    descriptor(g, frame + 168, 0, send, 6)?;
    let root = g.u32(classes::BUS_ROOT)?;
    let system = g.u32(root + 8)?;
    let graph = modules::build_graph(g, heap, trig, system, 2, 7, frame + 96)?;
    if graph == 0 {
        return Err(Error::new(
            0x824C_E108,
            "wheels bus graph allocation failed",
        ));
    }
    let bus = Bus {
        graph,
        modules: graph + 80,
    };
    let block = frame + 80;
    classes::class_defaults(g, send, 0, block)?;
    let manager = g.u32(device::BUS_MANAGER)?;
    let target = device::bus_for(g, &mut device::NoBuses, manager, eq_chain, 0)?;
    route(g, block, g.u32(bus.modules + 24)?, target)?;
    let environment = g.u32(g.u32(manager + 52)?)?;
    route(g, block, g.u32(bus.modules + 16)?, environment)?;
    let secondary = g.u32(g.u32(manager + 116)?)?;
    route(g, block, g.u32(bus.modules + 12)?, secondary)?;
    let zero = load_single(g, ZERO)?;
    device::post_property(g, g.u32(bus.modules + 12)?, 0, zero)?;
    Ok(bus)
}

/// `sub_824CEAF0` for a free voice slot: build the voice graph, play `stream` from `start`
/// seconds with `seek` as the detail, set gain 0 and pitch 1 (`sub_824CEE00`), and send it into
/// the bus.
pub fn start<H: Heap + ?Sized, T: Trig + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    bus: Bus,
    stream: u32,
    seek: u32,
    start: f32,
    sp: u32,
) -> Result<Voice> {
    let frame = sp.wrapping_sub(VOICE_FRAME);
    let sndplayer = class(g, SNDPLAYER_ID, "SnP1")?;
    let rechannel = class(g, RECHANNEL_ID, "Rch0")?;
    let resample = class(g, RESAMPLE_ID, "Rsp0")?;
    let gain = class(g, GAIN_ID, "Gai0")?;
    let send = class(g, SEND_ID, "Sen0")?;
    descriptor(g, frame + 96, 0, sndplayer, 1)?;
    descriptor(g, frame + 108, 0, rechannel, 1)?;
    descriptor(g, frame + 120, 0, resample, 1)?;
    descriptor(g, frame + 132, 0, gain, 1)?;
    descriptor(g, frame + 144, 0, send, 1)?;
    let root = g.u32(classes::BUS_ROOT)?;
    let system = g.u32(root + 8)?;
    let graph = modules::build_graph(g, heap, trig, system, 1, 5, frame + 96)?;
    if graph == 0 {
        return Err(Error::new(
            0x824C_EAF0,
            "wheels voice graph allocation failed",
        ));
    }
    let voice = Voice {
        graph,
        modules: graph + 80,
    };

    // SnP1 parameter 5: {0.0, 0.0, start} doubles, no name, the stream, the seek table, 1.0.
    let play = frame + 160;
    classes::class_defaults(g, sndplayer, 5, play)?;
    let zero_double = g.u64(ZERO_DOUBLE)?;
    g.set_u64(play + 16, f64::from(start).to_bits())?;
    g.set_u32(play + 36, stream)?;
    g.set_u32(play + 44, seek)?;
    g.set_u32(play + 28, 0)?;
    g.set_u64(play, zero_double)?;
    g.set_u64(play + 8, zero_double)?;
    g.set_u32(play + 32, TAG_POINTER)?;
    g.set_u32(play + 40, TAG_POINTER)?;
    g.set_u32(play + 56, TAG_SINGLE)?;
    g.set_u32(play + 24, TAG_STRING)?;
    g.set_u32(play + 60, g.u32(ONE)?)?;
    device::configure(g, g.u32(voice.modules)?, 5, play)?;

    let zero = load_single(g, ZERO)?;
    let one = load_single(g, ONE)?;
    set(g, voice, zero, one)?;

    let route_block = frame + 80;
    classes::class_defaults(g, send, 0, route_block)?;
    route(
        g,
        route_block,
        g.u32(voice.modules + 16)?,
        g.u32(bus.modules)?,
    )?;
    Ok(voice)
}

/// `sub_824CEE00`: the voice's gain (`Gai0`, module 3) then its pitch ratio (`Rsp0`, module 2).
pub fn set(g: &mut Guest, voice: Voice, gain: f64, pitch: f64) -> Result<()> {
    device::post_property(g, g.u32(voice.modules + 12)?, 0, gain)?;
    device::post_property(g, g.u32(voice.modules + 8)?, 0, pitch)
}

/// `sub_824CEF60`: append the deferred player-stop record `{0x82B49238, graph}`.
pub fn stop(g: &mut Guest, voice: Voice) -> Result<()> {
    let system = g.u32(voice.graph + 16)?;
    let offset = g.u32(system + 204)?;
    let ring = g.u32(system + 48)?;
    g.set_u32(system + 204, offset.wrapping_add(8))?;
    g.set_u32(ring.wrapping_add(offset), COMMAND_PLAYER_STOP)?;
    g.set_u32(ring.wrapping_add(offset) + 4, voice.graph)
}

/// `[graph+71]`: 2 once the player has finished (the owner then stops it).
pub fn finished(g: &Guest, voice: Voice) -> Result<bool> {
    Ok(g.u8(voice.graph + 71)? == 2)
}

/// Post `value` as property `id` of bus module `index`.
pub fn post_bus(g: &mut Guest, bus: Bus, index: u32, id: u32, value: f32) -> Result<()> {
    device::post_property(g, g.u32(bus.modules + 4 * index)?, id, f64::from(value))
}
