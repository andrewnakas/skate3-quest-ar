//! Integration with the extracted stock data and the real simulation pipeline.
use super::*;
use crate::difficulty::Difficulty;

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn stock_difficulties_switch_without_resetting_the_player() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let data = Collections::load(root).unwrap();
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(root, None, Difficulty::Easy).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "easy").unwrap();
    let mut controls = PlayerControls::load(root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();

    // Check the complete mode/surface product before exercising live switches.
    for mode in Difficulty::ALL {
        for surface in 1..=5 {
            let settings = skater.ground_profiles.select(mode as u32, surface).unwrap();
            let key = ground_runtime::surface_key(surface).unwrap();
            assert_eq!(settings.wheel_material.dynamic_friction.to_bits(),
                data.float("physics_surfaces", key, "WheelDynamicFriction").unwrap().to_bits());
            assert_eq!(settings.board().propulsion.mode_speed_changes.map(f32::to_bits),
                ["MaxPushDVStart", "MaxPushDVEnd"].map(|f| data.float("physics_mode", mode.key(), f).unwrap().to_bits()));
            assert_eq!(settings.wobble_amplitude.to_bits(),
                data.float("physics_mode", mode.key(), "Hash_5B57F2CCCCEEF430").unwrap().to_bits());
        }
    }
    assert!(skater.ground_profiles.select(5, 1).is_err());
    assert!(skater.ground_profiles.select(0, 0).is_err());

    for mode in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hardcore, Difficulty::Easy] {
        let before = physics.board.bodies().map(|b| (b.rates.position, b.rates.linear_velocity));
        let ticks = physics.ticks;
        let generation = skater.pose_generation;
        physics.set_difficulty(mode);
        assert_eq!(physics.ticks, ticks);
        assert_eq!(skater.pose_generation, generation);
        assert_eq!(physics.board.bodies().map(|b| (b.rates.position, b.rates.linear_velocity)), before);
        for _ in 0..60 {
            input.sample_raw_for_test(skate_core::input::xbox::XboxState {
                buttons: 0, triggers: [0; 2], left: [0; 2], right: [0; 2],
            });
            let mut actions = input.player_actions();
            controls.update(&mut actions, physics.settings.step.simulation.time_step,
                physics.settings.input_magnitude_threshold, skater.player_input.physical.scoring.capabilities_204);
            controls.publish_gestures(physics.difficulty_index(), skater.player_input.physical.state.state_16);
            frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera).unwrap();
            assert_eq!(skater.player_input.processed.state_variant_index_2528, mode as u32);
            let selected = skater.ground_profiles.select(mode as u32, skater.player_input.processed.surface_mode_2540).unwrap();
            assert!(std::sync::Arc::ptr_eq(&selected, &skater.ground_settings));
        }
        assert_eq!(physics.ticks, ticks + 60);
    }
}
