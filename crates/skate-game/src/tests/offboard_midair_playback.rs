//! Raw-input reproduction of dismounting while falling; no forced physical state.
use super::*;
use skate_core::player::state::PhysicalStateId;

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_y_while_falling_preserves_biped_air_joint_and_velocity_continuity() {
    for release_board in [false, true] {
        falling_dismount(release_board, if release_board { 20.0 } else { 8.0 }, false);
    }
}

#[test]
#[ignore = "requires private stock assets; complete midair dismount/landing regression"]
fn raw_midair_dismount_at_two_heights_runs_through_sustained_landing() {
    for drop in [2.0, 6.0] {
        for released in [false, true] {
            falling_dismount(released, drop, true);
        }
    }
}

fn falling_dismount(release_board: bool, drop: f32, complete_landing: bool) {
    eprintln!(
        "Midair scenario release_board={release_board} drop={drop} complete_landing={complete_landing}"
    );
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    // A real authored drop gives the stock dismount animation time to run.
    // These are fixture dimensions, not gameplay constants or fake contacts.
    let floor_y = ground::HEIGHT - drop;
    physics.world =
        super::offboard_air_tests::edge_world(physics.settings.floor_material, 2.0, floor_y);
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut y_tick = None;
    let mut air_tick = None;
    let mut air_frames = 0;
    let mut continuity_frames = 0;
    let mut saw_selector_completion = false;
    let mut saw_held_board = false;
    let mut release_tick = None;
    let mut released_air_frames = 0;
    let mut landed_frames = 0;
    let mut previous_state = skater.player_state.current();
    for tick in 0..1200 {
        let before_velocity = skater.skeleton.record.centre_of_mass_velocity;
        let before_position = skater.skeleton.record.centre_of_mass;
        let before_state = skater.player_state.current();
        let before_board_state = skater.skateboard_controller.fields.state_448;
        let falling = before_state.category() == 200 && before_velocity[1] < 0.0;
        let press_y = y_tick.is_none() && falling;
        let buttons = if press_y {
            y_tick = Some(tick);
            0x8000
        } else if y_tick.is_none() && tick >= 12 && before_state.category() != 200 {
            0x1000
        } else {
            0
        };
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons,
            //Use the same raw RT input as the verified held->released ground
            //replay, after actual Air entry. The stock graph owns its timing.
            triggers: if release_board && air_frames > 0 {
                [0, 255]
            } else {
                [0; 2]
            },
            left: [0; 2],
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls
            .update_for_physics(&mut actions, &physics, &skater, &camera)
            .unwrap();
        frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut actions,
            true,
            &mut camera,
        )
        .unwrap_or_else(|error| {
            panic!("Midair dismount tick{tick}, Y={y_tick:?}, prior={before_state:?}: {error}")
        });
        let state = skater.player_state.current();
        if state != previous_state
            || press_y
            || before_board_state != skater.skateboard_controller.fields.state_448
        {
            trace_air(&skater, tick);
            eprintln!(
                "Midair tick{tick}: {previous_state:?}->{state:?}, Y={y_tick:?}; COM={:?};                  velocity={:?}; root_velocity={:?}; board={:?}; hand={}; selector={:?}",
                skater.skeleton.record.centre_of_mass,
                skater.skeleton.record.centre_of_mass_velocity,
                skater.skeleton_input.root_velocity,
                skater.skateboard_controller.fields,
                skater.board_possession.state.selected_hand_424,
                skater.offboard_air_selector.core.selected_index,
            );
            previous_state = state;
        }
        if state == PhysicalStateId::BipedAir {
            assert!(
                y_tick.is_some(),
                "BipedAir preceded requested dismount at tick{tick}"
            );
            if air_tick.is_none() {
                air_tick = Some(tick);
            }
            air_frames += 1;
            let board_state = skater.skateboard_controller.fields.state_448;
            saw_held_board |= board_state == 1;
            if release_board && board_state == 2 {
                assert!(
                    saw_held_board,
                    "Released scenario never held the board in Air"
                );
                release_tick.get_or_insert(tick);
                released_air_frames += 1;
            }
            let selector = &skater.offboard_air_selector.core;
            saw_selector_completion |=
                !selector.sampling.pending_8492 && selector.selected_index.is_some();
            assert!(
                skater.skateboard_controller.fields.system_on_452,
                "Air entry left the canonical possession controller off at tick{tick}"
            );
        }
        if y_tick.is_some() {
            //Air548 remains set until its native landing condition. Ground
            //selection while it is set exposes the wrong-width flag read.
            if before_state == PhysicalStateId::BipedAir
                && state == PhysicalStateId::BipedGround
                && skater.biped_air.state.flags_544_550[4]
            {
                panic!("Air selected Ground with retained Air328=true at tick{tick}");
            }
            assert_physical_joints(&skater, tick);
            let velocity = skater.skeleton.record.centre_of_mass_velocity;
            assert!(
                velocity.iter().all(|v| v.is_finite()),
                "Nonfinite actual COM velocity at tick{tick}: {velocity:?}"
            );
            assert!(
                skater
                    .skeleton_input
                    .root_velocity
                    .iter()
                    .all(|v| v.is_finite()),
                "Nonfinite drive-root velocity at tick{tick}"
            );
            assert!(
                skater
                    .render_pose
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|v| v.is_finite()),
                "Nonfinite rendered pose at tick{tick}"
            );
            assert!(
                camera
                    .frame
                    .as_ref()
                    .expect("completed camera")
                    .position
                    .iter()
                    .all(|v| v.is_finite()),
                "Nonfinite gameplay camera at tick{tick}"
            );
            // Acceptance bounds, not original force/solver constants. Apply only
            // during the first four Air/release ticks, away from the floor;
            // contact impulses on later landing are not a continuity failure.
            if state == PhysicalStateId::BipedAir
                && (air_frames <= 4 || release_tick.is_some_and(|start| tick < start + 4))
                && before_position[1] > floor_y + 2.0
            {
                continuity_frames += 1;
                let delta_velocity = distance(velocity, before_velocity);
                let displacement = distance(skater.skeleton.record.centre_of_mass, before_position);
                let travel =
                    magnitude(before_velocity) * physics.settings.step.simulation.time_step;
                assert!(
                    delta_velocity < 10.0,
                    "Midair COM velocity discontinuity tick{tick}: delta={delta_velocity},                      before={before_velocity:?}, after={velocity:?}"
                );
                assert!(
                    displacement < travel + 0.5,
                    "Midair COM position discontinuity tick{tick}: step={displacement},                      previous-speed travel={travel}"
                );
            }
            if skater.skateboard_controller.fields.state_448 == 2 {
                assert_eq!(
                    physics.board.hook().drive.dynamics,
                    [0, 0, 0, 2, 0, 0, 0, 2],
                    "Released deck still driven tick{tick}"
                );
                assert_eq!(
                    skater.board_possession.state.selected_hand_424, 2,
                    "Released board still has a hand owner tick{tick}"
                );
                for hand in &skater.board_possession.state.hands {
                    assert_eq!(
                        hand.dynamics,
                        [[0, 0, 0, 2]; 2],
                        "Released hand drive still armed tick{tick}"
                    );
                }
            }
        }
        // Continue beyond entry and query completion, but do not turn this
        // isolated reproduction into a wipeout/recovery feature acceptance.
        if air_tick.is_some() && state == PhysicalStateId::BipedGround {
            landed_frames += 1;
        }
        if if complete_landing {
            landed_frames >= 120
        } else {
            (!release_board && air_frames >= 12) || (release_board && released_air_frames >= 24)
        } {
            break;
        }
    }
    if complete_landing {
        assert!(
            landed_frames >= 120,
            "Dismount drop={drop}, released={release_board} did not sustain landing: {landed_frames}; state={:?}",
            skater.player_state.current()
        );
    }
    assert!(
        y_tick.is_some(),
        "Raw push never produced observed falling onboard Air"
    );
    assert!(air_tick.is_some(), "Falling raw Y never entered BipedAir");
    assert!(
        air_frames >= 12,
        "BipedAir did not survive twelve update frames"
    );
    assert!(
        continuity_frames >= 1,
        "No airborne velocity-continuity interval observed"
    );
    assert!(
        saw_selector_completion,
        "No real selector candidate completed while BipedAir was observed"
    );
    if release_board {
        assert!(
            release_tick.is_some(),
            "Raw RT never released the held board in Air"
        );
        //The original20m continuity fixture must still cover24 release ticks.
        //A2m drop lands sooner; the new case instead requires120 landed ticks
        //above, plus proof the raw release actually occurred while airborne.
        let required_release_frames = if complete_landing { 1 } else { 24 };
        assert!(
            released_air_frames >= required_release_frames,
            "Did not observe{required_release_frames} released-board Air ticks: {released_air_frames}"
        );
    }
}

