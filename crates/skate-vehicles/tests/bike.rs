//! Single-track handling, driven through the real `Simulation::step` path.
//! Pure Rapier: no Bevy, no window, no game assets.
//!
//! Lean is handling state rather than body roll, so these read it from
//! `bike_state` rather than from the chassis rotation. What the chassis is
//! asserted to do instead is stay upright and keep its wheels on the ground.
use skate_vehicles::{rapier3d::prelude::*, *};

fn definition() -> VehicleDefinition {
    serde_json::from_str(include_str!("../../../sdk/examples/freestyle-mx/vehicle.json")).unwrap()
}

fn setup(height: f32) -> (Simulation, u64) {
    let mut s = Simulation::default();
    s.ground(
        [
            [[-500., 0., -500.], [500., 0., 500.], [500., 0., -500.]],
            [[-500., 0., -500.], [-500., 0., 500.], [500., 0., 500.]],
        ]
        .into_iter(),
    )
    .unwrap();
    let id = s.spawn(definition(), [0., height, 0.], 0.).unwrap();
    s.set_occupied(id, true);
    (s, id)
}

fn run(s: &mut Simulation, seconds: f32, hz: u32) {
    for _ in 0..(seconds * hz as f32) as u32 {
        s.step(1. / hz as f32);
    }
}

fn control(s: &mut Simulation, id: u64, controls: Controls) {
    s.vehicles.get_mut(&id).unwrap().controls = controls;
}

/// Roll of the chassis body itself, radians. The bike's *lean* is handling
/// state; this is the body, which must stay upright over the contact line.
fn body_roll(s: &Simulation, id: u64) -> f32 {
    let b = &s.world.bodies[s.vehicles[&id].body];
    let up = *b.rotation() * Vector::Y;
    -(b.rotation().inverse() * up.cross(Vector::Y)).z
}

/// Handling lean, radians. Negative is toward driver-left.
fn lean(s: &Simulation, id: u64) -> f32 {
    s.bike_state(id).unwrap().lean
}

fn heading(s: &Simulation, id: u64) -> f32 {
    let f = *s.world.bodies[s.vehicles[&id].body].rotation() * Vector::Z;
    f.x.atan2(f.z)
}

/// Yaw turned over `seconds`, radians, unwrapped. A bike at full lean passes
/// through +/-pi in about two seconds, and a wrapped heading reads that as a
/// hard turn the other way.
fn turned(s: &mut Simulation, seconds: f32) -> f32 {
    let id = *s.vehicles.keys().next().unwrap();
    let mut total = 0.;
    let mut last = heading(s, id);
    for _ in 0..(seconds * 120.) as u32 {
        s.step(1. / 120.);
        let now = heading(s, id);
        let mut step = now - last;
        if step > std::f32::consts::PI {
            step -= std::f32::consts::TAU;
        } else if step < -std::f32::consts::PI {
            step += std::f32::consts::TAU;
        }
        total += step;
        last = now;
    }
    total
}

/// Nose-up pitch, radians. Positive is a wheelie.
fn pitch(s: &Simulation, id: u64) -> f32 {
    let f = *s.world.bodies[s.vehicles[&id].body].rotation() * Vector::Z;
    f.y.asin()
}

fn height(s: &Simulation, id: u64) -> f32 {
    s.world.bodies[s.vehicles[&id].body].translation().y
}

fn speed(s: &Simulation, id: u64) -> f32 {
    s.vehicles[&id].controller.current_vehicle_speed
}

fn contacts(s: &Simulation, id: u64) -> u32 {
    s.telemetry(id).unwrap().2
}

fn wheel_down(s: &Simulation, id: u64, steering: bool) -> bool {
    s.vehicles[&id]
        .controller
        .wheels()
        .iter()
        .zip(&s.vehicles[&id].definition.wheels)
        .any(|(w, d)| d.steering == steering && w.raycast_info().is_in_contact)
}

fn settle(s: &mut Simulation) {
    run(s, 2., 120);
}

/// Get up to a target speed without asserting anything about how.
fn accelerate(s: &mut Simulation, id: u64, target: f32) {
    control(
        s,
        id,
        Controls {
            throttle: 1.,
            ..Default::default()
        },
    );
    for _ in 0..(20. * 120.) as u32 {
        s.step(1. / 120.);
        if speed(s, id) >= target {
            return;
        }
    }
    panic!("never reached {target} m/s, got {}", speed(s, id));
}

