use super::*;
fn constant(value: f32) -> PointGraph<8> {
    PointGraph {
        x: [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
        y: [value; 8],
    }
}
fn settings() -> GroundOrientationSettings {
    GroundOrientationSettings {
        ground_normal_smoothing: [1.0, 0.0, 0.0, 1.0],
        up_vector_smoothing_slow: [1.0, 0.0, 0.0, 1.0],
        up_vector_smoothing_fast: [1.0, 0.0, 0.0, 1.0],
        dynamic_up_vs_ground_y: constant(1.0),
        ground_vector_blend: constant(0.0),
        deck_angle_usage_vs_speed: constant(0.0),
        up_vector_smoothing_vs_speed: constant(0.0),
        up_vector_max_delta_vs_speed: constant(1.0),
        ground_blend_max_delta: 0.1,
        up_vector_max_acceleration: 2.0,
        anti_wobble_damping: 0.5,
        extra_side_damping: 0.6,
        minimum_wheels_for_ground_blend: 0,
    }
}
fn input(normal: Vector3) -> GroundOrientationInput {
    GroundOrientationInput {
        com_to_deck: UP,
        ground_normal: normal,
        dynamic_up: normal,
        speed: 0.0,
        wheel_contact_count: 4,
        animation_balance: 0.0,
        deck_angle_curve_input: 0.0,
        board_up: UP,
        board_forward: Vector3::new(0.0, 0.0, 1.0),
        effective_board_forward: Vector3::new(0.0, 0.0, 1.0),
        previous_reckoning_right: Vector3::new(1.0, 0.0, 0.0),
        prevent_up_behind_board: false,
    }
}
#[test]
fn stationary_filter_startup_has_no_false_tilt_or_acceleration() {
    let s = settings();
    let mut state = GroundOrientation::new(&s);
    for _ in 0..120 {
        state.update(&s, input(UP));
    }
    assert_eq!(state.up, UP);
    assert_eq!(state.ground_normal, UP);
    assert_eq!(state.up_velocity, Vector3::ZERO);
}
#[test]
fn changed_contact_normal_does_not_instantly_replace_the_skater_up() {
    let s = settings();
    let mut state = GroundOrientation::new(&s);
    let wall = Vector3::new(1.0, 0.0, 0.0);
    state.update(&s, input(wall));
    assert_eq!(state.ground_normal, wall);
    assert_ne!(state.up, state.ground_normal);
    assert!(state.up.y > 0.99 && state.up.x > 0.0 && state.up.x < 0.04);
    assert_eq!(
        &state.slow_filter.words()[4..8],
        &lanes(state.up).map(f32::to_bits)
    );
}

/// The landing NaN, at the point where it is made.
///
/// `up_velocity` decays geometrically while the skater settles after a touchdown, so a lane
/// reaching ~1e-20 is ordinary. Squaring that underflows to a denormal, which used to walk
/// straight past `length`'s `squared == 0.0` guard and come back NaN -- and `clamp_length`'s
/// `magnitude < ...` early return is false against NaN, so it scaled by a NaN reciprocal.
#[test]
fn an_up_velocity_decayed_to_a_denormal_square_does_not_turn_the_up_vector_nan() {
    let settled = Vector3::new(-3.2526066e-20, 0.0, 0.0);
    assert_eq!(length(settled), 0.0, "a vector this short has no representable length");
    let clamped = clamp_length(settled, 2.0 * f32::from_bits(0x3C88_8889));
    assert_eq!(clamped, settled, "a vector far inside the maximum is returned unchanged");

    // And end to end: the same decay driven through `update` from a settled stance.
    let s = settings();
    let mut state = GroundOrientation::new(&s);
    state.up_velocity = settled;
    for _ in 0..8 {
        state.update(&s, input(UP));
        for lane in [state.up.x, state.up.y, state.up.z] {
            assert!(lane.is_finite(), "up went non-finite: {:?}", state.up);
        }
    }
}

/// No finite stance the engine can hand in may produce a non-finite up vector, because one
/// does not recover: `calculate_transform` writes it into `system[1]`, and the skeleton solve
/// aborts on it a frame or more later, long after the origin is gone.
#[test]
fn no_finite_input_sequence_turns_the_up_vector_non_finite() {
    let s = settings();
    let mut seed: u32 = 0x1234_5678;
    let mut rng = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    // A unit row, a zero row or an axis -- what the callers in `riding_outputs` actually pass.
    let direction = |a: u32, b: u32| -> Vector3 {
        match a % 6 {
            0 => Vector3::ZERO,
            1 => UP,
            2 => Vector3::new(0.0, -1.0, 0.0),
            3 => Vector3::new(1.0, 0.0, 0.0),
            4 => Vector3::new(0.0, 0.0, 1.0),
            _ => {
                let yaw = (b % 6283) as f32 / 1000.0;
                let pitch = ((b >> 13) % 6283) as f32 / 1000.0;
                Vector3::new(yaw.cos() * pitch.sin(), pitch.cos(), yaw.sin() * pitch.sin())
            }
        }
    };
    let speed = |r: u32| ((r >> 2) % 40_000) as f32 / 1000.0;
    for case in 0..50_000u32 {
        let mut state = GroundOrientation::new(&s);
        for step in 0..6 {
            macro_rules! direction {
                () => {
                    direction(rng(), rng())
                };
            }
            let inputs = GroundOrientationInput {
                com_to_deck: direction!(),
                ground_normal: direction!(),
                dynamic_up: direction!(),
                speed: speed(rng()),
                wheel_contact_count: (rng() % 5) as i32,
                animation_balance: if rng() % 2 == 0 { 0.0 } else { 1.0 },
                deck_angle_curve_input: speed(rng()),
                board_up: direction!(),
                board_forward: direction!(),
                effective_board_forward: direction!(),
                previous_reckoning_right: direction!(),
                prevent_up_behind_board: rng() % 2 == 0,
            };
            let before = state.clone();
            state.update(&s, inputs);
            let non_finite =
                |v: Vector3| !(v.x.is_finite() && v.y.is_finite() && v.z.is_finite());
            assert!(
                !non_finite(state.up) && !non_finite(state.up_velocity),
                "case {case} step {step}: up={:?} up_velocity={:?}\n  entered with up={:?} up_velocity={:?}\n  inputs={inputs:?}",
                state.up,
                state.up_velocity,
                before.up,
                before.up_velocity,
            );
        }
    }
}
