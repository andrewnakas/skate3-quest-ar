//! The mix dispatcher over the six-pointer stage descriptor, and the two-source crossfade it runs.
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`crossfade`] | `sub_82B3D0A8` | verified | 662 | 93,954 | 111,902 |
//! | [`run_mix`] | `sub_82B3D4F8` | verified | 76 | 136,586 | 225,914 |
//!
//! Both `.inc` headers lead with `// STATUS: verified` and `docs/ports.md` agrees. One caveat on the
//! dispatcher's label, which its own note raises: until 2026-09-12 its `Windows()` refused every
//! two-gain call, so the label was first earned on the one-gain path alone. Every harness log since
//! (`probe/harness/out/vec_mixb.log` through `trig_boot.log`) shows it at `skipped=0` over 84,825 to
//! 137,561 runs a session, so both of its paths are compared now.
//!
//! ## The family this belongs to
//!
//! Three dispatchers read one 24-byte descriptor through `r7` and a gain block through `r3`, and
//! choose on the descriptor's `+8`: [`crate::stage::run_stage`] (`sub_82B39FA0`), [`run_mix`] here, and
//! [`crate::allpass::run_allpass`] (`sub_82B38B68`). `+16` and `+20` are the two buffers written on
//! every path of all three, and `+16`/`+20` of the gain block are its first two singles. The offsets
//! below are asserted against `stage`'s at compile time, so the three readings cannot drift apart
//! silently.
//!
//! What `+8` *means* is the one thing that differs: a selector in `run_stage`, a bypass flag in
//! `run_allpass`, and here **a pointer** — the crossfade's second source. Zero means there is nothing
//! to crossfade with, so the one-gain kernel [`crate::dsp::scale_add`] runs instead.
//!
//! **They share a file for the reason [`crate::stage`]'s pair do.** `sub_82B3D0A8` has no `stwu` and
//! reads its ninth argument, `F`, off `84(r1)` — the slot `sub_82B3D4F8` stores into its own fresh
//! frame immediately before the call. [`run_mix`] writes that slot and [`crossfade`] reads it back out
//! of guest memory, so the coupling is exercised rather than modelled.
//!
//! ## What the green here means
//!
//! **Unit-tested against a verified reference**, the crate README's second kind of green, with one
//! qualification that matters for the kernel. `sub_82B3D0A8.md` records that every call the game makes
//! arrives with `B` and `C` misaligned (`0x…4D8`, `0x…4EC`) and a count of 128, so **only the four-wide
//! scalar path has ever been compared** by the harness. The vector path and the one-at-a-time tail have
//! a C++ body but no evidence behind it. For those two paths this translation was checked against the
//! **lifted body directly**, instruction by instruction — every effective address reduced to its
//! array's base plus an offset in 32-bit wrapping arithmetic, and every operand order read off the
//! lifted line — and it agrees with the C++ at every site. That is a transcription check, not a
//! measurement.
//!
//! ## What the kernel computes
//!
//! Six float arrays and two gains. With `A` the accumulator, `B` and `C` the two sources, `D` a
//! per-sample coefficient, `E` and `F` the outputs:
//!
//! ```text
//! F[i] = D[i]·C[i] + (1 − D[i])·B[i]                       the plain blend
//! E[i] = A[i] + ((1 − D[i])·B[i])·gain_b + (D[i]·C[i])·gain_c   the weighted blend, accumulated
//! ```
//!
//! ## Three paths, and more rounding shapes than paths
//!
//! | path | taken for | `F` | `C`·`D`·`gain_c` | the `1.0` |
//! |---|---|---|---|---|
//! | vector, 16 a pass | `16·⌊count/16⌋` elements, only if `B` **and** `C` are 16-byte aligned | three roundings: `mul`, `mul`, `add` | `(D·C)·gain_c`, f32 | built by `vupkd3d128` |
//! | four-wide scalar | whole groups of four from where the vector path stopped | one `fmadds` | lanes 0, 1: `(C·D)·gain_c`; lane 2: **`(D·gain_c)·C`**; lane 3: `(D·C)·gain_c` | loaded from [`ONE_CELL`] |
//! | one at a time | the remainder | one `fmadds` | `(D·C)·gain_c` | loaded |
//!
//! Lane 2 of the four-wide loop associates its weighting differently from the other three: one
//! algebraic expression, two roundings of it, in one loop body. Each site is written the way it is
//! lifted. `C·D` against `D·C` is the other difference in that column and it is **not** observable on
//! numbers: the two products are the same value, and differ only in which NaN payload survives, which
//! `docs/vmx128-exactness.md` rule 4 puts beyond what source can decide. They are copied anyway.
//!
//! The gains reach the paths differently too: the vector path narrows each to a single once (four
//! `stfs` and one `lvx128`), while the scalar paths multiply by the guest FPR in double. They agree for
//! any gain that is already a single, which [`run_mix`]'s `lfs` guarantees; they are not one function.
//!
//! ## Reproduced rather than fixed
//!
//! - **Only `B` and `C` are alignment-tested.** On the vector path `A`, `D`, `E` and `F` go through
//!   `lvx128`/`stvx128`, which mask the address down to 16 bytes, so a misaligned `E` or `F` is written
//!   **low**: the words below the pointer are clobbered and the top of the nominal run is not written.
//! - **`A` is loaded after the four `F` stores** in each vector pass, and each quarter of it after the
//!   previous quarter's `E` store, so an `A` that aliases `F` feeds back the blend just written.
//! - **The scalar paths reload `B`, `C` and `D` after every `E` store**, so an `E` that aliases one of
//!   them computes `F` from the value just stored.
//!
//! ## Not reproduced
//!
//! The kernel's frame, none of which a caller can read back: `stw r3,20(r1)` (the count's home slot,
//! which sits in the **caller's** frame, at [`COUNT_HOME_SLOT`]), `__savegprlr_14`'s spill of `r14`-`r31`
//! and the link register at `-152(r1)`…`-5(r1)`, the two gain splats at `-192(r1)`…`-161(r1)`, and the
//! three spills at `-224(r1)`…`-201(r1)`. The splats are built as registers instead, which is exact only
//! when the frame is 16-byte aligned — see [`crate::vmx::splat_frame_unaligned`] for what happens when
//! it is not. `r14`-`r31` and every vector register the kernel touches (`v0`-`v13`, `v32`-`v63`, all
//! volatile) have no model here, and none is part of its result.
//!
//! The dispatcher's `stw r12,-8(r1)` (the link register) is absent for the same reason. Its back chain
//! and its outgoing argument slot are written, as [`crate::stage::run_stage`] writes its own.

#![allow(unused_unsafe)] // see the note at the top of `crate::vmx`

use crate::vmx::{self, Fpscr};
use crate::{Guest, Result, dsp, fp, stage};
use core::arch::x86_64::*;

/// `((imm & 0xFFFF) << 16)` — the `lis` half of an address, **computed**.
const fn lis(imm: i32) -> u32 {
    ((imm as u32) & 0xFFFF) << 16
}

// ------------------------------------------------------------ sub_82B3D0A8: its two frame words

/// `stw r3,20(r1)` — the count's own parameter home slot. The kernel has no `stwu`, so this is a
/// write into the **caller's** frame; it is reloaded after the vector loop and kept in a local here.
pub const COUNT_HOME_SLOT: u32 = 20;
/// `lwz r30,84(r1)` — `F`, the ninth argument. The ABI gives every argument an eight-byte home slot
/// from `20(r1)`, float arguments included, which is also why `r4` and `r5` are never read: they are
/// `f1`'s and `f2`'s slots.
pub const ARG_F: u32 = 84;

const _: () = assert!(
    ARG_F == COUNT_HOME_SLOT + 8 * 8,
    "the ninth eight-byte slot from 20(r1)"
);
const _: () = assert!(
    ARG_F == stage::ARG_MIXED,
    "the slot sub_82B399D0 reads its first stack word from"
);

/// `stfs f2,-192(r1)` ×4, reloaded by `lvx128 v63` — `gain_c`'s splat. Not written; see the module note.
pub const GAIN_C_SPLAT: i32 = -192;
/// `stfs f1,-176(r1)` ×4, reloaded by `lvx128 v0` — `gain_b`'s splat.
pub const GAIN_B_SPLAT: i32 = -176;
const _: () = assert!(
    GAIN_C_SPLAT % 16 == 0 && GAIN_B_SPLAT % 16 == 0,
    "aligned iff r1 is"
);

