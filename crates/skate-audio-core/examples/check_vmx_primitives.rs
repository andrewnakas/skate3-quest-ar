//! Replay `crate::vmx` against the recorded C++ results in `probe/vmx128/`.
//!
//! This is the crate's one piece of **replayed** green outside `system.rs`/`player.rs`/`buffers.rs`:
//! the reference is not a model written here, it is RexGlue's own lowering compiled by the recomp's
//! own compiler and run over adversarial inputs, with the answers on disk.
//!
//! ```sh
//! probe/vmx128/run.sh                              # regenerate vectors.bin and the .bin results
//! cargo run --release --example check_vmx_primitives
//! ```
//!
//! It is an example rather than a unit test because the recorded files are not in the tree —
//! `probe/vmx128/.gitignore` excludes `vectors.bin` and `*_results*.bin`, since they are build
//! products of a C++ toolchain. A `cargo test` that silently skipped when they were absent would be
//! exactly the vacuous green this project keeps warning about, and one that failed when they were
//! absent would make the suite environment-dependent. So it lives here, and its result is a number
//! to quote rather than a gate.
//!
//! ## What it covers
//!
//! The 45 operations of `probe/vmx128/compare.py`'s table, over 158 adversarial vectors — denormals,
//! NaN payloads, ±0, ±inf, rounding boundaries, conversion edges, the `rsqrt` table sweep and 512
//! xorshift patterns — in both flush-to-zero states. 56,880 lane comparisons per reference build.
//!
//! Four of the ops carry `docs/vmx128-exactness.md` **rule 4**: `vaddfp128`, `vmulfp128`, `vmaddfp`
//! and `vnmsubfp` are not NaN-commutative, and the operand slot that wins is chosen by register
//! allocation rather than by the source. The `_pinned` reference builds put `a` in the winning slot
//! deliberately; the `_plain` ones inherit whatever the optimiser picked, and the cookbook records
//! that **GCC and clang diverge from Rust on different pairs there**. So a `_plain` mismatch on
//! exactly those four ops is the documented state of the world and not a fault in this crate; a
//! `_pinned` mismatch would be a real finding. The output labels which is which.
//!
//! What it does **not** cover is listed in `vmx`'s module documentation: `vrfin128`/`vrfip128`/
//! `vrfim128`, every store, `dcbzl`, `lvrx128`, and `vspltw128` at three of its four immediates.
//! Those have unit tests against a hand-written model instead, which is a weaker instrument.

use skate_audio_core::vmx::{self, Fpscr};
use skate_audio_core::{Guest, Segment};
use std::arch::x86_64::*;

/// `compare.py`'s `OPS`, in order. The result file is indexed by this, so it may not be reordered.
const OPS: [&str; 45] = [
    "vaddfp128",
    "vsubfp128",
    "vmulfp128",
    "vmaddfp",
    "vnmsubfp",
    "vmaxfp128",
    "vminfp128",
    "vrefp",
    "vrsqrtefp",
    "vrfiz128",
    "vmsum3fp128",
    "vmsum4fp128",
    "vcmpeqfp128",
    "vcmpgefp128",
    "vcmpgtfp128",
    "vexptefp128",
    "vlogefp128",
    "vcsxwfp128_0",
    "vcsxwfp128_15",
    "vcuxwfp128_0",
    "vcfpsxws128",
    "vcfpuxws128",
    "vperm128",
    "vsel",
    "vand128",
    "vandc128",
    "vor128",
    "vnor128",
    "vxor128",
    "vaddshs",
    "vaddsws",
    "vsubsws",
    "vadduwm",
    "vpkshus128",
    "vpkswss128",
    "vsraw128",
    "vslw128",
    "vsrw128",
    "vupkhsb128",
    "vmrghw128",
    "vmrglw128",
    "vspltw128",
    "vsldoi128",
    "lvx128_swap",
    "lvlx128_swap5",
];

/// The four operations `docs/vmx128-exactness.md` rule 4 applies to.
const RULE_4: [&str; 4] = ["vaddfp128", "vmulfp128", "vmaddfp", "vnmsubfp"];

/// A guest map for the two memory ops, so the check runs the real `vmx::lvx128`/`lvlx128` rather
/// than a re-spelling of their innards.
const STAGE: u32 = 0x4000_0000;

