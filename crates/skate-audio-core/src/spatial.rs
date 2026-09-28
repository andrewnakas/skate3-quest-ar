//! The spatial layer: a source's position becomes a gain per speaker.
//!
//! Five verified bodies that between them own that transformation. They are one module because they
//! are one chain over two shared structures, and reading any of them alone hides what the offsets
//! mean:
//!
//! ```text
//!   angle, depth ──► place_panner ──► clamp_to_unit_disc ──► the 16-byte source record
//!                                                                │
//!                          the config object (r3) ───────────────┤
//!                                                                ▼
//!                                        pan_distance ──► the speaker-gain array (r6)
//!                                        add_angular  ──►   "     "     "      "
//!                                        scale_gains  ──►   "     "     "      "
//! ```
//!
//! | function | guest | `docs/ports.md` | lifted lines | calls/boot | calls/play |
//! |---|---|---|---|---|---|
//! | [`clamp_to_unit_disc`] | `sub_82B453D8` | verified | 66 | 292,509 | 371,733 |
//! | [`place_panner`] | `sub_82B269C0` | verified | 109 | 218,583 | 273,708 |
//! | [`pan_distance`] | `sub_82B454B8` | verified | 409 | 219,648 | 280,497 |
//! | [`add_angular`] | `sub_82B45788` | verified | 608 | 219,648 | 280,497 |
//! | [`scale_gains`] | `sub_82B45B60` | verified | 151 | 219,648 | 280,497 |
//!
//! Every one of those `.inc` headers leads with `// STATUS: verified`, and `docs/ports.md` agrees
//! with all five — checked before translating, because a `thin` or `gate-` header means the C++ was
//! never compared on enough calls, or not on the path being translated, and is not a reference.
//!
//! The three that take a gain array are called in sequence over the same array by the same two
//! callers, `sub_82B45440` and `sub_82B460A0`; the second loops with `r6 += 32` and `r4 += 16`, which
//! is where the 32-byte gain array and the 16-byte source record come from. The two 5.1/7.1 speaker
//! counts and the four speaker indices at `+172` are read by all three.
//!
//! ## What the green here means
//!
//! **Unit-tested against a verified reference**, the crate README's second kind of green. The C++
//! body each of these was translated from was compared call-for-call against the original under the
//! shadow harness, on the real inputs those call counts come from, at zero divergence. The Rust has
//! no recorded vectors of its own yet, and two of these cannot have any until `mathlib`'s sine and
//! cosine are ported — see [`add_angular`] and [`place_panner`].
//!
//! ## Reads no `Windows()` declares — which is what makes some of these unreplayable today
//!
//! A read the C++ `Windows()` does not declare is not recorded, so a vector that reaches it cannot
//! be replayed against this crate; feeding fabricated bytes instead would turn a failure into a
//! meaningless pass. Three of the five reach such a cell, and every one is a *rodata constant*, so
//! the fix is one `spec.read` line in the `.inc` and costs nothing against the window budget:
//!
//! | function | undeclared read | why it is reached |
//! |---|---|---|
//! | [`place_panner`] | [`ONE_SINGLE`], [`SNAP_SINGLE`] | its callee `sub_82B453D8` reads both |
//! | [`add_angular`] | [`ZERO_SINGLE`] | sector 1's initial centre share |
//! | [`add_angular`] | `mathlib::POOL_STEP_DOWN`, `mathlib::POOL_INTEGRAL_MAGNITUDE` | `sub_82F4DE80` reads both |
//!
//! [`ZERO_SINGLE`] is a difference from the C++ rather than from the original: `sub_82B45788`'s port
//! writes `double centre = 0.0;` with the lifted `lfs f12,23056(r11)` in the comment beside it. The
//! original loads the cell, so this port loads it too, per the crate's rule that guest constants are
//! read live. On the measured image the cell is `0.0` and the two agree exactly.
//!
//! ## The constants, measured
//!
//! Every cell below is read live through the [`Guest`] map, and every address is computed from its
//! `lis` immediate and asserted at compile time. The values are from the validated image dump
//! (`probe/harness/out/image/`) and are recorded here because two of them settle what these
//! functions are *doing*, not merely that the addresses are right:
//!
//! | cell | address | measured | what it makes the code mean |
//! |---|---|---|---|
//! | [`ONE_SINGLE`] | `0x8231A844` | 1.0 | |
//! | [`SNAP_SINGLE`] | `0x820ED800` | 0.999 | the unit-disc snap window |
//! | [`ZERO_SINGLE`] | `0x82165A10` | 0.0 | |
//! | [`HALF_SINGLE`] | `0x8209975C` | 0.5 | `weight = 1 - 0.5·distance` |
//! | [`EPSILON_SINGLE`] | `0x821A0318` | 0.0005 | the front/back snap band |
//! | [`TURNS_PER_RADIAN`] | `0x822F8904` | 0.15915494 | **1/2π** |
//! | [`RADIANS_PER_TURN`] | `0x820B411C` | 6.2831855 | **2π** |
//! | [`DEGREES_TO_RADIANS`] | `0x822F8A6C` | −0.017453292 | **−π/180** |
//! | [`HALF_TURN`] | `0x82060C44` | 3.1415927 | **π** |
//!
//! The first pair settles [`add_angular`]'s reduction: multiply by `1/2π`, take the floor, keep the
//! fraction, multiply by `2π`. That is an angle **wrapped into one turn**, and the sector bound at
//! `+60` is added before the wrap and subtracted after it, so the seven sectors partition
//! `[-bound0, 2π - bound0)`. The second pair settles [`place_panner`]'s argument: `f1` is an angle in
//! **degrees**, negated into radians, and the `depth <= 0` path adds half a turn — a reflection
//! through the origin, which is what a negative radius means.
//!
//! ## What no test here can catch
//!
//! Three things these bodies do are reproduced because the originals do them and **not** because
//! anything in this crate would notice their absence. Each was measured the way the crate's other
//! such notes were — by making the change and watching all 275 tests stay green — rather than
//! argued from the code:
//!
//! - **[`pan_distance`] reloads all four indices between the four stores.** Replacing them with the
//!   entry values leaves the suite green. The two *count* reloads on the same path are pinned, by
//!   `the_count_is_reloaded_between_the_index_stores_and_the_fixed_ones`; the index reloads are not,
//!   because an index that changed mid-call would have to be overwritten by one of this function's
//!   own float stores, and the resulting `4·index` is then a wild address rather than another slot.
//!   The C++ `Windows()` refuses that layout outright, so nothing is known about it in either
//!   language.
//! - **[`add_angular`] reads the second index of sectors 2, 5 and 7 *after* the first store.**
//!   Hoisting sector 2's read above its store leaves the suite green, for the same reason.
//! - **[`clamp_to_unit_disc`] stores x and y before it computes the length.** Moving both stores
//!   after the `fmadds` leaves the suite green: the only input that could tell them apart is an
//!   `out` that overlaps the rodata cell the load in between reads, which is not a layout any
//!   caller can produce.
//!
//! ## Rule 4, once, for all five
//!
//! Every float operation is written with its operands in the lifted order, including the commutative
//! ones — `mul_single(depth, gain172)` here and `mul_single(rear180, depth)` two lines later are the
//! two orders the original uses and they are not normalised. `docs/vmx128-exactness.md` rule 4
//! establishes that with two NaN operands the surviving payload is chosen by register allocation, in
//! clang-20 as much as in GCC, so it cannot be derived from source in either language. If a NaN ever
//! reaches a speaker position or a source record, these outputs are not predictable from this code
//! or from the C++.
//!
//! Nothing in `docs/rw_audio_structs.h` names either structure. Every offset below is the raw offset
//! plus the use the lifted bodies make of it, and the speaker-table reading is an inference from the
//! offset arithmetic that `probe/ports/notes/sub_82B454B8.md` already flags as such.

use crate::mathlib::{self, Trig};
use crate::vmx::Fpscr;
use crate::{Guest, Result, fp};

// ------------------------------------------------------------------------- the rodata cells
//
// `((lis_imm & 0xFFFF) << 16) + offset`, computed rather than read off the disassembly: one misread
// digit in `sub_82B2FE00` cost this project its first shadow divergence.

const LIS_82320000: u32 = ((-32206i32 as u32) & 0xFFFF) << 16;
const LIS_820F0000: u32 = ((-32241i32 as u32) & 0xFFFF) << 16;
const LIS_82160000: u32 = ((-32234i32 as u32) & 0xFFFF) << 16;
const LIS_820A0000: u32 = ((-32246i32 as u32) & 0xFFFF) << 16;
const LIS_821A0000: u32 = ((-32230i32 as u32) & 0xFFFF) << 16;
const LIS_82300000: u32 = ((-32208i32 as u32) & 0xFFFF) << 16;
const LIS_820B0000: u32 = ((-32245i32 as u32) & 0xFFFF) << 16;
const LIS_82060000: u32 = ((-32250i32 as u32) & 0xFFFF) << 16;
const _: () = assert!(LIS_82320000 == 0x8232_0000, "lis -32206");
const _: () = assert!(LIS_820F0000 == 0x820F_0000, "lis -32241");
const _: () = assert!(LIS_82160000 == 0x8216_0000, "lis -32234");
const _: () = assert!(LIS_820A0000 == 0x820A_0000, "lis -32246");
const _: () = assert!(LIS_821A0000 == 0x821A_0000, "lis -32230");
const _: () = assert!(LIS_82300000 == 0x8230_0000, "lis -32208");
const _: () = assert!(LIS_820B0000 == 0x820B_0000, "lis -32245");
const _: () = assert!(LIS_82060000 == 0x8206_0000, "lis -32250");

/// `lfs f13,-22460(r11)` — measured 1.0. The clamp target, the sector-normalisation numerator's
/// comparand, and the `1 -` in every distance weight.
pub const ONE_SINGLE: u32 = LIS_82320000.wrapping_add(-22460i32 as u32);
/// `lfs f12,-10240(r11)` — measured 0.999. A squared length above this snaps to 1.0.
pub const SNAP_SINGLE: u32 = LIS_820F0000.wrapping_add(-10240i32 as u32);
/// `lfs f8,23056(r5)` — measured 0.0.
pub const ZERO_SINGLE: u32 = LIS_82160000.wrapping_add(23056);
/// `lfs f12,-26788(r31)` — measured 0.5, the slope of the distance weight.
pub const HALF_SINGLE: u32 = LIS_820A0000.wrapping_add(-26788i32 as u32);
/// `lfs f6,792(r11)` — measured 0.0005, the band inside which a front or back share snaps to zero.
pub const EPSILON_SINGLE: u32 = LIS_821A0000.wrapping_add(792);

/// The constant pool at `0x822F8600` that [`crate::mix`] also reads, reached as
/// `lis -32208 ; addi -31232`.
pub const POOL: u32 = LIS_82300000.wrapping_add(-31232i32 as u32);
/// `lfs f0,772(r10)` — measured 0.15915494, i.e. **1/2π**: radians into turns.
pub const TURNS_PER_RADIAN: u32 = POOL + 772;
/// `lfs f0,1132(r10)` — measured −0.017453292, i.e. **−π/180**: degrees into radians, negated.
pub const DEGREES_TO_RADIANS: u32 = POOL + 1132;
/// `lfs f29,16668(r9)` — measured 6.2831855, i.e. **2π**: turns back into radians, and the value the
/// two reflected sector bounds are subtracted from.
pub const RADIANS_PER_TURN: u32 = LIS_820B0000.wrapping_add(16668);
/// `lfs f13,3140(r11)` — measured 3.1415927, i.e. **π**: the bias a non-positive depth adds.
pub const HALF_TURN: u32 = LIS_82060000.wrapping_add(3140);

const _: () = assert!(ONE_SINGLE == 0x8231_A844);
const _: () = assert!(SNAP_SINGLE == 0x820E_D800);
const _: () = assert!(ZERO_SINGLE == 0x8216_5A10);
const _: () = assert!(HALF_SINGLE == 0x8209_975C);
const _: () = assert!(EPSILON_SINGLE == 0x821A_0318);
const _: () = assert!(POOL == 0x822F_8600);
const _: () = assert!(TURNS_PER_RADIAN == 0x822F_8904);
const _: () = assert!(DEGREES_TO_RADIANS == 0x822F_8A6C);
const _: () = assert!(RADIANS_PER_TURN == 0x820B_411C);
const _: () = assert!(HALF_TURN == 0x8206_0C44);
// Two cross-checks against modules that reached the same cells from different lifted lines. If
// either of these ever fails, one of the two address computations is wrong.
const _: () = assert!(POOL == crate::mix::POOL, "the same pool sub_82B34E08 reads");
const _: () = assert!(ZERO_SINGLE == crate::mix::ZERO_SINGLE, "the same 0.0f cell");

// --------------------------------------------------------- the source record `sub_82B453D8` writes

/// `stfs f1,0(r3)` — the clamped x.
pub const RECORD_X: u32 = 0;
/// `stfs f2,4(r3)` — the clamped y.
pub const RECORD_Y: u32 = 4;
/// `stfs f0,8(r3)` — the squared length, itself clamped to 1.0.
pub const RECORD_LENGTH_SQ: u32 = 8;
/// `stfs f31,12(r31)` — the angle [`place_panner`] settled on, read back by [`add_angular`].
pub const RECORD_ANGLE: u32 = 12;
/// 16, which is what `sub_82B460A0` advances its `r4` by per source.
pub const RECORD_BYTES: u32 = 16;

// ------------------------------------------------------------------ the config object (`r3`)

/// `object + 8*k` is speaker `k`'s `(x, y)`, for `k` in `0..7`. **Inferred** from the offset
/// arithmetic — `probe/ports/notes/sub_82B454B8.md` records it as an inference and so does this —
/// and consistent with the count sitting at `+56 == 8*7`.
pub const SPEAKER_PAIR_BYTES: u32 = 8;
/// `lfs f8,8(r3)` / `lfs f13,12(r3)` — speaker 1, whose gain is `gains + 4`.
pub const SPEAKER_1: u32 = 8;
/// `lfs f7,40(r3)` / `lfs f7,44(r3)` — speaker 5, whose gain is `gains + 20`.
pub const SPEAKER_5: u32 = 40;
/// `lfs f6,48(r3)` / `lfs f6,52(r3)` — speaker 6, whose gain is `gains + 24`.
pub const SPEAKER_6: u32 = 48;
/// `s32`, compared with 4, 6 and 8. Inferred: the output channel count. **Reloaded** between stores
/// by three of the four bodies that read it.
pub const COUNT: u32 = 56;
/// `f32` sector edge 0. Added to the angle before the wrap and subtracted after it, so it is both a
/// bound and the rotation of the whole sector ladder.
pub const BOUND_0: u32 = 60;
/// `f32` sector edge 1, reflected as `2π - bound1` for the far side.
pub const BOUND_1: u32 = 64;
/// `f32` sector edge 2, reflected as `2π - bound2`.
pub const BOUND_2: u32 = 68;
/// `f32` — scales the share sector 1 moves into the centre channel.
pub const CENTRE_SPREAD: u32 = 72;
/// `f32[4]` per sector: `{sin0, cos0, sin1, cos1}`. Sectors 4 and 5 share [`SECTOR_45`].
pub const SECTOR_1: u32 = 76;
/// See [`SECTOR_1`].
pub const SECTOR_2: u32 = 92;
/// See [`SECTOR_1`].
pub const SECTOR_3: u32 = 108;
/// See [`SECTOR_1`]. Used by sectors 4 and 5 both.
pub const SECTOR_45: u32 = 124;
/// See [`SECTOR_1`].
pub const SECTOR_6: u32 = 140;
/// See [`SECTOR_1`].
pub const SECTOR_7: u32 = 156;
/// `s32[4]` at `+172`, `+176`, `+180`, `+184`. Used two ways: `8*index` reads a speaker pair out of
/// the object, `4*index` addresses the gain array. **Reloaded between stores.**
pub const INDEX: u32 = 172;
/// The four index words as one span, which is what a `Windows()` overlap test covers.
pub const INDEX_BYTES: u32 = 16;

const _: () = assert!(SPEAKER_1 == SPEAKER_PAIR_BYTES * 1);
const _: () = assert!(SPEAKER_5 == SPEAKER_PAIR_BYTES * 5);
const _: () = assert!(SPEAKER_6 == SPEAKER_PAIR_BYTES * 6);
const _: () = assert!(
    COUNT == SPEAKER_PAIR_BYTES * 7,
    "the table ends where the count begins"
);
const _: () = assert!(SECTOR_2 == SECTOR_1 + 16 && SECTOR_7 == SECTOR_1 + 80);

