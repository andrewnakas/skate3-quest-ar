use super::*;

#[test]
#[ignore = "requires the user's converted stock skater and graph assets"]
fn stock_skater_startup_builds_the_graphs_and_physical_body_from_the_same_rig() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let manifest = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &manifest).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    // This regression isolates grounded controls on the original flat fixture.
    // The production box is only six metres wide: sustained kickturning leaves
    // it at x=-3.32, correctly selecting KnownAir and ForcePhysics2. Separate
    // air_playback tests exercise the shipped course and deliberate roll-off.
    let corners = [
        Vector3::new(-20.0, ground::HEIGHT, -20.0),
        Vector3::new(20.0, ground::HEIGHT, -20.0),
        Vector3::new(20.0, ground::HEIGHT, 20.0),
        Vector3::new(-20.0, ground::HEIGHT, 20.0),
    ];
    physics.world = BoardWorld::new(
        [[0, 2, 1], [0, 3, 2]]
            .map(|indices| {
                skate_core::physics::board_world::WorldTriangle::from_vertices(
                    indices.map(|i| corners[i]),
                    physics.settings.floor_material,
                    0,
                    skate_core::physics::collision::TriangleFeature::ONE_SIDED,
                    [1.0; 3],
                    0.0,
                )
                .unwrap()
            })
            .to_vec(),
    );
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    assert_eq!(
        skater.animation.pose.len(),
        skater.animation.evaluator.frames.bone_names.len()
    );
    assert_eq!(skater.skeleton.bodies().len(), 26);
    //InitJoints uses the raw animation frames, while physical part setup
    //orthonormalizes its frames. Coincident startup anchors are not a source
    //guarantee; validate finite geometry and the real compiled joint rows.
    let parts = skater.skeleton.part_transforms();
    let point = |part: usize, anchor: &[u32]| -> [f32; 3] {
        let p: [f32; 3] = std::array::from_fn(|i| f32::from_bits(anchor[i]));
        std::array::from_fn(|i| {
            let x = parts[part][0][i].mul_add(p[0], parts[part][3][i]);
            let y = parts[part][1][i].mul_add(p[1], x);
            parts[part][2][i].mul_add(p[2], y)
        })
    };
    for joint in &skater.skeleton_joints.records {
        let child = point(joint.child, &joint.frames.words[4..8]);
        let parent = point(joint.parent, &joint.frames.words[12..16]);
        assert!(child.into_iter().chain(parent).all(f32::is_finite));
    }
    let joints = skater.skeleton_joints.build(
        skater.skeleton.bodies(),
        8,
        physics.settings.step.simulation.time_step,
    );
    assert_eq!(joints.len(), 22);
    let drives = skater.skeleton_drives.build(
        skater.skeleton.bodies(),
        8,
        34,
        physics.settings.step.simulation.time_step,
    );
    assert_eq!(drives.rows.len(), 48);
    assert!(
        drives
            .rows
            .iter()
            .all(|drive| (8..38).contains(&drive.frame_a_body.reaction_index)
                && (8..38).contains(&drive.frame_b_body.reaction_index))
    );
    assert_eq!(
        skater.skeleton_drives.targets.transform(2)[3],
        skater.animated_skeleton.board_frames.lifted_com_frame[3]
    );
    assert_eq!(
        skater.skeleton_drives.targets.transform(3)[3],
        skater.animated_skeleton.board_frames.com_frame[3]
    );
    assert!(
        joints
            .iter()
            .all(|j| (8..34).contains(&j.reaction_a) && (8..34).contains(&j.reaction_b))
    );
    let com = skater.skeleton.record.centre_of_mass;
    assert!(com.iter().all(|v| v.is_finite()));
    let deck = physics.board.part_transforms()[BodyId::Deck.index()];
    assert!(com[1] > deck.translation.y);
    for (body, part) in skater
        .skeleton
        .bodies()
        .iter()
        .zip(skater.skeleton.part_transforms())
    {
        assert!(body.inertia.inverse_mass.is_finite() && body.inertia.inverse_mass > 0.0);
        assert!(part.into_iter().flatten().all(f32::is_finite));
    }
    assert!(
        skater
            .skeleton
            .record
            .velocities
            .iter()
            .flatten()
            .all(|v| *v == 0.0)
    );
    let volumes =
        skeleton_colliders::world_volumes(&skater.skeleton, &skater.skeleton_collision).unwrap();
    assert_eq!(volumes.len(), 23);
    let before = skater.skeleton.bodies().map(|body| body.rates.position);
    let render_before = skater.render_pose.clone();
    let mut controls = PlayerControls::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let physical_step = physics.settings.step.simulation.time_step;
    let normal_period = physics.clock.period();
    camera
        .simulation_rate_requests
        .push(skate_core::camera::SimulationRateRequest {
            timestep: 1.0 / 35.0,
            ticks: 1,
        });
    frame::advance(
        &mut physics,
        &mut skater,
        &mut controls,
        &graphs,
        &mut input.player_actions(),
        false,
        &mut camera,
    )
    .unwrap();
    assert_eq!(physics.ticks, 1);
    assert!(camera.simulation_rate_requests.is_empty());
    assert!(physics.clock.period() > normal_period);
    assert_eq!(physics.settings.step.simulation.time_step, physical_step);
    assert_eq!(skater.player_input.processed.timestep_2604, physical_step);
    //Ctor does not publish Ground before the initial input reset. The stock
    //graph enters Ground on the next tick and its Begin writes ForcePhysics1.
    assert_eq!(skater.skeleton_input.force_mode, 0);
    assert!(
        camera.frame.is_some(),
        "First frame did not publish the gameplay camera"
    );
    eprintln!(
        "First gameplay frame: deck={:?}; COM={:?}; feet={:?}; camera={:?}",
        physics.board.bodies()[6].rates.position,
        skater.skeleton.record.centre_of_mass,
        [
            skater.skeleton.record.pose[15][3],
            skater.skeleton.record.pose[19][3]
        ],
        camera.frame
    );
    assert_eq!(skater.pose_generation, physics.ticks);
    assert_ne!(render_before, skater.render_pose);
    assert!(
        skater
            .render_pose
            .iter()
            .flatten()
            .flatten()
            .all(|v| v.is_finite())
    );
    assert_eq!(skater.solved_drives.as_ref().unwrap().rows.len(), 48);
    assert_ne!(
        before,
        skater.skeleton.bodies().map(|body| body.rates.position)
    );
    // User crash: standing kickturn after spawn, left stick nearly full left,
    // no right-stick/trigger/button input. Stock Riding retains ForcePhysics1.
    let mut saw_kickturn_balance = false;
    for tick in 0..1800 {
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: 0,
            triggers: [0; 2],
            left: if (12..612).contains(&tick) {
                [-32768, 256]
            } else {
                [0; 2]
            },
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physical_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut actions,
            true,
            &mut camera,
        )
        .unwrap_or_else(|e| {
            panic!(
                "Standing kickturn tick{tick}: {e}; force_mode={}; board_axis_y={}",
                skater.skeleton_input.force_mode, skater.skeleton_input.drive_frames[0][2][1]
            )
        });
        assert_eq!(
            skater.skeleton_input.force_mode,
            1,
            "Stock Ground ForcePhysics Begin was lost at standing tick{tick}; state={:?}; deck={:?}; contacts={}",
            skater.player_state.current(),
            physics.board.bodies()[6].rates.position,
            physics.riding.ground.wheel_contact_count
        );
        saw_kickturn_balance |= skater.animation_input.fields.balance != 0.0;
    }
    assert!(
        saw_kickturn_balance,
        "Standing input never reached the stock kickturn balance"
    );
    eprintln!(
        "Standing kickturn and release completed1800ticks; mode={}; balance_seen={saw_kickturn_balance}",
        skater.skeleton_input.force_mode
    );
    // Use raw signed axes and XInput button bits through the same converter,
    // history, Pad and action map as the game. This verifies connectivity;
    // matched original-game recordings and hardware polling remain separate.
    let start = physics.board.bodies()[6].rates.position;
    let mut saw_push = false;
    let mut saw_steering = false;
    for tick in 0..180 {
        let left = match tick {
            0..20 => [if tick % 2 == 0 { 8191 } else { -8191 }, 0],
            90..120 => [32767, 0],
            120..150 => [-32768, 0],
            150..180 => [32767, 32767],
            _ => [0; 2],
        };
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if (20..130).contains(&tick) { 0x1000 } else { 0 },
            triggers: [0; 2],
            left,
            right: [0; 2],
        });
        if tick < 20 {
            assert_eq!(
                input.mapped_actions[0][..2],
                [0.0; 2],
                "Sub-dead-zone stick drift reached gameplay actions"
            );
        }
        let mut action = input.player_actions();
        controls.update(
            &mut action,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs,
            &mut action, true, &mut camera).unwrap_or_else(|e| panic!(
                "Production tick{tick}: {e}; state={:?}; flags={:08x}; contacts={}; speed={}; targets={:?}; force_mode={}; board_axis_y={}",
                skater.player_state.current(), skater.player_input.processed.flags_2468,
                physics.riding.ground.wheel_contact_count, physics.riding.motion.ground_speed,
                skater.ground.steering.targets, skater.skeleton_input.force_mode,
                skater.skeleton_input.drive_frames[0][2][1]));
        saw_push |= skater.player_input.processed.flags_2468 & (1 << 25) != 0;
        saw_steering |= skater
            .ground
            .steering
            .targets
            .iter()
            .any(|v| v.abs() > 0.001);
        if matches!(tick, 119 | 149 | 179) {
            eprintln!(
                "Raw stick={left:?}; mapped={:?}; physical Turn={}; trucks={:?}",
                &input.mapped_actions[0][..2],
                skater.animation_input.fields.turn,
                skater.ground.steering.targets
            );
            assert!(
                skater.animation_input.fields.turn.abs() > 0.001,
                "Raw stick input did not reach physical steering"
            );
        }
    }
    let end = physics.board.bodies()[6].rates.position;
    assert_eq!(physics.clock.period(), normal_period);
    eprintln!(
        "Completed riding: deck={end:?}; COM={:?}; wheel contacts={}; speed={}; truck targets={:?}; push={saw_push}; steering={saw_steering}",
        skater.skeleton.record.centre_of_mass,
        physics.riding.ground.wheel_contact_count,
        physics.riding.motion.ground_speed,
        skater.ground.steering.targets
    );
    assert!(
        saw_push,
        "Stock push animation never reached physical push input"
    );
    assert!(
        saw_steering,
        "Stock steering never reached truck drive targets"
    );
    assert!(
        (end.x - start.x).hypot(end.z - start.z) > 0.01,
        "Pushing did not move the live board"
    );
    assert!(
        physics.riding.ground.wheel_contact_count > 0,
        "Grounded riding lost every wheel contact on the flat test surface"
    );
    assert!(
        end.y > ground::HEIGHT,
        "The live deck penetrated the flat test surface"
    );
    assert!(camera.frame.is_some(), "Gameplay camera never published");
    assert_eq!(skater.pose_generation, physics.ticks);
    assert_eq!(skater.animation.ticks, physics.ticks);
    let speed_before_brake = physics.riding.motion.ground_speed.abs();
    let mut saw_brake = false;
    for tick in 180..360 {
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: 0x2000,
            triggers: [0; 2],
            left: [0; 2],
            right: [0; 2],
        });
        let mut action = input.player_actions();
        controls.update(
            &mut action,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut action,
            true,
            &mut camera,
        )
        .unwrap_or_else(|e| panic!("Braking tick{tick}: {e}"));
        // Brake82BDB0D4 writes the scalar consumed by Ground forces;
        // flags2468 bit29 is the separate ManualBrake attribute.
        saw_brake |= skater.animation_input.fields.brake > 0.0;
    }
    assert!(saw_brake, "Stock braking never reached physical input");
    assert!(
        physics.riding.motion.ground_speed.abs() < speed_before_brake,
        "Braking did not reduce the moving board's speed"
    );
    assert!(physics.riding.ground.wheel_contact_count > 0);
    eprintln!(
        "Completed braking: speed={} -> {}; deck={:?}",
        speed_before_brake,
        physics.riding.motion.ground_speed,
        physics.board.bodies()[6].rates.position
    );
}

