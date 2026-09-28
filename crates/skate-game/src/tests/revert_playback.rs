//! Reverts, which are neither a flip nor a grab and are not named by any scoring leaf.
//!
//! `MotionGraphIncludes/revert.xml`'s `Fs`/`Bs` children are gated on `SlideFs180` / `SlideBs180`
//! -- left-stick patterns from `skaterls.pat`, not right-stick trick scoops -- and each carries
//! `<behaviour name="SetScoreAugmentation" augment="FSRevert" mirrorAugment="BSRevert"/>`.
//!
//! That augmentation is the whole crediting path. `ScorePacket::set` turns the augmentation
//! index into a flag bit (`FSRevert` is bit 29, `BSRevert` bit 28) and `scoring_runtime` reads
//! those two bits straight into `revert_id` 4 and 5. No `ScoringTrick` leaf ever spells
//! `fsrevert`, which is why a name-only reachability audit reports both as unreachable.
use super::*;
/// Tick the left stick first deflects on, well after the roll-in has settled.
const ENTRY: u32 = 120;
/// Ticks holding the pattern's entry coordinate before the sweep; see the note in `replay`.
const HOLD: u32 = 30;

/// Both directions reach their authored `Revert` child and publish its score augmentation.
///
/// The assertion is the augmentation bit and the authored clip, not a ledger credit. A revert
/// extends a line rather than opening one, so thrown on its own -- with no trick before it --
/// nothing is banked and the snapshot stays at zero. That is retail behaviour, not a gap: the
/// ledger side of the path is already exercised live, `fsrevert` having been credited in a
/// recorded session (see `docs/trick-reachability.md`).
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn left_stick_slides_reach_both_reverts() {
    for (gesture, bit, clip) in [
        ("SlideFs180", 0x2000_0000u32, "B_FS_REVERT"),
        ("SlideBs180", 0x1000_0000u32, "B_BS_REVERT"),
    ] {
        let run = replay(gesture, 320);
        assert!(
            run.augmentation & bit != 0,
            "{gesture}: the graph never published the augmentation; flags={:08x} over {:?}",
            run.augmentation,
            run.animations
        );
        assert!(
            run.animations.iter().any(|a| a == clip),
            "{gesture}: the augmentation fired without the authored clip {clip}; saw {:?}",
            run.animations
        );
    }
}

struct Outcome {
    animations: Vec<String>,
    augmentation: u32,
}

fn replay(gesture: &str, ticks: u32) -> Outcome {
    let root =
        std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_terrain(&root, ground::Terrain::Course).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "normal").unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controller = crate::input::ControllerInput::default();
    let mut controls = PlayerControls::load(&root).unwrap();

    // The slides live on the left stick, in their own recognizer.
    let scoop =
        skate_data::gesture_patterns::load(&root.join("private/stock/data/joystick/skaterls.pat"))
            .unwrap()
            .into_iter()
            .find(|p| p.name.eq_ignore_ascii_case(gesture))
            .unwrap_or_else(|| panic!("authored {gesture} pattern in skaterls.pat"))
            .points;
    let sample = |p: [f32; 2]| [(p[0] * 32767.) as i16, (-p[1] * 32767.) as i16];

    let mut outcome = Outcome {
        animations: Vec::new(),
        augmentation: 0,
    };
    for tick in 0..ticks {
        if tick == 60 {
            // A rolling approach, without a steering input that would also spin.
            for body in physics.board.bodies_mut() {
                body.rates.linear_velocity = Vector3::new(0., 0., 8.);
            }
            for body in skater.skeleton.bodies_mut() {
                body.rates.linear_velocity = Vector3::new(0., 0., 8.);
            }
        }
        // Hold the pattern's entry coordinate, then sweep the rest one coordinate per tick.
        //
        // Both halves matter. Sweeping from a centred stick never produces a `SlideFs180` at
        // all: `BackFlip`'s first authored coordinate sits within tolerance of centre, so idling
        // has already advanced its node, and the arc's second point completes it one tick before
        // the slide's third. Only one pattern wins a file, `permitted` then drops `BackFlip`
        // because no grab is held, and the file yields nothing. Entering from the deflected
        // stick leaves `BackFlip` parked at its first coordinate -- which is also how the trick
        // is really thrown, out of a turn or a powerslide, per `ground.xml`'s transitions into
        // `Revert`.
        //
        // The sweep itself has to be one coordinate per tick: the recognizer culls a node after
        // `NumTicksPatternNotInRangeBeforeCulling` ticks not yet within tolerance of the next
        // point, and that budget is read per stick, the left stick's being the tighter.
        let left = if tick >= ENTRY {
            let step = (tick - ENTRY).saturating_sub(HOLD) as usize;
            scoop.get(step).copied().map(sample).unwrap_or([0; 2])
        } else {
            [0; 2]
        };
        controller.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: 0,
            triggers: [0; 2],
            left,
            right: [0; 2],
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

        if std::env::var_os("TRACE_REVERT").is_some() && (148..175).contains(&tick) {
            eprintln!(
                "tick{tick} left={left:?} packet={:08x} line={:.2} acc={:.2} last={:.2} anim={:?}",
                skater.animation.motion.score_packet.flags,
                skater.scoring.session.holder.snapshot.line,
                skater.scoring.session.holder.snapshot.accumulated,
                skater.scoring.session.holder.snapshot.last_reward,
                skater.animation.motion.animation.current_name
            );
        }
        outcome.augmentation |= skater.animation.motion.score_packet.flags;
        if let Some(name) = skater.animation.motion.animation.current_name.as_deref()
            && outcome.animations.last().map(String::as_str) != Some(name)
        {
            outcome.animations.push(name.to_owned());
        }
    }
    eprintln!(
        "{gesture}: flags={:08x}\n  animations={:?}",
        outcome.augmentation, outcome.animations
    );
    outcome
}
