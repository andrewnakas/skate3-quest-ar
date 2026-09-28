//! Raw X -> stock OB_Jump -> Ground launch -> real Air/landing regression.
//! Source: input8259C260, attribute82BDBA88, Ground82D32338/82D32D38,
//! launch82D7BA78. Acceptance bounds are not native tuning/parity fixtures.
use super::*;
use skate_core::{input::xbox::XboxState, player::state::PhysicalStateId};

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_x_offboard_jump_connects_input_ground_launch_air_and_landing() {
    jump_playback(false);
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_x_offboard_jump_with_thrown_board_connects_ground_air_and_landing() {
    jump_playback(true);
}

fn jump_playback(throw_board: bool) {
    jump_playback_with_tweak(throw_board, false);
}

#[test]
#[ignore = "requires private stock assets; connected raw right-stick gameplay regression"]
fn raw_right_stick_four_directions_reach_air_tweak_with_held_board() {
    jump_playback_with_tweak(false, true);
}

#[test]
#[ignore = "requires private stock assets; connected raw right-stick gameplay regression"]
fn raw_right_stick_four_directions_reach_air_tweak_with_released_board() {
    jump_playback_with_tweak(true, true);
}

fn jump_playback_with_tweak(throw_board: bool, tweak: bool) {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut jump_intents = 0;
    let mut launch_tick = None;
    let mut air_tick = None;
    let mut landing_tick = None;
    let mut air_frames = 0;
    let mut landed_frames = 0;
    let mut launch_height = None;
    let mut maximum_height = f32::NEG_INFINITY;
    let mut saw_upward_physical_velocity = false;
    let mut saw_completed_selector = false;
    let mut saw_stock_end_com_adjustment = false;
    let mut tweak_directions = [false; 4];
    for tick in 0..1200 {
        let before_state = skater.player_state.current();
        if tick == 400 {
            assert_eq!(
                before_state,
                PhysicalStateId::BipedGround,
                "Raw Y did not settle into Ground before the jump"
            );
            assert_ne!(
                skater.biped_ground.contact.flags_176 & 1,
                0,
                "Jump fixture has no actual analyzer support"
            );
            assert!(skater.skateboard_controller.fields.system_on_452);
            if throw_board {
                assert_released(&physics, &skater, tick);
            } else {
                assert_eq!(
                    skater.skateboard_controller.fields.state_448, 1,
                    "Held variant must begin carrying the board through the real owner"
                );
            }
            launch_height = Some(skater.skeleton.record.centre_of_mass[1]);
        }
        //Stock input.cfg: GP_XFace=X=Button14, native action78 -> bit23.
        //Hold X for sixty frames:8259C260 emits only on the rising edge.
        // Release before native82BBB030's <0.1s landing boundary. Holding a
        // tweak into that boundary issues a separate native bit16 request.
        let tweak_index = if tweak && (410..442).contains(&tick) {
            Some((tick - 410) / 8)
        } else {
            None
        };
        let right = tweak_index
            .map(|i| [[24000, 0], [-24000, 0], [0, 24000], [0, -24000]][i])
            .unwrap_or([0; 2]);
        input.sample_raw_for_test(XboxState {
            buttons: if tick == 20 {
                0x8000
            } else if (400..460).contains(&tick) {
                0x4000
            } else {
                0
            },
            //8259C260 publishes throw/retrieve on a full RT rising edge. The
            //stock graph and actual possession owner decide the release.
            triggers: if throw_board && (200..260).contains(&tick) {
                [0, 255]
            } else {
                [0; 2]
            },
            left: [0; 2],
            right,
        });
        let mut actions = input.player_actions();
        controls
            .update_for_physics(&mut actions, &physics, &skater, &camera)
            .unwrap();
        frame::advance(&mut physics, &mut skater, &mut controls,
            &graphs, &mut actions, true, &mut camera)
            .unwrap_or_else(|e| {
                let p = &skater.player_input.processed;
                let controller = &skater.biped_ground.controller.state;
                panic!("Offboard jump tick{tick}, thrown={throw_board}, prior={before_state:?}: {e}; \
                    states={}/{} categories={}/{} flags={:08x}/{:08x}/{:08x}/{:08x}; \
                    controller_up={:?} controller_motion_up={:?} velocity608={:?} velocity912={:?} \
                    steering={} speed704={} edge={:?} raw_xz={:?}; \
                    processed_up544={:?} processed_forward224={:?} ground_result={:?} localCOM10960={:?}",
                    p.state_2504, p.state_2508, p.category_2516, p.category_2512,
                    p.flags_2472, p.flags_2476, p.flags_2480, p.flags_2484,
                    controller.frame_output.frame[1], controller.motion.frame_0[1],
                    p.vectors_544_560_592_608[3].map(f32::from_bits),
                    p.vectors_880_896_912_928_944[2].map(f32::from_bits),
                    controller.intent.steering, controller.motion.speed_704, controller.intent.edge_target,
                    [skater.animation_input.extra.biped_world_x, skater.animation_input.extra.biped_world_z],
                    p.vectors_544_560_592_608[0].map(f32::from_bits),
                    p.effective_anim_transform_192[2].map(f32::from_bits),
                    skater.biped_ground.result, skater.animated_skeleton.record.centre_of_mass);
            });
        let state = skater.player_state.current();
        let p = &skater.player_input.processed;
        if skater
            .animation
            .motion
            .action_intents
            .contains_key("OB_Jump")
        {
            jump_intents += 1;
            assert_eq!(tick, 400, "Jump intent was not the raw X rising edge");
        }
        if tick >= 400 {
            if throw_board {
                assert_released(&physics, &skater, tick);
            }
            let com = skater.skeleton.record.centre_of_mass;
            let velocity = skater.skeleton.record.centre_of_mass_velocity;
            maximum_height = maximum_height.max(com[1]);
            saw_upward_physical_velocity |= velocity[1] > 0.1;
            assert!(
                velocity.iter().all(|v| v.is_finite()),
                "Nonfinite jump velocity tick{tick}"
            );
            assert_joints(&skater, tick);
            if state == PhysicalStateId::BipedGround
                && p.flags_2476 & 0x80000 != 0
                && skater.biped_ground.ground.flags_144_to_150[2]
            {
                launch_tick.get_or_insert(tick);
                assert_ne!(
                    skater.biped_ground.contact.flags_176 & 1,
                    0,
                    "Expected an explicit supported jump, not falling off support"
                );
                let output = &skater.player_input.physical.off_board;
                assert_eq!(output.flag_328, 1, "Ground Fill dropped the launch flag");
                assert_eq!(
                    output.scalar_32, 1.,
                    "Ground Fill dropped the selector scalar"
                );
                let selector = &skater.offboard_air_selector.core;
                assert!(
                    selector.sampling.pending_8492,
                    "Ground did not submit its real Air query"
                );
                assert_eq!(selector.launch.kind_108, 6);
                assert_eq!(
                    selector.launch.kind_112, 3,
                    "Launch was not native jump mode4"
                );
                assert!(
                    selector.launch.velocity_0[1] > 0.,
                    "Jump packet has no upward velocity"
                );
            }
            if state == PhysicalStateId::BipedAir {
                let end_com = skater.animation_input.fields.animation_end_com;
                let translation = skater.animation_input.fields.animation_translation;
                if end_com[1] > 0.1
                    && translation[1] <= 0.1
                    && skater.biped_air.state.flags_544_550[0]
                {
                    //Native82D2EFF0/F014 reads0xB40, not0xB30. The stock
                    //jump's distinct attributes make a swapped callsite fail.
                    assert!(
                        skater.biped_air.state.flags_544_550[1],
                        "Air did not consume stock AnimEndCOM at tick{tick}"
                    );
                    saw_stock_end_com_adjustment = true;
                }
                if let Some(index) = tweak_index {
                    let animation = &skater.animation.motion.animation;
                    if animation.channels.has("AirBodyTweak") {
                        assert!(animation.motion_intents.contains_key("OB_AirBodyTweakX"));
                        assert!(animation.motion_intents.contains_key("OB_AirBodyTweakY"));
                        tweak_directions[index] = true;
                    }
                }
                assert!(
                    launch_tick.is_some(),
                    "Air preceded the completed Ground jump packet"
                );
                assert!(
                    landing_tick.is_none(),
                    "A held X/republication caused another Air entry"
                );
                air_tick.get_or_insert(tick);
                air_frames += 1;
                let selector = &skater.offboard_air_selector.core;
                saw_completed_selector |=
                    !selector.sampling.pending_8492 && selector.selected_index.is_some();
                assert_eq!(
                    skater.player_input.physical.off_board.trajectory_valid_331, 1,
                    "Air Fill has no real trajectory tick{tick}"
                );
            }
            if air_tick.is_some() && state == PhysicalStateId::BipedGround {
                landing_tick.get_or_insert(tick);
                landed_frames += 1;
            }
            if state != before_state || launch_tick == Some(tick) {
                eprintln!(
                    "Jump tick{tick}: {before_state:?}->{state:?} COM={com:?} velocity={velocity:?} flags={:08x}/{:08x}/{:08x} support={:x} remaining={} launch={launch_tick:?}",
                    p.flags_2476,
                    p.flags_2480,
                    p.flags_2484,
                    skater.biped_ground.contact.flags_176,
                    skater.biped_air.state.time_remaining_444
                );
            }
        }
        assert!(
            skater
                .render_pose
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite()),
            "Nonfinite jump pose tick{tick}"
        );
        assert!(
            camera
                .frame
                .as_ref()
                .expect("completed camera")
                .position
                .iter()
                .all(|v| v.is_finite()),
            "Nonfinite jump camera tick{tick}"
        );
        if landed_frames >= 120 {
            break;
        }
    }
    assert_eq!(
        jump_intents, 1,
        "A single raw X edge must produce one AG jump intent"
    );
    if tweak {
        assert!(
            tweak_directions.into_iter().all(|seen| seen),
            "Raw right-stick directions did not all reach the live AirBodyTweak channel: {tweak_directions:?}"
        );
    }
    assert!(
        launch_tick.is_some(),
        "Stock jump animation never produced a Ground launch"
    );
    assert!(
        air_frames >= 8,
        "Jump did not sustain an airborne interval: {air_frames}"
    );
    assert!(saw_completed_selector, "Real jump query was never consumed");
    assert!(
        saw_stock_end_com_adjustment,
        "Jump did not exercise the distinct stock AnimEndCOM/AnimTrans inputs"
    );
    assert!(
        saw_upward_physical_velocity,
        "Jump did not raise the physical skeleton"
    );
    assert!(
        maximum_height > launch_height.expect("jump attempted") + 0.1,
        "Physical COM never rose above the supported launch height"
    );
    assert!(
        landing_tick.is_some() && landed_frames >= 120,
        "Jump did not return to sustained Ground through the native selector"
    );
}

