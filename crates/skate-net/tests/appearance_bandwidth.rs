use skate_net::{
    Body, Pose,
    lobby::{Info, Session},
    packed::{self, BodyState, Packed},
};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

#[test]
fn model_crosses_real_udp_host_while_movement_stays_current() {
    let sockets: Vec<_> = (0..3)
        .map(|_| {
            let s = UdpSocket::bind("127.0.0.1:0").unwrap();
            skate_net::socket::configure(&s).unwrap();
            s
        })
        .collect();
    let addresses: Vec<_> = sockets.iter().map(|s| s.local_addr().unwrap()).collect();
    let mut sessions: Vec<_> = (0..3)
        .map(|i| {
            Session::new(
                91,
                Info {
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
    let model: Vec<u8> = (0..12_000_000).map(|i| (i * 17) as u8).collect();
    assert!(sessions[1].blobs.publish(11, model.clone()));
    let start = Instant::now();
    let mut frame = 0;
    let mut buffer = [0; 1500];
    let mut completed = None;
    let mut last_data = 0;
    let mut lost = std::collections::BTreeSet::new();
    loop {
        let now = start.elapsed().as_millis() as u64;
        assert!(
            now < 15_000,
            "transfer stalled at {:?}",
            sessions[2].blobs.progress(12)
        );
        for i in 0..3 {
            loop {
                match sockets[i].recv_from(&mut buffer) {
                    Ok((n, from)) => {
                        let peer = addresses.iter().position(|a| *a == from).unwrap() + 1;
                        // Lose selected first attempts on the client-host leg.
                        if i == 0 && n >= 33 && buffer[24] == skate_net::blob::DATA {
                            let chunk = u32::from_le_bytes(buffer[29..33].try_into().unwrap());
                            if chunk % 37 == 0 && lost.insert(chunk) {
                                continue;
                            }
                        }
                        sessions[i].receive(peer as u64, &buffer[..n], now);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("{e}"),
                }
            }
            // A stale Steam congestion sample must not impose 128 KB/s.
            sessions[i].set_congested(true);
            let pose = Pose {
                p: [frame as f32 * 0.001, 0., 0.],
                q: [0., 0., 0., 1.],
            };
            sessions[i].publish(
                packed::BODY,
                Packed::body(&BodyState {
                    root: pose,
                    enabled: (1u64 << 33) - 1,
                    bodies: vec![
                        Body {
                            pose,
                            velocity: [0.; 3],
                            angular: [0.; 3]
                        };
                        33
                    ],
                })
                .unwrap(),
                now,
            );
            let packets = sessions[i].service(now);
            assert!(packets.iter().map(|p| p.data.len()).sum::<usize>() < 2_100_000);
            for p in packets {
                if p.data[24] == skate_net::blob::DATA {
                    last_data = now;
                }
                sockets[i]
                    .send_to(&p.data, addresses[p.peer as usize - 1])
                    .unwrap();
            }
        }
        if now > 1000 {
            assert!(
                sessions[2]
                    .actors
                    .get(&11)
                    .and_then(|a| a.body.latest())
                    .is_some_and(|b| now - b.received < 300),
                "model transfer starved movement"
            );
        }
        if completed.is_none() && sessions[2].blobs.ready(11).is_some_and(|(_, b)| b == model) {
            eprintln!("12 MB client-host-client UDP transfer: {now} ms at 15 FPS");
            assert!(now < 12_000);
            completed = Some(now);
        }
        if let Some(done) = completed {
            assert!(
                sessions[2].actors[&11]
                    .body
                    .latest()
                    .is_some_and(|b| now - b.received < 300)
            );
            if now > done + 1000 {
                assert!(last_data < done + 500);
                break;
            }
        }
        frame += 1;
        if let Some(wait) = Duration::from_millis(frame * 67).checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
    }
}