/// `lis r11,-32206 ; lfs f0,-22460(r11)` — the scalar paths' `1.0`, read **live**.
///
/// The address is `((imm & 0xFFFF) << 16) + offset`, computed above and asserted below rather than read
/// off by eye. It is the cell [`crate::eval::ONE_SINGLE`] names, whose dump value is `1.0f`; nothing is
/// assumed about that here, because the original loads it and a patched image should reach the port.
pub const ONE_CELL: u32 = lis(-32206).wrapping_add(-22460i32 as u32);
const _: () = assert!(lis(-32206) == 0x8232_0000, "lis r11,-32206");
const _: () = assert!(ONE_CELL == 0x8231_A844, "lis -32206 ; lfs -22460");
const _: () = assert!(ONE_CELL == crate::eval::ONE_SINGLE, "the pool's 1.0f cell");

/// `vupkd3d128 v62,v63,0` of `vspltisw128 v63,0`: every lane is `0x3F800000 | 0`, which is `1.0f`.
/// The vector path's `1.0`, **built**, not loaded — so a patched [`ONE_CELL`] does not reach it.
const VUPKD3D_OF_ZERO: i32 = 0x3F80_0000;

/// Elements per vector pass: four `lvx128` of each input.
pub const BLOCK_SAMPLES: i32 = 16;
/// Bytes per array per vector pass.
pub const BLOCK_BYTES: u32 = 64;
/// Bytes per array per four-wide scalar trip.
pub const GROUP_BYTES: u32 = 16;
const _: () = assert!(BLOCK_BYTES == 4 * BLOCK_SAMPLES as u32 && GROUP_BYTES * 4 == BLOCK_BYTES);

// ---------------------------------------------------------- sub_82B3D4F8: the descriptor and gains

/// `lwz r6,0(r11)` — `A`, the accumulator, on the two-gain path; `y`, the addend, on the other.
pub const DESC_A: u32 = 0;
/// `lwz r7,4(r11)` — `B`; `x`, the one-gain kernel's source.
pub const DESC_B: u32 = 4;
/// `lwz r8,8(r7)` — zero picks the one-gain kernel; anything else **is** `C`, the second source.
pub const DESC_C: u32 = 8;
/// `lwz r9,12(r11)` — `D`, the coefficient. Read on the two-gain path only.
pub const DESC_D: u32 = 12;
/// `lwz r10,16(r11)` — `E`; `z` on the other path.
pub const DESC_E: u32 = 16;
/// `lwz r5,20(r11)` — `F`; `w` on the other path. Passed twice on the two-gain path: in `r5`, which the
/// kernel never reads, and through [`ARG_F`] of the new frame, which it does.
pub const DESC_F: u32 = 20;
/// The whole descriptor, which is what the C++ `Windows()` declares as a read span.
pub const DESC_BYTES: u32 = 24;

const _: () = assert!(DESC_A == stage::DESC_ADDEND && DESC_B == stage::DESC_SOURCE);
const _: () = assert!(DESC_C == stage::DESC_SELECT && DESC_E == stage::DESC_MIXED);
const _: () = assert!(DESC_F == stage::DESC_COPY && DESC_BYTES == stage::DESC_BYTES);

/// `lfs f1,16(r6)` — `gain_b` on the two-gain path, the only gain on the other.
pub const GAIN0: u32 = 16;
/// `lfs f2,20(r6)` — `gain_c`. Read on the two-gain path only.
pub const GAIN1: u32 = 20;
const _: () = assert!(GAIN0 == stage::GAIN0 && GAIN1 == stage::GAIN1);

/// `stwu r1,-96(r1)`.
pub const FRAME_BYTES: u32 = 96;
const _: () = assert!(ARG_F < FRAME_BYTES && FRAME_BYTES % 16 == 0);

/// Whether a call with these two sources takes the vector path: `clrlwi r11,r7,28` and then
/// `clrlwi r11,r8,28`, each against zero. `A`, `D`, `E` and `F` are not tested.
pub const fn takes_vector_path(b: u32, c: u32) -> bool {
    (b & 0xF) == 0 && (c & 0xF) == 0
}

// ======================================================= sub_82B3D0A8: the two-source crossfade

/// `sub_82B3D0A8` — crossfade `B` into `C` by `D`: the plain blend into `F`, and the gain-weighted blend
/// accumulated onto `A` into `E`.
///
/// Arguments, by register:
///
/// | register | parameter | what it is |
/// |---|---|---|
/// | `r3` | `count` | elements; only the low word is used, and it is compared **signed** |
/// | `r6` | `a` | `A`, the accumulator, read only |
/// | `r7` | `b` | `B`, the first source; with `C`, picks the path |
/// | `r8` | `c` | `C`, the second source |
/// | `r9` | `d` | `D`, the per-sample coefficient, read only |
/// | `r10` | `e` | `E`, written |
/// | `r1` | `sp` | the caller's frame; `84(r1)` holds `F`, written |
/// | `f1` | `gain_b` | weights the `B` side of `E` |
/// | `f2` | `gain_c` | weights the `C` side |
///
/// `r4` and `r5` are not arguments. The six pointers are full registers here and truncated where the
/// original truncates them: every effective address in the lifted body is a `.u32` sum, so all of them
/// wrap at 32 bits. There is no return value (`kReturnNone`).
///
/// **Writes** exactly two runs per output. On the vector path, `64·⌊count/16⌋` bytes from `E & ~15` and
/// from `F & ~15`; then four bytes per element for the rest, at the exact `E + 4i` and `F + 4i`.
/// **Reads** the ninth-argument word at `sp + 84`; `A` and `D` through the same masking as the stores,
/// `B` and `C` at their aligned bases on the vector path, all four exactly on the scalar paths; and the
/// four-byte [`ONE_CELL`] **whenever the vector path leaves anything over**. The C++ `Windows()` does not
/// declare that last cell as a read.
///
/// **Refuses** (an `Err` before anything is read or written) exactly one input the original handles: a
/// vector loop that would run with `sp` not 16-byte aligned. See [`vmx::splat_frame_unaligned`].
#[allow(clippy::too_many_arguments)]
pub fn crossfade(
    g: &mut Guest,
    count: u64,
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
    sp: u32,
    gain_b: f64,
    gain_c: f64,
) -> Result<()> {
    if !vmx::supported() {
        return Err(vmx::unsupported());
    }
    unsafe {
        crossfade_body(
            g,
            count as u32 as i32,
            a as u32,
            b as u32,
            c as u32,
            d as u32,
            e as u32,
            sp,
            gain_b,
            gain_c,
        )
    }
}

