//! The ground plant family: bonelesses, fastplants, the no-comply and the hippy jump.
//!
//! All four `PushTrick` children (`Motion.OnBoard.Trick.PushTrick.*`) are entered from the
//! ground rather than from the air, and `docs/plants.md` records the controller recipes the
//! stock graphs accept. A ground grab is stricter than an air grab: `trick_intentions::produce`
//! advances the ground timers only when the trigger action reads *exactly* 1.0, so a partial
//! analog value is not a held grab.
use super::*;
use skate_core::animation::skeleton_input::name::encode;

/// Hold a grab trigger fully, then press a push button. Which trigger and which push foot select
/// the stock variant; `docs/plants.md` has the mapping.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn held_ground_grab_plus_a_push_reaches_the_plant_family() {
    let mut seen: Vec<String> = Vec::new();
    for hand in 0..2 {
        for button in [A, X] {
            let run = replay(Some(hand), button, None, 300);
            eprintln!("hand={hand} button={button:#06x} -> {:?}", run.tricks);
            seen.extend(run.tricks);
        }
    }
    for expected in ["fsboneless", "bsboneless", "fsfastplant", "bsfastplant"] {
        assert!(
            seen.iter().any(|t| t == expected),
            "{expected} was never published across the four trigger/push combinations; saw {seen:?}"
        );
    }
}

/// Both pushes at once, which `ActionGraphIncludes/onground.xml`'s `HippyJumping` is gated on.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn both_pushes_reach_the_hippy_jump() {
    let run = replay(None, A | X, None, 300);
    assert!(
        run.tricks.iter().any(|t| t == "hippyjump"),
        "hippyjump was never published; saw {:?} over {:?}",
        run.tricks,
        run.animations
    );
}

// The no-comply is deliberately not covered here yet. `T_PushTrick.xml`'s `PlantingFoot` wants
// two MotionGraph intents together -- `NewFootPlant` *and* a `NoComply` trick intent -- and a
// push plus an ollie scoop produces `Ollie`, not `NoComply`, however the two are spaced: the
// push enters (`B_PUSH_INTO`) and the scoop then pops an ordinary ollie straight over the top of
// it. Which gesture group `CreateTrickIntentFromGesture` is meant to resolve under a planted
// push foot is the open question; see docs/trick-input-recipes.md.

/// Xbox face buttons as `XboxState::buttons` bits: A is `RightPush`, X is `LeftPush`.
const A: u16 = 0x1000;
const X: u16 = 0x4000;

/// Tick the push arrives on, after the roll-in has settled and any grab is already held.
const PUSH: u32 = 150;
/// Tick the grab trigger goes down, well before the push.
const GRAB: u32 = 120;
/// Ticks the push is held before release. The hippy jump is a *charge*: its antic cycles for as
/// long as both pushes are down (`T_HIPPYJUMP_ANTIC_CYC`) and only launches on the release, so a
/// button held for the whole run never leaves the anticipation.
const PUSH_HOLD: u32 = 40;
/// Ticks after the push before the scoop. The no-comply wants the flick *during push entry*, so
/// a scoop thrown on the same tick as the button simply pops an ordinary ollie first.
const SCOOP_DELAY: u32 = 12;

#[derive(Default)]
struct Outcome {
    animations: Vec<String>,
    tricks: Vec<String>,
}

const WATCHED: [&str; 7] = [
    "fsboneless",
    "bsboneless",
    "fsfastplant",
    "bsfastplant",
    "nocomply",
    "hippyjump",
    "ollie",
];

fn replay(hand: Option<usize>, buttons: u16, scoop: Option<&str>, ticks: u32) -> Outcome {
    let root =
        std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_terrain(&root, ground::Terrain::Course).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "normal").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controller = crate::input::ControllerInput::default();
    let mut controls = PlayerControls::load(&root).unwrap();

    let scoop: Vec<[f32; 2]> = scoop
        .map(|name| {
            skate_data::gesture_patterns::load(&root.join("private/stock/data/joystick/skater.pat"))
                .unwrap()
                .into_iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("authored {name} pattern"))
                .points
        })
        .unwrap_or_default();
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
        // 255 is exactly 1.0 after conversion, which is what a *ground* grab requires.
        let triggers = match hand {
            Some(hand) if tick >= GRAB => {
                let mut t = [0u8; 2];
                t[hand] = 255;
                t
            }
            _ => [0; 2],
        };
        // The scoop rides on top of the push, during its entry.
        let right = if scoop.is_empty() || tick < PUSH + SCOOP_DELAY {
            [0; 2]
        } else {
            let step = (tick - PUSH - SCOOP_DELAY) as usize / 3;
            scoop.get(step).copied().map(sample).unwrap_or([0; 2])
        };
        controller.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if (PUSH..PUSH + PUSH_HOLD).contains(&tick) {
                buttons
            } else {
                0
            },
            triggers,
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
        .unwrap_or_else(|e| panic!("tick{tick}: {e}"));

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
        "hand={hand:?} buttons={buttons:#06x}: tricks={:?}\n  animations={:?}",
        outcome.tricks, outcome.animations
    );
    outcome
}