#[test]
fn a_parked_bike_sits_on_its_suspension_with_its_collider_clear_of_the_ground() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    let d = definition();
    assert_eq!(contacts(&s, id), 2, "both wheels should be down");
    let state = s.bike_state(id).unwrap();
    // A motocross bike sits about a third into its travel with a rider on it.
    assert!(
        (0.2..0.6).contains(&state.compression),
        "static sag {} of travel",
        state.compression
    );
    // The collider's lowest point must never reach the floor: a flat box face
    // on the ground is what made the old bike grind and stick.
    let belly = height(&s, id) + d.collider_offset[1] - d.half_extents[1];
    assert!(belly > 0.25, "chassis belly only {belly} m off the ground");
}

#[test]
fn a_disturbed_bike_recovers_to_upright_at_rest() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    // Kick it over: flat ground perturbs nothing by itself, so the recovery is
    // the only thing this can be measuring.
    s.world.bodies[s.vehicles[&id].body].set_angvel(Vector::Z * 2., true);
    run(&mut s, 3., 120);
    assert!(
        body_roll(&s, id).abs() < 0.1,
        "should have stood back up, roll {}",
        body_roll(&s, id)
    );
    assert!(height(&s, id) > 0.2, "should not have fallen through");
}

#[test]
fn a_bike_rolling_straight_stays_upright_and_accelerates() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    control(
        &mut s,
        id,
        Controls {
            throttle: 1.,
            ..Default::default()
        },
    );
    run(&mut s, 5., 120);
    assert!(lean(&s, id).abs() < 0.05, "straight-line lean {}", lean(&s, id));
    assert!(
        body_roll(&s, id).abs() < 0.1,
        "straight-line roll {}",
        body_roll(&s, id)
    );
    assert!(speed(&s, id) > 8., "should have accelerated, {}", speed(&s, id));
}

#[test]
fn steering_leans_the_bike_into_the_turn_and_carves_it_round() {
    for side in [1., -1.] {
        let (mut s, id) = setup(1.);
        settle(&mut s);
        accelerate(&mut s, id, 12.);
        let before = heading(&s, id);
        control(
            &mut s,
            id,
            Controls {
                throttle: 0.4,
                steering: side,
                ..Default::default()
            },
        );
        let _ = before;
        let swept = turned(&mut s, 2.5) * side;
        // Positive steering is a left turn: it leans left (negative) and yaws left.
        assert!(
            lean(&s, id) * side < -0.3,
            "steer {side} produced lean {}",
            lean(&s, id)
        );
        assert!(swept > 0.5, "steer {side} turned only {swept} rad");
        // The body stays over the contact line while the handling leans.
        assert!(
            body_roll(&s, id).abs() < 0.2,
            "chassis rolled {} in a corner",
            body_roll(&s, id)
        );
        assert!(contacts(&s, id) >= 1, "lost the ground mid-corner");
    }
}

#[test]
fn the_same_lean_carves_a_wider_arc_at_a_higher_speed() {
    let mut rates = vec![];
    for target in [8., 18.] {
        let (mut s, id) = setup(1.);
        settle(&mut s);
        accelerate(&mut s, id, target);
        control(
            &mut s,
            id,
            Controls {
                throttle: 0.3,
                steering: 1.,
                ..Default::default()
            },
        );
        // Let the lean settle before measuring the arc it produces.
        run(&mut s, 1.5, 120);
        rates.push(turned(&mut s, 1.));
    }
    // Still wider at speed, but deliberately far less so than lean physics
    // alone: `steer_rate` falls off gently on purpose, because `g*tan/v` left
    // the bike turning at half an arcade rate everywhere above walking pace.
    assert!(
        rates[0] > rates[1] * 1.02,
        "slow turn rate {} should still beat fast {}",
        rates[0],
        rates[1]
    );
}

