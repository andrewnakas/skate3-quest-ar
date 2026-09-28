//! Observed deck motion from TU3 Skateboard::FillPhysOut82C02A80.
//! These calculations consume the live solver body and the actual Reckoning
//! normal. They do not infer contact state from height or vertical velocity.
use super::{board::BodyId, board_runtime::BoardRuntime, native_arithmetic};
use crate::math::{Basis3, Vector3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoardMotionOutput {
    pub angular_velocity: Vector3,
    pub linear_velocity: Vector3,
    pub ground_velocity: Vector3,
    /// SkateboardMotion+160: magnitude of the unprojected deck velocity.
    pub speed: f32,
    /// SkateboardMotion+164: magnitude after rejecting the Reckoning normal.
    pub ground_speed: f32,
    /// SkateboardMotion+168: raw deck velocity projected onto effective Z.
    pub forward_speed: f32,
    pub effective_basis: Basis3,
}

impl BoardMotionOutput {
    pub fn from_board(
        board: &BoardRuntime,
        reckoning_ground_normal: Vector3,
        processed_flags_2468: u32,
    ) -> Self {
        let deck = board.bodies()[BodyId::Deck.index()].rates;
        let mut effective_basis = board.part_transforms()[BodyId::Deck.index()].basis;
        // Complete GetEffectiveTransform82C01BF8 axis adjustment; translation
        // and Y remain the physical part's values.
        if processed_flags_2468 & 0x0010_0000 != 0 {
            for axis in [0, 2] {
                effective_basis.columns[axis] = effective_basis.columns[axis].map(|v| -v);
            }
        }
        let normal_speed = dot(deck.linear_velocity, reckoning_ground_normal);
        let ground_velocity = subtract(
            deck.linear_velocity,
            scale(reckoning_ground_normal, normal_speed),
        );
        let z = effective_basis.columns[2];
        Self {
            angular_velocity: deck.angular_velocity,
            linear_velocity: deck.linear_velocity,
            ground_velocity,
            speed: length(deck.linear_velocity),
            ground_speed: length(ground_velocity),
            forward_speed: dot(deck.linear_velocity, Vector3::new(z[0], z[1], z[2])),
            effective_basis,
        }
    }
}

pub fn dot(a: Vector3, b: Vector3) -> f32 {
    native_arithmetic::dot3([a.x, a.y, a.z, 0.0], [b.x, b.y, b.z, 0.0])
}
pub(crate) fn scale(v: Vector3, s: f32) -> Vector3 {
    Vector3::new(v.x * s, v.y * s, v.z * s)
}
pub(crate) fn add(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}
pub(crate) fn subtract(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
pub(crate) fn inverse_length_squared(squared: f32, refinements: usize) -> f32 {
    let mut inverse = native_arithmetic::reciprocal_square_root_estimate(squared);
    for _ in 0..refinements {
        let correction = (-squared).mul_add(inverse * inverse, 1.0);
        inverse = (inverse * 0.5).mul_add(correction, inverse);
    }
    inverse
}
/// Normalise one Gram-Schmidt row, keeping `fallback` when it is degenerate.
///
/// [`inverse_length_squared`] refines an `frsqrte` estimate, and that estimate is infinite for a
/// zero-length row: the first Newton step is `fma(-0, inf, 1)` = NaN, so the row -- and then the
/// whole affine, because `compose_affine`'s translation mixes every rotation row -- becomes NaN.
/// Retail never reaches exactly zero here; this port does, because the board offset's rows lerp
/// *linearly* toward identity, so a ~180-degree offset crosses an exactly-zero row mid-decay. That
/// is the landing crash: `animation_to_world` went NaN, `lifted_com_frame[3]` with it, and the
/// skeleton's lifted-COM target body took a NaN velocity into the solver.
///
/// Only an exactly-degenerate or non-finite row is touched, so a row retail could produce keeps
/// the original arithmetic bit for bit.
pub(crate) fn normalize_row_or(vector: [f32; 4], fallback: [f32; 4]) -> [f32; 4] {
    let squared = crate::physics::native_arithmetic::dot3(vector, vector);
    if squared <= 0.0 || !squared.is_finite() {
        return fallback;
    }
    let reciprocal = inverse_length_squared(squared, 2);
    if !reciprocal.is_finite() {
        return fallback;
    }
    vector.map(|v| v * reciprocal)
}

/// Vector length, with the source's degenerate guard given the reach it has on the hardware.
///
/// Retail tests `squared == 0.0` and that is enough on Xenon: VMX runs in non-Java mode, where a
/// denormal result flushes to zero, so every vector shorter than ~1.1e-19 reaches the test as an
/// exact zero. Here `dot` keeps the denormal, so the test misses and [`inverse_length_squared`]
/// runs on it -- and its Newton step squares an estimate of ~1e19, which overflows f32 to
/// infinity and settles on `inf - inf` = NaN. So `length` returned NaN for any squared length
/// below ~2.9e-39.
///
/// That is the landing crash. `up_velocity` decays geometrically through `anti_wobble_damping`
/// and `extra_side_damping` as the skater settles after a touchdown; once a lane reaches ~1e-20,
/// `clamp_length`'s `magnitude < ...` test is false against NaN, so it skips its own early return
/// and scales by a NaN reciprocal. That NaN becomes `up_velocity`, then `up`, and
/// `calculate_transform` writes it straight into `system[1]` -- the NaN up row the owner's crash
/// log names, with rows 0 and 2 still finite because their Gram-Schmidt guards fell back.
///
/// Flushing the denormal restores the original guard's reach. Every squared length the hardware
/// would have kept normal is untouched, so a non-degenerate vector keeps its exact former value.
pub fn length(v: Vector3) -> f32 {
    let squared = dot(v, v);
    if squared < f32::MIN_POSITIVE {
        return 0.0;
    }
    squared * inverse_length_squared(squared, 2)
}

#[cfg(test)]
mod normalize_row_tests {
    use super::*;

    /// The guard's contract: a row retail could produce normalises exactly as before, and a
    /// degenerate or non-finite one keeps the fallback instead of becoming NaN.
    #[test]
    fn a_degenerate_or_non_finite_row_keeps_the_fallback() {
        let fallback = [1.0, 0.0, 0.0, 0.0];
        assert_eq!(normalize_row_or([0.0; 4], fallback), fallback);
        assert_eq!(
            normalize_row_or([f32::NAN, 0.0, 0.0, 0.0], fallback),
            fallback
        );
        assert_eq!(
            normalize_row_or([f32::INFINITY, 0.0, 0.0, 0.0], fallback),
            fallback
        );
        // An ordinary row is untouched: same arithmetic as before the guard.
        let row = [0.0, 3.0, 4.0, 0.0];
        let reciprocal = inverse_length_squared(native_arithmetic::dot3(row, row), 2);
        assert_eq!(normalize_row_or(row, fallback), row.map(|v| v * reciprocal));
    }
}