#[target_feature(enable = "sse4.1,fma")]
unsafe fn apply(index: usize, g: &mut Guest, a: __m128i, b: __m128i, c: __m128i) -> __m128i {
    let (ps, si) = (_mm_castsi128_ps, _mm_castps_si128);
    unsafe {
        match index {
            0 => si(vmx::vaddfp(ps(a), ps(b))),
            1 => si(vmx::vsubfp(ps(a), ps(b))),
            2 => si(vmx::vmulfp(ps(a), ps(b))),
            3 => si(vmx::vmaddfp(ps(a), ps(b), ps(c))),
            4 => si(vmx::vnmsubfp(ps(a), ps(b), ps(c))),
            5 => si(vmx::vmaxfp(ps(a), ps(b))),
            6 => si(vmx::vminfp(ps(a), ps(b))),
            7 => si(vmx::vrefp(ps(a))),
            8 => si(vmx::vrsqrtefp(ps(a))),
            9 => si(vmx::vrfiz(ps(a))),
            10 => si(vmx::vmsum3fp(ps(a), ps(b))),
            11 => si(vmx::vmsum4fp(ps(a), ps(b))),
            12 => si(vmx::vcmpeqfp(ps(a), ps(b))),
            13 => si(vmx::vcmpgefp(ps(a), ps(b))),
            14 => si(vmx::vcmpgtfp(ps(a), ps(b))),
            15 => si(vmx::vexptefp(ps(a))),
            16 => si(vmx::vlogefp(ps(a))),
            17 => si(vmx::vcsxwfp::<0>(a)),
            18 => si(vmx::vcsxwfp::<15>(a)),
            19 => si(vmx::vcuxwfp::<0>(a)),
            20 => vmx::vcfpsxws(ps(a)),
            21 => vmx::vcfpuxws(ps(a)),
            22 => vmx::vperm(a, b, c),
            23 => vmx::vsel(a, b, c),
            24 => vmx::vand(a, b),
            25 => vmx::vandc(a, b),
            26 => vmx::vor(a, b),
            27 => vmx::vnor(a, b),
            28 => vmx::vxor(a, b),
            29 => vmx::vaddshs(a, b),
            30 => vmx::vaddsws(a, b),
            31 => vmx::vsubsws(a, b),
            32 => vmx::vadduwm(a, b),
            33 => vmx::vpkshus(a, b),
            34 => vmx::vpkswss(a, b),
            35 => vmx::vsraw::<5>(a),
            36 => vmx::vslw::<5>(a),
            37 => vmx::vsrw::<5>(a),
            38 => vmx::vupkhsb(a),
            39 => vmx::vmrghw(a, b),
            40 => vmx::vmrglw(a, b),
            41 => vmx::vspltw128::<{ vmx::SPLAT_W1 }>(a), // the probe's 0xAA
            42 => vmx::vsldoi::<4>(a, b),
            43 => {
                // The probe applies the row-0 mask to a register. Staging those bytes in the guest
                // map and loading them back is the same permutation *and* runs the real accessor.
                let mut raw = [0u8; 16];
                _mm_storeu_si128(raw.as_mut_ptr() as *mut __m128i, a);
                g.set_span(STAGE, &raw).unwrap();
                vmx::lvx128(g, STAGE).unwrap()
            }
            44 => {
                let mut raw = [0u8; 16];
                _mm_storeu_si128(raw.as_mut_ptr() as *mut __m128i, a);
                g.set_span(STAGE, &raw).unwrap();
                vmx::lvlx128(g, STAGE + 5).unwrap()
            }
            _ => unreachable!(),
        }
    }
}