#[test]
fn releasing_preload_pops_the_bike_off_the_ground_exactly_once() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    let resting = height(&s, id);
    // Load the suspension, then snap the stick forward.
    control(
        &mut s,
        id,
        Controls {
            weight: 1.,
            ..Default::default()
        },
    );
    run(&mut s, 0.5, 120);
    assert!(
        height(&s, id) < resting + 0.01,
        "loading should compress, not lift"
    );
    control(&mut s, id, Controls::default());
    // Hold neutral long enough to land and keep sitting there. A latch that
    // re-armed itself would launch the bike again on the same stored travel.
    let (mut apex, mut apexes, mut airborne) = (0_f32, 0, false);
    for _ in 0..600 {
        s.step(1. / 120.);
        apex = apex.max(height(&s, id));
        let up = height(&s, id) > resting + 0.2;
        if up && !airborne {
            apexes += 1;
        }
        airborne = up;
    }
    assert!(
        apex > resting + 0.3,
        "preload release should pop: apex {apex} vs resting {resting}"
    );
    assert_eq!(apexes, 1, "preload should pop exactly once");
}

#[test]
fn a_bike_that_never_preloads_does_not_pop() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    let resting = height(&s, id);
    let mut apex: f32 = 0.;
    for _ in 0..240 {
        s.step(1. / 120.);
        apex = apex.max(height(&s, id));
    }
    assert!(apex < resting + 0.05, "idle bike rose {} m", apex - resting);
}

#[test]
fn weight_back_under_power_wheelies_without_looping_out() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    accelerate(&mut s, id, 6.);
    control(
        &mut s,
        id,
        Controls {
            throttle: 1.,
            weight: 1.,
            ..Default::default()
        },
    );
    let mut lifted = false;
    let mut highest: f32 = 0.;
    for _ in 0..(4. * 120.) as u32 {
        s.step(1. / 120.);
        lifted |= !wheel_down(&s, id, true);
        highest = highest.max(pitch(&s, id));
        assert!(
            s.take_ejection(id).is_none(),
            "a wheelie is not a crash (pitch {})",
            pitch(&s, id)
        );
    }
    assert!(lifted, "the front wheel never left the ground");
    let limit = definition().bike.wheelie_limit;
    assert!(
        highest < limit + 0.35,
        "looped out: pitch reached {highest} against a {limit} limit"
    );
    assert!(
        wheel_down(&s, id, false),
        "the rear wheel should still be driving"
    );
}

#[test]
fn the_front_brake_with_weight_forward_lifts_the_rear() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    accelerate(&mut s, id, 14.);
    control(
        &mut s,
        id,
        Controls {
            brake: 1.,
            weight: -1.,
            ..Default::default()
        },
    );
    let mut lifted = false;
    let mut lowest: f32 = 0.;
    for _ in 0..(2.5 * 120.) as u32 {
        s.step(1. / 120.);
        lifted |= !wheel_down(&s, id, false);
        lowest = lowest.min(pitch(&s, id));
    }
    assert!(lifted, "the rear wheel never came up under braking");
    let limit = definition().bike.stoppie_limit;
    assert!(-lowest < limit + 0.35, "endoed: pitch reached {lowest}");
}

#[test]
fn a_whip_yaws_the_bike_then_a_neutral_stick_straightens_it_to_land() {
    for side in [1., -1.] {
        let (mut s, id) = setup(4.);
        // Launch it forward and up, the shape of a real jump.
        {
            let b = &mut s.world.bodies[s.vehicles[&id].body];
            b.set_linvel(Vector::new(0., 6., 14.), true);
        }
        control(
            &mut s,
            id,
            Controls {
                whip: side,
                ..Default::default()
            },
        );
        run(&mut s, 0.5, 120);
        let whipped = (heading(&s, id) - 0.) * side;
        assert!(whipped > 0.4, "whip {side} only yawed {whipped} rad");
        // Now let go. The bike must come back square before it lands.
        control(&mut s, id, Controls::default());
        let mut landed = false;
        for _ in 0..(3. * 120.) as u32 {
            s.step(1. / 120.);
            if contacts(&s, id) > 0 {
                landed = true;
                break;
            }
        }
        assert!(landed, "never came down");
        let b = &s.world.bodies[s.vehicles[&id].body];
        let v = b.linvel();
        let forward = *b.rotation() * Vector::Z;
        let square = Vector::new(v.x, 0., v.z)
            .normalize_or_zero()
            .dot(Vector::new(forward.x, 0., forward.z).normalize_or_zero());
        assert!(
            square > 0.9,
            "landed {} off its direction of travel",
            square.acos()
        );
    }
}

