//! The retail `GrainPlayer` (`.47.cpp`, `sub_828EBD88` … `sub_828ECE98`): a 372-byte guest object
//! that plays short overlapping windows ("grains") of one `.grain` recording through two voices of
//! its own, each `SndPlayer1 → Resample → GainFader → Send`, crossfading attack/sustain/release
//! from a per-block scheduler callback.
//!
//! **Unverified new work** in the crate's sense: these run on the game thread (constructor, bind,
//! stop) or as a scheduler plug-in, and have no shadow-verified C++ body; each is transliterated
//! from its lifted body store by store and unit-tested. The object layout, the defaults, the
//! GrainParams copy, the recent-list bookkeeping and the per-block timer arithmetic were confirmed
//! against the retail capture (`.local/captures`, see the module tests and `grain_repro`).
//!
//! | function | here |
//! |---|---|
//! | `sub_828EBD88`, the constructor | [`construct`] |
//! | `sub_828EBCB0`, stop one voice slot (defers the graph's stop command) | [`stop_voice`] |
//! | `sub_828EBE68`, reset the pick state and register the scheduler plug-in | [`reset`] |
//! | `sub_82481BE0`, register a scheduler instance (bucket, pool node, plug-in fields) | [`register_instance`] |
//! | `sub_828EBF90`, stop both voices and detach the plug-in | [`stop`] |
//! | `sub_828EC040`, bind a grain file and resolve the module classes | [`bind`] |
//! | `sub_828EC208`, a voice's fade-in over the attack time | [`attack`] |
//! | `sub_828EC2F8`, a voice's fade-out over the release time | [`release`] |
//! | `sub_828EC3F0`, build a voice graph and start it at a grain | [`start_voice`] |
//! | `sub_828ECA08`, cut a free interval into candidate windows | [`candidates`] |
//! | `sub_828ECAB0`, pick the next grain window | [`pick`] |
//! | `sub_828EC6F0` (via the stub `sub_828ECE98`), the per-block plug-in | [`tick`] |
//!
//! **The object** (all big-endian):
//!
//! | offset | field |
//! |---|---|
//! | `+0 +4 +8 +12` | the owner's record `{gain, pitch, 0, position}` written each game frame |
//! | `+16 +20 +24 +28 +32` | `GrainParams`: attack s, sustain s, release s, search window s, drift |
//! | `+36` (u8) | hold: when set, no new grain is started |
//! | `+40` | the send target (a bus module) every voice's `Sen0` is pointed at |
//! | `+44` (u8) | bound / plug-in registered |
//! | `+48 +52 +56 +60 +64` | grain data, header length, duration, seek table (`data+8`), EAAC stream (`data+H`) |
//! | `+68 +72 +76 +80 +84` | classes `SnP1`, `Rsp0`, `GaF0`, `Sen0`, `Gai0` (the last is resolved but never used) |
//! | `+88`, `+116` | two 28-byte voice slots: `+0` module table, `+4` graph, `+8` state timer, `+12` start s, `+16` position at pick, `+20` state (1 attack, 2 sustain, 3 release) |
//! | `+144` | the active slot |
//! | `+148` | the 24-byte scheduler instance (`+0` node, `+4` plug-in, `+8` context, `+12` name, `+16` elapsed, `+20` bucket, `+21` flag) |
//! | `+172` | 16 recent-window entries `{f32 start, f32 end, u16 next, pad}` |
//! | `+364 +366 +368` (u16) | used-list head, last inserted entry, free-list head |
//!
//! Locks are left out as elsewhere in the crate: every body here brackets itself with the audio
//! system's lock (`[system+84]` / `RtlEnterCriticalSection([system+96])`), which only serialises the
//! game thread against the audio thread; the concrete owner here is single-threaded.

use crate::classes::{self, TAG_POINTER, TAG_SINGLE, TAG_STRING};
use crate::device::{self, COMMAND_PLAYER_STOP, ZERO_DOUBLE};
use crate::fp::{
    add_single, div_single, fcfid, fctiwz_low_word, frsp, load_single, mul_single, nmsub_single,
    store_single, sub_single,
};
use crate::mathlib::Trig;
use crate::modules::{self, HALF, MINUS_ONE, ONE, SYSTEM, UNKNOWN_NAME, ZERO};
use crate::patch::Heap;
use crate::{Error, Guest, Result, scheduler};

use super::rng;

/// `li r3,372` at the owner's allocation of each player.
pub const PLAYER_BYTES: u32 = 372;

pub const GAIN: u32 = 0;
pub const PITCH: u32 = 4;
pub const RECORD_WORD: u32 = 8;
pub const POSITION: u32 = 12;
pub const ATTACK: u32 = 16;
pub const SUSTAIN: u32 = 20;
pub const RELEASE: u32 = 24;
pub const WINDOW: u32 = 28;
pub const DRIFT: u32 = 32;
pub const HOLD: u32 = 36;
pub const BUS: u32 = 40;
pub const BOUND: u32 = 44;
pub const DATA: u32 = 48;
pub const HEADER_LEN: u32 = 52;
pub const DURATION: u32 = 56;
pub const SEEK_TABLE: u32 = 60;
pub const STREAM: u32 = 64;
pub const CLASS_SNDPLAYER: u32 = 68;
pub const CLASS_RESAMPLE: u32 = 72;
pub const CLASS_FADER: u32 = 76;
pub const CLASS_SEND: u32 = 80;
pub const CLASS_GAIN: u32 = 84;
pub const SLOTS: u32 = 88;
pub const SLOT_BYTES: u32 = 28;
pub const ACTIVE: u32 = 144;
pub const INSTANCE: u32 = 148;
pub const RECENT: u32 = 172;
pub const RECENT_BYTES: u32 = 12;
pub const RECENT_COUNT: u32 = 16;
pub const RECENT_HEAD: u32 = 364;
pub const RECENT_LAST: u32 = 366;
pub const RECENT_FREE: u32 = 368;

