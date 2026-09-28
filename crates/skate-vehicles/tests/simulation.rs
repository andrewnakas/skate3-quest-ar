use skate_vehicles::*;
fn definition() -> VehicleDefinition {
    serde_json::from_str(include_str!(
        "../../../sdk/examples/mario-kart/vehicle.json"
    ))
    .unwrap()
}
fn simulation() -> Simulation {
    let mut s = Simulation::default();
    s.ground(
        [
            [[-100., 0., -100.], [100., 0., 100.], [100., 0., -100.]],
            [[-100., 0., -100.], [-100., 0., 100.], [100., 0., 100.]],
        ]
        .into_iter(),
    )
    .unwrap();
    s
}
#[test]
fn kart_suspension_acceleration_braking_reset_and_cleanup() {
    let mut s = simulation();
    let id = s.spawn(definition(), [0., 2., 0.], 0.).unwrap();
    for _ in 0..240 {
        s.step(1. / 120.);
    }
    let height = s.pose(id).unwrap().0[1];
    assert!((0.2..1.).contains(&height), "settled chassis {height}");
    assert!(
        s.vehicles[&id]
            .controller
            .wheels()
            .iter()
            .all(|w| w.raycast_info().is_in_contact)
    );
    s.vehicles.get_mut(&id).unwrap().controls = Controls {
        throttle: 1.,
        ..Default::default()
    };
    for _ in 0..360 {
        s.step(1. / 120.);
    }
    let speed = s.vehicles[&id].controller.current_vehicle_speed;
    assert!(speed > 3., "forward speed {speed}");
    assert!(s.pose(id).unwrap().0[2] > 3.);
    s.vehicles.get_mut(&id).unwrap().controls = Controls {
        brake: 1.,
        ..Default::default()
    };
    for _ in 0..240 {
        s.step(1. / 120.);
    }
    assert!(
        s.vehicles[&id].controller.current_vehicle_speed.abs() < speed * 0.25,
        "brake speed {} initial {speed}, pose {:?}",
        s.vehicles[&id].controller.current_vehicle_speed,
        s.pose(id)
    );
    s.reset(id, [5., 2., 5.], 1.).unwrap();
    assert_eq!(s.pose(id).unwrap().0, [5., 2., 5.]);
    s.remove(id);
    assert!(s.vehicles.is_empty());
    assert_eq!(s.world.bodies.len(), 1);
}
#[test]
fn definitions_and_controls_reject_invalid_inputs() {
    let mut d = definition();
    d.mass = f32::NAN;
    assert!(d.validate().is_err());
    let mut d = definition();
    d.model = "../outside.glb".into();
    assert!(d.validate().is_err());
    let mut d = definition();
    d.animations.file = None;
    d.animations.drive = Some("drive".into());
    assert!(d.validate().is_err());
    let mut d = definition();
    d.wheels.clear();
    assert!(d.validate().is_err());
    assert!(
        !Controls {
            throttle: 2.,
            ..Default::default()
        }
        .valid()
    );
    assert!(
        !Controls {
            brake: f32::NAN,
            ..Default::default()
        }
        .valid()
    );
    assert!(!package_path("C:/model.glb"));
    assert!(!package_path("../model.glb"));
    assert!(!package_path("model.glb#Scene1"));
    assert!(
        VehicleTuning {
            max_speed: Some(-1.),
            ..Default::default()
        }
        .apply(&definition())
        .is_err()
    );
}
#[test]
fn independent_vehicles_collide_and_remain_finite() {
    let mut s = simulation();
    let a = s.spawn(definition(), [0., 1., -4.], 0.).unwrap();
    let b = s
        .spawn(definition(), [0., 1., 4.], std::f32::consts::PI)
        .unwrap();
    for id in [a, b] {
        s.vehicles.get_mut(&id).unwrap().controls = Controls {
            throttle: 1.,
            ..Default::default()
        };
    }
    for _ in 0..400 {
        s.step(1. / 120.);
    }
    for id in [a, b] {
        let (p, q) = s.pose(id).unwrap();
        assert!(p.iter().chain(q.iter()).all(|x| x.is_finite()));
    }
}

