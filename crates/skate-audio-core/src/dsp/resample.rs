//! `sub_82B43FB8` — the linearly interpolating resampler, walked by a 16.16 phase.
//!
//! `docs/ports.md`: **verified**, 404,358 calls a boot and 748,902 in a played session, zero
//! divergence and zero skipped. **Unit-tested against a verified reference** here — the crate
//! README's second kind of green; the Rust has no recorded vectors of its own.
//!
//! The shape is one line of arithmetic and three loops of plumbing:
//!
//! ```text
//! out[n] = a + (b - a) * (fraction * scale)     a = table[index], b = table[index + 1]
//! ```
//!
//! where `fraction` is the low 16 bits of a 32-bit phase, `index` advances by the phase's high 16
//! bits plus every carry out of the low half, and `scale` is a rodata single that is **1/65536 to
//! six digits only** — see [`FRACTION_SCALE_CELL`].
//!
//! ## The three loops, and why they are not one loop
//!
//! Eight samples per trip, then four, then one. They are not interchangeable, and the difference is
//! not speed:
//!
//! - The **eight** loop forms all eight phases *up front* from the phase as it stands at the top of
//!   the trip, adding a precomputed `step_to[k]` to each. Those offsets are built as
//!   `2*step` **truncated to 32 bits** plus `k*step` in 64 bits, so they are not `(k+2)*step` once
//!   the step is large enough to overflow bit 31 ([`step_offsets`]).
//! - The **four** loop chains instead: each phase is the previous 16-bit *fraction* plus the step,
//!   so the whole part is folded into the index as it is produced. Chaining and precomputing agree
//!   only while eight steps cannot overflow the 16-bit field.
//! - Sample 0 of each of the first two loops takes `clrldi r29,r11,32` — the running phase's whole
//!   low **word**, not its low 16 bits. On a fraction that has already been masked those agree; on
//!   one that has not, they do not, and the mask is where it is because the original put it there.
//!
//! ## Load and store order is reproduced exactly
//!
//! Twelve loads, three stores, four loads, five stores in the eight-sample loop; in the four-sample
//! loop one pair is read `+4` before `+0`. **The output buffer may alias the table** — that is what
//! makes the interleaving observable — so none of it is rearranged into the order a reader would
//! find tidy.
//!
//! ## Two things this port cannot be replayed on as things stand
//!
//! **The step arrives in `r8`, and the recorded vector format stops at `r7`.** Nothing else about
//! this function is unusual; until the recorder stores `r8`, a replay would have to invent the one
//! argument that determines every address the call reads. Inventing it would turn a gap in
//! coverage into a meaningless pass, so the right outcome is *unreplayable*, not a zero.
//!
//! **`r3` is an input here, not a result.** It is the output sample count. The port returns
//! nothing (`kReturnNone`) and a dispatcher that compared `r3` against the recorded return would be
//! comparing the count against itself.

use crate::vmx::Fpscr;
use crate::{Guest, Result, fp};