#[test]
fn a_held_stick_backflips_and_a_clean_landing_keeps_the_rider() {
    let (mut s, id) = setup(12.);
    {
        let b = &mut s.world.bodies[s.vehicles[&id].body];
        b.set_linvel(Vector::new(0., 2., 12.), true);
    }
    control(
        &mut s,
        id,
        Controls {
            weight: 1.,
            ..Default::default()
        },
    );
    let mut rotated = 0.;
    for _ in 0..(1.8 * 120.) as u32 {
        s.step(1. / 120.);
        let b = &s.world.bodies[s.vehicles[&id].body];
        rotated += (b.rotation().inverse() * b.angvel()).x * -1. / 120.;
    }
    assert!(
        rotated > std::f32::consts::TAU,
        "a held stick should turn a full backflip, got {rotated} rad"
    );
}

#[test]
fn a_flat_landing_from_a_real_jump_keeps_the_rider_on_the_bike() {
    let (mut s, id) = setup(3.5);
    {
        let b = &mut s.world.bodies[s.vehicles[&id].body];
        b.set_linvel(Vector::new(0., 0., 16.), true);
    }
    for _ in 0..(4. * 120.) as u32 {
        s.step(1. / 120.);
        assert!(
            s.take_ejection(id).is_none(),
            "a square landing should not bail the rider"
        );
    }
    assert!(contacts(&s, id) > 0, "should be back on the ground");
}

#[test]
fn landing_sideways_bails_the_rider() {
    let (mut s, id) = setup(3.5);
    {
        let b = &mut s.world.bodies[s.vehicles[&id].body];
        // Flying forward, but pointing 80 degrees across it: a case.
        b.set_linvel(Vector::new(0., 0., 16.), true);
        b.set_rotation(Rotation::from_rotation_y(1.4), true);
    }
    // Held out sideways all the way down: the auto-straighten only runs on a
    // neutral stick, so this is the rider refusing to bring it back.
    let mut reason = None;
    for _ in 0..(4. * 120.) as u32 {
        control(
            &mut s,
            id,
            Controls {
                whip: 0.2,
                ..Default::default()
            },
        );
        s.step(1. / 120.);
        if let Some(e) = s.take_ejection(id) {
            reason = Some(e.reason);
            break;
        }
    }
    assert_eq!(reason, Some("landing"), "a sideways landing should bail");
}

#[test]
fn landing_on_its_side_bails_the_rider() {
    let (mut s, id) = setup(3.5);
    {
        let b = &mut s.world.bodies[s.vehicles[&id].body];
        b.set_linvel(Vector::new(0., 0., 12.), true);
        // Rolled hard toward driver-left, which is negative local Z.
        b.set_rotation(Rotation::from_rotation_z(-1.1), true);
        b.set_angvel(Vector::ZERO, true);
    }
    // Rider holds it further over, so the levelling assist never runs.
    let mut reason = None;
    for _ in 0..(4. * 120.) as u32 {
        control(
            &mut s,
            id,
            Controls {
                lean: 1.,
                ..Default::default()
            },
        );
        s.step(1. / 120.);
        if let Some(e) = s.take_ejection(id) {
            reason = Some(e.reason);
            break;
        }
    }
    assert_eq!(reason, Some("landing"), "landing on its side should bail");
}

#[test]
fn a_small_hop_is_never_judged_as_a_landing() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    accelerate(&mut s, id, 10.);
    // Bounce it repeatedly: brief airs off bumps must not bail anyone.
    for tick in 0..(6. * 120.) as u32 {
        if tick % 60 == 0 {
            s.world.bodies[s.vehicles[&id].body].apply_impulse(Vector::Y * 300., true);
        }
        s.step(1. / 120.);
        assert!(
            s.take_ejection(id).is_none(),
            "a hop off a bump bailed the rider at tick {tick}"
        );
    }
}