/// Voice slot fields.
pub const SLOT_MODULES: u32 = 0;
pub const SLOT_GRAPH: u32 = 4;
pub const SLOT_TIMER: u32 = 8;
pub const SLOT_START: u32 = 12;
pub const SLOT_PICK_POSITION: u32 = 16;
pub const SLOT_STATE: u32 = 20;
pub const STATE_ATTACK: u32 = 1;
pub const STATE_SUSTAIN: u32 = 2;
pub const STATE_RELEASE: u32 = 3;

/// `lis -32243 ; lfs 29160`: 0.01, the default attack and release.
pub const PERCENT: u32 = 0x820D_71E8;
/// `lis -32219 ; lfs 29448`: 4.0, the default search window.
pub const FOUR: u32 = 0x8225_7308;
/// `lis -32234 ; lfs 23040`: 0.05, the default drift threshold.
pub const DRIFT_DEFAULT: u32 = 0x8216_5A00;
/// `lis -32247 ; lfs 16760`: -2.0, the sentinel entry's start.
pub const MINUS_TWO: u32 = 0x8209_4178;
/// `lis -32208 ; addi -31232 ; lfs 1276`: 2^-31.
pub const TWO_POW_MINUS_31: u32 = 0x822F_8AFC;
/// `addi r7,r29,24940` off `0x8216DEE0`: "Grain Player", the instance's name.
pub const NAME: u32 = 0x8217_404C;
/// `lis -32113 ; addi -12648`: `sub_828ECE98`, the plug-in stub that branches to `sub_828EC6F0`.
pub const PLUG_IN: u32 = 0x828E_CE98;
/// Class ids the binder resolves.
pub const SNDPLAYER_ID: u32 = 0x536E_5031; // "SnP1"
pub const RESAMPLE_ID: u32 = 0x5273_7030; // "Rsp0"
pub const FADER_ID: u32 = 0x4761_4630; // "GaF0"
pub const SEND_ID: u32 = 0x5365_6E30; // "Sen0"
pub const GAIN_ID: u32 = 0x4761_6930; // "Gai0"
/// `GainFader`'s descriptor, registered by the title's audio setup `sub_828DD590` (its first
/// `sub_82B46770`) and looked up by id here.
pub const FADER_DESCRIPTOR: u32 = 0x82FC_E4CC;

/// Frame sizes (`stwu r1,-N(r1)`).
pub const TICK_FRAME: u32 = 176;
pub const PICK_FRAME: u32 = 928;
pub const START_FRAME: u32 = 336;
pub const FADE_FRAME: u32 = 144;

const _: () = {
    const fn lis(hi: i32, lo: i32) -> u32 {
        (((hi & 0xFFFF) << 16) as u32).wrapping_add(lo as u32)
    }
    assert!(PERCENT == lis(-32243, 29160) && FOUR == lis(-32219, 29448));
    assert!(DRIFT_DEFAULT == lis(-32234, 23040) && MINUS_TWO == lis(-32247, 16760));
    assert!(TWO_POW_MINUS_31 == lis(-32208, -31232 + 1276));
    assert!(NAME == lis(-32233, -8480 + 24940) && PLUG_IN == lis(-32113, -12648));
    assert!(SNDPLAYER_ID == (21358 << 16 | 20529) && RESAMPLE_ID == (21107 << 16 | 28720));
    assert!(FADER_ID == (18273 << 16 | 17968) && SEND_ID == (21349 << 16 | 28208));
    assert!(GAIN_ID == (18273 << 16 | 26928) && FADER_DESCRIPTOR == lis(-32003, -6964));
};

fn slot_address(player: u32, index: u32) -> u32 {
    player + SLOTS + SLOT_BYTES * index
}

fn entry(player: u32, index: i32) -> u32 {
    // rlwinm r9,r10,1 ; add ; rlwinm 2: twelve times the sign-extended index, 32-bit.
    player.wrapping_add((index as u32).wrapping_mul(RECENT_BYTES))
}

/// `(i + 1) % 2` the way `srawi ; addze ; rlwinm ; subf` forms it (C remainder of a signed word).
fn other(index: u32) -> u32 {
    let next = index.wrapping_add(1) as i32;
    (next - (next / 2) * 2) as u32
}

/// `sub_828EBD88`: construct a player in 372 bytes at `player`.
pub fn construct(g: &mut Guest, player: u32) -> Result<u32> {
    let one = load_single(g, ONE)?;
    let percent = load_single(g, PERCENT)?;
    store_single(g, player + GAIN, one)?;
    store_single(g, player + PITCH, one)?;
    g.set_u32(player + RECORD_WORD, 0)?;
    store_single(g, player + POSITION, load_single(g, ZERO)?)?;
    let half = load_single(g, HALF)?;
    let four = load_single(g, FOUR)?;
    let drift = load_single(g, DRIFT_DEFAULT)?;
    store_single(g, player + ATTACK, percent)?;
    store_single(g, player + SUSTAIN, half)?;
    store_single(g, player + RELEASE, percent)?;
    store_single(g, player + WINDOW, four)?;
    store_single(g, player + DRIFT, drift)?;
    g.set_u8(player + BOUND, 0)?;
    for index in 0..2 {
        let slot = slot_address(player, index);
        g.set_u32(slot + SLOT_MODULES, 0)?;
        g.set_u32(slot + SLOT_GRAPH, 0)?;
        stop_voice(g, slot)?;
    }
    g.set_u32(player + INSTANCE, 0)?;
    g.set_u32(player + INSTANCE + 16, 0)?;
    g.set_u8(player + INSTANCE + 20, 3)?;
    g.set_u32(player + INSTANCE + 12, UNKNOWN_NAME)?;
    let minus_one = load_single(g, MINUS_ONE)?;
    for i in 0..RECENT_COUNT {
        let at = player + RECENT + RECENT_BYTES * i;
        store_single(g, at, minus_one)?;
        store_single(g, at + 4, minus_one)?;
        g.set_u16(at + 8, 0)?;
    }
    Ok(player)
}