#[test]
fn steering_turns_toward_the_drivers_requested_side() {
    for heading in [0., std::f32::consts::FRAC_PI_2] {
        for steering in [-0.5, 0.5] {
            let mut s = simulation();
            let id = s.spawn(definition(), [0., 1., 0.], heading).unwrap();
            for _ in 0..120 {
                s.step(1. / 120.);
            }
            s.vehicles.get_mut(&id).unwrap().controls = Controls {
                throttle: 1.,
                steering,
                ..Default::default()
            };
            for _ in 0..180 {
                s.step(1. / 120.);
            }
            // Driver/camera left is up cross forward: +X at heading zero (+Z forward).
            let p = s.pose(id).unwrap().0;
            let left_displacement = p[0] * heading.cos() - p[2] * heading.sin();
            assert!(
                left_displacement * steering > 0.1,
                "heading={heading}, steering={steering}, left displacement={left_displacement}"
            );
        }
    }
}
#[test]
fn kart_climbs_ramps_without_nose_catching() {
    for degrees in [20_f32, 30., 40.] {
        let slope = degrees.to_radians().tan();
        let mut s = Simulation::default();
        let y = 20. * slope;
        s.ground(
            [
                [[-20., 0., -30.], [20., 0., 3.], [20., 0., -30.]],
                [[-20., 0., -30.], [-20., 0., 3.], [20., 0., 3.]],
                [[-20., 0., 3.], [20., y, 23.], [20., 0., 3.]],
                [[-20., 0., 3.], [-20., y, 23.], [20., y, 23.]],
            ]
            .into_iter(),
        )
        .unwrap();
        let id = s.spawn(definition(), [0., 1., -2.], 0.).unwrap();
        for _ in 0..240 {
            s.step(1. / 120.);
        }
        s.vehicles.get_mut(&id).unwrap().controls = Controls {
            throttle: 1.,
            ..Default::default()
        };
        let mut highest = 0_f32;
        let mut forward = 0_f32;
        for _ in 0..960 {
            s.step(1. / 120.);
            let p = s.pose(id).unwrap().0;
            highest = highest.max(p[1]);
            forward = forward.max(p[2]);
        }
        println!("Ramp {degrees}: height={highest}, forward={forward}");
        assert!(
            highest > 2. && forward > 7.,
            "failed {degrees} degree ramp: {highest}, {forward}"
        );
    }
}
#[test]
fn collision_mass_and_audio_tuning_are_validated() {
    let base = definition();
    let mut d = base.clone();
    d.collider_rounding = 0.5;
    assert!(d.validate().is_err());
    let mut d = base.clone();
    d.center_of_mass[1] = f32::NAN;
    assert!(d.validate().is_err());
    let mut d = base.clone();
    d.inertia_half_extents = Some([0., 1., 1.]);
    assert!(d.validate().is_err());
    let mut d = base.clone();
    d.engine_audio.max_pitch = 0.1;
    assert!(d.validate().is_err());
    let mute = VehicleTuning {
        engine_volume: Some(0.),
        ..Default::default()
    };
    assert!(mute.valid());
    assert_eq!(mute.apply(&base).unwrap().engine_audio.volume, 0.);
    let invalid = VehicleTuning {
        engine_volume: Some(f32::NAN),
        ..Default::default()
    };
    assert!(!invalid.valid());
    assert!(invalid.apply(&base).is_err());
}
#[test]
fn occupied_crash_ejects_with_preimpact_momentum() {
    use rapier3d::prelude::*;
    let mut s = simulation();
    s.world.insert(
        RigidBodyBuilder::fixed().translation(Vector::new(0., 1., 3.)),
        ColliderBuilder::cuboid(5., 2., 0.2),
    );
    let id = s.spawn(definition(), [0., 0.6, 0.], 0.).unwrap();
    s.set_occupied(id, true);
    let handle = s.vehicles[&id].body;
    s.world.bodies[handle].set_linvel(Vector::new(0., 0., 20.), true);
    let mut crash = None;
    for _ in 0..120 {
        s.step(1. / 120.);
        if let Some(e) = s.take_ejection(id) {
            crash = Some(e);
            break;
        }
    }
    let crash = crash.expect("wall impact should eject");
    assert!(matches!(crash.reason, "crash" | "rider_impact"));
    assert!(
        crash.velocity[2] > 15.,
        "momentum was lost: {:?}",
        crash.velocity
    );
    s.set_occupied(id, false);
    for _ in 0..60 {
        s.step(1. / 120.);
    }
    assert!(s.take_ejection(id).is_none());
}

