//! The scatter-mixer and the bank gather above it: `sub_82B426D0` and `sub_82B468C0`.
//!
//! Ported from `recomp/src/audio_ports/sub_82B426D0.inc`, **STATUS: verified** — 83,885 calls in a
//! played session on `RwAudioCore Dac`, compared against the original under the shadow harness.
//!
//! Replayed against **1,100 recorded calls, 0 disagreements** — 500 for the mixer and 600 for the
//! gather — and compared live against the original 17,788 and 16,032 times. Recording it needed the harness's read-span cap raised from 32
//! to 192 — it declares 64 spans, and the excess used to be dropped silently, which made every
//! recorded vector unreplayable while the shadow comparison stayed green (`docs/port-loop.md`).
//!
//! This is the first port here that is mostly *composition*: every sample it moves is moved by
//! [`crate::dsp::scale`]'s two kernels, and every byte it clears is cleared by [`crate::mem::memset`],
//! all three of which are verified and replayed in their own right. So the risk that lives here is
//! the decoding and the bookkeeping, and that is what the tests below aim at.
//!
//! ## The route byte
//!
//! One byte per route, and it packs three fields — which a reader would not guess from the three
//! `rlwinm`s that extract them, since each produces a *byte offset* rather than an index:
//!
//! | bits | field | extracted as |
//! |---|---|---|
//! | 2:0 | destination slot | `(route & 7) << 2` |
//! | 5:3 | source slot | `(route >> 1) & 0x1C` |
//! | 7:6 | gain | `(route >> 4) & 0xC`, a word index into [`GAIN_TABLE`] |
//!
//! ## The bookkeeping, which is the whole function
//!
//! Eight flag bytes say whether a destination has been written yet in *this call*. The first route
//! to reach a destination **overwrites** it (`sub_82B3BED8`, `dst[i] = src[i] * gain`) and sets the
//! flag; every later route to the same destination **accumulates** (`sub_82B44B20`,
//! `dst[i] += src[i] * gain`). Then a closing loop zeroes every destination slot the caller owns
//! whose flag is still clear. That is what makes the mixer's output independent of what was left in
//! those buffers by the previous block.
//!
//! Three details are reproduced rather than tidied, each because the original does it:
//!
//! - **The range's last index is re-read from memory every iteration**, so a route that wrote over
//!   `range + 1` would change its own trip count. (The C++ window builder refuses such a call; here
//!   the read simply happens where the original has it.)
//! - **The route byte is re-read after the overwriting call**, and it is the *reloaded* value that
//!   picks which flag gets set.
//! - **The flags live in the function's own frame.** `sp+80..87`, cleared by eight `stbu`s, and no
//!   callee is ever given their address — so a local array is the same machine, and this port takes
//!   no `sp` argument. With more than eight slots the original reads frame bytes it never wrote,
//!   which is uninitialised stack; the C++ declines those calls and so does this ([`Error`]).

use crate::{Error, Guest, Result, dsp, mem};

/// `lis r11,-32241 ; addi r24,r11,-10560` — four floats at `0x820ED6C0`, indexed by the route's
/// top two bits. Computed from the immediates, not read off.
pub const GAIN_TABLE: u32 = (((-32241i32 as u32) & 0xFFFF) << 16).wrapping_sub(10560);
const _: () = assert!(GAIN_TABLE == 0x820E_D6C0, "lis -32241 ; addi -10560");

/// `lbz r31,0(r25)` — the first route index, inclusive.
pub const RANGE_FIRST: u32 = 0;
/// `lbz r11,1(r25)` — the last route index, inclusive, and re-read every iteration.
pub const RANGE_LAST: u32 = 1;
/// The eight frame flag bytes, and so the most destination slots a call may own.
pub const FLAG_COUNT: u32 = 8;

/// The destination slot a route names: bits 2:0.
pub fn dest_slot(route: u8) -> u32 {
    u32::from(route) & 7 // clrlwi r9,r11,29
}
/// The source slot: bits 5:3, extracted as a byte offset by `rlwinm r10,r11,31,27,29`.
pub fn source_slot(route: u8) -> u32 {
    ((u32::from(route) >> 1) & 0x1C) >> 2
}
/// The gain word: bits 7:6, extracted as a byte offset by `rlwinm r8,r11,28,4,29`.
pub fn gain_slot(route: u8) -> u32 {
    ((u32::from(route) >> 4) & 0xC) >> 2
}

