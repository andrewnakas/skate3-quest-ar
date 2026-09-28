//! One production tick: controller graphs, physical input/state, shared solve,
//! physical animation output and the normal gameplay camera.
use super::{
    GamePhysics, PlayerControls, SkaterRuntime, animation_phase, camera_output, ground_phase,
    input_phase, player_state, solve,
};
use crate::{camera::CameraRuntime, graph_runtime::StockGraphs};
use skate_core::{
    input::controller::ActionMap,
    math::Vector3,
    physics::{
        board::BodyId,
        phase::{PhysicalOutputSnapshot, PhysicsEvent},
    },
};

pub(super) fn advance(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
    controls: &mut PlayerControls,
    graphs: &StockGraphs,
    actions: &mut dyn ActionMap,
    input_available: bool,
    camera: &mut CameraRuntime,
) -> Result<(), String> {
    let tick = physics.ticks;
    super::offboard_audit_trace::stage(tick, "begin", physics, skater, controls);
    #[cfg(test)]
    super::offboard_root_trace::trace(tick, "begin", skater);
    physics.exchange = super::SimulationExchange::new(tick);
    //SimController8285C968 dispatches the preceding tick's camera messages
    //before simulation. Keep End/Begin ordering when both occur in one update.
    for request in camera.simulation_rate_requests.drain(..) {
        if !physics.network_active {
            physics.clock.apply(request)?;
        }
    }
    if physics.network_active {
        physics.clock = super::clock::SimulationClock::default();
    }
    if physics.ticks == 0 {
        player_state::initialize(physics, skater)?;
        //Ctor82DB3008 enters Ground without ProcessOutput. Keep the constructed
        //State/FilteredState zeros (82DE3BE8/82DE5588) until the completed tick.
        //Publishing Ground here runs its ForcePhysics Begin before the initial
        //input reset82DB8998, which would immediately clear that mode again.
    }
    if super::climbing::advance(physics, skater, controls, camera)? {
        return Ok(());
    }
    //8285A06C/A080 completes prior trajectory batches, then A0D0 calls
    //Player slot16=82DB3C78 (vtable82328688). Consume with retained input
    //BEFORE this frame's queries, ProcessInput, and state selection.
    super::biped_air::consume_selector(physics, skater)?;
    physics.board.clear_forces();
    //World8275EA20 starts both batches before actor SetUpPhysics. Foot
    //results stay pending while PlayerInput consumes the preceding records.
    physics
        .riding
        .start_wheel_queries(&physics.board, &physics.world)?;
    let skeleton_queries = super::foot_ik_queries::query(&physics.world, &skater.skeleton)?;
    let animation = bevy::log::info_span!("fixed_animation_graphs")
        .in_scope(|| {
            animation_phase::advance(
                physics,
                skater,
                controls,
                graphs,
                &physics.animation_profile,
            )
        })
        .map_err(|e| format!("Animation tick{}: {e}", physics.ticks))?;
    super::offboard_audit_trace::stage(tick, "animation", physics, skater, controls);
    #[cfg(test)]
    super::offboard_root_trace::trace(tick, "animation", skater);
    //ForcePhysics Begin82BB2868 writes the live Skeleton16420 mode once.
    //Consume the graph request so a later physical reset can retain its own0.
    if let Some(mode) = skater.animation.motion.riding.force_mode.take() {
        skater.skeleton_input.force_mode = mode.native_value();
    }
    //PlayerUI82898920 remaps the gameplay packet before both controller
    //and physical input consume it. Reuse this tick's sampled mapping.
    let mut simulation_actions = controls.simulation_actions(actions);
    let teleported = input_phase::advance(
        physics,
        skater,
        &animation.packet(),
        &mut simulation_actions,
        input_available,
    )?;
    skater.ground_settings = skater.ground_profiles.select(
        skater.player_input.processed.state_variant_index_2528,
        skater.player_input.processed.surface_mode_2540,
    )?;
    if physics.trainer != skate_mods::TrainerTuning::default() {
        skater.ground_settings = std::sync::Arc::new(skater.ground_settings.tuned(physics.trainer));
    }
    let mut vehicle_ejected = false;
    if teleported {
        skater.respawn.reset_measurements();
        #[cfg(test)]
        super::offboard_root_trace::trace(tick, "teleport", skater);
        //82DB93B0..CC: complete pending queries and clear contact history.
        skater.offboard_contact.reset_history();
        skater.player_state.reset_for_teleport();
        skater.centre_of_mass_filter.reset();
        skater.animation_feedback.reset();
        player_state::enter_after_teleport(physics, skater)?;
        super::offboard::board_manager::runtime::finish_teleport(physics, skater);
        //82DB8E40 calls SetUpNormal after pose reset, state100 and board reset.
        skater.wipeout_state.ragdoll.restore_normal(
            &mut skater.skeleton,
            &mut skater.skeleton_joints,
            &mut skater.skeleton_collision,
            &mut skater.collision_feedback,
        );
        vehicle_ejected = player_state::apply_vehicle_ejection(physics, skater)?;
    }
    //World8275EC0C ends board queries before PostInput/state selection.
    physics.riding.finish_wheel_queries()?;
    //Complete preceding-state submissions before PostInput82DB573C publishes
    //them. Host execution is synchronous; no current-state submission exists
    //yet, preserving next-tick visibility and later PreState82DB60EC consumption.
    {
        let scene =
            super::offboard::grab_scene::Scene::new(&physics.world, &physics.offboard_grab_scene);
        skater.offboard_grab.execute_queries(&scene)?;
    }
    let state_before_selection = skater.player_state.current();
    if vehicle_ejected {
        // Vehicle ejection is a host transition that must retain WipeoutGround,
        // so it bypasses the native state selector. ProcessInput has already
        // prepared this tick's grind work, however, and native PostInput is its
        // mandatory consumer. Skipping both phases leaves a stale request that
        // aborts the following tick.
        player_state::complete_post_input(physics, skater)?;
    } else {
        player_state::post_input_and_select(physics, skater)?;
    }
    let state_after_selection = skater.player_state.current();
    super::offboard_audit_trace::stage(tick, "selected", physics, skater, controls);
    #[cfg(test)]
    super::offboard_root_trace::trace(tick, "selected", skater);
    if state_before_selection != state_after_selection {
        if skater.player_input.processed.flags_2476 & (1 << 22) != 0
            || state_before_selection == skate_core::player::state::PhysicalStateId::HandPlant
            || state_after_selection == skate_core::player::state::PhysicalStateId::HandPlant
        {
            bevy::log::info!(
                "HANDPLANT_STATE tick={tick} from={state_before_selection:?} to={state_after_selection:?} flags={:08x} phase={} processed={:08x}/{:08x}/{:08x}",
                skater.handplant.flags,
                skater.handplant.phase,
                skater.player_input.processed.flags_2468,
                skater.player_input.processed.flags_2476,
                skater.player_input.processed.flags_2480
            );
        }
        physics.exchange.emit_event(
            tick,
            PhysicsEvent::StateChanged {
                from: state_before_selection,
                to: state_after_selection,
            },
        )?;
    }
    player_state::pre_state(physics, skater)?;
    match skater.player_state.current() {
        skate_core::player::state::PhysicalStateId::RevertGround => {
            super::revert_state::update(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::HandPlant => {
            super::handplant::update(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::FootPlant => {
            super::footplant::ground::update(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::Boneless => {
            super::boneless::update(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::PhysicsGround => {
            let p = &skater.player_input.processed;
            let com = p.animation_com_to_deck_752.map(f32::from_bits);
            physics.riding.update_ground_reckoning(
                &physics.board,
                super::riding_outputs::RidingPoseInputs {
                    com_to_deck: Vector3::new(com[0], com[1], com[2]),
                    body_spin: skater.animation_input.extra.physical_body_spin,
                },
                p.flags_2468,
                skater.animation_input.fields.balance,
                p.flags_2476 & 0x4000_0000 != 0,
                p,
            );
            //PhysicsGround::PreUpdate82D37C50 clears the previous push suppression
            //after Reckoning; this frame's propulsion may then set it again.
            skater.ground.state.push_suppressed_2730 = false;
            ground_phase::advance(physics, skater)?;
            super::handplant::ground_update(physics, skater)?;
            input_phase::update_ground(physics, skater)?;
        }
        skate_core::player::state::PhysicalStateId::PhysicsAir
        | skate_core::player::state::PhysicalStateId::PhysicsAirSecondary => {
            super::air_phase::advance(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::KnownAir => {
            super::known_air::update(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::BipedAir => {
            super::biped_air::update(physics, skater, controls)?
        }
        skate_core::player::state::PhysicalStateId::BipedGround
        | skate_core::player::state::PhysicalStateId::OffBoardPushing => {
            let contact = skate_core::player::offboard::ground_job::ContactSnapshot {
                readiness: skater.offboard_contact.readiness() as i32,
                prefix: skater.offboard_contact.prefix(),
            };
            let input = super::biped_ground::update(physics, skater, controls, contact)?;
            //Sync82D32164 submits AFTER Skeleton; both query batches become
            //visible only to their next input/PreUpdate consumer.
            skater.offboard_contact.submit(
                input,
                skater.player_input.processed.actor_query_2952 as i32,
                &super::offboard::contact_toolkit::StaticScene::new(&physics.world)?,
            )?;
            super::biped_ground::submit_geometry(physics, skater)?;
        }
        skate_core::player::state::PhysicalStateId::GroundAnimation => {
            super::ground_animation::advance(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::LandingOnDeck => {
            super::landing_on_deck::advance(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::SlideGround => {
            super::slide_state::update(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::WipeoutGround => {
            super::wipeout_states::advance(physics, skater)?
        }
        skate_core::player::state::PhysicalStateId::Teleporting => {
            //VT823272FC slot8=82D431F0; slots12/40 are original empty leaves.
            if skater.teleport_state.update(&skater.player_input.processed) {
                super::respawn::request(physics, skater)?;
            }
        }
        state
            if state.is_grind()
                || state == skate_core::player::state::PhysicalStateId::Nonspecific =>
        {
            super::grind::advance(physics, skater)?
        }
        state => return Err(format!("Selected {state:?} requires its physical update")),
    }
    // Source State82DB6120 submits the queue once, then advances state time.
    // BoardRuntime applies that same queue during its shared solve.
    skater.player_input.player.state_timer_1344 += skater.player_input.processed.timestep_2604;
    super::offboard_audit_trace::stage(tick, "state", physics, skater, controls);
    #[cfg(test)]
    super::offboard_root_trace::trace(tick, "state", skater);
    //82DB6150: update the real possession owner after state movement, before solve.
    super::offboard::board_manager::runtime::update(physics, skater);
    super::offboard_audit_trace::stage(tick, "possession", physics, skater, controls);
    physics.processed_flags_2468 = skater.player_input.processed.flags_2468;
    //World8275ECA4 ends skeleton tests after state/forces and before solving.
    //Teleport resets previous observations, but preserves this pending batch.
    skeleton_queries.publish(&mut skater.player_input.player);
    bevy::log::info_span!("fixed_collision_and_solve")
        .in_scope(|| solve::advance(physics, skater, skater.ground.steering.targets))?;
    super::offboard_audit_trace::stage(tick, "solve", physics, skater, controls);
    #[cfg(test)]
    super::offboard_root_trace::trace(tick, "solve", skater);
    //ProcessOutput82DB6EE8 resets the packet before its component publishers.
    //All consumers of the preceding output have completed this frame's input.
    super::player_input::reset_outputs(&mut skater.player_input.physical);
    bevy::log::info_span!("fixed_finish_skater").in_scope(|| physics.finish_skater(skater))?;
    super::offboard_audit_trace::stage(tick, "finish", physics, skater, controls);
    #[cfg(test)]
    super::offboard_root_trace::trace(tick, "finish", skater);
    let simulation = physics.settings.step.simulation;
    skater
        .player_input
        .update_dynamic_normal(&physics.riding, simulation.gravity_acceleration);
    skater.player_input.publish_board(&physics.riding)?;
    skater.player_input.physical.skeleton.publish_deck_angles(
        skater.animated_skeleton.record.pose[0][2],
        skater.animation.packet.board_flipped,
    );
    let up = physics.riding.reckoning.up;
    skater.player_input.publish_grind_graph_outputs(
        &skater.skeleton.record,
        [up.x, up.y, up.z, 0.0],
        &skater.trajectory.selector,
    )?;
    player_state::publish(physics, skater)?;
    skater
        .grind_camera
        .condition_fields(&mut skater.player_input.physical.grinds);
    let physical = &skater.player_input.physical;
    skater.centre_of_mass_output = skater.centre_of_mass_filter.update(
        physical.reckoning.vector_64.map(f32::from_bits),
        physical.reckoning.vector_16.map(f32::from_bits),
    );
    let feedback = animation_phase::publish_feedback(physics, skater);
    super::respawn::observe(physics, skater)?;
    let deck = physics.board.bodies()[BodyId::Deck.index()];
    let rider = skater
        .skeleton
        .bodies()
        .first()
        .ok_or("Physical output requires the constructed rider body")?;
    let ground_normal = physics.riding.ground.wheel_normal;
    let predicted = skater.player_input.physical.collision.predicted_position_64;
    let predicted_position = Vector3::new(
        f32::from_bits(predicted[0]),
        f32::from_bits(predicted[1]),
        f32::from_bits(predicted[2]),
    );
    let events = physics.exchange.events().to_vec();
    physics.exchange.publish_output(PhysicalOutputSnapshot {
        tick,
        state: skater.player_state.current(),
        board_position: deck.rates.position,
        board_linear_velocity: deck.rates.linear_velocity,
        rider_root_position: rider.rates.position,
        rider_linear_velocity: rider.rates.linear_velocity,
        ground_normal,
        contact_count: physics.riding.ground.part_contact_count as u32,
        predicted_position,
        grounded: matches!(
            skater.player_state.current(),
            skate_core::player::state::PhysicalStateId::PhysicsGround
        ),
        wiping_out: skater.skeleton_collision.is_ragdoll,
        landed: events
            .iter()
            .any(|event| matches!(event, PhysicsEvent::Landing)),
        footstep_strength: skater.animation_input.extra.footstep_strength,
        footstep_bone: skater.animation_input.contacts.bone,
        foot_push_speed: skater.animation_input.contacts.push_speed,
        feet_supported: skater
            .offboard_feet
            .hands
            .map(|foot| foot.flags_104_to_107[1]),
        events,
    });
    camera_output::advance(physics, skater, &feedback, camera)?;
    let score = &skater.animation.motion.score_packet;
    let deck_frame = physics.board.part_transforms()[BodyId::Deck.index()];
    let position = deck.rates.position;
    let velocity = deck.rates.linear_velocity;
    let filtered = skater.player_state.filtered_output;
    skater.scoring.advance(crate::scoring_runtime::Frame {
        tick: tick as u32,
        dt: simulation.time_step,
        category: filtered.map_or(Default::default(), |f| f.category),
        state: skater.player_state.current() as u32,
        // `ScoringHandPlants` (82BBF758) selects a handplant's name the same way
        // `ScoringGrabs` (82BBEF60) selects a grab's, directional variants and all, and
        // publishes it to `score_packet.handplant` -- where nothing read it, so a handplant
        // reached the scorer with no descriptor at all and could never be named or credited.
        descriptor: score
            .trick_names
            .first
            .or_else(|| score.grab.map(|g| g.0))
            .or_else(|| score.handplant.map(|h| h.0)),
        grind_id: filtered.map_or(-1, |f| f.grind.scorable_id),
        flags: score.flags,
        position: [position.x, position.y, position.z],
        velocity: [velocity.x, velocity.y, velocity.z],
        forward: deck_frame.basis.columns[2],
        // PhysOut_Skeleton +368, which 82BE1AE8 fills from Physics::Skeleton +11920 -- the
        // rider's animation-to-world root. Rows are right / up / forward / translation.
        // Retail's spin accumulator measures *this*, never the deck: feeding it the board
        // counted a shove-it's rotation as a body spin.
        rider_up: {
            let row = skater.player_input.physical.skeleton.anim_to_world_11920[1];
            std::array::from_fn(|lane| f32::from_bits(row[lane]))
        },
        rider_forward: {
            let row = skater.player_input.physical.skeleton.anim_to_world_11920[2];
            std::array::from_fn(|lane| f32::from_bits(row[lane]))
        },
        switch: skater.animation.packet.riding_switch,
        fakie: skater.animation.packet.riding_fakie,
        nollie: skater.animation.packet.weight_forwards,
        body_flip: skater.player_input.physical.air.flag_441 != 0,
        body_flip_side: skater.player_input.physical.air.flag_445 != 0,
        // PhysOut.Ground +192 and +208. 82DB6EC0 publishes the origin unconditionally and
        // the hit only while the test is valid, so a miss leaves the previous hit standing
        // rather than clearing it; the scorer's own guard is Ground +320.
        hips_position: {
            let origin = skater.player_input.player.hips_line_origin_1632;
            std::array::from_fn(|lane| f32::from_bits(origin[lane]))
        },
        hips_ground: {
            let hips = skater.player_input.player.hips_line_test_1488;
            (hips.valid != 0)
                .then(|| std::array::from_fn(|lane| f32::from_bits(hips.position[lane])))
        },
        hips_surface: skater.player_input.player.hips_line_test_1488.surface,
        suspend_air: skater.player_input.physical.air.use_air_reckoning_452 != 0,
        landing: skater.landing_quality,
        teleported,
        // Revert Fill (82D43B10) publishes its active lifetime in State66, which is
        // what the revert scorable is selected from.
        reverting: skater.player_input.physical.state.flag_66 != 0,
        // State70 is a different byte, and it is the one 82DAA8E0 gates the manual slot
        // on. 82DB6EC0 fills it from `[base+1888]+56`, which this host does not publish,
        // so it stays clear rather than borrowing State66's much longer lifetime.
        manual_block_70: false,
    })?;
    super::climbing::approach::advance(physics, skater, controls);
    skater.animation_input.finish_output_publication();
    skater.player_input.player.update_count_1316 =
        skater.player_input.player.update_count_1316.wrapping_add(1);
    physics.clock.finish_tick();
    Ok(())
}
