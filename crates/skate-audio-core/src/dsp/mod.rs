//! The DSP kernels: the vector half of the mixer, on top of [`crate::vmx`].
//!
//! A module per kernel family, which is what the crate README reserved this directory for ("one
//! file per plug-in family"). These are the functions that actually move samples, and they are
//! where the whole of `docs/vmx128-exactness.md` has to be applied rather than quoted.
//!
//! | module | guest | role | `docs/ports.md` | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`sine`] | `sub_824531C8` | four-lane sine, range reduction plus an 11-term odd polynomial | verified | 9,572,672 | 9,906,721 |
//! | [`scale`] | `sub_82B3BED8` | `dst[i] = src[i] * scale` | verified | 2,843,558 | 4,411,988 |
//! | [`scale`] | `sub_82B44B20` | `dst[i] += src[i] * scale` | verified | 3,738,884 | 6,110,212 |
//! | [`gain_ramp`] | `sub_82B3C098` | gain-ramped copy of 256 singles | verified | 2,126,778 | 3,023,068 |
//! | [`biquad`] | `sub_82B43AF8` | a biquad over a run of singles, eight a pass | verified | 1,152,252 | 1,592,578 |
//! | [`resample`] | `sub_82B43FB8` | linear interpolation walked by a 16.16 phase | verified | 404,358 | 748,902 |
//! | [`clip`] | `sub_82B22678` | hard clipper: clamp 256 samples a channel, then swap the pair | verified | 49,325 | 76,864 |
//! | [`allpass`] | `sub_82B389A0` | four-lane one-multiply allpass over an unaligned tap, accumulated at a gain | verified | 50,570 | 59,894 |
//! | [`scale_add`] | `sub_82B3CF58` | `z[i] = x[i]·gain + y[i]`, and a copy of `x` into `w` | verified | 179,218 | 339,926 |
//!
//! The last two are **scalar**, which is worth saying in a directory named for vector kernels: they
//! are here because they are DSP the mixer runs per block, not because they use [`crate::vmx`]'s
//! operation table. What they do take from `vmx` is [`crate::vmx::Fpscr`] — the guest's flush mode
//! is carried on the scalar side too (`rex/ppc/context.h` sets `FlushMask` when it initialises the
//! host FPU), and `biquad` adds a rodata bias to every feed-forward sum precisely because denormals
//! reach it.
//!
//! Every one of those `.inc` bodies leads with `// STATUS: verified` — checked before
//! translating, because a header leading with `thin`, `partial` or `gate-` means the C++ was not
//! compared on enough calls, or not on the path being translated, and would not be a reference at
//! all. `sub_82B373C8` is the counter-example the README already names: verified but `thin`, and
//! deliberately left out.
//!
//! ## What the green here means, and what it does not
//!
//! These are **unit-tested against a verified reference**, the crate README's second kind of green
//! — not replayed. The C++ each one is translated from was compared call-for-call against the
//! original recompiled code under the shadow harness, on real inputs, at zero divergence; the call
//! counts above are that evidence. The Rust has no recorded per-call vectors of its own, because
//! the harness records none for these functions, so there is nothing to replay and no number to
//! quote for the translation itself. What that buys is a small search space: a fault here is a
//! transcription error, not a misreading of the engine. What it does not buy is a number.
//!
//! The one exception worth naming: the *primitives* underneath, in [`crate::vmx`], **can** be
//! replayed, against `probe/vmx128/`'s recorded C++ results —
//! `cargo run --example check_vmx_primitives`. That covers the arithmetic these kernels are built
//! from, at 45 operations by 158 adversarial vectors by two FTZ states, but it says nothing about
//! whether they are composed in the right order. Composition is what the unit tests below are for.
//!
//! ## Rule 4, once, for all four
//!
//! Every float operation in these kernels is written with its operands in the lifted order. That is
//! necessary and it is not sufficient: `docs/vmx128-exactness.md` rule 4 establishes that when two
//! operands are both NaN the surviving payload is chosen by **register allocation**, in clang-20 —
//! the recomp's own compiler — as much as in GCC, so it cannot be derived from source in either
//! language. If a NaN ever reaches the source buffer or the gain of any kernel here, its output is
//! not predictable from this code or from the C++, and a divergence that appears on some calls and
//! not others should be checked for NaN in the input run before anything else. Nothing in this
//! directory may be reasoned about on that point; it stays bit-checked or it stays unknown.

pub mod allpass;
pub mod biquad;
pub mod clip;
pub mod gain_ramp;
pub mod ramps;
pub mod resample;
pub mod scale;
pub mod scale_add;
pub mod sine;