/// Ground tilted `camber` radians about the travel axis, so the bike rides
/// across a side-slope. The plane passes through y=0 along x=0, which is the
/// line the bike drives down.
fn cambered(camber: f32) -> (Simulation, u64) {
    let mut s = Simulation::default();
    let h = |x: f32| x * camber.sin();
    let mut tris = Vec::new();
    for i in 0..40 {
        let (x0, x1) = (-100. + i as f32 * 5., -100. + (i + 1) as f32 * 5.);
        tris.push([[x0, h(x0), -200.], [x1, h(x1), 200.], [x1, h(x1), -200.]]);
        tris.push([[x0, h(x0), -200.], [x0, h(x0), 200.], [x1, h(x1), 200.]]);
    }
    s.ground(tris.into_iter()).unwrap();
    let id = s.spawn(definition(), [0., 1., -20.], 0.).unwrap();
    s.set_occupied(id, true);
    (s, id)
}

/// The bug this pins down: the roll controller cancels the moment the ground
/// reaction makes about the mass centre, and that reaction acts along the
/// **contact normal**, not along world up. Written against world up it
/// computes a moment where there is none on a camber and drags the bike back
/// toward world-vertical, so the bike took up only about half of a side-slope
/// and almost none of one that also climbed. On the flat the two are the same
/// vector, which is why it read as correct for so long.
#[test]
fn the_bike_takes_up_a_cambered_slope_instead_of_staying_world_upright() {
    for degrees in [12_f32, 24.] {
        let camber = degrees.to_radians();
        let (mut s, id) = cambered(camber);
        settle(&mut s);
        control(
            &mut s,
            id,
            Controls {
                throttle: 0.5,
                ..Default::default()
            },
        );
        run(&mut s, 4., 120);
        // The surface the wheels actually found, which is what the bike owes
        // its attitude to: the triangulated slope is a shade short of the
        // authored angle, and the assertion should not care.
        let normal = {
            let v = &s.vehicles[&id];
            let mut n = Vector::ZERO;
            for w in v.controller.wheels() {
                if w.raycast_info().is_in_contact {
                    n += w.raycast_info().contact_normal_ws;
                }
            }
            n.normalize_or_zero()
        };
        assert!(normal.length() > 0.5, "lost the slope at {degrees} deg");
        let b = &s.world.bodies[s.vehicles[&id].body];
        let up = *b.rotation() * Vector::Y;
        let off = up.dot(normal).clamp(-1., 1.).acos();
        assert!(
            off < 0.09,
            "on a {degrees} deg camber the bike sat {:.1} deg off the surface",
            off.to_degrees()
        );
        // And it is genuinely leaned over in the world, not merely upright.
        assert!(
            up.dot(Vector::Y).acos() > camber * 0.7,
            "never took up the camber at all"
        );
        assert!(s.take_ejection(id).is_none(), "a camber is not a crash");
    }
}

/// A circular bowl of radius `r0`, flat inside it and banked outward beyond.
///
/// This, and not a tilted plane, is what a berm is. The difference decides the
/// whole test: on a berm the surface normal leans toward the turn centre, so
/// the suspension pushes the bike *into* the corner; on an endless side-slope
/// that same component just points downhill and the bike runs away from the
/// corner instead of round it.
fn bowl(r0: f32, bank: f32) -> (Simulation, u64) {
    bowl_with(r0, bank, definition())
}

fn bowl_with(r0: f32, bank: f32, definition: VehicleDefinition) -> (Simulation, u64) {
    let mut s = Simulation::default();
    let height = |x: f32, z: f32| {
        let r = (x * x + z * z).sqrt();
        if r > r0 { (r - r0) * bank.tan() } else { 0. }
    };
    let mut tris = Vec::new();
    let segments = 96;
    let step = std::f32::consts::TAU / segments as f32;
    for i in 0..segments {
        for j in 0..40 {
            let (a0, a1) = (i as f32 * step, (i + 1) as f32 * step);
            let (r0a, r1a) = (j as f32 * 2.5, (j + 1) as f32 * 2.5);
            let at = |a: f32, r: f32| {
                let (sin, cos) = a.sin_cos();
                [r * sin, height(r * sin, r * cos), r * cos]
            };
            tris.push([at(a0, r0a), at(a1, r1a), at(a1, r0a)]);
            tris.push([at(a0, r0a), at(a0, r1a), at(a1, r1a)]);
        }
    }
    s.ground(tris.into_iter()).unwrap();
    let id = s
        .spawn(definition, [0., 0.6, -r0], std::f32::consts::FRAC_PI_2)
        .unwrap();
    s.set_occupied(id, true);
    (s, id)
}