/// `sub_828EBCB0`: stop a voice slot. A live graph gets the eight-byte deferred player-stop record
/// `{0x82B49238, graph}` on its system's command ring; the slot is cleared to state 1.
pub fn stop_voice(g: &mut Guest, slot: u32) -> Result<()> {
    let graph = g.u32(slot + SLOT_GRAPH)?;
    g.set_u32(slot + SLOT_MODULES, 0)?;
    if graph != 0 {
        let system = g.u32(graph + 16)?;
        let offset = g.u32(system + 204)?;
        let ring = g.u32(system + 48)?;
        g.set_u32(system + 204, offset.wrapping_add(8))?;
        let at = ring.wrapping_add(offset);
        g.set_u32(at, COMMAND_PLAYER_STOP)?;
        g.set_u32(at + 4, graph)?;
    }
    let zero = load_single(g, ZERO)?;
    g.set_u32(slot + SLOT_GRAPH, 0)?;
    g.set_u32(slot + SLOT_STATE, STATE_ATTACK)?;
    store_single(g, slot + SLOT_TIMER, zero)?;
    store_single(g, slot + SLOT_START, zero)?;
    store_single(g, slot + SLOT_PICK_POSITION, zero)
}

/// `sub_82481BE0`: register `instance` with the scheduler bucket `bucket` of `system`, growing the
/// bucket's node pool by 74 the first time. Returns the pool's status (0 = registered).
#[allow(clippy::too_many_arguments)]
pub fn register_instance<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    system: u32,
    instance: u32,
    process: u32,
    context: u32,
    name: u32,
    bucket: u32,
    flag: u8,
) -> Result<u32> {
    let pool = system
        .wrapping_add(scheduler::SYSTEM_SCHEDULER)
        .wrapping_add(bucket.wrapping_shl(5));
    if g.u32(pool + 28)? as i32 == 0 {
        modules::pool_grow(g, heap, pool, 74)?;
    }
    let status = modules::pool_take(g, heap, pool, instance)?;
    if status & 0xFF == 0 {
        g.set_u32(instance + 4, process)?;
        g.set_u32(instance + 8, context)?;
        g.set_u32(instance + 12, name)?;
        g.set_u8(instance + 20, bucket as u8)?;
        g.set_u8(instance + 21, flag)?;
        g.set_u32(instance + 16, 0)?;
    }
    Ok(status)
}

/// `sub_828EBE68`: reset the recent list to the single sentinel `{-2, -1}` and register the plug-in
/// in scheduler bucket 0.
pub fn reset<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, player: u32) -> Result<()> {
    reset_recent(g, player)?;
    let system = g.u32(SYSTEM)?;
    register_instance(
        g,
        heap,
        system,
        player + INSTANCE,
        PLUG_IN,
        player,
        NAME,
        0,
        0,
    )?;
    g.set_u8(player + BOUND, 1)
}

/// `sub_828EBE68` up to its registration: the active slot, the recent list and the hold byte.
pub fn reset_recent(g: &mut Guest, player: u32) -> Result<()> {
    let minus_one = load_single(g, MINUS_ONE)?;
    g.set_u32(player + ACTIVE, 0)?;
    g.set_u16(player + RECENT_LAST, 0)?;
    g.set_u16(player + RECENT_HEAD, 0)?;
    g.set_u16(player + RECENT_FREE, 1)?;
    for i in 0..RECENT_COUNT {
        let at = player + RECENT + RECENT_BYTES * i;
        store_single(g, at, minus_one)?;
        g.set_u16(at + 8, 0)?;
        store_single(g, at + 4, minus_one)?;
        g.set_u16(at + 8, (i + 1) as u16)?;
    }
    store_single(g, player + RECENT + 4, minus_one)?;
    g.set_u8(player + HOLD, 0)?;
    g.set_u16(player + RECENT + RECENT_BYTES * 15 + 8, 0xFFFF)?;
    g.set_u16(player + RECENT + 8, 0xFFFF)?;
    store_single(g, player + RECENT, load_single(g, MINUS_TWO)?)
}

/// `sub_828EBF90`: when bound, stop both voices and detach the plug-in.
pub fn stop(g: &mut Guest, player: u32) -> Result<()> {
    if g.u8(player + BOUND)? == 0 {
        return Ok(());
    }
    for index in 0..2 {
        stop_voice(g, slot_address(player, index))?;
    }
    let system = g.u32(SYSTEM)?;
    scheduler::detach_instance(g, u64::from(system), player + INSTANCE)?;
    g.set_u8(player + BOUND, 0)
}

/// The binder's registry walk: `[registry]` links, class at `link − 32`, id at `+36`; 0 when absent.
fn lookup(g: &Guest, registry: u32, id: u32) -> Result<u32> {
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

/// `sub_82481B08` then the walk, as `sub_828EC208`/`sub_828EC2F8` repeat it for `GaF0`.
fn class_by_id<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, id: u32) -> Result<u32> {
    let system = g.u32(SYSTEM)?;
    let registry = classes::class_registry(g, heap, system)?;
    lookup(g, registry, id)
}

