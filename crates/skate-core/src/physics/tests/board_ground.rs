use super::*;

/// Regression for the paired postphysics branch contract, not fidelity proof.
/// S2 82B372E0 names mAngularDrag; S3 82C08634 stores inertia+36.
#[test]
fn wheel_angular_drag_tracks_physical_contact_and_wipeout() {
    let mut ground = BoardGroundState::default();
    let mut lines = WheelLineState::default();
    // A successful line query is not physical wheel contact for drag selection.
    lines.publish(
        [Some(WheelLineHit {
            fraction: 0.1,
            normal: UP,
            surface_tag: 0,
        }); 4],
    );
    let frequency = f32::from_bits(0x426F_FFFF);
    let free = f32::from_bits(0x3BC4_9BA6) * frequency;
    let wiping_out = f32::from_bits(0x3D23_D70A) * frequency;
    let contacts = [
        report(BodyId::RightFrontWheel, UP),
        report(BodyId::LeftBackWheel, UP),
    ];
    ground.update(&contacts, &lines, UP, 80.0, false);
    assert_eq!(ground.wheel_angular_drag, [0.0, free, free, 0.0]);
    ground.update(&contacts, &lines, UP, 80.0, true);
    assert_eq!(
        ground.wheel_angular_drag,
        [wiping_out, free, free, wiping_out]
    );
    ground.update(&[], &lines, UP, 80.0, true);
    assert_eq!(ground.wheel_angular_drag, [free; 4]);
    ground.update(&contacts, &lines, UP, 80.0, false);
    assert_eq!(ground.wheel_angular_drag, [0.0, free, free, 0.0]);
}

fn report(part: BodyId, normal: Vector3) -> BoardContactReport {
    BoardContactReport {
        part,
        other: CollisionBody::StaticWorld,
        is_body_a: true,
        normal,
        position: Vector3::ZERO,
        relative_linear_velocity: Vector3::ZERO,
        other_surface: 0,
        normal_force_on_a: UP,
        friction_force_on_a: Vector3::ZERO,
        tangents: [Vector3::ZERO; 2],
    }
}

#[test]
fn lost_wheel_support_keeps_wheel_normal_but_rebuilds_overall_normal() {
    let mut ground = BoardGroundState::default();
    let mut lines = WheelLineState::default();
    lines.publish(
        [Some(WheelLineHit {
            fraction: 0.1,
            normal: UP,
            surface_tag: 0,
        }); 4],
    );
    ground.update(
        &[report(BodyId::RightFrontWheel, UP)],
        &lines,
        UP,
        80.0,
        false,
    );
    assert_eq!(ground.wheel_contact_count, 1);
    let retained = ground.wheel_normal;
    ground.update(
        &[report(BodyId::Deck, Vector3::new(1.0, 0.0, 0.0))],
        &lines,
        UP,
        80.0,
        false,
    );
    assert_eq!(ground.wheel_contact_count, 0);
    assert_eq!(ground.part_contact_count, 1);
    assert_eq!(ground.wheel_normal, retained);
    assert!(ground.overall_normal.x > 0.99999);
    assert!(ground.overall_normal.y.abs() < 0.00001);
}

#[test]
fn no_hit_preserves_line_geometry_and_contact_status_does_not_come_from_lines() {
    let mut lines = WheelLineState::default();
    let normal = Vector3::new(0.6, 0.8, 0.0);
    lines.publish(
        [Some(WheelLineHit {
            fraction: 0.2,
            normal,
            surface_tag: 42 | (3 << 7),
        }); 4],
    );
    assert_eq!(lines.audio_surfaces, [42; 4]);
    assert_eq!(lines.physics_surfaces, [3; 4]);
    let distances = lines.distances;
    lines.publish([None; 4]);
    assert_eq!(lines.normals, [normal; 4]);
    assert_eq!(lines.distances, distances);
    assert_eq!(lines.audio_surfaces, [0; 4]);
    assert_eq!(lines.physics_surfaces, [0; 4]);
    assert_eq!(lines.minimum_distance, WHEEL_LINE_LENGTH);
    let mut ground = BoardGroundState::default();
    ground.update(&[], &lines, UP, 80.0, false);
    assert_eq!(ground.part_contact_count, 0);
    assert_eq!(ground.wheel_contact_count, 0);
    assert_eq!(ground.wheel_normal, UP);
    assert_eq!(ground.overall_normal, UP);
}

#[test]
fn per_part_selection_uses_highest_y_and_keeps_first_on_ties() {
    let first = report(BodyId::Deck, Vector3::new(0.6, 0.8, 0.0));
    let tied = report(BodyId::Deck, Vector3::new(-0.6, 0.8, 0.0));
    let lower = report(BodyId::Deck, Vector3::new(0.0, 0.4, 0.9));
    let mut ground = BoardGroundState::default();
    ground.update(
        &[first, tied, lower],
        &WheelLineState::default(),
        UP,
        80.0,
        false,
    );
    assert_eq!(ground.parts[6].normal, first.normal);
    assert_eq!(ground.part_contact_count, 1);
}

#[test]
fn observed_acceleration_retains_each_part_velocity_and_resets_with_collision_info() {
    let mut state = super::BoardGroundState::default();
    let first = core::array::from_fn(|i| crate::math::Vector3::new(i as f32, 2.0, -1.0));
    state.sample_accelerations(first, 0.5);
    assert_eq!(
        state.accelerations[6],
        crate::math::Vector3::new(12.0, 4.0, -2.0)
    );
    state.sample_accelerations(first, 0.25);
    assert_eq!(state.accelerations, [crate::math::Vector3::ZERO; 7]);
    let mut next = first;
    next[6].x += 1.0;
    state.sample_accelerations(next, 0.25);
    assert_eq!(
        state.accelerations[6],
        crate::math::Vector3::new(4.0, 0.0, 0.0)
    );
    assert_eq!(state.accelerations[5], crate::math::Vector3::ZERO);
    let reset = super::BoardGroundState::default();
    assert_eq!(reset.previous_velocities, [crate::math::Vector3::ZERO; 7]);
    assert_eq!(reset.accelerations, [crate::math::Vector3::ZERO; 7]);
}