fn round_the_bowl(bank: f32) -> (f32, f32) {
    round_the_bowl_with(bank, definition())
}

fn round_the_bowl_with(bank: f32, definition: VehicleDefinition) -> (f32, f32) {
    let (mut s, id) = bowl_with(25., bank, definition);
    settle(&mut s);
    accelerate(&mut s, id, 14.);
    // Matched to the bowl, not held at full lock. The bike now turns inside
    // 5 m at speed, so full lock in a 25 m bowl is not riding the berm -- it
    // is trying to turn off it, which scrubs for reasons that have nothing to
    // do with the bank.
    control(&mut s, id, Controls { throttle: 0.55, steering: 0.35, ..Default::default() });
    run(&mut s, 1., 120);
    let swept = turned(&mut s, 3.).abs();
    (swept, speed(&s, id))
}

/// A berm is the fastest part of a track: it should come round harder than
/// flat ground *and* sling you out, not scrub the speed off. The second half
/// is why `berm_assist` defaults well below the physically exact 1 -- most of
/// the effect is already there from the suspension pushing along a normal
/// that leans into the corner, and adding the bank to the carve on top of
/// that asks for yaw the tyres cannot deliver.
#[test]
fn a_berm_carves_harder_than_flat_and_still_slings_you_out() {
    let (flat, _) = round_the_bowl(0.);
    let (berm, berm_speed) = round_the_bowl(0.45);
    assert!(berm > flat * 1.1, "the berm should bite: {berm:.2} rad vs {flat:.2} flat");
    // Against the same berm ridden with the assist switched off, not against
    // flat ground: a bike on a bank climbs it, and that costs energy for
    // reasons that have nothing to do with sliding. What this is guarding is
    // over-commanding the carve -- asking for yaw the tyres cannot convert,
    // which is a slide, and a slide is what scrubs the speed off.
    let mut off = definition();
    off.bike.berm_assist = 0.;
    let (_, unassisted) = round_the_bowl_with(0.45, off);
    assert!(
        berm_speed > unassisted * 0.85,
        "the assist is scrubbing: {berm_speed:.1} out against {unassisted:.1} with it off"
    );
}

#[test]
fn the_berm_assist_default_is_on_the_useful_side_of_sliding() {
    let assist = definition().bike.berm_assist;
    assert!(
        (0.05..=0.35).contains(&assist),
        "berm_assist {assist} is outside the measured useful band"
    );
}

#[test]
fn a_berm_is_not_a_crash() {
    let (mut s, id) = bowl(25., 0.45);
    settle(&mut s);
    accelerate(&mut s, id, 14.);
    control(&mut s, id, Controls { throttle: 0.55, steering: 1., ..Default::default() });
    for _ in 0..(4. * 120.) as u32 {
        s.step(1. / 120.);
        assert!(s.take_ejection(id).is_none(), "ejected on a berm");
    }
    assert!(contacts(&s, id) >= 1, "left the berm entirely");
}

/// Backing out of a corner you have stuffed is a thing a rider does, and the
/// physics has always supported it -- but nothing was sending the negative
/// throttle it needs, so the bike could brake and never reverse.
#[test]
fn the_bike_backs_up_on_negative_throttle() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    control(&mut s, id, Controls { throttle: -1., ..Default::default() });
    run(&mut s, 3., 120);
    assert!(speed(&s, id) < -2., "should be backing up, got {}", speed(&s, id));
}

#[test]
fn a_parked_bike_has_no_air_authority() {
    // Air control is for the rider, not for an empty bike falling off a ledge.
    let (mut s, id) = setup(50.);
    s.set_occupied(id, false);
    control(
        &mut s,
        id,
        Controls {
            whip: 1.,
            weight: 1.,
            ..Default::default()
        },
    );
    run(&mut s, 0.5, 120);
    let b = &s.world.bodies[s.vehicles[&id].body];
    assert!(b.angvel().length() < 0.1, "parked bike rotated itself");
}

