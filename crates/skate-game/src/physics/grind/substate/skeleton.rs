//!82D40D28 calls82BDDA10 with fast_blend=false only for a pop THIS update.
use super::{GamePhysics, SkaterRuntime};
use crate::physics::{input_phase, skeleton_input_runtime::SkeletonOwners};
use skate_core::physics::skeleton_animation_record::AnimationPartTransform;

pub(super) fn animated(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
) -> Result<(), String> {
    animated_target(physics, skater).map(|_| ())
}

pub(super) fn animated_target(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
) -> Result<AnimationPartTransform, String> {
    let collision = input_phase::collision(skater);
    let mut owners = SkeletonOwners {
        animated: &mut skater.animated_skeleton,
        body: &mut skater.skeleton,
        drives: &mut skater.skeleton_drives,
        ik: &mut skater.foot_ik,
        animation_input: &mut skater.animation_input,
        correction: &mut skater.skeleton_output.correction,
        pose_errors: &mut skater.pose_errors,
    };
    skater.skeleton_input.update_animated(
        &mut skater.skeleton_air,
        &mut physics.board,
        &physics.riding.reckoning_frames.system,
        &mut skater.player_input.processed,
        &mut owners,
        &skater.animation.packet.hierarchy,
        &collision,
        physics.settings.step.simulation,
        false,
    )
}

///Nonspecific performs82C04368 in its subsequent board helper, after steering.
pub(super) fn ground_target(
    physics: &GamePhysics,
    skater: &mut SkaterRuntime,
) -> Result<AnimationPartTransform, String> {
    let collision = input_phase::collision(skater);
    let mut owners = SkeletonOwners {
        animated: &mut skater.animated_skeleton,
        body: &mut skater.skeleton,
        drives: &mut skater.skeleton_drives,
        ik: &mut skater.foot_ik,
        animation_input: &mut skater.animation_input,
        correction: &mut skater.skeleton_output.correction,
        pose_errors: &mut skater.pose_errors,
    };
    skater.skeleton_input.update_ground(
        &physics.board,
        &physics.riding.reckoning_frames.system,
        &mut skater.player_input.processed,
        &mut owners,
        &skater.animation.packet.hierarchy,
        &collision,
        physics.settings.step.simulation,
    )
}