#[test]
#[ignore = "requires private stock skater, animation banks and collections"]
fn customiser_equipment_reaches_ground_force_and_torque() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let manifest = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &manifest).unwrap();
    let mut samples = Vec::new();
    for hardness in [0.0_f32, 0.7, 1.0] {
        let mut physics = GamePhysics::load(root).unwrap();
        let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
        crate::customiser::apply_preferences(
            &serde_json::json!({"truck": hardness, "wheel": hardness}),
            &mut physics,
            &mut skater.animation,
        );
        let mut controls = PlayerControls::default();
        let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
        let input = crate::input::ControllerInput::default();
        frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut input.player_actions(),
            false,
            &mut camera,
        )
        .unwrap();

        // Replay identical observed sideways travel at the Ground input boundary.
        // Only the published profile input changes; run the actual Ground adapter.
        let p = &mut skater.player_input.processed;
        assert_eq!(p.scalar_2764, hardness);
        assert_eq!(p.truck_tightness_2760, hardness);
        p.vectors_400_416 = [[1.0_f32, 0.0, 3.0, 0.0].map(f32::to_bits); 2];
        p.scalar_2656 = 10.0_f32.sqrt();
        skater.ground.state.elapsed_2648 = 0.25;
        physics.board.forces_mut().clear();
        let outcome = ground_phase::advance(&mut physics, &mut skater).unwrap();
        assert!(matches!(
            outcome,
            skate_core::riding::grounded::state::board::GroundBoardOutcome::Ordinary(_)
        ));
        let side = physics
            .board
            .forces()
            .entries()
            .iter()
            .find(|f| f.tag == 1)
            .unwrap();
        samples.push((
            side.force_world.x,
            physics.board.bodies()[6].rates.torque_acceleration.y,
        ));
    }
    // Original82D92AD4 multiplies side friction by lerp(softest,1,hardness).
    let data = skate_data::collections::Collections::load(root).unwrap();
    let softest = data
        .float("physicswheels", "default", "SoftestWheelSideFrictionScalar")
        .unwrap();
    assert!(
        samples[2].0.abs() > 0.001,
        "Sideways travel produced no side force: {samples:?}"
    );
    for (index, hardness) in [0.0_f32, 0.7, 1.0].into_iter().enumerate() {
        let expected = (1.0 - softest).mul_add(hardness, softest);
        assert!(
            (samples[index].0 / samples[2].0 - expected).abs() < 0.0001,
            "Profile hardness was lost before live friction: {samples:?}"
        );
    }
    // Original82D92BBC also changes straighten duration, reaching deck torque.
    assert!(
        (samples[0].1 - samples[2].1).abs() > 0.001,
        "Profile hardness was lost before live straighten torque: {samples:?}"
    );
    eprintln!("Ground wheel-hardness force/torque samples: {samples:?}");
}
#[test]
#[ignore = "requires the user's converted stock collection assets"]
fn stock_geometry_loads_and_reaches_the_live_board_solver() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let mut physics = GamePhysics::load(std::path::Path::new(&root)).unwrap();
    let volumes = colliders::world_volumes(&physics.board, &physics.settings);
    assert_eq!(volumes.len(), 21); // Four wheels, two trucks, 15 stock deck children.
    for _ in 0..120 {
        physics.advance_board().unwrap();
    }
    assert!(physics.contact_count > 0);
    let reports = physics.board.contact_reports();
    assert!(
        !reports.is_empty(),
        "actual solved contacts must reach gameplay feedback"
    );
    assert!(reports.len() <= 16);
    assert!(reports.iter().any(|report| report.part.index() < 4));
    assert!(
        reports
            .iter()
            .all(|report| report.normal_force_on_a.y.is_finite())
    );
    assert_eq!(physics.ticks, 120);
    let deck = physics.board.part_transforms()[BodyId::Deck.index()];
    assert!(deck.translation.y > ground::HEIGHT);
}

