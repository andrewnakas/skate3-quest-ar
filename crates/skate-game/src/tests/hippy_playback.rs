//! Real raw input, stock graphs, shared physical solve and camera; no state injection.
use super::*;
use skate_core::player::state::PhysicalStateId;

#[test]
#[ignore = "requires private stock graph/animation/physics assets"]
fn raw_controller_hippy_jump_returns_to_riding() {
    run_jump(25, false, 1);
}

#[test]
#[ignore = "requires private stock graph/animation/physics assets"]
fn hippy_charge_repeat_and_rolling_gameplay() {
    for hold in [12, 60] {
        run_jump(hold, false, 1);
    }
    run_jump(25, false, 2);
    run_jump(25, true, 1);
}

fn run_jump(hold: usize, pushing: bool, repetitions: usize) {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::load(root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut entered = false;
    let mut descended = false;
    let mut landed = false;
    let mut peak = f32::NEG_INFINITY;
    let mut launch_height = 0.0;
    let mut landing_ticks = 0;
    let mut previous_landing = false;
    let mut landing_count = 0;
    let mut board_launch = None;
    let mut rolling_distance = 0.0_f32;
    for tick in 0..(180 * repetitions + 120) {
        let charge = tick >= 45 && tick < 45 + 180 * repetitions && (tick - 45) % 180 < hold;
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if charge {
                0x5000
            } else if pushing && (12..32).contains(&tick) {
                0x1000
            } else {
                0
            },
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
        controls.publish_gestures(
            physics.animation_profile.physics_mode,
            skater.player_input.physical.state.state_16,
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
        .unwrap_or_else(|error| {
            panic!(
                "Hippy tick={tick}: {error}; state={:?}; root={:?}",
                skater.player_state.current(),
                skater.animated_skeleton.roots.animation_to_world[3]
            )
        });
        let state = skater.player_state.current();
        let height = skater.skeleton.record.centre_of_mass[1];
        assert!(height.is_finite(), "nonfinite physical COM at {tick}");
        if state == PhysicalStateId::LandingOnDeck {
            if !previous_landing {
                board_launch = Some(physics.board.bodies()[6].rates.position);
            }
            if let Some(start) = board_launch {
                let now = physics.board.bodies()[6].rates.position;
                rolling_distance = rolling_distance
                    .max(((now.x - start.x).powi(2) + (now.z - start.z).powi(2)).sqrt());
            }
            if !entered {
                launch_height = height;
            }
            entered = true;
            landing_ticks += 1;
            peak = peak.max(height);
            descended |= height < peak - 0.1;
            assert!(skater.landing_deck.manager.trajectory_valid_164);
            assert_eq!(skater.ground_lifecycle.board_animated_290, 0);
        } else if entered && state == PhysicalStateId::PhysicsGround {
            landed = true;
            if previous_landing {
                landing_count += 1;
            }
            assert!(!skater.landing_deck.manager.trajectory_valid_164);
        }
        if tick % 40 == 0 || previous_landing != (state == PhysicalStateId::LandingOnDeck) {
            eprintln!(
                "hippy tick={tick} state={state:?} com_y={height} time={} can_land={} near={}",
                skater.landing_on_deck.state.time_to_land,
                skater.landing_deck.fill().can_land_316,
                skater.landing_on_deck.state.near_deck
            );
        }
        previous_landing = state == PhysicalStateId::LandingOnDeck;
        assert_eq!(skater.pose_generation, physics.ticks);
        assert!(camera.frame.is_some());
    }
    assert!(entered, "Raw A+X never entered stock hippy graph/state503");
    assert!(
        peak > launch_height + 0.15,
        "physical rider never rose: {launch_height} -> {peak}"
    );
    assert!(descended, "physical rider did not descend from apex");
    assert!(
        landed,
        "physical rider never returned to riding; final={:?}",
        skater.player_state.current()
    );
    assert_eq!(
        landing_count, repetitions,
        "hold={hold} pushing={pushing}: missing re-entry/landing"
    );
    assert!(
        landing_ticks > 5 && landing_ticks < 100 * repetitions,
        "landing state stalled: {landing_ticks}"
    );
    if pushing {
        assert!(
            rolling_distance > 0.1,
            "board stopped during rolling hippy jump"
        );
    }
}
