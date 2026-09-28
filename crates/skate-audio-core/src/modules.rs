//! Building a voice's module graph: the classes' size and constructor functions, the builder
//! `sub_82B48C48`, and the command it enqueues, which installs the built graph.
//!
//! **Unverified new work.** None of these has a C++ body in `recomp/src/audio_ports/`: the device
//! open calls them on the game thread, outside the 216 audio-thread functions, so nothing here was
//! compared under the shadow harness. Each is transcribed from its lifted body, store by store, and
//! unit-tested. Stack-frame spills are not written. The `msgs1` graph probe confirmed the classes
//! and their order (`docs/audio-banks.md`).
//!
//! | function | here |
//! |---|---|
//! | `sub_82B48C48`, lay out a player and construct its modules | [`build_graph`] |
//! | `sub_82B49210` and `sub_82B49280`, the enqueued install | [`install_command`], [`install_player`] |
//! | `sub_82B46350`, copy a class's rows into a new instance | [`copy_class_rows`] |
//! | the classes' `+4` size functions | [`class_size`] |
//! | the classes' `+8` constructors | [`construct`] |
//! | `sub_82B44FE8` and `sub_82B44F98`, `Pan2D1`'s speaker tables | [`speaker_tables`] |
//! | `sub_82B39728` and `sub_82B395E8`, the system's node pool | [`pool_grow`], [`pool_take`] |
//!
//! A class descriptor, as the builder reads it: `+4` size function, `+8` constructor, `+12` the
//! function table an entry gets, `+20` a table of 40-byte rows, and bytes `+40` (a kind; `<= 3`
//! marks the player's `+70`), `+41` (the row to copy) and `+42` (how many words).
//!
//! A module descriptor is 12 bytes: `{u32 constructor argument, u32 class, u8 channels}`.
//!
//! **Allocation.** The originals allocate through the system allocator `[system + 36]` (vtable `+4`,
//! alignment 16) and free through its vtable `+12`. Here a [`Heap`] stands in for both calls; the
//! allocator object and its vtable are not read.

use crate::delay;
use crate::fp::{
    add_single, div_single, fcfid, fctiwz_low_word, fmsub_single, frsp, load_single, mul_single,
    neg_double, rlwinm, sqrt_single, store_single, sub_single,
};
use crate::mathlib::Trig;
use crate::patch::Heap;
use crate::{Error, Guest, Result, mem};

/// `lis -31993 ; lwz 30252`: the audio system every module instance records at `+8`.
pub const SYSTEM: u32 = 0x8307_762C;

/// `lis -32206 ; addi -18140`: the vtable every constructor but `SndPlayer1`'s, `Send`'s and
/// `GainFader`'s installs.
pub const BASE_VTABLE: u32 = 0x8231_B924;
/// `lis -32206 ; addi -18172`.
pub const FADER_VTABLE: u32 = 0x8231_B904;
/// `lis -32206 ; addi -18252`.
pub const SEND_VTABLE: u32 = 0x8231_B8B4;
/// `lis -32206 ; addi -17996`.
pub const SNDPLAYER_VTABLE: u32 = 0x8231_B9B4;
/// `lis -32206 ; addi -18188`: `Del0`'s property-node vtable.
pub const DELAY_VTABLE: u32 = 0x8231_B8F4;

/// `lis -32206 ; lfs -22460`: 1.0.
pub const ONE: u32 = 0x8231_A844;
/// `lis -32234 ; lfs 23056`: 0.0.
pub const ZERO: u32 = 0x8216_5A10;
/// `lis -32246 ; lfs -26788`: 0.5.
pub const HALF: u32 = 0x8209_975C;
/// `lis -32250 ; lfs 3152`: 2.0.
pub const TWO: u32 = 0x8206_0C50;
/// `lis -32233 ; lfs -8480`: -1.0.
pub const MINUS_ONE: u32 = 0x8216_DEE0;
/// `lis -32208 ; addi -31232 ; lfs 460`: 48000.0.
pub const DEFAULT_RATE: u32 = 0x822F_87CC;
/// `lis -32208 ; addi -31232 ; lfs 2188`: 450.0, what an `Iir2` adds to the player's `+40`.
pub const IIR2_COST: u32 = 0x822F_8E8C;
/// `lis -32208 ; addi -31232 ; lfs 2180`: 1500.0, what a `PI20` peak adds to player `+40`.
pub const PEAK_COST: u32 = 0x822F_8E84;
/// `lis -32247 ; lfs -2228`: 6.0, what a `Resample` adds to the player's `+40`.
pub const RESAMPLE_COST: u32 = 0x8208_F74C;
/// `lis -32249 ; lfs -12016`: π/180 as a single.
pub const DEGREES_TO_RADIANS: u32 = 0x8206_D110;
/// `lis -32243 ; lfs 18724`: 30.0, `Pan2D1`'s default front angle.
pub const PAN_FRONT: u32 = 0x820D_4924;
/// `lis -32221 ; lfs 21336`: 110.0, the default side angle.
pub const PAN_SIDE: u32 = 0x8223_5358;
/// `lis -32249 ; lfs -11920`: 150.0, the default rear angle.
pub const PAN_REAR: u32 = 0x8206_D170;
/// `lis -32219 ; lfs 28632`: 90.0, the front angle when the panner has two channels.
pub const PAN_FRONT_STEREO: u32 = 0x8225_6FD8;
/// `lis -32241 ; lfs -10884`: 100.0, the player's `+56`.
pub const HUNDRED: u32 = 0x820E_D57C;
/// `lis -32246 ; lfs -27028`: 800.0, the player's `+0`, `+4` and `+8`.
pub const EIGHT_HUNDRED: u32 = 0x8209_966C;
/// `lis -32241 ; addi -10884 ; addi 916`: the word the builder stores at the player's `+20`.
pub const PLAYER_WORD: u32 = 0x820E_D910;
/// `lis -32241 ; addi -9976`: the string "Unknown".
pub const UNKNOWN_NAME: u32 = 0x820E_D908;
/// `lis -32238 ; addi -26432`: the string "SndPlayer".
pub const SNDPLAYER_NAME: u32 = 0x8211_98C0;
/// `lis -32077 ; addi 7904`: the function `SndPlayer1` stores in its pool node.
pub const NODE_CALLBACK: u32 = 0x82B3_1EE0;
/// `lis -32075 ; addi -28144`: the command the builder enqueues.
pub const INSTALL_COMMAND: u32 = 0x82B4_9210;
/// `Sub0`'s deferred accumulator installation command.
pub const SUBMIX_INSTALL_COMMAND: u32 = 0x82B3_4B10;
/// `lis -31988 ; addi -8520`: the intrusive head for live `Sub0` accumulators.
pub const SUBMIX_HEAD: u32 = 0x830B_DEB8;
/// `lis 32759 ; ori 65521`: the word `Pan2D1` stores beside each of its three angles.
pub const PAN_ANGLE_WORD: u32 = 0x7FF7_FFF1;
/// The builder's teardown, which this module does not port.
pub const RELEASE_PLAYER: u32 = 0x82B4_8F28;

const _: () = {
    const fn lis(hi: i32, lo: i32) -> u32 {
        (((hi & 0xFFFF) << 16) as u32).wrapping_add(lo as u32)
    }
    assert!(SYSTEM == lis(-31993, 30252));
    assert!(BASE_VTABLE == lis(-32206, -18140) && FADER_VTABLE == lis(-32206, -18172));
    assert!(SEND_VTABLE == lis(-32206, -18252) && SNDPLAYER_VTABLE == lis(-32206, -17996));
    assert!(DELAY_VTABLE == lis(-32206, -18188));
    assert!(
        ONE == lis(-32206, -22460) && ZERO == lis(-32234, 23056) && HALF == lis(-32246, -26788)
    );
    assert!(TWO == lis(-32250, 3152) && MINUS_ONE == lis(-32233, -8480));
    assert!(DEFAULT_RATE == lis(-32208, -31232 + 460) && PEAK_COST == lis(-32208, -31232 + 2180));
    assert!(IIR2_COST == lis(-32208, -31232 + 2188));
    assert!(RESAMPLE_COST == lis(-32247, -2228) && DEGREES_TO_RADIANS == lis(-32249, -12016));
    assert!(PAN_FRONT == lis(-32243, 18724) && PAN_SIDE == lis(-32221, 21336));
    assert!(PAN_REAR == lis(-32249, -11920) && PAN_FRONT_STEREO == lis(-32219, 28632));
    assert!(HUNDRED == lis(-32241, -10884) && EIGHT_HUNDRED == lis(-32246, -27028));
    assert!(PLAYER_WORD == lis(-32241, -10884 + 916) && UNKNOWN_NAME == lis(-32241, -9976));
    assert!(SNDPLAYER_NAME == lis(-32238, -26432) && NODE_CALLBACK == lis(-32077, 7904));
    assert!(INSTALL_COMMAND == lis(-32075, -28144));
};

/// Size functions (class `+4`).
pub const GAIN_SIZE: u32 = 0x8267_1F50;
/// `Sub0`, the six-channel bus accumulator.
pub const SUBMIX_SIZE: u32 = 0x82B2_0648;
/// `DCl0`, the 6-channel output clipper.
pub const CLIP_SIZE: u32 = 0x82B2_6970;
/// `PI20`, the peaking equaliser.
pub const PEAK_SIZE: u32 = 0x82B2_C538;
pub const SEND_SIZE: u32 = 0x82B3_12D8;
pub const PAN_SIZE: u32 = 0x82B2_96D8;
pub const IIR2_SIZE: u32 = 0x82B2_7D18;
pub const RECHANNEL_SIZE: u32 = 0x82B2_C8C0;
pub const RESAMPLE_SIZE: u32 = 0x82B2_C9E8;
pub const SNDPLAYER_SIZE: u32 = 0x82B3_2408;
/// `Del0`, the final-output delay stage.
pub const DELAY_SIZE: u32 = 0x82B2_1F80;

/// Constructors (class `+8`).
pub const GAIN_CONSTRUCT: u32 = 0x82B2_36E8;
/// `Sub0`.
pub const SUBMIX_CONSTRUCT: u32 = 0x82B3_4A48;
/// `DCl0`.
pub const CLIP_CONSTRUCT: u32 = 0x82B2_6978;
/// `PI20`.
pub const PEAK_CONSTRUCT: u32 = 0x82B2_C540;
pub const FADER_CONSTRUCT: u32 = 0x82B2_3738;
pub const SEND_CONSTRUCT: u32 = 0x82B3_12E0;
pub const RECHANNEL_CONSTRUCT: u32 = 0x82B2_C8C8;
pub const RESAMPLE_CONSTRUCT: u32 = 0x82B2_CA00;
pub const IIR2_CONSTRUCT: u32 = 0x82B2_7D20;
pub const PAN_CONSTRUCT: u32 = 0x82B2_96E0;
pub const SNDPLAYER_CONSTRUCT: u32 = 0x82B3_25A8;
pub const DELAY_CONSTRUCT: u32 = 0x82B2_2008;

/// `sub_82B46350`: copy `[class+42]` 64-bit words into `dst`, from the class's row table
/// `[class+20]`, starting 8 bytes into row `[class+41]` and stepping 40 bytes per word.
pub fn copy_class_rows(g: &mut Guest, class: u32, dst: u32) -> Result<()> {
    let row = g.u8(class.wrapping_add(41))? as u64; // lbz r11,41(r3)
    let words = g.u8(class.wrapping_add(42))? as u64; // lbz r8,42(r3)
    let table = g.u32(class.wrapping_add(20))? as u64; // lwz r9,20(r3)
    let end = (words << 3) + dst as u64; // rotlwi r8,r8,3 ; add r11,r8,r4
    let first = rlwinm(row + (row << 2), 3, 0xFFFF_FFF8) + table; // add r7 ; rlwinm r10,r7,3,0,28 ; add
    if dst >= end as u32 {
        return Ok(()); // cmplw cr6,r4,r11 ; bgelr
    }
    let count = ((end.wrapping_sub(dst as u64).wrapping_sub(1) as u32) >> 3) + 1;
    let mut src = (first as u32).wrapping_sub(32);
    let mut out = dst.wrapping_sub(8);
    for _ in 0..count {
        src = src.wrapping_add(40); // ldu r9,40(r11)
        let word = g.u64(src)?;
        out = out.wrapping_add(8); // stdu r9,8(r10)
        g.set_u64(out, word)?;
    }
    Ok(())
}

