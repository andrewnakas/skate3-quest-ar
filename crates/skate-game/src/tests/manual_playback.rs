//! Raw controller -> authored manual states -> real shared solve and release.
use super::*;
use skate_core::player::state::PhysicalStateId;

#[test]
#[ignore = "requires private stock graph/animation/physics assets"]
fn raw_controller_manuals_enter_balance_and_release_in_gameplay() {
    for sign in [-1_i16, 1] {
        run_manual(sign);
    }
}

fn run_manual(sign: i16) {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::load(root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut manual_seen = false;
    let mut balance_seen = false;
    let mut controller_seen = false;
    let mut timer_seen = false;
    let mut release_seen = false;
    for tick in 0..330 {
        // A sustained partial stick is not a Flickit flick. Use the actual
        // converter and gesture pipeline, including its authored exclusions.
        let holding = (45..195).contains(&tick);
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if (12..40).contains(&tick) { 0x1000 } else { 0 },
            triggers: [0; 2],
            left: [0; 2],
            right: if holding { [0, sign * 22000] } else { [0; 2] },
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
                "Manual sign={sign} tick={tick}: {error}; state={:?}; balance={}; deck={:?}",
                skater.player_state.current(),
                skater.animation_input.fields.balance,
                physics.board.bodies()[6].rates
            )
        });
        manual_seen |= skater.animation.motion.flags.manualing;
        balance_seen |=
            skater.animation.motion.flags.manualing && skater.animation_input.fields.balance != 0.0;
        controller_seen |= skater.ground.manual.elapsed > 0.0
            && skater.ground.manual.angular_correction != 0.0
            && skater.player_state.current() == PhysicalStateId::PhysicsGround;
        if tick >= 195 {
            assert!(controls.action_intents.get("Manual").is_none());
            assert!(
                skater
                    .animation
                    .motion
                    .animation
                    .motion_intents
                    .get("Manual")
                    .is_none()
            );
            timer_seen |= skater.animation.motion.riding.manual_out_timer > 0.0;
            release_seen |= !skater.animation.motion.flags.manualing
                && skater.animation_input.fields.balance == 0.0
                && skater.ground.manual.elapsed == 0.0;
        }
        assert_eq!(skater.pose_generation, physics.ticks);
        assert!(camera.frame.is_some());
        assert_eq!(physics.exchange.output().unwrap().tick + 1, physics.ticks);
    }
    assert!(manual_seen, "Authored manual never entered: sign={sign}");
    assert!(
        balance_seen,
        "Manual never published physical balance: sign={sign}"
    );
    assert!(
        controller_seen,
        "Manual never reached Ground controller: sign={sign}"
    );
    assert!(
        timer_seen,
        "Manual exit never armed authored timer: sign={sign}"
    );
    assert!(
        release_seen,
        "Manual controller did not clear after release: sign={sign}"
    );
    assert_eq!(skater.animation.motion.riding.manual_out_timer, 0.0);
    eprintln!(
        "Manual sign={sign}: authored entry, physical balance/controller, release, pose and camera passed {} ticks",
        physics.ticks
    );
}