fn trace_air(skater: &SkaterRuntime, tick: usize) {
    let p = &skater.player_input.processed;
    let air = &skater.biped_air.state;
    let selector = &skater.offboard_air_selector.core;
    let selected = &selector.sampling.selection;
    eprintln!(
        "Air trace tick{tick}: states={}/{} categories={}/{} timer={} \
         flags2472/76/80/84/88={:08x}/{:08x}/{:08x}/{:08x}/{:08x}; \
         Airframe={} remaining={} duration={} valid={} contact={:?} normal={:?} \
         flags544..550={:?} output320={}; pending8492={} ready8494={} requery8499={} \
         selection={:?} present={} valid6200={} landingframe={} sampleframe={} \
         scalar8392={} query={:?}",
        p.state_2504,
        p.state_2508,
        p.category_2516,
        p.category_2512,
        p.state_timer_2664,
        p.flags_2472,
        p.flags_2476,
        p.flags_2480,
        p.flags_2484,
        p.flags_2488,
        air.frame_452,
        air.time_remaining_444,
        air.duration_448,
        air.result.valid_404,
        air.result.contact_position_336,
        air.result.normal_304,
        air.flags_544_550,
        u8::from(air.output().flag_320),
        selector.sampling.pending_8492,
        selector.sampling.preinitialized_8494,
        selector.requery_pending_8499,
        selector.selected_index,
        selected.result_present_3888,
        selected.valid_6200,
        selected.landing_frame_8480,
        selector.sampling.frame_8484,
        selected.scalar_8392,
        selector
            .selected_index
            .and_then(|i| selector.predictions.get(i))
            .map(|p| p.result),
    );
}