/// A class's size function (class `+4`) on a module descriptor: the instance's byte count, as the
/// full register the builder adds.
pub fn class_size(g: &Guest, function: u32, descriptor: u32) -> Result<u64> {
    match function {
        GAIN_SIZE => Ok(64),
        SUBMIX_SIZE => Ok(168),
        CLIP_SIZE => Ok(56),
        PEAK_SIZE => Ok(240),
        SEND_SIZE => Ok(120),
        PAN_SIZE => Ok(760),
        IIR2_SIZE => Ok(208),
        RECHANNEL_SIZE => Ok(44),
        RESAMPLE_SIZE => {
            // lbz r11,8(r3) ; rotlwi r10,r11,1 ; add ; rlwinm 3,0,28 ; addi r3,r11,88
            let channels = g.u8(descriptor.wrapping_add(8))? as u64;
            Ok(rlwinm(channels + (channels << 1), 3, 0xFFFF_FFF8) + 88)
        }
        SNDPLAYER_SIZE => {
            let voices = sndplayer_voices(g, g.u32(descriptor)?)? as u64;
            let channels = g.u8(descriptor.wrapping_add(8))? as u64;
            let three = voices + rlwinm(voices, 1, 0xFFFF_FFFE); // rlwinm r10,r11,1,0,30 ; add r8
            let head = rlwinm((channels << 2) + 487, 0, 0xFFFF_FFF8); // rotlwi r9,r9,2 ; addi 487 ; rlwinm
            Ok(head + rlwinm(three, 4, 0xFFFF_FFF0)) // rlwinm r10,r8,4,0,27 ; add r3
        }
        DELAY_SIZE => Ok(192),
        // The grain chain's two extra classes (`crate::grain::fss`).
        #[cfg(target_arch = "x86_64")]
        crate::grain::fss::FSS_SIZE => crate::grain::fss::fss_size(g, descriptor),
        #[cfg(target_arch = "x86_64")]
        crate::grain::fss::HS_SIZE => Ok(crate::grain::fss::HS_SIZE_BYTES),
        _ => Err(Error::new(
            function,
            "module class size function not ported",
        )),
    }
}

/// `SndPlayer1`'s argument: the float at `+4` rounded half away from zero, or 1 with no argument.
/// Shared by its size function and its constructor, which compute it identically.
fn sndplayer_voices(g: &Guest, arg: u32) -> Result<u32> {
    if arg == 0 {
        return Ok(1);
    }
    let value = load_single(g, arg.wrapping_add(4))?;
    let zero = load_single(g, ZERO)?;
    let half = load_single(g, HALF)?;
    // fcmpu cr6,f0,f13 ; blt: an unordered compare takes the add.
    let rounded = if value < zero {
        sub_single(value, half)
    } else {
        add_single(value, half)
    };
    Ok(fctiwz_low_word(rounded))
}

/// A class's constructor (class `+8`) on a laid-out instance. `false` is the original's `r3 = 0`.
pub fn construct<H: Heap + ?Sized, T: Trig + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    function: u32,
    instance: u32,
    arg: u32,
) -> Result<bool> {
    match function {
        GAIN_CONSTRUCT => construct_gain(g, instance),
        SUBMIX_CONSTRUCT => construct_submix(g, heap, instance, arg),
        CLIP_CONSTRUCT => construct_clip(g, instance),
        PEAK_CONSTRUCT => construct_peak(g, instance),
        FADER_CONSTRUCT => construct_gain_fader(g, instance),
        SEND_CONSTRUCT => construct_send(g, instance),
        RECHANNEL_CONSTRUCT => {
            // sub_82B2C8C8: li r3,1 ; beqlr ; the vtable.
            if instance != 0 {
                g.set_u32(instance, BASE_VTABLE)?;
            }
            Ok(true)
        }
        RESAMPLE_CONSTRUCT => construct_resample(g, instance),
        IIR2_CONSTRUCT => construct_iir2(g, instance),
        PAN_CONSTRUCT => construct_pan(g, trig, instance, arg),
        SNDPLAYER_CONSTRUCT => construct_sndplayer(g, heap, instance, arg),
        DELAY_CONSTRUCT => construct_delay(g, heap, instance, arg),
        // The grain chain's two extra classes (`crate::grain::fss`).
        #[cfg(target_arch = "x86_64")]
        crate::grain::fss::FSS_CONSTRUCT => crate::grain::fss::construct_fss(g, instance, arg),
        #[cfg(target_arch = "x86_64")]
        crate::grain::fss::HS_CONSTRUCT => crate::grain::fss::construct_hs(g, instance),
        _ => Err(Error::new(function, "module constructor not ported")),
    }
}

/// `addi r4,rX,48 ; lwz r3,20(rX) ; stw r4,16(rX) ; bl sub_82B46350`, which every constructor but
/// `Rechannel`'s runs after its vtable.
fn data_rows(g: &mut Guest, instance: u32) -> Result<()> {
    let data = instance.wrapping_add(48);
    let class = g.u32(instance.wrapping_add(20))?;
    g.set_u32(instance.wrapping_add(16), data)?;
    copy_class_rows(g, class, data)
}

/// `sub_82B34A48` and its inlined `sub_82B34C90`: `Sub0`.
///
/// The retail helper allocates its accumulator through the system allocator at `+36`. The Rust
/// graph builder owns that allocation through its [`Heap`] argument, so the allocation boundary is
/// expressed there while the guest layout, command, byte rounding, and zeroing stay unchanged.
fn construct_submix<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    instance: u32,
    arg: u32,
) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
        for offset in [52, 56, 60] {
            g.set_u32(instance.wrapping_add(offset), 0)?;
        }
    }
    if arg == 0 {
        g.set_u8(instance.wrapping_add(100), 0)?;
    } else {
        let mut from = g.u32(arg.wrapping_add(4))?;
        let mut to = instance.wrapping_add(100);
        loop {
            let byte = g.u8(from)?;
            g.set_u8(to, byte)?;
            if byte == 0 {
                break;
            }
            from = from.wrapping_add(1);
            to = to.wrapping_add(1);
        }
    }
    let system = g.u32(instance.wrapping_add(8))?;
    let offset = g.u32(system.wrapping_add(204))?;
    let ring = g.u32(system.wrapping_add(48))?;
    g.set_u32(system.wrapping_add(204), offset.wrapping_add(8))?;
    g.set_u32(ring.wrapping_add(offset), SUBMIX_INSTALL_COMMAND)?;
    g.set_u32(ring.wrapping_add(offset).wrapping_add(4), instance)?;
    let player = g.u32(instance.wrapping_add(12))?;
    g.set_u8(instance.wrapping_add(165), g.u8(player.wrapping_add(73))?)?;

    let pair = instance.wrapping_add(52);
    g.set_u32(pair, instance)?;
    for offset in (4..=32).step_by(4) {
        g.set_u32(pair.wrapping_add(offset), 0)?;
    }
    g.set_u8(pair.wrapping_add(46), 0)?;
    let channels = u32::from(g.u8(instance.wrapping_add(42))?);
    // `sub_82B34C90` first rounds to eight bytes, adds sixteen, then rounds to sixteen.
    // The two steps are observably different for a zero-channel accumulator (which still
    // reserves the allocator's 16-byte minimum).
    let bytes = channels.wrapping_shl(10).wrapping_add(7) & !7;
    let bytes = bytes.wrapping_add(16) & !15;
    let buffer = heap.alloc(g, bytes, 16)?;
    g.set_u32(pair.wrapping_add(8), buffer)?;
    if buffer == 0 {
        return Ok(false);
    }
    g.fill(buffer, 0, bytes)?;
    let low = buffer & 0xFFFF;
    let padding =
        (channels.wrapping_shl(10).wrapping_add(low).wrapping_add(7) & 0xFFF8).wrapping_sub(low);
    g.set_u16(pair.wrapping_add(44), padding as u16)?;
    Ok(true)
}

/// `sub_82B236E8`: `Gain`.
fn construct_gain(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
    }
    data_rows(g, instance)?;
    let value = load_single(g, instance.wrapping_add(52))?; // lfs f0,52(r6)
    store_single(g, instance.wrapping_add(56), value)?; // stfs f0,56(r6)
    Ok(true)
}

/// `sub_82B26978`: `DCl0`, the output hard clipper.
fn construct_clip(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
    }
    data_rows(g, instance)?;
    Ok(true)
}

/// `sub_82B22008`: `Del0`'s module-local delay state.
///
/// The retail constructor also registers `instance + 160` with the device's property dispatcher.
/// That dispatcher is not yet a Rust runtime boundary, so the constructor deliberately leaves the
/// registration flag clear rather than manufacturing a node that could receive stale property
/// callbacks. The module allocation and all fields read by the audio process are reproduced here.
fn construct_delay<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    instance: u32,
    arg: u32,
) -> Result<bool> {
    let zero = load_single(g, ZERO)?;
    if instance != 0 {
        g.set_u32(instance, DELAY_VTABLE)?;
        for offset in [
            76, 80, 88, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144, 148, 152, 160,
            176,
        ] {
            g.set_u32(instance.wrapping_add(offset), 0)?;
        }
        store_single(g, instance.wrapping_add(92), zero)?;
        store_single(g, instance.wrapping_add(96), zero)?;
        g.set_u32(instance.wrapping_add(84), 1)?;
        g.set_u8(instance.wrapping_add(156), 0)?;
        g.set_u32(instance.wrapping_add(172), UNKNOWN_NAME)?;
        g.set_u8(instance.wrapping_add(180), 3)?;
    }
    data_rows(g, instance)?;
    let duration = if arg == 0 {
        zero
    } else {
        load_single(g, arg.wrapping_add(4))?
    };
    store_single(g, instance.wrapping_add(72), duration)?;
    g.set_u8(instance.wrapping_add(184), 0)?;
    g.set_u32(instance.wrapping_add(64), 0)?;
    let rate = load_single(g, DEFAULT_RATE)?;
    store_single(g, instance.wrapping_add(68), rate)?;
    let samples = mul_single(duration, rate);
    let half = load_single(g, HALF)?;
    let rounded = if samples < zero {
        sub_single(samples, half)
    } else {
        add_single(samples, half)
    };
    delay::allocate(
        g,
        heap,
        instance.wrapping_add(100),
        u32::from(g.u8(instance.wrapping_add(41))?),
        fctiwz_low_word(rounded),
        g.u32(instance.wrapping_add(84))?,
    )
}