#[test]
fn impact_uses_preceding_velocity_even_after_the_solver_stops_the_board() {
    let mut ground = BoardGroundState::default();
    let mut before = [Vector3::ZERO; BODY_COUNT];
    before[6] = Vector3::new(-8.0, -3.0, 0.0);
    ground.sample_accelerations(before, 1.0 / 60.0);
    let wall = report(BodyId::Deck, Vector3::new(1.0, 0.0, 0.0));
    let floor = report(BodyId::Deck, UP);
    //Both actual solver reports have zero remaining relative velocity. The
    //wall still has the larger incoming impact, while the floor wins normal Y.
    ground.update(&[wall, floor], &WheelLineState::default(), UP, 80.0, false);
    ground.sample_accelerations([Vector3::ZERO; BODY_COUNT], 1.0 / 60.0);
    assert_eq!(ground.closing_velocity, Vector3::new(-8.0, -0.0, -0.0));
    assert_eq!(ground.maximum_closing_speed, 8.0);
    assert_eq!(ground.parts[6].normal, UP);
    ground.update(&[], &WheelLineState::default(), UP, 80.0, false);
    assert_eq!(ground.closing_velocity, Vector3::ZERO);
    assert_eq!(ground.maximum_closing_speed, 0.0);
}

#[test]
fn surface_flags_use_actual_contacts_and_reset_each_frame() {
    let mut ground = BoardGroundState::default();
    let mut first = report(BodyId::Deck, UP);
    first.other_surface = 12 << 7;
    first.position.y = 2.0;
    let mut last = first;
    last.position.y = 5.0;
    let mut lines = WheelLineState::default();
    lines.physics_surfaces[0] = 8;
    ground.update(&[first, last], &lines, UP, 80.0, false);
    assert_eq!(ground.collision_flags, 1 << 25);
    assert_eq!(ground.surface_twelve_height, 5.0);
    //A wheel ray reporting surface8 alone does not set the physical flag.
    ground.update(
        &[report(BodyId::RightFrontWheel, UP)],
        &lines,
        UP,
        80.0,
        false,
    );
    assert_eq!(ground.collision_flags, 1 << 31);
    ground.update(&[], &lines, UP, 80.0, false);
    assert_eq!(ground.collision_flags, 0);
    assert_eq!(ground.surface_twelve_height, 0.0);
}

#[test]
fn opposing_contacts_use_deck_projection_range_and_reset_without_deck_contacts() {
    let mut state = BoardGroundState::default();
    let lines = WheelLineState::default();
    let reports = [
        report(BodyId::Deck, Vector3::new(0.0, 0.75, 0.0)),
        report(BodyId::Deck, Vector3::new(0.0, -0.5, 0.0)),
        report(BodyId::FrontTruck, Vector3::new(0.0, -1.0, 0.0)),
    ];
    state.update(&reports, &lines, UP, 80.0, false);
    assert_eq!(state.opposing_contact, 1.25);
    state.update(&reports, &lines, Vector3::new(1.0, 0.0, 0.0), 80.0, false);
    assert_eq!(state.opposing_contact, 0.0);
    state.update(&reports[2..], &lines, UP, 80.0, false);
    assert_eq!(state.opposing_contact, 0.0);
}

/// 82C079E0 keeps tag & 0x7F, (tag >> 7) & 31 and (tag >> 12) & 0xF per wheel and clears all
/// three on a miss; 82C07D20 keeps the audio/seam bits of truck/deck reports (last wins) after
/// Reset 82C00CA0 cleared them.
#[test]
fn audio_material_and_seam_bits_are_preserved_from_the_same_tags() {
    let tag = (0xB << 12) | (3 << 7) | 0x45;
    let mut lines = WheelLineState::default();
    lines.publish([
        Some(WheelLineHit {
            fraction: 0.1,
            normal: UP,
            surface_tag: tag,
        }),
        None,
        None,
        None,
    ]);
    assert_eq!(lines.audio_surfaces, [0x45, 0, 0, 0]);
    assert_eq!(lines.physics_surfaces, [3, 0, 0, 0]);
    assert_eq!(lines.seam_patterns, [0xB, 0, 0, 0]);
    lines.publish([None; 4]);
    assert_eq!(lines.seam_patterns, [0; 4]);

    let mut ground = BoardGroundState::default();
    let mut first = report(BodyId::Deck, UP);
    first.other_surface = ((0x4 << 12) | 0x02) as u16;
    let mut last = report(BodyId::Deck, UP);
    last.other_surface = ((0x8 << 12) | (2 << 7) | 0x10) as u16;
    let mut truck = report(BodyId::BackTruck, UP);
    truck.other_surface = 0x03;
    ground.update(&[first, last, truck], &lines, UP, 80.0, false);
    assert_eq!(ground.part_audio_surfaces, [0, 0x03, 0x10]);
    assert_eq!(ground.part_seam_patterns, [0, 0, 0x8]);
    ground.update(&[], &lines, UP, 80.0, false);
    assert_eq!(ground.part_audio_surfaces, [0; 3]);
    assert_eq!(ground.part_seam_patterns, [0; 3]);
}
