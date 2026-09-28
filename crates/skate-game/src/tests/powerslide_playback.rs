//! Raw controller -> stock graphs -> canonical packet -> SlideGround regression.
use super::*;
use skate_core::{animation::skeleton_input::name::encode, player::state::PhysicalStateId};

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn raw_controller_powerslide_uses_stock_graph_and_releases_both_sides() {
    for side in [-1_i16, 1] {
        run_side(side);
    }
}

fn run_side(side: i16) {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let data = skate_data::collections::Collections::load(root).unwrap();
    let entry_speed = data
        .float("anim_motion", "power_slide", "slide_speed_threshold")
        .unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    // Test phases only choose controller packets. Physics and graph state are
    // never overwritten, and the production world remains intact.
    let mut sequence_start = None;
    let mut saw_push = false;
    let mut saw_start = false;
    let mut saw_slide = false;
    let mut saw_packet = false;
    let mut release_clear = false;
    for tick in 0..900 {
        if sequence_start.is_none()
            && tick >= 60
            && physics.riding.motion.ground_speed > entry_speed * 1.5
            && skater.player_state.current() == PhysicalStateId::PhysicsGround
        {
            sequence_start = Some(tick);
        }
        let phase = sequence_start.map(|start| tick - start);
        let left = match phase {
            Some(12..=23) => [side * 32767, 0],
            Some(36..=95) => [side * 22000, 32767],
            _ => [0; 2],
        };
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if tick >= 12 && sequence_start.is_none() {
                0x1000
            } else {
                0
            },
            triggers: [0; 2],
            left,
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        frame::advance(&mut physics, &mut skater, &mut controls, &graphs,
            &mut actions, true, &mut camera).unwrap_or_else(|error| panic!(
                "Powerslide side={side} tick={tick} phase={phase:?}: {error}; state={:?}; speed={}; deck={:?}",
                skater.player_state.current(), physics.riding.motion.ground_speed,
                physics.board.bodies()[6].rates));
        saw_push |= skater.player_input.processed.flags_2468 & (1 << 25) != 0;
        let motion = &skater.animation.motion;
        let starts = motion.slide_latch.start(true) || motion.slide_latch.start(false);
        if matches!(phase, Some(12..=23)) {
            assert!(
                controls.action_intents.get("RightSlideStart").is_none()
                    && controls.action_intents.get("LeftSlideStart").is_none(),
                "Pure lateral stick generated a slide start: side={side} tick={tick}"
            );
            assert!(!starts, "Pure lateral stick started the slide latch");
            assert_ne!(
                skater.player_state.current(),
                PhysicalStateId::SlideGround,
                "Pure lateral stick entered SlideGround"
            );
        }
        if matches!(phase, Some(36..=95)) {
            saw_start |= starts;
            saw_slide |= skater.player_state.current() == PhysicalStateId::SlideGround;
            let emitted = motion
                .animation
                .motion_attributes
                .iter()
                .any(|attribute| attribute.name == encode(b"slide") && attribute.value != 0.0);
            let published = skater
                .animation
                .attributes
                .entries()
                .iter()
                .rev()
                .find(|attribute| {
                    attribute.name == encode(b"slide") && matches!(attribute.kind, 0 | 2)
                })
                .and_then(|attribute| attribute.payload.0[0])
                .map(f32::from_bits);
            if emitted && let Some(value) = published {
                assert_eq!(
                    skater.animation_input.fields.slide.to_bits(),
                    value.to_bits(),
                    "Slide changed between canonical animation packet and Skeleton input"
                );
                saw_packet |= value != 0.0;
            }
        }
        if matches!(phase, Some(97..)) {
            for name in [
                "LeftSlideStart",
                "RightSlideStart",
                "LeftSlide",
                "RightSlide",
            ] {
                assert!(
                    controls.action_intents.get(name).is_none(),
                    "Released AG intent retained: {name}"
                );
                assert!(
                    motion.animation.motion_intents.get(name).is_none(),
                    "Released MG intent retained: {name}"
                );
            }
            release_clear |= !motion.is_power_sliding
                && skater.player_state.current() != PhysicalStateId::SlideGround;
        }
        assert!(camera.frame.is_some());
        assert_eq!(skater.pose_generation, physics.ticks);
        if phase.is_some_and(|phase| phase >= 215) {
            break;
        }
    }
    assert!(
        sequence_start.is_some(),
        "Real pushing never attained stock slide speed: side={side}, speed={}, threshold={entry_speed}",
        physics.riding.motion.ground_speed
    );
    assert!(saw_push, "No physical push was published");
    assert!(
        saw_start,
        "Diagonal input never started the stock slide latch: side={side}"
    );
    assert!(
        saw_packet,
        "No canonical nonzero slide packet reached Skeleton: side={side}"
    );
    assert!(
        saw_slide,
        "Stock graph never entered SlideGround: side={side}"
    );
    assert!(
        release_clear,
        "Slide failed to exit after controller release: side={side}"
    );
    eprintln!(
        "Powerslide side={side}: stock entry, packet, physical SlideGround and release passed in {} ticks",
        physics.ticks
    );
}
