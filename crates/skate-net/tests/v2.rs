use skate_net::{
    Body, Bone, Pose,
    lobby::{Info, Session},
    packed::{self, BodyState, Packed, PoseState},
};
fn pose(p: [f32; 3], t: f32) -> Pose {
    Pose {
        p,
        q: [0., (t / 2.).sin(), 0., (t / 2.).cos()],
    }
}
fn body() -> BodyState {
    BodyState {
        root: pose([330., 20., 100.], 0.),
        enabled: ((1 << 33) - 1) | (1 << 63),
        bodies: (0..33)
            .map(|i| Body {
                pose: pose(
                    [330. + i as f32 * 0.04567, 20.1234, 100.2345],
                    i as f32 * 0.23,
                ),
                velocity: [12.345, 0., -31.123],
                angular: [0.123, 24.789, 0.],
            })
            .collect(),
    }
}
fn info(id: u64) -> Info {
    Info {
        id,
        map: 1,
        rig: 2,
        physics: 3,
        appearance: 4,
    }
}
fn exchange(host: &mut Session, guest: &mut Session, endpoint: u64, time: u64) {
    for p in guest.service(time) {
        host.receive(endpoint, &p.data, time);
    }
    for p in host.service(time) {
        if p.peer == endpoint {
            guest.receive(1, &p.data, time);
        }
    }
}
#[test]
fn compact_roundtrip_and_delta_survive_a_lost_update() {
    let original = body();
    let a = Packed::body(&original).unwrap();
    let wire = packed::delta(1, 2, packed::BODY, 1, &a, None);
    assert_eq!(wire.len(), 833);
    let decoded = packed::apply(&wire, None).unwrap();
    for (a, b) in original
        .bodies
        .iter()
        .zip(decoded.unpack_body().unwrap().bodies)
    {
        for i in 0..3 {
            assert!((a.pose.p[i] - b.pose.p[i]).abs() < 0.00055);
            assert!((a.velocity[i] - b.velocity[i]).abs() < 0.02);
        }
        let dot = a
            .pose
            .q
            .iter()
            .zip(b.pose.q)
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .abs()
            .min(1.);
        assert!(2. * dot.acos() < 0.006);
    }
    let mut moved = original.clone();
    moved.bodies[0].pose.p[0] += 0.02;
    let b = Packed::body(&moved).unwrap();
    let delta = packed::delta(1, 2, packed::BODY, 3, &b, Some((1, &a)));
    assert!(delta.len() < 110);
    assert_eq!(packed::apply(&delta, Some(&decoded)), Some(b));
    assert!(packed::apply(&delta, None).is_none());
    for n in 0..wire.len() {
        assert!(packed::apply(&wire[..n], None).is_none());
    }
}
#[test]
fn maximum_offsets_and_pose_fit_one_datagram() {
    let mut b = body();
    for body in &mut b.bodies {
        body.pose.p[0] += 1000.;
    }
    let p = Packed::body(&b).unwrap();
    let wire = packed::delta(1, 2, packed::BODY, 1, &p, None);
    assert_eq!(wire.len(), 1031);
    assert!(packed::apply(&wire, None).is_some());
    let state = PoseState {
        root: b.root,
        bones: (0..32)
            .map(|i| Bone {
                index: i,
                pose: pose([1000.; 3], i as f32),
            })
            .collect(),
    };
    let p = Packed::pose(&state).unwrap();
    let wire = packed::delta(1, 2, packed::POSE, 1, &p, None);
    assert!(wire.len() <= packed::MTU);
    assert!(packed::apply(&wire, None).is_some());
    b.bodies[0].velocity[0] = f32::NAN;
    assert!(Packed::body(&b).is_none());
}
#[test]
fn admission_full_lobby_mismatch_departure_and_rejoin() {
    let mut host = Session::new(1, info(1), None);
    for id in 2..=10 {
        let mut guest = Session::new(1, info(id), Some(1));
        exchange(&mut host, &mut guest, id, id * 100);
        assert!(guest.connected());
    }
    assert_eq!(host.actors.len(), 10);
    let mut extra = Session::new(1, info(11), Some(1));
    exchange(&mut host, &mut extra, 11, 1100);
    assert!(extra.notice.contains("full"));
    assert!(!extra.connected());
    let mut bad = info(12);
    bad.physics = 99;
    let mut guest = Session::new(1, bad, Some(1));
    exchange(&mut host, &mut guest, 12, 1300);
    assert!(guest.notice.contains("full"));
    host.receive(2, &packed::header(1, 2, skate_net::lobby::GOODBYE, 0), 1400);
    assert_eq!(host.actors.len(), 9);
    exchange(&mut host, &mut extra, 11, 1700);
    assert!(extra.connected());
    assert_eq!(host.actors.len(), 10);
    let mut restarted = Session::new(1, info(22), Some(1));
    exchange(&mut host, &mut restarted, 3, 1800);
    assert!(!host.actors.contains_key(&3));
    assert!(host.actors.contains_key(&22));
}
#[test]
fn guests_cannot_spoof_other_actors_and_timeouts_remove_proxies() {
    let mut host = Session::new(1, info(1), None);
    let mut guest = Session::new(1, info(2), Some(1));
    exchange(&mut host, &mut guest, 2, 100);
    let p = Packed::body(&body()).unwrap();
    host.receive(2, &packed::delta(1, 1, packed::BODY, 1, &p, None), 110);
    assert!(host.actors[&1].body.latest().is_none());
    host.receive(999, &packed::delta(1, 2, packed::BODY, 1, &p, None), 120);
    assert!(host.actors[&2].body.latest().is_none());
    host.service(6000);
    assert_eq!(host.actors.len(), 1);
    guest.service(6000);
    assert_eq!(guest.actors.len(), 1);
    assert!(!guest.connected());
}
#[test]
fn ten_sessions_exchange_production_packets_over_real_loopback_udp() {
    use std::{net::UdpSocket, time::Duration};
    let sockets: Vec<_> = (0..10)
        .map(|_| {
            let s = UdpSocket::bind("127.0.0.1:0").unwrap();
            s.set_nonblocking(true).unwrap();
            s
        })
        .collect();
    let addresses: Vec<_> = sockets.iter().map(|s| s.local_addr().unwrap()).collect();
    let mut sessions: Vec<_> = (0..10)
        .map(|i| Session::new(9, info(i + 1), (i != 0).then_some(1)))
        .collect();
    let packed = Packed::body(&body()).unwrap();
    let mut buf = [0; 1500];
    for time in (0..2500).step_by(10) {
        for (i, s) in sessions.iter_mut().enumerate() {
            while let Ok((n, from)) = sockets[i].recv_from(&mut buf) {
                let peer = addresses.iter().position(|&a| a == from).unwrap() as u64 + 1;
                s.receive(peer, &buf[..n], time);
            }
            if time % 50 == 0 {
                s.publish(packed::BODY, packed.clone(), time);
            }
            for p in s.service(time) {
                sockets[i]
                    .send_to(&p.data, addresses[p.peer as usize - 1])
                    .unwrap();
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    for s in &sessions {
        assert_eq!(s.actors.len(), 10);
        assert!(s.actors.values().all(|a| a.body.latest().is_some()));
        assert_eq!(s.stats.invalid + s.stats.baseline_miss, 0);
    }
}

#[test]
fn repeated_host_migration_preserves_local_state_and_rebuilds_ten_player_lobby() {
    let mut sessions: Vec<_> = (1..=10)
        .map(|id| Session::new(77, info(id), if id == 1 { None } else { Some(1) }))
        .collect();
    let state = Packed::body(&body()).unwrap();
    for s in &mut sessions {
        s.publish(packed::BODY, state.clone(), 50);
    }
    fn pump(sessions: &mut [Session], first: usize, begin: u64) {
        for now in (begin..begin + 2000).step_by(10) {
            let mut packets = Vec::new();
            for (i, s) in sessions.iter_mut().enumerate().skip(first) {
                for p in s.service(now) {
                    packets.push((i as u64 + 1, p));
                }
            }
            for (from, p) in packets {
                if p.peer as usize > first {
                    sessions[p.peer as usize - 1].receive(from, &p.data, now);
                }
            }
        }
    }
    pump(&mut sessions, 0, 100);
    assert!(sessions.iter().all(|s| s.actors.len() == 10));
    for first in 1..=2 {
        let now = 2200 * first as u64;
        for (i, s) in sessions.iter_mut().enumerate().skip(first) {
            let before = s.actors[&s.local].body.latest().unwrap().clone();
            s.migrate(
                if i == first {
                    None
                } else {
                    Some(first as u64 + 1)
                },
                now,
            );
            let after = s.actors[&s.local].body.latest().unwrap();
            assert_eq!(before.seq, after.seq);
            assert_eq!(before.state, after.state);
            assert_eq!(s.actors.len(), 1);
        }
        // Late packets from the departed host must not restore its membership.
        let stale = packed::delta(77, 1, packed::BODY, 100, &state, None);
        for s in sessions.iter_mut().skip(first) {
            s.receive(1, &stale, now);
        }
        pump(&mut sessions, first, now);
        for s in sessions.iter().skip(first) {
            assert!(s.connected());
            assert_eq!(s.actors.len(), 10 - first);
            assert!(s.actors.values().all(|a| a.body.latest().is_some()));
            assert!(!s.actors.contains_key(&1));
        }
    }
}

#[test]
fn occupied_driver_flag_preserves_root_without_native_collision_parts() {
    let mut frame = body();
    frame.enabled = 1u64 << 62;
    frame.root = pose([10., 4., 20.], 0.4);
    let packed = Packed::body(&frame).unwrap();
    let bytes = packed::delta(1, 2, packed::BODY, 1, &packed, None);
    let decoded = packed::apply(&bytes, None).unwrap().unpack_body().unwrap();
    assert_eq!(decoded.enabled, 1u64 << 62);
    assert_eq!(decoded.root.p, frame.root.p);
    frame.enabled = 1u64 << 61;
    assert!(Packed::body(&frame).is_none());
}

#[test]
fn different_maps_physics_and_rigs_can_join_and_exchange_body_updates() {
    let mut host = Session::new(1, info(1), None);
    let mut other = info(2);
    other.map = 88;
    other.physics = 99;
    other.rig = 77;
    let mut guest = Session::new(1, other, Some(1));
    exchange(&mut host, &mut guest, 2, 100);
    assert!(guest.connected());
    assert_eq!(host.actors[&2].info.map, 88);
    host.publish(packed::BODY, Packed::body(&body()).unwrap(), 150);
    guest.publish(packed::BODY, Packed::body(&body()).unwrap(), 150);
    exchange(&mut host, &mut guest, 2, 200);
    assert!(host.actors[&2].body.latest().is_some());
    assert!(guest.actors[&1].body.latest().is_some());
}

#[test]
fn discovery_accepts_other_game_versions_but_not_unrelated_spacewar_lobbies() {
    assert!(skate_net::directory::is_game_lobby(
        "skate3rust-free-skate-v4"
    ));
    assert!(skate_net::directory::is_game_lobby(
        "skate3rust-free-skate-v5"
    ));
    assert!(!skate_net::directory::is_game_lobby("another-game"));
}