/// `sub_828EC040`: reset, then bind the grain file at `data` and send target `bus`, and resolve the
/// five module classes.
pub fn bind<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    player: u32,
    bus: u32,
    data: u32,
) -> Result<()> {
    reset(g, heap, player)?;
    g.set_u32(player + DATA, data)?;
    g.set_u32(player + BUS, bus)?;
    let header = g.u32(data)?;
    g.set_u32(player + HEADER_LEN, header)?;
    store_single(g, player + DURATION, load_single(g, data + 4)?)?; // lfsu f0,4(r31) ; stfs
    g.set_u32(player + STREAM, data.wrapping_add(header))?;
    g.set_u32(player + SEEK_TABLE, data + 8)?;
    let system = g.u32(SYSTEM)?;
    let registry = classes::class_registry(g, heap, system)?;
    for (at, id) in [
        (CLASS_SNDPLAYER, SNDPLAYER_ID),
        (CLASS_RESAMPLE, RESAMPLE_ID),
        (CLASS_FADER, FADER_ID),
        (CLASS_SEND, SEND_ID),
        (CLASS_GAIN, GAIN_ID),
    ] {
        let class = lookup(g, registry, id)?;
        g.set_u32(player + at, class)?;
    }
    Ok(())
}

/// The fade block `sub_828EC208` and `sub_828EC2F8` build: `GaF0` parameter 0's defaults, then
/// `{0.0 (double), time, target, 1.0}` with single tags, handed to the fader's configure entry.
fn fade<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    slot: u32,
    time: f64,
    target: f64,
    sp: u32,
) -> Result<()> {
    let frame = sp.wrapping_sub(FADE_FRAME);
    let block = frame + 80;
    let class = class_by_id(g, heap, FADER_ID)?;
    classes::class_defaults(g, class, 0, block)?;
    store_single(g, block + 12, time)?;
    g.set_u32(block + 8, TAG_SINGLE)?;
    g.set_u32(block + 16, TAG_SINGLE)?;
    g.set_u32(block + 24, TAG_SINGLE)?;
    store_single(g, block + 20, target)?;
    g.set_u64(block, g.u64(ZERO_DOUBLE)?)?;
    store_single(g, block + 28, load_single(g, ONE)?)?;
    let modules = g.u32(slot + SLOT_MODULES)?;
    if modules != 0 {
        device::configure(g, g.u32(modules + 8)?, 0, block)?;
    }
    Ok(())
}

/// `sub_828EC208`: fade the slot's voice in over `time` and enter state 1.
pub fn attack<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    slot: u32,
    time: f64,
    sp: u32,
) -> Result<()> {
    let one = load_single(g, ONE)?;
    fade(g, heap, slot, time, one, sp)?;
    store_single(g, slot + SLOT_TIMER, time)?;
    g.set_u32(slot + SLOT_STATE, STATE_ATTACK)
}

/// `sub_828EC2F8`: enter state 3 and fade the slot's voice out over `time`.
pub fn release<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    slot: u32,
    time: f64,
    sp: u32,
) -> Result<()> {
    store_single(g, slot + SLOT_TIMER, time)?;
    g.set_u32(slot + SLOT_STATE, STATE_RELEASE)?;
    let zero = load_single(g, ZERO)?;
    fade(g, heap, slot, time, zero, sp)
}

