//! Real raw controller -> stock graphs -> possession -> shared solver checks.
//! Acceptance regressions, not original-game or Xenon parity fixtures.
use super::*;
use skate_core::input::xbox::XboxState;
use skate_core::player::state::PhysicalStateId;

struct Replay {
    physics: GamePhysics,
    skater: SkaterRuntime,
    controls: PlayerControls,
    input: crate::input::ControllerInput,
    camera: crate::camera::CameraRuntime,
    graphs: crate::graph_runtime::StockGraphs,
}
impl Replay {
    fn load() -> Self {
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
        let root = std::path::Path::new(&root);
        let assets = skate_data::GameAssets::load(root).unwrap();
        let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
        let physics = GamePhysics::load(root).unwrap();
        let skater =
            SkaterRuntime::load(root, &graphs, &physics, crate::physics::PHYSICS_MODE).unwrap();
        let camera = crate::camera::CameraRuntime::load(root).unwrap();
        Self {
            physics,
            skater,
            camera,
            graphs,
            controls: PlayerControls::default(),
            input: crate::input::ControllerInput::default(),
        }
    }
    fn tick(&mut self, tick: usize, buttons: u16, triggers: [u8; 2]) {
        self.tick_raw(
            tick,
            XboxState {
                buttons,
                triggers,
                left: [0; 2],
                right: [0; 2],
            },
        );
    }
    fn tick_raw(&mut self, tick: usize, raw: XboxState) {
        let previous = (
            self.skater.player_state.current(),
            self.skater.skateboard_controller.fields.state_448,
        );
        self.input.sample_raw_for_test(raw);
        let mut actions = self.input.player_actions();
        self.controls
            .update_for_physics(&mut actions, &self.physics, &self.skater, &self.camera)
            .unwrap();
        frame::advance(
            &mut self.physics,
            &mut self.skater,
            &mut self.controls,
            &self.graphs,
            &mut actions,
            true,
            &mut self.camera,
        )
        .unwrap_or_else(|e| panic!("Recall/remount tick{tick}: {e}"));
        let current = (
            self.skater.player_state.current(),
            self.skater.skateboard_controller.fields.state_448,
        );
        if previous != current || tick % 240 == 0 || (780..900).contains(&tick) && tick % 10 == 0 {
            let channel = &self.skater.animation.motion.animation.channels;
            eprintln!(
                "board replay tick{tick}: {previous:?}->{current:?} flags={:08x}/{:08x} progress={} physical={:?} recallAG={} mountMG={} channel={}/{}/{}",
                self.skater.player_input.processed.flags_2476,
                self.skater.player_input.processed.flags_2480,
                self.skater.board_possession.state.retrieval.progress_200,
                self.skater.animation.motion.toggle_board_physical,
                self.skater
                    .animation
                    .motion
                    .action_intents
                    .contains_key("OB_RetrieveBoard"),
                self.skater
                    .animation
                    .motion
                    .animation
                    .motion_intents
                    .contains_key("OB_Mount"),
                channel.has("RetrieveBoard"),
                channel.elapsed("RetrieveBoard"),
                channel.remaining("RetrieveBoard")
            );
        }
        assert!(
            self.skater
                .render_pose
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite()),
            "Nonfinite pose tick{tick}"
        );
        let roots = &self.skater.animated_skeleton.roots.animation_to_world;
        assert!(
            roots.iter().flatten().all(|v| v.is_finite()),
            "Nonfinite FULL animation root (including W) tick{tick}: {roots:?}"
        );
        assert!(
            self.skater
                .skeleton
                .record
                .centre_of_mass
                .iter()
                .chain(self.skater.skeleton.record.centre_of_mass_velocity.iter())
                .all(|v| v.is_finite()),
            "Nonfinite full physical COM/velocity tick{tick}"
        );
        assert!(
            self.camera
                .frame
                .as_ref()
                .expect("completed camera")
                .position
                .iter()
                .all(|v| v.is_finite()),
            "Nonfinite camera tick{tick}"
        );
    }
    fn assert_held(&self) {
        let s = &self.skater;
        assert_eq!(s.skateboard_controller.fields.state_448, 1);
        assert!(s.skateboard_controller.fields.system_on_452);
        let hand = s.board_possession.state.selected_hand_424 as usize;
        assert!(hand < 2, "Held board has no physical hand owner");
        assert_ne!(
            s.board_possession.state.hands[hand].dynamics,
            [[0, 0, 0, 2]; 2]
        );
        assert!(s.board_possession_live.volumes.deck);
        assert_eq!(s.player_input.physical.off_board.flag_311, 1);
    }
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_right_bumper_reaches_offboard_stock_arm_ready_channel() {
    let mut r = Replay::load();
    let mut ready = false;
    for tick in 0..300 {
        let held = (200..230).contains(&tick);
        r.tick(
            tick,
            if tick == 20 {
                0x8000
            } else if held {
                0x0200
            } else {
                0
            },
            [0; 2],
        );
        if std::env::var_os("SKATE3_TRACE_OFFBOARD_ANIMATION").is_some()
            && matches!(tick, 199 | 220 | 260)
        {
            let animation = &r.skater.animation;
            let globals = animation.evaluator.hierarchy(&animation.pose).unwrap();
            eprintln!(
                "RB_ARM tick={tick} stance={:?} flags={:?} selected_hand={}",
                animation.stance(),
                animation.motion.animation.skater_animation_flags,
                r.skater.board_possession.state.selected_hand_424
            );
            for (bone, name) in animation.evaluator.frames.bone_names.iter().enumerate() {
                if name.contains("ARM") || name.contains("HAND") || name.contains("SHOULDER") {
                    eprintln!(
                        "RB_ARM tick={tick} bone={bone}/{name} mirror={} animation={:?} solved_render={:?}",
                        animation.evaluator.frames.mirror_indices[bone],
                        globals[bone][3],
                        r.skater.render_pose[bone][3]
                    );
                }
            }
        }
        if tick >= 200 {
            assert_eq!(
                r.skater.player_state.current(),
                PhysicalStateId::BipedGround
            );
            assert_eq!(
                r.controls.action_intents.get("GrabWorld").copied(),
                held.then_some(1.0),
                "RB AG publication at tick{tick}"
            );
            assert_eq!(
                r.skater
                    .animation
                    .motion
                    .animation
                    .motion_intents
                    .get("GrabWorld")
                    .copied(),
                held.then_some(1.0),
                "Stock AG-to-MG publication at tick{tick}"
            );
            if held {
                ready |= r
                    .skater
                    .animation
                    .motion
                    .animation
                    .channels
                    .has("SkitchAntic");
                assert_ne!(
                    r.skater.player_input.processed.flags_2476 & 0x400000,
                    0,
                    "Stock offboard AttachIntent did not publish GrabWorld"
                );
            }
        }
    }
    assert!(
        ready,
        "Stock Shove never started the offboard arm-ready channel"
    );
    assert!(
        !r.skater
            .animation
            .motion
            .animation
            .channels
            .has("SkitchAntic"),
        "Arm-ready channel persisted after RB release and its fade"
    );
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn connected_biped_ground_advances_shared_board_steering() {
    let mut r = Replay::load();
    for tick in 0..200 {
        r.tick(tick, if tick == 20 { 0x8000 } else { 0 }, [0; 2]);
    }
    assert_eq!(
        r.skater.player_state.current(),
        PhysicalStateId::BipedGround
    );
    r.skater.ground.steering.deck_tilt = 0.75;
    r.skater.ground.steering.targets = [0.25, -0.5];
    r.skater.ground.steering.activation_time = [0.1, 0.1];
    let mut expected = r.skater.ground.steering;
    r.tick(200, 0, [0; 2]);
    let p = &r.skater.player_input.processed;
    expected.update(
        0.0,
        r.skater.air_settings.steering_blend,
        p.flags_2468,
        p.flags_2472,
    );
    assert_eq!(
        r.skater.ground.steering, expected,
        "Ground must call the existing board steering owner once after feet, before possession"
    );
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn diagnostic_released_board_jump_apex_rb_arm_response() {
    // Diagnostic comparison, not a backwards-man compatibility pass. No Y
    // remount is injected yet: isolate the user's reported RB arm response.
    let mut runs = [Replay::load(), Replay::load()];
    if std::env::var_os("SKATE3_TRACE_OFFBOARD_ANIMATION").is_some() {
        use skate_data::animation_metadata::TreeMetadata;
        let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
        let metadata = skate_data::animation_banks::AnimationBanks::load(&root)
            .unwrap()
            .metadata()
            .unwrap();
        let mut pending = vec!["B_OBAIR_BODYTWEAK_NB_INTO".to_string()];
        while let Some(name) = pending.pop() {
            match metadata.tree(&name).unwrap() {
                TreeMetadata::PhaseBlend(tree) => {
                    eprintln!("APEX_STOCK {tree:?}");
                    pending.extend(tree.children.iter().cloned());
                }
                TreeMetadata::Clip(clip) => {
                    for attribute in &clip.attributes {
                        if attribute.name.to_ascii_lowercase().contains("bodytweak") {
                            eprintln!(
                                "APEX_STOCK clip={} offset={:#x} parameter={attribute:?}",
                                clip.name, clip.source_offset
                            );
                        }
                    }
                }
                _ => panic!("Extend diagnostic for authored tree {name}"),
            }
        }
    }
    let mut apex_tick = None;
    let mut rose = false;
    for tick in 0..560 {
        let s = &runs[0].skater;
        if s.player_state.current() == PhysicalStateId::BipedAir && tick > 400 {
            let vy = s.skeleton.record.centre_of_mass_velocity[1];
            rose |= vy > 0.0;
            if rose && vy <= 0.0 {
                apex_tick.get_or_insert(tick);
            }
        }
        for (variant, r) in runs.iter_mut().enumerate() {
            let rb = variant == 1 && apex_tick == Some(tick);
            let buttons = if tick == 20 {
                0x8000
            } else if tick == 400 {
                0x4000
            } else if rb {
                0x0200
            } else {
                0
            };
            r.tick(
                tick,
                buttons,
                if (200..260).contains(&tick) {
                    [0, 255]
                } else {
                    [0; 2]
                },
            );
            if tick == 400 {
                assert!(
                    matches!(r.skater.skateboard_controller.fields.state_448, 2 | 3),
                    "Diagnostic did not release the board before jumping"
                );
            }
            if apex_tick.is_some_and(|apex| (apex..=apex + 10).contains(&tick)) {
                let a = &r.skater.animation;
                let globals = a.evaluator.hierarchy(&a.pose).unwrap();
                eprintln!(
                    "APEX_RB variant={variant} tick={tick} apex={apex_tick:?} state={:?} stance={:?} AG={:?} MG={:?} flags={:08x}/{:08x}/{:08x} board={}",
                    r.skater.player_state.current(),
                    a.stance(),
                    r.controls.action_intents.get("GrabWorld"),
                    a.motion.animation.motion_intents.get("GrabWorld"),
                    r.skater.player_input.processed.flags_2476,
                    r.skater.player_input.processed.flags_2480,
                    r.skater.player_input.processed.flags_2484,
                    r.skater.skateboard_controller.fields.state_448
                );
                for name in ["RIGHTHAND", "LEFTHAND"] {
                    let bone = a
                        .evaluator
                        .frames
                        .bone_names
                        .iter()
                        .position(|n| n == name)
                        .unwrap();
                    eprintln!(
                        "APEX_RB variant={variant} tick={tick} {name} animation={:?} solved_render={:?}",
                        globals[bone][3], r.skater.render_pose[bone][3]
                    );
                }
            }
        }
    }
    assert!(
        apex_tick.is_some(),
        "Diagnostic did not reach a physical jump apex"
    );
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_repeated_walk_turn_throw_recall_jump_remount_preserves_full_roots_and_joints() {
    let mut r = Replay::load();
    // The uninterrupted route crosses the default flat fixture's z=50 edge
    // at tick4585, before cycle3's commanded jump. Give this endurance test
    // actual collision geometry covering its route; keep the dedicated ledge
    // tests and every landing/ownership/root/joint assertion unchanged.
    r.physics.world = super::offboard_air_tests::edge_world(
        r.physics.settings.floor_material,
        250.0,
        ground::HEIGHT - 100.0,
    );
    //Three uninterrupted cycles exceed both reported827 and3315 histories.
    //All timing and movement below are test commands, not physics parameters.
    for cycle in 0..3 {
        let mut dismounted = false;
        let mut released = false;
        let mut recall_pulse = false;
        let mut returning = false;
        let mut caught = false;
        let mut air_frames = 0;
        let mut landed_frames = 0;
        let mut mounted = false;
        for local in 0..1800 {
            let tick = cycle * 1800 + local;
            let side = if cycle % 2 == 0 { 20000 } else { -20000 };
            let left = match local {
                120..220 => [0, 24000],
                220..280 => [side, 0],
                280..320 => [-side, 12000],
                480..540 => [side, 18000],
                540..600 => [-side, 0],
                930..1030 => [0, 18000],
                1030..1120 => [side, 12000],
                1160..1220 => [-side, 8000],
                _ => [0; 2],
            };
            r.tick_raw(
                tick,
                XboxState {
                    buttons: if local == 20 || local == 1400 {
                        0x8000
                    } else if (1050..1110).contains(&local) {
                        0x4000
                    } else {
                        0
                    },
                    triggers: if (400..460).contains(&local) || (800..860).contains(&local) {
                        [0, 255]
                    } else {
                        [0; 2]
                    },
                    left,
                    right: [0; 2],
                },
            );
            let s = &r.skater;
            let state = s.player_state.current();
            let controller = s.skateboard_controller.fields;
            dismounted |= local < 400 && state == PhysicalStateId::BipedGround;
            if (400..800).contains(&local) && matches!(controller.state_448, 2 | 3) {
                released = true;
                assert_eq!(
                    s.board_possession.state.selected_hand_424, 2,
                    "Free board retained a hand in cycle{cycle} tick{tick}"
                );
                for hand in &s.board_possession.state.hands {
                    assert_eq!(hand.dynamics, [[0, 0, 0, 2]; 2]);
                }
            }
            if (800..1050).contains(&local) {
                recall_pulse |= s.player_input.processed.flags_2476 & 0x800 != 0;
                if controller.state_448 == 4 {
                    returning = true;
                    assert!(!s.board_possession_live.volumes.deck);
                    assert!(!s.board_possession_live.volumes.trucks);
                    assert!(!s.board_possession_live.volumes.wheels);
                }
                if returning && controller.state_448 == 1 {
                    r.assert_held();
                    caught = true;
                }
            }
            if (1050..1400).contains(&local) {
                if state == PhysicalStateId::BipedAir {
                    air_frames += 1;
                }
                if air_frames > 0 && state == PhysicalStateId::BipedGround {
                    landed_frames += 1;
                }
            }
            if local >= 1400 && matches!(state.category(), 100 | 200) {
                mounted = true;
                assert!(!controller.system_on_452);
                assert_eq!(controller.state_448, 0);
                assert_eq!(s.board_possession.state.selected_hand_424, 2);
            }
            if tick >= 120 {
                assert_joint_anchors(s, tick);
            }
        }
        assert!(
            dismounted && released && recall_pulse && returning && caught,
            "Incomplete possession cycle{cycle}: dismounted={dismounted} free={released} pulse={recall_pulse} returning={returning} caught={caught}"
        );
        assert!(
            air_frames >= 8 && landed_frames >= 60,
            "Walking/turning jump cycle{cycle} lacks sustained Air/landing: {air_frames}/{landed_frames}"
        );
        assert!(
            mounted,
            "Remount failed after throw/recall/walking jump cycle{cycle}"
        );
        assert_eq!(
            r.skater.player_state.current(),
            PhysicalStateId::PhysicsGround,
            "Cycle{cycle} did not finish stably onboard before next raw dismount"
        );
    }
}

fn assert_joint_anchors(skater: &SkaterRuntime, tick: usize) {
    let bodies = skater.skeleton.bodies();
    let anchor = |part: usize, words: &[u32]| -> [f32; 3] {
        let b = &bodies[part];
        let local: [f32; 3] = std::array::from_fn(|i| f32::from_bits(words[i]));
        let basis = b.rates.basis.columns;
        let p = b.rates.position;
        let position = [p.x, p.y, p.z];
        std::array::from_fn(|i| {
            position[i] + basis[0][i] * local[0] + basis[1][i] * local[1] + basis[2][i] * local[2]
        })
    };
    for joint in &skater.skeleton_joints.records {
        let a = anchor(joint.child, &joint.frames.words[4..7]);
        let b = anchor(joint.parent, &joint.frames.words[12..15]);
        let gap = (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt();
        //Visible-failure bound already used by the raw dismount/jump tests;
        //this is acceptance measurement, never a solver clamp or parity claim.
        assert!(
            gap < 0.35,
            "Combined replay joint failure tick{tick}: {}->{} gap={gap}",
            joint.parent,
            joint.child
        );
    }
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_y_dismount_then_rt_within_six_frames_does_not_bail() {
    // User reproduction: idle onboard, Y, then RT a few frames later. The
    // first unexpected bail is the failure, not only later joint explosion.
    // delay0 is the Y-only control; 1..6 are delayed RT; case7 is simultaneous.
    for delay in 0..=7usize {
        eprintln!("DISMOUNT_RT_CASE delay={delay}");
        let mut r = Replay::load();
        let mut dismounted = false;
        for tick in 0..240usize {
            let rt = if delay == 7 { 20 } else { 20 + delay };
            r.tick(
                tick,
                if tick == 20 { 0x8000 } else { 0 },
                if delay != 0 && (rt..rt + 30).contains(&tick) {
                    [0, 255]
                } else {
                    [0; 2]
                },
            );
            if std::env::var_os("SKATE3_TRACE_OFFBOARD_ANIMATION").is_some()
                && (20..=24).contains(&tick)
            {
                let s = &r.skater;
                eprintln!(
                    "YRT_INPUT delay={delay} tick={tick} root={:?} board_anim={:?} board_unadjusted={:?} board_drive={:?} force_mode={} trajectory={:?}",
                    s.animated_skeleton.roots.animation_to_world[3],
                    s.animated_skeleton.animation_board,
                    s.animated_skeleton.unadjusted_board,
                    s.skeleton_input.drive_frames[0],
                    s.skeleton_input.force_mode,
                    s.animated_skeleton.motion.next_trajectory
                );
                for (bone, name) in s.animation.evaluator.frames.bone_names.iter().enumerate() {
                    if bone == 0 || name.eq_ignore_ascii_case("Skateboard") {
                        eprintln!(
                            "YRT_POSE delay={delay} tick={tick} bone={bone}/{name} sqt={:?}",
                            s.animation.pose[bone]
                        );
                    }
                }
            }
            let state = r.skater.player_state.current();
            dismounted |= state.category() == 500;
            assert_ne!(
                state.category(),
                300,
                "Y then RT caused bail: delay={delay} tick={tick} state={state:?} requested={:?} board={:?} flags={:08x?}",
                r.skater.player_state.requested_state,
                r.skater.skateboard_controller.fields,
                [
                    r.skater.player_input.processed.flags_2468,
                    r.skater.player_input.processed.flags_2472,
                    r.skater.player_input.processed.flags_2476,
                    r.skater.player_input.processed.flags_2480,
                    r.skater.player_input.processed.flags_2484
                ]
            );
        }
        assert!(
            dismounted,
            "Y never dismounted in delay={delay} reproduction"
        );
    }
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn diagnostic_rt_before_y_dismount_possession_timing() {
    let mut r = Replay::load();
    for tick in 0..180usize {
        r.tick(
            tick,
            if tick == 21 { 0x8000 } else { 0 },
            if (20..50).contains(&tick) {
                [0, 255]
            } else {
                [0; 2]
            },
        );
        if (19..65).contains(&tick) {
            let s = &r.skater;
            eprintln!(
                "RTY_TIMING tick={tick} state={:?} controller={:?} root={:?} deck={:?} physical={:?} channel={}/{}/{} flags={:08x}/{:08x}",
                s.player_state.current(),
                s.skateboard_controller.fields,
                s.animated_skeleton.roots.animation_to_world[3],
                s.animated_skeleton.animation_board[3],
                s.animation.motion.toggle_board_physical,
                s.animation.motion.animation.channels.has("RetrieveBoard"),
                s.animation
                    .motion
                    .animation
                    .channels
                    .elapsed("RetrieveBoard"),
                s.animation
                    .motion
                    .animation
                    .channels
                    .remaining("RetrieveBoard"),
                s.player_input.processed.flags_2476,
                s.player_input.processed.flags_2480
            );
        }
    }
    // Diagnostic completion is not an assertion that possession timing matches stock.
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_deck_free_predicate_tracks_completed_possession_not_not_held() {
    let mut r = Replay::load();
    let mut seen = [false; 5];
    for tick in 0..1000usize {
        let published_state = r.skater.skateboard_controller.fields.state_448;
        r.tick(
            tick,
            if tick == 20 { 0x8000 } else { 0 },
            if (400..430).contains(&tick) || (800..830).contains(&tick) {
                [0, 255]
            } else {
                [0; 2]
            },
        );
        let free = crate::graph_host::motion_stock_conditions::Condition::IsDeckFree
            .evaluate(&r.skater.animation.motion)
            .unwrap();
        assert_eq!(
            free,
            matches!(published_state, 2 | 3 | 4),
            "IsDeckFree tick{tick} completed board state={published_state}"
        );
        if let Some(value) = seen.get_mut(published_state as usize) {
            *value = true;
        }
    }
    assert!(
        seen[0] && seen[1] && seen[2] && seen[4],
        "Replay must cover riding, held, released and returning: {seen:?}"
    );
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_y_and_right_trigger_adjacent_edges_preserve_roots_and_joints() {
    // Reproduction commands only: exercise overlapping dismount/throw and
    // remount/recall without injecting intentions or physical state changes.
    for context in ["onboard", "held", "free"] {
        for offset in [-1isize, 0, 1] {
            eprintln!("Y_RT_CASE context={context} trigger_offset={offset}");
            let mut r = Replay::load();
            for tick in 0..1000usize {
                let trigger_tick = (600isize + offset) as usize;
                let preparing_release = context == "free" && (250..310).contains(&tick);
                r.tick_raw(
                    tick,
                    XboxState {
                        buttons: if (context != "onboard" && tick == 20) || tick == 600 {
                            0x8000
                        } else {
                            0
                        },
                        triggers: if preparing_release
                            || (trigger_tick..trigger_tick + 30).contains(&tick)
                        {
                            [0, 255]
                        } else {
                            [0; 2]
                        },
                        left: if (450..750).contains(&tick) {
                            [0, 22000]
                        } else {
                            [0; 2]
                        },
                        right: [0; 2],
                    },
                );
                if tick == 440 {
                    match context {
                        "onboard" => assert!(matches!(
                            r.skater.player_state.current().category(),
                            100 | 200
                        )),
                        "held" => r.assert_held(),
                        "free" => assert!(matches!(
                            r.skater.skateboard_controller.fields.state_448,
                            2 | 3
                        )),
                        _ => unreachable!(),
                    }
                }
                if tick >= 120 {
                    assert_joint_anchors(&r.skater, tick);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_walk_turn_and_recall_without_jumping_preserves_full_roots_and_joints() {
    let mut r = Replay::load();
    let mut released = false;
    let mut returning = false;
    let mut caught = false;
    for tick in 0..3600usize {
        let phase = tick % 1200;
        r.tick_raw(
            tick,
            XboxState {
                buttons: if tick == 20 { 0x8000 } else { 0 },
                triggers: if (400..430).contains(&phase) || (800..830).contains(&phase) {
                    [0, 255]
                } else {
                    [0; 2]
                },
                left: if tick < 120 {
                    [0; 2]
                } else {
                    match phase {
                        0..300 => [0, 24000],
                        300..600 => [18000, 14000],
                        600..900 => [-18000, 18000],
                        _ => [0; 2],
                    }
                },
                right: [0; 2],
            },
        );
        let state = r.skater.skateboard_controller.fields.state_448;
        released |= matches!(state, 2 | 3);
        returning |= state == 4;
        caught |= returning && state == 1;
        if tick >= 120 {
            assert_joint_anchors(&r.skater, tick);
        }
    }
    assert!(
        released && returning && caught,
        "Walking replay did not complete throw/recall"
    );
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_trigger_recall_returns_free_board_to_physical_hand() {
    let mut r = Replay::load();
    let mut released = false;
    let mut recall_intent = false;
    let mut recall_attribute = false;
    let mut returning = false;
    let mut caught = false;
    for tick in 0..1800 {
        //A second right-trigger rising edge requests recall once the board is
        //free. No injected intentions, animation attributes or physical states.
        let triggers = if (400..460).contains(&tick) || (800..860).contains(&tick) {
            [0, 255]
        } else {
            [0; 2]
        };
        r.tick(tick, if tick == 20 { 0x8000 } else { 0 }, triggers);
        if tick == 300 {
            r.assert_held();
        }
        let s = &r.skater;
        if (400..800).contains(&tick) {
            released |= matches!(s.skateboard_controller.fields.state_448, 2 | 3);
        }
        if tick >= 800 {
            recall_intent |= s
                .animation
                .motion
                .action_intents
                .contains_key("OB_RetrieveBoard");
            recall_attribute |= s.player_input.processed.flags_2476 & 0x800 != 0;
            returning |= s.skateboard_controller.fields.state_448 == 4;
            if returning && s.skateboard_controller.fields.state_448 == 1 {
                r.assert_held();
                caught = true;
            }
        }
    }
    assert!(released, "Trigger never released the originally held board");
    assert!(
        recall_intent,
        "Raw trigger never reached stock recall AG input"
    );
    assert!(
        recall_attribute,
        "RetrieveBoard animation never emitted physical recall pulse2476bit11"
    );
    assert!(
        returning,
        "Recall pulse never entered possession RETURNING4"
    );
    assert!(
        caught,
        "Real retrieval never completed into held board and armed hand drive"
    );
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_y_offboard_reaches_stock_remount_and_stops_hand_controller() {
    let mut r = Replay::load();
    let mut mount_intent = false;
    let mut mounting_attribute = false;
    let mut mounted = false;
    for tick in 0..1800 {
        r.tick(
            tick,
            if tick == 20 || tick == 400 { 0x8000 } else { 0 },
            [0; 2],
        );
        if tick == 300 {
            assert_eq!(
                r.skater.player_state.current(),
                PhysicalStateId::BipedGround
            );
            r.assert_held();
        }
        let s = &r.skater;
        if tick == 400 {
            assert!(
                s.animation
                    .motion
                    .action_intents
                    .contains_key("NewToggleOffBoardState"),
                "Y while offboard was suppressed instead of reaching stock remount"
            );
            assert!(
                s.animation
                    .motion
                    .action_intents
                    .contains_key("ToggleOffBoardState")
            );
            assert!(
                !s.animation
                    .motion
                    .action_intents
                    .contains_key("OB_RetrieveBoard"),
                "Y was incorrectly replaced with trigger recall"
            );
        }
        if tick >= 400 {
            mount_intent |= s
                .animation
                .motion
                .animation
                .motion_intents
                .contains_key("OB_Mount");
            mounting_attribute |= s.player_input.processed.flags_2480 & 0x80000 != 0;
            if matches!(s.player_state.current().category(), 100 | 200) {
                mounted = true;
                assert!(!s.skateboard_controller.fields.system_on_452);
                assert_eq!(s.skateboard_controller.fields.state_448, 0);
                assert_eq!(s.board_possession.state.selected_hand_424, 2);
                for hand in &s.board_possession.state.hands {
                    assert_eq!(
                        hand.dynamics,
                        [[0, 0, 0, 2]; 2],
                        "Onboard physical state retains offboard hand drive at tick{tick}"
                    );
                }
            }
        }
    }
    assert!(mount_intent, "Stock OffBoard AG never mapped Y to OB_Mount");
    assert!(
        mounting_attribute,
        "Stock mount animation never emitted OB_Mounting"
    );
    assert!(
        mounted,
        "Stock remount never returned to an onboard physical state"
    );
}