/// `rlwinm rD,rS,3,0,28` on a zero-extended word: `8 * index`, the speaker-pair offset.
///
/// A 32-bit shift, not a rotate: the rotate can only bring index bits 31..29 into the bits the mask
/// clears. Written as its own function because the same index is also used as [`word_offset`], and
/// conflating the two is the one arithmetic mistake this structure invites.
pub fn pair_offset(index: u32) -> u32 {
    index << 3
}

/// `rlwinm rD,rS,2,0,29` on a zero-extended word: `4 * index`, the gain-array offset.
pub fn word_offset(index: u32) -> u32 {
    index << 2
}

// ============================================================== sub_82B453D8: the unit-disc clamp

/// `sub_82B453D8` — store a 2D vector and its squared length, clamped to the unit disc.
///
/// `out` is `r3`, `x` is `f1`, `y` is `f2`. No result: the mask is `kReturnNone`, and all ten direct
/// callers reload what they need from memory afterwards.
///
/// Writes the 12 bytes at `out` on every path — `+0` and `+4` once, or twice on the normalising
/// path; `+8` once or twice. Reads [`ONE_SINGLE`] always and [`SNAP_SINGLE`] only when the squared
/// length came out below 1.0. It loads **nothing** from `out`.
///
/// Three outcomes, and the middle one is the surprise:
///
/// | `lensq` | `+0`, `+4` | `+8` |
/// |---|---|---|
/// | `≤ 0.999` | x, y | `lensq` |
/// | `(0.999, 1.0)` | x, y | **1.0** — x and y are *not* rescaled |
/// | `> 1.0` | `x/len`, `y/len` | 1.0 |
/// | `== 1.0`, or NaN | x, y | `lensq` |
///
/// So a source just inside the disc is reported as being exactly on its edge while keeping its
/// interior position. That is reproduced, not corrected; it is what the original does and the
/// consumers of `+8` treat 1.0 as the degenerate case.
pub fn clamp_to_unit_disc(g: &mut Guest, out: u32, x: f64, y: f64) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at fmuls f0,f2,f2

    let y_sq = fp::mul_single(y, y); // fmuls f0,f2,f2
    // The two position stores happen *before* the length is known, and before the 1.0 cell is even
    // loaded. Kept in that order: the normalising path below rewrites both.
    fp::store_single(g, out + RECORD_X, x)?; // stfs f1,0(r3)
    fp::store_single(g, out + RECORD_Y, y)?; // stfs f2,4(r3)
    let one = fp::load_single(g, ONE_SINGLE)?; // lfs f13,-22460(r11)
    let length_sq = fp::fmadd_single(x, x, y_sq); // fmadds f0,f1,f1,f0
    fp::store_single(g, out + RECORD_LENGTH_SQ, length_sq)?; // stfs f0,8(r3)

    // fcmpu cr6,f0,f13 ; bge cr6,loc_82B45414 — `bge` is "not lt", so a NaN takes the branch.
    if length_sq < one {
        let snap = fp::load_single(g, SNAP_SINGLE)?; // lfs f12,-10240(r11)
        // fcmpu cr6,f0,f12 ; blelr cr6 — written as `!(a > b)` so that a NaN returns, as it must.
        if !(length_sq > snap) {
            return Ok(());
        }
        return fp::store_single(g, out + RECORD_LENGTH_SQ, one); // stfs f13,8(r3)
    }

    // loc_82B45414.
    fpscr.disable_flush_mode_unconditional(); // emitted at the second fcmpu
    // blelr cr6 — exactly 1.0 and NaN return here, leaving the three stores above in place.
    if !(length_sq > one) {
        return Ok(());
    }
    let length = fp::sqrt_single(length_sq); // fsqrts f0,f0
    fp::store_single(g, out + RECORD_LENGTH_SQ, one)?; // stfs f13,8(r3)
    let scale = fp::div_single(one, length); // fdivs f13,f13,f0
    let unit_x = fp::mul_single(scale, x); // fmuls f12,f13,f1
    fp::store_single(g, out + RECORD_X, unit_x)?; // stfs f12,0(r3)
    let unit_y = fp::mul_single(scale, y); // fmuls f11,f13,f2
    fp::store_single(g, out + RECORD_Y, unit_y) // stfs f11,4(r3)
}

// ============================================================ sub_82B269C0: place a panner

/// `sub_82B269C0` — place a panner at an angle and a depth.
///
/// `panner` is `r3`, `angle_degrees` is `f1` and `depth` is `f2`. No result (`kReturnNone`).
///
/// Writes the 16 bytes at `panner`: `+0`/`+4`/`+8` through [`clamp_to_unit_disc`], and `+12` itself.
/// Reads [`DEGREES_TO_RADIANS`], [`ZERO_SINGLE`], [`HALF_TURN`], and — through the callee, which the
/// C++ `Windows()` does not declare — [`ONE_SINGLE`] and [`SNAP_SINGLE`].
///
/// `angle = f1 · (−π/180)`, then `(cos angle · depth, sin angle · depth)` goes to the clamp, and the
/// angle itself is stored at `+12` — biased by π when the depth is **not** strictly positive. The
/// compare is unordered, so a NaN depth takes the bias path.
///
/// The original spills `f29`-`f31` and `r31` into its red zone and restores them, because its three
/// callees may clobber the volatile file. There is no register file here, so that is not
/// reproduced — the C++ keeps them in locals for the same reason, and the write never reaches
/// memory in either version.
pub fn place_panner<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    panner: u32,
    angle_degrees: f64,
    depth: f64,
) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfs f0,1132(r10)

    // lfs f0,1132(r10) ; fmuls f31,f1,f0 — f1 first, as lifted.
    let scale = fp::load_single(g, DEGREES_TO_RADIANS)?;
    let angle = fp::mul_single(angle_degrees, scale);

    // bl 0x82f4dfb0 — cosine first, then sine. The order is kept because a `Trig` implementation
    // may fail, and because the real bodies are two separate guest functions.
    let cosine_raw = trig.cosine(g, angle)?;
    fpscr.disable_flush_mode_unconditional(); // emitted on return, before the frsp
    let cosine = fp::frsp(cosine_raw); // frsp f0,f1
    let x = fp::mul_single(cosine, depth); // fmuls f29,f0,f30
    let sine_raw = trig.sine(g, angle)?; // fmr f1,f31 ; bl 0x82f4ded0
    fpscr.disable_flush_mode_unconditional();
    let sine = fp::frsp(sine_raw); // frsp f13,f1
    let y = fp::mul_single(sine, depth); // fmuls f2,f13,f30

    // bl 0x82b453d8 — the same body [`clamp_to_unit_disc`] is, reached here rather than duplicated.
    clamp_to_unit_disc(g, panner, x, y)?;

    // The callee sets its own flush mode; this is the disable the lifted line emits on return.
    fpscr.disable_flush_mode_unconditional();
    let zero = fp::load_single(g, ZERO_SINGLE)?;
    // fcmpu ; ble — a positive depth keeps the angle; NaN is unordered and takes the bias.
    if depth > zero {
        fp::store_single(g, panner + RECORD_ANGLE, angle) // stfs f31,12(r31)
    } else {
        let bias = fp::load_single(g, HALF_TURN)?;
        fp::store_single(g, panner + RECORD_ANGLE, fp::add_single(angle, bias))
    }
}

// ==================================================== sub_82B454B8: distance panning

/// `sub_82B454B8` — pan a 2D source into the speaker-gain array by distance.
///
/// `object` is `r3`, `input` is `r4` (the record [`clamp_to_unit_disc`] writes), `gains` is `r6` and
/// `focus` is `f1` — the caller's own weight for speaker 1, used only when the count is not 4.
/// No result (`kReturnNone`); `f1` is left holding `depth · gain172`, which neither caller reads and
/// the harness mask does not compare, so it is not returned here.
///
/// Writes, all inside `gains`:
///
/// | path | words |
/// |---|---|
/// | `input+8 == 1.0` | `+0`…`+16` as raw zero `stw`, then `+20`/`+24` as `0.0f` when the count is 8 |
/// | otherwise | `4·index` for each of the four indices, `+4` when the count is ≥ 6, `+20`/`+24` when it is 8 |
///
/// Reads the 12-byte record, the count, the four indices, the speaker pairs they select, speakers 5
/// and 6 when the count is 8, speaker 1 when it is not 4, and [`ONE_SINGLE`], [`ZERO_SINGLE`],
/// [`HALF_SINGLE`], [`EPSILON_SINGLE`].
///
/// The arithmetic: `weight_k = 1 − 0.5·|speaker_k − source|`; `front = 0.5·(x + 1)` and
/// `back = 1 − front`, each snapped to zero inside 0.0005; the front pair is normalised by
/// `sqrt(front / Σweight²)` over two weights at a count of 4 and over three otherwise, the third
/// being speaker 1's weight times `focus`; the back pair by `sqrt(back / Σweight²)` over speakers 5,
/// 6 and indices 184, 180. Every stored gain is then scaled by `sqrt(1 − lensq²)`, with `lensq`
/// **reloaded** from the record after the first pass over it.
///
/// **The count and all four indices are reloaded from the object between the stores.** That is why
/// the C++ `Windows()` declines any call whose gain array overlaps `object + 172..+187`: a later
/// store address would not be derivable from entry state. The reloads are reproduced here, so a
/// caller with that layout gets the original's behaviour rather than a hoisted one, and nothing
/// establishes what the guest does with it.
pub fn pan_distance(g: &mut Guest, object: u32, input: u32, gains: u32, focus: f64) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfs f13,8(r4)

    let lensq = fp::load_single(g, input + RECORD_LENGTH_SQ)?; // lfs f13,8(r4)
    let one = fp::load_single(g, ONE_SINGLE)?; // lfs f0,-22460(r11)

    // fcmpu cr6,f13,f0 ; bne cr6 — a NaN lensq is not equal, so it takes the main path.
    if lensq == one {
        // Five raw word stores of zero, not `stfs` of the 0.0f cell: `li r11,0 ; stw r11,0(r6)`.
        for slot in [0u32, 4, 8, 12, 16] {
            g.set_u32(gains + slot, 0)?;
        }
        // lwz r11,56(r3) — reloaded *after* those five stores.
        if (g.u32(object + COUNT)? as i32) != 8 {
            return Ok(());
        }
        let zero = fp::load_single(g, ZERO_SINGLE)?;
        fp::store_single(g, gains + 20, zero)?; // stfs f0,20(r6)
        return fp::store_single(g, gains + 24, zero); // stfs f0,24(r6)
    }

    // loc_82B45518. The loads interleave with the index reads exactly as lifted.
    let index172 = g.u32(object + INDEX)?; // lwz r11,172(r3)
    fpscr.disable_flush_mode_unconditional();
    let x = fp::load_single(g, input + RECORD_X)?; // lfs f13,0(r4)
    let index176 = g.u32(object + INDEX + 4)?; // lwz r10,176(r3)
    let y = fp::load_single(g, input + RECORD_Y)?; // lfs f11,4(r4)
    let index180 = g.u32(object + INDEX + 8)?; // lwz r9,180(r3)
    let index184 = g.u32(object + INDEX + 12)?; // lwz r8,184(r3)
    let count = g.u32(object + COUNT)? as i32; // lwz r7,56(r3)
    let pair172 = object.wrapping_add(pair_offset(index172));
    let pair176 = object.wrapping_add(pair_offset(index176));
    let pair180 = object.wrapping_add(pair_offset(index180));
    let pair184 = object.wrapping_add(pair_offset(index184));

    // The four x coordinates, then the four y, each load and subtract in the lifted order. The
    // interleaving of a pure `fsubs` with the next `lfs` is value-neutral — there is no store
    // between them — and it is written out anyway so that this block can be read against the `.inc`
    // line for line.
    let sx172 = fp::load_single(g, pair172)?; // lfs f12,0(r11)
    let sx176 = fp::load_single(g, pair176)?; // lfs f10,0(r10)
    let dx172 = fp::sub_single(sx172, x); // fsubs f9,f12,f13
    let sx180 = fp::load_single(g, pair180)?; // lfs f8,0(r9)
    let dx176 = fp::sub_single(sx176, x); // fsubs f7,f10,f13
    let sx184 = fp::load_single(g, pair184)?; // lfs f6,0(r8)
    let dx180 = fp::sub_single(sx180, x); // fsubs f5,f8,f13
    let dx184 = fp::sub_single(sx184, x); // fsubs f4,f6,f13
    let sy172 = fp::load_single(g, pair172 + 4)?; // lfs f3,4(r11)
    let sy176 = fp::load_single(g, pair176 + 4)?; // lfs f2,4(r10)
    let dy172 = fp::sub_single(sy172, y); // fsubs f10,f3,f11
    let sy180 = fp::load_single(g, pair180 + 4)?; // lfs f8,4(r9)
    let dy176 = fp::sub_single(sy176, y); // fsubs f6,f2,f11
    let sy184 = fp::load_single(g, pair184 + 4)?; // lfs f3,4(r8)
    let dy180 = fp::sub_single(sy180, y); // fsubs f2,f8,f11
    let dy184 = fp::sub_single(sy184, y); // fsubs f3,f3,f11
    let half = fp::load_single(g, HALF_SINGLE)?; // lfs f12,-26788(r31)
    let zero = fp::load_single(g, ZERO_SINGLE)?; // lfs f8,23056(r5)

    // weight = 1 − 0.5·|speaker − source|, per selected speaker.
    let q172 = fp::mul_single(dx172, dx172); // fmuls f9,f9,f9
    let q176 = fp::mul_single(dx176, dx176); // fmuls f7,f7,f7
    let q180 = fp::mul_single(dx180, dx180); // fmuls f31,f5,f5
    let q184 = fp::mul_single(dx184, dx184); // fmuls f30,f4,f4
    // f5 and f4 are set from the 0.0f constant here and only overwritten on the count == 8 path.
    let mut weight5 = zero; // fmr f5,f8
    let mut weight6 = zero; // fmr f4,f8
    let q172 = fp::fmadd_single(dy172, dy172, q172); // fmadds f10,f10,f10,f9
    let q176 = fp::fmadd_single(dy176, dy176, q176); // fmadds f9,f6,f6,f7
    let q180 = fp::fmadd_single(dy180, dy180, q180); // fmadds f7,f2,f2,f31
    let q184 = fp::fmadd_single(dy184, dy184, q184); // fmadds f6,f3,f3,f30
    let r172 = fp::sqrt_single(q172); // fsqrts f3,f10
    let r176 = fp::sqrt_single(q176); // fsqrts f2,f9
    let r180 = fp::sqrt_single(q180); // fsqrts f7,f7
    let r184 = fp::sqrt_single(q184); // fsqrts f6,f6
    let weight172 = fp::nmsub_single(r172, half, one); // fnmsubs f10,f3,f12,f0
    let weight176 = fp::nmsub_single(r176, half, one); // fnmsubs f9,f2,f12,f0
    let weight180 = fp::nmsub_single(r180, half, one); // fnmsubs f3,f7,f12,f0
    let weight184 = fp::nmsub_single(r184, half, one); // fnmsubs f2,f6,f12,f0

    // cmpwi cr6,r7,8 ; bne cr6 — the two 7.1 side speakers.
    if count == 8 {
        let sx5 = fp::load_single(g, object + SPEAKER_5)?; // lfs f7,40(r3)
        let sx6 = fp::load_single(g, object + SPEAKER_6)?; // lfs f6,48(r3)
        let dx5 = fp::sub_single(sx5, x); // fsubs f5,f7,f13
        let dx6 = fp::sub_single(sx6, x); // fsubs f4,f6,f13
        let sy5 = fp::load_single(g, object + SPEAKER_5 + 4)?; // lfs f7,44(r3)
        let sy6 = fp::load_single(g, object + SPEAKER_6 + 4)?; // lfs f6,52(r3)
        let dy5 = fp::sub_single(sy5, y); // fsubs f7,f7,f11
        let dy6 = fp::sub_single(sy6, y); // fsubs f6,f6,f11
        let q5 = fp::mul_single(dx5, dx5); // fmuls f5,f5,f5
        let q6 = fp::mul_single(dx6, dx6); // fmuls f4,f4,f4
        let q5 = fp::fmadd_single(dy5, dy5, q5); // fmadds f7,f7,f7,f5
        let q6 = fp::fmadd_single(dy6, dy6, q6); // fmadds f6,f6,f6,f4
        let r5 = fp::sqrt_single(q5); // fsqrts f5,f7
        let r6 = fp::sqrt_single(q6); // fsqrts f4,f6
        weight5 = fp::nmsub_single(r5, half, one); // fnmsubs f5,f5,f12,f0
        weight6 = fp::nmsub_single(r6, half, one); // fnmsubs f4,f4,f12,f0
    }

    // loc_82B45634: the front/back split, each snapped to 0 inside 0.0005 of it.
    fpscr.disable_flush_mode_unconditional();
    let mut front = fp::add_single(x, one); // fadds f7,f13,f0
    let eps = fp::load_single(g, EPSILON_SINGLE)?; // lfs f6,792(r11)
    front = fp::mul_single(front, half); // fmuls f7,f7,f12
    // fabs ; fcmpu ; bge — unordered is not lt, so a NaN front is kept rather than snapped.
    if fp::abs_double(front) < eps {
        front = zero; // fmr f7,f8
    }

    // loc_82B45654.
    fpscr.disable_flush_mode_unconditional();
    let mut back = fp::sub_single(one, front); // fsubs f31,f0,f7
    if fp::abs_double(back) < eps {
        back = zero; // fmr f31,f8
    }

    // loc_82B45668: normalise the front pair over two speakers at a count of 4, three otherwise.
    let gain172;
    let gain176;
    // f8 still holds the 0.0f constant on the count == 4 path, so the centre share is exactly it.
    let mut centre = zero;
    if count == 4 {
        fpscr.disable_flush_mode_unconditional();
        let sq = fp::mul_single(weight172, weight172); // fmuls f13,f10,f10
        let sum = fp::fmadd_single(weight176, weight176, sq); // fmadds f12,f9,f9,f13
        let ratio = fp::div_single(front, sum); // fdivs f11,f7,f12
        let k = fp::sqrt_single(ratio); // fsqrts f7,f11
        gain172 = fp::mul_single(k, weight172); // fmuls f13,f7,f10
        gain176 = fp::mul_single(k, weight176); // fmuls f12,f7,f9
    } else {
        // loc_82B4568C.
        fpscr.disable_flush_mode_unconditional();
        let sx1 = fp::load_single(g, object + SPEAKER_1)?; // lfs f8,8(r3)
        let dx1 = fp::sub_single(sx1, x); // fsubs f6,f8,f13
        let sy1 = fp::load_single(g, object + SPEAKER_1 + 4)?; // lfs f13,12(r3)
        let dy1 = fp::sub_single(sy1, y); // fsubs f11,f13,f11
        let q1 = fp::mul_single(dx1, dx1); // fmuls f8,f6,f6
        let q1 = fp::fmadd_single(dy1, dy1, q1); // fmadds f6,f11,f11,f8
        let r1 = fp::sqrt_single(q1); // fsqrts f13,f6
        let weight1 = fp::nmsub_single(r1, half, one); // fnmsubs f12,f13,f12,f0
        let scaled1 = fp::mul_single(weight1, focus); // fmuls f11,f12,f1
        let sum = fp::mul_single(scaled1, scaled1); // fmuls f8,f11,f11
        let sum = fp::fmadd_single(weight176, weight176, sum); // fmadds f6,f9,f9,f8
        let sum = fp::fmadd_single(weight172, weight172, sum); // fmadds f1,f10,f10,f6
        let ratio = fp::div_single(front, sum); // fdivs f13,f7,f1
        let k = fp::sqrt_single(ratio); // fsqrts f8,f13
        gain172 = fp::mul_single(k, weight172); // fmuls f13,f8,f10
        gain176 = fp::mul_single(k, weight176); // fmuls f12,f8,f9
        centre = fp::mul_single(k, scaled1); // fmuls f8,f8,f11
    }

    // loc_82B456D0: every gain scaled by sqrt(1 − lensq²), the back pair normalised over the four
    // remaining weights, and **each index reloaded after the previous store**.
    fpscr.disable_flush_mode_unconditional();
    let spread = fp::mul_single(weight5, weight5); // fmuls f11,f5,f5
    let lensq2 = fp::load_single(g, input + RECORD_LENGTH_SQ)?; // lfs f10,8(r4) — reloaded
    let lift = fp::nmsub_single(lensq2, lensq2, one); // fnmsubs f9,f10,f10,f0
    let out172 = gains.wrapping_add(word_offset(g.u32(object + INDEX)?)); // lwz r11,172(r3)
    let spread = fp::fmadd_single(weight6, weight6, spread); // fmadds f7,f4,f4,f11
    let depth = fp::sqrt_single(lift); // fsqrts f0,f9
    let spread = fp::fmadd_single(weight184, weight184, spread); // fmadds f6,f2,f2,f7
    fp::store_single(g, out172, fp::mul_single(depth, gain172))?; // fmuls f1,f0,f13 ; stfsx
    let out176 = gains.wrapping_add(word_offset(g.u32(object + INDEX + 4)?)); // lwz r9,176(r3)
    let spread = fp::fmadd_single(weight180, weight180, spread); // fmadds f13,f3,f3,f6
    fp::store_single(g, out176, fp::mul_single(depth, gain176))?; // fmuls f12,f0,f12 ; stfsx
    let out180 = gains.wrapping_add(word_offset(g.u32(object + INDEX + 8)?)); // lwz r7,180(r3)
    let ratio = fp::div_single(back, spread); // fdivs f11,f31,f13
    let k2 = fp::sqrt_single(ratio); // fsqrts f13,f11
    let rear180 = fp::mul_single(k2, weight180); // fmuls f10,f13,f3
    let rear184 = fp::mul_single(k2, weight184); // fmuls f9,f13,f2
    fp::store_single(g, out180, fp::mul_single(rear180, depth))?; // fmuls f7,f10,f0 ; stfsx
    let out184 = gains.wrapping_add(word_offset(g.u32(object + INDEX + 12)?)); // lwz r4,184(r3)
    fp::store_single(g, out184, fp::mul_single(rear184, depth))?; // fmuls f6,f9,f0 ; stfsx

    // lwz r10,56(r3) — reloaded after the four stores.
    if (g.u32(object + COUNT)? as i32) < 6 {
        return Ok(());
    }
    fp::store_single(g, gains + 4, fp::mul_single(depth, centre))?; // fmuls f12,f0,f8 ; stfs
    // lwz r11,56(r3) — reloaded again.
    if (g.u32(object + COUNT)? as i32) != 8 {
        return Ok(());
    }
    let side5 = fp::mul_single(k2, weight5); // fmuls f12,f13,f5
    let side6 = fp::mul_single(k2, weight6); // fmuls f11,f13,f4
    fp::store_single(g, gains + 20, fp::mul_single(side5, depth))?; // fmuls f10,f12,f0 ; stfs
    fp::store_single(g, gains + 24, fp::mul_single(side6, depth)) // fmuls f9,f11,f0 ; stfs
}

