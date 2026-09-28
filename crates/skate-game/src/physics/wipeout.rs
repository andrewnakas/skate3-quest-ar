//! Shared Ground/Air wipeout requests from original TU3 82D8F9E0/82D90358.
mod observations;
mod settings;
pub(crate) use observations::Observations;
use skate_core::player::{
    input_phase::ProcessedPhysicsInput,
    wipeout::{self, Mode, Requests, Settings},
};
use skate_data::collections::Collections;
pub(crate) struct Wipeout {
    pub state: Requests,
    settings: Settings,
    modes: [Mode; 5],
}
impl Wipeout {
    pub fn check_air_collision(
        &mut self,
        p: &ProcessedPhysicsInput,
        frame: &wipeout::Frame,
    ) -> Result<(), String> {
        let mode = self.mode(p)?;
        wipeout::check_air_collision(&mut self.state, &self.settings, &mode, frame);
        Ok(())
    }
    pub fn load(data: &Collections) -> Result<Self, String> {
        let (settings, modes) = settings::load(data)?;
        let mut state = Requests::new();
        state.initialize_player(); //Player82DB3024, after the component ctor.
        Ok(Self {
            state,
            settings,
            modes,
        })
    }
    pub fn check_ground(&mut self, input: &Observations<'_>) -> Result<(), String> {
        let mode = self.mode(input.processed)?;
        let frame = input.frame()?;
        wipeout::check_ground(&mut self.state, &self.settings, &mode, &frame);
        Ok(())
    }
    pub fn check_ground_animation(
        &mut self,
        input: &Observations<'_>,
        scale: f32,
    ) -> Result<(), String> {
        let mode = self.mode(input.processed)?;
        let frame = input.frame()?;
        wipeout::check_ground_animation(&mut self.state, &self.settings, &mode, &frame, scale);
        Ok(())
    }
    pub fn check_air(&mut self, input: &Observations<'_>, use_com: bool) -> Result<(), String> {
        let mode = self.mode(input.processed)?;
        let frame = input.frame()?;
        wipeout::check_air(&mut self.state, &self.settings, &mode, &frame, use_com);
        Ok(())
    }
    pub(crate) fn mode(&self, p: &ProcessedPhysicsInput) -> Result<Mode, String> {
        self.modes
            .get(p.state_variant_index_2528 as usize)
            .copied()
            .ok_or_else(|| {
                format!(
                    "Undefined wipeout physics mode {}",
                    p.state_variant_index_2528
                )
            })
    }
    ///Consumed by state selection and by the same tick's postphysics IK branch.
    pub fn requests_runout(&self, p: &ProcessedPhysicsInput) -> bool {
        self.state.requests_runout(&observations::request_input(p))
    }
    pub fn requests_wipeout(&self, p: &ProcessedPhysicsInput) -> bool {
        self.state.requests_wipeout(&observations::request_input(p))
    }
}

///Player postphysics82BD83E0 follows contact/error publication with the
///selected state's check, before FootIK sees that tick's wipeout request.
pub(super) fn check_after_physics(
    physics: &super::GamePhysics,
    skater: &mut super::SkaterRuntime,
) -> Result<(), String> {
    use skate_core::player::state::PhysicalStateId;
    let observations = Observations {
        processed: &skater.player_input.processed,
        board: &physics.riding.ground,
        collision: &skater.collision_feedback,
        deck: super::solve::deck_frame(&physics.board),
        //Ground82BDF694..6A4 stores this target into Processed0..48;
        //Animated/KnownAir publish the same unblended target before board drive.
        input_board: skater.animated_skeleton.board_frames.animation_target,
        world_to_animation: skater.animated_skeleton.roots.world_to_animation,
        pose_error: skater.collision_pose_error,
        maximum_pose_error: skater.collision_maximum_error,
        jump_fix_frames: skater.player_state.post.jump_fix_frames,
        air: &skater.air_reckoning.state,
        system_up_y: physics.riding.reckoning_frames.system[1][1],
        grind_locked_to_middle: skater.trajectory.selector.grind_locked_to_middle(),
        grind_normal: skater.trajectory.selector.grind_normal(),
    };
    match skater.player_state.current() {
        PhysicalStateId::FootPlant | PhysicalStateId::HandPlant => {
            let frame = observations.frame()?;
            if skater.player_state.current() == PhysicalStateId::HandPlant {
                super::handplant::trace_solved(physics, skater);
                //HandPlant vtable823273CC slot40 ->82D4C530 ->82D90358(false).
                //FootPlant's82D4C6D8 alone uses82D8FDC0. Its low trick-impact
                //thresholds must not replace HandPlant's normal air checks.
                skater.wipeout.check_air(&observations, false)?;
            } else {
                wipeout::check_plant(&mut skater.wipeout.state, &skater.wipeout.settings, &frame);
            }
            if skater.wipeout.state.count != 0 {
                let reasons: Vec<_> = skater
                    .wipeout
                    .state
                    .reasons
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &active)| active.then_some(i))
                    .collect();
                bevy::log::info!(
                    "PLANT_BAIL tick={} state={:?} reasons={reasons:?} pose_error={:?} max_pose_error={} regions={:?} closing={:?} board_contact={} world_to_animation={:?}",
                    physics.ticks,
                    skater.player_state.current(),
                    frame.pose_error,
                    frame.maximum_pose_error,
                    frame.regions_force,
                    frame.closing_velocity,
                    frame.board_contact,
                    frame.world_to_animation
                );
            }
            if skater.player_state.current() == PhysicalStateId::FootPlant {
                super::footplant::ground::post_physics(skater);
            }
            Ok(())
        }
        PhysicalStateId::PhysicsGround
        | PhysicalStateId::SlideGround
        | PhysicalStateId::RevertGround => skater.wipeout.check_ground(&observations),
        PhysicalStateId::GroundAnimation => {
            skater.wipeout.check_ground_animation(&observations, 1.0)
        }
        PhysicalStateId::PhysicsAir | PhysicalStateId::PhysicsAirSecondary => {
            skater.wipeout.check_air(&observations, false)
        }
        _ => Ok(()), //Other concrete states dispatch their own postphysics check.
    }
}
