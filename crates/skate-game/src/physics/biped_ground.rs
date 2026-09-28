use super::{GamePhysics, SkaterRuntime};
mod entry;
pub(crate) mod feet;
pub(crate) mod grab;
pub(crate) mod grab_runtime;
pub(crate) mod ground_board;
mod lifecycle;
pub(crate) mod services;
pub(crate) mod sync;
pub(crate) use lifecycle::{fill, post, publish_fields};
use skate_core::player::offboard::{contact_toolkit, ground_entry, ground_job};
pub(crate) struct Owner {
    pub controller: skate_core::player::offboard::controller::Controller,
    pub result: Option<skate_core::player::offboard::controller::GroundResult>,
    pub ground: ground_entry::State,
    pub contact: contact_toolkit::ContactPrefix,
    pub geometry_adjustment: Option<skate_core::player::offboard::ground_query::GroundAdjustment>,
    skeleton_state: super::offboard::skeleton_ground::State,
    geometry: super::offboard::ground_geometry::State,
    movement_vs_stick_angle: skate_core::point_graph::PointGraph<8>,
    turn_vs_stick_angle: skate_core::point_graph::PointGraph<8>,
    air_launch: skate_core::player::offboard::air_launch::Settings,
    collision_settings: skate_core::player::offboard::ground_lifecycle::CollisionSettings,
    pub grab_settings: skate_core::player::offboard::ground_sync::BoardSettings,
}

impl Owner {
    ///Only the shared82D8E3E0 callee, not Ground movement/lifecycle.
    ///Ground82BDE2CC and Air82BDDFFC both supply enable-body-spin r7=1.
    ///Each caller supplies its own native up/forward/blend and phase ordering.
    pub(crate) fn finish_reckoning(
        &self,
        update: super::offboard::skeleton_ground::ReckoningUpdate,
        riding: &mut super::riding_outputs::RidingOutputs,
        air: &mut super::air_reckoning::AirReckoning,
        processed: &skate_core::player::input_phase::ProcessedPhysicsInput,
        physical_body_spin: f32,
    ) -> skate_core::player::offboard::ground_reckoning::Output {
        self.skeleton_state
            .finish_reckoning(update, riding, air, processed, physical_body_spin)
    }

    pub(crate) fn air_settings(
        &self,
    ) -> (
        skate_core::point_graph::PointGraph<8>,
        skate_core::player::offboard::air_launch::Settings,
    ) {
        (self.turn_vs_speed(), self.air_launch)
    }

    fn turn_vs_speed(&self) -> skate_core::point_graph::PointGraph<8> {
        self.controller.settings.movement_velocity.turn_vs_speed
    }

    pub(crate) fn load(
        data: &skate_data::collections::Collections,
        metadata: &skate_data::animation_metadata::AnimationMetadata,
    ) -> Result<Self, String> {
        let settings = super::offboard::settings::Settings::load(data, metadata)?;
        let grab_settings = settings.board;
        let (
            controller_settings,
            metrics,
            movement_vs_stick_angle,
            turn_vs_stick_angle,
            air_launch,
        ) = settings.into_controller_parts();
        Ok(Self {
            controller: skate_core::player::offboard::controller::Controller::new(
                controller_settings,
                metrics,
            ),
            result: None,
            ground: ground_entry::State::default(),
            contact: contact_toolkit::ContactPrefix::reset(),
            geometry_adjustment: None,
            skeleton_state: super::offboard::skeleton_ground::State::load(data)?,
            geometry: super::offboard::ground_geometry::State::load(data)?,
            movement_vs_stick_angle,
            turn_vs_stick_angle,
            air_launch,
            collision_settings: lifecycle::load_collision_settings(data)?,
            grab_settings,
        })
    }

    pub(crate) fn reset(&mut self, toolkit: &mut contact_toolkit::Owner) {
        //82D30BD0 is a Ground-state reset, not Biped82D7B1C0.
        //Keep controller cadence, correction vectors and shared candidate alive.
        self.result = None;
        self.ground.flags_144_to_150 = [false; 7];
        self.ground.counter_152 = 0;
        self.ground.counter_156 = 0;
        self.ground.elapsed_160 = 0.;
        toolkit.reset_history();
        self.ground = ground_entry::State::default();
        self.contact = contact_toolkit::ContactPrefix::reset();
        self.geometry_adjustment = None;
    }

