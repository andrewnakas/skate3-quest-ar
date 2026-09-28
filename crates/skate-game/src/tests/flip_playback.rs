//! The stock held-flip ladder and the air late flips.
//!
//! `T_Kickflip.xml` turns one authored flip into `<Trick>2`/`3`/`4` by cycling
//! `B_<TRICK>_CYC1..3`, but only while the scoop coordinate is still held *and*
//! the takeoff trajectory has the authored time left: `End.Out.Out1` claims the
//! exit at `TimeToLand < 0.525` and `Out2` at `< 0.8`, and those exits are
//! listed before the cycle transitions, so a short pop ends the ladder early.
//! `KnownAir` takes its prediction once, on entering the air, so the pop alone
//! decides how far the ladder can run. That is why these replays pop hard
//! instead of editing the world: the canonical course carries authored query
//! metadata that a rebuilt `BoardWorld` would drop.
//!
//! Every input here is a real controller sample fed through the production
//! gesture recognizer, action graph, motion graph and physics.
use super::*;
use skate_core::animation::{output::attributes::AttributeName, skeleton_input::name::encode};

/// Pop, hold the scoop, and the graph must walk the whole authored ladder.
/// The heelflip's authored clips are longer than the kickflip's, so it needs a
/// taller pop to reach the same count -- that is the authored gate, not a bug.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn held_flips_cycle_through_the_authored_double_triple_and_quad() {
    for (trick, boost) in [("Kickflip", 8.0), ("Heelflip", 10.0)] {
        let run = replay(Input::Held { trick, boost }, 260);
        for count in ["", "2", "3", "4"] {
            assert!(
                run.tricks.contains(&format!("{trick}{count}")),
                "{trick}: held ladder never published {trick}{count}; saw {:?} over {:?}",
                run.tricks,
                run.animations
            );
        }
        let cycles = run
            .animations
            .iter()
            .filter(|a| a.ends_with("_CYC1") || a.ends_with("_CYC2") || a.ends_with("_CYC3"))
            .count();
        assert_eq!(
            cycles, 3,
            "{trick}: expected the three authored cycle clips, saw {:?}",
            run.animations
        );
    }
}

/// The ladder, landed: the quad has to reach the scorer as the quad's own
/// scorable and be named by the quad's authored label, not the single's.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn landed_quads_bank_and_name_the_quad_scorable() {
    for (trick, boost, label) in [
        ("Kickflip", 8.0, "#ID_TRICK_FLIP_QUADRUPLE_KICKFLIP"),
        ("Heelflip", 10.0, "#ID_TRICK_FLIP_QUADRUPLE_HEELFLIP"),
    ] {
        let run = replay(Input::Held { trick, boost }, 300);
        assert!(
            run.tricks.contains(&format!("{trick}4")),
            "{trick}: the ladder no longer reaches the quad: {:?} over {:?}",
            run.tricks,
            run.animations
        );
        assert_eq!(run.name, label, "{trick}: the landed quad published");
        // The ladder converts rather than accumulating, so only the quad's own
        // authored 250 is banked -- never the single's 100 as well.
        assert!(
            run.reward > 250.,
            "{trick}: the landed quad banked {}",
            run.reward
        );
    }
}

/// The single flip is the baseline the ladder must not fall below. The ladder
/// *converts* its carrier (`LINKS`), so each count replaces the previous one's
/// reward instead of adding to it -- a regression there would silently make a
/// quad worth less than a single.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn a_single_flip_is_the_ladder_baseline() {
    let single = replay(
        Input::Single {
            trick: "Kickflip",
            boost: 8.0,
        },
        300,
    );
    let quad = replay(
        Input::Held {
            trick: "Kickflip",
            boost: 8.0,
        },
        300,
    );
    eprintln!("single={} quad={}", single.reward, quad.reward);
    assert_eq!(
        single.tricks,
        ["Kickflip"],
        "the released flick still cycled"
    );
    assert!(
        quad.reward >= single.reward,
        "the quad banked {} but the single banked {}",
        quad.reward,
        single.reward
    );
}

/// An ollie, then the air scoop: `air.xml`'s `Lateflip` state, whose leaves name
/// `latekickflip`/`lateheelflip` and friends. These gestures live in their own
/// recognizer (`skater_air.pat`) and never enter the 270-row trick mapping, so
/// they reach the motion graph as bare intents.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn air_scoops_reach_the_authored_late_flips() {
    for (gesture, trick, label) in [
        (
            "L_F_Kickflip",
            "LateKickflip",
            "#ID_TRICK_FLIP_LATE_KICKFLIP",
        ),
        (
            "L_F_Heelflip",
            "LateHeelflip",
            "#ID_TRICK_FLIP_LATE_HEELFLIP",
        ),
    ] {
        let run = replay(Input::Late { gesture, trick }, 260);
        assert!(
            run.tricks.contains(&trick.to_owned()),
            "{gesture}: the air scoop never published {trick}; saw {:?} over {:?}",
            run.tricks,
            run.animations
        );
        assert_eq!(run.name, label, "{gesture}: the landed late flip published");
    }
}