// ================================================ sub_82B45788: the angular contribution

/// The two projections a sector takes: coefficient `+0` against the sine and `+4` against the
/// cosine for the first, `+8` and `+12` for the second.
///
/// The four loads and the two `fmuls` are in the lifted order, and the two `fmadds` follow every
/// load in all six blocks, so grouping them moves nothing across a store. Sector 1 does **not** use
/// this: its own block interleaves a count read between the loads and it is written out inline.
fn project(g: &Guest, coeff: u32, sine: f64, cosine: f64) -> Result<(f64, f64)> {
    let c0 = fp::load_single(g, coeff)?; // lfs f0,92(r31)
    let p0 = fp::mul_single(c0, sine); // fmuls f12,f0,f30
    let c1 = fp::load_single(g, coeff + 4)?; // lfs f11,96(r31)
    let c2 = fp::load_single(g, coeff + 8)?; // lfs f10,100(r31)
    let p1 = fp::mul_single(c2, sine); // fmuls f9,f10,f30
    let c3 = fp::load_single(g, coeff + 12)?; // lfs f8,104(r31)
    Ok((
        fp::fmadd_single(c1, cosine, p0), // fmadds f5,f11,f13,f12
        fp::fmadd_single(c3, cosine, p1), // fmadds f4,f8,f13,f9
    ))
}

/// `length / sqrt(first² + second²)`: `fmuls`, `fmadds`, `fsqrts`, `fdivs`, in that order.
fn normalise(pair: (f64, f64), length: f64) -> f64 {
    let sq = fp::mul_single(pair.0, pair.0); // fmuls f3,f5,f5
    let sum = fp::fmadd_single(pair.1, pair.1, sq); // fmadds f2,f4,f4,f3
    fp::div_single(length, fp::sqrt_single(sum)) // fsqrts f1,f2 ; fdivs f0,f7,f1
}

/// Which of the seven sectors a call took. Returned because it is the whole of the control flow and
/// a test that cannot see it has to infer it from the words that moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sector {
    /// `reduced < bound0`. Coefficients `+76`, destinations `4·idx176`, `4·idx172`, and `gains + 4`
    /// when the count is ≥ 6. The only sector with a centre channel.
    Front,
    /// `reduced < bound1`. Coefficients `+92`, destinations `4·idx172`, `4·idx180`.
    FrontSide,
    /// `reduced < bound2` and count 8. Coefficients `+108`, destinations `gains + 12`, `+20`.
    Side,
    /// `reduced < 2π − bound2` and count 8. Coefficients `+124`, destinations `gains + 20`, `+24`.
    SideRear,
    /// `reduced < 2π − bound1` and count ≤ 6. Coefficients `+124`, destinations `4·idx180`,
    /// `4·idx184`.
    Rear,
    /// `reduced < 2π − bound1` and count 8. Coefficients `+140`, destinations `gains + 24`, `+16`.
    RearWide,
    /// Everything else. Coefficients `+156`, destinations `4·idx184`, `4·idx176`.
    Back,
}