#[allow(clippy::too_many_arguments)]
#[target_feature(enable = "sse4.1,fma")]
unsafe fn crossfade_body(
    g: &mut Guest,
    count: i32,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    e: u32,
    sp: u32,
    gain_b: f64,
    gain_c: f64,
) -> Result<()> {
    let vector = takes_vector_path(b, c);
    // srawi r11,r3,4 ; addze r5,r11 — a signed divide by sixteen that truncates toward zero; the
    // `srawi` carry is the rounding correction `addze` folds back in, which is Rust's `/`.
    let blocks = count / BLOCK_SAMPLES;
    // rlwinm r11,r5,2,0,29 ; cmpwi cr6,r11,0 ; ble — whether the vector loop runs at all. Decided here,
    // ahead of its lifted position, only because it decides whether the splats are read back; nothing
    // before that point in the original writes guest memory this crate models.
    let quads = ((blocks as u32) << 2) as i32;
    if vector && quads > 0 && sp & 0xF != 0 {
        return Err(vmx::splat_frame_unaligned(sp));
    }

    let f = g.u32(sp.wrapping_add(ARG_F))?; // lwz r30,84(r1)
    // stw r3,20(r1) — into the caller's frame, reloaded after the vector loop. Kept in `count`.
    let mut fpscr = Fpscr::capture();
    let mut done: i32 = 0; // li r27,0

    // clrlwi r11,r7,28 ; bne cr6 ; clrlwi r11,r8,28 ; bne cr6 — both to loc_82B3D2A4.
    if vector {
        // The first `stfs f2,-192(r1)` emits the disable, and it sits *before* the `ble` that skips the
        // loop — so it is issued, and both gains narrowed, even when the loop does not run.
        fpscr.disable_flush_mode_unconditional();
        let gain_c_v = unsafe { _mm_set1_ps(gain_c as f32) }; // stfs f2 ×4 ; lvx128 v63,r0,r11
        let gain_b_v = unsafe { _mm_set1_ps(gain_b as f32) }; // stfs f1 ×4 ; lvx128 v0,r0,r10
        let one_v = unsafe { _mm_castsi128_ps(_mm_set1_epi32(VUPKD3D_OF_ZERO)) }; // vupkd3d128 v62

        if quads > 0 {
            // addi r11,r11,-1 ; rlwinm r11,r11,30,2,31 ; addi r3,r11,1 ; mtctr r3 — exactly `blocks`.
            let iterations = ((quads as u32 - 1) >> 2) + 1;
            for k in 0..iterations {
                // The lifted cursors are offset from the block they address (r31 = A, r5 = D + 16,
                // r11 = B + 32, r4 = F + 48) and reach the other arrays through precomputed base
                // differences. Every effective address reduces, in 32-bit wrapping arithmetic, to one
                // array's base plus 64k plus 0, 16, 32 or 48, which is what these are.
                let u = BLOCK_BYTES.wrapping_mul(k);
                let (pa, pb, pc) = (a.wrapping_add(u), b.wrapping_add(u), c.wrapping_add(u));
                let (pd, pe, pf) = (d.wrapping_add(u), e.wrapping_add(u), f.wrapping_add(u));

                let d0 = unsafe { vmx::lvx128_ps(g, pd)? }; // lvx128 v61,r5,r15
                let d1 = unsafe { vmx::lvx128_ps(g, pd.wrapping_add(16))? }; // lvx128 v60,r0,r5
                fpscr.enable_flush_mode_unconditional(); // at vsubfp128 v59,v62,v61
                let inv0 = unsafe { vmx::vsubfp(one_v, d0) }; // vsubfp128 v59,v62,v61
                let d2 = unsafe { vmx::lvx128_ps(g, pd.wrapping_add(32))? }; // lvx128 v58,r28,r11
                let inv1 = unsafe { vmx::vsubfp(one_v, d1) }; // vsubfp128 v57,v62,v60
                let c0 = unsafe { vmx::lvx128_ps(g, pc)? }; // lvx128 v56,r31,r17
                let inv2 = unsafe { vmx::vsubfp(one_v, d2) }; // vsubfp128 v55,v62,v58
                let d3 = unsafe { vmx::lvx128_ps(g, pd.wrapping_add(48))? }; // lvx128 v53,r5,r10
                // D is the FIRST operand of every product with C, and (1 - D) of every product with B,
                // as lifted. Rule 4: the slot is part of the semantics on two NaNs, so it is copied.
                let wet0 = unsafe { vmx::vmulfp(d0, c0) }; // vmulfp128 v54,v61,v56
                let inv3 = unsafe { vmx::vsubfp(one_v, d3) }; // vsubfp128 v52,v62,v53
                let b0 = unsafe { vmx::lvx128_ps(g, pb)? }; // lvx128 v51,r11,r14
                let b1 = unsafe { vmx::lvx128_ps(g, pb.wrapping_add(16))? }; // lvx128 v50,r11,r3
                let b2 = unsafe { vmx::lvx128_ps(g, pb.wrapping_add(32))? }; // lvx128 v49,r0,r11
                let c1 = unsafe { vmx::lvx128_ps(g, pc.wrapping_add(16))? }; // lvx128 v47,r5,r24
                let b3 = unsafe { vmx::lvx128_ps(g, pb.wrapping_add(48))? }; // lvx128 v48,r11,r9
                let wet1 = unsafe { vmx::vmulfp(d1, c1) }; // vmulfp128 v45,v60,v47
                let c2 = unsafe { vmx::lvx128_ps(g, pc.wrapping_add(32))? }; // lvx128 v46,r29,r11
                let c3 = unsafe { vmx::lvx128_ps(g, pc.wrapping_add(48))? }; // lvx128 v44,r4,r20
                let wet2 = unsafe { vmx::vmulfp(d2, c2) }; // vmulfp128 v43,v58,v46
                let dry0 = unsafe { vmx::vmulfp(inv0, b0) }; // vmulfp128 v13,v59,v51
                let dry1 = unsafe { vmx::vmulfp(inv1, b1) }; // vmulfp128 v12,v57,v50
                let dry2 = unsafe { vmx::vmulfp(inv2, b2) }; // vmulfp128 v11,v55,v49
                let wet3 = unsafe { vmx::vmulfp(d3, c3) }; // vmulfp128 v42,v53,v44
                let dry3 = unsafe { vmx::vmulfp(inv3, b3) }; // vmulfp128 v10,v52,v48
                let wg0 = unsafe { vmx::vmulfp(wet0, gain_c_v) }; // vmulfp128 v9,v54,v63
                let wg1 = unsafe { vmx::vmulfp(wet1, gain_c_v) }; // vmulfp128 v8,v45,v63
                let wg2 = unsafe { vmx::vmulfp(wet2, gain_c_v) }; // vmulfp128 v7,v43,v63
                // F is a separate multiply, multiply and add here -- NOT an FMA. The scalar paths fuse
                // the same expression; the difference is the original's.
                let blend0 = unsafe { vmx::vaddfp(wet0, dry0) }; // vaddfp128 v41,v54,v13
                let blend1 = unsafe { vmx::vaddfp(wet1, dry1) }; // vaddfp128 v40,v45,v12
                let blend2 = unsafe { vmx::vaddfp(wet2, dry2) }; // vaddfp128 v39,v43,v11
                let wg3 = unsafe { vmx::vmulfp(wet3, gain_c_v) }; // vmulfp128 v6,v42,v63
                let blend3 = unsafe { vmx::vaddfp(wet3, dry3) }; // vaddfp128 v38,v42,v10
                let mix0 = unsafe { vmx::vmaddfp(dry0, gain_b_v, wg0) }; // vmaddfp v13,v13,v0,v9
                let mix1 = unsafe { vmx::vmaddfp(dry1, gain_b_v, wg1) }; // vmaddfp v12,v12,v0,v8
                let mix2 = unsafe { vmx::vmaddfp(dry2, gain_b_v, wg2) }; // vmaddfp v11,v11,v0,v7
                unsafe { vmx::stvx128_ps(g, pf, blend0)? }; // stvx128 v41,r4,r10
                unsafe { vmx::stvx128_ps(g, pf.wrapping_add(16), blend1)? }; // stvx128 v40,r5,r23
                unsafe { vmx::stvx128_ps(g, pf.wrapping_add(32), blend2)? }; // stvx128 v39,r27,r11
                let mix3 = unsafe { vmx::vmaddfp(dry3, gain_b_v, wg3) }; // vmaddfp v10,v10,v0,v6
                unsafe { vmx::stvx128_ps(g, pf.wrapping_add(48), blend3)? }; // stvx128 v38,r0,r4

                // A is loaded AFTER the four F stores, and each quarter after the previous E store.
                let a0 = unsafe { vmx::lvx128_ps(g, pa)? }; // lvx128 v37,r0,r31
                let out0 = unsafe { vmx::vaddfp(mix0, a0) }; // vaddfp128 v36,v13,v37
                unsafe { vmx::stvx128_ps(g, pe, out0)? }; // stvx128 v36,r31,r16
                let a1 = unsafe { vmx::lvx128_ps(g, pa.wrapping_add(16))? }; // lvx128 v35,r5,r22
                let out1 = unsafe { vmx::vaddfp(mix1, a1) }; // vaddfp128 v34,v12,v35
                unsafe { vmx::stvx128_ps(g, pe.wrapping_add(16), out1)? }; // stvx128 v34,r5,r21
                let a2 = unsafe { vmx::lvx128_ps(g, pa.wrapping_add(32))? }; // lvx128 v33,r11,r26
                let out2 = unsafe { vmx::vaddfp(mix2, a2) }; // vaddfp128 v32,v11,v33
                unsafe { vmx::stvx128_ps(g, pe.wrapping_add(32), out2)? }; // stvx128 v32,r11,r25
                let a3 = unsafe { vmx::lvx128_ps(g, pa.wrapping_add(48))? }; // lvx128 v61,r4,r19
                let out3 = unsafe { vmx::vaddfp(mix3, a3) }; // vaddfp128 v60,v10,v61
                unsafe { vmx::stvx128_ps(g, pe.wrapping_add(48), out3)? }; // stvx128 v60,r4,r18
            } // bdnz 0x82b3d198
        }
        // loc_82B3D2A0: rlwinm r27,r5,4,0,27 — sixteen per block, zero-extended, compared signed below.
        done = ((blocks as u32) << 4) as i32;
    }

    // loc_82B3D2A4: mr r15,r27 ; cmpw cr6,r27,r3 ; bge cr6,0x82b3d4f4 — signed, so a zero or negative
    // count leaves here having written nothing and read nothing past `84(r1)`.
    let mut tail_start = done;
    if done >= count {
        return Ok(());
    }

    // lis r11,-32206 ; subf r5,r27,r3 ; cmpwi cr6,r5,4 ; lfs f0,-22460(r11) ; blt — the load sits
    // before the `blt`, so the cell is read whenever anything is left, four-wide loop or not.
    fpscr.disable_flush_mode_unconditional();
    let one = fp::load_single(g, ONE_CELL)?;
    let left = count.wrapping_sub(done);
    if left >= 4 {
        // subf r11,r27,r3 ; addi r11,r11,-4 ; rlwinm r11,r11,30,2,31 ; addi r11,r11,1 ; mtctr r11
        let groups = ((left as u32).wrapping_sub(4) >> 2) + 1;
        // rlwinm r22,r11,2,0,29 ; add r15,r22,r27 — where the one-at-a-time loop starts.
        tail_start = ((groups << 2) & 0xFFFF_FFFC).wrapping_add(done as u32) as i32;
        // rlwinm r5,r27,2,0,29. The lifted cursors are r11 = &B[j+1], r5 = &D[j+2], r4 = &C[j+3] and
        // r31 = &E[j]; all 24 effective addresses reduce to one of these bases plus 0, 4, 8 or 12.
        let offset = ((done as u32) << 2) & 0xFFFF_FFFC;
        for k in 0..groups {
            let o = offset.wrapping_add(GROUP_BYTES.wrapping_mul(k));
            let (pa, pb, pc) = (a.wrapping_add(o), b.wrapping_add(o), c.wrapping_add(o));
            let (pd, pe, pf) = (d.wrapping_add(o), e.wrapping_add(o), f.wrapping_add(o));

            // --- element j ------------------------------------------------------------------------
            fpscr.disable_flush_mode_unconditional(); // at lfs f13,-8(r5)
            let d0 = fp::load_single(g, pd)?; // lfs f13,-8(r5)
            let c0 = fp::load_single(g, pc)?; // lfs f12,-12(r4)
            let inv0 = fp::sub_single(one, d0); // fsubs f11,f0,f13
            let wet0 = fp::mul_single(c0, d0); // fmuls f10,f12,f13 — C first
            let b0 = fp::load_single(g, pb)?; // lfs f9,-4(r11)
            let a0 = fp::load_single(g, pa)?; // lfsx f8,r31,r28
            let dry0 = fp::mul_single(inv0, b0); // fmuls f7,f11,f9
            let wg0 = fp::mul_single(wet0, gain_c); // fmuls f6,f10,f2
            let mix0 = fp::fmadd_single(dry0, gain_b, wg0); // fmadds f5,f7,f1,f6
            fp::store_single(g, pe, fp::add_single(mix0, a0))?; // fadds f4,f5,f8 ; stfs f4,0(r31)
            // The three inputs are reloaded after that store, as lifted, because it may alias them.
            let c0b = fp::load_single(g, pc)?; // lfs f3,-12(r4)
            let b0b = fp::load_single(g, pb)?; // lfs f13,-4(r11)
            let d0b = fp::load_single(g, pd)?; // lfs f12,-8(r5)
            let inv0b = fp::sub_single(one, d0b); // fsubs f11,f0,f12
            let wet0b = fp::mul_single(c0b, d0b); // fmuls f10,f3,f12 — C first
            fp::store_single(g, pf, fp::fmadd_single(inv0b, b0b, wet0b))?; // fmadds f9 ; stfsx f9,r31,r16

            // --- element j+1 ----------------------------------------------------------------------
            let c1 = fp::load_single(g, pc.wrapping_add(4))?; // lfsx f8,r26,r11
            let b1 = fp::load_single(g, pb.wrapping_add(4))?; // lfs f7,0(r11)
            let a1 = fp::load_single(g, pa.wrapping_add(4))?; // lfsx f6,r22,r28
            let d1 = fp::load_single(g, pd.wrapping_add(4))?; // lfsx f5,r25,r11
            let inv1 = fp::sub_single(one, d1); // fsubs f4,f0,f5
            let wet1 = fp::mul_single(c1, d1); // fmuls f3,f8,f5 — C first
            let dry1 = fp::mul_single(inv1, b1); // fmuls f13,f4,f7
            let wg1 = fp::mul_single(wet1, gain_c); // fmuls f12,f3,f2
            let mix1 = fp::fmadd_single(dry1, gain_b, wg1); // fmadds f11,f13,f1,f12
            fp::store_single(g, pe.wrapping_add(4), fp::add_single(mix1, a1))?; // fadds f10 ; stfsx f10,r11,r23
            let c1b = fp::load_single(g, pc.wrapping_add(4))?; // lfsx f9,r26,r11
            let d1b = fp::load_single(g, pd.wrapping_add(4))?; // lfsx f7,r25,r11
            let inv1b = fp::sub_single(one, d1b); // fsubs f6,f0,f7
            let wet1b = fp::mul_single(c1b, d1b); // fmuls f5,f9,f7 — C first
            let b1b = fp::load_single(g, pb.wrapping_add(4))?; // lfs f8,0(r11)
            fp::store_single(g, pf.wrapping_add(4), fp::fmadd_single(inv1b, b1b, wet1b))?; // fmadds f4 ; stfsx f4,r19,r11

            // --- element j+2: this lane weights by (D·gain_c)·C, not (C·D)·gain_c ------------------
            let b2 = fp::load_single(g, pb.wrapping_add(8))?; // lfs f3,4(r11)
            let c2 = fp::load_single(g, pc.wrapping_add(8))?; // lfsx f9,r5,r24
            let a2 = fp::load_single(g, pa.wrapping_add(8))?; // lfsx f13,r20,r28
            let d2 = fp::load_single(g, pd.wrapping_add(8))?; // lfs f12,0(r5)
            let inv2 = fp::sub_single(one, d2); // fsubs f11,f0,f12
            let dg2 = fp::mul_single(d2, gain_c); // fmuls f10,f12,f2
            let dry2 = fp::mul_single(inv2, b2); // fmuls f8,f11,f3
            let wg2 = fp::mul_single(dg2, c2); // fmuls f7,f10,f9
            let mix2 = fp::fmadd_single(dry2, gain_b, wg2); // fmadds f6,f8,f1,f7
            fp::store_single(g, pe.wrapping_add(8), fp::add_single(mix2, a2))?; // fadds f5 ; stfsx f5,r27,r29
            let c2b = fp::load_single(g, pc.wrapping_add(8))?; // lfsx f4,r5,r24
            let b2b = fp::load_single(g, pb.wrapping_add(8))?; // lfs f3,4(r11)
            let d2b = fp::load_single(g, pd.wrapping_add(8))?; // lfs f13,0(r5)
            let inv2b = fp::sub_single(one, d2b); // fsubs f12,f0,f13
            let wet2b = fp::mul_single(d2b, c2b); // fmuls f11,f13,f4 — D first
            fp::store_single(g, pf.wrapping_add(8), fp::fmadd_single(inv2b, b2b, wet2b))?; // fmadds f10 ; stfsx f10,r5,r18

            // --- element j+3: (D·C)·gain_c ---------------------------------------------------------
            let c3 = fp::load_single(g, pc.wrapping_add(12))?; // lfs f9,0(r4)
            let b3 = fp::load_single(g, pb.wrapping_add(12))?; // lfs f8,8(r11)
            let a3 = fp::load_single(g, pa.wrapping_add(12))?; // lfsx f7,r21,r28
            let d3 = fp::load_single(g, pd.wrapping_add(12))?; // lfs f6,4(r5)
            let inv3 = fp::sub_single(one, d3); // fsubs f5,f0,f6
            let wet3 = fp::mul_single(d3, c3); // fmuls f4,f6,f9 — D first
            let dry3 = fp::mul_single(inv3, b3); // fmuls f3,f5,f8
            let wg3 = fp::mul_single(wet3, gain_c); // fmuls f13,f4,f2
            let mix3 = fp::fmadd_single(dry3, gain_b, wg3); // fmadds f12,f3,f1,f13
            fp::store_single(g, pe.wrapping_add(12), fp::add_single(mix3, a3))?; // fadds f11 ; stfsx f11,r29,r4
            let c3b = fp::load_single(g, pc.wrapping_add(12))?; // lfs f10,0(r4)
            let b3b = fp::load_single(g, pb.wrapping_add(12))?; // lfs f9,8(r11)
            let d3b = fp::load_single(g, pd.wrapping_add(12))?; // lfs f8,4(r5)
            let inv3b = fp::sub_single(one, d3b); // fsubs f7,f0,f8
            let wet3b = fp::mul_single(d3b, c3b); // fmuls f6,f8,f10 — D first
            fp::store_single(g, pf.wrapping_add(12), fp::fmadd_single(inv3b, b3b, wet3b))?; // fmadds f5 ; stfsx f5,r17,r4
        } // bdnz 0x82b3d330
    }

    // loc_82B3D474: cmpw cr6,r15,r3 ; bge cr6,0x82b3d4f4
    if tail_start >= count {
        return Ok(());
    }
    // subf r5,r15,r3 ; mtctr r5 — exactly count - tail_start trips, at least one.
    let remaining = count.wrapping_sub(tail_start) as u32;
    // rlwinm r11,r15,2,0,29 ; add r11,r11,r7 — the cursor is &B[m], the rest base differences.
    let offset = ((tail_start as u32) << 2) & 0xFFFF_FFFC;
    for m in 0..remaining {
        let o = offset.wrapping_add(4u32.wrapping_mul(m));
        let (qa, qb, qc) = (a.wrapping_add(o), b.wrapping_add(o), c.wrapping_add(o));
        let (qd, qe, qf) = (d.wrapping_add(o), e.wrapping_add(o), f.wrapping_add(o));
        fpscr.disable_flush_mode_unconditional(); // at lfsx f13,r9,r11
        let dm = fp::load_single(g, qd)?; // lfsx f13,r9,r11
        let cm = fp::load_single(g, qc)?; // lfsx f12,r8,r11
        let invm = fp::sub_single(one, dm); // fsubs f11,f0,f13
        let wetm = fp::mul_single(dm, cm); // fmuls f10,f13,f12 — D first
        let bm = fp::load_single(g, qb)?; // lfs f9,0(r11)
        let am = fp::load_single(g, qa)?; // lfsx f8,r5,r6
        let drym = fp::mul_single(invm, bm); // fmuls f7,f11,f9
        let wgm = fp::mul_single(wetm, gain_c); // fmuls f6,f10,f2
        let mixm = fp::fmadd_single(drym, gain_b, wgm); // fmadds f5,f7,f1,f6
        fp::store_single(g, qe, fp::add_single(mixm, am))?; // fadds f4,f5,f8 ; stfsx f4,r11,r10
        let cm2 = fp::load_single(g, qc)?; // lfsx f3,r8,r11
        let bm2 = fp::load_single(g, qb)?; // lfs f13,0(r11)
        let dm2 = fp::load_single(g, qd)?; // lfsx f12,r9,r11
        let invm2 = fp::sub_single(one, dm2); // fsubs f11,f0,f12
        let wetm2 = fp::mul_single(dm2, cm2); // fmuls f10,f12,f3 — D first
        fp::store_single(g, qf, fp::fmadd_single(invm2, bm2, wetm2))?; // fmadds f9 ; stfsx f9,r7,r11
    } // bdnz 0x82b3d4a0
    Ok(())
}

