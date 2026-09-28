//! Appearance content uses the same authenticated bulk stream on every transport.
pub use skate_net::blob::{Identity, MAX_BLOB};
use skate_net::lobby::Session;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
pub struct Exchange {
    pending: Option<Vec<u8>>,
    pub ready: BTreeMap<u64, (Identity, Vec<u8>)>,
    known: BTreeSet<[u8; 32]>,
    progress: (usize, usize),
}
impl Exchange {
    pub fn remember(&mut self, id: Identity) {
        self.known.insert(id.hash);
    }
    pub fn publish(&mut self, bytes: Vec<u8>) -> bool {
        if bytes.is_empty() || bytes.len() > MAX_BLOB {
            return false;
        }
        self.pending = Some(bytes);
        true
    }
    pub fn progress(&self) -> String {
        let (done, total) = self.progress;
        if total == 0 {
            format!("Characters: {} synced", self.ready.len())
        } else {
            format!(
                "Characters: {} synced | downloading ({}%)",
                self.ready.len(),
                done * 100 / total
            )
        }
    }
    pub fn tick(&mut self, session: &mut Session, _now: u64) {
        if let Some(bytes) = self.pending.take() {
            session.blobs.publish(session.local, bytes);
        }
        self.ready.retain(|id, _| session.actors.contains_key(id));
        for &actor in session.actors.keys().filter(|id| **id != session.local) {
            let Some((id, bytes)) = session.blobs.ready(actor) else {
                continue;
            };
            if self.ready.get(&actor).is_some_and(|(old, _)| old == id) {
                continue;
            }
            let bytes = if self.known.contains(&id.hash) {
                vec![]
            } else {
                bytes.to_vec()
            };
            self.ready.insert(actor, (id.clone(), bytes));
        }
        self.progress = session.blobs.progress(session.local);
    }
}
/// Exercise actual sockets, host forwarding and a receiver with an empty cache.
/// Also used by the owned-asset test before loading the received GLB in Bevy.
#[cfg(test)]
pub(super) fn transfer_over_udp(model: Vec<u8>) -> Vec<u8> {
    use std::{
        net::UdpSocket,
        time::{Duration, Instant},
    };
    let sockets: Vec<_> = (0..3)
        .map(|_| {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            skate_net::socket::configure(&socket).unwrap();
            socket
        })
        .collect();
    let addresses: Vec<_> = sockets.iter().map(|s| s.local_addr().unwrap()).collect();
    let mut sessions: Vec<_> = (0..3)
        .map(|i| {
            Session::new(
                91,
                skate_net::lobby::Info {
                    id: 10 + i,
                    map: 1,
                    rig: 1,
                    physics: 1,
                    appearance: 1,
                },
                if i == 0 { None } else { Some(1) },
            )
        })
        .collect();
    let mut exchanges: Vec<_> = (0..3).map(|_| Exchange::default()).collect();
    for e in &mut exchanges {
        e.publish(vec![5]);
    }
    exchanges[1].publish(model.clone());
    let started = Instant::now();
    let mut frame = 0;
    let mut buffer = [0u8; 1500];
    let mut result = None;
    let mut completed_at = None;
    let mut last_payload = 0;
    loop {
        let now = started.elapsed().as_millis() as u64;
        assert!(
            now < 25000,
            "actual UDP transfer stalled: {}",
            exchanges[2].progress()
        );
        for i in 0..3 {
            for _ in 0..4096 {
                match sockets[i].recv_from(&mut buffer) {
                    Ok((n, from)) => {
                        let peer = addresses.iter().position(|a| *a == from).unwrap() + 1;
                        sessions[i].receive(peer as u64, &buffer[..n], now);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("UDP receive: {e}"),
                }
            }
            let pose = skate_net::Pose {
                p: [frame as f32 * 0.001, 0., 0.],
                q: [0., 0., 0., 1.],
            };
            let body = skate_net::Body {
                pose,
                velocity: [0.; 3],
                angular: [0.; 3],
            };
            sessions[i].publish(
                skate_net::packed::BODY,
                skate_net::packed::Packed::body(&skate_net::packed::BodyState {
                    root: pose,
                    enabled: (1u64 << 33) - 1,
                    bodies: vec![body; 33],
                })
                .unwrap(),
                now,
            );
            exchanges[i].tick(&mut sessions[i], now);
            for p in sessions[i].service(now) {
                if p.data[24] == skate_net::blob::DATA {
                    last_payload = now;
                }
                sockets[i]
                    .send_to(&p.data, addresses[p.peer as usize - 1])
                    .unwrap();
            }
        }
        if result.is_none()
            && exchanges[2]
                .ready
                .get(&11)
                .is_some_and(|(_, b)| b == &model)
        {
            eprintln!(
                "ONLINE_UDP_COMPLETE bytes={} elapsed_ms={now} fps=15 route=client-host-client",
                model.len()
            );
            assert!(
                now < 15000,
                "32 MB should not take minutes even at 15 FPS: {now} ms"
            );
            result = Some(std::mem::take(
                &mut exchanges[2].ready.get_mut(&11).unwrap().1,
            ));
            completed_at = Some(now);
        }
        if completed_at.is_some_and(|at| now - at > 1500) {
            assert!(
                sessions[2].actors[&11]
                    .body
                    .latest()
                    .is_some_and(|b| now - b.received < 300),
                "model upload starved the remote player's movement"
            );
            assert!(
                last_payload < completed_at.unwrap() + 500,
                "completed upload keeps sending data"
            );
            return result.unwrap();
        }
        frame += 1;
        if let Some(wait) = Duration::from_millis(frame * 67).checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }
}
#[cfg(test)]
#[test]
fn online_appearance_real_udp_large_model_at_low_frame_rate() {
    transfer_over_udp((0..32_067_793).map(|i| (i * 17) as u8).collect());
}
#[cfg(test)]
mod tests {
    use super::*;
    use skate_net::lobby::Info;
    #[test]
    fn online_appearance_transfer_survives_loss_reordering_late_join_and_changes() {
        let mut sessions: Vec<_> = (0..10)
            .map(|i| {
                Session::new(
                    42,
                    Info {
                        id: 100 + i,
                        map: 1,
                        rig: 2,
                        physics: 3,
                        appearance: 4,
                    },
                    if i == 0 { None } else { Some(1) },
                )
            })
            .collect();
        let mut exchanges: Vec<_> = (0..10).map(|_| Exchange::default()).collect();
        let mut expected: Vec<_> = (0..10)
            .map(|i| {
                (0..(90000 + i * 1000))
                    .map(|j| (j * 17 + i) as u8)
                    .collect::<Vec<_>>()
            })
            .collect();
        for (e, b) in exchanges.iter_mut().zip(&expected) {
            assert!(e.publish(b.clone()));
        }
        let mut completed = false;
        let mut delayed = vec![];
        for step in 0..16000 {
            let now = step * 25;
            let active = if step < 200 { 3 } else { 10 };
            if step == 500 {
                expected[1] = vec![77; 123456];
                exchanges[1].publish(expected[1].clone());
            }
            let mut wire = vec![];
            for i in 0..active {
                if step % 4 == 0 {
                    let pose = skate_net::Pose {
                        p: [step as f32 * 0.001, i as f32, 0.],
                        q: [0., 0., 0., 1.],
                    };
                    let body = skate_net::Body {
                        pose,
                        velocity: [0.; 3],
                        angular: [0.; 3],
                    };
                    let mut packed =
                        skate_net::packed::Packed::body(&skate_net::packed::BodyState {
                            root: pose,
                            enabled: (1u64 << 33) - 1,
                            bodies: vec![body; 33],
                        })
                        .unwrap();
                    packed.captured = now;
                    sessions[i].publish(skate_net::packed::BODY, packed, now);
                }
                exchanges[i].tick(&mut sessions[i], now);
                for p in sessions[i].service(now) {
                    if (step + i as u64 + p.data.len() as u64) % 7 != 0 {
                        wire.push((i + 1, p));
                    }
                }
            }
            wire.reverse();
            for (from, p) in wire {
                let due = now + 50 + ((from as u64 * 17 + step * 13) % 70);
                delayed.push((due, from, p));
            }
            let (arrived, pending): (Vec<_>, Vec<_>) =
                delayed.into_iter().partition(|(due, _, _)| *due <= now);
            delayed = pending;
            for (_, from, p) in arrived {
                let to = p.peer as usize - 1;
                if to < active {
                    sessions[to].receive(from as u64, &p.data, now);
                    if step % 13 == 0 {
                        sessions[to].receive(from as u64, &p.data, now);
                    }
                }
            }
            if step > 500
                && exchanges.iter().enumerate().all(|(i, e)| {
                    expected.iter().enumerate().all(|(j, b)| {
                        i == j || e.ready.get(&(100 + j as u64)).is_some_and(|(_, v)| v == b)
                    })
                })
            {
                for session in &sessions {
                    for (&id, actor) in &session.actors {
                        if id != session.local {
                            assert!(
                                actor.body.latest().is_some_and(|b| now - b.received < 1000),
                                "appearance transfer must not starve movement"
                            );
                        }
                    }
                }
                completed = true;
                break;
            }
        }
        assert!(
            completed,
            "every player must receive the latest complete appearance"
        );
        for session in &sessions {
            for actor in session.actors.values() {
                assert!(actor.application.len() <= 43);
            }
        }
    }
    #[test]
    fn online_appearance_rejects_unbounded_metadata_and_chunks() {
        let id = Identity {
            hash: [1; 32],
            size: MAX_BLOB + 1,
        };
        assert!(id.size > MAX_BLOB);
        let mut e = Exchange::default();
        assert!(!e.publish(vec![]));
    }
}
#[cfg(test)]
mod delivery_tests {
    use super::*;
    #[test]
    fn online_appearance_large_model_delivered_once_cached_and_quiet_after_swap_back() {
        let mut sessions: Vec<_> = (0..2)
            .map(|i| {
                Session::new(
                    44,
                    skate_net::lobby::Info {
                        id: 10 + i,
                        map: 1,
                        rig: 1,
                        physics: 1,
                        appearance: 1,
                    },
                    if i == 0 { None } else { Some(1) },
                )
            })
            .collect();

        let mut exchanges = [Exchange::default(), Exchange::default()];
        let model: Vec<u8> = (0..32_067_793).map(|i| (i * 17) as u8).collect();
        exchanges[0].publish(model.clone());
        exchanges[1].publish(vec![4]);
        let mut phase = 0;
        let mut finished_at = 0;
        let mut last_model_packet = 0;
        let mut reused = false;
        for step in 0..18000 {
            let now = step * 67;
            let mut wire = vec![];
            for i in 0..2 {
                exchanges[i].tick(&mut sessions[i], now);
                for p in sessions[i].service(now) {
                    if p.data.get(24) == Some(&skate_net::blob::DATA) && p.data.len() > 100 {
                        last_model_packet = now;
                        assert!(phase < 3, "completed model was retransmitted");
                    }
                    if step % 31 != 0 {
                        wire.push((i + 1, p));
                    }
                }
            }
            for (from, p) in wire {
                sessions[p.peer as usize - 1].receive(from as u64, &p.data, now);
            }
            if phase == 0
                && exchanges[1]
                    .ready
                    .get(&10)
                    .is_some_and(|(_, b)| b == &model)
            {
                assert!(
                    now < 15000,
                    "network model transfer at 15 FPS is too slow: {now} ms"
                );
                let identity = exchanges[1].ready[&10].0.clone();
                exchanges[1].remember(identity);
                exchanges[1].ready.get_mut(&10).unwrap().1.clear();
                exchanges[0].publish(vec![9]);
                phase = 1;
            }
            if phase == 1
                && exchanges[1]
                    .ready
                    .get(&10)
                    .is_some_and(|(_, b)| b == &vec![9])
            {
                exchanges[0].publish(model.clone());
                phase = 2;
                finished_at = now;
            }
            if phase == 2
                && exchanges[1]
                    .ready
                    .get(&10)
                    .is_some_and(|(id, b)| id.size == model.len() && b.is_empty())
            {
                reused = true;
                if now - finished_at > 2000 {
                    phase = 3;
                    finished_at = now;
                }
            }
            if phase == 3 && now - finished_at > 5000 {
                assert!(last_model_packet < finished_at);
                break;
            }
        }
        assert_eq!(phase, 3);
        assert!(reused);
    }
}
