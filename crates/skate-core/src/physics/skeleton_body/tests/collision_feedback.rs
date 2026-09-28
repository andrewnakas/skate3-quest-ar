use super::*;
use crate::physics::{contact::RetailContactMaterial, skeleton_animation_record::IDENTITY};

#[test]
fn handplant_contact_mask_survives_updates_and_restores_on_exit() {
    let mut collision = SkeletonCollisionMode::new_normal(settings().body, false);
    for _ in 0..3 {
        collision.disable_handplant_contacts(2);
        // A driven-state refresh must not restore these volumes mid-plant.
        collision.select_driven(5).unwrap();
        for part in [1, 3, 4, 7, 8] {
            assert!(!collision.parts[part].enabled, "plant contact part {part}");
        }
        for part in [2, 5, 6, 9, 10, 15, 19] {
            assert!(collision.parts[part].enabled, "unaffected part {part}");
        }
        collision.finish_contact_frame();
        assert!(collision.pending_reenable);
        assert_eq!(collision.disable_count[3], 1);
        assert!(!collision.parts[3].enabled);
    }
    // No refresh from the plant on the next tick: restore normal contact.
    collision.finish_contact_frame();
    assert!(!collision.pending_reenable);
    for part in [1, 3, 4, 7, 8] {
        assert!(collision.parts[part].enabled);
        assert_eq!(collision.disable_count[part], 0);
    }
}

fn settings() -> SkeletonFeedbackSettings {
    SkeletonFeedbackSettings {
        body: SkeletonCollisionSettings {
            enabled: true,
            normal_material: RetailContactMaterial {
                static_friction: 0.0,
                dynamic_friction: 0.0,
                restitution: 0.0,
            },
            compliant: [true; 24],
            priority: [0.0; 24],
            effect_time: 0.25,
        },
        small_object_mass: 5.5,
        ground_plane_max_distance: 0.3,
        ground_plane_max_angle: 0.8,
        skater_scalar: 0.5,
        ai_scalar: 1.0,
        groin_offset: [0.0; 4],
        face_offset: [0.0; 4],
        groin_radius: 0.0,
        face_radius: 0.0,
    }
}
fn input<'a>(
    physical: &'a SkeletonPhysicalRecord,
    frames: &'a [[[f32; 4]; 4]; 26],
    weights: &'a [f32; 24],
) -> SkeletonCollisionInput<'a> {
    SkeletonCollisionInput {
        dt: 0.125,
        plane_point: [0.0; 4],
        plane_normal: [0.0, 1.0, 0.0, 0.0],
        reference_velocity: [2.0, 0.0, 0.0, 0.0],
        com_velocity: [0.0; 4],
        ragdoll: false,
        disable_ground_filter: false,
        ai_collision_scalar: false,
        offboard: false,
        entering_offboard: false,
        category_600: false,
        request_partial_ragdoll: false,
        physical,
        part_weights: weights,
        body_frames: frames,
    }
}
fn report(normal: [f32; 4]) -> SkeletonContactReport {
    SkeletonContactReport {
        part: 2,
        normal,
        point: [0.0; 4],
        tag: 0,
        other_group: 0,
        other_entity: None,
        body_a: SkeletonContactBody {
            state_flags: 4,
            inverse_mass: 0.5,
            linear_velocity: [2.0, 0.0, 0.0, 0.0],
        },
        body_b: SkeletonContactBody {
            state_flags: 1,
            inverse_mass: 0.0,
            linear_velocity: [0.0; 4],
        },
        side_a: true,
        solved_vector: [100.0, 0.0, 0.0, 0.0],
    }
}

#[test]
fn support_contact_is_filtered_but_wall_contact_changes_drives_and_recovers() {
    let mut physical = SkeletonPhysicalRecord::default();
    physical.velocities[2] = [2.0, 0.0, 0.0, 0.0];
    let frames = [IDENTITY; 26];
    let weights = [1.0; 24];
    let input = input(&physical, &frames, &weights);
    let mut collision = SkeletonCollisionFeedback::new(settings());
    collision.update(&input, &[report([0.0, 1.0, 0.0, 0.0])]);
    assert!(collision.flags.any);
    assert!(!collision.flags.compliant);
    assert_eq!(collision.drive_weight, 1.0);
    collision.update(&input, &[report([-1.0, 0.0, 0.0, 0.0])]);
    assert!(collision.flags.compliant && collision.flags.has_impulse);
    assert_eq!(collision.bones[2].force, 2.0);
    assert_eq!(collision.drive_weight, 0.0);
    assert!(collision.regions[1].weighted_force.is_finite());
    collision.update(&input, &[]);
    assert_eq!(collision.drive_weight, 0.5);
    collision.update(&input, &[]);
    assert_eq!(collision.drive_weight, 1.0);
}

#[test]
fn tiny_dynamic_opponent_is_noncompliant_even_for_compliant_bone() {
    let mut physical = SkeletonPhysicalRecord::default();
    physical.velocities[2] = [2.0, 0.0, 0.0, 0.0];
    let frames = [IDENTITY; 26];
    let weights = [1.0; 24];
    let input = input(&physical, &frames, &weights);
    let mut contact = report([-1.0, 0.0, 0.0, 0.0]);
    contact.body_b.state_flags = 4;
    contact.body_b.inverse_mass = 1.0;
    let mut collision = SkeletonCollisionFeedback::new(settings());
    collision.update(&input, &[contact]);
    assert_eq!(collision.bones[2].force, 1.0);
    assert!(collision.flags.noncompliant);
    assert!(!collision.flags.compliant && !collision.flags.has_impulse);
}

#[test]
fn contact_planes_constrain_physical_pose_error_without_inventing_impulse() {
    let mut collision = SkeletonCollisionFeedback::new(settings());
    let axis = [0.0, 1.0, 0.0, 0.0];
    let error = [2.0, 3.0, 0.0, 0.0];
    assert_eq!(collision.filter_error(error, axis), [0.0; 4]);
    collision.planes.push(ContactPlane {
        normal: [1.0, 0.0, 0.0, 0.0],
        part: 2,
    });
    let filtered = collision.filter_error(error, axis);
    assert!((filtered[0] - 2.0).abs() < 1e-5);
    assert_eq!(filtered[1], 0.0);
    let diagonal = 0.5_f32.sqrt();
    collision.planes[0].normal = [diagonal, 0.0, diagonal, 0.0];
    let projected = collision.filter_error(error, axis);
    assert!((projected[0] - 1.0).abs() < 1e-5);
    assert!((projected[2] - 1.0).abs() < 1e-5);
    collision.planes[0].normal = [-1.0, 0.0, 0.0, 0.0];
    assert_eq!(collision.filter_error(error, axis), [0.0; 4]);
}