// ============================================================== sub_82B3D4F8: the mix dispatcher

/// `sub_82B3D4F8` — run the one-gain kernel over a descriptor, or crossfade it with a second source.
///
/// Arguments, by register: `gains` is `r3` (the gain block, read at `+16` and `+20`), `count` is `r4`,
/// `desc` is `r7` (the 24-byte descriptor), and `sp` is the entry `r1`. `r5`, `r6` and `r8`-`r10` are not
/// read on entry. There is no return value (`kReturnNone`). `count` is the full 64-bit register because
/// the original hands it on whole, as the kernel's `r3` (`mr r3,r4`); each kernel truncates it itself.
///
/// | `desc[8]` | what runs |
/// |---|---|
/// | `0` | [`dsp::scale_add::scale_add_with_copy`]`(count, y = desc[0], x = desc[4], z = desc[16], w = desc[20], gains[16])` |
/// | anything else | [`crossfade`]`(count, A = desc[0], B = desc[4], C = desc[8], D = desc[12], E = desc[16], F = desc[20], gains[16], gains[20])` |
///
/// **Writes** this call's back chain at `sp - 96` and, on the two-gain path, [`ARG_F`] of that frame —
/// both inside its own 96-byte frame, which no recording holds; the link-register slot at `sp - 8` is not
/// reproduced. Then exactly what the kernel writes: `desc[16]` and `desc[20]`, on either path. **Reads**
/// the descriptor, `gains + 16`, `gains + 20` on the two-gain path only, and what the kernel reads —
/// including [`ONE_CELL`] on the two-gain path, which the C++ `Windows()` does not declare.
pub fn run_mix(g: &mut Guest, gains: u32, count: u64, desc: u32, sp: u32) -> Result<()> {
    // mflr r12 ; stw r12,-8(r1) — the link register, which this crate has no model of.
    let frame = sp.wrapping_sub(FRAME_BYTES);
    g.set_u32(frame, sp)?; // stwu r1,-96(r1): the back chain, then r1 moves
    // mr r6,r3 ; lwz r8,8(r7) ; mr r3,r4 ; mr r11,r7 ; cmplwi cr6,r8,0
    let select = g.u32(desc.wrapping_add(DESC_C))?;
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // at lfs f1,16(r6)
    let gain0 = fp::load_single(g, gains.wrapping_add(GAIN0))?; // lfs f1,16(r6)

    // bne cr6,0x82b3d544
    if select == 0 {
        let w = g.u32(desc.wrapping_add(DESC_F))?; // lwz r8,20(r7)
        let z = g.u32(desc.wrapping_add(DESC_E))?; // lwz r7,16(r7)
        let x = g.u32(desc.wrapping_add(DESC_B))?; // lwz r6,4(r11)
        let y = g.u32(desc.wrapping_add(DESC_A))?; // lwz r5,0(r11)
        // bl 0x82b3cf58 — its r3 is the count, whose low word it uses.
        dsp::scale_add::scale_add_with_copy(g, count as u32, y, x, z, w, gain0)?;
    } else {
        // loc_82B3D544
        let f = g.u32(desc.wrapping_add(DESC_F))?; // lwz r5,20(r11)
        fpscr.disable_flush_mode_unconditional(); // at lfs f2,20(r6)
        let gain1 = fp::load_single(g, gains.wrapping_add(GAIN1))?; // lfs f2,20(r6)
        let e = g.u32(desc.wrapping_add(DESC_E))?; // lwz r10,16(r11)
        let d = g.u32(desc.wrapping_add(DESC_D))?; // lwz r9,12(r11)
        let b = g.u32(desc.wrapping_add(DESC_B))?; // lwz r7,4(r11)
        let a = g.u32(desc.wrapping_add(DESC_A))?; // lwz r6,0(r11)
        g.set_u32(frame.wrapping_add(ARG_F), f)?; // stw r5,84(r1) — the ninth argument
        // bl 0x82b3d0a8 — r8 still holds the selector, which is the kernel's C.
        crossfade(
            g,
            count,
            a as u64,
            b as u64,
            select as u64,
            d as u64,
            e as u64,
            frame,
            gain0,
            gain1,
        )?;
    }
    // addi r1,r1,96 ; lwz r12,-8(r1) ; mtlr r12 ; blr
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x4000_0000;
    const GAINS: u32 = BASE + 0x0100;
    const DESC: u32 = BASE + 0x0140;
    const A: u32 = BASE + 0x0400;
    const B: u32 = BASE + 0x0800;
    const C: u32 = BASE + 0x0C00;
    const D: u32 = BASE + 0x1000;
    const E: u32 = BASE + 0x1400;
    const F: u32 = BASE + 0x1800;
    /// The caller's `r1`, 16-byte aligned as every measured entry is. [`run_mix`]'s frame sits 96 below.
    const SP: u32 = BASE + 0x3000;
    const FRAME: u32 = SP - FRAME_BYTES;
    const POISON: u32 = 0x7F7F_7F7F;
    /// Both gains are singles, as an `lfs` in the caller makes them, with full mantissas so that the
    /// products round and the tests can see which roundings happened.
    const GB: f64 = 0.707_106_77f32 as f64;
    const GC: f64 = 1.333_333_4f32 as f64;
    /// Every array aligned: the vector path.
    const ALIGNED: [u32; 4] = [A, B, C, D];
    /// The game's layout: `B` and `C` misaligned (`0x…4D8`, `0x…4EC` in the census), so the scalar paths.
    const GAME: [u32; 4] = [A, B + 8, C + 12, D];

    /// The base segment, the 1.0 cell in a segment of its own, and `F` at `84(r1)`.
    ///
    /// The cell gets its own segment so that a body computing the wrong address gets an `Err` rather
    /// than a plausible neighbouring word.
    fn guest() -> Guest {
        let mut g = Guest::single(BASE, 0x4000);
        g.put(ONE_CELL, 1.0f32.to_bits().to_be_bytes().to_vec());
        g.set_u32(SP + ARG_F, F).unwrap();
        g
    }

    fn put(g: &mut Guest, at: u32, values: &[f32]) {
        for (i, v) in values.iter().enumerate() {
            g.set_u32(at + 4 * i as u32, v.to_bits()).unwrap();
        }
    }

    fn get(g: &Guest, at: u32, n: usize) -> Vec<f32> {
        (0..n as u32).map(|i| g.f32(at + 4 * i).unwrap()).collect()
    }

    fn words(g: &Guest, at: u32, n: usize) -> Vec<u32> {
        (0..n as u32).map(|i| g.u32(at + 4 * i).unwrap()).collect()
    }

    fn poison(g: &mut Guest, at: u32, n: usize) {
        for i in 0..n as u32 {
            g.set_u32(at + 4 * i, POISON).unwrap();
        }
    }

    struct Rng(u32);
    impl Rng {
        fn bits(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }
        /// A normal single with a random mantissa, exponent `exp` or `exp + 1`, optionally signed.
        fn value(&mut self, exp: u32, signed: bool) -> f32 {
            let r = self.bits();
            let sign = if signed { r & 0x8000_0000 } else { 0 };
            let e = (exp + ((r >> 30) & 1)) << 23;
            f32::from_bits(sign | e | (self.bits() & 0x007F_FFFF))
        }
    }

    struct Inputs {
        a: Vec<f32>,
        b: Vec<f32>,
        c: Vec<f32>,
        d: Vec<f32>,
    }

    /// `A` in ±[1, 4), `B` and `C` in ±[0.5, 2), `D` in [0.25, 1) — full mantissas throughout.
    fn inputs(n: usize, seed: u32) -> Inputs {
        let mut r = Rng(seed);
        let a = (0..n).map(|_| r.value(127, true)).collect();
        let b = (0..n).map(|_| r.value(126, true)).collect();
        let c = (0..n).map(|_| r.value(126, true)).collect();
        let d = (0..n).map(|_| r.value(125, false)).collect();
        Inputs { a, b, c, d }
    }

    fn lay(g: &mut Guest, inp: &Inputs, at: [u32; 4]) {
        put(g, at[0], &inp.a);
        put(g, at[1], &inp.b);
        put(g, at[2], &inp.c);
        put(g, at[3], &inp.d);
    }

    fn run(g: &mut Guest, count: u64, at: [u32; 4], e: u32) -> Result<()> {
        crossfade(
            g,
            count,
            at[0] as u64,
            at[1] as u64,
            at[2] as u64,
            at[3] as u64,
            e as u64,
            SP,
            GB,
            GC,
        )
    }

    /// How one element is computed. Lanes 0, 1 and 3 of the four-wide loop and the tail all multiply
    /// `C·D` (or `D·C`) by `gain_c`, which are one value, so they share a form.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Form {
        Vector,
        Scalar,
        Lane2,
    }

    /// One element's `E` and `F`, written from the formulas in the module note — the vector form in f32
    /// with three roundings for `F`, the scalar forms in the guest's single-rounded double operations.
    #[allow(clippy::too_many_arguments)]
    fn element(
        form: Form,
        one: f64,
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        gb: f64,
        gc: f64,
    ) -> (f32, f32) {
        if form == Form::Vector {
            let (gb, gc) = (gb as f32, gc as f32);
            let inv = 1.0f32 - d;
            let wet = d * c;
            let dry = inv * b;
            // vmaddfp: the recomp's is unfused, so the product rounds before the add (corrected
            // 2026-09-13; see vmx.rs).
            let mix = dry * gb + wet * gc;
            return (mix + a, wet + dry);
        }
        let (a, b, c, d) = (a as f64, b as f64, c as f64, d as f64);
        let inv = fp::sub_single(one, d);
        let dry = fp::mul_single(inv, b);
        let weighted = if form == Form::Lane2 {
            fp::mul_single(fp::mul_single(d, gc), c)
        } else {
            fp::mul_single(fp::mul_single(c, d), gc)
        };
        let e = fp::add_single(fp::fmadd_single(dry, gb, weighted), a);
        let f = fp::fmadd_single(inv, b, fp::mul_single(d, c));
        (e as f32, f as f32)
    }

    /// Which form each element takes, from the note's description of the three paths: the vector path
    /// for `16·⌊count/16⌋`, whole groups of four after it, one at a time for the rest. It knows nothing
    /// about cursors or trip counts, which is what makes agreeing with it evidence.
    fn forms(count: usize, vector: bool) -> Vec<Form> {
        let done = if vector { count / 16 * 16 } else { 0 };
        let grouped = done + (count - done) / 4 * 4;
        (0..count)
            .map(|i| {
                if i < done {
                    Form::Vector
                } else if i < grouped && (i - done) % 4 == 2 {
                    Form::Lane2
                } else {
                    Form::Scalar
                }
            })
            .collect()
    }

    fn model(inp: &Inputs, forms: &[Form], one: f64, gb: f64, gc: f64) -> (Vec<f32>, Vec<f32>) {
        forms
            .iter()
            .enumerate()
            .map(|(i, &f)| element(f, one, inp.a[i], inp.b[i], inp.c[i], inp.d[i], gb, gc))
            .unzip()
    }

    // ------------------------------------------------------------------ sub_82B3D0A8

    #[test]
    fn the_four_wide_path_matches_the_per_lane_model() {
        // The game's own call: B and C misaligned, 128 elements, so 32 trips of the four-wide loop and
        // nothing else. This is the one path the harness has actually compared.
        let n = 128;
        let inp = inputs(n, 0x1234_5678);
        let mut g = guest();
        lay(&mut g, &inp, GAME);
        poison(&mut g, E, n + 4);
        poison(&mut g, F, n + 4);

        run(&mut g, n as u64, GAME, E).unwrap();

        let shape = forms(n, false);
        let (want_e, want_f) = model(&inp, &shape, 1.0, GB, GC);
        assert_eq!(get(&g, E, n), want_e);
        assert_eq!(get(&g, F, n), want_f);
        assert_eq!(
            words(&g, E + 4 * n as u32, 4),
            vec![POISON; 4],
            "past the E run"
        );
        assert_eq!(
            words(&g, F + 4 * n as u32, 4),
            vec![POISON; 4],
            "past the F run"
        );

        // Lane 2's association has to be visible in this data, or this test could not tell a port that
        // wrote all four lanes alike.
        let uniform: Vec<Form> = shape
            .iter()
            .map(|&f| if f == Form::Lane2 { Form::Scalar } else { f })
            .collect();
        assert_ne!(
            model(&inp, &uniform, 1.0, GB, GC).0,
            want_e,
            "lane 2 must be distinguishable"
        );
    }

    #[test]
    fn the_vector_path_blends_in_three_roundings_and_the_scalar_paths_in_one() {
        // Sixteen elements through each path, the same data. `F` is `mul, mul, add` on the vector path
        // and one `fmadds` on the scalar ones; the data has to make the two differ, and it is checked.
        let n = 16;
        let inp = inputs(n, 0x0BAD_F00D);
        let scalar_at = [A, B + 4, C + 4, D];

        let mut v = guest();
        lay(&mut v, &inp, ALIGNED);
        run(&mut v, n as u64, ALIGNED, E).unwrap();
        let mut s = guest();
        lay(&mut s, &inp, scalar_at);
        run(&mut s, n as u64, scalar_at, E).unwrap();

        let (ve, vf) = model(&inp, &forms(n, true), 1.0, GB, GC);
        let (se, sf) = model(&inp, &forms(n, false), 1.0, GB, GC);
        assert_eq!(
            (get(&v, E, n), get(&v, F, n)),
            (ve, vf.clone()),
            "vector path"
        );
        assert_eq!(
            (get(&s, E, n), get(&s, F, n)),
            (se, sf.clone()),
            "scalar path"
        );
        assert_ne!(
            vf, sf,
            "fused and unfused blends have to differ somewhere in this data"
        );
    }

    #[test]
    fn the_vector_path_needs_both_b_and_c_aligned() {
        // `clrlwi r11,r7,28` and `clrlwi r11,r8,28`, both against zero. The path is read off where the
        // E run lands: `E + 4` is misaligned, and only the vector path's `stvx128` floors it to `E`.
        let n = 16;
        let inp = inputs(n, 0x5EED_0001);
        for (bo, co) in [(0u32, 0u32), (0, 4), (4, 0), (8, 12)] {
            let at = [A, B + bo, C + co, D];
            let vector = bo == 0 && co == 0;
            assert_eq!(takes_vector_path(at[1], at[2]), vector);
            let mut g = guest();
            lay(&mut g, &inp, at);
            poison(&mut g, E, n + 4);

            run(&mut g, n as u64, at, E + 4).unwrap();

            let (want_e, _) = model(&inp, &forms(n, vector), 1.0, GB, GC);
            if vector {
                assert_eq!(
                    get(&g, E, n),
                    want_e,
                    "B+{bo} C+{co}: written from the floor"
                );
                assert_eq!(
                    g.u32(E + 64).unwrap(),
                    POISON,
                    "B+{bo} C+{co}: the top is not written"
                );
            } else {
                assert_eq!(g.u32(E).unwrap(), POISON, "B+{bo} C+{co}: nothing below E");
                assert_eq!(
                    get(&g, E + 4, n),
                    want_e,
                    "B+{bo} C+{co}: written at E exactly"
                );
            }
        }
    }

    #[test]
    fn the_vector_path_floors_the_four_pointers_it_never_checks() {
        // A, D, E and F reach `lvx128`/`stvx128` unchecked, and both mask the address down to 16
        // bytes: the inputs are read from the block below the pointer, and the outputs land there too,
        // clobbering the words below the pointer and leaving the top of the nominal run alone.
        let n = 16;
        let inp = inputs(n, 0x5EED_0002);
        let mut g = guest();
        lay(&mut g, &inp, ALIGNED); // the data sits at the floors
        g.set_u32(SP + ARG_F, F + 4).unwrap();
        poison(&mut g, E, n + 4);
        poison(&mut g, F, n + 4);

        let at = [A + 4, B, C, D + 8];
        run(&mut g, n as u64, at, E + 12).unwrap();

        let (want_e, want_f) = model(&inp, &forms(n, true), 1.0, GB, GC);
        assert_eq!(
            get(&g, E, n),
            want_e,
            "E from its floor, over A and D from theirs"
        );
        assert_eq!(get(&g, F, n), want_f, "F from its floor");
        assert_eq!(
            words(&g, E + 64, 3),
            vec![POISON; 3],
            "the top of E's nominal run"
        );
        assert_eq!(g.u32(F + 64).unwrap(), POISON, "the top of F's nominal run");
    }

    #[test]
    fn the_three_paths_split_the_count_where_the_note_says() {
        // 23 aligned: 16 vector, 4 four-wide, 3 one at a time. 20 aligned: the four-wide loop takes
        // exactly four, which is the only count where `>= 4` and `> 4` part company. 7 and 4 misaligned:
        // the same two boundaries without the vector path. Everything past the count stays poisoned.
        for (n, at) in [(23usize, ALIGNED), (20, ALIGNED), (7, GAME), (4, GAME)] {
            // Seed offset 2853, found by search: the first offset at which the lane-2 element's
            // four-wide form and the one-at-a-time form differ at every count below. The original
            // seed made them agree at count 23, so the guard at the end of this loop could not fire.
            let inp = inputs(n, 0x5EED_0100 + 2853 + n as u32);
            let mut g = guest();
            lay(&mut g, &inp, at);
            poison(&mut g, E, n + 4);
            poison(&mut g, F, n + 4);

            run(&mut g, n as u64, at, E).unwrap();

            let vector = at == ALIGNED;
            let shape = forms(n, vector);
            let (want_e, want_f) = model(&inp, &shape, 1.0, GB, GC);
            assert_eq!(get(&g, E, n), want_e, "count {n}: E");
            assert_eq!(get(&g, F, n), want_f, "count {n}: F");
            assert_eq!(
                g.u32(E + 4 * n as u32).unwrap(),
                POISON,
                "count {n}: nothing past E"
            );
            assert_eq!(
                g.u32(F + 4 * n as u32).unwrap(),
                POISON,
                "count {n}: nothing past F"
            );

            // The lane-2 element of the four-wide group has to be distinguishable from the tail form,
            // or a port that handed that group to the one-at-a-time loop would pass.
            let lane2 = shape.iter().position(|&f| f == Form::Lane2).unwrap();
            let as_tail = element(
                Form::Scalar,
                1.0,
                inp.a[lane2],
                inp.b[lane2],
                inp.c[lane2],
                inp.d[lane2],
                GB,
                GC,
            );
            assert_ne!(
                as_tail.0, want_e[lane2],
                "count {n}: element {lane2} must tell the forms apart"
            );
        }
    }

    #[test]
    fn a_non_positive_count_writes_nothing_and_reads_no_constant() {
        // `cmpw cr6,r27,r3 ; bge` is signed on the low word, so 0 and every negative count leave before
        // the `lfs` of the 1.0 — which is why the cell is removed here: reading it would be an `Err`.
        // -16 is the edge where `done` equals the count rather than exceeding it.
        for count in [
            0u64,
            0xFFFF_FFFF,
            0xFFFF_FFFC,
            0xFFFF_FFF0,
            0xFFFF_FFEF,
            0xDEAD_0000_0000_0000,
        ] {
            for at in [ALIGNED, GAME] {
                let mut g = Guest::single(BASE, 0x4000);
                g.set_u32(SP + ARG_F, F).unwrap();
                lay(&mut g, &inputs(16, 7), at);
                poison(&mut g, E, 20);
                poison(&mut g, F, 20);
                run(&mut g, count, at, E).unwrap();
                assert_eq!(words(&g, E, 20), vec![POISON; 20], "count {count:#x}");
                assert_eq!(words(&g, F, 20), vec![POISON; 20], "count {count:#x}");
            }
        }
    }

    #[test]
    fn the_scalar_path_reloads_b_c_and_d_after_the_e_store() {
        // E aliases B exactly. Every element stores E[j] — onto B[j] — and then reloads B, C and D, so
        // F[j] is blended from the value just stored rather than from the source. Seven elements: one
        // four-wide group (all four lanes) and three from the tail.
        let n = 7;
        let inp = inputs(n, 0x5EED_0003);
        let at = [A, B + 4, C + 4, D];
        let mut g = guest();
        lay(&mut g, &inp, at);
        poison(&mut g, F, n + 1);

        run(&mut g, n as u64, at, B + 4).unwrap();

        let (want_e, plain_f) = model(&inp, &forms(n, false), 1.0, GB, GC);
        let want_f: Vec<f32> = (0..n)
            .map(|j| {
                let (d, c, b) = (inp.d[j] as f64, inp.c[j] as f64, want_e[j] as f64);
                fp::fmadd_single(fp::sub_single(1.0, d), b, fp::mul_single(d, c)) as f32
            })
            .collect();
        assert_eq!(get(&g, B + 4, n), want_e, "E, stored over B");
        assert_eq!(get(&g, F, n), want_f, "F from the reloaded B");
        for j in 0..n {
            assert_ne!(
                want_f[j], plain_f[j],
                "element {j}: the reload has to be visible"
            );
        }
    }

    #[test]
    fn a_is_read_after_the_f_stores_on_the_vector_path() {
        // A aliases F exactly. All four F stores of a pass precede the first A load, so E is the weighted
        // mix plus the blend just written — not plus the accumulator the caller put there.
        let n = 16;
        let inp = inputs(n, 0x5EED_0004);
        let mut g = guest();
        lay(&mut g, &inp, [F, B, C, D]);

        run(&mut g, n as u64, [F, B, C, D], E).unwrap();

        let (plain_e, want_f) = model(&inp, &forms(n, true), 1.0, GB, GC);
        let fed_back = Inputs {
            a: want_f.clone(),
            b: inp.b.clone(),
            c: inp.c.clone(),
            d: inp.d.clone(),
        };
        let (want_e, _) = model(&fed_back, &forms(n, true), 1.0, GB, GC);
        assert_eq!(get(&g, F, n), want_f);
        assert_eq!(
            get(&g, E, n),
            want_e,
            "E accumulated onto the blend, not onto the old A"
        );
        assert_ne!(want_e, plain_e, "the ordering has to be visible");
    }

    #[test]
    fn the_scalar_paths_read_one_from_the_image_and_the_vector_path_builds_its_own() {
        // `lfs f0,-22460(r11)` on the scalar paths; `vupkd3d128` of zero on the vector path. Patching the
        // cell to 2.0 has to move the first and leave the second alone, and removing it has to break
        // only the first.
        let n = 16;
        let inp = inputs(n, 0x5EED_0005);
        let scalar_at = [A, B + 4, C + 4, D];

        let mut s = guest();
        s.put(ONE_CELL, 2.0f32.to_bits().to_be_bytes().to_vec());
        lay(&mut s, &inp, scalar_at);
        run(&mut s, n as u64, scalar_at, E).unwrap();
        let (want_e, want_f) = model(&inp, &forms(n, false), 2.0, GB, GC);
        assert_eq!((get(&s, E, n), get(&s, F, n)), (want_e.clone(), want_f));
        assert_ne!(
            model(&inp, &forms(n, false), 1.0, GB, GC).0,
            want_e,
            "the patch has to matter"
        );

        let mut v = guest();
        v.put(ONE_CELL, 2.0f32.to_bits().to_be_bytes().to_vec());
        lay(&mut v, &inp, ALIGNED);
        run(&mut v, n as u64, ALIGNED, E).unwrap();
        assert_eq!(
            get(&v, E, n),
            model(&inp, &forms(n, true), 1.0, GB, GC).0,
            "built, not loaded"
        );

        // Without the cell: sixteen aligned elements never reach the `lfs`; a misaligned call does.
        let mut bare = Guest::single(BASE, 0x4000);
        bare.set_u32(SP + ARG_F, F).unwrap();
        lay(&mut bare, &inp, ALIGNED);
        assert!(run(&mut bare, n as u64, ALIGNED, E).is_ok());
        lay(&mut bare, &inp, scalar_at);
        assert!(run(&mut bare, n as u64, scalar_at, E).is_err());
    }

    #[test]
    fn the_ninth_argument_is_read_off_the_callers_frame() {
        // `lwz r30,84(r1)`: repointing the slot alone has to move every F write.
        let n = 8;
        let inp = inputs(n, 0x5EED_0006);
        let alt = F + 0x100;
        let mut g = guest();
        lay(&mut g, &inp, GAME);
        g.set_u32(SP + ARG_F, alt).unwrap();
        poison(&mut g, F, n);

        run(&mut g, n as u64, GAME, E).unwrap();

        assert_eq!(
            get(&g, alt, n),
            model(&inp, &forms(n, false), 1.0, GB, GC).1
        );
        assert_eq!(
            words(&g, F, n),
            vec![POISON; n],
            "the default F was never written"
        );
    }

    #[test]
    fn a_misaligned_stack_pointer_is_refused_only_where_the_vector_loop_reads_its_splats() {
        // The splats are four `stfs` below r1 read back by one `lvx128`, which masks to 16 bytes. With r1
        // eight bytes off, that load would mix frame bytes nothing recorded into the gains, so the call is
        // refused, before anything is written. Where the loop does not run the splats are never read, and
        // the same r1 is fine.
        let sp = SP + 8;
        let inp = inputs(16, 0x5EED_0007);
        for (n, at, refused) in [
            (16usize, ALIGNED, true),
            (16, GAME, false),
            (15, ALIGNED, false),
        ] {
            let mut g = guest();
            g.set_u32(sp + ARG_F, F).unwrap();
            lay(&mut g, &inp, at);
            poison(&mut g, E, 16);
            poison(&mut g, F, 16);
            let got = crossfade(
                &mut g,
                n as u64,
                at[0] as u64,
                at[1] as u64,
                at[2] as u64,
                at[3] as u64,
                E as u64,
                sp,
                GB,
                GC,
            );
            if refused {
                assert_eq!(got.unwrap_err().message, vmx::SPLAT_FRAME_UNALIGNED);
                assert_eq!(
                    words(&g, E, 16),
                    vec![POISON; 16],
                    "a refused call writes nothing"
                );
                assert_eq!(words(&g, F, 16), vec![POISON; 16]);
            } else {
                got.unwrap();
                assert_eq!(
                    get(&g, E, n),
                    model(&inp, &forms(n, false), 1.0, GB, GC).0,
                    "count {n}"
                );
            }
        }
    }

    #[test]
    fn a_gain_that_is_not_a_single_stays_in_double_on_the_scalar_paths() {
        // The scalar paths multiply by the guest FPR as it stands; the vector path narrows it first. No
        // recorded call can show this — the dispatcher loads both gains with `lfs` — so only this test
        // separates a port that narrowed the gain on the scalar side too.
        // 1.5 + 0.99 * 2^-24: not a single, and not near one. Two earlier choices, 1 + 2^-30 and then
        // 1 + 2^-25, could never have worked: every product here is a single times the gain, a single
        // already sits on the rounding grid, and a gain within half an ulp of 1.0 moves the product
        // by less than half an ulp, so it rounds straight back. Measured: 20,000 seeds, not one that
        // told the two forms apart. This gain rounds nearly half an ulp away and its products are
        // off the grid, so 5 of the 16 outputs differ between the double and the narrowed forms.
        let gc = f64::from_bits(0x3FF8_0000_0FD7_0A3D);
        assert_ne!(gc, gc as f32 as f64);
        let n = 16;
        let inp = inputs(n, 0x5EED_0008);
        let at = [A, B + 4, C + 4, D];
        let mut g = guest();
        lay(&mut g, &inp, at);
        crossfade(
            &mut g,
            n as u64,
            A as u64,
            at[1] as u64,
            at[2] as u64,
            D as u64,
            E as u64,
            SP,
            GB,
            gc,
        )
        .unwrap();
        let want = model(&inp, &forms(n, false), 1.0, GB, gc).0;
        assert_eq!(get(&g, E, n), want);
        assert_ne!(
            model(&inp, &forms(n, false), 1.0, GB, gc as f32 as f64).0,
            want,
            "must be visible"
        );
    }

    // ------------------------------------------------------------------ sub_82B3D4F8

    /// The two gains and a descriptor with `select` at `+8`.
    fn dispatcher(g: &mut Guest, b: u32, select: u32) {
        put(g, GAINS + GAIN0, &[GB as f32, GC as f32]);
        for (slot, value) in [
            (DESC_A, A),
            (DESC_B, b),
            (DESC_C, select),
            (DESC_D, D),
            (DESC_E, E),
            (DESC_F, F),
        ] {
            g.set_u32(DESC + slot, value).unwrap();
        }
    }

    #[test]
    fn a_zero_selector_runs_the_one_gain_kernel_on_the_descriptor() {
        // y = desc[0], x = desc[4], z = desc[16], w = desc[20], gain = gains[16]. Compared against the
        // kernel called directly with that mapping, so a swapped pair or the wrong gain is caught.
        let n = 20;
        let inp = inputs(n, 0x5EED_0010);
        let mut g = guest();
        lay(&mut g, &inp, ALIGNED);
        dispatcher(&mut g, B, 0);
        poison(&mut g, E, n + 4);
        poison(&mut g, F, n + 4);
        let mut want = g.clone();
        dsp::scale_add::scale_add_with_copy(&mut want, n as u32, A, B, E, F, GB).unwrap();

        run_mix(&mut g, GAINS, n as u64, DESC, SP).unwrap();

        assert_eq!(words(&g, E, n + 4), words(&want, E, n + 4));
        assert_eq!(words(&g, F, n + 4), words(&want, F, n + 4));
        assert_ne!(
            words(&g, E, n),
            vec![POISON; n],
            "the kernel has to have run"
        );
    }

    #[test]
    fn a_nonzero_selector_is_the_second_source_and_runs_the_crossfade() {
        // C = desc[8] — the selector word itself — with gains[16] as gain_b and gains[20] as gain_c.
        let n = 20;
        let inp = inputs(n, 0x5EED_0011);
        let mut g = guest();
        lay(&mut g, &inp, GAME);
        dispatcher(&mut g, GAME[1], GAME[2]);
        poison(&mut g, E, n + 4);
        poison(&mut g, F, n + 4);
        let mut want = g.clone();
        want.set_u32(FRAME + ARG_F, F).unwrap();
        let direct = |h: &mut Guest, gb: f64, gc: f64| {
            crossfade(
                h,
                n as u64,
                A as u64,
                GAME[1] as u64,
                GAME[2] as u64,
                D as u64,
                E as u64,
                FRAME,
                gb,
                gc,
            )
        };
        let mut swapped = want.clone();
        direct(&mut want, GB, GC).unwrap();
        direct(&mut swapped, GC, GB).unwrap();

        run_mix(&mut g, GAINS, n as u64, DESC, SP).unwrap();

        assert_eq!(words(&g, E, n + 4), words(&want, E, n + 4));
        assert_eq!(words(&g, F, n + 4), words(&want, F, n + 4));
        assert_ne!(
            words(&want, E, n),
            words(&swapped, E, n),
            "swapped gains have to be visible"
        );
    }

    #[test]
    fn the_sixth_pointer_is_stored_at_84_of_the_new_frame() {
        // `stw r5,84(r1)` after `stwu r1,-96(r1)`: the slot is in this call's frame, and the kernel reads
        // it from there. The one-gain path never stores it. Both write the back chain.
        for select in [0u32, GAME[2]] {
            let mut g = guest();
            lay(&mut g, &inputs(16, 1), GAME);
            dispatcher(&mut g, GAME[1], select);
            poison(&mut g, FRAME, (FRAME_BYTES / 4) as usize);

            run_mix(&mut g, GAINS, 16, DESC, SP).unwrap();

            assert_eq!(
                g.u32(FRAME).unwrap(),
                SP,
                "selector {select:#x}: the back chain"
            );
            let slot = g.u32(FRAME + ARG_F).unwrap();
            if select == 0 {
                assert_eq!(slot, POISON, "the one-gain path stores no ninth argument");
            } else {
                assert_eq!(slot, F, "desc[20] -> 84(r1)");
            }
        }
    }

    #[test]
    fn the_second_gain_is_read_only_on_the_two_gain_path() {
        // `lfs f2,20(r6)` is after the branch. A gain block whose `+20` is unmapped has to serve the
        // one-gain path and fail the other.
        let gains = 0x5000_0000 - GAIN0;
        for (select, ok) in [(0u32, true), (GAME[2], false)] {
            let mut g = guest();
            g.put(0x5000_0000, (GB as f32).to_bits().to_be_bytes().to_vec());
            lay(&mut g, &inputs(16, 2), GAME);
            dispatcher(&mut g, GAME[1], select);
            assert_eq!(
                run_mix(&mut g, gains, 16, DESC, SP).is_ok(),
                ok,
                "selector {select:#x}"
            );
        }
    }
}