/// `sub_82B45788` — add one source's angular contribution to the speaker-gain array.
///
/// `object` is `r3`, `record` is `r4`, `gains` is `r6` and `weight` is `f1` — used only by
/// [`Sector::Front`]. The guest function is void (`kReturnNone`); the [`Sector`] returned here is
/// not a guest value, it is this port telling its tests which branch ran.
///
/// Writes **two words** of `gains` per call, never more: two of the fixed slots `+4`/`+12`/`+16`/
/// `+20`/`+24`, or two of `gains + 4·index`, plus `gains + 4` in the one sector that has a centre
/// channel. Every store is a read-modify-write — the old gain is loaded and the contribution is
/// **mix-added** — so this accumulates over sources rather than replacing.
///
/// Reads the 16-byte record, `object + 56..+187` (the count, three bounds, the centre scale, six
/// coefficient blocks and the four indices), [`TURNS_PER_RADIAN`], [`RADIANS_PER_TURN`],
/// [`ZERO_SINGLE`], and `mathlib`'s two pool doubles through [`mathlib::floor`]. The last three are
/// **not** in the C++ `Windows()`, which is why a recorded vector for this function cannot be
/// replayed yet even once the trigonometry exists.
///
/// ## The reduction
///
/// `scaled = (record+12 + bound0) · 1/2π`; `fraction = scaled − frsp(floor(scaled))`;
/// `reduced = fraction · 2π − bound0`. With the two constants measured, that is an angle wrapped
/// into one turn with the sector ladder rotated by `bound0`. The `frsp` is not decoration: it
/// narrows the floor to a single before the subtraction, so the fraction a caller gets is the one
/// the original computed and not a more accurate one.
///
/// ## Two structural details a reader would simplify away
///
/// - In the three index-pair sectors ([`Sector::FrontSide`], [`Sector::Rear`], [`Sector::Back`]) the
///   **second index is read after the first store**. In the three fixed-slot sectors both old gains
///   are loaded before either store. The shared tail `loc_82B45B3C` is therefore inlined at each of
///   its three arrival points rather than factored out.
/// - Arriving at `loc_82B45A80` from the `bge` above it re-tests the same value, so that arm always
///   falls through to [`Sector::Back`]; [`Sector::RearWide`] is reachable only through the
///   `count > 6` branch. Written as `inside && count <= 6` then `inside && count == 8`, which is the
///   same set of paths.
///
/// ## What is not reproduced
///
/// `stwu r1,-160(r1)`. The C++ **does** reproduce it, because the sine and cosine helpers spill
/// `f30`/`f31`/`f10` at `r1-8 … r1-32` and would otherwise land 160 bytes higher than the original
/// put them. Here those two are a [`Trig`] parameter rather than guest code, so no callee spills
/// anything and there is nothing to make room for — the same position `crate::dsp::biquad` is in
/// with its own frame. **If `sub_82F4DED0` and `sub_82F4DFB0` are ever ported as guest bodies that
/// write their red zone, this frame becomes load-bearing again** and this port needs the entry `r1`
/// as an argument. The back chain itself is inside this function's own frame, which no window
/// declares and no comparison sees.
///
/// The `__savegprlr_29`/`__savefpr_27` spills and the matching restores are likewise not
/// reproduced: there is no register file here, and `fmr f27,f31` writes a register that
/// `__restfpr_27` immediately overwrites.
pub fn add_angular<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    object: u32,
    record: u32,
    gains: u32,
    weight: f64,
) -> Result<Sector> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional(); // emitted at lfs f0,12(r4)

    let angle = fp::load_single(g, record + RECORD_ANGLE)?; // lfs f0,12(r4)
    let bound0 = fp::load_single(g, object + BOUND_0)?; // lfs f13,60(r3)
    let biased = fp::add_single(angle, bound0); // fadds f12,f0,f13
    let turns = fp::load_single(g, TURNS_PER_RADIAN)?; // lfs f0,772(r10)
    let scaled = fp::mul_single(biased, turns); // fmuls f31,f12,f0

    // bl 0x82f4de80 — floor. The caller keeps only frsp(result).
    let floored = mathlib::floor(g, scaled)?;
    fpscr.disable_flush_mode_unconditional(); // emitted on return, before the frsp
    let rounded = fp::frsp(floored); // frsp f11,f1
    let bound0_again = fp::load_single(g, object + BOUND_0)?; // lfs f10,60(r31) — reloaded
    let step = fp::load_single(g, RADIANS_PER_TURN)?; // lfs f29,16668(r9)
    let fraction = fp::sub_single(scaled, rounded); // fsubs f9,f31,f11
    let reduced = fp::fmsub_single(fraction, step, bound0_again); // fmsubs f31,f9,f29,f10

    // bl 0x82f4ded0 then bl 0x82f4dfb0 — sine first here, unlike `place_panner`.
    let sine_raw = trig.sine(g, reduced)?;
    fpscr.disable_flush_mode_unconditional();
    let sine = fp::frsp(sine_raw); // frsp f30,f1
    let cosine_raw = trig.cosine(g, reduced)?; // fmr f1,f31 ; bl 0x82f4dfb0
    fpscr.disable_flush_mode_unconditional();
    let edge0 = fp::load_single(g, object + BOUND_0)?; // lfs f8,60(r31) — reloaded a third time
    let cosine = fp::frsp(cosine_raw); // frsp f13,f1

    // fcmpu cr6,f31,f8 ; bge cr6 — unordered is not lt, so a NaN angle walks the whole ladder.
    if reduced < edge0 {
        // Sector 1. Its projection is inlined because a count read sits between the loads.
        let c0 = fp::load_single(g, object + SECTOR_1)?; // lfs f0,76(r31)
        let c2 = fp::load_single(g, object + SECTOR_1 + 8)?; // lfs f12,84(r31)
        let p0 = fp::mul_single(c0, sine); // fmuls f11,f0,f30
        let p1 = fp::mul_single(c2, sine); // fmuls f10,f12,f30
        let c1 = fp::load_single(g, object + SECTOR_1 + 4)?; // lfs f9,80(r31)
        let count = g.u32(object + COUNT)? as i32; // lwz r10,56(r31)
        let c3 = fp::load_single(g, object + SECTOR_1 + 12)?; // lfs f8,88(r31)
        let mut left = fp::fmadd_single(c1, cosine, p0); // fmadds f0,f9,f13,f11
        let mut right = fp::fmadd_single(c3, cosine, p1); // fmadds f13,f8,f13,f10
        // lfs f12,23056(r11) — the 0.0f cell. The C++ writes a literal 0.0 here; this loads it, as
        // the original does. See the module note.
        let mut centre = fp::load_single(g, ZERO_SINGLE)?;
        // cmpwi cr6,r10,6 ; blt cr6
        if count >= 6 {
            // fcmpu cr6,f0,f13 ; bge cr6 — a NaN takes the second arm, as `!(a < b)` does.
            let mut share = if left < right { left } else { right };
            share = fp::mul_single(share, weight); // fmuls f12,f12,f28
            let spread = fp::load_single(g, object + CENTRE_SPREAD)?; // lfs f11,72(r31)
            left = fp::sub_single(left, share); // fsubs f0,f0,f12
            right = fp::sub_single(right, share); // fsubs f13,f13,f12
            centre = fp::mul_single(spread, share); // fmuls f12,f11,f12
        }

        // loc_82B4586C: three terms, the third exactly the 0.0f cell below a count of 6.
        fpscr.disable_flush_mode_unconditional();
        let right_sq = fp::mul_single(right, right); // fmuls f11,f13,f13
        let length = fp::load_single(g, record + RECORD_LENGTH_SQ)?; // lfs f10,8(r29)
        let slot176 = gains.wrapping_add(word_offset(g.u32(object + INDEX + 4)?)); // lwz r11,176
        let old176 = fp::load_single(g, slot176)?; // lfsx f9,r11,r30
        let with_centre = fp::fmadd_single(centre, centre, right_sq); // fmadds f8,f12,f12,f11
        let sum = fp::fmadd_single(left, left, with_centre); // fmadds f7,f0,f0,f8
        let norm = fp::div_single(length, fp::sqrt_single(sum)); // fsqrts f6,f7 ; fdivs f11
        fp::store_single(g, slot176, fp::fmadd_single(norm, left, old176))?; // fmadds f5 ; stfsx
        // lwz r10,172(r31) — read AFTER the store above.
        let slot172 = gains.wrapping_add(word_offset(g.u32(object + INDEX)?));
        let old172 = fp::load_single(g, slot172)?; // lfsx f4,r11,r30
        fp::store_single(g, slot172, fp::fmadd_single(norm, right, old172))?; // fmadds f3 ; stfsx
        // lwz r9,56(r31) — reloaded again.
        if (g.u32(object + COUNT)? as i32) >= 6 {
            let old4 = fp::load_single(g, gains + 4)?; // lfs f0,4(r30)
            fp::store_single(g, gains + 4, fp::fmadd_single(norm, centre, old4))?; // stfs f13
        }
        return Ok(Sector::Front);
    }

    // loc_82B458D4.
    fpscr.disable_flush_mode_unconditional();
    let edge1 = fp::load_single(g, object + BOUND_1)?; // lfs f12,64(r31)
    if reduced < edge1 {
        // Sector 2: the index-pair form. The second index is read after the first store.
        let slot_a = gains.wrapping_add(word_offset(g.u32(object + INDEX)?)); // lwz r11,172
        let pair = project(g, object + SECTOR_2, sine, cosine)?;
        let length = fp::load_single(g, record + RECORD_LENGTH_SQ)?; // lfs f7,8(r29)
        let old_a = fp::load_single(g, slot_a)?; // lfsx f6,r11,r30
        let norm = normalise(pair, length);
        fp::store_single(g, slot_a, fp::fmadd_single(pair.0, norm, old_a))?; // fmadds f13 ; stfsx
        // lwz r10,180(r31) ; b 0x82b45b3c
        let slot_b = gains.wrapping_add(word_offset(g.u32(object + INDEX + 8)?));
        let old_b = fp::load_single(g, slot_b)?; // lfsx f12,r11,r30
        fp::store_single(g, slot_b, fp::fmadd_single(pair.1, norm, old_b))?; // fmadds f11 ; stfsx
        return Ok(Sector::FrontSide);
    }

    // loc_82B45930.
    fpscr.disable_flush_mode_unconditional();
    let edge2 = fp::load_single(g, object + BOUND_2)?; // lfs f0,68(r31)
    // bge cr6 ; then lwz r11,56(r31) ; cmpwi 8 ; bne cr6 — both must hold.
    if reduced < edge2 && (g.u32(object + COUNT)? as i32) == 8 {
        // Sector 3: fixed slots +12 and +20, both old values loaded before either store.
        let pair = project(g, object + SECTOR_3, sine, cosine)?;
        let length = fp::load_single(g, record + RECORD_LENGTH_SQ)?; // lfs f7,8(r29)
        let old12 = fp::load_single(g, gains + 12)?; // lfs f6,12(r30)
        let old20 = fp::load_single(g, gains + 20)?; // lfs f5,20(r30)
        let norm = normalise(pair, length);
        fp::store_single(g, gains + 12, fp::fmadd_single(pair.0, norm, old12))?; // stfs f12,12
        fp::store_single(g, gains + 20, fp::fmadd_single(pair.1, norm, old20))?; // stfs f11,20
        return Ok(Sector::Side);
    }

    // loc_82B459A4: the first reflected bound, 2π − the +68 single just loaded.
    fpscr.disable_flush_mode_unconditional();
    let reflect2 = fp::sub_single(step, edge2); // fsubs f0,f29,f0
    if reduced < reflect2 && (g.u32(object + COUNT)? as i32) == 8 {
        // Sector 4: fixed slots +20 and +24, sector 5's coefficients.
        let pair = project(g, object + SECTOR_45, sine, cosine)?;
        let length = fp::load_single(g, record + RECORD_LENGTH_SQ)?; // lfs f7,8(r29)
        let old20 = fp::load_single(g, gains + 20)?; // lfs f6,20(r30)
        let old24 = fp::load_single(g, gains + 24)?; // lfs f5,24(r30)
        let norm = normalise(pair, length);
        fp::store_single(g, gains + 20, fp::fmadd_single(pair.0, norm, old20))?; // stfs f12,20
        fp::store_single(g, gains + 24, fp::fmadd_single(pair.1, norm, old24))?; // stfs f11,24
        return Ok(Sector::SideRear);
    }

    // loc_82B45A18: the second reflected bound, 2π − the +64 single.
    fpscr.disable_flush_mode_unconditional();
    let reflect1 = fp::sub_single(step, edge1); // fsubs f0,f29,f12
    let inside = reduced < reflect1; // fcmpu cr6,f31,f0
    let count = g.u32(object + COUNT)? as i32; // lwz r11,56(r31)
    if inside && count <= 6 {
        // Sector 5: the index pair (+180, +184), sector 4's coefficients.
        let slot_a = gains.wrapping_add(word_offset(g.u32(object + INDEX + 8)?)); // lwz r11,180
        let pair = project(g, object + SECTOR_45, sine, cosine)?;
        let length = fp::load_single(g, record + RECORD_LENGTH_SQ)?; // lfs f7,8(r29)
        let old_a = fp::load_single(g, slot_a)?; // lfsx f6,r11,r30
        let norm = normalise(pair, length);
        fp::store_single(g, slot_a, fp::fmadd_single(pair.0, norm, old_a))?; // stfsx f13
        // lwz r10,184(r31) ; b 0x82b45b3c
        let slot_b = gains.wrapping_add(word_offset(g.u32(object + INDEX + 12)?));
        let old_b = fp::load_single(g, slot_b)?; // lfsx f12,r11,r30
        fp::store_single(g, slot_b, fp::fmadd_single(pair.1, norm, old_b))?; // stfsx f11
        return Ok(Sector::Rear);
    }
    if inside && count == 8 {
        // loc_82B45A80. Sector 6: fixed slots +24 and +16, in that store order.
        let pair = project(g, object + SECTOR_6, sine, cosine)?;
        let length = fp::load_single(g, record + RECORD_LENGTH_SQ)?; // lfs f7,8(r29)
        let old16 = fp::load_single(g, gains + 16)?; // lfs f6,16(r30)
        let old24 = fp::load_single(g, gains + 24)?; // lfs f5,24(r30)
        let norm = normalise(pair, length);
        fp::store_single(g, gains + 24, fp::fmadd_single(pair.0, norm, old24))?; // stfs f12,24
        fp::store_single(g, gains + 16, fp::fmadd_single(pair.1, norm, old16))?; // stfs f11,16
        return Ok(Sector::RearWide);
    }

    // loc_82B45AF0 — sector 7: the index pair (+184, +176).
    let slot_a = gains.wrapping_add(word_offset(g.u32(object + INDEX + 12)?)); // lwz r11,184
    let pair = project(g, object + SECTOR_7, sine, cosine)?;
    let length = fp::load_single(g, record + RECORD_LENGTH_SQ)?; // lfs f7,8(r29)
    let old_a = fp::load_single(g, slot_a)?; // lfsx f6,r11,r30
    let norm = normalise(pair, length);
    fp::store_single(g, slot_a, fp::fmadd_single(pair.0, norm, old_a))?; // stfsx f13
    // lwz r10,176(r31) ; loc_82B45B3C
    let slot_b = gains.wrapping_add(word_offset(g.u32(object + INDEX + 4)?));
    let old_b = fp::load_single(g, slot_b)?; // lfsx f12,r11,r30
    fp::store_single(g, slot_b, fp::fmadd_single(pair.1, norm, old_b))?; // stfsx f11
    Ok(Sector::Back)
}

// ======================================================= sub_82B45B60: scale the gain array

/// `sub_82B45B60` — scale a speaker-gain array by `f1·f2`, power-normalised when `f3 < 1.0`.
///
/// `object` is `r3` and `gains` is `r6`; `gain_a` is `f1`, `gain_b` is `f2` and `spread` is `f3`.
/// Both callers take `f3` from `lfs f3,8(r30)`, i.e. from the source record's squared length, so
/// "`f3 < 1.0`" means "the source is strictly inside the unit disc". No result (`kReturnNone`).
///
/// Writes `gains + 0..+19` always, and `+20`/`+24` when the count reads 8 — where **the count is
/// reloaded after the first five stores**, so a `gains` array overlapping `object + 56` can change
/// the answer mid-call. The C++ `Windows()` declares the two extra words unconditionally in that
/// case rather than declining, because a superset compares equal.
///
/// Reads the count, [`ONE_SINGLE`], and — only on the normalising path, since the indices need not
/// be valid otherwise — the four indices and the four gains they select.
///
/// The normalisation is `Σ g[i]²` over the four selected gains, plus `g[1]²` when the count is ≥ 6,
/// plus `g[5]² + g[6]²` when it is 8; the scale is then divided by the square root of that. The
/// four indices are loaded in the order 176, 172, 180, 184 — not ascending — and the sum accumulates
/// in that order through one `fmuls` and three `fmadds`, so it is not reassociable.
pub fn scale_gains(
    g: &mut Guest,
    object: u32,
    gains: u32,
    gain_a: f64,
    gain_b: f64,
    spread: f64,
) -> Result<()> {
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();

    let mut scale = fp::mul_single(gain_a, gain_b); // fmuls f0,f1,f2
    let one = fp::load_single(g, ONE_SINGLE)?; // lfs f13,-22460(r11)

    // fcmpu cr6,f3,f13 ; bge cr6 — unordered is not lt, so a NaN f3 skips the normalisation.
    if spread < one {
        let index176 = g.u32(object + INDEX + 4)?; // lwz r11,176(r3)
        let index172 = g.u32(object + INDEX)?; // lwz r10,172(r3)
        let index180 = g.u32(object + INDEX + 8)?; // lwz r8,180(r3)
        let index184 = g.u32(object + INDEX + 12)?; // lwz r5,184(r3)
        let count = g.u32(object + COUNT)? as i32; // lwz r11,56(r3)
        let a = fp::load_single(g, gains.wrapping_add(word_offset(index176)))?; // lfsx f13
        let mut sum = fp::mul_single(a, a); // fmuls f12,f13,f13
        let b = fp::load_single(g, gains.wrapping_add(word_offset(index172)))?; // lfsx f11
        let c = fp::load_single(g, gains.wrapping_add(word_offset(index180)))?; // lfsx f10
        let d = fp::load_single(g, gains.wrapping_add(word_offset(index184)))?; // lfsx f9
        sum = fp::fmadd_single(b, b, sum); // fmadds f8,f11,f11,f12
        sum = fp::fmadd_single(c, c, sum); // fmadds f7,f10,f10,f8
        sum = fp::fmadd_single(d, d, sum); // fmadds f13,f9,f9,f7
        // blt cr6 — the centre channel joins the sum from a count of 6 up.
        if !(count < 6) {
            let e = fp::load_single(g, gains + 4)?; // lfs f12,4(r6)
            sum = fp::fmadd_single(e, e, sum); // fmadds f13,f12,f12,f13
            // cmpwi cr6,r11,8 ; bne cr6
            if count == 8 {
                let g6 = fp::load_single(g, gains + 24)?; // lfs f12,24(r6)
                let g6_squared = fp::mul_single(g6, g6); // fmuls f11,f12,f12
                let g5 = fp::load_single(g, gains + 20)?; // lfs f10,20(r6)
                let rear = fp::fmadd_single(g5, g5, g6_squared); // fmadds f9,f10,f10,f11
                sum = fp::add_single(rear, sum); // fadds f13,f9,f13
            }
        }
        // loc_82B45BE4.
        fpscr.disable_flush_mode_unconditional();
        scale = fp::div_single(scale, fp::sqrt_single(sum)); // fsqrts f13,f13 ; fdivs f0,f0,f13
    }

    // loc_82B45BEC: all five loads precede all five stores, as lifted, and the operand order of
    // each `fmuls` alternates between (gain, scale) and (scale, gain) exactly as the original has
    // it — see the module note on rule 4.
    fpscr.disable_flush_mode_unconditional();
    let g0 = fp::load_single(g, gains)?; // lfs f13,0(r6)
    let g1 = fp::load_single(g, gains + 4)?; // lfs f12,4(r6)
    let s0 = fp::mul_single(g0, scale); // fmuls f11,f13,f0
    let g2 = fp::load_single(g, gains + 8)?; // lfs f10,8(r6)
    let s1 = fp::mul_single(g1, scale); // fmuls f9,f12,f0
    let g3 = fp::load_single(g, gains + 12)?; // lfs f8,12(r6)
    let s2 = fp::mul_single(scale, g2); // fmuls f7,f0,f10
    let g4 = fp::load_single(g, gains + 16)?; // lfs f6,16(r6)
    let s3 = fp::mul_single(g3, scale); // fmuls f5,f8,f0
    let s4 = fp::mul_single(scale, g4); // fmuls f4,f0,f6
    fp::store_single(g, gains, s0)?; // stfs f11,0(r6)
    fp::store_single(g, gains + 4, s1)?; // stfs f9,4(r6)
    fp::store_single(g, gains + 8, s2)?; // stfs f7,8(r6)
    fp::store_single(g, gains + 12, s3)?; // stfs f5,12(r6)
    fp::store_single(g, gains + 16, s4)?; // stfs f4,16(r6)

    // lwz r11,56(r3) — reloaded after the five stores ; cmpwi cr6,r11,8 ; bnelr cr6
    if (g.u32(object + COUNT)? as i32) != 8 {
        return Ok(());
    }
    let g5 = fp::load_single(g, gains + 20)?; // lfs f13,20(r6)
    let g6 = fp::load_single(g, gains + 24)?; // lfs f12,24(r6)
    let s5 = fp::mul_single(scale, g5); // fmuls f11,f0,f13
    let s6 = fp::mul_single(g6, scale); // fmuls f10,f12,f0
    fp::store_single(g, gains + 20, s5)?; // stfs f11,20(r6)
    fp::store_single(g, gains + 24, s6) // stfs f10,24(r6)
}

// ====================================================== sub_82B45C50: laying out the panners

const LAYOUT_POOL: u32 = (((-32208i32 as u32) & 0xFFFF) << 16).wrapping_sub(31232);
/// `lfs f0,2120(r27)` — scales the centre angle and the turn.
pub const LAYOUT_ANGLE_SCALE: u32 = LAYOUT_POOL + 2120;
/// `lfs f26,2124(r27)` — scales the three spreads.
pub const LAYOUT_SPREAD_SCALE: u32 = LAYOUT_POOL + 2124;
/// `lis -32219 ; lfs 28632` — a first spread equal to this selects the mirrored pair.
pub const LAYOUT_MIRROR_MARKER: u32 = (((-32219i32 as u32) & 0xFFFF) << 16) + 28632;
/// `lis -32219 ; lfs 14524` — added to the angle on the mirrored path.
pub const LAYOUT_MIRROR_BIAS: u32 = (((-32219i32 as u32) & 0xFFFF) << 16) + 14524;
const _: () = assert!(LAYOUT_ANGLE_SCALE == 0x822F_8E48 && LAYOUT_SPREAD_SCALE == 0x822F_8E4C);
const _: () = assert!(LAYOUT_MIRROR_MARKER == 0x8225_6FD8 && LAYOUT_MIRROR_BIAS == 0x8225_38BC);
/// `stwu r1,-224(r1)`. Real: [`crate::mathlib::atan2`] spills its two arguments at `r1+16`/`+24`.
pub const LAYOUT_FRAME: u32 = 224;
/// 16 bytes an entry: `{x, y, lengthsq, angle}`.
pub const PANNER_ENTRY_BYTES: u32 = 16;
/// `stfs fN,12(rN)` — the angle this function stores.
pub const PANNER_ENTRY_ANGLE: u32 = 12;

