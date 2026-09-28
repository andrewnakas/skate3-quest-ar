//! Synthetic ten-peer WAN test: production codec/session, no game or Steam launched.
use skate_net::{
    Body, Bone, Pose,
    lobby::{Info, Session},
    packed::{self, BodyState, Packed, PoseState},
};
use std::collections::VecDeque;
fn pose(p: [f32; 3], t: f32) -> Pose {
    Pose {
        p,
        q: [0., (t * 0.5).sin(), 0., (t * 0.5).cos()],
    }
}
fn state(id: usize, time: u64, spread: f32, idle: bool) -> (Packed, Packed) {
    let t = if idle { 0. } else { time as f32 / 1000. };
    let root = pose([id as f32 * spread + t * 3., 0., 0.], t * 0.2);
    let bodies = (0..33)
        .map(|i| {
            let phase = t * 3. + i as f32;
            Body {
                pose: pose(
                    [
                        root.p[0] + phase.sin() * 0.2,
                        i as f32 * 0.03,
                        phase.cos() * 0.1,
                    ],
                    phase * 0.1,
                ),
                velocity: [3. + phase.cos(), 0., phase.sin()],
                angular: [0., phase.cos(), 0.],
            }
        })
        .collect();
    let bones = (0..30)
        .map(|i| Bone {
            index: i,
            pose: pose(
                [0., i as f32 * 0.03, (t * 3. + i as f32).sin() * 0.1],
                t + i as f32,
            ),
        })
        .collect();
    (
        Packed::body(&BodyState {
            root,
            enabled: (1 << 33) - 1,
            bodies,
        })
        .unwrap(),
        Packed::pose(&PoseState { root, bones }).unwrap(),
    )
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
struct Delivery {
    at: u64,
    from: usize,
    to: usize,
    data: Vec<u8>,
}
fn run(label: &str, spread: f32, loss: u64, idle: bool) {
    let started = std::time::Instant::now();
    let mut sessions: Vec<_> = (0..10)
        .map(|i| Session::new(123, info(i + 1), (i != 0).then_some(1)))
        .collect();
    let mut queue = VecDeque::<Delivery>::new();
    let mut rng = 9173u64;
    let mut bytes = [[0u64; 2]; 10];
    let mut packets = 0;
    let mut max = 0;
    let mut worst_age = 0;
    let mut worst_pose_age = 0;
    for time in (0..30_000).step_by(10) {
        // Delivery order deliberately differs from send order (jitter and loss).
        let mut i = 0;
        while i < queue.len() {
            if queue[i].at > time {
                i += 1;
                continue;
            }
            let p = queue.remove(i).unwrap();
            if time >= 5000 {
                bytes[p.to][1] += p.data.len() as u64;
            }
            sessions[p.to].receive(p.from as u64 + 1, &p.data, time);
        }
        for (i, s) in sessions.iter_mut().enumerate() {
            if time % 50 == 0 {
                let (body, pose) = state(i, time, spread, idle);
                s.publish(packed::BODY, body, time);
                if time % 100 == 0 {
                    s.publish(packed::POSE, pose, time);
                }
            }
            for p in s.service(time) {
                assert!(p.data.len() <= packed::MTU);
                max = max.max(p.data.len());
                packets += 1;
                s.record_send(p.data.len(), true);
                if time >= 5000 {
                    bytes[i][0] += p.data.len() as u64;
                }
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                if (rng >> 32) % 100 < loss {
                    continue;
                }
                let delay = 30 + ((rng >> 40) % 61);
                queue.push_back(Delivery {
                    at: time + delay,
                    from: i,
                    to: p.peer as usize - 1,
                    data: p.data,
                });
            }
        }
        if time >= 5000 {
            for s in &sessions {
                assert_eq!(s.actors.len(), 10, "lost membership: {label}");
                for (&id, a) in &s.actors {
                    if id == s.local {
                        continue;
                    }
                    let body = a.body.latest().expect("Every player receives every actor");
                    let age = time - body.received;
                    worst_age = worst_age.max(age);
                    let pose_age = time - a.pose.latest().expect("pose available").received;
                    worst_pose_age = worst_pose_age.max(pose_age);
                    assert!(pose_age < 3500, "pose stalled: {pose_age}");
                    assert!(
                        age < if spread == 0. { 1200 } else { 2200 },
                        "stalled replication {label}: {age}"
                    );
                }
            }
        }
    }
    let errors: u64 = sessions
        .iter()
        .map(|s| s.stats.invalid + s.stats.baseline_miss)
        .sum();
    assert_eq!(errors, 0, "codec errors / missing acknowledged baseline");
    println!(
        "{label}: host up/down {:.2}/{:.2} kB/s; guest avg {:.2}/{:.2} kB/s; max datagram {max}; max body/pose arrival age {worst_age}/{worst_pose_age} ms; {packets} packets; CPU wall {:?}",
        bytes[0][0] as f64 / 25_000.,
        bytes[0][1] as f64 / 25_000.,
        bytes[1..].iter().map(|b| b[0]).sum::<u64>() as f64 / 225_000.,
        bytes[1..].iter().map(|b| b[1]).sum::<u64>() as f64 / 225_000.,
        started.elapsed()
    );
}
fn main() {
    run("10 active, clustered, 0% loss", 0., 0, false);
    run("10 active, clustered, 5% loss", 0., 5, false);
    run("10 active, spread 150m, 5% loss", 150., 5, false);
    run("10 idle, clustered, 5% loss", 0., 5, true);
}