/// `sub_828EC3F0`: build `SnP1 → Rsp0 → GaF0 → Sen0` (one channel, scheduler order 0), play the
/// grain stream from `start` seconds (clamped to `[0, duration]`), set the fader to 0 at once and
/// then attack, point the send at the player's bus and stamp the current gain.
pub fn start_voice<H: Heap + ?Sized, T: Trig + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    player: u32,
    slot: u32,
    start: f64,
    sp: u32,
) -> Result<()> {
    let frame = sp.wrapping_sub(START_FRAME);
    let sndplayer = g.u32(player + CLASS_SNDPLAYER)?;
    // SnP1's constructor argument: its leading rows' 8-byte slots, then row 0 = {single, 1.0}.
    let words = u32::from(g.u8(sndplayer + 41)?);
    let rows = g.u32(sndplayer + 20)?;
    for k in 0..words {
        let value = g.u64(rows.wrapping_add(8 + 40 * k))?;
        g.set_u64(frame + 80 + 8 * k, value)?;
    }
    let one = load_single(g, ONE)?;
    g.set_u32(frame + 132, sndplayer)?;
    g.set_u8(frame + 136, 1)?;
    store_single(g, frame + 84, one)?;
    g.set_u32(frame + 168, g.u32(player + CLASS_SEND)?)?;
    g.set_u32(frame + 144, g.u32(player + CLASS_RESAMPLE)?)?;
    g.set_u32(frame + 80, TAG_SINGLE)?;
    g.set_u32(frame + 128, frame + 80)?;
    g.set_u32(frame + 156, g.u32(player + CLASS_FADER)?)?;
    g.set_u32(frame + 140, 0)?;
    g.set_u8(frame + 148, 1)?;
    g.set_u32(frame + 152, 0)?;
    g.set_u8(frame + 160, 1)?;
    g.set_u32(frame + 164, 0)?;
    g.set_u8(frame + 172, 1)?;
    let system = g.u32(SYSTEM)?;
    let graph = modules::build_graph(g, heap, trig, system, 0, 4, frame + 128)?;
    if graph == 0 {
        return Err(Error::new(
            modules::RELEASE_PLAYER,
            "grain voice graph allocation failed",
        ));
    }
    g.set_u32(slot + SLOT_GRAPH, graph)?;
    g.set_u32(slot + SLOT_MODULES, graph + 80)?;

    // Play: SnP1 parameter 5 -- no name, the EAAC stream, the seek table, and the start offset.
    let play = frame + 176;
    classes::class_defaults(g, sndplayer, 5, play)?;
    store_single(g, frame + 236, one)?;
    g.set_u32(frame + 232, TAG_SINGLE)?;
    g.set_u32(frame + 212, g.u32(player + STREAM)?)?;
    g.set_u32(frame + 220, g.u32(player + SEEK_TABLE)?)?;
    g.set_u32(frame + 208, TAG_POINTER)?;
    let zero_double = g.u64(ZERO_DOUBLE)?;
    g.set_u64(frame + 176, zero_double)?;
    g.set_u32(frame + 216, TAG_POINTER)?;
    let zero = load_single(g, ZERO)?;
    let mut at = start;
    if !(at >= zero) {
        at = zero;
    } else {
        let duration = load_single(g, player + DURATION)?;
        if !(at <= duration) {
            at = duration;
        }
    }
    g.set_u64(frame + 192, at.to_bits())?;
    g.set_u64(frame + 184, zero_double)?;
    g.set_u32(frame + 204, 0)?;
    g.set_u32(frame + 200, TAG_STRING)?;
    let modules = g.u32(slot + SLOT_MODULES)?;
    device::configure(g, g.u32(modules)?, 5, play)?;

    // The fader straight to zero.
    let block = frame + 96;
    classes::class_defaults(g, g.u32(player + CLASS_FADER)?, 0, block)?;
    store_single(g, frame + 108, zero)?;
    store_single(g, frame + 116, zero)?;
    g.set_u32(frame + 104, TAG_SINGLE)?;
    g.set_u64(frame + 96, zero_double)?;
    g.set_u32(frame + 112, TAG_SINGLE)?;
    store_single(g, frame + 124, zero)?;
    g.set_u32(frame + 120, TAG_SINGLE)?;
    let modules = g.u32(slot + SLOT_MODULES)?;
    device::configure(g, g.u32(modules + 8)?, 0, block)?;

    let attack_time = load_single(g, player + ATTACK)?;
    attack(g, heap, slot, attack_time, frame)?;

    // The send to the player's bus.
    let route = frame + 88;
    classes::class_defaults(g, g.u32(player + CLASS_SEND)?, 0, route)?;
    g.set_u32(route, TAG_POINTER)?;
    g.set_u32(route + 4, g.u32(player + BUS)?)?;
    let modules = g.u32(slot + SLOT_MODULES)?;
    device::configure(g, g.u32(modules + 12)?, 0, route)?;
    let modules = g.u32(slot + SLOT_MODULES)?;
    let gain = load_single(g, player + GAIN)?;
    device::post_property(g, g.u32(modules + 12)?, 0, gain)?;

    store_single(g, slot + SLOT_START, at)?;
    store_single(
        g,
        slot + SLOT_PICK_POSITION,
        load_single(g, player + POSITION)?,
    )
}

/// `sub_828ECA08`: clamp the free interval at `interval` (`{lo, hi}` singles, updated in place) to
/// `[low, high]` and append back-to-back windows of `length` to the 12-byte records at `out`,
/// counting in the word at `count`, while a whole window still fits and fewer than 64 exist.
pub fn candidates(
    g: &mut Guest,
    interval: u32,
    out: u32,
    count: u32,
    length: f64,
    low: f64,
    high: f64,
) -> Result<()> {
    let hi = load_single(g, interval + 4)?;
    if hi < low {
        return Ok(());
    }
    let lo = load_single(g, interval)?;
    if lo > high {
        return Ok(());
    }
    if !(lo >= low) {
        store_single(g, interval, low)?;
    }
    if !(hi <= high) {
        store_single(g, interval + 4, high)?;
    }
    let mut f0 = load_single(g, interval)?;
    let f13 = load_single(g, interval + 4)?;
    if sub_single(f13, f0) < length {
        return Ok(());
    }
    loop {
        let k = g.u32(count)?;
        let next = k.wrapping_add(1);
        g.set_u32(count, next)?;
        let at = out.wrapping_add(k.wrapping_mul(12));
        store_single(g, at, f0)?;
        let lo = load_single(g, interval)?;
        store_single(g, at + 4, add_single(lo, length))?;
        let hi = load_single(g, interval + 4)?;
        let lo = load_single(g, interval)?;
        f0 = add_single(lo, length);
        let left = sub_single(hi, f0);
        store_single(g, interval, f0)?;
        if left < length {
            return Ok(());
        }
        if add_single(f0, length) > high {
            return Ok(());
        }
        if (next as i32) >= 64 {
            return Ok(());
        }
    }
}