fn magnitude(v: [f32; 4]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn distance(a: [f32; 4], b: [f32; 4]) -> f32 {
    magnitude(std::array::from_fn(|i| a[i] - b[i]))
}
fn assert_physical_joints(skater: &SkaterRuntime, tick: usize) {
    let bodies = skater.skeleton.bodies();
    let anchor = |part: usize, words: &[u32]| -> [f32; 3] {
        let body = &bodies[part];
        let local: [f32; 3] = std::array::from_fn(|i| f32::from_bits(words[i]));
        let basis = body.rates.basis.columns;
        let position = body.rates.position;
        let position = [position.x, position.y, position.z];
        std::array::from_fn(|i| {
            position[i] + basis[0][i] * local[0] + basis[1][i] * local[1] + basis[2][i] * local[2]
        })
    };
    for joint in &skater.skeleton_joints.records {
        let child = anchor(joint.child, &joint.frames.words[4..7]);
        let parent = anchor(joint.parent, &joint.frames.words[12..15]);
        let gap = (0..3)
            .map(|i| (child[i] - parent[i]).powi(2))
            .sum::<f32>()
            .sqrt();
        // Same visible-failure acceptance bound as the real ground replay.
        assert!(
            gap < 0.35,
            "Midair joint failure tick{tick}: {} -> {} gap={gap}; board={:?};              hand={}; root_velocity={:?}; flags={:08x}/{:08x}/{:08x}",
            joint.parent,
            joint.child,
            skater.skateboard_controller.fields,
            skater.board_possession.state.selected_hand_424,
            skater.skeleton_input.root_velocity,
            skater.player_input.processed.flags_2476,
            skater.player_input.processed.flags_2480,
            skater.player_input.processed.flags_2484,
        );
    }
}
