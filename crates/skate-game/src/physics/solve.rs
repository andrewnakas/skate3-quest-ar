//! One physical solve for the board, skater and original animation targets.
//! The caller publishes forces and drive targets before entering this phase.
mod assembly_contacts;
mod diagnostics;
use super::{GamePhysics, SkaterRuntime, colliders, skeleton_colliders};
use skate_core::physics::{
    board::BodyId,
    board_step::{ATTACHED_REACTION_BASE, AttachedStep},
    skeleton_animation_record::{AnimationPartTransform, IDENTITY},
    skeleton_body::PART_COUNT,
};

pub(super) fn advance(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
    truck_targets: [f32; 2],
) -> Result<(), String> {
    let before = diagnostics::snapshot(physics, skater);
    diagnostics::validate(&before, "before shared solve").map_err(|error| format!(
        "{error}; com_frame={:?}; lifted_com_frame={:?}; animation_root={:?}; biped_position={:?}; biped_surface={:?}",
        skater.animated_skeleton.board_frames.com_frame,
        skater.animated_skeleton.board_frames.lifted_com_frame,
        skater.animated_skeleton.roots.animation_to_world,
        skater.biped_ground.controller.state.position_368,
        skater.biped_ground.controller.state.surface,
    ))?;
    let mut board_volumes = colliders::world_volumes(&physics.board, &physics.settings);
    board_volumes.retain(|volume| skater.board_possession_live.volume_enabled(volume.body));
    let skeleton_volumes =
        skeleton_colliders::enabled_volumes(&skater.skeleton, &skater.skeleton_collision)?;
    // Each native assembly has its own query record and retention buffer.
    // Skeleton82BE5094 passes false to82768728: its edge threshold is -1,
    // whereas the board requests .999. GroundPipeline supplies the remaining
    // shared values. Do not let the second query overwrite the first's rows.
    let mut contacts = physics
        .world
        .query_primitives(&board_volumes, physics.query, physics.retention)
        .to_vec();
    let mut skeleton_query = physics.query;
    skeleton_query.edge_cos_bend_normal_threshold = -1.0;
    let mut skeleton_world_volumes = skeleton_volumes.clone();
    skeleton_colliders::retain_world_volumes(
        &mut skeleton_world_volumes,
        &skater.skeleton_collision,
    );
    contacts.extend_from_slice(physics.world.query_primitives(
        &skeleton_world_volumes,
        skeleton_query,
        physics.retention,
    ));
    assembly_contacts::append(
        &mut contacts,
        &board_volumes,
        &skeleton_volumes,
        physics.board.collision_group(),
        &skater.skeleton_collision,
    )?;
    // Remote actors use separate reaction indices after skeleton and targets.
    // Never apply the single-skater self-culling bitmap to another player.
    let before_remote = contacts.len();
    for a in board_volumes.iter().chain(&skeleton_volumes) {
        for b in &physics.network_proxies.volumes {
            let (ac, ar) = super::network::bounds(a.primitive);
            let (bc, br) = super::network::bounds(b.primitive);
            if ac.distance_squared(bc) <= (ar + br + 0.05).powi(2) {
                assembly_contacts::append_pair(&mut contacts, a, b);
            }
        }
    }
    physics.network_contacts = contacts.len() - before_remote;
    physics.contact_count = contacts.len();
    let dt = physics.settings.step.simulation.time_step;
    let mut joints =
        skater
            .skeleton_joints
            .build(skater.skeleton.bodies(), ATTACHED_REACTION_BASE, dt);
    let mut drives = skater.skeleton_drives.build(
        skater.skeleton.bodies(),
        ATTACHED_REACTION_BASE,
        ATTACHED_REACTION_BASE + PART_COUNT,
        dt,
    );
    let skeleton_drive_count = drives.rows.len();
    //82D74FD8: persistent hand drives share the deck and skeleton reactions.
    skater.board_possession.append_drives(
        physics.board.bodies()[BodyId::Deck.index()],
        [skater.skeleton.bodies()[3], skater.skeleton.bodies()[7]],
        BodyId::Deck.index(),
        [ATTACHED_REACTION_BASE + 3, ATTACHED_REACTION_BASE + 7],
        dt,
        &mut drives.rows,
    );
    let bodies = skater
        .skeleton
        .bodies_mut()
        .iter_mut()
        .chain(skater.skeleton_drives.targets.bodies.iter_mut())
        .chain(physics.network_proxies.bodies.iter_mut())
        .collect();
    physics.board.advance_attached(
        &contacts,
        truck_targets,
        physics.settings.step,
        AttachedStep {
            bodies,
            contacts: &mut [],
            joints: &mut joints,
            drives: &mut drives.rows,
        },
    );
    if let Err(error) = diagnostics::validate(
        &diagnostics::snapshot(physics, skater),
        "after shared solve",
    ) {
        return Err(format!(
            "{error}; input_bodies={before:?}; contacts={contacts:?}; joints={joints:?}; drives={:?}",
            drives.rows
        ));
    }
    skater
        .skeleton
        .publish_physical_record(deck_frame(&physics.board));
    // These solved rows are consumed by the actual collision/drive feedback
    // phase; keep their identity and impulses after the shared solve.
    // Possession drives have no skeleton spy identity. They participate in the
    // same solve above, but must not enter the skeleton-only feedback batch.
    drives.rows.truncate(skeleton_drive_count);
    skater.solved_drives = Some(drives);
    Ok(())
}

pub(super) fn deck_frame(
    board: &skate_core::physics::board_runtime::BoardRuntime,
) -> AnimationPartTransform {
    let deck = board.part_transforms()[BodyId::Deck.index()];
    let mut frame = IDENTITY;
    for (axis, column) in deck.basis.columns.iter().enumerate() {
        frame[axis][..3].copy_from_slice(column);
    }
    frame[3] = [
        deck.translation.x,
        deck.translation.y,
        deck.translation.z,
        0.0,
    ];
    frame
}