fn main() {
    if !vmx::supported() {
        eprintln!("this CPU lacks sse4.1 or fma; nothing to check");
        std::process::exit(2);
    }

    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../probe/vmx128");
    let vectors = match std::fs::read(format!("{root}/vectors.bin")) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{root}/vectors.bin: {e}");
            eprintln!(
                "run probe/vmx128/run.sh first — it generates the vectors and the reference."
            );
            std::process::exit(2);
        }
    };

    let n = u32::from_le_bytes(vectors[0..4].try_into().unwrap()) as usize;
    let lanes: Vec<u32> = (0..n)
        .map(|i| u32::from_le_bytes(vectors[4 + i * 4..8 + i * 4].try_into().unwrap()))
        .collect();
    let nvec = n / 4;

    // Compute this crate's answers in the probe's own layout: ftz outer, then op, then vector.
    let mut g = Guest::from_segments(vec![Segment {
        base: STAGE,
        bytes: vec![0u8; 64],
    }]);
    let mut ours = Vec::with_capacity(2 * OPS.len() * nvec * 16);
    let mut fpscr = Fpscr::capture();
    for ftz in 0..2 {
        if ftz == 0 {
            // The probe clears FZ|DAZ outright for its first pass; `Fpscr` never does, because
            // RexGlue never does either. Drive MXCSR directly so both states are covered.
            vmx::set_mxcsr(vmx::get_mxcsr() & !(vmx::FLUSH_ZERO | vmx::DENORMALS_ZERO));
        } else {
            fpscr.enable_flush_mode_unconditional();
        }
        for (index, _name) in OPS.iter().enumerate() {
            for v in 0..nvec {
                unsafe {
                    let a = _mm_loadu_si128(lanes.as_ptr().add(v * 4) as *const __m128i);
                    let b =
                        _mm_loadu_si128(lanes.as_ptr().add(((v + 1) % nvec) * 4) as *const __m128i);
                    let c =
                        _mm_loadu_si128(lanes.as_ptr().add(((v + 2) % nvec) * 4) as *const __m128i);
                    let r = apply(index, &mut g, a, b, c);
                    let mut out = [0u8; 16];
                    _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, r);
                    ours.extend_from_slice(&out);
                }
            }
        }
    }
    drop(fpscr);

    println!(
        "ops={} vectors={nvec} ftz states=2 -> {} lane comparisons per reference",
        OPS.len(),
        OPS.len() * nvec * 4 * 2
    );

    let mut any = false;
    let mut hard_failure = false;
    for build in ["clang20_pinned", "gcc_pinned", "clang20_plain", "gcc_plain"] {
        let path = format!("{root}/cpp_results_{build}.bin");
        let Ok(reference) = std::fs::read(&path) else {
            println!("\n{build:<16} absent");
            continue;
        };
        any = true;
        if reference.len() != ours.len() {
            println!(
                "\n{build:<16} SIZE MISMATCH {} vs {}",
                reference.len(),
                ours.len()
            );
            hard_failure = true;
            continue;
        }
        let pinned = build.ends_with("_pinned");
        let stride = nvec * 16;
        let mut bad: Vec<(&str, usize, usize)> = Vec::new();
        for ftz in 0..2 {
            for (index, name) in OPS.iter().enumerate() {
                let off = (ftz * OPS.len() + index) * stride;
                let mut lanes_differing = 0;
                for i in (0..stride).step_by(4) {
                    if reference[off + i..off + i + 4] != ours[off + i..off + i + 4] {
                        lanes_differing += 1;
                    }
                }
                if lanes_differing > 0 {
                    bad.push((name, ftz, lanes_differing));
                }
            }
        }

        if bad.is_empty() {
            println!("\n{build:<16} ALL 45 OPS BIT-IDENTICAL");
            continue;
        }
        let ok = OPS.len() * 2 - bad.len();
        println!(
            "\n{build:<16} {ok}/{} (op, ftz) pairs bit-identical",
            OPS.len() * 2
        );
        for (name, ftz, count) in &bad {
            let tag = if RULE_4.contains(name) && !pinned {
                "rule 4, expected on a _plain build"
            } else {
                "UNEXPECTED"
            };
            if tag == "UNEXPECTED" {
                hard_failure = true;
            }
            println!(
                "   {name:<16} ftz={ftz}  {count}/{} lanes differ   [{tag}]",
                nvec * 4
            );
        }
    }

    if !any {
        eprintln!("\nno recorded C++ results found; run probe/vmx128/run.sh");
        std::process::exit(2);
    }
    if hard_failure {
        eprintln!("\nFAIL: a mismatch that rule 4 does not account for");
        std::process::exit(1);
    }
    println!("\nOK");
}