/// `sub_828ECAB0`: pick the next grain's start time. The target is `(duration − window) ×
/// position`; the free gaps between recently played windows (a start-sorted list) are cut into
/// windows of the grain length `attack + sustain + release` inside `[target, target + window]`,
/// one is drawn with the title's generator and inserted into the list. With no candidate, or once
/// entry 15 was the last used, the list collapses to its most recent entry and the search runs
/// again -- or, when the window holds at most one grain (`int(window / length) ≤ 1`), the target
/// itself is returned.
pub fn pick(g: &mut Guest, player: u32, sp: u32) -> Result<f64> {
    let frame = sp.wrapping_sub(PICK_FRAME);
    let minus_one = load_single(g, MINUS_ONE)?; // f9
    let p = player;
    loop {
        let sustain = load_single(g, p + SUSTAIN)?;
        let release_t = load_single(g, p + RELEASE)?;
        let f12 = add_single(sustain, release_t);
        let attack_t = load_single(g, p + ATTACK)?;
        let window = load_single(g, p + WINDOW)?;
        let duration = load_single(g, p + DURATION)?;
        let f7 = sub_single(duration, window);
        let position = load_single(g, p + POSITION)?;
        let length = add_single(f12, attack_t); // f1
        let target = mul_single(f7, position); // f2
        let ratio = div_single(window, length); // f5
        let end = add_single(window, target); // f3
        let fits = fctiwz_low_word(ratio) as i32; // r6
        for k in 0..64u32 {
            let at = frame + 112 + 12 * k;
            store_single(g, at, minus_one)?;
            store_single(g, at + 4, minus_one)?;
            g.set_u16(at + 8, 0)?;
        }
        let interval = frame + 88;
        let count = frame + 80;
        let out = frame + 112;
        let head = g.u16(p + RECENT_HEAD)? as i16 as i32;
        store_single(g, interval, minus_one)?;
        store_single(g, interval + 4, minus_one)?;
        let mut found = 0u32; // r30
        g.set_u16(frame + 96, 0)?;
        g.set_u32(count, 0)?;
        let first = load_single(g, entry(p, head) + RECENT)?;
        if !(target >= first) {
            store_single(g, interval, target)?;
            store_single(g, interval + 4, first)?;
            candidates(g, interval, out, count, length, target, end)?;
            found = g.u32(count)?;
        }
        let mut cur = g.u16(p + RECENT_HEAD)? as i16 as i32;
        while cur != -1 {
            let next_at = entry(p, cur) + RECENT + 8;
            let next = g.u16(next_at)? as i16 as i32;
            store_single(g, interval, load_single(g, entry(p, cur) + RECENT + 4)?)?;
            let limit = if next == -1 {
                load_single(g, p + DURATION)?
            } else {
                load_single(g, entry(p, next) + RECENT)?
            };
            store_single(g, interval + 4, limit)?;
            if (found as i32) < 64 {
                candidates(g, interval, out, count, length, target, end)?;
                found = g.u32(count)?;
            }
            cur = g.u16(next_at)? as i16 as i32;
        }
        if found == 0 || g.u16(p + RECENT_LAST)? == 15 {
            // Collapse the list to the most recent entry, free the rest in order.
            let last = g.u16(p + RECENT_LAST)? as i16 as i32;
            let src = entry(p, last) + RECENT;
            let a = g.u32(src)?;
            g.set_u32(p + RECENT, a)?;
            let b = g.u32(src + 4)?;
            g.set_u32(p + RECENT + 4, b)?;
            let c = g.u32(src + 8)?;
            g.set_u32(p + RECENT + 8, c)?;
            g.set_u16(p + RECENT + 8, 0xFFFF)?;
            g.set_u16(p + RECENT_HEAD, 0)?;
            g.set_u16(p + RECENT_LAST, 0)?;
            g.set_u16(p + RECENT_FREE, 1)?;
            for i in 1..RECENT_COUNT {
                let at = p + RECENT + RECENT_BYTES * i;
                store_single(g, at, minus_one)?;
                g.set_u16(at + 8, 0)?;
                store_single(g, at + 4, minus_one)?;
                g.set_u16(at + 8, (i + 1) as u16)?;
            }
            g.set_u16(p + RECENT + RECENT_BYTES * 15 + 8, 0xFFFF)?;
            if fits > 1 {
                continue;
            }
            return Ok(target);
        }

        let random = rng::next(g)?;
        let free = g.u16(p + RECENT_FREE)?; // r7
        let n = free as i16 as i32; // r9
        let countf = frsp(fcfid(i64::from(found as i32))); // f8
        let randf = frsp(fcfid(i64::from(random))); // f11, zero-extended
        let scale = load_single(g, TWO_POW_MINUS_31)?;
        let half = load_single(g, HALF)?;
        let new_at = entry(p, n) + RECENT;
        g.set_u16(p + RECENT_FREE, g.u16(new_at + 8)?)?;
        let f9 = mul_single(randf, scale);
        let f7 = mul_single(f9, half);
        let f6 = mul_single(f7, countf);
        let index = fctiwz_low_word(f6);
        let cand = out.wrapping_add(index.wrapping_mul(12));
        let w0 = g.u32(cand)?;
        let w2 = g.u32(cand.wrapping_add(8))?;
        let w1 = g.u32(cand.wrapping_add(4))?;
        g.set_u32(new_at, w0)?;
        g.set_u32(new_at + 8, w2)?;
        g.set_u32(new_at + 4, w1)?;
        g.set_u16(p + RECENT_LAST, free)?;
        // Insert into the start-sorted used list.
        let start = load_single(g, new_at)?;
        let mut cur_word = g.u16(p + RECENT_HEAD)?; // r10
        let mut cur = cur_word as i16 as i32; // r11
        let mut prev: i32 = -1; // r5
        let head_start = load_single(g, entry(p, cur) + RECENT)?;
        let mut append = false;
        if !(head_start >= start) {
            loop {
                prev = cur_word as i16 as i32;
                cur_word = g.u16(entry(p, cur) + RECENT + 8)?;
                cur = cur_word as i16 as i32;
                if cur == -1 {
                    append = true;
                    break;
                }
                let s = load_single(g, entry(p, cur) + RECENT)?;
                if !(s < load_single(g, new_at)?) {
                    break;
                }
            }
        }
        if append || cur == -1 {
            g.set_u16(entry(p, prev) + RECENT + 8, free)?;
            let result = load_single(g, new_at)?;
            g.set_u16(new_at + 8, 0xFFFF)?;
            return Ok(result);
        }
        if prev == -1 {
            g.set_u16(p + RECENT_HEAD, free)?;
            let result = load_single(g, new_at)?;
            g.set_u16(new_at + 8, cur_word)?;
            return Ok(result);
        }
        g.set_u16(new_at + 8, cur_word)?;
        g.set_u16(entry(p, prev) + RECENT + 8, free)?;
        return load_single(g, new_at);
    }
}