/// A repeated rotation must play the authored cycle clip, not the trick's own air clip.
///
/// Reported from play twice: holding a 360 flip had the back foot and hips re-popping the trick
/// every rotation, and then looked like the foot was glued to a jittery board. Both came from the
/// same mistake -- looping `360FLIP_D_HIGH_A`, which is a whole performance, and then patching its
/// seams by entering part-way and freezing the body.
///
/// Retail shipped cycle clips for these tricks and wired none of them up. `T_360FLIP_H_CYC` closes
/// its own loop to within 0.21 where the air clip is 10.56 out. Playing the authored cycle is the
/// fix, and it makes both patches unnecessary.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn a_repeated_rotation_plays_the_authored_cycle_clip() {
    for (trick, cycle) in [
        ("360Flip", "T_360FLIP_H_CYC"),
        ("Laserflip", "T_LASERFLIP_H_CYC"),
    ] {
        let stock = replay(Input::Held { trick, boost: 14.0 }, 600);
        assert!(
            !stock.animations.iter().any(|a| a == cycle),
            "{trick}: stock must never reach the cycle clip"
        );

        let endless = replay(Input::Endless { trick, boost: 14.0 }, 600);
        assert!(
            endless.animations.iter().any(|a| a == cycle),
            "{trick}: expected {cycle} among {:?}",
            endless.animations
        );
        assert!(
            endless.tricks.len() > stock.tricks.len(),
            "{trick}: the loop never ran"
        );
    }
}

/// Past the octuple the counted names run out and the trick takes the mod's own name.
///
/// The rung is pinned with the mod's own cap, so this measures the naming and nothing else --
/// anchoring it to a takeoff speed only measures how much air the test course happens to give.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn the_ninth_flip_and_beyond_is_named_endless() {
    let octuple = replay(
        Input::EndlessCapped {
            trick: "Kickflip",
            boost: 16.0,
            extra: 4,
        },
        700,
    );
    assert_eq!(octuple.tricks.len(), 8, "expected the counted ceiling");
    assert_eq!(octuple.name, "#OCTUPLE KICKFLIP");

    let ninth = replay(
        Input::EndlessCapped {
            trick: "Kickflip",
            boost: 16.0,
            extra: 5,
        },
        700,
    );
    assert_eq!(ninth.tricks.len(), 9, "expected exactly a ninth rung");
    assert_eq!(
        ninth.name, "#ENDLESS KICKFLIP",
        "the ninth rung should stop counting and take the mod's name"
    );
}

/// The loop junction must not be rougher than an authored one.
///
/// `_CYC3` is authored as the third clip of a sequence, so re-entering it cuts. Measured before
/// the fix, a skeleton part moved 0.41 m in a single tick at each junction against 0.24 m for the
/// worst authored one, once per cycle -- which read on screen as the arms snapping around. The
/// loop transition carries a `blendWithCurrentFrame` override so it cross-fades out of the pose
/// on screen instead. Comparing against the same pop with the feature off keeps this honest: the
/// takeoff is the roughest moment in any flip, and it is stock.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn the_loop_junction_is_no_rougher_than_an_authored_one() {
    let boost = 14.0;
    let worst_in_flip = |o: &Outcome| {
        o.jolts
            .iter()
            .filter(|(_, _, clip)| clip.contains("KICKFLIP"))
            .map(|(_, d, _)| *d)
            .fold(0f32, f32::max)
    };
    let stock = replay(Input::Held { trick: "Kickflip", boost }, 600);
    let endless = replay(Input::Endless { trick: "Kickflip", boost }, 600);
    assert!(endless.tricks.len() > stock.tricks.len(), "the loop never ran");

    let (a, b) = (worst_in_flip(&stock), worst_in_flip(&endless));
    assert!(
        b <= a + 0.01,
        "the endless junction is rougher than the authored worst: {b} against {a}"
    );
    let roughest_clip = endless
        .jolts
        .iter()
        .filter(|(_, _, clip)| clip.contains("KICKFLIP"))
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(_, _, clip)| clip.clone())
        .unwrap_or_default();
    assert!(
        !roughest_clip.contains("_CYC"),
        "a cycle clip is the roughest moment of the flip: {roughest_clip}"
    );
}