/// `sub_82B22228`: accept a changed delay target and grow its history before the next process
/// block. The property dispatcher invokes this after changing the class-row value at `+52`; the
/// graph kernel calls it at the same boundary until that dispatcher is connected.
pub fn refresh_delay<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, instance: u32) -> Result<bool> {
    let source_duration = load_single(g, instance.wrapping_add(52))?;
    let current_duration = load_single(g, instance.wrapping_add(72))?;
    if current_duration < source_duration {
        store_single(g, instance.wrapping_add(72), source_duration)?;
    }
    let default_rate = load_single(g, DEFAULT_RATE)?;
    if load_single(g, instance.wrapping_add(68))? != default_rate {
        store_single(g, instance.wrapping_add(68), default_rate)?;
        g.set_u32(instance.wrapping_add(64), 0)?;
    }
    let samples = mul_single(
        load_single(g, instance.wrapping_add(72))?,
        load_single(g, instance.wrapping_add(68))?,
    );
    let zero = load_single(g, ZERO)?;
    let half = load_single(g, HALF)?;
    let rounded = if samples < zero {
        sub_single(samples, half)
    } else {
        add_single(samples, half)
    };
    let requested = fctiwz_low_word(rounded);
    if (g.u32(instance.wrapping_add(112))? as i32) < requested as i32 {
        return delay::resize(g, heap, instance.wrapping_add(100), requested);
    }
    Ok(true)
}

/// `sub_82B2C540`: `PI20`, the peaking equaliser.
fn construct_peak(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
        let zero = load_single(g, ZERO)?;
        for offset in (72..=196).step_by(4) {
            store_single(g, instance.wrapping_add(offset), zero)?;
        }
    }
    data_rows(g, instance)?;
    g.set_u32(instance.wrapping_add(200), 0)?;
    for (cached, source) in [(224, 52), (228, 60), (232, 68)] {
        store_single(
            g,
            instance.wrapping_add(cached),
            load_single(g, instance.wrapping_add(source))?,
        )?;
    }
    let player = g.u32(instance.wrapping_add(12))?;
    let cost = load_single(g, PEAK_COST)?;
    let previous = load_single(g, instance.wrapping_add(32))?;
    let total = load_single(g, player.wrapping_add(40))?;
    store_single(
        g,
        player.wrapping_add(40),
        add_single(sub_single(cost, previous), total),
    )?;
    store_single(g, instance.wrapping_add(32), cost)?;
    Ok(true)
}

/// `sub_82B23738`: `GainFader`.
fn construct_gain_fader(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, FADER_VTABLE)?;
    }
    data_rows(g, instance)?;
    g.set_u8(instance.wrapping_add(112), 0)?;
    g.set_u8(instance.wrapping_add(113), 0)?;
    let value = load_single(g, instance.wrapping_add(52))?;
    store_single(g, instance.wrapping_add(108), value)?;
    Ok(true)
}

/// `sub_82B312E0`: `Send`.
fn construct_send(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, SEND_VTABLE)?;
        g.set_u32(instance.wrapping_add(64), 0)?;
        g.set_u32(instance.wrapping_add(68), 0)?;
        g.set_u32(instance.wrapping_add(72), 0)?;
        g.set_u8(instance.wrapping_add(78), 0)?;
    }
    data_rows(g, instance)?;
    g.set_u8(instance.wrapping_add(116), 0)?;
    let one = load_single(g, ONE)?;
    store_single(g, instance.wrapping_add(112), one)?;
    for k in 0..8u32 {
        g.set_u32(instance.wrapping_add(80 + 4 * k), 0)?; // stwu r5,4(r11) from +76
    }
    Ok(true)
}

/// `sub_82B2CA00`: `Resample`.
fn construct_resample(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
    }
    data_rows(g, instance)?;
    // addi r11,r31,95 ; rlwinm r10,r11,0,16,28 ; subf r9,r31,r10 ; clrlwi r11,r9,16
    let base = instance as u64;
    let offset = rlwinm(base + 95, 0, 0xFFF8).wrapping_sub(base) & 0xFFFF;
    g.set_u16(instance.wrapping_add(76), offset as u16)?;
    let rows = (offset + base) as u32;
    let channels = g.u8(instance.wrapping_add(42))? as u64;
    let len = rlwinm(channels + (channels << 1), 3, 0xFFFF_FFF8);
    mem::memset(g, rows, 0, len)?; // bl sub_82EE5E80
    let player = g.u32(instance.wrapping_add(12))?;
    let cost = load_single(g, RESAMPLE_COST)?;
    g.set_u32(instance.wrapping_add(68), 0)?;
    let rate = load_single(g, DEFAULT_RATE)?;
    g.set_u32(instance.wrapping_add(72), 0)?;
    store_single(g, instance.wrapping_add(64), rate)?;
    g.set_u8(instance.wrapping_add(80), 0)?;
    let minus_one = load_single(g, MINUS_ONE)?;
    g.set_u8(instance.wrapping_add(81), 2)?;
    store_single(g, instance.wrapping_add(60), minus_one)?;
    let total = load_single(g, player.wrapping_add(40))?;
    let previous = load_single(g, instance.wrapping_add(32))?;
    let sum = add_single(sub_single(cost, previous), total); // fsubs f12,f0,f13 ; fadds f10,f12,f11
    store_single(g, player.wrapping_add(40), sum)?;
    store_single(g, instance.wrapping_add(32), cost)?;
    store_single(g, instance.wrapping_add(28), cost)?;
    Ok(true)
}

/// `sub_82B27D20`: `HighPassIir2` and `LowPassIir2`, which share it.
fn construct_iir2(g: &mut Guest, instance: u32) -> Result<bool> {
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
        let zero = load_single(g, ZERO)?;
        for k in 0..32u32 {
            store_single(g, instance.wrapping_add(56 + 4 * k), zero)?; // +56 through +180
        }
    }
    data_rows(g, instance)?;
    let player = g.u32(instance.wrapping_add(12))?;
    let cost = load_single(g, IIR2_COST)?;
    let previous = load_single(g, instance.wrapping_add(32))?;
    let delta = sub_single(cost, previous);
    let value = load_single(g, instance.wrapping_add(52))?;
    store_single(g, instance.wrapping_add(204), value)?;
    let total = load_single(g, player.wrapping_add(40))?;
    store_single(g, player.wrapping_add(40), add_single(delta, total))?;
    store_single(g, instance.wrapping_add(32), cost)?;
    Ok(true)
}

/// `sub_82B296E0`: `Pan2D1`. The argument, when present, holds four floats: `+4` front, `+12` side,
/// `+20` the law, `+28` rear.
fn construct_pan<T: Trig + ?Sized>(
    g: &mut Guest,
    trig: &mut T,
    instance: u32,
    arg: u32,
) -> Result<bool> {
    let zero = load_single(g, ZERO)?;
    if instance != 0 {
        g.set_u32(instance, BASE_VTABLE)?;
        store_single(g, instance.wrapping_add(188), zero)?;
        store_single(g, instance.wrapping_add(192), zero)?;
        g.set_u32(instance.wrapping_add(184), 0)?;
        store_single(g, instance.wrapping_add(196), zero)?;
    }
    data_rows(g, instance)?;
    let inputs = g.u8(instance.wrapping_add(41))? as u32;
    let channels = g.u8(instance.wrapping_add(42))? as u32;
    g.set_u32(instance.wrapping_add(748), inputs)?;
    let two = load_single(g, TWO)?;
    g.set_u32(instance.wrapping_add(752), channels)?;
    let (mut front, side, rear, law) = if arg != 0 {
        (
            load_single(g, arg.wrapping_add(4))?,
            load_single(g, arg.wrapping_add(12))?,
            load_single(g, arg.wrapping_add(28))?,
            load_single(g, arg.wrapping_add(20))?,
        )
    } else {
        (
            load_single(g, PAN_FRONT)?,
            load_single(g, PAN_SIDE)?,
            load_single(g, PAN_REAR)?,
            two,
        )
    };
    if channels as i32 == 2 {
        front = load_single(g, PAN_FRONT_STEREO)?;
    }
    if law == zero {
        let one = load_single(g, ONE)?;
        store_single(g, instance.wrapping_add(744), one)?;
    } else {
        let one = load_single(g, ONE)?;
        let n = inputs as i32 as i64; // extsw r11,r11
        if law == one {
            let count = frsp(fcfid(n));
            let scale = if (inputs as i32) < 6 {
                div_single(one, count)
            } else {
                div_single(one, sub_single(count, one))
            };
            store_single(g, instance.wrapping_add(744), scale)?;
        } else if law == two {
            let scale = if (inputs as i32) < 6 {
                div_single(one, sqrt_single(fcfid(n))) // fcfid f9 ; fsqrts f8,f9 -- no frsp between
            } else {
                div_single(one, sqrt_single(sub_single(frsp(fcfid(n)), one)))
            };
            store_single(g, instance.wrapping_add(744), scale)?;
        }
    }
    let f0 = load_single(g, instance.wrapping_add(100))?;
    let f10 = load_single(g, instance.wrapping_add(52))?;
    let f9 = load_single(g, instance.wrapping_add(60))?;
    let f8 = load_single(g, instance.wrapping_add(68))?;
    let f7 = load_single(g, instance.wrapping_add(76))?;
    let f6 = load_single(g, instance.wrapping_add(84))?;
    let f5 = load_single(g, instance.wrapping_add(92))?;
    for (offset, value) in [
        (704, f10),
        (708, f9),
        (712, f8),
        (716, f7),
        (720, f6),
        (724, f5),
        (728, f0),
        (700, f0),
        (732, front),
    ] {
        store_single(g, instance.wrapping_add(offset), value)?;
    }
    g.set_u32(instance.wrapping_add(104), PAN_ANGLE_WORD)?;
    store_single(g, instance.wrapping_add(108), front)?;
    store_single(g, instance.wrapping_add(736), side)?;
    g.set_u32(instance.wrapping_add(112), PAN_ANGLE_WORD)?;
    store_single(g, instance.wrapping_add(116), side)?;
    store_single(g, instance.wrapping_add(740), rear)?;
    g.set_u32(instance.wrapping_add(120), PAN_ANGLE_WORD)?;
    store_single(g, instance.wrapping_add(124), rear)?;
    let channels = g.u32(instance.wrapping_add(752))?;
    speaker_tables(
        g,
        trig,
        instance.wrapping_add(128),
        channels,
        front,
        side,
        rear,
    )?;
    Ok(true)
}

/// `cos` to `+0` and `sin` to `+4`, each rounded to single: `sub_82B44F98`, and the same four lines
/// inline in [`speaker_tables`].
fn unit_vector<T: Trig + ?Sized>(g: &mut Guest, trig: &mut T, at: u32, angle: f64) -> Result<()> {
    let c = frsp(trig.cosine(g, angle)?); // bl sub_82F4DFB0 ; frsp
    store_single(g, at, c)?;
    let s = frsp(trig.sine(g, angle)?); // bl sub_82F4DED0 ; frsp
    store_single(g, at.wrapping_add(4), s)
}

/// The 2x2 inverse `sub_82B44FE8` takes seven times, from rows `a` and `b` (`{x, y}` singles):
/// `[x_a/d, -(y_a/d), -(x_b/d), y_b/d]` with `d = x_a*y_b - y_a*x_b`, one fused multiply-subtract.
/// The operand order of every product is the lifted one.
fn inverse(g: &Guest, a: u32, b: u32) -> Result<[f64; 4]> {
    let (ax, ay) = (load_single(g, a)?, load_single(g, a.wrapping_add(4))?);
    let (bx, by) = (load_single(g, b)?, load_single(g, b.wrapping_add(4))?);
    let one = load_single(g, ONE)?;
    let det = fmsub_single(ax, by, mul_single(ay, bx));
    let inv = div_single(one, det);
    Ok([
        mul_single(inv, ax),
        neg_double(mul_single(inv, ay)),
        neg_double(mul_single(inv, bx)),
        mul_single(inv, by),
    ])
}

