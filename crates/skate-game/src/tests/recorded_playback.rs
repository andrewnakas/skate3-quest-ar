//! User-captured packets through the production frame. The level, initial
//! pose and controller phase are clone-owned; this is not a stock parity oracle.
use super::*;

#[test]
#[ignore = "requires private assets and SKATE3_INPUT_RECORDING"]
fn recorded_controller_sequence_runs_production_gameplay() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let path = std::env::var_os("SKATE3_INPUT_RECORDING").expect("set SKATE3_INPUT_RECORDING");
    let recording =
        skate_data::input_recording::Recording::load(std::path::Path::new(&path)).unwrap();
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    // User confirmed this recording was made on flat ground. Keep the same
    // authored surface material and query metadata while removing the ramp.
    physics.world = ground::flat_world(physics.world.triangles()[0].material);
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::load(root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut seconds = 0.0_f64;
    let mut previous = None;
    let mut airborne = 0;
    let mut manual = 0;
    let mut bail = 0;
    let mut maximum_rider_com_y = f32::NEG_INFINITY;
    while seconds * 1_000_000.0 <= recording.duration_us() as f64 {
        let sample = recording.at_elapsed_us((seconds * 1_000_000.0) as u64);
        assert!(sample.connected, "fixture disconnected at {}", sample.t_us);
        input.sample_raw_for_test(sample.raw_state());
        let mut actions = input.player_actions();
        let dt = physics.settings.step.simulation.time_step;
        controls.update(
            &mut actions,
            dt,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        controls.publish_gestures(
            physics.animation_profile.physics_mode,
            skater.player_input.physical.state.state_16,
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
        let current = (
            state,
            skater.animation.motion.flags.manualing,
            skater.animation.motion.flags.doing_trick,
            skater.animation.motion.animation.current_name.clone(),
            skater.wipeout.state.count,
        );
        if previous.as_ref() != Some(&current) || result.is_err() {
            let deck = physics.board.bodies()[6].rates;
            eprintln!(
                "RECORDING t={seconds:.4} sample={} state={current:?} p={:?} v={:?} balance={} jump={} rider_com={:?} rider_v={:?} pose_error={:?} bail_reasons={:?}",
                sample.sample,
                deck.position,
                deck.linear_velocity,
                skater.animation_input.fields.balance,
                skater.animation_input.extra.jump_strength,
                skater.skeleton.record.centre_of_mass,
                skater.animated_skeleton.board_frames.com_velocity,
                skater.collision_maximum_error,
                skater.wipeout.state.reasons,
            );
            previous = Some(current);
        }
        result.unwrap_or_else(|e| panic!("Recording t={seconds:.4} sample={}: {e}", sample.sample));
        airborne += u32::from(state.category() == 200);
        manual += u32::from(skater.animation.motion.flags.manualing);
        bail += u32::from(state.category() == 300);
        maximum_rider_com_y = maximum_rider_com_y.max(skater.skeleton.record.centre_of_mass[1]);
        assert_eq!(skater.pose_generation, physics.ticks);
        assert!(camera.frame.is_some());
        seconds += f64::from(dt);
    }
    assert!(airborne > 0, "capture never reached an onboard air state");
    eprintln!(
        "RECORDING completed: ticks={} air={airborne} manual={manual} bail={bail} max_rider_com_y={maximum_rider_com_y}; smoke check only, NOT stock parity",
        physics.ticks
    );
}
