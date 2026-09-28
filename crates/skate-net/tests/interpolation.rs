use skate_net::interpolation::{Buffer, Clock, position};
#[test]
fn independent_pose_samples_do_not_hold_on_intervening_physics_updates() {
    let mut poses = Buffer::default();
    poses.insert(0., 0f32);
    poses.insert(0.1, 1f32);
    // Physics at 50 ms must not insert another zero-valued pose here.
    let (a, b, u) = poses.pair(0.075).unwrap();
    let value = poses.samples[a].value * (1. - u) + poses.samples[b].value * u;
    assert!((value - 0.75).abs() < 1e-6);
    assert!(!poses.insert(0.1, 99.));
    assert!(!poses.insert(0.05, 99.));
    assert_eq!(poses.samples.len(), 2);
}
#[test]
fn root_curve_is_exact_at_samples_and_does_not_overshoot_an_impact() {
    let mut roots = Buffer::default();
    for (t, x) in [(0., 0.), (0.1, 1.), (0.2, 1.1), (0.3, 0.5)] {
        roots.insert(t, [x, 0., 0.]);
    }
    for sample in &roots.samples {
        assert_eq!(position(&roots, sample.time).unwrap(), sample.value);
    }
    for i in 100..200 {
        let p = position(&roots, i as f64 / 1000.).unwrap();
        assert!(p[0] >= 1. && p[0] <= 1.10001);
    }
    let mut linear = Buffer::default();
    for i in 0..8 {
        let t = i as f64 * 0.05;
        linear.insert(t, [t as f32 * 3., 0., 0.]);
    }
    for i in 0..350 {
        let t = i as f64 / 1000.;
        assert!((position(&linear, t).unwrap()[0] - t as f32 * 3.).abs() < 1e-6);
    }
}
fn simulate(fps: u32, loss: u64, jitter: bool) -> (f64, u64, f64) {
    let mut clock = Clock::default();
    let mut body = Buffer::default();
    let mut pose = Buffer::default();
    let mut packets = Vec::new();
    let mut rng = 8741u64;
    for i in 0..600 {
        let source = i as f64 * 0.05;
        for stream in 0..2 {
            if stream == 1 && i % 2 != 0 {
                continue;
            }
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            if (rng >> 32) % 100 < loss {
                continue;
            }
            let delay = if jitter {
                0.03 + ((rng >> 40) % 61) as f64 / 1000.
            } else {
                0.04
            };
            packets.push((source + delay, stream, source));
        }
    }
    packets.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut received = 0;
    let mut previous = None;
    let mut max_step = 0f64;
    let mut last = 0.;
    for frame in 0..fps * 29 {
        let now = frame as f64 / fps as f64;
        while received < packets.len() && packets[received].0 <= now {
            let (at, stream, source) = packets[received];
            received += 1;
            let buffer = if stream == 0 { &mut body } else { &mut pose };
            if buffer.insert(source, source as f32) {
                clock.observe(stream, source, at);
            }
        }
        if let Some(time) = clock.step(now) {
            if let Some(prev) = previous {
                assert!(time >= prev);
                if now > 2. {
                    max_step = max_step.max(time - prev);
                }
            }
            previous = Some(time);
            last = time;
            if now > 2. {
                assert!(now - time < 0.8, "unbounded delay {fps}Hz: {}", now - time);
                // Every rendered value is evaluated at source time, independent of arrival jitter.
                for buffer in [&body, &pose] {
                    let (a, b, u) = buffer.pair(time).unwrap();
                    if a != b {
                        let value =
                            buffer.samples[a].value * (1. - u) + buffer.samples[b].value * u;
                        assert!((value - time as f32).abs() < 0.00001);
                    }
                }
            }
        }
    }
    assert!(
        max_step <= 1.081 / fps as f64,
        "render discontinuity: {max_step}"
    );
    println!(
        "{fps}fps loss={loss}% jitter={jitter}: delay={:.1}ms underruns={} max frame advance={:.3}ms",
        clock.delay * 1000.,
        clock.underruns,
        max_step * 1000.
    );
    (last, clock.underruns, max_step)
}
#[test]
fn playback_is_smooth_at_60_120_144_fps_with_jitter_and_loss() {
    for fps in [60, 120, 144] {
        let (_, stalls, _) = simulate(fps, 0, false);
        assert_eq!(stalls, 0);
        let (_, stalls, _) = simulate(fps, 5, true);
        assert!(stalls < 20, "excessive buffer starvation");
    }
}
#[test]
fn suspended_render_loop_and_teleport_reset_do_not_replay_old_motion() {
    let mut c = Clock::default();
    c.observe(0, 1., 1.);
    c.observe(1, 1., 1.);
    c.step(1.);
    c.observe(0, 10., 10.);
    c.observe(1, 10., 10.);
    assert!(c.step(10.).unwrap() > 7.);
    c = Clock::default();
    c.observe(0, 20., 20.);
    c.observe(1, 20., 20.);
    assert!(c.step(20.).unwrap() > 19.);
    let mut b = Buffer::default();
    for i in 0..100 {
        b.insert(i as f64, i);
    }
    assert_eq!(b.samples.len(), 64);
}
#[test]
fn source_timestamps_survive_host_forwarding_and_do_not_disable_idle_detection() {
    use skate_net::{
        Body, Pose,
        lobby::{Info, Session},
        packed::{self, BodyState, Packed},
    };
    let info = |id| Info {
        id,
        map: 1,
        rig: 2,
        physics: 3,
        appearance: 4,
    };
    let mut host = Session::new(8, info(1), None);
    let mut sender = Session::new(8, info(2), Some(1));
    let mut viewer = Session::new(8, info(3), Some(1));
    for (id, guest) in [(2, &mut sender), (3, &mut viewer)] {
        for p in guest.service(0) {
            host.receive(id, &p.data, 1000);
        }
    }
    for p in host.service(1000) {
        if p.peer == 2 {
            sender.receive(1, &p.data, 10);
        } else {
            viewer.receive(1, &p.data, 10);
        }
    }
    let pose = Pose {
        p: [0.; 3],
        q: [0., 0., 0., 1.],
    };
    let state = Packed::body(&BodyState {
        root: pose,
        enabled: 0,
        bodies: vec![
            Body {
                pose,
                velocity: [0.; 3],
                angular: [0.; 3]
            };
            33
        ],
    })
    .unwrap();
    sender.publish(packed::BODY, state.clone(), 25);
    for p in sender.service(25) {
        host.receive(2, &p.data, 1050);
    }
    for p in host.service(1060) {
        if p.peer == 3 {
            viewer.receive(1, &p.data, 80);
        }
    }
    let received = viewer.actors[&2].body.latest().unwrap();
    assert_eq!(received.state.captured, 25);
    assert_eq!(received.received, 80);
    sender.publish(packed::BODY, state, 75);
    assert_eq!(sender.actors[&2].body.changed, 25);
    assert_eq!(sender.actors[&2].body.latest().unwrap().state.captured, 75);
}
#[test]
fn loopback_uses_a_short_buffer_and_recovers_after_sender_stalls() {
    let mut clock = Clock::for_connection(true);
    let mut last_source = -1.;
    let mut final_age = 0.;
    for frame in 0..1200 {
        let now = frame as f64 / 120.;
        let source = ((now - 0.006).max(0.) / 0.05).floor() * 0.05;
        if !(4.0..5.0).contains(&now) && source > last_source {
            clock.observe(0, source, now);
            clock.observe(1, source, now);
            last_source = source;
        }
        if let Some(t) = clock.step(now) {
            if now > 1. && !(4.0..5.1).contains(&now) {
                assert!(now - t < 0.18, "localhost backlog: {}", now - t);
            }
            final_age = now - t;
        }
        assert!(clock.delay <= 0.15);
    }
    println!(
        "Loopback: target {:.1}ms, final visual age {:.1}ms",
        clock.delay * 1000.,
        final_age * 1000.
    );
    assert!(clock.delay < 0.08);
}
#[test]
fn head_offset_survives_packing_and_bind_pose_composition() {
    use skate_core::animation::{output::Sqt, pose_add};
    use skate_net::{
        Bone, Pose,
        packed::{self, Packed, PoseState},
    };
    let delta = Sqt {
        scale: [1.; 4],
        rotation: [0., 0., 0., 1.],
        translation: [0., 0., 0., 1.],
    };
    let bind = Sqt {
        translation: [0., 0.15, 0., 1.],
        ..delta
    };
    let composed = pose_add::add(delta, bind, true);
    assert_eq!(composed.translation[1], 0.15);
    let pose = |y| Pose {
        p: [0., y, 0.],
        q: [0., 0., 0., 1.],
    };
    let state = PoseState {
        root: pose(0.),
        bones: vec![
            Bone {
                index: 6,
                pose: pose(1.55),
            },
            Bone {
                index: 7,
                pose: pose(1.55 + composed.translation[1]),
            },
        ],
    };
    let packed = Packed::pose(&state).unwrap();
    let data = packed::delta(1, 1, packed::POSE, 1, &packed, None);
    let decoded = packed::apply(&data, None).unwrap().unpack_pose().unwrap();
    assert!((decoded.bones[1].pose.p[1] - decoded.bones[0].pose.p[1] - 0.15).abs() < 0.001);
}
