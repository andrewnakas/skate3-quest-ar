//! Ordinary Ground82D37C88 executes persistent controller updates and actual
//!body forces before prediction and IK publication. Ground's handplant query
//!82D38430 is dispatched by ground_phase through the retained Handplant owner.
use super::{
    super::{
        animated_skeleton::AnimatedSkeleton, animation_input::AnimationInput, foot_ik::FootIk,
        riding_outputs::RidingOutputs,
    },
    GroundControllers, GroundInputObservations, GroundPhysicalFrame, GroundRuntime, GroundSettings,
    GroundState,
};
use skate_core::{
    math::Vector3,
    physics::{
        board_runtime::BoardRuntime, board_toolkit::BoardToolkit, board_world::BoardWorld,
        drive_frames::RetailAffineTransform,
    },
    player::input_phase::ProcessedPhysicsInput,
    riding::grounded::state::{board::GroundBoardOutcome, motion},
};

pub(crate) struct GroundUpdateFrame<'a> {
    pub processed: &'a ProcessedPhysicsInput,
    pub animation: &'a AnimationInput,
    pub riding: &'a RidingOutputs,
    pub skeleton: &'a AnimatedSkeleton,
    pub toolkit: &'a BoardToolkit,
    pub base_trucks: [RetailAffineTransform; 2],
    pub extra: GroundInputObservations,
}
pub(crate) struct GroundUpdateTargets<'a> {
    pub foot_ik: &'a mut FootIk,
    pub skeleton_elapsed_16505: &'a mut bool,
    /// Borrow the canonical SkeletonOutput correction latch (Skeleton16388).
    pub board_correction_pending: &'a mut bool,
    ///Applies to SkeletonDrives hook432 body and Skeleton16112 together.
    pub move_future_deck: &'a mut dyn FnMut(Vector3) -> Result<(), String>,
    pub offboard_grab: &'a mut super::super::biped_ground::grab_runtime::Owner,
}
impl GroundState {
    pub fn update(
        &mut self,
        runtime: &mut GroundRuntime,
        board: &mut BoardRuntime,
        world: &BoardWorld,
        settings: &GroundSettings,
        frame: GroundUpdateFrame<'_>,
        physical: GroundPhysicalFrame<'_>,
        targets: GroundUpdateTargets<'_>,
    ) -> Result<GroundBoardOutcome, String> {
        if !self.entered {
            return Err("Ground::Enter must finish before Ground::Update".into());
        }
        let p = frame.processed;
        let signals = self
            .state
            .begin_update_at(p.wheel_count_2556 as i32, frame.toolkit.deck[3]);
        if signals.set_skeleton_flag_16505 {
            *targets.skeleton_elapsed_16505 = true;
        }
        //GrabWorld (2476 bit22) requests Handplant::ground_query82D38430,
        //already submitted by ground_phase. It is not proof of a Skitch object.
        //The same Handplant owner handles the inactive reset and a query miss;
        //ordinary ground forces continue for both held and released input.
        let pumping_mode = self.pumping_settings.mode(p.state_variant_index_2528)?;
        self.pumping_settings.update(
            &mut self.pumping,
            frame.toolkit,
            frame.riding,
            frame.skeleton,
            p.flags_2476,
            pumping_mode,
        );
        let input = settings.input(
            frame.toolkit,
            p,
            frame.animation,
            &self.pumping,
            pumping_mode.unintentional_scalar,
            frame.riding,
            frame.skeleton,
            frame.base_trucks,
            frame.extra,
        );
        let controllers = GroundControllers {
            speed_wobble: &mut self.wobble,
            truck_steering: &mut self.steering,
            speed_model: &mut self.speed,
            manual: &mut self.manual,
            heading_previous: &mut self.heading_previous,
        };
        let outcome = runtime.update_board(
            board,
            world,
            settings,
            &mut self.state,
            controllers,
            input,
            physical,
        )?;
        let normal = p.vectors_464_480_496_512_528[0].map(f32::from_bits);
        if let Some(delta) = motion::future_deck_displacement(
            board.forces(),
            frame.toolkit.total_mass,
            p.timestep_2604,
            normal,
            self.state.flag_2720,
            self.state.manual_correction_2732,
        ) {
            (targets.move_future_deck)(delta)?;
        }
        if self.state.flag_2722 {
            targets.foot_ik.state.contacts.support_failed_this_update = true;
        }
        *targets.board_correction_pending = true;
        self.state.finish_update(p.timestep_2604);
        //82D37F10 calls82D749D0 on the same PlayerGrabSpline owner as offboard.
        targets.offboard_grab.invalidate();
        Ok(outcome)
    }
}
