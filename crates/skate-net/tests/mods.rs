use skate_net::{
    lobby::{APPLICATION, Info, MAX_APP_KEYS, Session},
    packed,
};
fn info(id: u64) -> Info {
    Info {
        id,
        map: 1,
        rig: 2,
        physics: 3,
        appearance: 4,
    }
}
fn pump(host: &mut Session, guests: &mut [Session], now: u64, loss: bool) {
    for (i, g) in guests.iter_mut().enumerate() {
        for (n, p) in g.service(now).into_iter().enumerate() {
            if !loss || (now / 50 + n as u64) % 3 != 0 {
                host.receive(i as u64 + 10, &p.data, now);
            }
        }
    }
    for (n, p) in host.service(now).into_iter().enumerate() {
        assert!(p.data.len() <= packed::MTU);
        if !loss || (now / 50 + n as u64) % 4 != 0 {
            if let Some(g) = guests.get_mut((p.peer - 10) as usize) {
                g.receive(1, &p.data, now);
            }
        }
    }
}
#[test]
fn owner_state_recovers_loss_late_join_and_tombstones() {
    let mut h = Session::new(7, info(1), None);
    let mut g = vec![Session::new(7, info(2), Some(1))];
    assert!(g[0].publish_application("kart", b"spawned".to_vec(), 0));
    for t in (0..2000).step_by(50) {
        pump(&mut h, &mut g, t, true);
    }
    assert_eq!(h.actors[&2].application["kart"].value, b"spawned");
    g.push(Session::new(7, info(3), Some(1)));
    for t in (2000..4000).step_by(50) {
        pump(&mut h, &mut g, t, true);
    }
    assert_eq!(g[1].actors[&2].application["kart"].value, b"spawned");
    g[0].publish_application("kart", vec![], 4000);
    for t in (4000..6000).step_by(50) {
        pump(&mut h, &mut g, t, true);
    }
    assert!(g[1].actors[&2].application["kart"].value.is_empty());
    for p in g[0].goodbye() {
        h.receive(10, &p.data, 6000);
    }
    // Only the second guest remains; service it without reviving the first.
    for t in (6000..7000).step_by(50) {
        for p in g[1].service(t) {
            h.receive(11, &p.data, t);
        }
        for p in h.service(t) {
            if p.peer == 11 {
                g[1].receive(1, &p.data, t);
            }
        }
    }
    assert!(!g[1].actors.contains_key(&2));
}
fn wire(actor: u64, seq: u32, key: &str, value: &[u8]) -> Vec<u8> {
    let mut b = packed::header(7, actor, APPLICATION, seq);
    b.push(key.len() as u8);
    b.extend(key.as_bytes());
    b.extend(value);
    b
}
#[test]
fn spoofed_owners_old_updates_and_oversized_records_are_rejected() {
    let mut h = Session::new(7, info(1), None);
    let mut g = vec![
        Session::new(7, info(2), Some(1)),
        Session::new(7, info(3), Some(1)),
    ];
    for t in (0..1000).step_by(50) {
        pump(&mut h, &mut g, t, false);
    }
    h.receive(10, &wire(3, 10, "state", b"forged"), 1000);
    assert!(h.actors[&3].application.is_empty());
    h.receive(10, &wire(2, 5, "state", b"new"), 1000);
    h.receive(10, &wire(2, 4, "state", b"old"), 1001);
    assert_eq!(h.actors[&2].application["state"].value, b"new");
    h.receive(10, &wire(2, 6, "state", &vec![0; 1025]), 1002);
    assert_eq!(h.actors[&2].application["state"].value, b"new");
    assert!(!h.publish_application("big", vec![0; 1025], 0));
    for i in 0..MAX_APP_KEYS {
        assert!(h.publish_application(&format!("k{i}"), vec![1], 0));
    }
    assert!(!h.publish_application("overflow", vec![], 0));
    assert!(h.publish_application("k0", vec![], 0));
}
#[test]
fn many_mod_records_eventually_reach_every_player() {
    let mut h = Session::new(7, info(1), None);
    let mut g: Vec<_> = (2..=10)
        .map(|id| Session::new(7, info(id), Some(1)))
        .collect();
    for (i, g) in g.iter_mut().enumerate() {
        for k in 0..24 {
            assert!(g.publish_application(&format!("object{k}"), vec![i as u8; 900], 0));
        }
    }
    for t in (0..30000).step_by(50) {
        pump(&mut h, &mut g, t, true);
    }
    for guest in &g {
        assert_eq!(guest.actors.len(), 10);
        for (&id, a) in &guest.actors {
            if id != 1 {
                assert_eq!(a.application.len(), 24, "peer {} actor {id}", guest.local);
            }
        }
    }
}
