//! Original Biped Skeleton update82BDE060 with the existing physical owners.
mod frames;
#[cfg(test)]
mod pose_audit;
mod state;
use crate::physics::{
    skeleton_air::SkeletonAir,
    skeleton_input_runtime::{CollisionInput, SkeletonInputRuntime, SkeletonOwners},
};
#[cfg(test)]
pub(crate) use pose_audit::audit_render_parts;
use skate_core::{
    animation::output::NativeMatrix,
    physics::{
        board_runtime::BoardRuntime, rigid_body::RetailSimulationStep,
        skeleton_animation_record::AnimationPartTransform as Transform,
    },
    player::input_phase::ProcessedPhysicsInput,
};
pub(crate) use state::State;
///Actual Sync input: caller's constructed frame and Biped state1056 COM.
///State retains animation-space16016 independently of world-space12496.
pub(crate) struct Input<'a> {
    pub world_frame: &'a Transform,
    pub centre_of_mass_1056: [f32; 4],
}
///Terminal82D8E3E0 arguments, emitted after GeneralUpdate and trajectory clear.
///The caller invokes the canonical ground_reckoning producer in this callback.
pub(crate) struct ReckoningUpdate {
    pub up: [f32; 4],
    pub forward: [f32; 4],
    pub blend: f32,
}
impl SkeletonInputRuntime {
    pub(crate) fn update_biped_ground<F>(
        &mut self,
        air: &mut SkeletonAir,
        board: &mut BoardRuntime,
        input: Input<'_>,
        state: &mut State,
        p: &mut ProcessedPhysicsInput,
        owners: &mut SkeletonOwners<'_>,
        globals: &[NativeMatrix],
        collision: &CollisionInput,
        simulation: RetailSimulationStep,
        finish_reckoning: F,
    ) -> Result<Transform, String>
    where
        F: FnOnce(ReckoningUpdate) -> Result<(), String>,
    {
        let s = &mut owners.animated;
        let target = frames::prepare_pose(
            &mut s.roots,
            &mut s.board_frames,
            &s.record.pose[0],
            &self.drive_frames[0],
            &mut state.retained_board,
            input,
            p.flags_2476,
            p.flags_2484,
            &mut p.flags_2468,
        );
        if p.flags_2480 & 0x8000 != 0 || p.flags_2484 & 1 != 0 {
            //Reuse the shared Ground/Air history and actual board anchor.
            s.board_frames.physical_board = air.apply_board(board, &target, true);
        }
        self.general_update(p, owners, globals, collision, simulation)?;
        owners.animated.finish_ground();
        let root = owners.animated.roots.animation_to_world;
        finish_reckoning(ReckoningUpdate {
            up: root[1],
            forward: root[2],
            blend: 0.5,
        })?;
        owners.animation_input.fields.flags2468 = p.flags_2468;
        Ok(target)
    }
}