#[test]
#[ignore = "requires private stock animation banks and collections; no window or renderer"]
fn customiser_styles_and_postures_change_stock_pose() {
    use skate_core::animation::playback::{
        PlayAnimation, PlayAnimationInstance, TransitionSettings,
    };
    use skate_core::animation::playback_tree::{Evaluation, PoseCommand};
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let manifest = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &manifest).unwrap();
    let mut physics = GamePhysics::load(&root).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "normal").unwrap();
    let operation = PlayAnimation {
        animation: "S_R_PUSHLSP_HSTR_N_0_CYC1".into(),
        switch_animation: None,
        mirror_animation: None,
        no_board_animation: None,
        playback_speed: 1.0,
        apply_posture: true,
        transition: TransitionSettings {
            kind: 1,
            seconds: 0.0,
            under: 0,
            matching: 0,
            use_channels_from_weights: false,
        },
        parameters: vec![],
    };
    let mut samples = Vec::new();
    for (style, posture) in [(0, 0), (1, 0), (2, 0), (3, 0), (0, 1), (0, 2), (0, 3)] {
        crate::customiser::apply_preferences(
            &serde_json::json!({"style":style, "posture":posture}),
            &mut physics,
            &mut skater.animation,
        );
        let motion = &mut skater.animation.motion;
        PlayAnimationInstance::default()
            .begin(
                &operation,
                &mut motion.playback_context,
                &mut motion.animation,
            )
            .unwrap();
        motion.animation.apply_parameters().unwrap();
        motion.animation.seek_current_fraction(0.35);
        let commands = motion
            .animation
            .evaluate_pose(Evaluation {
                cull_threshold: 0.0001,
                update_history: false,
            })
            .unwrap();
        let pose = skater.animation.evaluator.evaluate(&commands).unwrap();
        let names: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                PoseCommand::Clip { name, .. } | PoseCommand::Pose { name } => Some(name.clone()),
                _ => None,
            })
            .collect();
        eprintln!("Style {style}, posture {posture}: {names:?}");
        if let Some(baseline) = samples.first() {
            assert!(
                &pose != baseline,
                "Style {style}, posture {posture} did not change the evaluated stock pose"
            );
        }
        samples.push(pose);
    }
    skater.animation.set_customisation(1, 0);
    let stance = skater.animation.stance();
    skater.animation.set_customisation(0, 0);
    assert_ne!(skater.animation.stance(), stance);
    skater.animation.set_customisation(1, 0);
    assert_eq!(skater.animation.stance(), stance);
}

