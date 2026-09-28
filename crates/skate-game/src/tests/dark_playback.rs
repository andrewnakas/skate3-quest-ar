//! Dark catches: an ordinary flip scoop thrown with the dark-catch button held.
//!
//! `trick_intentions::produce` reads the request from the packed action flags rather than from
//! gesture timing -- `dark_flags = (1 << 20) | (1 << 28)`, so either of two buttons arms it --
//! and publishes `DarkCatch` while held plus `NewDarkCatch` on the rising edge.
//! `gesture_mapping::select` then resolves the flick through the dark-catch column of the
//! 270-row table, and `motion_tricks::Operation::MonitorUnderflip` latches
//! `trick_requests.dark_catch` when no `U_*` underflip intent is present instead.
use super::*;
use skate_core::animation::skeleton_input::name::encode;

/// The kickflip and heelflip dark catches, off the same authored scoops the ordinary ladder
/// uses.
///
/// `dark_flags` arms on either of two packed bits, but only RB reaches the catch from a rolling
/// approach: the other bit is B, and B while riding is `Brake` (`riding_intentions` emits it
/// from the same button), so the pop never happens. Both are asserted so that stays pinned.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn a_flip_with_the_dark_button_held_reaches_the_dark_catch() {
    for (gesture, trick) in [
        ("Kickflip", "kickflip_darkcatch"),
        ("Heelflip", "heelflip_darkcatch"),
    ] {
        let armed = replay(gesture, RB, 300);
        assert!(
            armed.tricks.iter().any(|t| t == trick),
            "{trick}: RB never published it; saw {:?} over {:?}",
            armed.tricks,
            armed.animations
        );
        let braked = replay(gesture, B, 300);
        assert!(
            braked.tricks.is_empty(),
            "{gesture}: B is Brake while riding, so it should not pop at all; saw {:?}",
            braked.tricks
        );
    }
}

const B: u16 = 0x2000;
const RB: u16 = 0x0200;

/// Takeoff boost, as in `flip_playback.rs`: the flat course's own pop is too short for the
/// catch to resolve before landing.
const BOOST: f32 = 12.0;

#[derive(Default)]
struct Outcome {
    animations: Vec<String>,
    tricks: Vec<String>,
}

const WATCHED: [&str; 8] = [
    "kickflip",
    "heelflip",
    "kickflip_darkcatch",
    "heelflip_darkcatch",
    "kickflip2_darkcatch",
    "heelflip2_darkcatch",
    "360flip_darkcatch",
    "laserflip_darkcatch",
];

fn replay(gesture: &str, buttons: u16, ticks: u32) -> Outcome {
    let root =
        std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_terrain(&root, ground::Terrain::Course).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "normal").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controller = crate::input::ControllerInput::default();
    let mut controls = PlayerControls::load(&root).unwrap();

    let scoop =
        skate_data::gesture_patterns::load(&root.join("private/stock/data/joystick/skater.pat"))
            .unwrap()
            .into_iter()
            .find(|p| p.name == gesture)
            .unwrap_or_else(|| panic!("authored {gesture} pattern"))
            .points;
    let sample = |p: [f32; 2]| [(p[0] * 32767.) as i16, (-p[1] * 32767.) as i16];
    let watched: Vec<(&str, _)> = WATCHED.iter().map(|n| (*n, encode(n.as_bytes()))).collect();

    let mut outcome = Outcome::default();
    for tick in 0..ticks {
        if tick == 60 {
            for body in physics.board.bodies_mut() {
                body.rates.linear_velocity = Vector3::new(0., 0., 8.);
            }
            for body in skater.skeleton.bodies_mut() {
                body.rates.linear_velocity = Vector3::new(0., 0., 8.);
            }
        }
        if tick == 123 {
            for body in physics.board.bodies_mut() {
                body.rates.linear_velocity.y += BOOST;
            }
            for body in skater.skeleton.bodies_mut() {
                body.rates.linear_velocity.y += BOOST;
            }
        }
        // Crouch on the scoop's first coordinate, then flick it: the ordinary ground pop.
        let right = if (100..112).contains(&tick) {
            sample(scoop[0])
        } else if (112..118).contains(&tick) {
            sample(scoop[scoop.len() - 1])
        } else {
            [0; 2]
        };
        // The dark button is armed before the pop and held through the air, so both `DarkCatch`
        // and its rising edge are available whenever the graph looks.
        controller.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if tick >= 95 { buttons } else { 0 },
            triggers: [0; 2],
            left: [0; 2],
            right,
        });
        let mut actions = controller.player_actions();
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
        .unwrap_or_else(|e| panic!("{gesture} tick{tick}: {e}"));

        let motion = &skater.animation.motion;
        if let Some(name) = motion.animation.current_name.as_deref()
            && outcome.animations.last().map(String::as_str) != Some(name)
        {
            outcome.animations.push(name.to_owned());
        }
        if let Some(published) = motion.score_packet.trick_names.first
            && let Some((name, _)) = watched.iter().find(|(_, e)| *e == published)
            && outcome.tricks.last().map(String::as_str) != Some(*name)
        {
            outcome.tricks.push((*name).to_owned());
        }
    }
    eprintln!(
        "{gesture} buttons={buttons:#06x}: tricks={:?}\n  animations={:?}",
        outcome.tricks, outcome.animations
    );
    outcome
}