/// `sub_82B44FE8`: `Pan2D1`'s speaker rows and their pairwise inverses, at `table` (the instance's
/// `+128`), for `channels` outputs and three angles in degrees.
///
/// Rows are 8 bytes at `table + 8*slot`, `{cos, sin}`. Slots `+172..+184` name them: 0 for the
/// front angle, then either 1, 2, 3 or (above four channels) 2, 3, 4 for minus front, side and
/// minus side. The rear pair lives at `+40` and `+48` and is built only for eight channels.
pub fn speaker_tables<T: Trig + ?Sized>(
    g: &mut Guest,
    trig: &mut T,
    table: u32,
    channels: u32,
    front: f64,
    side: f64,
    rear: f64,
) -> Result<()> {
    let t = table;
    g.set_u32(t.wrapping_add(172), 0)?;
    let (a, b, c) = if channels as i32 > 4 {
        (2u32, 3u32, 4u32)
    } else {
        (1, 2, 3)
    };
    g.set_u32(t.wrapping_add(184), c)?;
    g.set_u32(t.wrapping_add(180), b)?;
    g.set_u32(t.wrapping_add(176), a)?;
    g.set_u32(t.wrapping_add(56), channels)?;
    let k = load_single(g, DEGREES_TO_RADIANS)?;
    let front = mul_single(front, k);
    store_single(g, t.wrapping_add(60), front)?;
    store_single(g, t.wrapping_add(64), mul_single(side, k))?;
    store_single(g, t.wrapping_add(68), mul_single(rear, k))?;
    let cosine = frsp(trig.cosine(g, front)?);
    let row = |g: &Guest, slot: u32| -> Result<u32> {
        Ok((rlwinm(g.u32(t.wrapping_add(slot))? as u64, 3, 0xFFFF_FFF8) + t as u64) as u32)
    };
    let r0 = row(g, 172)?;
    let angle = load_single(g, t.wrapping_add(60))?;
    let two = load_single(g, TWO)?;
    store_single(g, t.wrapping_add(72), mul_single(cosine, two))?;
    unit_vector(g, trig, r0, angle)?;
    let angle = neg_double(load_single(g, t.wrapping_add(60))?);
    let r1 = row(g, 176)?;
    unit_vector(g, trig, r1, angle)?;
    let angle = load_single(g, t.wrapping_add(64))?;
    let r2 = row(g, 180)?;
    unit_vector(g, trig, r2, angle)?;
    let angle = neg_double(load_single(g, t.wrapping_add(64))?);
    let r3 = row(g, 184)?;
    unit_vector(g, trig, r3, angle)?;

    // Each inverse stores in the lifted order.
    let store4 = |g: &mut Guest, m: [f64; 4], at: [u32; 4], order: [usize; 4]| -> Result<()> {
        for i in order {
            store_single(g, t.wrapping_add(at[i]), m[i])?;
        }
        Ok(())
    };
    let (p, q) = (row(g, 172)?, row(g, 176)?);
    store4(g, inverse(g, p, q)?, [76, 80, 84, 88], [0, 3, 1, 2])?;
    let (p, r) = (row(g, 172)?, row(g, 180)?);
    store4(g, inverse(g, r, p)?, [92, 96, 100, 104], [0, 3, 2, 1])?;
    let (s, q) = (row(g, 184)?, row(g, 176)?);
    store4(g, inverse(g, q, s)?, [156, 160, 164, 168], [0, 3, 1, 2])?;
    let n = channels as i32;
    if n == 4 || n == 6 {
        let (s, r) = (row(g, 184)?, row(g, 180)?);
        store4(g, inverse(g, s, r)?, [124, 128, 132, 136], [0, 3, 1, 2])?;
    }
    // blt at 0x82B452C8: the flags are the last `cmpwi r29,6`, taken on both paths.
    if n < 6 {
        return Ok(());
    }
    let one = load_single(g, ONE)?;
    store_single(g, t.wrapping_add(8), one)?;
    let zero = load_single(g, ZERO)?;
    store_single(g, t.wrapping_add(12), zero)?;
    if n != 8 {
        return Ok(());
    }
    let angle = load_single(g, t.wrapping_add(68))?;
    unit_vector(g, trig, t.wrapping_add(40), angle)?; // bl sub_82B44F98
    let angle = neg_double(load_single(g, t.wrapping_add(68))?);
    unit_vector(g, trig, t.wrapping_add(48), angle)?;
    store4(
        g,
        inverse(g, t.wrapping_add(40), t.wrapping_add(24))?,
        [108, 112, 116, 120],
        [0, 3, 1, 2],
    )?;
    store4(
        g,
        inverse(g, t.wrapping_add(48), t.wrapping_add(40))?,
        [124, 128, 132, 136],
        [3, 0, 1, 2],
    )?;
    store4(
        g,
        inverse(g, t.wrapping_add(32), t.wrapping_add(48))?,
        [140, 144, 148, 152],
        [0, 3, 1, 2],
    )?;
    Ok(())
}

/// `sub_82B325A8`: `SndPlayer1`. Allocates an external buffer of `80*voices + 8` bytes, takes a
/// node from the system pool at `system+144` (growing it by 74 when it has never grown), and
/// registers that node as the instance's first source.
fn construct_sndplayer<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    instance: u32,
    arg: u32,
) -> Result<bool> {
    let zero = load_single(g, ZERO)?;
    let voices = sndplayer_voices(g, arg)?;
    if instance != 0 {
        g.set_u32(instance, SNDPLAYER_VTABLE)?;
        g.set_u32(instance.wrapping_add(84), UNKNOWN_NAME)?;
        g.set_u32(instance.wrapping_add(72), 0)?;
        g.set_u32(instance.wrapping_add(88), 0)?;
        g.set_u8(instance.wrapping_add(92), 3)?;
    }
    data_rows(g, instance)?;
    let base = instance as u64;
    let v = voices as u64;
    g.set_u8(instance.wrapping_add(476), 0)?;
    let size = rlwinm(v + rlwinm(v, 2, 0xFFFF_FFFC), 4, 0xFFFF_FFF0) + 8;
    let gains_at = rlwinm(base + 487, 0, 0xFFFF_FFF8);
    g.set_u16(
        instance.wrapping_add(462),
        gains_at.wrapping_sub(base) as u16,
    )?;
    let channels = g.u8(instance.wrapping_add(42))? as u64;
    let _system = g.u32(instance.wrapping_add(8))?;
    let records_at = rlwinm((channels << 2) + gains_at + 7, 0, 0xFFF8);
    g.set_u16(
        instance.wrapping_add(464),
        records_at.wrapping_sub(base) as u16,
    )?;
    let buffer = heap.alloc(g, size as u32, 16)?;
    if buffer == 0 {
        return Ok(false);
    }
    g.set_u32(instance.wrapping_add(440), buffer)?;
    g.set_u8(instance.wrapping_add(470), voices as u8)?;
    g.set_u32(instance.wrapping_add(444), buffer.wrapping_add(4))?;
    g.set_u32(instance.wrapping_add(96), buffer.wrapping_add(8))?;
    let mut offset = 0u64;
    for _ in 0..voices {
        let record = (g.u16(instance.wrapping_add(464))? as u64 + offset + base) as u32;
        offset += 48;
        g.set_u8(record.wrapping_add(46), 0)?;
        g.set_u32(record.wrapping_add(40), 255)?;
    }
    let channels = g.u8(instance.wrapping_add(42))?;
    let buffer = g.u32(instance.wrapping_add(440))?;
    g.set_u8(instance.wrapping_add(466), channels)?;
    store_single(g, buffer, zero)?;
    let second = g.u32(instance.wrapping_add(444))?;
    g.set_u32(second, 0)?;
    let rate = load_single(g, DEFAULT_RATE)?;
    store_single(g, instance.wrapping_add(448), zero)?;
    g.set_u8(instance.wrapping_add(469), 0)?;
    store_single(g, instance.wrapping_add(452), zero)?;
    g.set_u8(instance.wrapping_add(468), 0)?;
    store_single(g, instance.wrapping_add(424), zero)?;
    store_single(g, instance.wrapping_add(428), rate)?;
    g.set_u8(instance.wrapping_add(467), 0)?;
    store_single(g, instance.wrapping_add(456), rate)?;
    g.set_u32(instance.wrapping_add(432), 0)?;
    g.set_u32(instance.wrapping_add(436), 0)?;
    for offset in [472, 471, 477, 473, 474, 475] {
        g.set_u8(instance.wrapping_add(offset), 0)?;
    }
    for k in 0..20u32 {
        g.set_u8(instance.wrapping_add(84 + 16 * k + 29), 0)?; // stb r30,29(r11)
        g.set_u32(instance.wrapping_add(84 + 16 * (k + 1)), 0)?; // stwu r30,16(r11)
    }
    let system = g.u32(instance.wrapping_add(8))?;
    let node = instance.wrapping_add(72);
    let pool = system.wrapping_add(112).wrapping_add(32);
    if g.u32(system.wrapping_add(172))? as i32 == 0 {
        pool_grow(g, heap, pool, 74)?;
    }
    if pool_take(g, heap, pool, node)? as u8 != 0 {
        return Ok(false);
    }
    g.set_u32(node.wrapping_add(8), instance)?;
    g.set_u32(node.wrapping_add(12), SNDPLAYER_NAME)?;
    g.set_u8(node.wrapping_add(20), 1)?;
    g.set_u8(node.wrapping_add(21), 1)?;
    g.set_u32(node.wrapping_add(4), NODE_CALLBACK)?;
    g.set_u32(node.wrapping_add(16), 0)?;
    g.set_u8(instance.wrapping_add(476), 1)?;
    // node is instance + 72, never 0, so the source slot is always taken.
    let sources = g.u8(instance.wrapping_add(43))? as u32;
    g.set_u32(instance.wrapping_add((sources + 6) * 4), node)?;
    let sources = g.u8(instance.wrapping_add(43))?;
    g.set_u8(instance.wrapping_add(43), sources.wrapping_add(1))?;
    let mut at = (g.u16(instance.wrapping_add(462))? as u64 + base) as u32;
    if g.u8(instance.wrapping_add(466))? != 0 {
        at = at.wrapping_sub(4);
        let mut i = 0i32;
        loop {
            i += 1;
            at = at.wrapping_add(4);
            store_single(g, at, zero)?; // stfsu f31,4(r11)
            if i >= g.u8(instance.wrapping_add(466))? as i32 {
                break;
            }
        }
    }
    Ok(true)
}

/// `sub_82B39728`: grow the node pool at `pool` by `extra` plus its current total. Returns the
/// original's `r3`: 0 when it grew, 1 when the allocation failed.
///
/// The pool: `+0` first block, `+4` last block, `+8` block count, `+12` free list, `+16` used
/// list, `+24` nodes in use, `+28` nodes in all. A block is `{next, count}` then 16-byte nodes
/// `{next, prev, owner, u8 in use}`.
pub fn pool_grow<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    pool: u32,
    extra: u64,
) -> Result<u32> {
    let total = g.u32(pool.wrapping_add(28))? as u64;
    let count = extra.wrapping_add(total); // add r30,r4,r10
    let size = rlwinm(count, 4, 0xFFFF_FFF0) + 8;
    let block = heap.alloc(g, size as u32, 16)?;
    if block == 0 {
        return Ok(1);
    }
    g.set_u32(block.wrapping_add(4), count as u32)?;
    g.set_u32(block, 0)?;
    if g.u32(pool)? == 0 {
        g.set_u32(pool, block)?;
    } else {
        let last = g.u32(pool.wrapping_add(4))?;
        g.set_u32(last, block)?;
    }
    let blocks = g.u32(pool.wrapping_add(8))?;
    g.set_u32(pool.wrapping_add(4), block)?;
    g.set_u32(pool.wrapping_add(8), blocks.wrapping_add(1))?;
    if count as u32 as i32 > 0 {
        let mut node = block.wrapping_add(8);
        for _ in 0..count as u32 {
            g.set_u32(node.wrapping_add(8), 0)?;
            let free = g.u32(pool.wrapping_add(12))?;
            g.set_u32(node, free)?;
            g.set_u32(node.wrapping_add(4), 0)?;
            let free = g.u32(pool.wrapping_add(12))?;
            if free != 0 {
                g.set_u32(free.wrapping_add(4), node)?;
            }
            g.set_u32(pool.wrapping_add(12), node)?;
            node = node.wrapping_add(16);
        }
    }
    let total = g.u32(pool.wrapping_add(28))? as u64;
    g.set_u32(pool.wrapping_add(28), count.wrapping_add(total) as u32)?;
    Ok(0)
}