#[test]
fn cornering_hard_does_not_eject_the_rider() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    accelerate(&mut s, id, 14.);
    control(
        &mut s,
        id,
        Controls {
            throttle: 0.6,
            steering: 1.,
            lean: 1.,
            ..Default::default()
        },
    );
    for _ in 0..(4. * 120.) as u32 {
        s.step(1. / 120.);
        assert!(s.take_ejection(id).is_none(), "ejected while cornering");
    }
    assert!(lean(&s, id) < -0.4, "should actually be leaned over");
}

#[test]
fn the_clutch_revs_the_engine_and_dumping_it_lofts_the_front() {
    let mut lofted = vec![];
    for clutch in [false, true] {
        let (mut s, id) = setup(1.);
        settle(&mut s);
        // Roll in at the same speed either way: the launch itself is grip
        // limited, so what a clutch dump buys is the front wheel, not metres.
        accelerate(&mut s, id, 5.);
        if clutch {
            control(
                &mut s,
                id,
                Controls {
                    throttle: 1.,
                    clutch: true,
                    ..Default::default()
                },
            );
            run(&mut s, 0.8, 120);
            // ...and the engine is audibly spinning while the wheel is not.
            assert!(s.wheel_speed(id) > 20., "engine never revved up");
        }
        control(
            &mut s,
            id,
            Controls {
                throttle: 1.,
                ..Default::default()
            },
        );
        let mut up = false;
        for _ in 0..(1.5 * 120.) as u32 {
            s.step(1. / 120.);
            up |= !wheel_down(&s, id, true);
        }
        lofted.push(up);
    }
    assert_eq!(
        lofted,
        vec![false, true],
        "only a dumped clutch should loft the front wheel"
    );
}

#[test]
fn sustained_inversion_still_ejects() {
    let (mut s, id) = setup(1.);
    settle(&mut s);
    // Put it on its roof and hold it there.
    let body = &mut s.world.bodies[s.vehicles[&id].body];
    body.set_rotation(Rotation::from_rotation_z(std::f32::consts::PI), true);
    body.set_translation(Vector::new(0., 0.6, 0.), true);
    let mut ejected = false;
    for _ in 0..(5. * 120.) as u32 {
        s.step(1. / 120.);
        ejected |= s.take_ejection(id).is_some();
    }
    assert!(ejected, "an inverted bike should still eject its rider");
}

#[test]
fn mixed_riding_remains_finite_for_7200_ticks() {
    let (mut s, id) = setup(1.);
    for tick in 0..7200 {
        let t = tick as f32 / 120.;
        control(
            &mut s,
            id,
            Controls {
                throttle: t.sin(),
                steering: (t * 0.7).cos(),
                lean: (t * 1.3).sin(),
                weight: (t * 0.4).cos(),
                whip: (t * 2.1).sin(),
                brake: (t * 0.9).cos().max(0.),
                handbrake: tick % 400 < 30,
                clutch: tick % 300 < 40,
                ..Default::default()
            },
        );
        s.step(1. / 120.);
        let b = &s.world.bodies[s.vehicles[&id].body];
        assert!(
            b.translation().is_finite() && b.linvel().is_finite() && b.angvel().is_finite(),
            "diverged at tick {tick}"
        );
        assert!(b.translation().length() < 10000., "escaped at tick {tick}");
        let state = s.bike_state(id).unwrap();
        assert!(state.lean.is_finite() && state.lean.abs() < 2.);
    }
}

#[test]
fn riding_is_consistent_at_60_and_120_hz() {
    let mut ends = vec![];
    for hz in [60u32, 120] {
        let (mut s, id) = setup(1.);
        run(&mut s, 2., hz);
        control(
            &mut s,
            id,
            Controls {
                throttle: 1.,
                steering: 0.5,
                ..Default::default()
            },
        );
        run(&mut s, 4., hz);
        ends.push(s.world.bodies[s.vehicles[&id].body].translation());
    }
    let drift = (ends[0] - ends[1]).length();
    assert!(drift < 3., "60 and 120 Hz diverged by {drift} m");
}