/// The 360 flip family laddering, which is a different shape of problem from the kickflip.
///
/// `T_TrickWithDarkCatch.xml` authors no cycle *states* and `360flip`/`laserflip` have no numbered
/// scorables, so the rungs are synthesised. Retail also scopes the hold mechanic to the kickflip
/// and heelflip, so these gestures cannot be held at all unless the mod widens it. The rungs count
/// by board rotation: a doubled 360 flip is a 720.
///
/// The expectation is derived from the rungs actually reached rather than pinned to a boost. The
/// authored cycle clip is 61 frames against the air clip's 33, so a rotation takes about twice as
/// long as it used to and how many fit in a given pop is a property of the clip, not of the
/// naming this is here to check.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn the_360_flip_family_ladders_by_rotation() {
    let expected = |rung: usize, word: &str, endless: &str| -> String {
        if rung >= 9 {
            format!("#{endless}")
        } else {
            format!("#{} {word}", rung * 360)
        }
    };
    for (trick, word, endless) in [
        ("360Flip", "FLIP", "ENDLESS 360 FLIP"),
        ("Laserflip", "LASERFLIP", "ENDLESS LASERFLIP"),
    ] {
        let stock = replay(Input::Held { trick, boost: 14.0 }, 600);
        assert_eq!(
            stock.tricks.len(),
            1,
            "retail authors exactly one rung for {trick}"
        );

        let run = replay(Input::Endless { trick, boost: 14.0 }, 600);
        assert!(
            run.tricks.len() > 1,
            "{trick}: the ladder never left the authored rung"
        );
        assert_eq!(
            run.name,
            expected(run.tricks.len(), word, endless),
            "{trick} at {} rungs",
            run.tricks.len()
        );
        assert!(
            run.reward > stock.reward,
            "{trick}: {} did not beat the single {}",
            run.reward,
            stock.reward
        );
    }
}

/// Endless Tricks, the point of the whole extension: given the air to afford them, a held flick
/// keeps cycling past the authored quad.
///
/// The authored ladder cannot produce this. `Cyc3` has no fifth cycle state and its only exit is
/// an unconditional `WillExpire` to `End.Out.Out4`, so `<Trick>3`/`<Trick>4` appearing a second
/// time in publication order is the synthesised loop and nothing else. The same pop with the
/// feature off is replayed beside it, so any difference is the feature rather than the boost.
#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn endless_flips_cycle_past_the_authored_quad() {
    // Measured window on the flat course: 11 m/s buys a fifth flip, 16 an eighth, and past
    // ~18 the skater leaves the course and never lands. 14 sits well inside it.
    let boost = 14.0;
    let stock = replay(Input::Held { trick: "Kickflip", boost }, 600);
    let endless = replay(
        Input::EndlessCapped {
            trick: "Kickflip",
            boost,
            extra: 3,
        },
        600,
    );
    assert_eq!(
        stock.tricks,
        ["Kickflip", "Kickflip2", "Kickflip3", "Kickflip4"],
        "the retail ladder stopped somewhere other than the authored quad"
    );
    assert_eq!(
        endless.tricks,
        ["Kickflip", "Kickflip2", "Kickflip3", "Kickflip4", "Kickflip5", "Kickflip6", "Kickflip7"],
        "the loop did not walk the extension rungs"
    );
    // Plain words rather than an `ID_TRICK_*` key: `apt_text::localize` echoes tokens it cannot
    // resolve, so an unknown key would reach the HUD as raw text.
    assert_eq!(endless.name, "#SEPTUPLE KICKFLIP");
    // Three rungs past the quad at the authored +50 a rung -- and the ladder still *converts*,
    // so the gain is rung 7 minus rung 4, not the sum of every rung beneath it.
    assert!(
        (endless.reward - stock.reward - 150.).abs() < 1.,
        "expected three +50 rungs over the quad's {}, got {}",
        stock.reward,
        endless.reward
    );
    // A rung the skater cannot land out of is worse than no rung at all, so the air budget has
    // to leave room for the out clip. A wipeout banks nothing and shows up here as a zero.
    assert!(
        endless.reward > 0.,
        "the endless run did not land cleanly: animations={:?}",
        endless.animations
    );
}