#[test]
fn airborne_inversion_does_not_eject_a_stunt_driver() {
    use rapier3d::prelude::*;
    for occupied in [false, true] {
        let mut s = simulation();
        let id = s.spawn(definition(), [0., 4., 0.], 0.).unwrap();
        s.set_occupied(id, occupied);
        let h = s.vehicles[&id].body;
        s.world.bodies[h].set_rotation(Rotation::from_rotation_z(std::f32::consts::PI), true);
        s.world.bodies[h].set_linvel(Vector::new(5., 0., 0.), true);
        for _ in 0..30 {
            s.step(1. / 120.);
        }
        let e = s.take_ejection(id);
        assert!(e.is_none(), "airborne inversion must remain controllable");
    }
}

#[test]
fn rider_hitbox_catches_an_overhang_above_the_chassis() {
    use rapier3d::prelude::*;
    let mut s = simulation();
    s.world.insert(
        RigidBodyBuilder::fixed().translation(Vector::new(0., 1.55, 3.)),
        ColliderBuilder::cuboid(5., 0.15, 0.2),
    );
    let id = s.spawn(definition(), [0., 0.6, 0.], 0.).unwrap();
    s.set_occupied(id, true);
    let h = s.vehicles[&id].body;
    s.world.bodies[h].set_linvel(Vector::new(0., 0., 18.), true);
    let mut crash = None;
    for _ in 0..120 {
        s.step(1. / 120.);
        if let Some(e) = s.take_ejection(id) {
            crash = Some(e);
            break;
        }
    }
    assert_eq!(
        crash.expect("head/torso collision must eject").reason,
        "rider_impact"
    );
}
#[test]
fn occupied_ramp_driving_does_not_false_bail() {
    let mut s = Simulation::default();
    let slope = 30_f32.to_radians().tan();
    s.ground(
        [
            [[-20., 0., -20.], [20., 0., 3.], [20., 0., -20.]],
            [[-20., 0., -20.], [-20., 0., 3.], [20., 0., 3.]],
            [[-20., 0., 3.], [20., 20. * slope, 23.], [20., 0., 3.]],
            [
                [-20., 0., 3.],
                [-20., 20. * slope, 23.],
                [20., 20. * slope, 23.],
            ],
        ]
        .into_iter(),
    )
    .unwrap();
    let id = s.spawn(definition(), [0., 0.6, -2.], 0.).unwrap();
    s.set_occupied(id, true);
    s.vehicles.get_mut(&id).unwrap().controls = Controls {
        throttle: 1.,
        ..Default::default()
    };
    let mut climbed = false;
    for _ in 0..960 {
        s.step(1. / 120.);
        assert!(
            s.take_ejection(id).is_none(),
            "normal ramp triggered ejection"
        );
        if s.pose(id).unwrap().0[2] > 10. {
            climbed = true;
            break;
        }
    }
    assert!(climbed);
}

#[test]
fn local_car_collides_with_remote_chassis_and_can_eject() {
    use skate_vehicles::rapier3d::prelude::*;
    let mut s = simulation();
    let local = s.spawn(definition(), [0., 0.7, -4.], 0.).unwrap();
    let remote = s.spawn(definition(), [0., 0.7, 0.], 0.).unwrap();
    for _ in 0..120 {
        s.step(1. / 120.);
    }
    let remote_body = s.vehicles[&remote].body;
    s.vehicles.get_mut(&remote).unwrap().remote = true;
    s.world.bodies[remote_body].set_body_type(RigidBodyType::KinematicVelocityBased, true);
    s.world.bodies[remote_body].set_linvel(Vector::ZERO, true);
    s.world.bodies[remote_body].set_angvel(Vector::ZERO, true);
    let before = s.pose(remote).unwrap().0;
    s.set_occupied(local, true);
    let body = s.vehicles[&local].body;
    s.world.bodies[body].set_linvel(Vector::Z * 15., true);
    let mut contact = false;
    let mut ejected = false;
    for _ in 0..120 {
        s.step(1. / 120.);
        contact |= s
            .world
            .narrow_phase
            .contact_pairs()
            .any(|p| p.total_impulse_magnitude() > 0.);
        ejected |= s.take_ejection(local).is_some();
    }
    assert!(contact);
    assert!(ejected, "crash should produce a native bail request");
    assert!(
        s.pose(local).unwrap().0[2] < before[2] + 1.,
        "car passed through remote chassis"
    );
    assert_eq!(
        s.pose(remote).unwrap().0,
        before,
        "remote simulation must not change its owner's pose"
    );
}