#[test]
#[ignore = "requires private stock assets"]
fn raw_y_reaches_stock_dismount() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut saw_toggle = false;
    let mut first_offboard_tick = None;
    let mut previous_state = skater.player_state.current();
    let mut walk_start = None;
    let mut walk_end = None;
    let mut camera_start = None;
    let mut camera_end = None;
    let mut held_board_seen = false;
    let mut released_board_seen = false;
    for tick in 0..2400 {
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if tick == 20 { 0x8000 } else { 0 },
            triggers: if (400..460).contains(&tick) {
                [0, 255]
            } else {
                [0; 2]
            },
            left: match tick {
                120..300 => [0, 24000],
                800..920 => [24000, 0],
                920..1040 => [-24000, 0],
                _ => [0; 2],
            },
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls
            .update_for_physics(&mut actions, &physics, &skater, &camera)
            .unwrap();
        let published_slope = skater.player_input.physical.off_board.kind_88;
        let published_thin = skater.player_input.physical.off_board.flag_330 != 0;
        let result = frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut actions,
            true,
            &mut camera,
        );
        saw_toggle |= skater
            .animation
            .motion
            .action_intents
            .get("NewToggleOffBoardState")
            .is_some();
        result.unwrap_or_else(|error| {
            panic!(
                "Y tick{tick}, input_received={saw_toggle}, state={:?}: {error}",
                skater.player_state.current()
            )
        });
        assert_eq!(
            skater.animation.motion.ground_slope_type,
            Some(published_slope),
            "GroundSlopeType did not read completed OffBoard88 at tick{tick}"
        );
        assert_eq!(
            skater.animation.motion.biped_ground_thin,
            Some(published_thin),
            "IsBipedGroundThin did not read completed OffBoard330 at tick{tick}"
        );
        let root_position = skater.animated_skeleton.roots.animation_to_world[3];
        held_board_seen |= skater.skateboard_controller.fields.state_448 == 1;
        if held_board_seen && skater.skateboard_controller.fields.state_448 == 2 {
            released_board_seen = true;
            //LetGo82D75440 clears both deck-drive channels and the selected
            //hand drive before entering released state2. Test the live owners.
            assert_eq!(
                physics.board.hook().drive.dynamics,
                [0, 0, 0, 2, 0, 0, 0, 2],
                "Deck remains driven after release tick{tick}"
            );
            assert_eq!(
                skater.board_possession.state.selected_hand_424, 2,
                "Released board retains a hand owner at tick{tick}"
            );
            for hand in &skater.board_possession.state.hands {
                assert_eq!(
                    hand.dynamics,
                    [[0, 0, 0, 2]; 2],
                    "Hand drive remains armed after release tick{tick}"
                );
            }
        }
        let camera_position = camera
            .frame
            .as_ref()
            .expect("Missing gameplay camera")
            .position;
        assert!(
            camera_position.iter().all(|v| v.is_finite()),
            "Camera tick{tick}"
        );
        assert!(
            skater
                .render_pose
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite()),
            "Non-finite rendered skeleton at tick{tick}"
        );
        assert_eq!(
            skater
                .player_input
                .physical
                .reckoning
                .vector_64
                .map(f32::from_bits),
            skater.animated_skeleton.board_frames.centre_of_mass,
            "COM output was overwritten after skeleton publication at tick{tick}"
        );
        if tick == 120 {
            walk_start = Some(root_position);
            camera_start = Some(camera_position);
        }
        if tick == 299 {
            walk_end = Some(root_position);
            camera_end = Some(camera_position);
        }
        if tick == 240 {
            let attrs = skater.animation_input.extra;
            assert!(
                attrs.offboard_magnitude > 0.1,
                "Stock AG/MG did not produce offboard magnitude: {attrs:?}"
            );
            assert!(
                attrs.biped_world_x.abs() + attrs.biped_world_z.abs() > 0.1,
                "Stock AG/MG did not publish Biped world direction: {attrs:?}"
            );
        }
        if (120..2400).contains(&tick) {
            assert_eq!(
                skater.player_state.current(),
                skate_core::player::state::PhysicalStateId::BipedGround,
                "Ordinary walking/trigger input left BipedGround at tick{tick}"
            );
            // Measure the actual shared-solver joint anchors, not a synthetic
            // stand pose. A 35cm separation is visible limb failure, not
            // normal constraint compliance. This is an acceptance threshold.
            let bodies = skater.skeleton.bodies();
            let anchor = |part: usize, words: &[u32]| -> [f32; 3] {
                let body = &bodies[part];
                let local: [f32; 3] = std::array::from_fn(|i| f32::from_bits(words[i]));
                let basis = body.rates.basis.columns;
                let position = body.rates.position;
                let position = [position.x, position.y, position.z];
                std::array::from_fn(|i| {
                    position[i]
                        + basis[0][i] * local[0]
                        + basis[1][i] * local[1]
                        + basis[2][i] * local[2]
                })
            };
            for joint in &skater.skeleton_joints.records {
                let child = anchor(joint.child, &joint.frames.words[4..7]);
                let parent = anchor(joint.parent, &joint.frames.words[12..15]);
                let gap = (0..3)
                    .map(|i| (child[i] - parent[i]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                if gap >= 0.35 {
                    let collision = crate::physics::input_phase::collision(&skater);
                    eprintln!(
                        "Joint failure drive input: partial={}, weight={}",
                        collision.partial_ragdoll, collision.drive_weight_4028
                    );
                    for part in [15usize, 16] {
                        if let Some(bone) = &skater.skeleton_drives.bones[part] {
                            eprintln!(
                                "Joint failure bone{part}: parent={:?} active={:?} dynamics={:?} frames={:?}",
                                bone.parent, bone.active, bone.dynamics, bone.frames
                            );
                        }
                    }
                    eprintln!(
                        "Pose boundary errors: {:?}",
                        crate::physics::offboard::skeleton_ground::audit_render_parts(&skater)
                    );
                }
                assert!(
                    gap < 0.35,
                    "Broken joint at tick{tick}: {} -> {}, gap={gap}; board_state={:?}; hands={:?}; ground_result={:?}; errors={:?}; com_target={:?}; flags={:08x}/{:08x}/{:08x}",
                    joint.parent,
                    joint.child,
                    skater.skateboard_controller.fields,
                    skater.board_possession.state.hands,
                    skater.biped_ground.result,
                    skater.collision_extra_errors,
                    skater.animated_skeleton.board_frames.com_frame,
                    skater.player_input.processed.flags_2476,
                    skater.player_input.processed.flags_2480,
                    skater.player_input.processed.flags_2484
                );
            }
        }
        if skater.player_state.current() != previous_state || tick % 120 == 0 {
            for bone in skater.skeleton_output.pose.board_bones.all() {
                eprintln!(
                    "board bone tick{tick} {} parent={} animation_local={:?} rendered={:?}",
                    skater.animation.evaluator.frames.bone_names[bone],
                    skater.animation.evaluator.frames.parents[bone],
                    skate_core::animation::output::sqt_to_matrix(skater.animation.pose[bone])[3],
                    skater.render_pose[bone][3]
                );
            }
            eprintln!(
                "dismount tick={tick} state={:?} root={:?} com={:?} camera={:?}",
                skater.player_state.current(),
                skater.animated_skeleton.roots.animation_to_world[3],
                skater
                    .player_input
                    .physical
                    .reckoning
                    .vector_64
                    .map(f32::from_bits),
                camera.frame.as_ref().map(|f| f.position)
            );
            previous_state = skater.player_state.current();
        }
        if skater.player_state.current().category() == 500 {
            assert!(saw_toggle);
            first_offboard_tick.get_or_insert(tick);
        }
    }
    assert!(
        first_offboard_tick.is_some(),
        "Y did not reach off-board physics; input_received={saw_toggle}"
    );
    assert!(
        held_board_seen && released_board_seen,
        "Replay did not exercise held-to-released board possession"
    );
    let distance =
        |a: [f32; 4], b: [f32; 4]| ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    // Acceptance bounds for this recorded three-second input, not gameplay clamps.
    let walked = distance(walk_start.unwrap(), walk_end.unwrap());
    assert!(
        walked > 1.0 && walked < 20.0,
        "Implausible walking displacement: {walked}"
    );
    let followed = distance(camera_start.unwrap(), camera_end.unwrap());
    assert!(
        followed > 1.0 && followed < 20.0,
        "Camera did not follow walking: {followed}"
    );
}