enum Input<'a> {
    /// Hold the ground scoop's final coordinate through the whole air.
    Held { trick: &'a str, boost: f32 },
    /// Flick and release, so the ladder ends at the single flip.
    Single { trick: &'a str, boost: f32 },
    /// Ollie, then run a `skater_air.pat` scoop once airborne.
    Late { gesture: &'a str, trick: &'a str },
    /// Endless Tricks: held exactly as `Held`, with the non-retail loop switched on. Everything
    /// else about the run is identical, so a difference in the outcome is the feature and
    /// nothing else.
    Endless { trick: &'a str, boost: f32 },
    /// The same, capped to a fixed number of extra rungs. Pinning the rung with the mod's own cap
    /// makes a naming test deterministic; anchoring it to a boost only measures how much air the
    /// test course happens to give before something else ends the run.
    EndlessCapped {
        trick: &'a str,
        boost: f32,
        extra: u32,
    },
}

struct Outcome {
    /// PROBE: mean per-tick movement of the animated body bones while airborne in a flip clip.
    body_motion: f32,
    /// PROBE: the same for the board bones, as the control -- the board must keep turning.
    board_motion: f32,
    motion_ticks: u32,
    /// Worst per-tick skeleton pose jump, as (tick, metres, clip).
    jolts: Vec<(u32, f32, String)>,
    animations: Vec<String>,
    tricks: Vec<String>,
    reward: f32,
    name: String,
}

fn replay(input: Input<'_>, ticks: u32) -> Outcome {
    let root = std::path::PathBuf::from(
        std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT"),
    );
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_terrain(&root, ground::Terrain::Course).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, "normal").unwrap();
    // Endless Tricks arrives the way a mod delivers it, through the trainer table, so the test
    // exercises the same switch the game does. Off for every other run here, which is what makes
    // those runs proof that the compiled-in loop transitions are inert.
    physics.trainer.endless_flips = matches!(
        input,
        Input::Endless { .. } | Input::EndlessCapped { .. }
    );
    if let Input::EndlessCapped { extra, .. } = input {
        physics.trainer.endless_flip_max = extra;
    }
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut controller = crate::input::ControllerInput::default();
    let mut controls = PlayerControls::load(&root).unwrap();
    // The production `sample` system copies this from the trainer each tick; the harness
    // drives `publish_gestures` directly, so it sets it here.
    controls.endless_families = physics.trainer.endless_flips;

    let hold = !matches!(input, Input::Single { .. });
    // The authored scoops, straight out of the PATs the recognizer itself loads.
    let (ground_scoop, air_scoop, boost, watched) = match input {
        Input::Held { trick, boost }
        | Input::Single { trick, boost }
        | Input::Endless { trick, boost }
        | Input::EndlessCapped { trick, boost, .. } => (
            authored("skater.pat", &root, trick),
            Vec::new(),
            boost,
            ["", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"]
                .iter()
                .map(|count| format!("{trick}{count}"))
                .collect::<Vec<_>>(),
        ),
        Input::Late { gesture, trick } => (
            authored("skater.pat", &root, "Ollie"),
            authored("skater_air.pat", &root, gesture),
            9.0,
            vec![trick.to_owned()],
        ),
    };
    let watched: Vec<(String, AttributeName)> = watched
        .into_iter()
        .map(|name| {
            let encoded = encode(name.as_bytes());
            (name, encoded)
        })
        .collect();
    // GameInputManager 82696030 negates the mapped Y before matching.
    let sample = |p: [f32; 2]| [(p[0] * 32767.) as i16, (-p[1] * 32767.) as i16];

    let mut previous_pose: Option<Vec<[f32; 3]>> = None;
    let mut previous_animated: Option<Vec<[f32; 4]>> = None;
    let mut outcome = Outcome {
        body_motion: 0.,
        board_motion: 0.,
        motion_ticks: 0,
        jolts: Vec::new(),
        animations: Vec::new(),
        tricks: Vec::new(),
        reward: 0.,
        name: String::new(),
    };
    let mut air_ticks = 0u32;
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
        if tick == 123 {
            for body in physics.board.bodies_mut() {
                body.rates.linear_velocity.y += boost;
            }
            for body in skater.skeleton.bodies_mut() {
                body.rates.linear_velocity.y += boost;
            }
        }
        let right = if air_scoop.is_empty() {
            // Crouch on the first coordinate, then hold the flick: holding the
            // final coordinate is what keeps `<Trick>Hold` published.
            if ground_scoop.len() > 2 {
                // The 360 flip and laserflip gestures are four-point arcs -- out to one side, up
                // over the top, then down across. The recognizer walks every authored coordinate,
                // so skipping the middle two never completes the pattern and the trick silently
                // never fires. Two-point scoops keep the original timing below, untouched, so the
                // kickflip and heelflip measurements stay comparable.
                let last = ground_scoop.len() - 1;
                let step = 4;
                if tick < 100 {
                    [0; 2]
                } else {
                    let index = ((tick as usize - 100) / step).min(last);
                    if index < last || hold || tick < (100 + step * last + 6) as u32 {
                        sample(ground_scoop[index])
                    } else {
                        [0; 2]
                    }
                }
            } else if (100..112).contains(&tick) {
                sample(ground_scoop[0])
            } else if tick >= 112 && (hold || tick < 118) {
                sample(ground_scoop[ground_scoop.len() - 1])
            } else {
                [0; 2]
            }
        } else if (100..112).contains(&tick) {
            sample(ground_scoop[0])
        } else if (112..118).contains(&tick) {
            sample(ground_scoop[ground_scoop.len() - 1])
        } else if air_ticks >= 10 {
            // One coordinate every four ticks, comfortably inside the authored
            // culling window, and then release so the trick can end.
            let step = (air_ticks as usize - 10) / 4;
            air_scoop.get(step).copied().map(sample).unwrap_or([0; 2])
        } else {
            [0; 2]
        };
        controller.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: 0,
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
        .unwrap();
        if skater.player_state.current().category() == 200 {
            air_ticks += 1;
        }

        // PROBE: the animated pose itself, split the way the rig is -- body bones against board
        // bones. A working hold shows near-zero body movement while the board keeps turning.
        {
            let clip = skater
                .animation
                .motion
                .animation
                .current_name
                .clone()
                .unwrap_or_default();
            if clip.contains("FLIP_A") || clip.contains("_CYC") {
                let pose = &skater.animation.pose;
                let bones = skater.animation.evaluator.body_bones();
                // Bone rotations carry the motion in a skeletal rig; translations are lengths.
                let sample: Vec<[f32; 4]> = pose.iter().map(|p| p.rotation).collect();
                if let Some(prev) = &previous_animated {
                    let delta = |i: usize| -> f32 {
                        let (a, b) = (&sample[i], &prev[i]);
                        (0..4).map(|k| (a[k] - b[k]).abs()).sum::<f32>()
                    };
                    let body: f32 = bones.iter().map(|&b| delta(b)).sum();
                    let board: f32 = (0..sample.len())
                        .filter(|i| !bones.contains(i))
                        .map(delta)
                        .sum();
                    outcome.body_motion += body;
                    outcome.board_motion += board;
                    outcome.motion_ticks += 1;
                }
                previous_animated = Some(sample);
            }
        }

        // PROBE: every skeleton part relative to the root, so bulk travel cancels and only
        // the pose change remains. A blend discontinuity shows up as a one-tick spike.
        {
            let bodies = skater.skeleton.bodies();
            let root = bodies[0].rates.position;
            let pose: Vec<[f32; 3]> = bodies
                .iter()
                .map(|b| {
                    let p = b.rates.position;
                    [p.x - root.x, p.y - root.y, p.z - root.z]
                })
                .collect();
            if let Some(prev) = &previous_pose {
                let worst = pose
                    .iter()
                    .zip(prev)
                    .map(|(a, b)| {
                        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                    })
                    .fold(0f32, f32::max);
                let clip = skater
                    .animation
                    .motion
                    .animation
                    .current_name
                    .clone()
                    .unwrap_or_default();
                outcome.jolts.push((tick, worst, clip));
            }
            previous_pose = Some(pose);
        }

        let motion = &skater.animation.motion;
        if let Some(name) = motion.animation.current_name.as_deref() {
            if outcome.animations.last().map(String::as_str) != Some(name) {
                outcome.animations.push(name.to_owned());
            }
        }
        if let Some(published) = motion.score_packet.trick_names.first {
            if let Some((name, _)) = watched.iter().find(|(_, e)| *e == published) {
                if outcome.tricks.last() != Some(name) {
                    outcome.tricks.push(name.clone());
                }
            }
        }
        outcome.reward = skater.scoring.session.holder.snapshot.last_reward;
        let published = skater.scoring.hud_input().trick_name;
        if !published.is_empty() && published != "#ID_TRICK_AIR" {
            outcome.name = published;
        }
    }
    eprintln!(
        "{:?}: tricks={:?} reward={} name={:?}\n  animations={:?}",
        watched.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        outcome.tricks,
        outcome.reward,
        outcome.name,
        outcome.animations
    );
    outcome
}

fn authored(file: &str, root: &std::path::Path, name: &str) -> Vec<[f32; 2]> {
    skate_data::gesture_patterns::load(&root.join("private/stock/data/joystick").join(file))
        .unwrap()
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("authored {name} pattern in {file}"))
        .points
}