/// `sub_82B395E8`: move a node from the pool's free list to its used list, recording `out` in it
/// and it in `out`. Returns the original's `r3`: 0, or [`pool_grow`]'s failure.
pub fn pool_take<H: Heap + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    pool: u32,
    out: u32,
) -> Result<u32> {
    let mut status = 0u32;
    if g.u32(pool.wrapping_add(12))? == 0 {
        let used = g.u32(pool.wrapping_add(24))? as u64;
        status = pool_grow(g, heap, pool, used + 1)?;
    }
    if status as u8 != 0 {
        return Ok(status);
    }
    let node = g.u32(pool.wrapping_add(12))?;
    if node != 0 {
        let next = g.u32(node)?;
        g.set_u32(pool.wrapping_add(12), next)?;
        if next != 0 {
            g.set_u32(next.wrapping_add(4), 0)?;
        }
    }
    g.set_u32(node.wrapping_add(8), out)?;
    g.set_u8(node.wrapping_add(12), 1)?;
    g.set_u32(out, node)?;
    let used = g.u32(pool.wrapping_add(16))?;
    g.set_u32(node, used)?;
    g.set_u32(node.wrapping_add(4), 0)?;
    let used = g.u32(pool.wrapping_add(16))?;
    if used != 0 {
        g.set_u32(used.wrapping_add(4), node)?;
    }
    g.set_u32(pool.wrapping_add(16), node)?;
    let count = g.u32(pool.wrapping_add(24))?;
    g.set_u32(pool.wrapping_add(24), count.wrapping_add(1))?;
    Ok(status)
}

/// `sub_82B39820`: retire the head allocation block of a node pool when another block can cover
/// every one of its entries.
///
/// The routine first removes free entries belonging to the retiring block.  It then evacuates live
/// entries by returning their held node to the pool and taking a replacement for their owner.  The
/// allocation itself is finally released through [`Heap`].  `Heap::free` is the only host boundary
/// here: the original reaches the same allocator entry through its global vtable.
pub fn pool_retire_head<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, pool: u32) -> Result<u32> {
    const BLOCK_NEXT: u32 = 0;
    const BLOCK_ENTRIES: u32 = 4;
    const BLOCK_NODES: u32 = 8;
    const NODE_BYTES: u32 = 16;
    const NODE_OWNER: u32 = 8;
    const POOL_BLOCK_TAIL: u32 = 4;
    const POOL_BLOCK_COUNT: u32 = 8;
    const POOL_IN_USE: u32 = 24;
    const POOL_CAPACITY: u32 = 28;

    /// The list splice shared by both loops.  It removes `node` from the free list only; unlike
    /// `scheduler::recycle_node`, it does not enqueue it again or alter the active-node count.
    fn unlink_free(g: &mut Guest, pool: u32, node: u32) -> Result<()> {
        if g.u32(pool + 12)? == node {
            g.set_u32(pool + 12, g.u32(node)?)?;
        }
        let previous = g.u32(node + 4)?;
        if previous != 0 {
            g.set_u32(previous, g.u32(node)?)?;
        }
        let next = g.u32(node)?;
        if next != 0 {
            g.set_u32(next + 4, g.u32(node + 4)?)?;
        }
        Ok(())
    }

    let block = g.u32(pool)?;
    if block == 0 || g.u32(block + BLOCK_NEXT)? == 0 {
        return Ok(0);
    }
    let mut count = g.u32(block + BLOCK_ENTRIES)? as i32;
    let slack = g
        .u32(pool + POOL_CAPACITY)?
        .wrapping_sub(g.u32(pool + POOL_IN_USE)?);
    if count > slack as i32 {
        return Ok(0);
    }

    if count > 0 {
        let mut node = block + BLOCK_NODES;
        let mut index = 0i32;
        loop {
            if g.u32(node + NODE_OWNER)? == 0 {
                unlink_free(g, pool, node)?;
            }
            count = g.u32(block + BLOCK_ENTRIES)? as i32;
            index += 1;
            node = node.wrapping_add(NODE_BYTES);
            if index >= count {
                break;
            }
        }
    }

    if count > 0 {
        let mut node = block + BLOCK_NODES;
        let mut index = 0i32;
        loop {
            let owner = g.u32(node + NODE_OWNER)?;
            if owner != 0 {
                let held = g.u32(owner)?;
                g.set_u32(owner, 0)?;
                g.set_u32(held + NODE_OWNER, 0)?;
                crate::scheduler::recycle_node(g, pool, held)?;
                unlink_free(g, pool, node)?;
                pool_take(g, heap, pool, owner)?;
            }
            let bound = g.u32(block + BLOCK_ENTRIES)? as i32;
            index += 1;
            node = node.wrapping_add(NODE_BYTES);
            if index >= bound {
                break;
            }
        }
    }

    // The head is deliberately reloaded after possible pool growth by `pool_take`.
    let head = g.u32(pool)?;
    if head != 0 {
        let next = g.u32(head + BLOCK_NEXT)?;
        g.set_u32(pool, next)?;
        if next == 0 {
            g.set_u32(pool + POOL_BLOCK_TAIL, 0)?;
        }
        g.set_u32(
            pool + POOL_BLOCK_COUNT,
            g.u32(pool + POOL_BLOCK_COUNT)?.wrapping_sub(1),
        )?;
    }
    g.set_u32(
        pool + POOL_CAPACITY,
        g.u32(pool + POOL_CAPACITY)?
            .wrapping_sub(g.u32(block + BLOCK_ENTRIES)?),
    )?;
    heap.free(g, block)?;
    Ok(1)
}

/// `sub_82B48C48`: lay out a player for `count` module descriptors at `descriptors`, construct each
/// module, and enqueue [`INSTALL_COMMAND`] on the system's command ring. Returns the player, or 0
/// when its allocation fails.
///
/// The player: a header, then an entry table at `+24` of `{function table, u16 size}` per module,
/// then the instances, each 16-aligned. Instance header: `+4` 0, `+8` [`SYSTEM`]'s value, `+12`
/// the player, `+20` the class, `+28`/`+32` 0.0, `+36` 0, `+40` 0, `+41` the previous module's
/// channels, `+42` this one's, `+43` 0. Module pointers go to the player's `+80`.
///
/// A constructor that fails sends the original through its teardown `sub_82B48F28`, which is
/// not ported: that case is an `Err`, with the instances built so far left in place.
pub fn build_graph<H: Heap + ?Sized, T: Trig + ?Sized>(
    g: &mut Guest,
    heap: &mut H,
    trig: &mut T,
    system: u32,
    order: u8,
    count: u32,
    descriptors: u32,
) -> Result<u32> {
    let n = count as u64;
    let lead = rlwinm(n.wrapping_sub(1), 2, 0xFFFF_FFFC); // r26
    let entries = rlwinm(n, 3, 0xFFFF_FFF8); // r27
    let mut size = rlwinm(lead + 91, 0, 0xFFFF_FFF8) + entries;
    for i in 0..count {
        let d = descriptors.wrapping_add(12 * i);
        let class = g.u32(d.wrapping_add(4))?;
        let function = g.u32(class.wrapping_add(4))?;
        size = class_size(g, function, d)?.wrapping_add(rlwinm(size + 15, 0, 0xFFFF_FFF0));
    }
    let request = if size as u32 == 0 { 84 } else { size };
    let player = heap.alloc(g, request as u32, 16)?;
    if player == 0 {
        return Ok(0);
    }
    g.set_u32(player.wrapping_add(12), 0)?;
    g.set_u32(player.wrapping_add(64), size as u32)?;
    for i in 0..count {
        g.set_u32(player.wrapping_add(80 + 4 * i), 0)?;
    }
    g.set_u32(player.wrapping_add(16), system)?;
    g.set_u8(player.wrapping_add(71), 0)?;
    g.set_u32(player.wrapping_add(76), 0)?;
    g.set_u8(player.wrapping_add(68), count as u8)?;
    g.set_u8(player.wrapping_add(73), order)?;
    let one = load_single(g, ONE)?;
    g.set_u8(player.wrapping_add(72), 2)?;
    let zero = load_single(g, ZERO)?;
    g.set_u32(player.wrapping_add(20), PLAYER_WORD)?;
    store_single(g, player.wrapping_add(36), one)?;
    store_single(g, player.wrapping_add(40), zero)?;
    store_single(g, player.wrapping_add(44), zero)?;
    store_single(g, player.wrapping_add(48), zero)?;
    let tail = g.u32(system.wrapping_add(256))?;
    let hundred = load_single(g, HUNDRED)?;
    store_single(g, player.wrapping_add(56), hundred)?;
    g.set_u8(player.wrapping_add(69), 0)?;
    let table = rlwinm(player as u64 + lead + 91, 0, 0xFFFF_FFF8);
    g.set_u32(player.wrapping_add(60), 0)?;
    g.set_u32(player.wrapping_add(52), tail)?;
    let far = load_single(g, EIGHT_HUNDRED)?;
    let mut next = table + entries; // r24
    store_single(g, player, far)?;
    store_single(g, player.wrapping_add(4), far)?;
    store_single(g, player.wrapping_add(8), far)?;
    g.set_u32(player.wrapping_add(24), table as u32)?;
    g.set_u8(player.wrapping_add(70), 255)?;
    let mut previous = 0u8; // r25
    for i in 0..count {
        let d = descriptors.wrapping_add(12 * i);
        let class = g.u32(d.wrapping_add(4))?;
        let entry = (8 * i as u64 + g.u32(player.wrapping_add(24))? as u64) as u32;
        if g.u8(class.wrapping_add(40))? <= 3 {
            g.set_u8(player.wrapping_add(70), i as u8)?;
        }
        let function = g.u32(class.wrapping_add(4))?;
        let bytes = class_size(g, function, d)? & 0xFFFF; // clrlwi r11,r3,16
        let system_word = g.u32(SYSTEM)?;
        let instance = rlwinm(next + 15, 0, 0xFFFF_FFF0) as u32;
        g.set_u16(entry.wrapping_add(4), bytes as u16)?;
        next = bytes + instance as u64;
        store_single(g, instance.wrapping_add(28), zero)?;
        g.set_u8(instance.wrapping_add(40), 0)?;
        store_single(g, instance.wrapping_add(32), zero)?;
        g.set_u32(instance.wrapping_add(36), 0)?;
        g.set_u32(instance.wrapping_add(12), player)?;
        g.set_u32(instance.wrapping_add(20), class)?;
        g.set_u32(instance.wrapping_add(8), system_word)?;
        g.set_u8(instance.wrapping_add(41), previous)?;
        let channels = g.u8(d.wrapping_add(8))?;
        g.set_u8(instance.wrapping_add(42), channels)?;
        g.set_u32(instance.wrapping_add(4), 0)?;
        g.set_u8(instance.wrapping_add(43), 0)?;
        let arg = g.u32(d)?;
        let constructor = g.u32(class.wrapping_add(8))?;
        if !construct(g, heap, trig, constructor, instance, arg)? {
            return Err(Error::new(
                RELEASE_PLAYER,
                "a module constructor failed, and the builder's teardown is not ported",
            ));
        }
        g.set_u32(player.wrapping_add(80 + 4 * i), instance)?;
        let functions = g.u32(class.wrapping_add(12))?;
        g.set_u32(entry, functions)?;
        previous = g.u8(d.wrapping_add(8))?;
    }
    let offset = g.u32(system.wrapping_add(204))?;
    let ring = g.u32(system.wrapping_add(48))?;
    g.set_u32(system.wrapping_add(204), offset.wrapping_add(8))?;
    let at = ring.wrapping_add(offset);
    g.set_u32(at, INSTALL_COMMAND)?;
    g.set_u32(at.wrapping_add(4), player)?;
    Ok(player)
}