fn assert_released(physics: &GamePhysics, skater: &SkaterRuntime, tick: usize) {
    assert!(
        matches!(skater.skateboard_controller.fields.state_448, 2 | 3),
        "Raw RT did not leave the board free at tick{tick}: {:?}",
        skater.skateboard_controller.fields
    );
    assert_eq!(
        skater.board_possession.state.selected_hand_424, 2,
        "Free-board jump retained a selected hand at tick{tick}"
    );
    assert_eq!(
        physics.board.hook().drive.dynamics,
        [0, 0, 0, 2, 0, 0, 0, 2],
        "Free-board jump rearmed the deck drive at tick{tick}"
    );
    for hand in &skater.board_possession.state.hands {
        assert_eq!(
            hand.dynamics,
            [[0, 0, 0, 2]; 2],
            "Free-board jump rearmed a hand drive at tick{tick}"
        );
    }
}

fn assert_joints(skater: &SkaterRuntime, tick: usize) {
    let bodies = skater.skeleton.bodies();
    let anchor = |part: usize, words: &[u32]| -> [f32; 3] {
        let body = &bodies[part];
        let local: [f32; 3] = std::array::from_fn(|i| f32::from_bits(words[i]));
        let basis = body.rates.basis.columns;
        let p = body.rates.position;
        let position = [p.x, p.y, p.z];
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
        //Visible-failure acceptance bound shared with the dismount regression.
        assert!(
            gap < 0.35,
            "Jump joint failure tick{tick}: {}->{} gap={gap}",
            joint.parent,
            joint.child
        );
    }
}