    pub(crate) fn run(
        &mut self,
        job: skate_core::player::offboard::controller::GroundJob,
    ) -> skate_core::player::offboard::controller::GroundResult {
        let result = self.controller.step_ground(&job);
        self.result = Some(result);
        result
    }

    pub(crate) fn enter(
        &mut self,
        input: ground_entry::Input,
        current_state: u32,
        previous_frame: [[f32; 4]; 4],
        toolkit: &mut contact_toolkit::Owner,
    ) {
        self.reset(toolkit);
        self.geometry.reset();
        let placement = self.ground.enter(&input);
        self.controller
            .place(skate_core::player::offboard::controller::PlacementInput {
                frame: placement.frame,
                velocity: placement.planar_velocity,
                body_position: placement.body_position,
                current_state,
                previous_state: input.previous_state_2504,
                previous_frame,
            });
    }
}

pub(crate) fn enter(_physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    let p = &skater.player_input.processed;
    //82D30848 calls GetEffectiveRoot82BE3650 BEFORE Ground placement.
    let frame = entry::effective_root(
        skater.animated_skeleton.roots.animation_to_world,
        p.flags_2476,
    );
    let previous_frame = p
        .effective_anim_transform_192
        .map(|v| v.map(f32::from_bits));
    skater.biped_ground.enter(
        ground_entry::Input {
            animation_frame: frame,
            processed_velocity_608: p.vectors_544_560_592_608[3].map(f32::from_bits),
            processed_flags_2484: p.flags_2484,
            processed_flags_2476: p.flags_2476,
            requested_angle_2936: skater.animation_input.extra.biped_start_angle,
            requested_duration_2896: skater.animation_input.fields.animation_time,
            previous_state_2504: p.state_2504,
            previous_frame_up_208: previous_frame[1],
            //82BDE398..3E0: COM frame15824 + translation48 = Skeleton15872.
            body_position_15872: skater.animated_skeleton.board_frames.com_frame[3],
        },
        p.state_2508,
        previous_frame,
        &mut skater.offboard_contact,
    );
    //82D30B44: actual state56 ConsiderReset, after placement and before mode4.
    services::enter_feet(skater);
    //82D30B54 requests the shared skeleton collision controller's mode4.
    skater
        .ground_lifecycle
        .skeleton_controller
        .request(4, &mut skater.skeleton_collision)?;
    //82D30B5C..B64: state52 grab manager, NOT SkateboardController60.
    skater.offboard_grab.enter_reset();
    Ok(())
}

pub(crate) fn exit(skater: &mut SkaterRuntime) {
    //82D30CD8 refreshes, then30CE4..CF8 clears readiness/history before
    //releasing Ground geometry. Candidate and Biped cadence remain alive.
    skater.offboard_contact.reset_history();
    //82D30CC0 releases the ground query. It does NOT reset shared Biped
    //motion/cadence/correction; BipedAir and subsequent placement consume them.
    skater.biped_ground.geometry.reset();
}