/// `sub_82B45C50`'s float arguments, `f1` to `f7`.
#[derive(Clone, Copy, Debug)]
pub struct PannerLayout {
    /// `f1` — the centre's angle.
    pub angle: f64,
    /// `f2` — the centre's distance.
    pub distance: f64,
    /// `f3` — each entry's radius about the centre.
    pub radius: f64,
    /// `f4` — added to the centre angle before every spread.
    pub turn: f64,
    /// `f5`, `f6`, `f7` — the three spreads.
    pub spreads: [f64; 3],
}

/// Read the clamped position back out of the entry and store the angle of that direction.
fn store_panner_angle(g: &mut Guest, entry: u32, sp: u32) -> Result<()> {
    let x = fp::load_single(g, entry)?; // lfs f2,0(rN)
    let y = fp::load_single(g, entry.wrapping_add(4))?; // lfs f1,4(rN)
    let angle = crate::mathlib::atan2(g, y, x, sp)?; // bl 0x82f52318
    fp::store_single(g, entry.wrapping_add(PANNER_ENTRY_ANGLE), fp::frsp(angle)) // frsp ; stfs
}

/// One placement: `(cos, sin)` of `angle` times `radius`, about the centre, clamped, then its angle.
/// The two scaled components come back because the mirrored pair reuses them negated.
#[allow(clippy::too_many_arguments)]
fn place_entry<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    entry: u32,
    angle: f64,
    radius: f64,
    centre: (f64, f64),
    sp: u32,
) -> Result<(f64, f64)> {
    let cosine = fp::frsp(trig.cosine(g, angle)?); // bl 0x82f4dfb0 ; frsp f0,f1
    let dx = fp::mul_single(cosine, radius); // fmuls fN,f0,f31
    let sine = fp::frsp(trig.sine(g, angle)?); // bl 0x82f4ded0 ; frsp f13,f1
    let x = fp::add_single(dx, centre.0); // fadds f1,fN,f30
    let dy = fp::mul_single(sine, radius); // fmuls f12,f13,f31
    let y = fp::add_single(dy, centre.1); // fadds f2,f12,f29
    clamp_to_unit_disc(g, entry, x, y)?; // bl 0x82b453d8
    store_panner_angle(g, entry, sp)?;
    Ok((dx, dy))
}

/// Lay a source's panner entries out around a centre point (`sub_82B45C50`).
///
/// `entries` is `r3`, `count` the low word of `r4`, `sp` is `r1`. The centre is `(cos, sin)` of the
/// scaled angle times the distance. Each entry sits at the centre plus `(cos, sin)` of the centre
/// angle plus the scaled turn, offset by one of three scaled spreads, times the radius; it is clamped
/// into the unit disc by [`clamp_to_unit_disc`], and the angle of the clamped direction is stored at
/// `+12` through [`crate::mathlib::atan2`].
///
/// Which entries are written depends only on the count: 1 hands the whole job to [`place_panner`];
/// 2 writes entries 0 and 1 — a mirrored pair when the first spread equals the pool marker, which
/// reuses the first placement's offsets negated instead of asking for a second sine; 4 writes 0..3,
/// 6 writes 0..4 and 8 writes 0..6. Counts 3, 5 and 7 fall through the branch table, and the unsigned
/// `count - 2 > 6` test drops 0 and every negative count: none of those write anything.
pub fn lay_out_panners<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    entries: u32,
    count: i32,
    args: PannerLayout,
    sp: u32,
) -> Result<()> {
    let frame = sp.wrapping_sub(LAYOUT_FRAME);
    g.set_u32(frame, sp)?; // stwu r1,-224(r1)
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    if count == 1 {
        return place_panner(g, trig, entries, args.angle, args.distance); // bl 0x82b269c0
    }
    let scale = fp::load_single(g, LAYOUT_ANGLE_SCALE)?; // lfs f0,2120(r27)
    let angle_a = fp::mul_single(args.angle, scale); // fmuls f28,f1,f0
    let angle_b = fp::mul_single(args.turn, scale); // fmuls f27,f4,f0
    let centre_cos = fp::frsp(trig.cosine(g, angle_a)?);
    let centre_x = fp::mul_single(centre_cos, args.distance); // fmuls f30,f0,f29
    let centre_sin = fp::frsp(trig.sine(g, angle_a)?);
    let centre_y = fp::mul_single(centre_sin, args.distance); // fmuls f29,f13,f29
    let centre = (centre_x, centre_y);
    let at = |index: u32| entries.wrapping_add(index << 4);

    let selector = (count as u32).wrapping_sub(2); // addi r11,r26,-2 ; cmplwi cr6,r11,6
    if selector > 6 {
        return Ok(());
    }
    let spread_scale = fp::load_single(g, LAYOUT_SPREAD_SCALE)?; // lfs f26,2124(r27)
    let [spread5, spread6, spread7] = args.spreads;
    if selector == 0 {
        let marker = fp::load_single(g, LAYOUT_MIRROR_MARKER)?;
        if spread5 == marker {
            let sum = fp::add_single(angle_a, angle_b); // fadds f13,f28,f27
            let bias = fp::load_single(g, LAYOUT_MIRROR_BIAS)?;
            let angle = fp::add_single(sum, bias); // fadds f28,f13,f0
            let (dx, dy) = place_entry(g, trig, at(0), angle, args.radius, centre, frame)?;
            let second = at(1); // addi r31,r31,16
            let y = fp::sub_single(centre_y, dy); // fsubs f2,f29,f31
            let x = fp::sub_single(centre_x, dx); // fsubs f1,f30,f28
            clamp_to_unit_disc(g, second, x, y)?;
            store_panner_angle(g, second, frame)?;
        } else {
            let reload = fp::load_single(g, LAYOUT_SPREAD_SCALE)?; // lfs f0,2124(r27)
            let sum = fp::add_single(angle_a, angle_b);
            let offset = fp::mul_single(spread5, reload); // fmuls f27,f24,f0
            place_entry(
                g,
                trig,
                at(0),
                fp::add_single(sum, offset),
                args.radius,
                centre,
                frame,
            )?;
            place_entry(
                g,
                trig,
                at(1),
                fp::sub_single(sum, offset),
                args.radius,
                centre,
                frame,
            )?;
        }
    } else if selector % 2 == 0 {
        // 8 falls into 6 falls into 4.
        let (mut b, mut c, mut d) = (1, 2, 3); // li r30,1 ; li r29,2 ; li r28,3
        if selector == 6 {
            let offset7 = fp::mul_single(spread7, spread_scale); // fmuls f25,f25,f26
            let sum = fp::add_single(angle_a, angle_b); // fadds f22,f28,f27
            place_entry(
                g,
                trig,
                at(5),
                fp::add_single(sum, offset7),
                args.radius,
                centre,
                frame,
            )?;
            place_entry(
                g,
                trig,
                at(6),
                fp::sub_single(sum, offset7),
                args.radius,
                centre,
                frame,
            )?;
        }
        if selector >= 4 {
            let sum = fp::add_single(angle_a, angle_b); // fadds f25,f28,f27
            place_entry(g, trig, at(1), sum, args.radius, centre, frame)?;
            (b, c, d) = (2, 3, 4);
        }
        let sum = fp::add_single(angle_a, angle_b); // fadds f28,f28,f27
        let offset5 = fp::mul_single(spread5, spread_scale); // fmuls f25,f24,f26
        let offset6 = fp::mul_single(spread6, spread_scale); // fmuls f27,f23,f26
        place_entry(
            g,
            trig,
            at(0),
            fp::add_single(sum, offset5),
            args.radius,
            centre,
            frame,
        )?;
        place_entry(
            g,
            trig,
            at(b),
            fp::sub_single(sum, offset5),
            args.radius,
            centre,
            frame,
        )?;
        place_entry(
            g,
            trig,
            at(c),
            fp::add_single(sum, offset6),
            args.radius,
            centre,
            frame,
        )?;
        place_entry(
            g,
            trig,
            at(d),
            fp::sub_single(sum, offset6),
            args.radius,
            centre,
            frame,
        )?;
    }
    Ok(())
}

// ============================================================ sub_82B460A0: the mix matrix

/// `lwz r11,56(r3)` — the destination channel count, reloaded after the panning loop.
pub const MATRIX_DEST_COUNT: u32 = 56;
/// A matrix row: eight singles, one per destination.
pub const MATRIX_ROW_BYTES: u32 = 32;
/// Eight rows, one per source.
pub const MATRIX_ROWS: u32 = 8;
/// `lfs f3,8(r30)` — the entry field handed to [`scale_gains`].
pub const PANNER_ENTRY_LENGTH_SQ: u32 = 8;

/// `sub_82B460A0`'s float arguments, `f1` to `f4`.
#[derive(Clone, Copy, Debug)]
pub struct MatrixGains {
    /// `f1`.
    pub weight: f64,
    /// `f2` — handed to [`pan_distance`] and [`add_angular`].
    pub focus: f64,
    /// `f3` — stored into the column (or crossing cell) the tail fills.
    pub fill: f64,
    /// `f4`.
    pub gain: f64,
}

