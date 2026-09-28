use skate_vehicles::{rapier3d::prelude::*, *};
fn setup(height: f32) -> (Simulation, u64) {
    let mut s = Simulation::default();
    s.ground(
        [
            [[-100., 0., -100.], [100., 0., 100.], [100., 0., -100.]],
            [[-100., 0., -100.], [-100., 0., 100.], [100., 0., 100.]],
        ]
        .into_iter(),
    )
    .unwrap();
    let d = serde_json::from_str(include_str!(
        "../../../sdk/examples/mario-kart/vehicle.json"
    ))
    .unwrap();
    let id = s.spawn(d, [0., height, 0.], 0.).unwrap();
    s.set_occupied(id, true);
    (s, id)
}
#[test]
fn stick_controls_pitch_and_roll_only_in_air_without_adding_lift() {
    for (pitch, steering) in [(1., 0.), (-1., 0.), (0., 1.), (0., -1.)] {
        let (mut s, id) = setup(100.);
        s.vehicles.get_mut(&id).unwrap().controls = Controls {
            pitch,
            steering,
            ..Default::default()
        };
        for _ in 0..60 {
            s.step(1. / 120.);
            assert!(s.take_ejection(id).is_none());
        }
        let body = &s.world.bodies[s.vehicles[&id].body];
        let angular = body.rotation().inverse() * body.angvel();
        if pitch != 0. {
            assert!(angular.x * pitch > 1.);
        }
        if steering != 0. {
            assert!(angular.z * steering < -1.);
        }
        assert!(body.linvel().y < -4.);
        assert!(body.linvel().x.abs() < 0.001 && body.linvel().z.abs() < 0.001);
        s.vehicles.get_mut(&id).unwrap().controls = Controls::default();
        for _ in 0..60 {
            s.step(1. / 120.);
        }
        assert!(s.world.bodies[s.vehicles[&id].body].angvel().length() < angular.length() * 0.4);
    }
}
#[test]
fn grounded_pitch_input_does_not_lift_or_rotate_car() {
    let mut positions = Vec::new();
    for pitch in [0., 1.] {
        let (mut s, id) = setup(0.6);
        for _ in 0..240 {
            s.step(1. / 120.);
        }
        s.vehicles.get_mut(&id).unwrap().controls.pitch = pitch;
        for _ in 0..120 {
            s.step(1. / 120.);
        }
        positions.push(s.pose(id).unwrap());
    }
    assert_eq!(positions[0], positions[1]);
}
#[test]
fn low_speed_wall_bump_does_not_eject_driver() {
    let (mut s, id) = setup(0.6);
    for _ in 0..240 {
        s.step(1. / 120.);
    }
    s.world.insert(
        RigidBodyBuilder::fixed().translation(Vector::new(0., 1., 3.)),
        ColliderBuilder::cuboid(10., 2., 0.1),
    );
    s.world.step();
    let h = s.vehicles[&id].body;
    s.world.bodies[h].set_linvel(Vector::Z * 5., true);
    for _ in 0..180 {
        s.step(1. / 120.);
        assert!(s.take_ejection(id).is_none());
    }
    assert!(s.pose(id).unwrap().0[2] < 3.);
}
#[test]
fn sustained_grounded_inversion_still_ejects() {
    let (mut s, id) = setup(1.);
    let v = s.vehicles.get_mut(&id).unwrap();
    v.definition.rider_safety.crash_delta_v = 50.;
    v.definition.rider_safety.hit_impulse = 10000.;
    let h = v.body;
    s.world.bodies[h].set_rotation(Rotation::from_rotation_z(std::f32::consts::PI), true);
    // Constrain a wreck on its roof to isolate the contact-dependent timer.
    s.world.bodies[h].lock_rotations(true, true);
    let mut ejection = None;
    for _ in 0..480 {
        s.step(1. / 120.);
        if let Some(e) = s.take_ejection(id) {
            ejection = Some(e);
            break;
        }
    }
    assert_eq!(
        ejection.expect("grounded inverted wreck must eject").reason,
        "inverted"
    );
}

#[test]
fn ground_stabilizer_reduces_roll_disturbance() {
    let mut rates = Vec::new();
    for strength in [0., 0.8] {
        let (mut s, id) = setup(0.6);
        for _ in 0..240 {
            s.step(1. / 120.);
        }
        s.vehicles.get_mut(&id).unwrap().definition.ground_stability = strength;
        let h = s.vehicles[&id].body;
        s.world.bodies[h].set_angvel(Vector::Z, true);
        s.step(1. / 120.);
        rates.push(s.world.bodies[h].angvel().z);
    }
    assert!(
        rates[1] < rates[0] - 0.02,
        "roll rates without/with assist: {rates:?}"
    );
}