const LIS_82300000: u32 = ((-32208i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82300000 == 0x8230_0000, "lis r31,-32208");

/// `lis -32208 ; addi r31,r31,-31232` — the audio pool, shared with [`super::biquad::POOL`].
pub const POOL: u32 = LIS_82300000.wrapping_add(-31232i32 as u32);
const _: () = assert!(POOL == 0x822F_8600, "lis -32208 ; addi -31232");

/// `lfs f0,428(r31)` — the fraction-to-weight scale, read **live**.
///
/// The cell holds `0x377FFC9C`, which is the float literal `1.5258e-5`: that is `1/65536` to six
/// digits **only** — `1/65536` is exactly `0x37800000`, and these are not the same number. The body
/// loads the cell rather than dividing by 65536, and nothing here assumes a value; a port that
/// "simplified" this to `fraction / 65536.0` would be off by about one part in 2^21 on every
/// interpolated sample.
pub const FRACTION_SCALE_CELL: u32 = POOL + 428;
const _: () = assert!(
    FRACTION_SCALE_CELL == 0x822F_87AC,
    "lis -32208 ; addi -31232 ; lfs 428"
);

/// The 16.16 phase, split.
#[derive(Clone, Copy, Debug, Default)]
struct Phase {
    /// `clrlwi rN,rX,16` — the low 16 bits, the interpolation weight.
    fraction: u64,
    /// `rlwinm rN,rX,16,16,31` — the next 16, whole input samples to advance by.
    advance: u64,
}

fn split(value: u64) -> Phase {
    Phase {
        fraction: (value as u32 & 0xFFFF) as u64,
        advance: (((value as u32) >> 16) & 0xFFFF) as u64,
    }
}

/// `rlwinm rN,rIndex,2,0,29 ; add rN,rN,r4` — the sample index scaled to a float offset and added
/// to the table, all in 32 bits.
fn sample_address(index: u64, table: u32) -> u32 {
    (((index as u32) << 2) & 0xFFFF_FFFC).wrapping_add(table)
}

/// `std rN,-k(r1) ; lfd fM,-k(r1) ; fcfid ; frsp` — the fraction as a single, held in a double.
///
/// The spill goes through the function's own frame and is a bit copy, so it is not reproduced. The
/// `frsp` is: a fraction above 2^24 would lose low bits here, and dropping the narrowing would
/// disagree with the guest on a phase that has not been masked.
///
/// The conversion is **signed** (`fcfid` on the spilled 64-bit value), and the values reaching it
/// are either a 16-bit mask or a zero-extended low word, so the sign bit is never set.
fn fraction_to_single(fraction: u64) -> f64 {
    (((fraction as i64) as f64) as f32) as f64
}

/// One output sample: the pair difference, the fraction scaled into a weight, one `fmadds`.
///
/// Operand order follows the lifted lines exactly — `fmuls fN,fraction,f0` and
/// `fmadds fN,difference,weight,a` — because neither multiply is NaN-commutative
/// (`docs/vmx128-exactness.md` rule 4).
fn interpolate(a: f64, b: f64, fraction: u64, scale: f64) -> f64 {
    let weight = fp::mul_single(fraction_to_single(fraction), scale);
    fp::fmadd_single(fp::sub_single(b, a), weight, a)
}

/// The eight per-sample phase offsets of the unrolled loop, and the whole group's advance.
///
/// `rlwinm r21,r8,1,0,30` truncates `2*step` to 32 bits; `r20..r15` then add the step in **64**
/// bits. So offset `k` is `(2*step mod 2^32) + (k-2)*step` for `k >= 2`, which is *not*
/// `k*step` once the step is large enough to carry out of bit 31. Kept literal.
///
/// Returned separately from the body so the truncation has somewhere to be tested.
fn step_offsets(step: u64) -> ([u64; 8], u64) {
    let twice = (((step as u32) << 1) & 0xFFFF_FFFE) as u64; // rlwinm r21,r8,1,0,30
    let offsets = [
        0,                                        // sample 0 uses the running phase unchanged
        step,                                     // r8
        twice,                                    // r21
        twice.wrapping_add(step),                 // r20
        twice.wrapping_add(step.wrapping_mul(2)), // r19
        twice.wrapping_add(step.wrapping_mul(3)), // r18
        twice.wrapping_add(step.wrapping_mul(4)), // r17
        twice.wrapping_add(step.wrapping_mul(5)), // r16
    ];
    (offsets, twice.wrapping_add(step.wrapping_mul(6))) // r15: the whole group of eight
}

/// `sub_82B43FB8` — write `count` interpolated singles into `output`, walking `table` by a 16.16
/// phase, and publish the cursor and the fraction back through their slots.
///
/// Arguments, by register: `count` is `r3` (an **input**, not a result), `table` is `r4` (already
/// biased by the caller), `output` is `r5`, `index_slot` is `r6` (a `u32` whole-sample cursor, read
/// and written), `phase_slot` is `r7` (a 16-bit fraction living in the word's **high** half, read
/// as a halfword and written as a full word) and `step` is `r8`, the 16.16 increment.
///
/// `step` is the guest's full 64-bit `r8`: the group offsets add it in 64 bits and only the
/// `2*step` term truncates, so a chain kept at 32 bits would produce the right memory and the wrong
/// addresses on a large step.
///
/// **Writes** `[output, output + 4*(count & 0x3FFFFFFF))`, the 4-byte cursor at `index_slot` and
/// the 4-byte phase word at `phase_slot` — the last two **on every path, `count == 0` included**.
/// Nothing else. Reads [`FRACTION_SCALE_CELL`] and the table span the walk reaches.
///
/// The phase word is written as four bytes over what the entry read treats as a 16-bit field:
/// `rlwinm r11,r11,16,0,15` puts the fraction in the high half and zeroes the low half. That is
/// exact for any value, not only a 16-bit one, because the rotate's low half lands outside the
/// mask.
pub fn resample(
    g: &mut Guest,
    count: u32,
    table: u32,
    output: u32,
    index_slot: u32,
    phase_slot: u32,
    step: u64,
) -> Result<()> {
    let (step_to, step_group) = step_offsets(step);

    let mut fraction = g.u16(phase_slot)? as u64; // lhz r11,0(r7)
    let mut index = g.u32(index_slot)? as u64; // lwz r10,0(r6)
    let mut out = output; // r5: the output cursor

    // rlwinm r9,r3,2,14,26 ; add r9,r9,r5 — only bits 3..15 of the count survive that mask, so this
    // is the count rounded down to a multiple of eight.
    let group_end = out.wrapping_add((count << 2) & 0x3_FFE0);
    // rlwinm r3,r3,2,0,29 ; add r14,r3,r5 — the end of the whole buffer.
    let end = out.wrapping_add((count << 2) & 0xFFFF_FFFC);

    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // stfd f29,-176(r1)
    let scale = fp::load_single(g, FRACTION_SCALE_CELL)?; // lfs f0,428(r31)

    // ------------------------------------------------------------- eight samples, 32 bytes a trip
    if out < group_end {
        // subf r9,r5,r9 ; addi r9,r9,-1 ; rlwinm r9,r9,27,5,31 ; addi r9,r9,1 ; mtctr r9
        let mut trips = ((group_end.wrapping_sub(out).wrapping_sub(1)) >> 5) + 1;
        loop {
            let mut fraction_of = [0u64; 8];
            let mut address = [0u32; 8];
            // Sample 0 takes the running fraction whole (clrldi r29,r11,32 — the low *word*, not
            // the 16-bit mask) and the running index; samples 1..7 add their group offset. All
            // eight are formed from the index as it stands at the top of the trip.
            fraction_of[0] = fraction & 0xFFFF_FFFF;
            address[0] = sample_address(index, table);
            for k in 1..8 {
                let phase = split(fraction.wrapping_add(step_to[k]));
                fraction_of[k] = phase.fraction;
                address[k] = sample_address(phase.advance.wrapping_add(index), table);
            }

            fpscr.disable_flush_mode_unconditional();
            // Twelve loads, three stores, four loads, five stores: the original's interleaving,
            // kept because the output buffer may alias the table.
            let s0a = fp::load_single(g, address[0])?; // lfs f13,0(r9)
            let s0b = fp::load_single(g, address[0].wrapping_add(4))?; // lfs f12,4(r9)
            let s1a = fp::load_single(g, address[1])?;
            let s1b = fp::load_single(g, address[1].wrapping_add(4))?;
            let s2a = fp::load_single(g, address[2])?;
            let s2b = fp::load_single(g, address[2].wrapping_add(4))?;
            let s3a = fp::load_single(g, address[3])?;
            let s3b = fp::load_single(g, address[3].wrapping_add(4))?;
            let s4a = fp::load_single(g, address[4])?;
            let s5a = fp::load_single(g, address[5])?;
            let s6a = fp::load_single(g, address[6])?;
            let s7a = fp::load_single(g, address[7])?;

            let v = interpolate(s1a, s1b, fraction_of[1], scale);
            fp::store_single(g, out.wrapping_add(4), v)?; // stfs f12,4(r5)
            let v = interpolate(s2a, s2b, fraction_of[2], scale);
            fp::store_single(g, out.wrapping_add(8), v)?; // stfs f11,8(r5)
            let v = interpolate(s0a, s0b, fraction_of[0], scale);
            fp::store_single(g, out, v)?; // stfs f10,0(r5)

            let s4b = fp::load_single(g, address[4].wrapping_add(4))?;
            let s5b = fp::load_single(g, address[5].wrapping_add(4))?;
            let s6b = fp::load_single(g, address[6].wrapping_add(4))?;
            let s7b = fp::load_single(g, address[7].wrapping_add(4))?;

            let v = interpolate(s3a, s3b, fraction_of[3], scale);
            fp::store_single(g, out.wrapping_add(12), v)?; // stfs f5,12(r5)
            let v = interpolate(s6a, s6b, fraction_of[6], scale);
            fp::store_single(g, out.wrapping_add(24), v)?; // stfs f11,24(r5)
            let v = interpolate(s4a, s4b, fraction_of[4], scale);
            fp::store_single(g, out.wrapping_add(16), v)?; // stfs f8,16(r5)
            let v = interpolate(s5a, s5b, fraction_of[5], scale);
            fp::store_single(g, out.wrapping_add(20), v)?; // stfs f9,20(r5)
            let v = interpolate(s7a, s7b, fraction_of[7], scale);
            fp::store_single(g, out.wrapping_add(28), v)?; // stfs f7,28(r5)

            // add r11,r15,r11 ; rlwinm r9,r11,16,16,31 ; clrlwi r11,r11,16 ; add r10,r9,r10
            let group = split(fraction.wrapping_add(step_group));
            index = index.wrapping_add(group.advance);
            fraction = group.fraction;
            out = out.wrapping_add(32); // addi r5,r5,32
            trips -= 1;
            if trips == 0 {
                break; // bdnz 0x82b44028
            }
        }
    }

    // loc_82B44234
    if out < end {
        // subf r9,r5,r14 ; addi r9,r9,3 ; srawi r3,r9,2 ; addze r9,r3 — ceil((end-out)/4) samples
        // left, computed on the low word with the carry `srawi` sets.
        let biased = end.wrapping_sub(out).wrapping_add(3);
        let carry = (biased as i32) < 0 && (biased & 3) != 0;
        let remaining = ((biased as i32) >> 2) + if carry { 1 } else { 0 };

        // ------------------------------------------------------- four samples, 16 bytes a trip
        if remaining >= 4 {
            // subf ; addi r9,r9,-13 ; rlwinm r9,r9,28,4,31 ; addi r9,r9,1 ; mtctr r9
            let mut trips = ((end.wrapping_sub(out).wrapping_sub(13)) >> 4) + 1;
            loop {
                // Chained here rather than precomputed: each phase is the previous 16-bit fraction
                // plus the step, and each whole part lands on the index as it is produced.
                let fraction0 = fraction & 0xFFFF_FFFF; // clrldi r9,r11,32
                let index0 = index;
                let p1 = split(fraction.wrapping_add(step)); // add r11,r11,r8
                let index1 = index0.wrapping_add(p1.advance);
                let p2 = split(p1.fraction.wrapping_add(step));
                let index2 = index1.wrapping_add(p2.advance);
                let p3 = split(p2.fraction.wrapping_add(step));
                let index3 = index2.wrapping_add(p3.advance);
                let p4 = split(p3.fraction.wrapping_add(step));
                index = index3.wrapping_add(p4.advance); // the next trip's index
                fraction = p4.fraction; // clrlwi r11,r11,16

                let address0 = sample_address(index0, table);
                let address1 = sample_address(index1, table);
                let address2 = sample_address(index2, table);
                let address3 = sample_address(index3, table);

                fpscr.disable_flush_mode_unconditional();
                let s0a = fp::load_single(g, address0)?;
                let s0b = fp::load_single(g, address0.wrapping_add(4))?;
                let s1a = fp::load_single(g, address1)?;
                let s1b = fp::load_single(g, address1.wrapping_add(4))?;
                let s2b = fp::load_single(g, address2.wrapping_add(4))?; // +4 before +0 here
                let s2a = fp::load_single(g, address2)?;
                let s3a = fp::load_single(g, address3)?;
                let s3b = fp::load_single(g, address3.wrapping_add(4))?;

                let v = interpolate(s3a, s3b, p3.fraction, scale);
                fp::store_single(g, out.wrapping_add(12), v)?; // stfs f6,12(r5)
                let v = interpolate(s1a, s1b, p1.fraction, scale);
                fp::store_single(g, out.wrapping_add(4), v)?; // stfs f5,4(r5)
                let v = interpolate(s0a, s0b, fraction0, scale);
                fp::store_single(g, out, v)?; // stfs f3,0(r5)
                let v = interpolate(s2a, s2b, p2.fraction, scale);
                fp::store_single(g, out.wrapping_add(8), v)?; // stfs f2,8(r5)
                out = out.wrapping_add(16); // addi r5,r5,16
                trips -= 1;
                if trips == 0 {
                    break; // bdnz 0x82b44268
                }
            }
        }

        // ------------------------------------------------------------ loc_82B44374: one at a time
        if out < end {
            // subf ; addi r9,r9,-1 ; rlwinm r9,r9,30,2,31 ; addi r9,r9,1 ; mtctr r9
            let mut trips = ((end.wrapping_sub(out).wrapping_sub(1)) >> 2) + 1;
            let mut cursor = out.wrapping_sub(4); // addi r5,r5,-4, because stfsu adds 4 first
            loop {
                fpscr.disable_flush_mode_unconditional();
                let address = sample_address(index, table);
                let a = fp::load_single(g, address)?; // lfs f10,0(r9)
                let b = fp::load_single(g, address.wrapping_add(4))?; // lfs f9,4(r9)
                let sample = interpolate(a, b, fraction & 0xFFFF_FFFF, scale); // clrldi r3,r11,32
                let advanced = split(fraction.wrapping_add(step)); // add r11,r11,r8
                index = index.wrapping_add(advanced.advance);
                fraction = advanced.fraction;
                cursor = cursor.wrapping_add(4); // stfsu f6,4(r5): ea = 4 + r5
                fp::store_single(g, cursor, sample)?;
                trips -= 1;
                if trips == 0 {
                    break; // bdnz 0x82b44394
                }
            }
            // r5 is left holding the last store address and is not read again.
        }
    }

    // loc_82B443DC — publish the cursor and the fraction.
    let published = ((fraction as u32) << 16) & 0xFFFF_0000; // rlwinm r11,r11,16,0,15
    g.set_u32(index_slot, index as u32)?; // stw r10,0(r6)
    g.set_u32(phase_slot, published)?; // stw r11,0(r7) — 4 bytes over a 16-bit field
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const TABLE: u32 = BASE;
    const OUT: u32 = BASE + 0x800;
    const INDEX: u32 = BASE + 0xC00;
    const PHASE: u32 = BASE + 0xC04;

    /// `0x377FFC9C` — the value the image dump holds. Used as the *input* to the tests, never as
    /// an assumption inside the port.
    const SCALE_BITS: u32 = 0x377F_FC9C;

    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x1000);
        g.put(FRACTION_SCALE_CELL, SCALE_BITS.to_be_bytes().to_vec());
        g
    }

    /// A table of 256 singles, `table[i] == i`, so an interpolated sample reads as a position.
    fn table(g: &mut Guest) {
        for i in 0..256u32 {
            g.set_u32(TABLE + 4 * i, (i as f32).to_bits()).unwrap();
        }
    }

    fn start(g: &mut Guest, index: u32, fraction: u16) {
        g.set_u32(INDEX, index).unwrap();
        g.set_u32(PHASE, (fraction as u32) << 16).unwrap();
    }

    fn out(g: &Guest, n: usize) -> Vec<f32> {
        (0..n).map(|i| g.f32(OUT + 4 * i as u32).unwrap()).collect()
    }

    /// The scale as the port sees it, for models that have to agree with it bit for bit.
    fn scale() -> f64 {
        f32::from_bits(SCALE_BITS) as f64
    }

    /// An independent per-sample model: walk the phase, interpolate, one sample at a time. Written
    /// from what the function is *for*, not from its three loops — which is the point, since the
    /// three loops form their phases in three different ways.
    fn model(count: u32, index0: u32, fraction0: u16, step: u64) -> (Vec<f32>, u32, u16) {
        let mut index = index0 as u64;
        let mut fraction = fraction0 as u64;
        let mut out = Vec::new();
        for _ in 0..count {
            let a = index as f64; // table[i] == i
            let b = (index + 1) as f64;
            out.push(interpolate(a, b, fraction, scale()) as f32);
            let p = split(fraction.wrapping_add(step));
            index = index.wrapping_add(p.advance);
            fraction = p.fraction;
        }
        (out, index as u32, fraction as u16)
    }

    #[test]
    fn a_unit_step_walks_the_table_one_sample_at_a_time() {
        let mut g = guest();
        table(&mut g);
        start(&mut g, 4, 0);
        resample(&mut g, 8, TABLE, OUT, INDEX, PHASE, 0x0001_0000).unwrap();
        // A zero fraction means every output is exactly table[index].
        assert_eq!(out(&g, 8), (4..12).map(|i| i as f32).collect::<Vec<_>>());
        assert_eq!(g.u32(INDEX).unwrap(), 12);
        assert_eq!(g.u32(PHASE).unwrap(), 0);
    }

    #[test]
    fn all_three_loops_agree_with_the_one_sample_model() {
        // 8 exercises the group loop alone; 4 and 7 the four-and-one loops; 12 the group loop then
        // the four loop; 13 and 19 all three. A mistranscribed trip count or a wrong phase chain
        // shows up as a disagreement at exactly one of these lengths.
        for count in [1u32, 2, 3, 4, 5, 7, 8, 9, 11, 12, 13, 16, 19, 24, 31] {
            for step in [0x0000_8000u64, 0x0001_0000, 0x0001_5555, 0x0000_0001] {
                let mut g = guest();
                table(&mut g);
                start(&mut g, 3, 0x1234);
                resample(&mut g, count, TABLE, OUT, INDEX, PHASE, step).unwrap();

                let (expected, index, fraction) = model(count, 3, 0x1234, step);
                assert_eq!(
                    out(&g, count as usize),
                    expected,
                    "count {count}, step {step:#x}"
                );
                assert_eq!(
                    g.u32(INDEX).unwrap(),
                    index,
                    "count {count}, step {step:#x}: cursor"
                );
                assert_eq!(
                    g.u32(PHASE).unwrap(),
                    (fraction as u32) << 16,
                    "count {count}, step {step:#x}: phase"
                );
            }
        }
    }

    #[test]
    fn the_fraction_actually_interpolates_between_the_pair() {
        // Half way between table[2] == 2 and table[3] == 3, with the rodata scale. Not 2.5: the
        // cell is 1/65536 to six digits, so a half-step weight is 0x8000 * 0x377FFC9C, a hair under
        // a half. That discrepancy is the whole reason the constant is read live.
        let mut g = guest();
        table(&mut g);
        start(&mut g, 2, 0x8000);
        resample(&mut g, 1, TABLE, OUT, INDEX, PHASE, 0).unwrap();
        let got = g.f32(OUT).unwrap();
        assert!((got - 2.5).abs() < 1e-4, "got {got}");
        // Measured: 2.4999743, about one part in 2^15 short of a clean half. That is the cell's
        // own error and it is the reason the constant is read rather than divided by 65536.
        assert_ne!(got, 2.5, "the scale is not exactly 1/65536");
        assert_eq!(got, interpolate(2.0, 3.0, 0x8000, scale()) as f32);
    }

    #[test]
    fn the_scale_is_read_live_and_not_folded_in() {
        // Patch the cell and the answer moves. A port that had hard-coded 1/65536, or that divided
        // by 65536, would return the same number both times.
        let mut g = guest();
        table(&mut g);
        g.set_u32(FRACTION_SCALE_CELL, 0.0f32.to_bits()).unwrap();
        start(&mut g, 2, 0xFFFF);
        resample(&mut g, 1, TABLE, OUT, INDEX, PHASE, 0).unwrap();
        assert_eq!(
            g.f32(OUT).unwrap(),
            2.0,
            "a zero scale collapses onto the lower neighbour"
        );

        let mut h = guest();
        table(&mut h);
        h.set_u32(FRACTION_SCALE_CELL, (1.0f32 / 65536.0).to_bits())
            .unwrap();
        start(&mut h, 2, 0x8000);
        resample(&mut h, 1, TABLE, OUT, INDEX, PHASE, 0).unwrap();
        assert_eq!(
            h.f32(OUT).unwrap(),
            2.5,
            "the exact 1/65536 *does* give a clean half"
        );
    }

    #[test]
    fn the_index_advances_by_the_steps_whole_part_and_by_every_carry() {
        // A step of 2.5 samples: the index gains 2 or 3 alternately, which only happens if the
        // carry out of the 16-bit fraction is folded in. A body that used only the high half would
        // advance by exactly 2 every time.
        let mut g = guest();
        table(&mut g);
        start(&mut g, 0, 0);
        resample(&mut g, 8, TABLE, OUT, INDEX, PHASE, 0x0002_8000).unwrap();
        assert_eq!(g.u32(INDEX).unwrap(), 20, "8 * 2.5");
        assert_eq!(g.u32(PHASE).unwrap(), 0);

        // And the per-sample positions alternate .0 / .5 rather than landing on whole samples.
        let (expected, _, _) = model(8, 0, 0, 0x0002_8000);
        assert_eq!(out(&g, 8), expected);
        assert_ne!(expected[1], expected[1].round(), "sample 1 lands mid-pair");
    }

    #[test]
    fn a_zero_count_still_publishes_the_cursor_and_the_phase() {
        // `stw r10,0(r6)` and `stw r11,0(r7)` are past every loop and past every early exit. And
        // the phase store is four bytes over what the entry read treats as a halfword, so the low
        // half is cleared even though nothing else happened.
        let mut g = guest();
        table(&mut g);
        g.set_u32(INDEX, 77).unwrap();
        g.set_u32(PHASE, 0xABCD_EF01).unwrap();
        g.set_u32(OUT, 0x7F7F_7F7F).unwrap();

        resample(&mut g, 0, TABLE, OUT, INDEX, PHASE, 0x0001_0000).unwrap();

        assert_eq!(g.u32(INDEX).unwrap(), 77, "unchanged, but written");
        assert_eq!(
            g.u32(PHASE).unwrap(),
            0xABCD_0000,
            "the low half is zeroed by the store"
        );
        assert_eq!(
            g.u32(OUT).unwrap(),
            0x7F7F_7F7F,
            "and nothing was written to the output"
        );
    }

    #[test]
    fn the_entry_phase_is_read_as_a_halfword_from_the_high_half_of_the_word() {
        // `lhz r11,0(r7)` on a big-endian word takes the *top* two bytes. Reading the low half, or
        // reading the whole word, would start the walk at a different weight entirely.
        let mut g = guest();
        table(&mut g);
        g.set_u32(INDEX, 5).unwrap();
        g.set_u32(PHASE, 0x8000_FFFF).unwrap(); // fraction 0x8000, junk in the low half
        resample(&mut g, 1, TABLE, OUT, INDEX, PHASE, 0).unwrap();
        assert_eq!(
            g.f32(OUT).unwrap(),
            interpolate(5.0, 6.0, 0x8000, scale()) as f32
        );
    }

    #[test]
    fn the_group_offsets_truncate_twice_the_step_and_then_add_in_sixty_four_bits() {
        // `rlwinm r21,r8,1,0,30` is a 32-bit doubling with the low bit cleared; r20..r15 then add
        // the step in 64 bits. For a step whose double overflows bit 31 the offsets are therefore
        // *not* k*step, and that is the difference a "tidied" `(k as u64) * step` would erase.
        let step = 0x9000_0000u64;
        let (offsets, group) = step_offsets(step);
        assert_eq!(offsets[1], step);
        assert_eq!(
            offsets[2], 0x2000_0000,
            "2*step truncated to 32 bits, not 0x120000000"
        );
        assert_eq!(offsets[3], 0x2000_0000 + step);
        assert_eq!(offsets[7], 0x2000_0000 + 5 * step);
        assert_eq!(group, 0x2000_0000 + 6 * step);
        assert_ne!(offsets[2], 2 * step, "the truncation has to be visible");

        // The `rlwinm`'s mask (bits 0..30, PPC numbering — everything but the LSB) clears the bit
        // the rotate wrapped in from the MSB, which a left *shift* has already dropped. So the two
        // forms agree and the mask is inert; what is not inert is the modulo 2^32, which is the
        // assertion above. A small step therefore doubles cleanly:
        let (small, small_group) = step_offsets(1);
        assert_eq!(small[2], 2, "no truncation for a step that fits");
        assert_eq!(small[3], 3);
        assert_eq!(small[7], 7);
        assert_eq!(small_group, 8, "eight steps for the group advance");
    }

    #[test]
    fn the_stores_that_precede_the_second_batch_of_loads_are_seen_by_them() {
        // The interleaving, pinned **within one trip** — which is the only place it can be pinned,
        // and the reason this layout is so specific.
        //
        // One group of eight, unit step, index 0, and the output laid over `table[5..13]`. The
        // first three stores land on `table[5]`, `[6]` and `[7]`; the four loads that come *after*
        // them read `table[5]`, `[6]`, `[7]`, `[8]` as the upper halves of samples 4, 5, 6 and 7.
        // So three of those four loads see values this very trip has just written, and the answer
        // is derived below **by hand from the lifted order**, not from a model that shares it.
        //
        // Moving the three stores after the four loads — which is what a reader "tidying" the block
        // into loads-then-stores would do — changes samples 4, 5 and 6 and fails this test.
        const F: u64 = 0x8000;
        let mut g = guest();
        table(&mut g);
        start(&mut g, 0, F as u16);
        let overlap = TABLE + 4 * 5;

        resample(&mut g, 8, TABLE, overlap, INDEX, PHASE, 0x0001_0000).unwrap();

        let got: Vec<f32> = (0..8).map(|i| g.f32(overlap + 4 * i).unwrap()).collect();
        let s = scale();
        let i = |a: f64, b: f64| interpolate(a, b, F, s) as f32;

        // The twelve loads all happen before any store, so they see the original table.
        let out0 = i(0.0, 1.0); // stored third, into table[5]
        let out1 = i(1.0, 2.0); // stored first, into table[6]
        let out2 = i(2.0, 3.0); // stored second, into table[7]
        let out3 = i(3.0, 4.0); // stored fourth, into table[8]
        // Then s4b..s7b are loaded from table[5], [6], [7], [8] — the first three of which now
        // hold out0, out1 and out2. table[8] is still 8.0: out3 is stored *after* that load.
        let out4 = i(4.0, out0 as f64);
        let out5 = i(5.0, out1 as f64);
        let out6 = i(6.0, out2 as f64);
        let out7 = i(7.0, 8.0);

        assert_eq!(got, vec![out0, out1, out2, out3, out4, out5, out6, out7]);
        // And the aliasing is real rather than incidental: the un-aliased answers differ.
        assert_ne!(out4, i(4.0, 5.0));
        assert_ne!(out5, i(5.0, 6.0));
        assert_ne!(out6, i(6.0, 7.0));
    }

    #[test]
    fn it_restores_the_entry_flush_mode() {
        let mut g = guest();
        table(&mut g);
        start(&mut g, 0, 0);
        let before = crate::vmx::get_mxcsr();
        resample(&mut g, 19, TABLE, OUT, INDEX, PHASE, 0x0001_0000).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before);
    }
}