/// Fill one 8x8 mix matrix from a source's panner entries (`sub_82B460A0`).
///
/// `object` is `r3` (the configuration [`pan_distance`], [`add_angular`] and [`scale_gains`] take),
/// `entries` `r4`, `sources` `r5`, and `matrix` is `r10` — not a positional argument, the original
/// moves it at entry. A stereo destination gets two gains per source from the entry's `y` alone:
/// `right = (y + 1) / 2`, `left = 1 - right`, normalised by one fused `fmadds` and a single-rounded
/// square root. Any other destination pans each source through the three callees into its own row —
/// one row fewer from six sources up — then, for six or more destinations (the count **reloaded**),
/// zeroes one column of every row and one whole row and writes their crossing cell, or fills the
/// column when the source count is neither 6 nor 8.
///
/// The original's 176-byte frame is not reproduced: no Rust callee writes below the stack pointer.
pub fn fill_mix_matrix<T: Trig>(
    g: &mut Guest,
    trig: &mut T,
    object: u32,
    entries: u32,
    sources: i32,
    matrix: u32,
    args: MatrixGains,
) -> Result<()> {
    let dest = g.u32(object.wrapping_add(MATRIX_DEST_COUNT))? as i32; // lwz r11,56(r3)
    let mut fpscr = Fpscr::capture();
    fpscr.disable_flush_mode_unconditional();
    if dest == 2 {
        if sources > 0 {
            let half = fp::load_single(g, HALF_SINGLE)?; // lfs f12,-26788(r9)
            let one = fp::load_single(g, ONE_SINGLE)?; // lfs f0,-22460(r8)
            let mut row = matrix.wrapping_sub(28); // addi r11,r10,-28
            let mut entry = entries.wrapping_sub(12); // addi r10,r4,-12
            for _ in 0..sources as u32 {
                entry = entry.wrapping_add(PANNER_ENTRY_BYTES); // lfsu f13,16(r10)
                let y = fp::load_single(g, entry)?;
                let biased = fp::add_single(y, one); // fadds f13,f13,f0
                let right = fp::mul_single(biased, half); // fmuls f11,f13,f12
                let left = fp::sub_single(one, right); // fsubs f10,f0,f11
                let norm = fp::fmadd_single(left, left, fp::mul_single(right, right)); // fmadds
                let length = fp::sqrt_single(norm); // fsqrts f7,f8
                let scale = fp::div_single(one, length); // fdivs f6,f0,f7
                let total = fp::mul_single(fp::mul_single(scale, args.weight), args.gain);
                fp::store_single(g, row.wrapping_add(28), fp::mul_single(total, right))?; // stfs f3,28(r11)
                let second = fp::mul_single(left, total); // fmuls f2,f10,f4
                row = row.wrapping_add(MATRIX_ROW_BYTES); // stfsu f2,32(r11)
                fp::store_single(g, row, second)?;
            }
        }
        return Ok(());
    }

    // loc_82B4614C
    let rows = if sources < 6 { sources } else { sources - 1 }; // addi r29,r26,-1 ; bge ; mr
    if rows > 0 {
        let (mut entry, mut row) = (entries, matrix);
        for _ in 0..rows {
            pan_distance(g, object, entry, row, args.focus)?; // bl 0x82b454b8
            add_angular(g, trig, object, entry, row, args.focus)?; // bl 0x82b45788
            let lensq = fp::load_single(g, entry.wrapping_add(PANNER_ENTRY_LENGTH_SQ))?; // lfs f3,8(r30)
            scale_gains(g, object, row, args.weight, args.gain, lensq)?; // bl 0x82b45b60
            row = row.wrapping_add(MATRIX_ROW_BYTES);
            entry = entry.wrapping_add(PANNER_ENTRY_BYTES);
        }
    }
    // loc_82B461B4: reloaded, after every store above.
    fpscr.disable_flush_mode_unconditional();
    let dest_now = g.u32(object.wrapping_add(MATRIX_DEST_COUNT))? as i32;
    if dest_now >= 6 {
        let column: u32 = if dest_now == 6 { 5 } else { 7 };
        if sources == 6 || sources == 8 {
            let which_row: u32 = if sources == 6 { 5 } else { 7 };
            let row_base = matrix.wrapping_add(which_row * MATRIX_ROW_BYTES); // add r7,r10,r27
            let col_base = matrix.wrapping_add(column << 2); // add r10,r8,r27
            let zero = fp::load_single(g, ZERO_SINGLE)?; // lfs f0,23056(r11)
            let mut col = col_base.wrapping_sub(MATRIX_ROW_BYTES);
            let mut cell = row_base.wrapping_sub(4);
            for _ in 0..MATRIX_ROWS {
                col = col.wrapping_add(MATRIX_ROW_BYTES); // stfsu f0,32(r10)
                fp::store_single(g, col, zero)?;
                cell = cell.wrapping_add(4); // stfsu f0,4(r9)
                fp::store_single(g, cell, zero)?;
            }
            fp::store_single(g, row_base.wrapping_add(column << 2), args.fill)?; // stfsx f28,r8,r7
        } else {
            // loc_82B46230: matrix + 4*(column - 8), then eight rows down, all 32-bit.
            let mut col = matrix.wrapping_add(column.wrapping_sub(8) << 2);
            for _ in 0..MATRIX_ROWS {
                col = col.wrapping_add(MATRIX_ROW_BYTES); // stfsu f28,32(r10)
                fp::store_single(g, col, args.fill)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathlib::tests::Scripted;

    // The work area. The object needs 188 bytes plus the speaker pair a test index selects; the
    // record is 16 and the gain array 32, which is what `sub_82B460A0` advances by.
    const OBJECT: u32 = 0x4000_0000;
    const RECORD: u32 = 0x4000_0200;
    const GAINS: u32 = 0x4000_0240;

    /// Every rodata cell these bodies read, as its own small segment at the address the module
    /// computes, holding the **word measured in the validated image dump**. One segment per cell
    /// rather than one big rodata window on purpose: a port that read the wrong address would get an
    /// `Err` from [`Guest`] instead of a plausible number.
    fn rodata(g: &mut Guest) {
        for (addr, word) in [
            (ONE_SINGLE, 0x3F80_0000u32),      // 1.0
            (SNAP_SINGLE, 0x3F7F_BE77),        // 0.999
            (ZERO_SINGLE, 0x0000_0000),        // 0.0
            (HALF_SINGLE, 0x3F00_0000),        // 0.5
            (EPSILON_SINGLE, 0x3A03_126F),     // 0.0005
            (TURNS_PER_RADIAN, 0x3E22_F983),   // 1/2pi
            (DEGREES_TO_RADIANS, 0xBC8E_FA35), // -pi/180
            (RADIANS_PER_TURN, 0x40C9_0FDB),   // 2pi
            (HALF_TURN, 0x4049_0FDB),          // pi
        ] {
            g.put(addr, word.to_be_bytes().to_vec());
        }
        crate::mathlib::tests::with_pool_segments(g);
    }

    fn guest() -> Guest {
        let mut g = Guest::from_segments(vec![crate::Segment {
            base: OBJECT,
            bytes: vec![0u8; 0x300],
        }]);
        rodata(&mut g);
        g
    }

    fn put_f32(g: &mut Guest, ea: u32, v: f32) {
        g.set_u32(ea, v.to_bits()).unwrap();
    }
    fn get_f32(g: &Guest, ea: u32) -> f32 {
        g.f32(ea).unwrap()
    }

    /// The seven speaker positions and the four selected indices, written into the object.
    ///
    /// Speaker 1 is the centre (`gains + 4`), 5 and 6 the 7.1 sides (`gains + 20`/`+24`), which is
    /// the reading `probe/ports/notes/sub_82B454B8.md` records as an inference from `8*index`.
    fn layout(g: &mut Guest, count: i32, indices: [u32; 4], speakers: [(f32, f32); 7]) {
        for (k, (x, y)) in speakers.iter().enumerate() {
            put_f32(g, OBJECT + 8 * k as u32, *x);
            put_f32(g, OBJECT + 8 * k as u32 + 4, *y);
        }
        g.set_u32(OBJECT + COUNT, count as u32).unwrap();
        for (k, i) in indices.iter().enumerate() {
            g.set_u32(OBJECT + INDEX + 4 * k as u32, *i).unwrap();
        }
    }

    /// A 5.1/7.1-shaped layout: the four corners selected, centre at the front, sides at the flanks.
    /// `front` is `+x`, since the body computes `front = 0.5*(x + 1)`.
    const SPEAKERS: [(f32, f32); 7] = [
        (1.0, -1.0),  // 0 -> gains+0
        (1.0, 0.0),   // 1 -> gains+4, the centre
        (1.0, 1.0),   // 2 -> gains+8
        (-1.0, -1.0), // 3 -> gains+12
        (-1.0, 1.0),  // 4 -> gains+16
        (0.0, -1.0),  // 5 -> gains+20
        (0.0, 1.0),   // 6 -> gains+24
    ];
    const INDICES: [u32; 4] = [0, 2, 3, 4];

    /// `weight = 1 - 0.5*|speaker - source|`, written from the prose in
    /// `probe/ports/notes/sub_82B454B8.md` rather than from the body's loop.
    fn model_weight(s: (f32, f32), x: f32, y: f32) -> f64 {
        let dx = fp::sub_single(s.0 as f64, x as f64);
        let dy = fp::sub_single(s.1 as f64, y as f64);
        let q = fp::fmadd_single(dy, dy, fp::mul_single(dx, dx));
        fp::nmsub_single(fp::sqrt_single(q), 0.5, 1.0)
    }

    fn record(g: &mut Guest, x: f32, y: f32, lensq: f32, angle: f32) {
        put_f32(g, RECORD + RECORD_X, x);
        put_f32(g, RECORD + RECORD_Y, y);
        put_f32(g, RECORD + RECORD_LENGTH_SQ, lensq);
        put_f32(g, RECORD + RECORD_ANGLE, angle);
    }

    // =========================================================== sub_82B453D8

    #[test]
    fn the_clamp_stores_the_position_and_a_fused_squared_length() {
        let mut g = guest();
        clamp_to_unit_disc(&mut g, RECORD, 0.5, 0.25).unwrap();
        assert_eq!(get_f32(&g, RECORD + RECORD_X), 0.5);
        assert_eq!(get_f32(&g, RECORD + RECORD_Y), 0.25);
        // 0.25 + 0.0625, exact in a single, so this is the plain arithmetic check.
        assert_eq!(get_f32(&g, RECORD + RECORD_LENGTH_SQ), 0.3125);

        // The fusion, pinned by an input where it is visible — and one that stays *inside* the disc,
        // since a squared length above 1.0 would be rewritten by the clamp before it could be read.
        // x = 0.5 + 2^-24 is exact as a single and x*x = 0.25 + 2*ulp + 2^-48 needs 47 bits, so a
        // separate `fmuls` rounds the 2^-48 away first. Adding y*y = 0.5*ulp then lands on a tie the
        // unfused form breaks downward (to even) and the fused form breaks upward, because the 2^-48
        // is still there: 0.25 + 2*ulp against 0.25 + 3*ulp, with ulp = 2^-25.
        let ulp = 2f32.powi(-25);
        let x = 0.5f32 + 2f32.powi(-24);
        let y = 2f32.powi(-13);
        clamp_to_unit_disc(&mut g, RECORD, x as f64, y as f64).unwrap();
        let fused = 0.25f32 + 3.0 * ulp;
        let unfused = 0.25f32 + 2.0 * ulp;
        assert_ne!(fused, unfused);
        assert_eq!(
            get_f32(&g, RECORD + RECORD_LENGTH_SQ),
            fused,
            "fmadds rounds once; a multiply then an add would give {unfused}"
        );
    }

    #[test]
    fn a_length_inside_the_snap_window_reports_one_and_keeps_the_position() {
        // The behaviour worth a test of its own: between 0.999 and 1.0 the length is rewritten to
        // exactly 1.0 while x and y are left where they were. A reader who assumed "clamped to the
        // unit disc" meant the position was normalised would be wrong here.
        let mut g = guest();
        let x = 0.9998f32; // x*x = 0.99960004, inside (0.999, 1)
        clamp_to_unit_disc(&mut g, RECORD, x as f64, 0.0).unwrap();
        assert_eq!(get_f32(&g, RECORD + RECORD_LENGTH_SQ), 1.0);
        assert_eq!(
            get_f32(&g, RECORD + RECORD_X),
            x,
            "the position is NOT rescaled"
        );
        assert_eq!(get_f32(&g, RECORD + RECORD_Y), 0.0);

        // Exactly on the cell returns early instead: the test is `>`, not `>=`. Reaching that
        // boundary needs an `x*x` that lands on the cell **exactly**, which 0.999 has no short
        // square root for — so the cell is patched to 0.25 and x set to 0.5. That is legitimate
        // rather than a dodge: the body loads the cell live, which is the whole reason it can be.
        //
        // The first attempt used `sqrt(0.999)` rounded to a single and squared back, which misses
        // the cell by an ulp in one direction or the other; both forms of the test then agree and
        // the control passed. Recorded because that is the shape of a boundary test that is not on
        // the boundary.
        let mut h = guest();
        h.put(SNAP_SINGLE, 0.25f32.to_bits().to_be_bytes().to_vec());
        clamp_to_unit_disc(&mut h, RECORD, 0.5, 0.0).unwrap();
        assert_eq!(
            get_f32(&h, RECORD + RECORD_LENGTH_SQ),
            0.25,
            "exactly on the cell is left alone, not snapped to 1.0"
        );
    }

    #[test]
    fn a_length_above_one_normalises_the_position() {
        let mut g = guest();
        clamp_to_unit_disc(&mut g, RECORD, 3.0, 4.0).unwrap();
        // Derived from the documented shape: len = sqrt(lensq), scale = 1/len, position *= scale.
        let scale = fp::div_single(1.0, fp::sqrt_single(25.0));
        assert_eq!(
            get_f32(&g, RECORD + RECORD_X),
            fp::mul_single(scale, 3.0) as f32
        );
        assert_eq!(
            get_f32(&g, RECORD + RECORD_Y),
            fp::mul_single(scale, 4.0) as f32
        );
        assert_eq!(get_f32(&g, RECORD + RECORD_LENGTH_SQ), 1.0);
        // And the hand-computed answer, as a second anchor: 3/5 and 4/5.
        assert!((get_f32(&g, RECORD + RECORD_X) - 0.6).abs() < 1e-6);
        assert!((get_f32(&g, RECORD + RECORD_Y) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_nan_position_leaves_the_length_alone_rather_than_snapping_it() {
        // Both compares are unordered, so a NaN takes `bge` to the second test and then `blelr`
        // out: the three entry stores stand and nothing is rewritten. A translation that wrote
        // `length_sq >= one` for the first branch, or `<=` for the second, would snap it to 1.0.
        let mut g = guest();
        put_f32(&mut g, RECORD + RECORD_LENGTH_SQ, 7.0);
        clamp_to_unit_disc(&mut g, RECORD, f64::NAN, 0.25).unwrap();
        assert!(get_f32(&g, RECORD + RECORD_X).is_nan());
        assert_eq!(get_f32(&g, RECORD + RECORD_Y), 0.25);
        assert!(
            get_f32(&g, RECORD + RECORD_LENGTH_SQ).is_nan(),
            "not 1.0, and not 7.0"
        );
    }

    // =========================================================== sub_82B269C0

    #[test]
    fn placing_a_panner_converts_degrees_and_asks_for_the_cosine_first() {
        let mut g = guest();
        let mut trig = Scripted {
            sine: 0.5,
            cosine: 0.25,
            ..Default::default()
        };
        place_panner(&mut g, &mut trig, RECORD, 90.0, 1.0).unwrap();

        // The cell is -pi/180, so 90 degrees is -pi/2 radians. Both helpers see the same value, and
        // the cosine is asked first -- the order the lifted `bl`s are in.
        let expected = fp::mul_single(90.0, fp::load_single(&g, DEGREES_TO_RADIANS).unwrap());
        assert_eq!(trig.asked, vec![('c', expected), ('s', expected)]);
        assert!(
            (expected + std::f64::consts::FRAC_PI_2).abs() < 1e-6,
            "{expected} is -pi/2"
        );

        // x = cos*depth, y = sin*depth, then through the clamp: 0.25^2 + 0.5^2 = 0.3125.
        assert_eq!(get_f32(&g, RECORD + RECORD_X), 0.25);
        assert_eq!(get_f32(&g, RECORD + RECORD_Y), 0.5);
        assert_eq!(get_f32(&g, RECORD + RECORD_LENGTH_SQ), 0.3125);
        assert_eq!(get_f32(&g, RECORD + RECORD_ANGLE), expected as f32);
    }

    #[test]
    fn a_non_positive_depth_biases_the_stored_angle_by_half_a_turn() {
        // A negative radius is a reflection through the origin, and the measured cell is pi, so the
        // bias is exactly that. The compare is unordered, so a NaN depth takes the bias path too --
        // which is the branch a translation writing `depth <= zero` would get right and one writing
        // `!(depth > zero)` on a reordered test would not.
        for (depth, biased) in [(1.0f64, false), (0.0, true), (-1.0, true), (f64::NAN, true)] {
            let mut g = guest();
            let mut trig = Scripted {
                sine: 0.5,
                cosine: 0.25,
                ..Default::default()
            };
            place_panner(&mut g, &mut trig, RECORD, 90.0, depth).unwrap();
            let angle = fp::mul_single(90.0, fp::load_single(&g, DEGREES_TO_RADIANS).unwrap());
            let pi = fp::load_single(&g, HALF_TURN).unwrap();
            let want = if biased {
                fp::add_single(angle, pi)
            } else {
                angle
            };
            assert_eq!(
                get_f32(&g, RECORD + RECORD_ANGLE),
                want as f32,
                "depth {depth} should {}have been biased",
                if biased { "" } else { "not " }
            );
        }
    }

    #[test]
    fn the_unported_trig_fails_the_call_before_anything_is_stored() {
        // `Unported` is the default for a reason: a sine that is right to fifteen digits and wrong
        // in the bits the `frsp` keeps would make every future comparison of this function pass
        // vacuously. The failure has to arrive before the record is touched.
        let mut g = guest();
        put_f32(&mut g, RECORD + RECORD_X, 9.0);
        let err =
            place_panner(&mut g, &mut crate::mathlib::Unported, RECORD, 90.0, 1.0).unwrap_err();
        assert_eq!(
            err.address, 0x82F4_DFB0,
            "the cosine is asked first, so it fails first"
        );
        assert_eq!(get_f32(&g, RECORD + RECORD_X), 9.0, "nothing was written");
    }

    // =========================================================== sub_82B454B8

    #[test]
    fn a_degenerate_source_zeroes_the_gain_array() {
        // lensq == 1.0 exactly: five raw word stores, then the two 7.1 words only when the count
        // reads 8. The 5.1 case has to leave +20/+24 alone.
        for (count, sides_cleared) in [(6i32, false), (8, true)] {
            let mut g = guest();
            layout(&mut g, count, INDICES, SPEAKERS);
            record(&mut g, 0.25, 0.25, 1.0, 0.0);
            for slot in 0..8u32 {
                put_f32(&mut g, GAINS + 4 * slot, 9.0);
            }
            pan_distance(&mut g, OBJECT, RECORD, GAINS, 1.0).unwrap();
            for slot in 0..5u32 {
                assert_eq!(
                    g.u32(GAINS + 4 * slot).unwrap(),
                    0,
                    "count {count} slot {slot}"
                );
            }
            let want = if sides_cleared { 0.0 } else { 9.0 };
            assert_eq!(get_f32(&g, GAINS + 20), want, "count {count}");
            assert_eq!(get_f32(&g, GAINS + 24), want, "count {count}");
            assert_eq!(
                get_f32(&g, GAINS + 28),
                9.0,
                "nothing past +27 is ever written"
            );
        }
    }

    #[test]
    fn the_count_is_reloaded_after_the_five_zero_stores() {
        // The reload is not decoration. Overlap the gain array with the count word so that the five
        // zero stores clear it: the entry count is 8, the reloaded one is 0, and the two 7.1 words
        // are therefore NOT written. Hoisting the count would write them.
        //
        // The C++ `Windows()` handles this layout by declaring the two extra words unconditionally
        // (a superset compares equal) rather than declining, so it is bracketed -- but the entry
        // value still cannot predict the branch, which is what this pins.
        let mut g = guest();
        let gains = OBJECT + 44; // gains+12 lands exactly on OBJECT+56
        assert_eq!(gains + 12, OBJECT + COUNT);
        layout(&mut g, 8, INDICES, SPEAKERS);
        record(&mut g, 0.25, 0.25, 1.0, 0.0);
        put_f32(&mut g, gains + 20, 9.0);
        put_f32(&mut g, gains + 24, 9.0);
        pan_distance(&mut g, OBJECT, RECORD, gains, 1.0).unwrap();
        assert_eq!(
            g.u32(OBJECT + COUNT).unwrap(),
            0,
            "the store landed on the count"
        );
        assert_eq!(
            get_f32(&g, gains + 20),
            9.0,
            "the reloaded count is 0, so no 7.1 words"
        );
        assert_eq!(get_f32(&g, gains + 24), 9.0);
    }

    #[test]
    fn a_source_at_the_origin_spreads_evenly_over_the_four_selected_speakers() {
        // Every distance is sqrt(2), front and back are both 0.5, and the depth scale is 1, so all
        // four gains must come out equal -- and equal to 0.5, which is derivable by hand:
        // k = sqrt(front / 2w^2) = 0.5/w, so k*w = 0.5.
        let mut g = guest();
        layout(&mut g, 4, INDICES, SPEAKERS);
        record(&mut g, 0.0, 0.0, 0.0, 0.0);
        for slot in 0..8u32 {
            put_f32(&mut g, GAINS + 4 * slot, 9.0);
        }
        pan_distance(&mut g, OBJECT, RECORD, GAINS, 1.0).unwrap();

        let w = model_weight(SPEAKERS[0], 0.0, 0.0);
        assert!(
            (w - (1.0 - 0.5 * 2f64.sqrt())).abs() < 1e-7,
            "weight is 1 - 0.5*sqrt(2)"
        );
        let four: Vec<f32> = [0u32, 8, 12, 16]
            .iter()
            .map(|o| get_f32(&g, GAINS + o))
            .collect();
        for v in &four {
            assert!((v - 0.5).abs() < 1e-6, "{v} should be 0.5");
        }
        assert!(
            four.windows(2).all(|p| p[0] == p[1]),
            "the symmetry has to be exact: {four:?}"
        );
        // count == 4 stops before the centre and the sides.
        assert_eq!(get_f32(&g, GAINS + 4), 9.0, "no centre below a count of 6");
        assert_eq!(get_f32(&g, GAINS + 20), 9.0);
        assert_eq!(get_f32(&g, GAINS + 24), 9.0);
    }

    #[test]
    fn count_six_adds_the_centre_and_count_eight_the_two_sides() {
        // Hand-derived, not modelled. Source at the origin: w = 1 - 0.5*sqrt(2) = 0.292893 for the
        // four corners, 0.5 for speakers 1, 5 and 6 (all at distance 1).
        //   count 6: sum = (0.5*f1)^2 + 2w^2 = 0.4215729, k = sqrt(0.5/sum) = 1.089052
        //            corner gain = k*w = 0.318977, centre = k*0.5 = 0.544526
        //            spread = 2w^2 = 0.1715729, k2 = sqrt(0.5/spread) = 1.707107, rear = 0.5
        //   count 8: spread also carries w5^2 + w6^2, so spread = 0.6715729, k2 = 0.862859,
        //            rear = 0.252727 and each side = k2*0.5 = 0.431430
        let mut g = guest();
        layout(&mut g, 6, INDICES, SPEAKERS);
        record(&mut g, 0.0, 0.0, 0.0, 0.0);
        for slot in 0..8u32 {
            put_f32(&mut g, GAINS + 4 * slot, 9.0);
        }
        pan_distance(&mut g, OBJECT, RECORD, GAINS, 1.0).unwrap();
        assert!(
            (get_f32(&g, GAINS) - 0.318977).abs() < 1e-5,
            "{}",
            get_f32(&g, GAINS)
        );
        assert!((get_f32(&g, GAINS + 8) - 0.318977).abs() < 1e-5);
        assert!(
            (get_f32(&g, GAINS + 4) - 0.544526).abs() < 1e-5,
            "the centre"
        );
        assert!(
            (get_f32(&g, GAINS + 12) - 0.5).abs() < 1e-5,
            "the rear pair keeps 2w^2"
        );
        assert!((get_f32(&g, GAINS + 16) - 0.5).abs() < 1e-5);
        assert_eq!(get_f32(&g, GAINS + 20), 9.0, "no 7.1 words at a count of 6");
        assert_eq!(get_f32(&g, GAINS + 24), 9.0);

        let mut h = guest();
        layout(&mut h, 8, INDICES, SPEAKERS);
        record(&mut h, 0.0, 0.0, 0.0, 0.0);
        pan_distance(&mut h, OBJECT, RECORD, GAINS, 1.0).unwrap();
        assert!(
            (get_f32(&h, GAINS) - 0.318977).abs() < 1e-5,
            "the front is unchanged by the sides"
        );
        assert!(
            (get_f32(&h, GAINS + 12) - 0.252727).abs() < 1e-5,
            "{}",
            get_f32(&h, GAINS + 12)
        );
        assert!((get_f32(&h, GAINS + 16) - 0.252727).abs() < 1e-5);
        assert!(
            (get_f32(&h, GAINS + 20) - 0.431430).abs() < 1e-5,
            "{}",
            get_f32(&h, GAINS + 20)
        );
        assert!((get_f32(&h, GAINS + 24) - 0.431430).abs() < 1e-5);
    }

    #[test]
    fn the_front_and_back_shares_snap_to_zero_inside_the_epsilon_band() {
        // front = 0.5*(x+1), so x just above -1 puts it inside 0.0005 of zero and the whole front
        // pair comes out an exact zero -- not merely small, because the snapped share makes
        // `k = sqrt(0/sum)` exactly zero and every later product with it vanishes.
        //
        // Measured while writing this: the zero arrives **signed**. At x = -0.9999 the four corner
        // weights are negative (the source is more than two units from the front speakers, so
        // 1 - 0.5*distance < 0), and `0 * negative` is -0.0. That is the original's answer too; the
        // mask compares the stored word, so the sign is part of it. Hence the magnitude test plus an
        // explicit note, rather than an assertion that it is +0.0.
        let mut g = guest();
        layout(&mut g, 4, INDICES, SPEAKERS);
        record(&mut g, -0.9999, 0.0, 0.5, 0.0);
        pan_distance(&mut g, OBJECT, RECORD, GAINS, 1.0).unwrap();
        assert_eq!(
            get_f32(&g, GAINS).to_bits() & 0x7FFF_FFFF,
            0,
            "the front pair is a zero"
        );
        assert_eq!(get_f32(&g, GAINS + 8).to_bits() & 0x7FFF_FFFF, 0);
        assert_eq!(
            get_f32(&g, GAINS).to_bits(),
            0x8000_0000,
            "and it is -0.0 here"
        );
        assert_ne!(
            get_f32(&g, GAINS + 12),
            0.0,
            "and the back pair is not zero at all"
        );

        let mut h = guest();
        layout(&mut h, 4, INDICES, SPEAKERS);
        record(&mut h, 1.0, 0.0, 0.5, 0.0);
        pan_distance(&mut h, OBJECT, RECORD, GAINS, 1.0).unwrap();
        assert_eq!(
            get_f32(&h, GAINS + 12).to_bits() & 0x7FFF_FFFF,
            0,
            "the back pair is a zero"
        );
        assert_eq!(get_f32(&h, GAINS + 16).to_bits() & 0x7FFF_FFFF, 0);
        assert_ne!(get_f32(&h, GAINS), 0.0);
    }

    #[test]
    fn the_count_is_reloaded_between_the_index_stores_and_the_fixed_ones() {
        // The main path reloads the count twice more, after the four index stores. Select an index
        // whose `4*index` lands on the count word: the third store writes a float's bits there, and
        // the two reloads then see a value that is neither 8 nor below 6. So the centre word is
        // written and the two 7.1 words are not -- where the entry count of 8 would have written
        // all three.
        //
        // This is the layout the C++ `Windows()` supersets rather than declines. Nothing establishes
        // what the guest does with it; what it establishes is that the reload survived translation.
        let mut g = guest();
        let indices = [1u32, 2, 14, 4]; // 4*14 == 56 == COUNT
        assert_eq!(word_offset(14), COUNT);
        let mut speakers = SPEAKERS;
        speakers[1] = (1.0, 0.0);
        layout(&mut g, 8, indices, speakers);
        // Index 14's speaker pair is object + 8*14 = +112, inside the sector blocks this function
        // never reads. Give it a position so the weight is finite.
        put_f32(&mut g, OBJECT + 112, -1.0);
        put_f32(&mut g, OBJECT + 116, -1.0);
        record(&mut g, 0.0, 0.0, 0.0, 0.0);
        put_f32(&mut g, OBJECT + 20, 9.0); // gains+20, i.e. object+20
        put_f32(&mut g, OBJECT + 24, 9.0);
        pan_distance(&mut g, OBJECT, RECORD, OBJECT, 1.0).unwrap();

        let reloaded = g.u32(OBJECT + COUNT).unwrap() as i32;
        assert!(
            reloaded >= 6 && reloaded != 8,
            "the third store left {reloaded} in the count"
        );
        assert_ne!(get_f32(&g, OBJECT + 4), 9.0, "the centre word was written");
        assert_eq!(get_f32(&g, OBJECT + 20), 9.0, "the 7.1 words were not");
        assert_eq!(get_f32(&g, OBJECT + 24), 9.0);
    }

    // =========================================================== sub_82B45788

    /// The six coefficient blocks, each `{k, 0, 0, 1}` so that the first projection scales with the
    /// block and the second does not — which is what makes "the right block was read" visible.
    fn sectors(g: &mut Guest) {
        for (k, base) in [SECTOR_1, SECTOR_2, SECTOR_3, SECTOR_45, SECTOR_6, SECTOR_7]
            .iter()
            .enumerate()
        {
            put_f32(g, OBJECT + base, k as f32 + 1.0);
            put_f32(g, OBJECT + base + 4, 0.0);
            put_f32(g, OBJECT + base + 8, 0.0);
            put_f32(g, OBJECT + base + 12, 1.0);
        }
        put_f32(g, OBJECT + BOUND_0, 0.5);
        put_f32(g, OBJECT + BOUND_1, 1.5);
        put_f32(g, OBJECT + BOUND_2, 2.5);
        put_f32(g, OBJECT + CENTRE_SPREAD, 2.0);
    }

    /// `(first, second, norm)` for a block of `{k, 0, 0, 1}` — the documented projection and
    /// normalisation, with no share taken out. Used for the sectors that have no centre channel.
    ///
    /// The three parts are returned separately rather than pre-multiplied because the store is a
    /// **single** `fmadds`: `first*norm + old`, one rounding. A test that formed `first*norm` first
    /// would round twice and miss by an ulp — which is how this helper was first written, and it did.
    fn model_pair(k: f64, sine: f64, cosine: f64, length: f64) -> (f64, f64, f64) {
        let first = fp::fmadd_single(0.0, cosine, fp::mul_single(k, sine));
        let second = fp::fmadd_single(1.0, cosine, fp::mul_single(0.0, sine));
        let sum = fp::fmadd_single(second, second, fp::mul_single(first, first));
        let norm = fp::div_single(length, fp::sqrt_single(sum));
        (first, second, norm)
    }

    #[test]
    fn the_reduction_wraps_the_angle_into_one_turn() {
        // The two measured cells are 1/2pi and 2pi, so scaled -> floor -> fraction -> scaled back is
        // a wrap. Three angles a whole turn apart must reduce to the same value, which is what pins
        // both constants and the `floor` call between them at once.
        let two_pi = 6.2831854820251465f32;
        let mut asked = Vec::new();
        for turns in [-1.0f32, 0.0, 1.0, 3.0] {
            let mut g = guest();
            sectors(&mut g);
            layout(&mut g, 6, INDICES, SPEAKERS);
            record(&mut g, 0.0, 0.0, 1.0, 0.25 + turns * two_pi);
            let mut trig = Scripted {
                sine: 0.6,
                cosine: 0.8,
                ..Default::default()
            };
            add_angular(&mut g, &mut trig, OBJECT, RECORD, GAINS, 0.0).unwrap();
            asked.push(trig.asked[0].1);
        }
        for a in &asked {
            assert!(
                (a - 0.25).abs() < 1e-4,
                "reduced to {a}, not the 0.25 it started at"
            );
        }
        // And the wrap is the *same* computation each time, not four accidents: a single turn of
        // input moves the reduced angle by less than a single rounding of 2pi.
        let spread = asked.iter().cloned().fold(f64::MIN, f64::max)
            - asked.iter().cloned().fold(f64::MAX, f64::min);
        assert!(spread < 1e-4, "the four reductions disagree by {spread}");
    }

    #[test]
    fn each_of_the_seven_sectors_writes_its_own_pair_of_slots() {
        // The ladder, driven by the reduced angle and the count. With bound0/1/2 = 0.5/1.5/2.5 the
        // reflections are 2pi-2.5 = 3.783 and 2pi-1.5 = 4.783, which is what separates the last
        // three cases.
        let cases: [(f32, i32, Sector, &[u32]); 7] = [
            (0.0, 6, Sector::Front, &[8, 0, 4]),
            (1.0, 6, Sector::FrontSide, &[0, 12]),
            (2.0, 8, Sector::Side, &[12, 20]),
            (3.0, 8, Sector::SideRear, &[20, 24]),
            (4.0, 6, Sector::Rear, &[12, 16]),
            (4.0, 8, Sector::RearWide, &[24, 16]),
            (5.0, 6, Sector::Back, &[16, 8]),
        ];
        for (angle, count, want, slots) in cases {
            let mut g = guest();
            sectors(&mut g);
            layout(&mut g, count, INDICES, SPEAKERS);
            record(&mut g, 0.0, 0.0, 1.0, angle);
            for slot in 0..8u32 {
                put_f32(&mut g, GAINS + 4 * slot, 9.0);
            }
            let mut trig = Scripted {
                sine: 0.6,
                cosine: 0.8,
                ..Default::default()
            };
            let got = add_angular(&mut g, &mut trig, OBJECT, RECORD, GAINS, 0.5).unwrap();
            assert_eq!(got, want, "angle {angle} count {count}");
            for slot in 0..8u32 {
                let touched = slots.contains(&(4 * slot));
                let value = get_f32(&g, GAINS + 4 * slot);
                assert_eq!(
                    value != 9.0,
                    touched,
                    "{want:?}: slot +{} reads {value}, expected {}",
                    4 * slot,
                    if touched { "a contribution" } else { "9.0" }
                );
            }
        }
    }

    #[test]
    fn the_contribution_is_mix_added_and_the_block_it_comes_from_is_the_sectors_own() {
        // Sector 2 is the plain index-pair shape: coefficients at +92, destinations 4*idx172 and
        // 4*idx180, both read-modify-write. The expected values come from the documented projection
        // with k = 2, which is the second block -- so a port reading the wrong block fails here.
        let mut g = guest();
        sectors(&mut g);
        layout(&mut g, 6, INDICES, SPEAKERS);
        record(&mut g, 0.0, 0.0, 1.0, 1.0);
        put_f32(&mut g, GAINS, 0.25);
        put_f32(&mut g, GAINS + 12, -0.5);
        let mut trig = Scripted {
            sine: 0.6,
            cosine: 0.8,
            ..Default::default()
        };
        assert_eq!(
            add_angular(&mut g, &mut trig, OBJECT, RECORD, GAINS, 0.5).unwrap(),
            Sector::FrontSide
        );
        let (first, second, norm) = model_pair(2.0, 0.6, 0.8, 1.0);
        assert_eq!(
            get_f32(&g, GAINS),
            fp::fmadd_single(first, norm, 0.25) as f32
        );
        assert_eq!(
            get_f32(&g, GAINS + 12),
            fp::fmadd_single(second, norm, -0.5) as f32
        );
        // Mix-added, not assigned: the same call on a different starting value moves by the same
        // amount. (`fmadd(pair, norm, old)` has already applied `norm`, so the delta is exact here.)
        assert!(get_f32(&g, GAINS) > 0.25, "the old value was kept");
    }

    #[test]
    fn the_centre_share_is_the_smaller_projection_scaled_by_f1() {
        // Sector 1 with a count of 6: share = min(left, right) * f1, subtracted from BOTH and
        // multiplied by the +72 spread into the centre. With k = 1 the projections are left = 0.6
        // and right = 0.8, so the share comes off the left one.
        let mut g = guest();
        sectors(&mut g);
        layout(&mut g, 6, INDICES, SPEAKERS);
        record(&mut g, 0.0, 0.0, 1.0, 0.0);
        let mut trig = Scripted {
            sine: 0.6,
            cosine: 0.8,
            ..Default::default()
        };
        add_angular(&mut g, &mut trig, OBJECT, RECORD, GAINS, 0.5).unwrap();

        // Hand-derived: share = 0.6*0.5 = 0.3, left = 0.3, right = 0.5, centre = 2*0.3 = 0.6,
        // sum = 0.6^2 + 0.5^2 + 0.3^2 = 0.70, norm = 1/sqrt(0.70) = 1.1952286.
        let norm = 1.0 / 0.70f64.sqrt();
        assert!(
            (get_f32(&g, GAINS + 8) as f64 - 0.3 * norm).abs() < 1e-6,
            "idx176 takes left"
        );
        assert!(
            (get_f32(&g, GAINS) as f64 - 0.5 * norm).abs() < 1e-6,
            "idx172 takes right"
        );
        assert!(
            (get_f32(&g, GAINS + 4) as f64 - 0.6 * norm).abs() < 1e-6,
            "the centre"
        );

        // Below a count of 6 there is no share at all: left and right keep their full projections
        // and the centre word is not written.
        let mut h = guest();
        sectors(&mut h);
        layout(&mut h, 4, INDICES, SPEAKERS);
        record(&mut h, 0.0, 0.0, 1.0, 0.0);
        put_f32(&mut h, GAINS + 4, 9.0);
        let mut trig = Scripted {
            sine: 0.6,
            cosine: 0.8,
            ..Default::default()
        };
        add_angular(&mut h, &mut trig, OBJECT, RECORD, GAINS, 0.5).unwrap();
        assert_eq!(
            get_f32(&h, GAINS + 4),
            9.0,
            "no centre store below a count of 6"
        );
        assert!(
            (get_f32(&h, GAINS + 8) - 0.6).abs() < 1e-6,
            "left keeps its whole projection"
        );
        assert!((get_f32(&h, GAINS) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn the_count_is_reloaded_before_sector_ones_centre_store() {
        // Sector 1 reads the count twice: once for the share, once for the centre store. Overlap
        // the gain array so that the first store lands on the count word and leaves 0 there, and
        // the centre store is skipped even though the share was taken.
        //
        // Two things have to line up at once, and the first version of this test got only the
        // first: the word that lands on the count has to be zero, **and** the centre share has to
        // be non-zero, or the skipped store would have written the same value anyway and the
        // control would pass. (It did. Recorded, because a vacuous reload test is exactly the kind
        // of green this crate exists to avoid.)
        //
        // Both hold at f1 = 1.0 with left < right: the share is then the whole of `left`, so
        // `left - share` is exactly 0.0 while the centre keeps `spread * left`. The old gain at the
        // clobbered slot is the count word 6 read as a single — a denormal the guest's DAZ treats
        // as zero — so the first store writes a clean 0.0 over the count.
        let mut g = guest();
        sectors(&mut g);
        let gains = OBJECT + COUNT; // 4*idx176 == 0 puts the first store on the count
        layout(&mut g, 6, [2, 0, 3, 4], SPEAKERS);
        record(&mut g, 0.0, 0.0, 1.0, 0.0);
        put_f32(&mut g, gains + 4, 9.0);
        let mut trig = Scripted {
            sine: 0.6,
            cosine: 0.8,
            ..Default::default()
        };
        assert_eq!(
            add_angular(&mut g, &mut trig, OBJECT, RECORD, gains, 1.0).unwrap(),
            Sector::Front
        );
        assert_eq!(
            g.u32(OBJECT + COUNT).unwrap(),
            0,
            "the first store cleared the count"
        );
        assert_eq!(
            get_f32(&g, gains + 4),
            9.0,
            "so the centre store was skipped"
        );
        // And the centre really would have moved the word: the share was 0.6 and the spread 2.0.
        assert_ne!(
            get_f32(&g, gains + 8),
            0.0,
            "the second store ran, so the sector completed"
        );
    }

    #[test]
    fn the_unported_trig_also_stops_the_angular_pass() {
        let mut g = guest();
        sectors(&mut g);
        layout(&mut g, 6, INDICES, SPEAKERS);
        record(&mut g, 0.0, 0.0, 1.0, 0.0);
        put_f32(&mut g, GAINS, 9.0);
        let err = add_angular(
            &mut g,
            &mut crate::mathlib::Unported,
            OBJECT,
            RECORD,
            GAINS,
            0.5,
        )
        .unwrap_err();
        assert_eq!(err.address, 0x82F4_DED0, "the sine is asked first here");
        assert_eq!(get_f32(&g, GAINS), 9.0);
    }

    // =========================================================== sub_82B45B60

    #[test]
    fn scaling_the_gain_array_covers_five_words_and_two_more_at_a_count_of_eight() {
        for (count, sides) in [(6i32, false), (8, true)] {
            let mut g = guest();
            layout(&mut g, count, INDICES, SPEAKERS);
            for slot in 0..8u32 {
                put_f32(&mut g, GAINS + 4 * slot, 2.0);
            }
            // f3 >= 1.0 skips the normalisation, so the factor is exactly f1*f2 = 0.5.
            scale_gains(&mut g, OBJECT, GAINS, 0.25, 2.0, 1.0).unwrap();
            for slot in 0..5u32 {
                assert_eq!(
                    get_f32(&g, GAINS + 4 * slot),
                    1.0,
                    "count {count} slot {slot}"
                );
            }
            let want = if sides { 1.0 } else { 2.0 };
            assert_eq!(get_f32(&g, GAINS + 20), want, "count {count}");
            assert_eq!(get_f32(&g, GAINS + 24), want, "count {count}");
            assert_eq!(get_f32(&g, GAINS + 28), 2.0, "nothing past +27");
        }
    }

    #[test]
    fn a_spread_below_one_divides_the_factor_by_the_root_sum_of_squares() {
        // The selected gains are 1, 2, 3, 4 at 4*index for indices 0, 2, 3, 4; the centre (+4) is
        // 2 and joins from a count of 6 up; at 8 the two sides join as well. Hand-derived:
        //   count 4: sum = 4 + 1 + 9 + 16 = 30
        //   count 6: + 2^2 = 34
        //   count 8: + (5^2 + 6^2) = 95
        // The order is 176, 172, 180, 184 and it is not reassociable, but these are exact integers.
        for (count, sum) in [(4i32, 30.0f64), (6, 34.0), (8, 95.0)] {
            let mut g = guest();
            layout(&mut g, count, INDICES, SPEAKERS);
            for (slot, v) in [
                (0u32, 1.0f32),
                (4, 2.0),
                (8, 2.0),
                (12, 3.0),
                (16, 4.0),
                (20, 5.0),
                (24, 6.0),
            ] {
                put_f32(&mut g, GAINS + slot, v);
            }
            scale_gains(&mut g, OBJECT, GAINS, 1.0, 1.0, 0.5).unwrap();
            let factor = fp::div_single(1.0, fp::sqrt_single(sum));
            assert_eq!(
                get_f32(&g, GAINS),
                fp::mul_single(1.0, factor) as f32,
                "count {count}, sum {sum}"
            );
            assert_eq!(get_f32(&g, GAINS + 12), fp::mul_single(3.0, factor) as f32);
        }
    }

    #[test]
    fn a_nan_spread_skips_the_normalisation_entirely() {
        // `fcmpu ; bge` is "not lt", so an unordered compare leaves the factor at f1*f2 and never
        // touches the indices or the gains they select. A translation writing `!(spread >= one)`
        // would normalise instead, and on a malformed object would read through a wild index.
        let mut g = guest();
        layout(&mut g, 6, [0, 2, 3, 4], SPEAKERS);
        for slot in 0..8u32 {
            put_f32(&mut g, GAINS + 4 * slot, 2.0);
        }
        scale_gains(&mut g, OBJECT, GAINS, 0.25, 2.0, f64::NAN).unwrap();
        assert_eq!(
            get_f32(&g, GAINS),
            1.0,
            "scaled by 0.5, not by 0.5/sqrt(sum)"
        );

        // And the proof that the indices really are untouched on that path: point one of them at an
        // unmapped address. The normalising path would fail; this one cannot.
        let mut h = guest();
        layout(&mut h, 6, [0, 2, 3, 0x2000_0000], SPEAKERS);
        for slot in 0..8u32 {
            put_f32(&mut h, GAINS + 4 * slot, 2.0);
        }
        assert!(scale_gains(&mut h, OBJECT, GAINS, 0.25, 2.0, f64::NAN).is_ok());
        assert!(
            scale_gains(&mut h, OBJECT, GAINS, 0.25, 2.0, 0.5).is_err(),
            "the same object normalising reads the wild index"
        );
    }

    #[test]
    fn the_scaled_count_is_reloaded_after_the_five_stores() {
        // Same shape as the panning reload: overlap the array so a store clears the count. The
        // entry count is 8 and the reloaded one is 0, so the two 7.1 words stay untouched.
        let mut g = guest();
        let gains = OBJECT + 44;
        assert_eq!(gains + 12, OBJECT + COUNT);
        layout(&mut g, 8, INDICES, SPEAKERS);
        for slot in 0..8u32 {
            put_f32(&mut g, gains + 4 * slot, 2.0);
        }
        // The count goes in *after* the gains, because gains+12 is the count word: filling it with
        // 2.0 first would leave the entry count at 0x40000000 and the test would not distinguish a
        // hoisted read at all. It reads 8 on entry, and the store there leaves the word holding
        // `8-as-a-single * 0.5`, a denormal that flush-to-zero turns into a clean zero.
        g.set_u32(OBJECT + COUNT, 8).unwrap();
        scale_gains(&mut g, OBJECT, gains, 0.25, 2.0, 1.0).unwrap();
        assert_eq!(
            get_f32(&g, gains + 12).to_bits(),
            0,
            "the count word took a flushed zero"
        );
        assert_eq!(
            get_f32(&g, gains + 20),
            2.0,
            "the reloaded count is 0, so no 7.1 words"
        );
        assert_eq!(get_f32(&g, gains + 24), 2.0);
    }

    // =========================================================== all five

    #[test]
    fn every_body_restores_the_entry_flush_mode() {
        let mut g = guest();
        sectors(&mut g);
        layout(&mut g, 8, INDICES, SPEAKERS);
        record(&mut g, 0.25, 0.25, 0.125, 0.5);
        let before = crate::vmx::get_mxcsr();
        let mut trig = Scripted {
            sine: 0.6,
            cosine: 0.8,
            ..Default::default()
        };
        clamp_to_unit_disc(&mut g, RECORD, 0.5, 0.25).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before, "clamp_to_unit_disc");
        place_panner(&mut g, &mut trig, RECORD, 90.0, 1.0).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before, "place_panner");
        record(&mut g, 0.25, 0.25, 0.125, 0.5);
        pan_distance(&mut g, OBJECT, RECORD, GAINS, 1.0).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before, "pan_distance");
        add_angular(&mut g, &mut trig, OBJECT, RECORD, GAINS, 0.5).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before, "add_angular");
        scale_gains(&mut g, OBJECT, GAINS, 0.5, 0.5, 0.5).unwrap();
        assert_eq!(crate::vmx::get_mxcsr(), before, "scale_gains");
    }

    // ------------------------------------------------------------------ sub_82B45C50

    const ENTRIES: u32 = 0x4000_2000;
    const MATRIX: u32 = 0x4000_3000;
    const STACK_TOP: u32 = 0x4001_0000;
    const POISON: u32 = 0xDEAD_BEEF;

    /// Set a cell whether or not a segment already covers it.
    fn cell(g: &mut Guest, at: u32, v: f32) {
        if g.set_u32(at, v.to_bits()).is_err() {
            g.put(at, v.to_bits().to_be_bytes().to_vec());
        }
    }

    /// The panner area poisoned, a stack, the atan2 pool, and the layout constants: both scales
    /// 1, the mirror marker 7 and its bias 0.5.
    fn layout_guest() -> Guest {
        let mut g = guest();
        g.put(ENTRIES, POISON.to_be_bytes().repeat(32));
        g.put(MATRIX, POISON.to_be_bytes().repeat(64));
        g.put(STACK_TOP - 0x1000, vec![0u8; 0x1000]);
        crate::mathlib::tests::with_atan_pool(&mut g);
        cell(&mut g, LAYOUT_ANGLE_SCALE, 1.0);
        cell(&mut g, LAYOUT_SPREAD_SCALE, 1.0);
        cell(&mut g, LAYOUT_MIRROR_MARKER, 7.0);
        cell(&mut g, LAYOUT_MIRROR_BIAS, 0.5);
        g
    }

    fn trig() -> Scripted {
        Scripted {
            sine: 0.25,
            cosine: 0.5,
            asked: vec![],
        }
    }

    fn args(spread5: f64) -> PannerLayout {
        PannerLayout {
            angle: 0.1,
            distance: 0.5,
            radius: 0.25,
            turn: 0.2,
            spreads: [spread5, 0.3, 0.4],
        }
    }

    fn written(g: &Guest) -> Vec<bool> {
        (0..8)
            .map(|k| g.u32(ENTRIES + 16 * k + PANNER_ENTRY_ANGLE).unwrap() != POISON)
            .collect()
    }

    #[test]
    fn counts_three_five_seven_and_below_two_write_nothing() {
        for count in [3, 5, 7, 0, -1, 9] {
            let mut g = layout_guest();
            lay_out_panners(&mut g, &mut trig(), ENTRIES, count, args(0.1), STACK_TOP).unwrap();
            assert_eq!(written(&g), [false; 8], "count {count}");
        }
    }

    #[test]
    fn each_even_count_writes_its_prefix_and_never_entry_seven() {
        for (count, n) in [(2, 2), (4, 4), (6, 5), (8, 7)] {
            let mut g = layout_guest();
            lay_out_panners(&mut g, &mut trig(), ENTRIES, count, args(0.1), STACK_TOP).unwrap();
            let want: Vec<bool> = (0..8).map(|k| k < n).collect();
            assert_eq!(written(&g), want, "count {count}");
        }
    }

    #[test]
    fn one_entry_hands_the_job_to_place_panner() {
        let mut g = layout_guest();
        let mut h = g.clone();
        lay_out_panners(&mut g, &mut trig(), ENTRIES, 1, args(0.1), STACK_TOP).unwrap();
        place_panner(&mut h, &mut trig(), ENTRIES, 0.1, 0.5).unwrap();
        for k in 0..8 {
            assert_eq!(
                g.u32(ENTRIES + 4 * k).unwrap(),
                h.u32(ENTRIES + 4 * k).unwrap(),
                "word {k}"
            );
        }
    }

    #[test]
    fn the_mirrored_pair_reuses_the_first_offsets_negated() {
        // Centre (0.5 * 0.5, 0.25 * 0.5) = (0.25, 0.125); offsets (0.125, 0.0625). All exact.
        let mut g = layout_guest();
        let mut t = trig();
        lay_out_panners(&mut g, &mut t, ENTRIES, 2, args(7.0), STACK_TOP).unwrap();
        assert_eq!(
            (get_f32(&g, ENTRIES), get_f32(&g, ENTRIES + 4)),
            (0.375, 0.1875)
        );
        assert_eq!(
            (get_f32(&g, ENTRIES + 16), get_f32(&g, ENTRIES + 20)),
            (0.125, 0.0625)
        );
        assert_eq!(
            t.asked.len(),
            4,
            "the centre and one placement: no second pair of calls"
        );
        let mut spread = trig();
        lay_out_panners(
            &mut layout_guest(),
            &mut spread,
            ENTRIES,
            2,
            args(0.1),
            STACK_TOP,
        )
        .unwrap();
        assert_eq!(
            spread.asked.len(),
            6,
            "a spread pair asks for both placements"
        );
    }

    #[test]
    fn the_stored_angle_is_that_of_the_clamped_position() {
        let mut g = layout_guest();
        lay_out_panners(&mut g, &mut trig(), ENTRIES, 2, args(7.0), STACK_TOP).unwrap();
        let (x, y) = (
            f64::from(get_f32(&g, ENTRIES)),
            f64::from(get_f32(&g, ENTRIES + 4)),
        );
        let mut h = layout_guest();
        let angle = crate::mathlib::atan2(&mut h, y, x, STACK_TOP - 0x100).unwrap();
        assert_eq!(get_f32(&g, ENTRIES + PANNER_ENTRY_ANGLE), angle as f32);
    }

    // ------------------------------------------------------------------ sub_82B460A0

    fn gains() -> MatrixGains {
        MatrixGains {
            weight: 2.0,
            focus: 0.5,
            fill: 0.75,
            gain: 0.5,
        }
    }

    fn matrix(g: &Guest) -> Vec<u32> {
        (0..64).map(|k| g.u32(MATRIX + 4 * k).unwrap()).collect()
    }

    #[test]
    fn a_stereo_destination_pans_each_source_from_its_y_alone() {
        let mut g = layout_guest();
        g.set_u32(OBJECT + MATRIX_DEST_COUNT, 2).unwrap();
        g.set_u32(ENTRIES + 4, 0.0f32.to_bits()).unwrap(); // source 0 straight ahead
        g.set_u32(ENTRIES + 16 + 4, 1.0f32.to_bits()).unwrap(); // source 1 hard over
        fill_mix_matrix(&mut g, &mut trig(), OBJECT, ENTRIES, 2, MATRIX, gains()).unwrap();
        // y = 0: right = left = 0.5, normalised by sqrt(0.5); weight * gain is 1.
        let total = 1.0f32 / (0.5f64.sqrt() as f32);
        assert_eq!(get_f32(&g, MATRIX), total * 0.5);
        assert_eq!(get_f32(&g, MATRIX + 4), 0.5 * total);
        // y = 1: right = 1, left = 0, length 1.
        assert_eq!(get_f32(&g, MATRIX + 32), 1.0);
        assert_eq!(get_f32(&g, MATRIX + 36), 0.0);
        assert_eq!(g.u32(MATRIX + 8).unwrap(), POISON, "two columns only");
        assert_eq!(g.u32(MATRIX + 64).unwrap(), POISON, "two rows only");
    }

    /// The panning rows made directly, for comparison.
    fn panned(h: &mut Guest, rows: u32) {
        for r in 0..rows {
            let (entry, row) = (ENTRIES + 16 * r, MATRIX + 32 * r);
            pan_distance(h, OBJECT, entry, row, 0.5).unwrap();
            add_angular(h, &mut trig(), OBJECT, entry, row, 0.5).unwrap();
            let lensq = fp::load_single(h, entry + PANNER_ENTRY_LENGTH_SQ).unwrap();
            scale_gains(h, OBJECT, row, 2.0, 0.5, lensq).unwrap();
        }
    }

    fn panning_guest(dest: i32) -> Guest {
        let mut g = layout_guest();
        let speakers = [
            (0.0, 1.0),
            (0.7, 0.7),
            (1.0, 0.0),
            (0.7, -0.7),
            (0.0, -1.0),
            (-0.7, -0.7),
            (-1.0, 0.0),
        ];
        layout(&mut g, dest, [0, 2, 3, 4], speakers);
        for k in 0..8u32 {
            let angle = 0.3 * k as f32;
            for (off, v) in [
                (0, angle.cos() * 0.5),
                (4, angle.sin() * 0.5),
                (8, 0.25),
                (12, angle),
            ] {
                g.set_u32(ENTRIES + 16 * k + off, v.to_bits()).unwrap();
            }
        }
        g
    }

    #[test]
    fn six_sources_into_six_channels_pan_five_rows_then_clear_a_row_and_a_column() {
        let mut g = panning_guest(6);
        let mut h = g.clone();
        fill_mix_matrix(&mut g, &mut trig(), OBJECT, ENTRIES, 6, MATRIX, gains()).unwrap();
        panned(&mut h, 5);
        for k in 0..8 {
            h.set_u32(MATRIX + 32 * k + 4 * 5, 0).unwrap(); // column 5 of every row
            h.set_u32(MATRIX + 32 * 5 + 4 * k, 0).unwrap(); // the whole of row 5
        }
        h.set_u32(MATRIX + 32 * 5 + 4 * 5, 0.75f32.to_bits())
            .unwrap(); // their crossing
        assert_eq!(matrix(&g), matrix(&h));
    }

    #[test]
    fn two_sources_into_eight_channels_fill_column_seven() {
        let mut g = panning_guest(8);
        let mut h = g.clone();
        fill_mix_matrix(&mut g, &mut trig(), OBJECT, ENTRIES, 2, MATRIX, gains()).unwrap();
        panned(&mut h, 2);
        for k in 0..8 {
            h.set_u32(MATRIX + 32 * k + 4 * 7, 0.75f32.to_bits())
                .unwrap();
        }
        assert_eq!(matrix(&g), matrix(&h));
    }
}