pub(crate) fn update(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
    _controls: &super::PlayerControls,
    contact: ground_job::ContactSnapshot,
) -> Result<contact_toolkit::Input, String> {
    let p = skater.player_input.processed;
    let fields = skater.animation_input.fields;
    let extra = skater.animation_input.extra;
    let animation_motion_sq = fields.animation_translation[..3]
        .iter()
        .map(|value| value * value)
        .sum::<f32>();
    if p.flags_2488 & 0x0800_0000 != 0
        && skater.biped_ground.controller.state.motion.target_scale_784 < 0.0
        && animation_motion_sq < f32::from_bits(0x3a83_126f)
        && !(fields.animation_time > 0.0)
    {
        // Temporary first-producer trace. Native Ground movement82D7F4BC enters
        // the directed branch and82D7F624 divides target distance by AnimTime;
        // stock graph publication must make this packet valid before that call.
        bevy::log::error!(
            "OFFBOARD_DIRECTED_INPUT_INVALID state={}/{} previous={} filtered={} motion_state={:?} flags={:08x}/{:08x}/{:08x}/{:08x}/{:08x} grind_words={:08x?} grind_flags={:02x}/{:02x} offboard_edge_distance={} offboard_obstacle_distance={} anim_time={} anim_translation={:?} anim_velocity={:?} target_scale={} frame={:?} attributes={:?}",
            p.state_2508,
            p.category_2512,
            p.state_2504,
            skater.player_input.physical.filtered_state_0,
            skater.animation.motion_controller.frame.current,
            p.flags_2468,
            p.flags_2472,
            p.flags_2476,
            p.flags_2480,
            p.flags_2488,
            p.grind_words_2532_2536,
            skater.player_input.physical.grinds.flag_318,
            skater.player_input.physical.grinds.flag_322,
            skater.player_input.physical.off_board.distance_116,
            skater.player_input.physical.off_board.scalar_112,
            fields.animation_time,
            fields.animation_translation,
            skater.animated_skeleton.motion.velocity_world,
            skater.biped_ground.controller.state.motion.target_scale_784,
            skater.biped_ground.ground.frame_80,
            skater.animation.attributes.entries(),
        );
    }
    let frame = skater.biped_ground.ground.frame_80;
    let input = skate_core::player::offboard::ground_input::GroundInput {
        processed_flags_2472: p.flags_2472,
        processed_direct_2684: extra.offboard_magnitude,
        processed_direct_2680: extra.offboard_turn,
        processed_stick_2692: extra.biped_world_x,
        processed_stick_2688: extra.biped_world_z,
        processed_scale_2912: fields.magnitude_scale,
        processed_scale_2908: fields.turn_scale,
        frame_forward_112: [frame[2][0], frame[2][1], frame[2][2]],
    };
    let prepared = skate_core::player::offboard::ground_input::calculate(
        &input,
        &skater.biped_ground.movement_vs_stick_angle,
        &skater.biped_ground.turn_vs_stick_angle,
    );
    let third_line = p.line_tests_960_1008_1056[2];
    let owner = &mut skater.biped_ground;
    let job = ground_job::prepare(
        &mut owner.contact,
        &mut owner.ground.distance_164,
        ground_job::Input {
            contact,
            frame,
            previous_state: p.state_2504,
            third_line_position: (third_line.valid != 0)
                .then(|| third_line.position.map(f32::from_bits)),
            frames_since_teleport: p.frames_since_teleport_2584,
            processed_position: p.vectors_544_560_592_608[2].map(f32::from_bits),
            processed_velocity: p.vectors_544_560_592_608[3].map(f32::from_bits),
            controls: prepared,
            collision_displacements: skater.collision_extra_errors,
            animation_motion: fields.animation_translation,
            animation_velocity: skater.animated_skeleton.motion.velocity_world,
            requested_duration: fields.animation_time,
            requested_phase: fields.cadence_end_percent,
            override_duration: fields.animation_physics_blend_seconds,
            flags_2472: p.flags_2472,
            flags_2476: p.flags_2476,
            flags_2480: p.flags_2480,
            flags_2484: p.flags_2484,
            flags_2488: p.flags_2488,
        },
        |input| Ok::<_, String>(owner.geometry.consume(input)),
    )?;
    owner.geometry_adjustment = Some(job.geometry);
    let result = owner.run(job.job);
    if result
        .physical_frame
        .iter()
        .flatten()
        .chain(result.animation_frame.iter().flatten())
        .chain(result.surface_frame.iter().flatten())
        .chain(result.velocity.iter())
        .chain(result.position.iter())
        .any(|value| !value.is_finite())
    {
        // Temporary first-divergence trace; do not repair or suppress the value.
        bevy::log::error!(
            "OFFBOARD_GROUND_RESULT_NONFINITE job={:?} result={:?} controller={:?}",
            job.job,
            result,
            owner.controller.state,
        );
    }
    //82D31CE8..D04 is Ground FeetIK, not Air's timer-reset wrapper.
    services::update_feet(skater);
    //82D31D14 calls the shared board steering owner with zero input before
    //possession. Offboard Ground must still advance its tilt and timers.
    skater.ground.steering.update(
        0.0,
        skater.air_settings.steering_blend,
        p.flags_2468,
        p.flags_2472,
    );
    //82D31D1C..DB0 operates the EXISTING carried-board controller.
    ground_board::update_possession(physics, skater);
    //82D32338 precedes contact correction and angular unwind. Submit through
    //the parent's persistent selector; Ground never selects Air speculatively.
    //Current authored scene has no grind edges. Typed absence is accepted by
    //every native non-mode1 launch; no fabricated departure vectors.
    let launch_input = services::launch_input(skater, None)?;
    if let Some(packet) = skater.biped_ground.prepare_air(&launch_input)? {
        services::submit_air(physics, skater, packet)?;
    }
    let owner = &mut skater.biped_ground;
    owner.ground.flags_144_to_150[0] = false;
    // Sync82D31E64..82D32070: physical contact correction and the Enter
    // angular unwind must precede BOTH Skeleton and the next query submission.
    let animation_frame = ground_job::sync_frames(&mut owner.ground, owner.contact, result);
    let collision = super::input_phase::collision(skater);
    //82D32084 passes state1056 (GroundResult208), not processed752.
    let centre_of_mass = result.position;
    let body_spin = skater.animation_input.extra.physical_body_spin;
    let simulation = physics.settings.step.simulation;
    let mut owners = super::skeleton_input_runtime::SkeletonOwners {
        animated: &mut skater.animated_skeleton,
        body: &mut skater.skeleton,
        drives: &mut skater.skeleton_drives,
        ik: &mut skater.foot_ik,
        animation_input: &mut skater.animation_input,
        correction: &mut skater.skeleton_output.correction,
        pose_errors: &mut skater.pose_errors,
    };
    let mut reckoning_update = None;
    skater.skeleton_input.update_biped_ground(
        &mut skater.skeleton_air,
        &mut physics.board,
        crate::physics::offboard::skeleton_ground::Input {
            world_frame: &animation_frame,
            centre_of_mass_1056: centre_of_mass,
        },
        &mut skater.biped_ground.skeleton_state,
        &mut skater.player_input.processed,
        &mut owners,
        &skater.animation.packet.hierarchy,
        &collision,
        simulation,
        |update| {
            reckoning_update = Some(update);
            Ok(())
        },
    )?;
    if let Some(update) = reckoning_update {
        skater.biped_ground.skeleton_state.finish_reckoning(
            update,
            &mut physics.riding,
            &mut skater.air_reckoning,
            &skater.player_input.processed,
            body_spin,
        );
    }
    //82D32088..32108: real grab-spline host executes grab::sync here.
    services::sync_grab(skater);
    //82D32130..164: surface-frame axes, NOT physical-frame axes.
    Ok(contact_toolkit::Input {
        position: skater.biped_ground.ground.frame_80[3],
        forward: result.surface_frame[2],
        up: result.surface_frame[1],
        right: result.surface_frame[0],
        velocity: result.velocity,
        animation_up: animation_frame[1],
        animation_right: animation_frame[0],
    })
}

/// Call immediately AFTER submitting update's toolkit input through the shared
/// owner. The seven real line results become visible at the NEXT PreUpdate.
pub(crate) fn submit_geometry(
    physics: &GamePhysics,
    skater: &mut SkaterRuntime,
) -> Result<(), String> {
    let p = &skater.player_input.processed;
    let owner = &mut skater.biped_ground;
    let result = owner
        .result
        .ok_or("Ground geometry submission requires completed motion")?;
    owner.geometry.submit(
        &physics.world,
        owner.ground.frame_80,
        result.velocity,
        skate_core::player::offboard::ground_query::QueryContext {
            selection_flags_2948: p.actor_query_2948,
            matching_id_2952: p.actor_query_2952 as i32,
        },
        p.flags_2488,
    )
}