/// Run the routing table, then clear every destination no route wrote (`sub_82B426D0`).
///
/// Arguments are the guest's `r3`…`r8`: the eight-entry destination pointer array, the eight-entry
/// source pointer array, how many destination slots the caller owns, the float count per buffer, the
/// two-byte inclusive route range, and the route table itself. The original returns nothing.
///
/// Refuses, before touching anything, a `slot_count` above eight — see the module note.
pub fn scatter_mix(
    g: &mut Guest,
    dest_array: u32,
    source_array: u32,
    slot_count: u32,
    count: u32,
    range: u32,
    table: u32,
) -> Result<()> {
    if slot_count > FLAG_COUNT {
        // The original would read flag bytes its eight `stbu`s never wrote — uninitialised frame.
        return Err(Error::new(
            range,
            "scatter_mix: more destination slots than the original's eight frame flags",
        ));
    }

    // li r11,8 ; addi r10,r1,79 ; stbu r9,1(r10) x8
    let mut written = [0u8; FLAG_COUNT as usize];

    // lbz r31,0(r25) ; lbz r11,1(r25) ; cmplw ; bgt -- a do-while, so first > last runs nothing.
    let mut index = u32::from(g.u8(range + RANGE_FIRST)?);
    while index <= u32::from(g.u8(range + RANGE_LAST)?) {
        let route = g.u8(table + index)?; // lbzx r11,r31,r30
        let flag = written[dest_slot(route) as usize]; // lbzx r7,r9,r10
        let gain = crate::fp::load_single(g, GAIN_TABLE + gain_slot(route) * 4)?; // lfsx f1,r8,r24
        let src = g.u32(source_array + source_slot(route) * 4)?; // lwzx r4,r10,r28
        let dst = g.u32(dest_array + dest_slot(route) * 4)?; // lwzx r3,r9,r29

        if flag != 0 {
            dsp::scale::scale_accumulate(g, dst, src, count, gain)?; // bl 0x82b44b20
        } else {
            dsp::scale::scale(g, dst, src, count, gain)?; // bl 0x82b3bed8
            // lbzx r7,r31,r30 -- the route byte is reloaded after the call, and it is the reloaded
            // value that picks the flag.
            let reloaded = g.u8(table + index)?;
            written[dest_slot(reloaded) as usize] = 1; // li r26,1 ; stbx r26,r6,r8
        }

        index += 1; // addi r31,r31,1
    }

    // loc_82B42788: every slot the caller owns whose flag is still clear is zeroed.
    for slot in 0..slot_count {
        if written[slot as usize] == 0 {
            let buffer = g.u32(dest_array + slot * 4)?; // lwz r3,0(r30)
            // rlwinm r5,r27,2,0,29 -- the byte length, truncated to a multiple of four in 32 bits.
            let bytes = u64::from((count << 2) & 0xFFFF_FFFC);
            mem::memset(g, buffer, 0, bytes)?; // bl 0x82ee5e80
        }
    }

    Ok(())
}

// ------------------------------------------------------------------- the bank gather above it

/// `lis r11,-32206 ; lfs f1,-22460(r11)` — the single `1.0` every non-table path scales by.
/// Loaded once per call and never reloaded, which the port reproduces.
pub const UNITY_GAIN: u32 = (((-32206i32 as u32) & 0xFFFF) << 16).wrapping_sub(22460);
/// `lis r10,-32241 ; addi r10,r10,-10496` — 64 two-byte inclusive `[first, last]` route ranges,
/// one per (source layout, destination layout) pair.
pub const RANGE_TABLE: u32 = (((-32241i32 as u32) & 0xFFFF) << 16).wrapping_sub(10496);
/// `lis r8,-32241 ; addi r8,r8,-10368` — the route bytes those ranges index.
pub const ROUTE_TABLE: u32 = (((-32241i32 as u32) & 0xFFFF) << 16).wrapping_sub(10368);

const _: () = assert!(UNITY_GAIN == 0x8231_A844, "lis -32206 ; lfs -22460");
const _: () = assert!(RANGE_TABLE == 0x820E_D700 && ROUTE_TABLE == 0x820E_D780);

/// The five channel counts the route tables describe: `cmplwi cr6,r5,1 / 2 / 4 / 6 / 8`.
pub const STANDARD_LAYOUTS: [u32; 5] = [1, 2, 4, 6, 8];

/// Whether a channel count is one of the five layouts the route tables cover.
pub fn is_standard_layout(count: u32) -> bool {
    STANDARD_LAYOUTS.contains(&count)
}

