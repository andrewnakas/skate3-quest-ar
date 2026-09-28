use super::*;

#[test]
fn refresh_applies_without_decay_then_height_and_xz_decay_independently() {
    let mut offset = SkateboardOffset::default();
    let mut transform = IDENTITY;
    transform[3] = [15.0, 30.0, 45.0, 0.0];
    offset.refresh_transform(transform);
    let mut board = IDENTITY;
    let mut targets = [IDENTITY; 4];
    offset.update(&mut board, &mut targets);
    assert_eq!(board[3], transform[3]);
    assert_eq!(targets[3][3], transform[3]);
    assert_eq!(offset.orientation_frames, 15.0);
    offset.refresh_height(10.0, 7.0);
    board = IDENTITY;
    targets = [IDENTITY; 4];
    offset.update(&mut board, &mut targets);
    let ratio = 14.0f32 / 15.0;
    assert_eq!(offset.orientation_frames, 14.0);
    assert_eq!(offset.height_frames, 7.0);
    assert_eq!(board[3][0], 15.0 * (ratio * ratio));
    assert_eq!(board[3][1], 10.0);
    assert_eq!(board, targets[0]);
    offset.orientation_frames = 0.0;
    offset.height_frames = 1.0;
    board = IDENTITY;
    offset.update(&mut board, &mut targets);
    assert_eq!(offset.height_frames, 0.0);
    assert_eq!(offset.transform[3][1], 0.0);
    assert!(!offset.height_refreshed);
}

/// A non-finite row must not survive the decay's Gram-Schmidt.
///
/// A zero-length row's `frsqrte` estimate is infinite, so the unguarded normalise produced NaN,
/// and a NaN already in the row stayed NaN. Either way it reached the board pose, then
/// `animation_to_world`, then the skeleton's lifted-COM target velocity, and the solver's
/// finiteness check aborted the game. This decay is armed for 15 updates after a landing, which
/// is where the abort was seen.
#[test]
fn a_non_finite_offset_row_decays_without_going_non_finite() {
    let mut offset = SkateboardOffset::default();
    let mut transform = IDENTITY;
    transform[0] = [f32::NAN; 4];
    offset.refresh_transform(transform);
    for update in 0..20 {
        let mut board = IDENTITY;
        let mut targets = [IDENTITY; 4];
        offset.update(&mut board, &mut targets);
        // The refresh update applies the frame as written and decays nothing, so the injected row
        // is still NaN there; from the first decaying update on, the row must come back finite.
        if update == 0 {
            continue;
        }
        for (axis, row) in offset.transform.iter().enumerate() {
            assert!(
                row.iter().all(|v| v.is_finite()),
                "update {update}: offset row {axis} is {row:?}"
            );
        }
        assert!(
            board.iter().flatten().all(|v| v.is_finite()),
            "update {update}: board pose is {board:?}"
        );
    }
}