/// `sub_828EC6F0`: the per-block plug-in, `delta` seconds since the last block.
///
/// When the position has drifted more than `+32` from where the active grain was picked, the
/// other slot is released if busy, or else the active one is released and a new grain starts in
/// the other slot. Then each live voice gets the record's gain on its send and pitch on its
/// resampler, and its timer runs down (by `delta × pitch` in sustain): attack → sustain; sustain →
/// release, stopping the other slot and starting the next grain there; release → stop.
pub fn tick<H: Heap + ?Sized, T: Trig + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    player: u32,
    delta: f64,
    sp: u32,
) -> Result<()> {
    let frame = sp.wrapping_sub(TICK_FRAME);
    let p = player;
    let active = g.u32(p + ACTIVE)?;
    let position = load_single(g, p + POSITION)?;
    let threshold = load_single(g, p + DRIFT)?;
    let current = p.wrapping_add(active.wrapping_mul(SLOT_BYTES));
    let picked = load_single(g, current + SLOTS + SLOT_PICK_POSITION)?;
    let drift = sub_single(picked, position);
    if (drift > threshold || !(drift >= -threshold))
        && g.u8(p + HOLD)? == 0
        && g.u32(current + SLOTS + SLOT_GRAPH)? != 0
    {
        let o = other(active);
        let next = slot_address(p, o);
        if g.u32(next + SLOT_GRAPH)? != 0 {
            if g.u32(next + SLOT_STATE)? != STATE_RELEASE {
                let time = load_single(g, p + RELEASE)?;
                release(g, heap, next, time, frame)?;
            }
        } else {
            if g.u32(current + SLOTS + SLOT_STATE)? != STATE_RELEASE {
                let time = load_single(g, p + RELEASE)?;
                release(g, heap, current + SLOTS, time, frame)?;
            }
            let start = pick(g, p, frame)?;
            start_voice(g, heap, trig, p, next, start, frame)?;
            let a = g.u32(p + ACTIVE)?;
            g.set_u32(p + ACTIVE, other(a))?;
        }
    }
    let zero = load_single(g, ZERO)?;
    for i in 0..2u32 {
        let slot = slot_address(p, i);
        if g.u32(slot + SLOT_GRAPH)? == 0 {
            continue;
        }
        let modules = g.u32(slot + SLOT_MODULES)?;
        let gain = load_single(g, p + GAIN)?;
        device::post_property(g, g.u32(modules + 12)?, 0, gain)?;
        let modules = g.u32(slot + SLOT_MODULES)?;
        let pitch = load_single(g, p + PITCH)?;
        device::post_property(g, g.u32(modules + 4)?, 0, pitch)?;
        let state = g.u32(slot + SLOT_STATE)?;
        let timer = load_single(g, slot + SLOT_TIMER)?;
        if state == STATE_SUSTAIN {
            let pitch = load_single(g, p + PITCH)?;
            store_single(g, slot + SLOT_TIMER, nmsub_single(delta, pitch, timer))?;
        } else {
            store_single(g, slot + SLOT_TIMER, sub_single(timer, delta))?;
        }
        if load_single(g, slot + SLOT_TIMER)? >= zero {
            continue;
        }
        match state {
            STATE_ATTACK => {
                g.set_u32(slot + SLOT_STATE, STATE_SUSTAIN)?;
                store_single(g, slot + SLOT_TIMER, load_single(g, p + SUSTAIN)?)?;
            }
            STATE_SUSTAIN => {
                let time = load_single(g, p + RELEASE)?;
                release(g, heap, slot, time, frame)?;
                if g.u8(p + HOLD)? == 0 {
                    let o = other(i);
                    let next = slot_address(p, o);
                    if g.u32(next + SLOT_GRAPH)? != 0 {
                        stop_voice(g, next)?;
                    }
                    let start = pick(g, p, frame)?;
                    start_voice(g, heap, trig, p, next, start, frame)?;
                    g.set_u32(p + ACTIVE, o)?;
                }
            }
            STATE_RELEASE => {
                if g.u32(slot + SLOT_GRAPH)? != 0 {
                    stop_voice(g, slot)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// A read-only view of one player for diagnostics and tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerView {
    pub gain: f32,
    pub pitch: f32,
    pub position: f32,
    pub params: [f32; 5],
    pub active: u32,
    pub slots: [SlotView; 2],
    pub recent_head: u16,
    pub recent_last: u16,
    pub recent_free: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotView {
    pub graph: u32,
    pub timer: f32,
    pub start: f32,
    pub picked_at: f32,
    pub state: u32,
}

pub fn view(g: &Guest, player: u32) -> Result<PlayerView> {
    let slot = |i: u32| -> Result<SlotView> {
        let s = slot_address(player, i);
        Ok(SlotView {
            graph: g.u32(s + SLOT_GRAPH)?,
            timer: g.f32(s + SLOT_TIMER)?,
            start: g.f32(s + SLOT_START)?,
            picked_at: g.f32(s + SLOT_PICK_POSITION)?,
            state: g.u32(s + SLOT_STATE)?,
        })
    };
    Ok(PlayerView {
        gain: g.f32(player + GAIN)?,
        pitch: g.f32(player + PITCH)?,
        position: g.f32(player + POSITION)?,
        params: [
            g.f32(player + ATTACK)?,
            g.f32(player + SUSTAIN)?,
            g.f32(player + RELEASE)?,
            g.f32(player + WINDOW)?,
            g.f32(player + DRIFT)?,
        ],
        active: g.u32(player + ACTIVE)?,
        slots: [slot(0)?, slot(1)?],
        recent_head: g.u16(player + RECENT_HEAD)?,
        recent_last: g.u16(player + RECENT_LAST)?,
        recent_free: g.u16(player + RECENT_FREE)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The image constants these bodies load, at their guest addresses.
    fn guest() -> Guest {
        let mut g = Guest::single(0x4000_0000, 0x4000);
        for (at, v) in [
            (ONE, 1.0f32),
            (ZERO, 0.0),
            (HALF, 0.5),
            (MINUS_ONE, -1.0),
            (PERCENT, f32::from_bits(0x3C23_D70A)),
            (FOUR, 4.0),
            (DRIFT_DEFAULT, f32::from_bits(0x3D4C_CCCD)),
            (MINUS_TWO, -2.0),
            (TWO_POW_MINUS_31, f32::from_bits(0x3000_0000)),
        ] {
            g.put(at, v.to_bits().to_be_bytes().to_vec());
        }
        g.put(rng::STATE, vec![0; 24]);
        g
    }

    const PLAYER: u32 = 0x4000_1000;
    const SP: u32 = 0x4000_3F00;

    fn words(g: &Guest, at: u32, n: u32) -> Vec<u32> {
        (0..n).map(|i| g.u32(at + 4 * i).unwrap()).collect()
    }

    fn hex(s: &str) -> Vec<u32> {
        s.split_whitespace()
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect()
    }

    /// Retail capture, frame 2708: player `40C98CA0` (truck 1, A) constructed and never bound.
    #[test]
    fn construct_matches_the_capture() {
        let mut g = guest();
        construct(&mut g, PLAYER).unwrap();
        let captured = hex(concat!(
            "3F800000 3F800000 00000000 00000000 3C23D70A 3F000000 3C23D70A 40800000 3D4CCCCD ",
            "00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 ",
            "00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 ",
            "00000001 00000000 00000000 00000000 00000000 00000000 00000000 00000001 00000000 ",
            "00000000 00000000 00000000 00000000 820ED908 00000000 03000000"
        ));
        assert_eq!(words(&g, PLAYER, 43), captured);
        for i in 0..16 {
            let at = PLAYER + RECENT + 12 * i;
            assert_eq!(words(&g, at, 3), vec![0xBF80_0000, 0xBF80_0000, 0]);
        }
        assert_eq!(words(&g, PLAYER + 364, 2), vec![0, 0]);
    }

    /// Retail capture, frame 2709: player `40C98980` after binding `concrete_rough_hard` with
    /// GrainParams[0] at position 0 and its first pick (window 0.0 .. 0.4). A zero generator state
    /// draws candidate 0, which is the window retail drew; the recent list then reads exactly as
    /// captured: `{-2,-1,→1} {0,0.4,→end}`, free list 2..15, head 0, last 1, free 2.
    #[test]
    fn first_pick_matches_the_capture() {
        let mut g = guest();
        construct(&mut g, PLAYER).unwrap();
        reset_recent(&mut g, PLAYER).unwrap();
        for (i, bits) in [
            0x3DCC_CCCDu32,
            0x3E4C_CCCD,
            0x3DCC_CCCD,
            0x3FCC_CCCD,
            0x3D4C_CCCD,
        ]
        .into_iter()
        .enumerate()
        {
            g.set_u32(PLAYER + ATTACK + 4 * i as u32, bits).unwrap();
        }
        g.set_u32(PLAYER + DURATION, 0x41AE_BC39).unwrap();
        let start = pick(&mut g, PLAYER, SP).unwrap();
        assert_eq!(start, 0.0);
        let captured = hex(concat!(
            "C0000000 BF800000 00010000 00000000 3ECCCCCD FFFF0000 BF800000 BF800000 00030000 ",
            "BF800000 BF800000 00040000 BF800000 BF800000 00050000 BF800000 BF800000 00060000 ",
            "BF800000 BF800000 00070000 BF800000 BF800000 00080000 BF800000 BF800000 00090000 ",
            "BF800000 BF800000 000A0000 BF800000 BF800000 000B0000 BF800000 BF800000 000C0000 ",
            "BF800000 BF800000 000D0000 BF800000 BF800000 000E0000 BF800000 BF800000 000F0000 ",
            "BF800000 BF800000 FFFF0000 00000001 0002"
        ));
        let mut got = words(&g, PLAYER + RECENT, 49);
        got.push(u32::from(g.u16(PLAYER + RECENT_FREE).unwrap()));
        let mut want = captured;
        want[49] = 2;
        assert_eq!(got, want);
    }

    /// A second pick from the same state avoids the window just used, and candidates tile the
    /// search window back to back.
    #[test]
    fn candidates_tile_the_window() {
        let mut g = guest();
        g.put(0x4000_2000, vec![0; 0x400]);
        let interval = 0x4000_2000;
        let count = 0x4000_2010;
        let out = 0x4000_2020;
        store_single(&mut g, interval, -1.0).unwrap();
        store_single(&mut g, interval + 4, 21.84).unwrap();
        let length = f64::from(f32::from_bits(0x3ECC_CCCD));
        candidates(&mut g, interval, out, count, length, 0.0, 1.6f32.into()).unwrap();
        let n = g.u32(count).unwrap();
        let starts: Vec<f32> = (0..n).map(|k| g.f32(out + 12 * k).unwrap()).collect();
        assert_eq!(starts[0], 0.0);
        for pair in starts.windows(2) {
            assert!((pair[1] - pair[0] - 0.4).abs() < 1e-6);
        }
        assert!(n == 3 || n == 4, "{starts:?}");
    }
}
