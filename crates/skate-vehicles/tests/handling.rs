use skate_vehicles::{rapier3d::prelude::*, *};

fn car() -> (Simulation, u64) {
    let mut s = Simulation::default();
    s.ground(
        [
            [
                [-1000., 0., -1000.],
                [1000., 0., 1000.],
                [1000., 0., -1000.],
            ],
            [
                [-1000., 0., -1000.],
                [-1000., 0., 1000.],
                [1000., 0., 1000.],
            ],
        ]
        .into_iter(),
    )
    .unwrap();
    let d = serde_json::from_str(include_str!(
        "../../../sdk/examples/mario-kart/vehicle.json"
    ))
    .unwrap();
    let id = s.spawn(d, [0., 1., 0.], 0.).unwrap();
    run(&mut s, 2., 120);
    (s, id)
}
fn run(s: &mut Simulation, seconds: f32, hz: u32) {
    for _ in 0..(seconds * hz as f32) as u32 {
        s.step(1. / hz as f32);
    }
}
fn speed(s: &Simulation, id: u64) -> f32 {
    s.vehicles[&id].controller.current_vehicle_speed
}

#[test]
fn full_brakes_override_full_throttle_and_stop_without_reversing() {
    let (mut s, id) = car();
    s.vehicles.get_mut(&id).unwrap().controls.throttle = 1.;
    run(&mut s, 3., 120);
    let initial = speed(&s, id);
    assert!(initial > 5., "acceleration {initial}");
    s.vehicles.get_mut(&id).unwrap().controls.brake = 1.;
    run(&mut s, 3., 120);
    assert!(
        speed(&s, id).abs() < 0.2,
        "brake+throttle {}",
        speed(&s, id)
    );
}

#[test]
fn reverse_wakes_a_sleeping_car_and_opposing_pedal_stops_before_reversing() {
    let (mut s, id) = car();
    let h = s.vehicles[&id].body;
    s.world.bodies[h].sleep();
    s.vehicles.get_mut(&id).unwrap().controls.throttle = -1.;
    run(&mut s, 2., 120);
    assert!(speed(&s, id) < -1.);
    s.vehicles.get_mut(&id).unwrap().controls.throttle = 1.;
    run(&mut s, 4., 120);
    assert!(speed(&s, id) > 2.);
}

#[test]
fn tire_impulses_share_the_available_contact_load() {
    let (mut s, id) = car();
    s.vehicles.get_mut(&id).unwrap().controls = Controls {
        throttle: 1.,
        steering: 0.8,
        ..Default::default()
    };
    for _ in 0..1200 {
        s.step(1. / 120.);
        let v = &s.vehicles[&id];
        for w in v.controller.wheels() {
            let capacity = w.wheel_suspension_force.min(w.max_suspension_force)
                * v.definition.tire_grip
                / 120.;
            assert!(w.forward_impulse.hypot(w.side_impulse) <= capacity + 0.001);
        }
        let body = &s.world.bodies[v.body];
        assert!(
            (body.rotation() * Vector::Y).y > 0.5,
            "flat corner rolled car"
        );
    }
}

#[test]
fn steering_is_progressive_and_inner_wheel_has_more_lock() {
    let (mut s, id) = car();
    s.vehicles.get_mut(&id).unwrap().controls.steering = 1.;
    s.step(1. / 120.);
    let first = s.vehicles[&id].controller.wheels()[2].steering;
    assert!(first > 0. && first < 0.05);
    run(&mut s, 1., 120);
    let w = s.vehicles[&id].controller.wheels();
    assert!(w[2].steering > w[3].steering);
    s.reset(id, [0., 1., 0.], 0.).unwrap();
    assert!(
        s.vehicles[&id]
            .controller
            .wheels()
            .iter()
            .all(|w| w.rotation == 0. && w.steering == 0.)
    );
    assert_eq!(speed(&s, id), 0.);
}

#[test]
fn driving_is_consistent_at_60_and_120_hz() {
    let mut results = Vec::new();
    for hz in [60, 120] {
        let (mut s, id) = car();
        s.vehicles.get_mut(&id).unwrap().controls.throttle = 1.;
        run(&mut s, 4., hz);
        let accelerated = speed(&s, id);
        s.vehicles.get_mut(&id).unwrap().controls = Controls {
            brake: 0.4,
            ..Default::default()
        };
        run(&mut s, 1., hz);
        results.push((accelerated, speed(&s, id), s.pose(id).unwrap().0[2]));
    }
    assert!((results[0].0 - results[1].0).abs() < 0.05, "{results:?}");
    assert!((results[0].1 - results[1].1).abs() < 0.05, "{results:?}");
    assert!((results[0].2 - results[1].2).abs() < 0.1, "{results:?}");
}

#[test]
fn air_motion_does_not_count_as_forward_speed_or_create_tire_forces() {
    let (mut s, id) = car();
    s.reset(id, [0., 100., 0.], 0.).unwrap();
    let h = s.vehicles[&id].body;
    s.world.bodies[h].set_linvel(Vector::new(10., -10., 0.), true);
    s.vehicles.get_mut(&id).unwrap().controls.throttle = 1.;
    s.step(1. / 120.);
    assert!(speed(&s, id).abs() < 0.01);
    assert!(
        s.vehicles[&id]
            .controller
            .wheels()
            .iter()
            .all(|w| w.forward_impulse == 0. && w.side_impulse == 0.)
    );
}

#[test]
fn mixed_driving_remains_finite_for_7200_ticks() {
    let (mut s, id) = car();
    for tick in 0..7200 {
        let phase = (tick / 240) % 6;
        s.vehicles.get_mut(&id).unwrap().controls = match phase {
            0 => Controls {
                throttle: 1.,
                ..Default::default()
            },
            1 => Controls {
                throttle: 0.5,
                steering: 0.6,
                ..Default::default()
            },
            2 => Controls {
                brake: 0.5,
                steering: -0.3,
                ..Default::default()
            },
            3 => Controls {
                throttle: -0.5,
                ..Default::default()
            },
            4 => Controls {
                handbrake: true,
                steering: 0.3,
                ..Default::default()
            },
            _ => Controls::default(),
        };
        s.step(1. / 120.);
        let (p, q) = s.pose(id).unwrap();
        assert!(
            p.iter().chain(q.iter()).all(|x| x.is_finite()),
            "tick {tick}"
        );
        let body = &s.world.bodies[s.vehicles[&id].body];
        assert!(body.linvel().is_finite() && body.angvel().is_finite());
        assert!(
            s.vehicles[&id]
                .controller
                .wheels()
                .iter()
                .all(|w| w.rotation.is_finite())
        );
    }
}

#[test]
fn low_friction_surface_reduces_acceleration() {
    let mut results = Vec::new();
    for friction in [0.1, 1.] {
        let (mut s, id) = car();
        for (_, collider) in s.world.colliders.iter_mut() {
            collider.set_friction(friction);
        }
        s.vehicles.get_mut(&id).unwrap().controls.throttle = 1.;
        run(&mut s, 3., 120);
        results.push(speed(&s, id));
    }
    assert!(
        results[1] > results[0] * 1.5,
        "surface traction {results:?}"
    );
}