/// Gather one bank of float buffers into another (`sub_82B468C0`).
///
/// Three paths, and which one runs is decided entirely by the two channel counts:
///
/// 1. **Both counts are a standard layout** — look the pair up as `8*source + dest - 9` in
///    [`RANGE_TABLE`] and hand the whole job to [`scatter_mix`], which is the port above. The index
///    is 0..63 for the five layouts, so it always lands inside the 128-byte table.
/// 2. **At least as many sources as destinations** — copy buffer by buffer at unity gain through
///    [`crate::dsp::scale::scale`], one call per destination. Nothing is zeroed: every destination
///    is fed.
/// 3. **Fewer sources than destinations** — copy what there is, then `memset` the destinations the
///    sources ran out for.
///
/// `floats` is the guest's `r7` at full width; the kernels below it read only the low word.
///
/// **The 144-byte frame is not reproduced.** The C++ port does reproduce it, for a reason that does
/// not apply here: there, `sub_82B3BED8` spills a single below `r1` and `sub_82B426D0` opens its own
/// frame, and if `r1` sat 144 bytes higher during a replay those spills would land somewhere else
/// and read as a divergence. In this crate none of the three callees writes guest memory outside its
/// destination buffer — `dsp::scale` keeps its splat in registers, `scatter_mix` keeps its flags in a
/// local array — so there is nothing to place, and the back-chain store would only be a write into
/// memory no recording carries.
pub fn gather_bank(
    g: &mut Guest,
    dest_array: u32,
    source_array: u32,
    dest_count: u32,
    source_count: u32,
    floats: u64,
) -> Result<()> {
    if is_standard_layout(dest_count) && is_standard_layout(source_count) {
        // loc_82B469EC. Both rotates in the original wrap exactly the bits their masks clear, so
        // they are plain shifts: 8*source + dest - 9, doubled for the two-byte range entries.
        let index = ((source_count << 3) + dest_count) - 9;
        let range = RANGE_TABLE.wrapping_add(index << 1); // add r7,r11,r10
        return scatter_mix(
            g,
            dest_array,
            source_array,
            dest_count,
            floats as u32,
            range,
            ROUTE_TABLE,
        );
    }

    // The gain is loaded ONCE, before either loop, and the original never reloads it — so the loop
    // depends on `sub_82B3BED8` leaving `f1` alone. Reproduced rather than papered over.
    let gain = crate::fp::load_single(g, UNITY_GAIN)?; // lfs f1,-22460(r11)
    // subf r26,r28,r4 -- the source array is reached as a delta off the destination cursor, so the
    // sum wraps in 32 bits exactly as the original's `lwzx` does.
    let delta = source_array.wrapping_sub(dest_array);

    let scale_loop = |g: &mut Guest, trips: u32| -> Result<()> {
        let mut cursor = dest_array; // mr r31,r28
        for _ in 0..trips {
            let src = g.u32(delta.wrapping_add(cursor))?; // lwzx r4,r26,r31 -- B[i]
            let dst = g.u32(cursor)?; // lwz r3,0(r31) -- A[i]
            dsp::scale::scale(g, dst, src, floats as u32, gain)?; // bl 0x82b3bed8
            cursor = cursor.wrapping_add(4); // addi r31,r31,4
        }
        Ok(())
    };

    if source_count >= dest_count {
        // loc_82B469AC: more sources than destinations, so every destination is fed.
        if dest_count != 0 {
            scale_loop(g, dest_count)?;
        }
        return Ok(());
    }

    // Fewer sources than destinations.
    let mut fed = 0u32; // li r27,0
    if source_count != 0 {
        scale_loop(g, source_count)?;
        fed = source_count; // mr r27,r6
    }
    if fed < dest_count {
        // loc_82B4698C: one memset per destination the sources ran out for.
        let bytes = u64::from(((floats as u32) << 2) & 0xFFFF_FFFC); // rlwinm r26,r25,2,0,29
        // rlwinm r11,r27,2,0,29 ; add r11,r11,r28 ; addi r30,r11,-4 -- the `lwzu` pre-increments.
        let mut cursor = (((fed << 2) & 0xFFFF_FFFC).wrapping_add(dest_array)).wrapping_sub(4);
        for _ in 0..(dest_count - fed) {
            cursor = cursor.wrapping_add(4); // lwzu r3,4(r30)
            let buffer = g.u32(cursor)?;
            mem::memset(g, buffer, 0, bytes)?; // bl 0x82ee5e80
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------- the downmix program

/// `lis -32208 ; addi -31232 ; lfs 1024` — measured 1/65, the per-sample increment of the ramp.
pub const DOWNMIX_RAMP_RATE: u32 = (((-32208i32 as u32) & 0xFFFF) << 16).wrapping_sub(31232) + 1024;
const _: () = assert!(
    DOWNMIX_RAMP_RATE == 0x822F_8A00,
    "lis -32208 ; addi -31232 ; lfs 1024"
);

/// Run the (in-channels -> out-channels) downmix program (`sub_82B46810`): for each routing byte,
/// ramp-mix one source channel into one destination channel with [`dsp::gain_ramp::gain_ramp_accumulate`].
///
/// `dst_array` is `r3` and `src_array` `r4`, eight channel pointers each; `out_channels` is `r7` at
/// full width, `in_channels` `r8`, `target` `f1` and `current` `f2`. The route range comes from the
/// same tables [`scatter_mix`] uses, indexed `2 * (8*in + out - 9)`, and each routing byte decodes
/// exactly as there: destination in bits 2:0, source in 5:3, a gain index in 7:6.
///
/// Every route mixes from `scale * current` rising by `scale * (target - current) / 65` per sample —
/// and since the kernel ramps for 64 samples and then holds, **the ramp lands at 64/65 of the way to
/// the target, not on it**. That is the original's arithmetic, reproduced rather than corrected. The
/// last program index is re-read every iteration.
pub fn downmix(
    g: &mut Guest,
    dst_array: u32,
    src_array: u32,
    out_channels: u64,
    in_channels: u32,
    target: f64,
    current: f64,
) -> Result<()> {
    let mut fpscr = crate::vmx::Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    let delta = crate::fp::sub_single(target, current); // fsubs f13,f1,f2
    let ramp = crate::fp::mul_single(delta, crate::fp::load_single(g, DOWNMIX_RAMP_RATE)?); // fmuls f6

    // rlwinm r11,r8,3,0,28 ; add r11,r11,r7 ; addi r11,r11,-9 ; rlwinm r10,r11,1,0,30
    let row = u64::from((in_channels << 3) & 0xFFFF_FFF8);
    let entry = row.wrapping_add(out_channels).wrapping_sub(9);
    let bounds = RANGE_TABLE.wrapping_add((entry as u32).rotate_left(1) & 0xFFFF_FFFE);

    let mut index = u32::from(g.u8(bounds)?); // lbzx r31,r10,r11
    let last = u32::from(g.u8(bounds + 1)?); // lbz r6,1(r30)
    if index <= last {
        let mut cursor = ROUTE_TABLE.wrapping_add(index).wrapping_sub(1); // addi r29,r11,-1
        loop {
            cursor = cursor.wrapping_add(1); // lbzu r11,1(r29)
            let op = u32::from(g.u8(cursor)?);
            let gain_off = op.rotate_left(28) & 0x0FFF_FFFC; // rlwinm r10,r11,28,4,29
            let src_off = op.rotate_left(31) & 0x1C; // rlwinm r9,r11,31,27,29
            let dst_off = op.rotate_left(2) & 0x1C; // rlwinm r8,r11,2,27,29
            let scale = crate::fp::load_single(g, GAIN_TABLE.wrapping_add(gain_off))?; // lfsx f0
            let src = g.u32(src_array.wrapping_add(src_off))?; // lwzx r4,r9,r27
            let step = crate::fp::mul_single(scale, ramp); // fmuls f2,f0,f6
            let dst = g.u32(dst_array.wrapping_add(dst_off))?; // lwzx r3,r8,r28
            let start = crate::fp::mul_single(scale, current); // fmuls f1,f0,f7
            dsp::gain_ramp::gain_ramp_accumulate(g, dst, src, start, step)?; // bl 0x82b44d18
            index += 1;
            if index > u32::from(g.u8(bounds + 1)?) {
                break; // lbz r7,1(r30) ; cmplw ; ble -- the last index is reloaded
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const DEST_ARRAY: u32 = BASE + 0x40;
    const SOURCE_ARRAY: u32 = BASE + 0x80;
    const RANGE: u32 = BASE + 0xC0;
    const TABLE: u32 = BASE + 0xD0;
    const BUFFERS: u32 = BASE + 0x1000;
    /// One buffer per slot, well clear of the next.
    const STRIDE: u32 = 0x200;
    const COUNT: u32 = 8;
    const POISON: u32 = 0xDEAD_BEEF;

    /// Four distinguishable gains, so a wrong table index is a wrong answer rather than a tie.
    const GAINS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];

    /// Destination slots 0..4 at `BUFFERS`, source slots 0..4 after them.
    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x4000);
        let mut bytes = Vec::new();
        for gain in GAINS {
            bytes.extend_from_slice(&gain.to_bits().to_be_bytes());
        }
        g.put(GAIN_TABLE, bytes);
        for slot in 0..4u32 {
            let dst = BUFFERS + slot * STRIDE;
            let src = BUFFERS + (4 + slot) * STRIDE;
            g.set_u32(DEST_ARRAY + slot * 4, dst).unwrap();
            g.set_u32(SOURCE_ARRAY + slot * 4, src).unwrap();
            for i in 0..COUNT {
                g.set_u32(dst + i * 4, POISON).unwrap();
                // Source slot s holds s+1, s+2, ... so a swapped source is visible immediately.
                g.set_u32(src + i * 4, ((slot + 1) as f32 + i as f32).to_bits())
                    .unwrap();
            }
        }
        g
    }

    fn route(g: &mut Guest, bytes: &[u8]) {
        g.set_u8(RANGE + RANGE_FIRST, 0).unwrap();
        g.set_u8(RANGE + RANGE_LAST, (bytes.len() - 1) as u8)
            .unwrap();
        for (i, b) in bytes.iter().enumerate() {
            g.set_u8(TABLE + i as u32, *b).unwrap();
        }
    }

    fn run(g: &mut Guest, slots: u32) {
        scatter_mix(g, DEST_ARRAY, SOURCE_ARRAY, slots, COUNT, RANGE, TABLE).unwrap();
    }

    fn out(g: &Guest, slot: u32) -> Vec<f32> {
        (0..COUNT)
            .map(|i| g.f32(BUFFERS + slot * STRIDE + i * 4).unwrap())
            .collect()
    }

    fn source(slot: u32) -> Vec<f32> {
        (0..COUNT).map(|i| (slot + 1) as f32 + i as f32).collect()
    }

    /// `route byte = gain << 6 | source << 3 | dest`.
    fn byte(gain: u8, source: u8, dest: u8) -> u8 {
        (gain << 6) | (source << 3) | dest
    }

    // ------------------------------------------------------------------ the bank gather above it

    /// The three rodata tables sit within 320 bytes of each other, so one segment holds them all.
    const RODATA: u32 = GAIN_TABLE;
    const RODATA_BYTES: usize = 0x140;
    const G_DEST: u32 = BASE + 0x200;
    const G_SOURCE: u32 = BASE + 0x240;
    const G_BUFFERS: u32 = BASE + 0x2000;
    /// Distinguishable from the unity gain, so which path ran is visible in the output.
    const TABLE_GAIN: f32 = 3.0;

    /// Eight destination buffers then eight source buffers, `COUNT` floats each.
    fn gather_guest() -> Guest {
        let mut g = Guest::single(BASE, 0x8000);
        g.put(RODATA, vec![0u8; RODATA_BYTES]);
        g.put(UNITY_GAIN, 1.0f32.to_bits().to_be_bytes().to_vec());
        for i in 0..4u32 {
            g.set_u32(GAIN_TABLE + i * 4, TABLE_GAIN.to_bits()).unwrap();
        }
        for slot in 0..8u32 {
            let dst = G_BUFFERS + slot * STRIDE;
            let src = G_BUFFERS + (8 + slot) * STRIDE;
            g.set_u32(G_DEST + slot * 4, dst).unwrap();
            g.set_u32(G_SOURCE + slot * 4, src).unwrap();
            for i in 0..COUNT {
                g.set_u32(dst + i * 4, POISON).unwrap();
                g.set_u32(src + i * 4, ((slot + 1) as f32 + i as f32).to_bits())
                    .unwrap();
            }
        }
        g
    }

    fn g_out(g: &Guest, slot: u32) -> Vec<f32> {
        (0..COUNT)
            .map(|i| g.f32(G_BUFFERS + slot * STRIDE + i * 4).unwrap())
            .collect()
    }
    fn g_source(slot: u32) -> Vec<f32> {
        (0..COUNT).map(|i| (slot + 1) as f32 + i as f32).collect()
    }

    fn gather(g: &mut Guest, dest_count: u32, source_count: u32) {
        gather_bank(
            g,
            G_DEST,
            G_SOURCE,
            dest_count,
            source_count,
            u64::from(COUNT),
        )
        .unwrap();
    }

    #[test]
    fn the_layouts_are_the_five_the_ladder_tests() {
        for count in 0..12u32 {
            assert_eq!(
                is_standard_layout(count),
                [1, 2, 4, 6, 8].contains(&count),
                "{count}"
            );
        }
    }

    #[test]
    fn a_standard_pair_goes_through_the_route_table() {
        // (source 2, destination 2) indexes 8*2 + 2 - 9 = 9, so the range is two bytes at
        // RANGE_TABLE + 18. One route there, gain word 0, source slot 1, destination slot 0.
        let mut g = gather_guest();
        g.set_u8(RANGE_TABLE + 18, 5).unwrap(); // first
        g.set_u8(RANGE_TABLE + 19, 5).unwrap(); // last
        g.set_u8(ROUTE_TABLE + 5, byte(0, 1, 0)).unwrap();

        gather(&mut g, 2, 2);

        // Scaled by the *gain table*, not by unity: that is how we know which path ran.
        let want: Vec<f32> = g_source(1).iter().map(|v| v * TABLE_GAIN).collect();
        assert_eq!(g_out(&g, 0), want);
        // Destination 1 is one of the two slots the caller owns, and no route wrote it, so the
        // scatter-mixer's closing loop zeroed it.
        assert_eq!(g_out(&g, 1), vec![0.0f32; COUNT as usize]);
    }

    #[test]
    fn the_table_index_is_eight_sources_plus_destinations_minus_nine() {
        // Both ends of the 64-entry table: (1,1) is index 0 and (8,8) is index 63, the last entry.
        for (dest, source, index) in [(1u32, 1u32, 0u32), (8, 8, 63), (6, 4, 4 * 8 + 6 - 9)] {
            let mut g = gather_guest();
            g.set_u8(RANGE_TABLE + index * 2, 7).unwrap();
            g.set_u8(RANGE_TABLE + index * 2 + 1, 7).unwrap();
            g.set_u8(ROUTE_TABLE + 7, byte(0, 0, 0)).unwrap();

            gather(&mut g, dest, source);

            let want: Vec<f32> = g_source(0).iter().map(|v| v * TABLE_GAIN).collect();
            assert_eq!(
                g_out(&g, 0),
                want,
                "dest {dest}, source {source} -> index {index}"
            );
        }
    }

    #[test]
    fn more_sources_than_destinations_copies_at_unity_gain() {
        // 3 and 5 are not standard layouts, so this is the copy path. Every destination is fed and
        // nothing is zeroed.
        let mut g = gather_guest();
        gather(&mut g, 3, 5);
        for slot in 0..3u32 {
            assert_eq!(
                g_out(&g, slot),
                g_source(slot),
                "slot {slot} copied at unity gain"
            );
        }
    }

    #[test]
    fn fewer_sources_than_destinations_copies_what_there_is_and_zeroes_the_rest() {
        let mut g = gather_guest();
        gather(&mut g, 5, 3);
        for slot in 0..3u32 {
            assert_eq!(g_out(&g, slot), g_source(slot), "slot {slot} fed");
        }
        for slot in 3..5u32 {
            assert_eq!(
                g_out(&g, slot),
                vec![0.0f32; COUNT as usize],
                "slot {slot} zeroed"
            );
        }
        // And the sixth destination, which the caller does not own, keeps its poison.
        assert_eq!(g.u32(G_BUFFERS + 5 * STRIDE).unwrap(), POISON);
    }

    #[test]
    fn no_sources_at_all_zeroes_every_destination() {
        let mut g = gather_guest();
        gather(&mut g, 5, 0); // 0 sources, 5 destinations: the scale loop is skipped entirely
        for slot in 0..5u32 {
            assert_eq!(g_out(&g, slot), vec![0.0f32; COUNT as usize], "slot {slot}");
        }
    }

    #[test]
    fn no_destinations_does_nothing() {
        let mut g = gather_guest();
        gather(&mut g, 0, 3); // sources >= destinations, and the destination guard skips the loop
        assert_eq!(g.u32(G_BUFFERS).unwrap(), POISON);
    }

    #[test]
    fn the_unity_gain_is_read_from_the_image() {
        // The copy path's gain is a loaded single, not a literal 1.0: patch it and the copies scale.
        let mut g = gather_guest();
        g.put(UNITY_GAIN, 2.0f32.to_bits().to_be_bytes().to_vec());
        gather(&mut g, 2, 3); // destination 2 is standard but source 3 is not, so this is the copy
        let want: Vec<f32> = g_source(0).iter().map(|v| v * 2.0).collect();
        assert_eq!(g_out(&g, 0), want);
    }

    #[test]
    fn one_standard_count_is_not_enough_for_the_table_path() {
        // Destination 2 is a layout, source 3 is not, so the pair falls through to the copy path —
        // which is visible because the copy uses unity gain and the table path would use 3.0.
        let mut g = gather_guest();
        g.set_u8(RANGE_TABLE + 2 * ((3 << 3) + 2 - 9), 0).unwrap();
        g.set_u8(RANGE_TABLE + 2 * ((3 << 3) + 2 - 9) + 1, 0)
            .unwrap();
        g.set_u8(ROUTE_TABLE, byte(0, 0, 0)).unwrap();

        gather(&mut g, 2, 3);

        assert_eq!(
            g_out(&g, 0),
            g_source(0),
            "unity gain, so the copy path ran"
        );
        assert_eq!(g_out(&g, 1), g_source(1), "and both destinations were fed");
    }

    #[test]
    fn the_gather_table_addresses_come_from_the_lis_immediates() {
        assert_eq!(UNITY_GAIN, 0x8232_0000 - 22460);
        assert_eq!(RANGE_TABLE, 0x820F_0000 - 10496);
        assert_eq!(ROUTE_TABLE, 0x820F_0000 - 10368);
        assert_eq!(
            ROUTE_TABLE - RANGE_TABLE,
            128,
            "64 two-byte ranges, then the route bytes"
        );
        assert_eq!(RANGE_TABLE - GAIN_TABLE, 64);
    }

    #[test]
    fn the_byte_fields_decode_where_the_note_says() {
        // Every field, over every byte value — the three `rlwinm`s produce byte offsets, and getting
        // one shift wrong picks a neighbouring slot that still exists.
        for raw in 0..=255u8 {
            assert_eq!(dest_slot(raw), u32::from(raw) & 7, "dest of {raw:#04X}");
            assert_eq!(
                source_slot(raw),
                (u32::from(raw) >> 3) & 7,
                "source of {raw:#04X}"
            );
            assert_eq!(gain_slot(raw), u32::from(raw) >> 6, "gain of {raw:#04X}");
        }
        // And the packing this test file uses agrees with them.
        for (gain, src, dst) in [(0u8, 0u8, 0u8), (3, 7, 7), (1, 2, 3), (2, 5, 6)] {
            let raw = byte(gain, src, dst);
            assert_eq!(
                (gain_slot(raw), source_slot(raw), dest_slot(raw)),
                (u32::from(gain), u32::from(src), u32::from(dst))
            );
        }
    }

    #[test]
    fn one_route_scales_its_source_into_its_destination() {
        let mut g = guest();
        route(&mut g, &[byte(1, 2, 3)]); // gain 2.0, source slot 2, destination slot 3
        run(&mut g, 0); // no closing clear, so only the route's own write is visible

        let want: Vec<f32> = source(2).iter().map(|v| v * 2.0).collect();
        assert_eq!(out(&g, 3), want);
        // Nothing else moved.
        assert_eq!(g.u32(BUFFERS).unwrap(), POISON, "destination 0 untouched");
    }

    #[test]
    fn the_first_route_to_a_destination_overwrites_and_the_rest_accumulate() {
        // The whole point of the flag array. Three routes into slot 1 from three different sources:
        // the answer is the first one's product *replacing* the poison, then two accumulations.
        let mut g = guest();
        route(&mut g, &[byte(0, 0, 1), byte(1, 1, 1), byte(2, 2, 1)]);
        run(&mut g, 0);

        let want: Vec<f32> = (0..COUNT as usize)
            .map(|i| source(0)[i] * 1.0 + source(1)[i] * 2.0 + source(2)[i] * 4.0)
            .collect();
        assert_eq!(out(&g, 1), want);

        // If the first call accumulated instead of overwriting, the poison would still be in there.
        let poisoned = f32::from_bits(POISON);
        assert!(want.iter().all(|v| v.is_finite()));
        assert!(
            !out(&g, 1).iter().any(|v| *v == poisoned),
            "the first route must overwrite"
        );
    }

    #[test]
    fn every_untouched_slot_the_caller_owns_is_zeroed() {
        let mut g = guest();
        route(&mut g, &[byte(0, 0, 2)]); // only destination 2 is written
        run(&mut g, 4); // the caller owns four slots

        assert_eq!(out(&g, 2), source(0), "gain 1.0, so the copy is exact");
        for slot in [0u32, 1, 3] {
            assert_eq!(
                out(&g, slot),
                vec![0.0f32; COUNT as usize],
                "slot {slot} cleared"
            );
        }
    }

    #[test]
    fn a_slot_count_of_zero_clears_nothing() {
        let mut g = guest();
        route(&mut g, &[byte(0, 0, 0)]);
        run(&mut g, 0);
        // Slot 1 keeps its poison: the closing loop never ran.
        assert_eq!(g.u32(BUFFERS + STRIDE).unwrap(), POISON);
    }

    #[test]
    fn the_gain_comes_from_the_table_and_is_read_live() {
        for gain_index in 0..4u8 {
            let mut g = guest();
            route(&mut g, &[byte(gain_index, 0, 0)]);
            run(&mut g, 0);
            let want: Vec<f32> = source(0)
                .iter()
                .map(|v| v * GAINS[gain_index as usize])
                .collect();
            assert_eq!(out(&g, 0), want, "gain index {gain_index}");
        }
        // Patching the table moves the result, so the four floats are loaded rather than assumed.
        let mut g = guest();
        g.set_u32(GAIN_TABLE + 8, 100.0f32.to_bits()).unwrap();
        route(&mut g, &[byte(2, 0, 0)]);
        run(&mut g, 0);
        assert_eq!(
            out(&g, 0),
            source(0).iter().map(|v| v * 100.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_range_is_inclusive_and_an_empty_range_runs_no_route() {
        // last < first: the do-while's guard is checked before the first iteration, so nothing runs
        // — but the closing clear still does.
        let mut g = guest();
        route(&mut g, &[byte(0, 0, 0), byte(0, 1, 1)]);
        g.set_u8(RANGE + RANGE_FIRST, 1).unwrap();
        g.set_u8(RANGE + RANGE_LAST, 0).unwrap();
        run(&mut g, 2);
        assert_eq!(
            out(&g, 0),
            vec![0.0f32; COUNT as usize],
            "no route ran, so it was cleared"
        );
        assert_eq!(out(&g, 1), vec![0.0f32; COUNT as usize]);

        // And a range of [1, 1] runs exactly the second route.
        let mut g = guest();
        route(&mut g, &[byte(0, 0, 0), byte(0, 1, 1)]);
        g.set_u8(RANGE + RANGE_FIRST, 1).unwrap();
        g.set_u8(RANGE + RANGE_LAST, 1).unwrap();
        run(&mut g, 0);
        assert_eq!(out(&g, 1), source(1));
        assert_eq!(
            g.u32(BUFFERS).unwrap(),
            POISON,
            "the first route did not run"
        );
    }

    #[test]
    fn the_last_index_is_re_read_every_iteration() {
        // The original reloads `range + 1` each time round, so a route whose destination covers that
        // byte changes its own trip count. Reproduced, and this is what pins it: the destination of
        // route 0 is aimed at the range itself, and the value it writes there shortens the loop.
        let mut g = guest();
        g.set_u32(DEST_ARRAY, RANGE).unwrap(); // destination slot 0 *is* the range word
        route(&mut g, &[byte(0, 0, 0), byte(0, 1, 1)]);
        g.set_u8(RANGE + RANGE_FIRST, 0).unwrap();
        g.set_u8(RANGE + RANGE_LAST, 1).unwrap();
        // Source slot 0's first float is 1.0, whose big-endian bytes are 3F 80 00 00, so after the
        // first route `range[1]` is 0x80 -- larger, not smaller, so the loop would run on. Make the
        // source write zeroes instead, which sets range[1] = 0 and ends the loop after route 0.
        for i in 0..COUNT {
            g.set_u32(BUFFERS + 4 * STRIDE + i * 4, 0).unwrap();
        }

        run(&mut g, 0);

        assert_eq!(
            g.u8(RANGE + RANGE_LAST).unwrap(),
            0,
            "the range was overwritten"
        );
        // Route 1 never ran: destination slot 1 still holds its poison.
        assert_eq!(g.u32(BUFFERS + STRIDE).unwrap(), POISON);
    }

    #[test]
    fn more_slots_than_flags_is_refused_before_anything_is_written() {
        let mut g = guest();
        route(&mut g, &[byte(0, 0, 0)]);
        let e = scatter_mix(&mut g, DEST_ARRAY, SOURCE_ARRAY, 9, COUNT, RANGE, TABLE);
        assert!(
            e.is_err(),
            "nine slots reads a flag byte the original never wrote"
        );
        assert_eq!(g.u32(BUFFERS).unwrap(), POISON, "and nothing ran");
    }

    // -------------------------------------------------------------------- the downmix program

    const DM_DST_ARRAY: u32 = BASE + 0x300;
    const DM_SRC_ARRAY: u32 = BASE + 0x340;
    const DM_BUFFERS: u32 = BASE + 0x4000;

    /// The gain-ramp kernel's constant pool, mapped as its own tests map it, plus the routing tables.
    fn downmix_guest() -> Guest {
        use dsp::gain_ramp::{LANE2_SCALE, LANE3_SCALE, RAMP_SPAN, SCALE, SCALE_STEP, STEP_SCALE};
        let mut g = Guest::from_segments(vec![
            crate::Segment {
                base: BASE,
                bytes: vec![0u8; 0x10000],
            },
            crate::Segment {
                base: 0x8206_0000,
                bytes: vec![0u8; 0x4000],
            },
            crate::Segment {
                base: 0x8225_7000,
                bytes: vec![0u8; 0x1000],
            },
            crate::Segment {
                base: 0x820E_D000,
                bytes: vec![0u8; 0x1000],
            },
            crate::Segment {
                base: 0x8231_BA00,
                bytes: vec![0u8; 0x100],
            },
            crate::Segment {
                base: DOWNMIX_RAMP_RATE,
                bytes: (1.0f32 / 65.0).to_bits().to_be_bytes().to_vec(),
            },
        ]);
        g.set_u32(STEP_SCALE, 4.0f32.to_bits()).unwrap();
        g.set_u32(RAMP_SPAN, 64.0f32.to_bits()).unwrap();
        g.set_u32(LANE2_SCALE, 2.0f32.to_bits()).unwrap();
        g.set_u32(LANE3_SCALE, 3.0f32.to_bits()).unwrap();
        let splat = |g: &mut Guest, at: u32, v: f32| {
            for k in 0..4 {
                g.set_u32(at + 4 * k, v.to_bits()).unwrap();
            }
        };
        for group in 1..8u32 {
            splat(&mut g, SCALE[group as usize], group as f32);
        }
        splat(&mut g, SCALE_STEP, 8.0);
        for (i, v) in [1.0f32, 0.707, 0.5, 0.25].iter().enumerate() {
            g.set_u32(GAIN_TABLE + 4 * i as u32, v.to_bits()).unwrap();
        }
        for slot in 0..8u32 {
            let buffer = DM_BUFFERS + slot * 0x400;
            g.set_u32(DM_DST_ARRAY + 4 * slot, buffer).unwrap();
            g.set_u32(DM_SRC_ARRAY + 4 * slot, buffer + 8 * 0x400)
                .unwrap();
            for i in 0..256u32 {
                g.set_u32(buffer + 4 * i, 0.5f32.to_bits()).unwrap();
                g.set_u32(
                    buffer + 8 * 0x400 + 4 * i,
                    ((slot + 1) as f32 * 0.1).to_bits(),
                )
                .unwrap();
            }
        }
        g
    }

    #[test]
    fn each_routing_byte_ramp_mixes_its_source_into_its_destination() {
        // 2 in, 2 out: the range is at RANGE_TABLE + 2*(8*2 + 2 - 9) = +18. Two routes: source 0 into
        // destination 1 at gain index 2 (0.5), and source 1 into destination 0 at gain index 0.
        let mut g = downmix_guest();
        g.set_u8(RANGE_TABLE + 18, 3).unwrap();
        g.set_u8(RANGE_TABLE + 19, 4).unwrap();
        g.set_u8(ROUTE_TABLE + 3, byte(2, 0, 1)).unwrap();
        g.set_u8(ROUTE_TABLE + 4, byte(0, 1, 0)).unwrap();
        let (target, current) = (0.8f64, 0.2f64);

        // The reference: the two kernel calls, made directly with the gains the note derives.
        let mut h = g.clone();
        let ramp = crate::fp::mul_single(
            crate::fp::sub_single(target, current),
            f64::from(1.0f32 / 65.0),
        );
        for (dst, src, scale) in [(1u32, 0u32, 0.5f64), (0, 1, 1.0)] {
            let d = h.u32(DM_DST_ARRAY + 4 * dst).unwrap();
            let s = h.u32(DM_SRC_ARRAY + 4 * src).unwrap();
            dsp::gain_ramp::gain_ramp_accumulate(
                &mut h,
                d,
                s,
                crate::fp::mul_single(scale, current),
                crate::fp::mul_single(scale, ramp),
            )
            .unwrap();
        }

        downmix(&mut g, DM_DST_ARRAY, DM_SRC_ARRAY, 2, 2, target, current).unwrap();
        for slot in 0..2u32 {
            for i in [0u32, 1, 63, 64, 255] {
                let at = DM_BUFFERS + slot * 0x400 + 4 * i;
                assert_eq!(
                    g.u32(at).unwrap(),
                    h.u32(at).unwrap(),
                    "destination {slot}, sample {i}"
                );
            }
        }
        assert_ne!(
            g.u32(DM_BUFFERS).unwrap(),
            0.5f32.to_bits(),
            "destination 0 was mixed into"
        );
    }

    #[test]
    fn an_empty_program_range_writes_nothing() {
        let mut g = downmix_guest();
        g.set_u8(RANGE_TABLE + 18, 5).unwrap();
        g.set_u8(RANGE_TABLE + 19, 4).unwrap(); // first > last
        downmix(&mut g, DM_DST_ARRAY, DM_SRC_ARRAY, 2, 2, 0.8, 0.2).unwrap();
        assert_eq!(g.u32(DM_BUFFERS).unwrap(), 0.5f32.to_bits());
    }

    #[test]
    fn the_downmix_ramp_rate_is_the_lis_address() {
        assert_eq!(DOWNMIX_RAMP_RATE, 0x8230_0000 - 31232 + 1024);
    }
}
