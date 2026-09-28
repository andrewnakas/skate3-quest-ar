//!82D33E00 calls SkeletonUpdateAnimated82BDDA10 with fast blending enabled.
use super::*;
use crate::physics::{input_phase, skeleton_input_runtime::SkeletonOwners};
pub(super) fn advance(physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
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
        true,
    )?;
    Ok(())
}