/// `sub_82B34B10`: install one `Sub0` accumulator on its global intrusive list.
pub fn install_submix(g: &mut Guest, record: u32) -> Result<u32> {
    let submix = g.u32(record.wrapping_add(4))?;
    let head = g.u32(SUBMIX_HEAD)?;
    g.set_u32(submix.wrapping_add(48), 0)?;
    g.set_u32(submix.wrapping_add(44), head)?;
    if head != 0 {
        g.set_u32(head.wrapping_add(4), submix.wrapping_add(44))?;
    }
    g.set_u32(SUBMIX_HEAD, submix.wrapping_add(44))?;
    g.set_u8(submix.wrapping_add(164), 1)?;
    Ok(8)
}

/// `sub_82B49210`: the command record `{function, player}`. Returns its size, 8.
pub fn install_command<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, record: u32) -> Result<u32> {
    let player = g.u32(record.wrapping_add(4))?;
    install_player(g, heap, player)?;
    Ok(8)
}

/// `sub_82B49280`: insert `player` into the system's player list at `+108` (8-byte entries
/// `{player, size}`, count `+280`, capacity `+282`), before the first entry whose `+73` order is not
/// below its own, growing the list by `[system+276]` the first time and by 32 after.
///
/// When the list cannot grow the player is marked (`+71` = 2, `+76` = 1) and linked through its
/// `+28` onto the system's `+16` list instead, and the result is `false`.
pub fn install_player<H: Heap + ?Sized>(g: &mut Guest, heap: &mut H, player: u32) -> Result<bool> {
    let system = g.u32(player.wrapping_add(16))?;
    let count = g.u16(system.wrapping_add(280))?;
    let capacity = g.u16(system.wrapping_add(282))?;
    if count >= capacity {
        let step = if capacity == 0 {
            g.u32(system.wrapping_add(276))? as u64
        } else {
            32
        };
        let capacity = g.u16(system.wrapping_add(282))? as u64;
        let grown = capacity + step; // r29
        let list = heap.alloc(g, rlwinm(grown, 3, 0xFFFF_FFF8) as u32, 16)?;
        if list == 0 {
            let system = g.u32(player.wrapping_add(16))?;
            g.set_u8(player.wrapping_add(71), 2)?;
            g.set_u32(player.wrapping_add(76), 1)?;
            let link = player.wrapping_add(28);
            let head = g.u32(system.wrapping_add(16))?;
            g.set_u32(player.wrapping_add(32), 0)?;
            g.set_u32(player.wrapping_add(28), head)?;
            let head = g.u32(system.wrapping_add(16))?;
            if head != 0 {
                g.set_u32(head.wrapping_add(4), link)?;
            }
            g.set_u32(system.wrapping_add(16), link)?;
            return Ok(false);
        }
        let old = g.u32(g.u32(player.wrapping_add(16))?.wrapping_add(108))?;
        mem::memcpy(g, list, old, capacity << 3)?; // bl sub_82EDF460
        let old = g.u32(g.u32(player.wrapping_add(16))?.wrapping_add(108))?;
        heap.free(g, old)?;
        let system = g.u32(player.wrapping_add(16))?;
        g.set_u32(system.wrapping_add(108), list)?;
        let system = g.u32(player.wrapping_add(16))?;
        g.set_u16(system.wrapping_add(282), grown as u16)?;
    }
    let system = g.u32(player.wrapping_add(16))?; // r9
    let count = g.u16(system.wrapping_add(280))?; // r8
    let mut index = 0u32;
    if count as i32 > 0 {
        let order = g.u8(player.wrapping_add(73))?;
        let mut entry = g.u32(system.wrapping_add(108))?;
        loop {
            let other = g.u32(entry)?;
            if order <= g.u8(other.wrapping_add(73))? {
                break;
            }
            let current = g.u32(player.wrapping_add(16))?;
            index += 1;
            entry = entry.wrapping_add(8);
            if index as i32 >= g.u16(current.wrapping_add(280))? as i32 {
                break;
            }
        }
    }
    let list = g.u32(system.wrapping_add(108))?;
    let at = rlwinm(index as u64, 3, 0xFFFF_FFF8) as u32;
    let from = list.wrapping_add(at);
    let len = rlwinm((count as u64).wrapping_sub(index as u64), 3, 0xFFFF_FFF8);
    mem::memmove(g, from.wrapping_add(8), from, len)?; // bl sub_82F4DC60
    let list = g.u32(g.u32(player.wrapping_add(16))?.wrapping_add(108))?;
    g.set_u32(list.wrapping_add(at), player)?;
    let system = g.u32(player.wrapping_add(16))?;
    let size = g.u32(player.wrapping_add(64))?;
    let list = g.u32(system.wrapping_add(108))?;
    g.set_u32(list.wrapping_add(at).wrapping_add(4), size)?;
    let system = g.u32(player.wrapping_add(16))?;
    let count = g.u16(system.wrapping_add(280))?;
    g.set_u16(system.wrapping_add(280), count.wrapping_add(1))?;
    let system = g.u32(player.wrapping_add(16))?;
    let count = g.u16(system.wrapping_add(280))? as u32;
    if count > g.u32(system.wrapping_add(272))? {
        g.set_u32(system.wrapping_add(272), count)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::BumpHeap;

    const MEM: u32 = 0x5000_0000;
    const GAIN_CLASS: u32 = MEM + 0x200;
    const SEND_CLASS: u32 = MEM + 0x240;
    const OTHER_CLASS: u32 = MEM + 0x280;
    const DELAY_INSTANCE: u32 = MEM + 0x1800;
    const ROWS: u32 = MEM + 0x300;
    const DESCRIPTORS: u32 = MEM + 0x400;
    const ARG: u32 = MEM + 0x4C0;
    const PLAYERS: u32 = MEM + 0x600;
    const RING: u32 = MEM + 0x800;
    const SYS: u32 = MEM + 0xC00;
    const HEAP: u32 = MEM + 0x1000;
    const ROW_WORD: u64 = 0x1111_2222_3333_0000;

    /// Test-only trigonometry; the real callers need the guest's own `sub_82F4DED0`/`sub_82F4DFB0`.
    struct Libm;
    impl Trig for Libm {
        fn sine(&mut self, _g: &Guest, x: f64) -> Result<f64> {
            Ok(x.sin())
        }
        fn cosine(&mut self, _g: &Guest, x: f64) -> Result<f64> {
            Ok(x.cos())
        }
    }

    fn guest() -> Guest {
        let mut g = Guest::single(MEM, 0x4000);
        for (at, v) in [
            (ONE, 1.0f32),
            (ZERO, 0.0),
            (HALF, 0.5),
            (TWO, 2.0),
            (MINUS_ONE, -1.0),
            (DEFAULT_RATE, 48000.0),
            (IIR2_COST, 450.0),
            (PEAK_COST, 1500.0),
            (RESAMPLE_COST, 6.0),
            (DEGREES_TO_RADIANS, std::f32::consts::PI / 180.0),
            (PAN_FRONT, 30.0),
            (PAN_SIDE, 110.0),
            (PAN_REAR, 150.0),
            (PAN_FRONT_STEREO, 90.0),
            (HUNDRED, 100.0),
            (EIGHT_HUNDRED, 800.0),
        ] {
            g.put(at, v.to_bits().to_be_bytes().to_vec());
        }
        g.put(SYSTEM, SYS.to_be_bytes().to_vec());
        g.set_u32(SYS + 48, RING).unwrap();
        g.set_u32(SYS + 256, 0x1234).unwrap();
        for k in 0..6u32 {
            g.set_u64(ROWS + 8 + 40 * k, ROW_WORD + k as u64).unwrap();
        }
        g
    }

    fn heap() -> BumpHeap {
        BumpHeap {
            next: HEAP,
            end: MEM + 0x4000,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn class(
        g: &mut Guest,
        at: u32,
        size: u32,
        construct: u32,
        functions: u32,
        kind: u8,
        row: u8,
        words: u8,
    ) {
        g.set_u32(at + 4, size).unwrap();
        g.set_u32(at + 8, construct).unwrap();
        g.set_u32(at + 12, functions).unwrap();
        g.set_u32(at + 20, ROWS).unwrap();
        g.set_u8(at + 40, kind).unwrap();
        g.set_u8(at + 41, row).unwrap();
        g.set_u8(at + 42, words).unwrap();
    }

    fn descriptor(g: &mut Guest, i: u32, arg: u32, class: u32, channels: u8) {
        g.set_u32(DESCRIPTORS + 12 * i, arg).unwrap();
        g.set_u32(DESCRIPTORS + 12 * i + 4, class).unwrap();
        g.set_u8(DESCRIPTORS + 12 * i + 8, channels).unwrap();
    }

    fn f32_at(g: &Guest, at: u32) -> f32 {
        g.f32(at).unwrap()
    }

    #[test]
    fn delay_constructor_uses_the_titles_history_layout() {
        let mut g = guest();
        let mut heap = heap();
        class(&mut g, OTHER_CLASS, DELAY_SIZE, DELAY_CONSTRUCT, 0, 0, 0, 0);
        g.set_u32(DELAY_INSTANCE + 20, OTHER_CLASS).unwrap();
        g.set_u8(DELAY_INSTANCE + 41, 1).unwrap();

        assert!(construct_delay(&mut g, &mut heap, DELAY_INSTANCE, 0).unwrap());

        assert_eq!(g.u32(DELAY_INSTANCE).unwrap(), DELAY_VTABLE);
        assert_eq!(f32_at(&g, DELAY_INSTANCE + 68), 48_000.0);
        assert_eq!(f32_at(&g, DELAY_INSTANCE + 72), 0.0);
        assert_eq!(g.u32(DELAY_INSTANCE + 84).unwrap(), 1);
        assert_eq!(g.u32(DELAY_INSTANCE + 100).unwrap(), HEAP);
        assert_eq!(g.u32(DELAY_INSTANCE + 112).unwrap(), 256);
        assert_eq!(g.u32(DELAY_INSTANCE + 120).unwrap(), 288);
        assert_eq!(g.u32(DELAY_INSTANCE + 132).unwrap(), 288);
        assert_eq!(g.u8(DELAY_INSTANCE + 184).unwrap(), 0);
    }

    #[test]
    fn delay_refresh_grows_the_history_from_the_class_row_target() {
        let mut g = guest();
        let mut heap = heap();
        class(&mut g, OTHER_CLASS, DELAY_SIZE, DELAY_CONSTRUCT, 0, 0, 0, 0);
        g.set_u32(DELAY_INSTANCE + 20, OTHER_CLASS).unwrap();
        g.set_u8(DELAY_INSTANCE + 41, 1).unwrap();
        construct_delay(&mut g, &mut heap, DELAY_INSTANCE, 0).unwrap();
        store_single(&mut g, DELAY_INSTANCE + 52, 0.01).unwrap();
        store_single(&mut g, DELAY_INSTANCE + 68, 44_100.0).unwrap();
        g.set_u32(DELAY_INSTANCE + 64, 9).unwrap();

        assert!(refresh_delay(&mut g, &mut heap, DELAY_INSTANCE).unwrap());

        assert_eq!(f32_at(&g, DELAY_INSTANCE + 72), 0.01);
        assert_eq!(f32_at(&g, DELAY_INSTANCE + 68), 48_000.0);
        assert_eq!(g.u32(DELAY_INSTANCE + 64).unwrap(), 0);
        assert_eq!(g.u32(DELAY_INSTANCE + 112).unwrap(), 480);
        assert_eq!(g.u32(DELAY_INSTANCE + 120).unwrap(), 512);
    }

    /// `m` at `at` is `[x_a/d, -y_a/d, -x_b/d, y_b/d]`: check it inverts the rows at `a` and `b`.
    fn assert_inverse(g: &Guest, at: u32, a: u32, b: u32) {
        let m: Vec<f64> = (0..4).map(|i| f32_at(g, at + 4 * i) as f64).collect();
        let (ax, ay, bx, by) = (
            f32_at(g, a) as f64,
            f32_at(g, a + 4) as f64,
            f32_at(g, b) as f64,
            f32_at(g, b + 4) as f64,
        );
        let close =
            |x: f64, want: f64| assert!((x - want).abs() < 1e-5, "{x} vs {want} at {at:#x}");
        close(m[3] * ax + m[2] * ay, 1.0);
        close(m[3] * bx + m[2] * by, 0.0);
        close(m[1] * ax + m[0] * ay, 0.0);
        close(m[1] * bx + m[0] * by, 1.0);
    }

    #[test]
    fn sizes_follow_the_class_functions() {
        let mut g = guest();
        g.set_u8(DESCRIPTORS + 8, 2).unwrap();
        assert_eq!(class_size(&g, GAIN_SIZE, DESCRIPTORS).unwrap(), 64);
        assert_eq!(class_size(&g, SEND_SIZE, DESCRIPTORS).unwrap(), 120);
        assert_eq!(
            class_size(&g, RESAMPLE_SIZE, DESCRIPTORS).unwrap(),
            24 * 2 + 88
        );
        // No argument is one voice: the channel gains, 8-aligned, then one 48-byte record.
        assert_eq!(
            class_size(&g, SNDPLAYER_SIZE, DESCRIPTORS).unwrap(),
            488 + 48
        );
        g.set_u32(DESCRIPTORS, ARG).unwrap();
        g.set_u32(ARG + 4, 2.4f32.to_bits()).unwrap();
        assert_eq!(
            class_size(&g, SNDPLAYER_SIZE, DESCRIPTORS).unwrap(),
            488 + 96
        );
        g.set_u32(ARG + 4, 2.5f32.to_bits()).unwrap();
        assert_eq!(
            class_size(&g, SNDPLAYER_SIZE, DESCRIPTORS).unwrap(),
            488 + 144,
            "2.5 rounds away from zero"
        );
        assert!(class_size(&g, 0x8200_0000, DESCRIPTORS).is_err());
    }

    #[test]
    fn class_rows_come_from_the_named_row_one_word_per_row() {
        let mut g = guest();
        class(&mut g, SEND_CLASS, SEND_SIZE, SEND_CONSTRUCT, 0, 2, 1, 2);
        copy_class_rows(&mut g, SEND_CLASS, MEM + 0x900).unwrap();
        assert_eq!(g.u64(MEM + 0x900).unwrap(), ROW_WORD + 1);
        assert_eq!(g.u64(MEM + 0x908).unwrap(), ROW_WORD + 2);
        assert_eq!(g.u64(MEM + 0x910).unwrap(), 0);
        g.set_u8(SEND_CLASS + 42, 0).unwrap();
        g.set_u64(MEM + 0x900, 7).unwrap();
        copy_class_rows(&mut g, SEND_CLASS, MEM + 0x900).unwrap();
        assert_eq!(g.u64(MEM + 0x900).unwrap(), 7, "no words, no copy");
    }

    #[test]
    fn submix_builds_its_accumulator_and_enqueues_its_install() {
        let mut g = guest();
        class(
            &mut g,
            GAIN_CLASS,
            SUBMIX_SIZE,
            SUBMIX_CONSTRUCT,
            0,
            5,
            0,
            0,
        );
        descriptor(&mut g, 0, 0, GAIN_CLASS, 6);
        let player = build_graph(&mut g, &mut heap(), &mut Libm, SYS, 253, 1, DESCRIPTORS).unwrap();
        let submix = g.u32(player + 80).unwrap();
        let pair = submix + 52;
        let accumulator = g.u32(pair + 8).unwrap();
        assert_eq!(g.u32(submix).unwrap(), BASE_VTABLE);
        assert_eq!(g.u32(pair).unwrap(), submix);
        assert_eq!(g.u8(submix + 165).unwrap(), 253);
        assert_eq!(g.u16(pair + 44).unwrap(), 6144);
        assert!(
            g.span(accumulator, 6144)
                .unwrap()
                .iter()
                .all(|&byte| byte == 0)
        );
        assert_eq!(
            [
                g.u32(RING).unwrap(),
                g.u32(RING + 4).unwrap(),
                g.u32(SYS + 204).unwrap()
            ],
            [SUBMIX_INSTALL_COMMAND, submix, 16]
        );
    }

    #[test]
    fn submix_keeps_the_retail_sixteen_byte_minimum_for_zero_channels() {
        let mut g = guest();
        class(
            &mut g,
            GAIN_CLASS,
            SUBMIX_SIZE,
            SUBMIX_CONSTRUCT,
            0,
            5,
            0,
            0,
        );
        descriptor(&mut g, 0, 0, GAIN_CLASS, 0);
        let mut heap = heap();
        let player = build_graph(&mut g, &mut heap, &mut Libm, SYS, 0, 1, DESCRIPTORS).unwrap();
        let pair = g.u32(player + 80).unwrap() + 52;
        let accumulator = g.u32(pair + 8).unwrap();
        assert_eq!(heap.next, accumulator + 16);
        assert_eq!(g.u16(pair + 44).unwrap(), 0);
        assert!(
            g.span(accumulator, 16)
                .unwrap()
                .iter()
                .all(|&byte| byte == 0)
        );
    }

    #[test]
    fn a_gain_and_a_send_are_laid_out_constructed_and_enqueued() {
        let mut g = guest();
        class(
            &mut g,
            GAIN_CLASS,
            GAIN_SIZE,
            GAIN_CONSTRUCT,
            0xAAAA_0001,
            5,
            0,
            1,
        );
        class(
            &mut g,
            SEND_CLASS,
            SEND_SIZE,
            SEND_CONSTRUCT,
            0xAAAA_0002,
            2,
            1,
            2,
        );
        descriptor(&mut g, 0, 0, GAIN_CLASS, 1);
        descriptor(&mut g, 1, 0, SEND_CLASS, 2);
        let mut heap = heap();
        let p = build_graph(&mut g, &mut heap, &mut Libm, SYS, 7, 2, DESCRIPTORS).unwrap();
        // Header align8(4 + 91) + 16 = 104, then 64 at +112 and 120 at +176.
        assert_eq!(p, HEAP);
        assert_eq!(g.u32(p + 64).unwrap(), 296);
        assert_eq!(heap.next, p + 296);
        assert_eq!(g.u32(p + 24).unwrap(), p + 88);
        let (gain, send) = (p + 112, p + 176);
        assert_eq!(
            [g.u32(p + 80).unwrap(), g.u32(p + 84).unwrap()],
            [gain, send]
        );
        assert_eq!(
            (g.u32(p + 88).unwrap(), g.u16(p + 92).unwrap()),
            (0xAAAA_0001, 64)
        );
        assert_eq!(
            (g.u32(p + 96).unwrap(), g.u16(p + 100).unwrap()),
            (0xAAAA_0002, 120)
        );
        assert_eq!(
            g.u8(p + 70).unwrap(),
            1,
            "the last module whose kind is <= 3"
        );
        assert_eq!(
            (
                g.u8(p + 68).unwrap(),
                g.u8(p + 72).unwrap(),
                g.u8(p + 73).unwrap()
            ),
            (2, 2, 7)
        );
        assert_eq!(
            (g.u32(p + 20).unwrap(), g.u32(p + 52).unwrap()),
            (PLAYER_WORD, 0x1234)
        );
        assert_eq!(
            [
                f32_at(&g, p),
                f32_at(&g, p + 8),
                f32_at(&g, p + 36),
                f32_at(&g, p + 56)
            ],
            [800.0, 800.0, 1.0, 100.0]
        );

        assert_eq!(g.u32(gain).unwrap(), BASE_VTABLE);
        assert_eq!(
            [
                g.u32(gain + 8).unwrap(),
                g.u32(gain + 12).unwrap(),
                g.u32(gain + 16).unwrap(),
                g.u32(gain + 20).unwrap()
            ],
            [SYS, p, gain + 48, GAIN_CLASS]
        );
        assert_eq!((g.u8(gain + 41).unwrap(), g.u8(gain + 42).unwrap()), (0, 1));
        assert_eq!(g.u64(gain + 48).unwrap(), ROW_WORD);

        assert_eq!(g.u32(send).unwrap(), SEND_VTABLE);
        assert_eq!((g.u8(send + 41).unwrap(), g.u8(send + 42).unwrap()), (1, 2));
        assert_eq!(
            [g.u64(send + 48).unwrap(), g.u64(send + 56).unwrap()],
            [ROW_WORD + 1, ROW_WORD + 2]
        );
        assert_eq!(f32_at(&g, send + 112), 1.0);

        assert_eq!(
            [
                g.u32(RING).unwrap(),
                g.u32(RING + 4).unwrap(),
                g.u32(SYS + 204).unwrap()
            ],
            [INSTALL_COMMAND, p, 8]
        );
    }

    #[test]
    fn clip_and_peak_classes_have_their_own_sizes_and_peak_state() {
        let mut g = guest();
        assert_eq!(class_size(&g, CLIP_SIZE, DESCRIPTORS).unwrap(), 56);
        assert_eq!(class_size(&g, PEAK_SIZE, DESCRIPTORS).unwrap(), 240);
        class(&mut g, GAIN_CLASS, PEAK_SIZE, PEAK_CONSTRUCT, 0, 5, 0, 0);
        descriptor(&mut g, 0, 0, GAIN_CLASS, 6);
        let player = build_graph(&mut g, &mut heap(), &mut Libm, SYS, 0, 1, DESCRIPTORS).unwrap();
        let peak = g.u32(player + 80).unwrap();
        assert_eq!(g.u32(peak).unwrap(), BASE_VTABLE);
        assert!(
            g.span(peak + 72, 128)
                .unwrap()
                .iter()
                .all(|&byte| byte == 0)
        );
        assert_eq!(g.u32(peak + 200).unwrap(), 0);
        assert_eq!(f32_at(&g, player + 40), 1500.0);
        assert_eq!(f32_at(&g, peak + 32), 1500.0);
        assert_eq!(
            [
                f32_at(&g, peak + 224),
                f32_at(&g, peak + 228),
                f32_at(&g, peak + 232)
            ],
            [
                f32_at(&g, peak + 52),
                f32_at(&g, peak + 60),
                f32_at(&g, peak + 68)
            ]
        );
    }

    #[test]
    fn resample_and_iir2_add_their_costs_to_the_player() {
        let mut g = guest();
        g.fill(HEAP, 0xEE, 0x400).unwrap();
        class(
            &mut g,
            GAIN_CLASS,
            RESAMPLE_SIZE,
            RESAMPLE_CONSTRUCT,
            0,
            5,
            0,
            0,
        );
        class(&mut g, SEND_CLASS, IIR2_SIZE, IIR2_CONSTRUCT, 0, 5, 0, 0);
        descriptor(&mut g, 0, 0, GAIN_CLASS, 1);
        descriptor(&mut g, 1, 0, SEND_CLASS, 1);
        let p = build_graph(&mut g, &mut heap(), &mut Libm, SYS, 0, 2, DESCRIPTORS).unwrap();
        let (resample, iir) = (p + 112, p + 224);
        assert_eq!(f32_at(&g, p + 40), 456.0);
        assert_eq!(g.u16(resample + 76).unwrap(), 88);
        assert!(
            g.span(resample + 88, 24).unwrap().iter().all(|&b| b == 0),
            "one 24-byte row per channel"
        );
        assert_eq!(
            [
                f32_at(&g, resample + 28),
                f32_at(&g, resample + 32),
                f32_at(&g, resample + 60),
                f32_at(&g, resample + 64)
            ],
            [6.0, 6.0, -1.0, 48000.0]
        );
        assert_eq!(g.u8(resample + 81).unwrap(), 2);
        assert!(
            g.span(iir + 56, 128).unwrap().iter().all(|&b| b == 0),
            "+56 through +180"
        );
        assert_eq!(g.u8(iir + 184).unwrap(), 0xEE, "and not past it");
        assert_eq!(f32_at(&g, iir + 32), 450.0);
    }

    #[test]
    fn a_stereo_panner_takes_the_stereo_front_and_inverts_its_pairs() {
        let mut g = guest();
        class(
            &mut g,
            GAIN_CLASS,
            RECHANNEL_SIZE,
            RECHANNEL_CONSTRUCT,
            0,
            5,
            0,
            0,
        );
        class(&mut g, SEND_CLASS, PAN_SIZE, PAN_CONSTRUCT, 0, 5, 0, 0);
        descriptor(&mut g, 0, 0, GAIN_CLASS, 1);
        descriptor(&mut g, 1, 0, SEND_CLASS, 2);
        let p = build_graph(&mut g, &mut heap(), &mut Libm, SYS, 0, 2, DESCRIPTORS).unwrap();
        let pan = p + 160;
        assert_eq!(g.u32(p + 84).unwrap(), pan);
        assert_eq!(
            (g.u32(pan + 748).unwrap(), g.u32(pan + 752).unwrap()),
            (1, 2)
        );
        assert_eq!(
            f32_at(&g, pan + 744),
            1.0,
            "law 2 over one input: 1/sqrt(1)"
        );
        assert_eq!(
            [
                f32_at(&g, pan + 732),
                f32_at(&g, pan + 736),
                f32_at(&g, pan + 740)
            ],
            [90.0, 110.0, 150.0]
        );
        assert_eq!(
            [
                g.u32(pan + 104).unwrap(),
                g.u32(pan + 112).unwrap(),
                g.u32(pan + 120).unwrap()
            ],
            [PAN_ANGLE_WORD; 3]
        );
        let t = pan + 128;
        assert_eq!(
            [
                g.u32(t + 172).unwrap(),
                g.u32(t + 176).unwrap(),
                g.u32(t + 180).unwrap(),
                g.u32(t + 184).unwrap()
            ],
            [0, 1, 2, 3]
        );
        assert!(
            (f32_at(&g, t) - 0.0).abs() < 1e-6 && (f32_at(&g, t + 4) - 1.0).abs() < 1e-6,
            "cos, sin of 90 degrees"
        );
        assert_inverse(&g, t + 76, t, t + 8);
        assert_inverse(&g, t + 92, t + 16, t);
        assert_inverse(&g, t + 156, t + 8, t + 24);
    }

    #[test]
    fn surround_layouts_choose_their_rows_and_extra_inverses() {
        for n in [4u32, 5, 6, 8] {
            let mut g = guest();
            let t = MEM + 0x1000;
            speaker_tables(&mut g, &mut Libm, t, n, 30.0, 110.0, 150.0).unwrap();
            let first = if n > 4 { 2 } else { 1 };
            assert_eq!(g.u32(t + 176).unwrap(), first);
            let row = |slot: u32| t + 8 * slot;
            assert_inverse(&g, t + 76, row(0), row(first));
            assert_inverse(&g, t + 92, row(first + 1), row(0));
            assert_inverse(&g, t + 156, row(first), row(first + 2));
            if n == 4 || n == 6 {
                assert_inverse(&g, t + 124, row(first + 2), row(first + 1));
            }
            let centre = (f32_at(&g, t + 8), f32_at(&g, t + 12));
            if n >= 6 {
                assert_eq!(centre, (1.0, 0.0), "{n} channels");
            } else {
                assert_ne!(centre, (1.0, 0.0), "{n} channels: a row, not the centre");
            }
            if n == 8 {
                assert_inverse(&g, t + 108, t + 40, t + 24);
                assert_inverse(&g, t + 124, t + 48, t + 40);
                assert_inverse(&g, t + 140, t + 32, t + 48);
            } else {
                assert_eq!(g.u32(t + 40).unwrap(), 0, "{n} channels build no rear pair");
            }
        }
    }

    #[test]
    fn a_sound_player_allocates_its_buffer_and_takes_a_pool_node() {
        let mut g = guest();
        class(
            &mut g,
            OTHER_CLASS,
            SNDPLAYER_SIZE,
            SNDPLAYER_CONSTRUCT,
            0,
            0,
            0,
            1,
        );
        descriptor(&mut g, 0, ARG, OTHER_CLASS, 1);
        g.set_u32(ARG + 4, 2.0f32.to_bits()).unwrap();
        let mut heap = heap();
        let p = build_graph(&mut g, &mut heap, &mut Libm, SYS, 0, 1, DESCRIPTORS).unwrap();
        // Header 88 + 8, then 488 + 2*48.
        assert_eq!(g.u32(p + 64).unwrap(), 680);
        let player = p + 96;
        let buffer = p + 688;
        assert_eq!(g.u32(player).unwrap(), SNDPLAYER_VTABLE);
        assert_eq!(
            [
                g.u32(player + 440).unwrap(),
                g.u32(player + 444).unwrap(),
                g.u32(player + 96).unwrap()
            ],
            [buffer, buffer + 4, buffer + 8]
        );
        assert_eq!(
            (g.u8(player + 470).unwrap(), g.u8(player + 466).unwrap()),
            (2, 1)
        );
        assert_eq!(
            (g.u16(player + 462).unwrap(), g.u16(player + 464).unwrap()),
            (480, 488)
        );
        assert_eq!(
            [g.u32(player + 528).unwrap(), g.u32(player + 576).unwrap()],
            [255, 255]
        );
        assert_eq!(
            [f32_at(&g, player + 428), f32_at(&g, player + 456)],
            [48000.0, 48000.0]
        );

        let block = (buffer + 168 + 15) & !15; // the heap aligns to 16
        let node = block + 8 + 16 * 73;
        assert_eq!(heap.next, block + 74 * 16 + 8);
        assert_eq!(
            [
                g.u32(SYS + 172).unwrap(),
                g.u32(SYS + 168).unwrap(),
                g.u32(SYS + 160).unwrap()
            ],
            [74, 1, node]
        );
        assert_eq!(
            g.u32(SYS + 156).unwrap(),
            node - 16,
            "the free list's head moved on"
        );
        assert_eq!(
            [g.u32(player + 72).unwrap(), g.u32(node + 8).unwrap()],
            [node, player + 72]
        );
        assert_eq!(
            [
                g.u32(player + 76).unwrap(),
                g.u32(player + 80).unwrap(),
                g.u32(player + 84).unwrap()
            ],
            [NODE_CALLBACK, player, SNDPLAYER_NAME]
        );
        assert_eq!(
            [
                g.u32(player + 24).unwrap(),
                g.u8(player + 43).unwrap() as u32,
                g.u8(player + 476).unwrap() as u32
            ],
            [player + 72, 1, 1]
        );
    }

    #[test]
    fn submix_install_links_the_global_head_and_marks_the_instance() {
        let mut g = guest();
        let old = MEM + 0x1800;
        let submix = MEM + 0x1840;
        g.put(SUBMIX_HEAD, vec![0; 4]);
        g.set_u32(SUBMIX_HEAD, old).unwrap();
        g.set_u32(RING, SUBMIX_INSTALL_COMMAND).unwrap();
        g.set_u32(RING + 4, submix).unwrap();
        assert_eq!(install_submix(&mut g, RING).unwrap(), 8);
        assert_eq!(g.u32(SUBMIX_HEAD).unwrap(), submix + 44);
        assert_eq!(g.u32(submix + 44).unwrap(), old);
        assert_eq!(g.u32(old + 4).unwrap(), submix + 44);
        assert_eq!(g.u32(submix + 48).unwrap(), 0);
        assert_eq!(g.u8(submix + 164).unwrap(), 1);
    }

    #[test]
    fn players_are_installed_in_order_and_the_list_grows() {
        let mut g = guest();
        g.set_u32(SYS + 276, 2).unwrap();
        let mut heap = heap();
        for (i, (order, size)) in [(5u8, 11u32), (2, 22), (9, 33)].into_iter().enumerate() {
            let p = PLAYERS + 0x40 * i as u32;
            g.set_u32(p + 16, SYS).unwrap();
            g.set_u8(p + 73, order).unwrap();
            g.set_u32(p + 64, size).unwrap();
            g.set_u32(RING + 4, p).unwrap();
            assert_eq!(install_command(&mut g, &mut heap, RING).unwrap(), 8);
        }
        let list = g.u32(SYS + 108).unwrap();
        let entries: Vec<u32> = (0..6).map(|i| g.u32(list + 4 * i).unwrap()).collect();
        assert_eq!(
            entries,
            [PLAYERS + 0x40, 22, PLAYERS, 11, PLAYERS + 0x80, 33]
        );
        assert_eq!(
            (
                g.u16(SYS + 280).unwrap(),
                g.u16(SYS + 282).unwrap(),
                g.u32(SYS + 272).unwrap()
            ),
            (3, 34, 3)
        );
    }

    #[test]
    fn a_player_the_list_cannot_hold_is_parked_on_the_system_list() {
        let mut g = guest();
        let p = PLAYERS;
        g.set_u32(p + 16, SYS).unwrap();
        g.set_u32(SYS + 16, 0xABCD_0000).unwrap();
        g.set_u32(SYS + 276, 2).unwrap(); // a first growth of zero entries would allocate zero bytes, and succeed
        g.put(0xABCD_0000, vec![0; 8]);
        let mut full = BumpHeap {
            next: HEAP,
            end: HEAP,
        };
        assert!(!install_player(&mut g, &mut full, p).unwrap());
        assert_eq!((g.u8(p + 71).unwrap(), g.u32(p + 76).unwrap()), (2, 1));
        assert_eq!(
            [
                g.u32(p + 28).unwrap(),
                g.u32(p + 32).unwrap(),
                g.u32(SYS + 16).unwrap()
            ],
            [0xABCD_0000, 0, p + 28]
        );
        assert_eq!(g.u32(0xABCD_0004).unwrap(), p + 28);
    }

    #[test]
    fn retiring_a_free_head_block_keeps_the_next_pool_block() {
        let mut g = guest();
        let pool = MEM + 0x1a00;
        let first = MEM + 0x1c00;
        let second = MEM + 0x1d00;
        let node = first + 8;
        g.set_u32(pool, first).unwrap();
        g.set_u32(pool + 4, second).unwrap();
        g.set_u32(pool + 8, 2).unwrap();
        g.set_u32(pool + 12, node).unwrap();
        g.set_u32(pool + 24, 0).unwrap();
        g.set_u32(pool + 28, 1).unwrap();
        g.set_u32(first, second).unwrap();
        g.set_u32(first + 4, 1).unwrap();
        g.set_u32(node + 8, 0).unwrap();
        g.set_u32(second, 0).unwrap();

        assert_eq!(pool_retire_head(&mut g, &mut heap(), pool).unwrap(), 1);
        assert_eq!(g.u32(pool).unwrap(), second);
        assert_eq!(g.u32(pool + 4).unwrap(), second);
        assert_eq!(g.u32(pool + 8).unwrap(), 1);
        assert_eq!(g.u32(pool + 12).unwrap(), 0);
        assert_eq!(g.u32(pool + 28).unwrap(), 0);
    }
}
