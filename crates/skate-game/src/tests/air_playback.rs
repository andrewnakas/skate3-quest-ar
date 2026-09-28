//! Real controller/graph/physical/pose/camera regression across a level edge.
use super::*;
use skate_core::player::state::PhysicalStateId;

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn stock_push_off_edge_runs_air_and_landing_in_production() {
    roll_off(None);
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn airborne_stick_reaches_physical_spin() {
    for stick in [-32767, 32767] {
        roll_off(Some(stick));
    }
}

fn roll_off(air_stick: Option<i16>) {
    roll_off_input(air_stick, None, [0; 2]);
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn airborne_grabs_release_through_stock_graphs() {
    for hand in 0..2 {
        for stick in [[0, 0], [32767, 0], [-32767, 0], [0, 32767], [0, -32767]] {
            roll_off_input(None, Some(hand), stick);
        }
    }
}

fn roll_off_input(air_stick: Option<i16>, grab_hand: Option<usize>, grab_stick: [i16; 2]) {
    roll_off_sequence(air_stick, grab_hand, grab_stick, false);
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn airborne_tailwalk_from_raw_controller_reaches_stock_cycle() {
    roll_off_sequence(None, Some(0), [0, 32767], true);
}

fn roll_off_sequence(
    air_stick: Option<i16>,
    grab_hand: Option<usize>,
    grab_stick: [i16; 2],
    tailwalk: bool,
) {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_terrain(root, ground::Terrain::Course).unwrap();
    // Keep this air fixture's open edge now that the production level has a
    // downhill ramp. The dedicated ramp regression uses the unmodified world.
    physics.world = BoardWorld::new(
        physics
            .world
            .triangles()
            .iter()
            .copied()
            .filter(|face| {
                !(face.tag == 1 && face.triangle.vertices.iter().any(|vertex| vertex.z > 4.0))
            })
            .collect(),
    );
    if grab_hand.is_some() {
        // A deeper test landing gives the held cycle and OUT animation time
        // to run before contact can hide a failed airborne release.
        physics.world = BoardWorld::new(
            physics
                .world
                .triangles()
                .iter()
                .map(|face| {
                    let vertices = face.triangle.vertices.map(|mut vertex| {
                        if vertex.y < -1.0 {
                            vertex.y -= 18.0;
                        }
                        vertex
                    });
                    skate_core::physics::board_world::WorldTriangle::from_vertices(
                        vertices,
                        face.material,
                        face.tag,
                        skate_core::physics::collision::TriangleFeature::ONE_SIDED,
                        [1.0; 3],
                        0.0,
                    )
                    .unwrap()
                })
                .collect(),
        );
    }
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut previous = skater.player_state.current();
    let mut saw_air = false;
    let mut landed_frames = 0;
    let mut airborne_frames = 0;
    let mut grab_frames = 0;
    let mut saw_grab = false;
    let mut saw_release = false;
    for tick in 0..1000 {
        let mut triggers = [0; 2];
        if let Some(hand) = grab_hand {
            if saw_air && grab_frames < if tailwalk { 100 } else { 35 } {
                triggers[hand] = 255;
            }
        }
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if tick >= 12 && !saw_air {
                0x1000
            } else if tailwalk && saw_air && grab_frames >= 20 {
                0x2000
            } else {
                0
            },
            triggers,
            left: [if saw_air { air_stick.unwrap_or(0) } else { 0 }, 0],
            right: if triggers != [0; 2] {
                grab_stick
            } else {
                [0; 2]
            },
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        let result = frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut actions,
            true,
            &mut camera,
        );
        let state = skater.player_state.current();
        if state != previous {
            eprintln!(
                "Roll-off tick{tick}: {previous:?} -> {state:?}; deck={:?}; velocity={:?}; wipeout={:?}",
                physics.board.bodies()[6].rates.position,
                physics.board.bodies()[6].rates.linear_velocity,
                skater.wipeout.state
            );
            previous = state;
        }
        result.unwrap_or_else(|e| {
            panic!(
                "Roll-off tick{tick}: {e}; selected={:?}; flags={:08x}/{:08x}/{:08x}; board={:?}",
                skater.player_state.requested_state,
                skater.player_input.processed.flags_2468,
                skater.player_input.processed.flags_2472,
                skater.player_input.processed.flags_2476,
                physics.board.bodies()[6].rates
            )
        });
        saw_air |= state.category() == 200;
        if tailwalk && state.category() == 200 {
            let motion = &skater.animation.motion;
            if grab_frames >= 20 {
                eprintln!(
                    "Tailwalk frame={grab_frames} LeftPush={:?} RightPush={:?} angle={:?} TailGrab={:?} NoFootAirWalk={:?} animation={:?}",
                    controls.action_intents.get("LeftPush"),
                    controls.action_intents.get("RightPush"),
                    controls.action_intents.get("BoardAdjustAngle"),
                    motion.animation.motion_intents.get("TailGrab"),
                    motion.animation.motion_intents.get("NoFootAirWalk"),
                    motion.animation.current_name
                );
            }
            if motion.animation.current_name.as_deref() == Some("2FT_AIR_GRAB_N_TAIL_0_CYC") {
                assert!(motion.animation.motion_intents.contains_key("TailGrab"));
                assert!(
                    motion
                        .animation
                        .motion_intents
                        .contains_key("NoFootAirWalk")
                );
                assert_eq!(
                    motion.score_packet.grab.map(|(name, _)| name),
                    Some(skate_core::animation::skeleton_input::name::encode(
                        b"tailgrab_airwalk"
                    ))
                );
                assert_eq!(skater.pose_generation, physics.ticks);
                return;
            }
        }
        if grab_hand.is_some() && state.category() == 200 {
            grab_frames += 1;
            let anim = &skater.animation.motion.animation;
            let grabbing = skater.player_input.processed.flags_2468 & 0x20 != 0;
            saw_grab |= grabbing;
            saw_release |= saw_grab && grab_frames > 35 && !grabbing;
            if matches!(grab_frames, 3 | 34 | 36 | 50 | 65) {
                eprintln!(
                    "Grab hand={grab_hand:?} stick={grab_stick:?} frame={grab_frames} trigger={triggers:?} AG={:?}/{:?} MG={:?}/{:?} animation={:?} flags={:08x}",
                    controls.action_intents.get("LeftAirGrab"),
                    controls.action_intents.get("RightAirGrab"),
                    anim.motion_intents.get("WantsLeftAirGrab"),
                    anim.motion_intents.get("WantsRightAirGrab"),
                    anim.current_name,
                    skater.player_input.processed.flags_2468
                );
            }
        }
        if saw_air && air_stick.is_some() {
            airborne_frames += 1;
            eprintln!(
                "Air stick={air_stick:?} frame{airborne_frames}: AG={:?}/{:?} MG={:?}/{:?} attr={}/{} speed={} angle={} heading={:?} target={:?} deck={:?}",
                controls.action_intents.get("BodySpin"),
                controls.action_intents.get("PhysBodySpin"),
                skater
                    .animation
                    .motion
                    .animation
                    .motion_intents
                    .get("BodySpin"),
                skater
                    .animation
                    .motion
                    .animation
                    .motion_intents
                    .get("PhysBodySpin"),
                skater.animation_input.fields.body_spin,
                skater.animation_input.extra.physical_body_spin,
                skater.air_reckoning.state.spin_speed,
                skater.air_reckoning.state.spin_angle,
                physics.riding.reckoning_frames.heading,
                skater.animated_skeleton.roots.animation_to_world[2],
                physics.board.part_transforms()[6].basis.columns[2]
            );
            if airborne_frames == 16 {
                assert!(
                    skater.air_reckoning.state.spin_angle.abs() > 0.001,
                    "Raw airborne stick did not reach physical angular integration"
                );
                return;
            }
        }
        if saw_air
            && state == PhysicalStateId::PhysicsGround
            && physics.riding.ground.wheel_contact_count > 0
        {
            landed_frames += 1;
        }
        assert!(camera.frame.is_some());
        assert_eq!(skater.pose_generation, physics.ticks);
        if landed_frames >= 120 {
            break;
        }
    }
    assert!(saw_air, "Raw pushing never entered the actual Air family");
    assert!(
        !tailwalk,
        "Raw airborne LT + right stick up + B did not produce the Tailwalk cycle"
    );
    if grab_hand.is_some() {
        assert!(saw_grab, "Grab never became active");
        assert!(saw_release, "Grab never released while airborne");
    }
    assert!(
        landed_frames >= 120,
        "Air never returned to stable grounded riding"
    );
    eprintln!(
        "Completed roll-off/landing: ticks={}; grounded_after_air={landed_frames}",
        physics.ticks
    );
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn stock_starting_ramp_reaches_floor_without_air() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_terrain(root, ground::Terrain::Course).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    for tick in 0..600 {
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if tick >= 12 { 0x1000 } else { 0 },
            triggers: [0; 2],
            left: [0; 2],
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physics.settings.step.simulation.time_step,
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
        .unwrap_or_else(|e| panic!("Ramp tick{tick}: {e}"));
        assert_ne!(
            skater.player_state.current().category(),
            200,
            "Ramp entered air at tick{tick}, deck={:?}",
            physics.board.bodies()[6].rates.position
        );
        let deck = physics.board.bodies()[6].rates.position;
        if deck.z > 19.0 {
            assert!(
                deck.y < ground::FLOOR_HEIGHT + 0.5,
                "Did not descend to floor: {deck:?}"
            );
            assert!(physics.riding.ground.wheel_contact_count > 0);
            eprintln!("Reached lower floor without air at tick{tick}: {deck:?}");
            return;
        }
    }
    panic!("Did not reach the lower floor within 600 ticks");
}