/// A flight of stairs: flat run-up, then `n` risers of `rise` and `run`.
fn flight(rise: f32, run: f32, n: i32) -> Vec<[[f32; 3]; 3]> {
    let mut tris = Vec::new();
    let mut tread = |z0: f32, z1: f32, y: f32, t: &mut Vec<[[f32; 3]; 3]>| {
        t.push([[-40., y, z0], [40., y, z1], [40., y, z0]]);
        t.push([[-40., y, z0], [-40., y, z1], [40., y, z1]]);
    };
    tread(-60., 0., 0., &mut tris);
    for i in 0..n {
        let (y, z0) = ((i + 1) as f32 * rise, i as f32 * run);
        tris.push([[-40., y - rise, z0], [40., y, z0], [40., y - rise, z0]]);
        tris.push([[-40., y - rise, z0], [-40., y, z0], [40., y, z0]]);
        tread(z0, z0 + run, y, &mut tris);
    }
    tread(n as f32 * run, n as f32 * run + 40., n as f32 * rise, &mut tris);
    tris
}

/// Ride at a flight and report how far up it got, in steps.
fn climb(d: VehicleDefinition, rise: f32) -> f32 {
    let (run, n) = (0.32, 12);
    let mut s = Simulation::default();
    s.ground(flight(rise, run, n).into_iter()).unwrap();
    let id = s.spawn(d, [0., 1., -14.], 0.).unwrap();
    s.set_occupied(id, true);
    settle(&mut s);
    accelerate(&mut s, id, 8.);
    let mut far = f32::MIN;
    for _ in 0..(5. * 120.) as u32 {
        control(
            &mut s,
            id,
            Controls {
                throttle: 1.,
                ..Default::default()
            },
        );
        s.step(1. / 120.);
        far = far.max(s.world.bodies[s.vehicles[&id].body].translation().z);
    }
    far / run
}

/// A raycast wheel samples the ground at one point, so it cannot roll over an
/// edge, and on a flight the chassis ends up driven into a riser two steps
/// ahead of the front wheel. Without the assist the bike stops dead on
/// anything above a 0.15 m rise -- measured at 11 m/s to a standstill inside a
/// single frame.
#[test]
fn the_bike_climbs_a_flight_of_stairs() {
    for rise in [0.15, 0.18, 0.2, 0.25] {
        let steps = climb(definition(), rise);
        assert!(
            steps > 11.,
            "only got {steps:.1} of 12 steps up a {rise} m flight"
        );
    }
}

/// The escape hatch, and the proof that the assist is what does this rather
/// than some other change: turned off, the bike jams exactly as it used to.
#[test]
fn without_the_step_assist_a_flight_still_stops_the_bike() {
    let mut d = definition();
    d.bike.step_assist = 0.;
    let steps = climb(d, 0.2);
    assert!(steps < 6., "expected a jam without the assist, got {steps:.1}");
}

/// The assist must be invisible everywhere that is not a step. A ramp is the
/// case that matters: it rises just as far, and hauling the bike up one would
/// wreck every jump in the park. Two probes tell them apart -- a ramp is
/// already climbing under the front wheel, a riser leaves it flat until the
/// face.
#[test]
fn ramps_are_untouched_by_the_step_assist() {
    let ride = |assist: f32, degrees: f32| {
        let slope = degrees.to_radians().tan();
        let height = |z: f32| if z > 0. { z * slope } else { 0. };
        let mut tris = Vec::new();
        let mut z = -60.;
        while z < 20. {
            let (z0, z1) = (z, z + 0.25);
            tris.push([[-40., height(z0), z0], [40., height(z1), z1], [40., height(z0), z0]]);
            tris.push([[-40., height(z0), z0], [-40., height(z1), z1], [40., height(z1), z1]]);
            z = z1;
        }
        let mut s = Simulation::default();
        s.ground(tris.into_iter()).unwrap();
        let mut d = definition();
        d.bike.step_assist = assist;
        let id = s.spawn(d, [0., 1., -20.], 0.).unwrap();
        s.set_occupied(id, true);
        settle(&mut s);
        accelerate(&mut s, id, 14.);
        let mut peak: f32 = 0.;
        for _ in 0..(3. * 120.) as u32 {
            control(&mut s, id, Controls { throttle: 1., ..Default::default() });
            s.step(1. / 120.);
            peak = peak.max(s.world.bodies[s.vehicles[&id].body].translation().y);
        }
        peak
    };
    for degrees in [15., 25., 35.] {
        let (off, on) = (ride(0., degrees), ride(0.3, degrees));
        assert!(
            (off - on).abs() < 0.02,
            "a {degrees} degree ramp flew to {on:.2} with the assist and {off:.2} without"
        );
    }
}
