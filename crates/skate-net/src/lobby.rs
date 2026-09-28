//! Transport-neutral ten-player star lobby. The host admits and forwards;
//! each actor retains local physics ownership. No platform SDK or wall clock.
use crate::packed::{self, BODY, HEADER, POSE, Packed, Reader};
use std::collections::{BTreeMap, VecDeque};
pub const MAX_PLAYERS: usize = 10;
pub const HELLO: u8 = 3;
pub const ROSTER: u8 = 4;
pub const ACK: u8 = 5;
pub const PING: u8 = 6;
pub const PONG: u8 = 7;
pub const REJECT: u8 = 8;
pub const GOODBYE: u8 = 9;
pub const APPLICATION: u8 = 10;
pub const APPLICATION_ACK: u8 = 11;
pub const MAX_APP_KEYS: usize = 256;
pub const MAX_APP_VALUE: usize = 1024;
#[derive(Clone, Debug)]
pub struct Application {
    pub seq: u32,
    pub value: Vec<u8>,
    pub received: u64,
}

pub const DEFAULT_LINK_BUDGET: f64 = 180_000.;
pub const DEFAULT_HOST_BUDGET: f64 = 1_000_000.;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub id: u64,
    pub map: u64,
    pub rig: u64,
    pub physics: u64,
    pub appearance: u64,
}
#[derive(Clone, Debug)]
pub struct Revision {
    pub seq: u32,
    pub state: Packed,
    pub received: u64,
    interest: Vec<[f32; 3]>,
}
#[derive(Default)]
pub struct Stream {
    pub history: VecDeque<Revision>,
    pub changed: u64,
}
impl Stream {
    pub fn latest(&self) -> Option<&Revision> {
        self.history.back()
    }
    fn at(&self, seq: u32) -> Option<&Packed> {
        self.history.iter().find(|r| r.seq == seq).map(|r| &r.state)
    }
    fn push(&mut self, seq: u32, state: Packed, now: u64) {
        if self.latest().is_none_or(|r| !r.state.same_state(&state)) {
            self.changed = now;
        }
        let mut interest = vec![state.position()];
        if state.rows.len() == 33 {
            if let Some(body) = state.unpack_body() {
                interest.extend(body.bodies.iter().take(7).map(|b| b.pose.p));
            }
        }
        self.history.push_back(Revision {
            interest,
            seq,
            state,
            received: now,
        });
        while self.history.len() > 64 {
            self.history.pop_front();
        }
    }
}
pub struct Actor {
    pub info: Info,
    pub body: Stream,
    pub pose: Stream,
    pub application: BTreeMap<String, Application>,
}
impl Actor {
    fn new(info: Info) -> Self {
        Self {
            info,
            body: Stream::default(),
            pose: Stream::default(),
            application: BTreeMap::new(),
        }
    }
    pub fn stream(&self, kind: u8) -> &Stream {
        if kind == BODY { &self.body } else { &self.pose }
    }
    fn stream_mut(&mut self, kind: u8) -> &mut Stream {
        if kind == BODY {
            &mut self.body
        } else {
            &mut self.pose
        }
    }
}
struct Link {
    actor: u64,
    seen: u64,
    acks: BTreeMap<(u64, u8), u32>,
    sent: BTreeMap<(u64, u8), (u64, u32)>,
    keys: BTreeMap<(u64, u8), u64>,
    credits: f64,
    app_sent: BTreeMap<(u64, String), (u64, u32)>,
    app_acks: BTreeMap<(u64, String), u32>,
    app_round: usize,
    last_ack: u64,
    last_ping: u64,
    rtt: u64,
}
impl Link {
    fn new(actor: u64, now: u64) -> Self {
        Self {
            actor,
            seen: now,
            acks: BTreeMap::new(),
            sent: BTreeMap::new(),
            keys: BTreeMap::new(),
            credits: 2400.,
            app_sent: BTreeMap::new(),
            app_acks: BTreeMap::new(),
            app_round: 0,
            last_ack: 0,
            last_ping: 0,
            rtt: 0,
        }
    }
}
#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub decoded: u64,
    pub late: u64,
    pub baseline_miss: u64,
    pub invalid: u64,
    pub budget_skips: u64,
    pub send_errors: u64,
    pub rtt_ms: u64,
}
pub struct Outgoing {
    pub peer: u64,
    pub data: Vec<u8>,
}
pub struct Session {
    pub local: u64,
    pub session: u64,
    pub actors: BTreeMap<u64, Actor>,
    pub stats: Stats,
    pub notice: String,
    host: Option<u64>,
    links: BTreeMap<u64, Link>,
    pending: Vec<Outgoing>,
    last_hello: u64,
    last_roster: u64,
    roster_seq: u32,
    received_roster: u32,
    last_service: u64,
    credits: f64,
    budget: f64,
    link_budget: f64,
    round: usize,
    loopback: bool,
    pub blobs: crate::blob::Blobs,
}
impl Session {
    pub fn new(session: u64, info: Info, host: Option<u64>) -> Self {
        let mut actors = BTreeMap::new();
        actors.insert(info.id, Actor::new(info));
        let mut links = BTreeMap::new();
        if let Some(peer) = host {
            links.insert(peer, Link::new(0, 0));
        }
        Self {
            local: info.id,
            session,
            actors,
            host,
            links,
            stats: Stats::default(),
            notice: String::new(),
            pending: vec![],
            last_hello: 0,
            last_roster: 0,
            roster_seq: 0,
            received_roster: 0,
            last_service: 0,
            credits: 12_000.,
            budget: if host.is_none() {
                DEFAULT_HOST_BUDGET
            } else {
                DEFAULT_LINK_BUDGET
            },
            link_budget: DEFAULT_LINK_BUDGET,
            round: 0,
            loopback: false,
            blobs: Default::default(),
        }
    }
    /// An external membership authority chooses the new endpoint. Preserve our
    /// actor/capture history; rebuild membership and recipient delta baselines.
    pub fn migrate(&mut self, host: Option<u64>, now: u64) {
        self.host = host;
        self.links.clear();
        if let Some(peer) = host {
            self.links.insert(peer, Link::new(0, now));
        }
        self.actors.retain(|id, _| *id == self.local);
        self.pending.clear();
        self.received_roster = 0;
        self.roster_seq = 0;
        self.last_hello = 0;
        self.last_roster = 0;
        self.last_service = now;
        self.budget = if host.is_none() {
            DEFAULT_HOST_BUDGET
        } else {
            DEFAULT_LINK_BUDGET
        };
        self.credits = 12_000.;
        self.notice = if host.is_none() {
            String::new()
        } else {
            "Host changed; reconnecting players...".into()
        };
    }
    pub fn set_loopback(&mut self, enabled: bool) {
        self.loopback = enabled;
    }
    pub fn is_host(&self) -> bool {
        self.host.is_none()
    }
    pub fn connected(&self) -> bool {
        self.is_host() || self.received_roster != 0
    }
    pub fn set_congested(&mut self, congested: bool) {
        self.link_budget = if congested {
            DEFAULT_LINK_BUDGET * 0.5
        } else {
            DEFAULT_LINK_BUDGET
        };
    }
    pub fn publish(&mut self, kind: u8, mut state: Packed, now: u64) {
        state.captured = now;
        let stream = self.actors.get_mut(&self.local).unwrap().stream_mut(kind);
        let seq = stream.latest().map_or(1, |r| {
            r.seq.checked_add(1).expect("Session sequence exhausted")
        });
        stream.push(seq, state, now);
    }
    /// Latest owner-authenticated state. Empty values are persistent tombstones.
    /// Repeated full records recover loss and initialize late joiners without replaying Lua.
    pub fn publish_application(&mut self, key: &str, value: Vec<u8>, now: u64) -> bool {
        if key.is_empty() || key.len() > 128 || value.len() > MAX_APP_VALUE {
            return false;
        }
        let records = &mut self.actors.get_mut(&self.local).unwrap().application;
        if records.len() >= MAX_APP_KEYS && !records.contains_key(key) {
            return false;
        }
        if records.get(key).is_some_and(|r| r.value == value) {
            return true;
        }
        let seq = records.get(key).map_or(1, |r| r.seq.saturating_add(1));
        records.insert(
            key.into(),
            Application {
                seq,
                value,
                received: now,
            },
        );
        true
    }
    pub fn record_send(&mut self, len: usize, success: bool) {
        if success {
            self.stats.tx_bytes += len as u64;
        } else {
            self.stats.send_errors += 1;
        }
    }
    fn queue(&mut self, peer: u64, data: Vec<u8>) {
        if self.pending.len() < 64 && data.len() <= packed::MTU {
            self.pending.push(Outgoing { peer, data });
        }
    }
    fn local_info(&self) -> Info {
        self.actors[&self.local].info
    }
    pub fn receive(&mut self, peer: u64, data: &[u8], now: u64) {
        self.stats.rx_bytes += data.len() as u64;
        let Some((session, actor, kind, seq)) = packed::envelope(data) else {
            self.stats.invalid += 1;
            return;
        };
        if session != self.session || actor == 0 {
            return;
        }
        let mut r = Reader(&data[HEADER..]);
        if kind == HELLO && self.is_host() {
            let Some(info) = read_info(&mut r) else {
                return;
            };
            if !r.0.is_empty() || info.id != actor || actor == self.local {
                return;
            }
            // Map, rig and physics fingerprints describe peers; they do not gate admission.
            let error = if !self.links.contains_key(&peer) && self.links.len() >= MAX_PLAYERS - 1 {
                3
            } else {
                0
            };
            if error != 0 {
                let mut b = packed::header(self.session, self.local, REJECT, 0);
                b.push(error);
                self.queue(peer, b);
                return;
            }
            if self
                .links
                .iter()
                .any(|(&endpoint, l)| endpoint != peer && l.actor == actor)
            {
                return;
            }
            let old = self.links.get(&peer).map(|l| l.actor);
            if old != Some(actor) {
                if let Some(old) = old {
                    self.actors.remove(&old);
                }
                self.links.insert(peer, Link::new(actor, now));
                self.actors.insert(actor, Actor::new(info));
                self.last_roster = 0;
            }
            self.links.get_mut(&peer).unwrap().seen = now;
            self.notice.clear();
            return;
        }
        if self.host.is_some_and(|h| h != peer) {
            return;
        }
        if !self.links.contains_key(&peer) {
            return;
        }
        if kind == ROSTER && self.host == Some(peer) {
            if seq <= self.received_roster {
                return;
            }
            let Some(count) = r.byte() else {
                return;
            };
            if count == 0 || count as usize > MAX_PLAYERS {
                return;
            }
            let mut infos = vec![];
            for _ in 0..count {
                let Some(info) = read_info(&mut r) else {
                    return;
                };
                if info.id == 0 || infos.iter().any(|i: &Info| i.id == info.id) {
                    return;
                }
                infos.push(info);
            }
            if !r.0.is_empty() || infos[0].id != actor || !infos.iter().any(|i| i.id == self.local)
            {
                return;
            }
            self.received_roster = seq;
            self.links.get_mut(&peer).unwrap().actor = actor;
            self.links.get_mut(&peer).unwrap().seen = now;
            self.actors
                .retain(|id, _| infos.iter().any(|i| i.id == *id));
            for info in infos {
                self.actors
                    .entry(info.id)
                    .or_insert_with(|| Actor::new(info));
            }
            self.notice.clear();
            return;
        }
        if kind == REJECT && self.host == Some(peer) {
            self.notice = match r.byte() {
                Some(1) => "Map mismatch",
                Some(2) => "Physics definition mismatch",
                Some(3) => "Lobby full (10 players)",
                _ => "Lobby rejected connection",
            }
            .into();
            return;
        }
        if matches!(
            kind,
            crate::blob::META | crate::blob::DATA | crate::blob::ACK
        ) {
            let valid = if kind == crate::blob::ACK {
                self.links[&peer].actor == actor
            } else {
                actor != self.local
                    && self.actors.contains_key(&actor)
                    && (!self.is_host() || self.links[&peer].actor == actor)
            };
            if valid {
                self.blobs.receive(peer, actor, kind, seq, &data[HEADER..]);
                self.links.get_mut(&peer).unwrap().seen = now;
            }
            return;
        }
        if self.links[&peer].actor != actor && !matches!(kind, BODY | POSE | APPLICATION) {
            return;
        }
        if kind == APPLICATION_ACK {
            let Some(origin) = r.u64() else {
                return;
            };
            let Ok(key) = std::str::from_utf8(r.0) else {
                return;
            };
            if self
                .actors
                .get(&origin)
                .and_then(|a| a.application.get(key))
                .is_some_and(|record| seq <= record.seq)
            {
                let link = self.links.get_mut(&peer).unwrap();
                let ack = link.app_acks.entry((origin, key.into())).or_default();
                *ack = (*ack).max(seq);
                link.seen = now;
            }
            return;
        }
        if kind == APPLICATION {
            if actor == self.local || (self.is_host() && self.links[&peer].actor != actor) {
                return;
            }
            let Some(a) = self.actors.get_mut(&actor) else {
                return;
            };
            let Some(length) = r.byte() else {
                return;
            };
            let length = length as usize;
            if length == 0
                || length > 128
                || r.0.len() < length
                || r.0.len() - length > MAX_APP_VALUE
            {
                return;
            }
            let Ok(key) = std::str::from_utf8(&r.0[..length]) else {
                return;
            };
            if a.application.len() >= MAX_APP_KEYS && !a.application.contains_key(key) {
                return;
            }
            if a.application.get(key).is_none_or(|old| seq > old.seq) {
                a.application.insert(
                    key.into(),
                    Application {
                        seq,
                        value: r.0[length..].to_vec(),
                        received: now,
                    },
                );
            }
            let latest = a.application.get(key).map_or(seq, |r| r.seq);
            let mut ack = packed::header(self.session, self.local, APPLICATION_ACK, latest);
            ack.extend(actor.to_le_bytes());
            ack.extend(key.as_bytes());
            self.queue(peer, ack);
            self.links.get_mut(&peer).unwrap().seen = now;
            return;
        }
        if kind == GOODBYE {
            if self.is_host() {
                self.links.remove(&peer);
                self.actors.remove(&actor);
                self.last_roster = 0;
            } else {
                self.actors.retain(|id, _| *id == self.local);
                self.received_roster = 0;
                self.notice = "Host left. Join another lobby.".into();
            }
            return;
        }
        if kind == PING {
            let Some(stamp) = r.u64() else {
                return;
            };
            if !r.0.is_empty() {
                return;
            }
            let mut b = packed::header(self.session, self.local, PONG, 0);
            b.extend(stamp.to_le_bytes());
            self.queue(peer, b);
            self.links.get_mut(&peer).unwrap().seen = now;
            return;
        }
        if kind == PONG {
            let Some(stamp) = r.u64() else {
                return;
            };
            if !r.0.is_empty() || stamp > now || now - stamp > 5000 {
                return;
            }
            self.links.get_mut(&peer).unwrap().rtt = now - stamp;
            self.links.get_mut(&peer).unwrap().seen = now;
            return;
        }
        if kind == ACK {
            let Some(count) = r.byte() else {
                return;
            };
            if count as usize > MAX_PLAYERS {
                return;
            }
            let mut ack = vec![];
            for _ in 0..count {
                let (Some(id), Some(body), Some(pose)) = (r.u64(), r.u32(), r.u32()) else {
                    return;
                };
                ack.push((id, body, pose));
            }
            if !r.0.is_empty() {
                return;
            }
            let link = self.links.get_mut(&peer).unwrap();
            link.seen = now;
            for (id, body, pose) in ack {
                if self.actors.contains_key(&id) {
                    link.acks.insert((id, BODY), body);
                    link.acks.insert((id, POSE), pose);
                }
            }
            return;
        }
        if !matches!(kind, BODY | POSE) || actor == self.local || !self.actors.contains_key(&actor)
        {
            return;
        }
        if self.is_host() && self.links[&peer].actor != actor {
            return;
        }
        let stream = self.actors.get_mut(&actor).unwrap().stream_mut(kind);
        if stream.latest().is_some_and(|s| seq <= s.seq) {
            self.stats.late += 1;
            return;
        }
        let Some(base) = packed::baseline(data) else {
            return;
        };
        let baseline = if base == 0 { None } else { stream.at(base) };
        if base != 0 && baseline.is_none() {
            self.stats.baseline_miss += 1;
            return;
        }
        let Some(state) = packed::apply(data, baseline) else {
            self.stats.invalid += 1;
            return;
        };
        stream.push(seq, state, now);
        self.stats.decoded += 1;
        self.links.get_mut(&peer).unwrap().seen = now;
    }
    pub fn goodbye(&self) -> Vec<Outgoing> {
        self.links
            .keys()
            .map(|&peer| Outgoing {
                peer,
                data: packed::header(self.session, self.local, GOODBYE, 0),
            })
            .collect()
    }
    pub fn service(&mut self, now: u64) -> Vec<Outgoing> {
        let dt = now.saturating_sub(self.last_service).min(1000) as f64 / 1000.;
        self.last_service = now;
        self.credits = (self.credits + dt * self.budget).min(12_000.);
        for l in self.links.values_mut() {
            l.credits = (l.credits + dt * self.link_budget).min(3600.);
        }
        if self.is_host() {
            let expired: Vec<_> = self
                .links
                .iter()
                .filter(|(_, l)| now.saturating_sub(l.seen) > 5000)
                .map(|(&p, l)| (p, l.actor))
                .collect();
            for (peer, id) in expired {
                self.links.remove(&peer);
                self.actors.remove(&id);
                self.last_roster = 0;
            }
        } else if self
            .links
            .values()
            .any(|l| now.saturating_sub(l.seen) > 5000)
        {
            self.actors.retain(|id, _| *id == self.local);
            self.received_roster = 0;
            self.notice = "Host unavailable; waiting to reconnect".into();
        }
        if let Some(peer) = self.host {
            if now.saturating_sub(self.last_hello) >= 500 || self.last_hello == 0 {
                let mut b = packed::header(self.session, self.local, HELLO, 0);
                write_info(&mut b, self.local_info());
                self.queue(peer, b);
                self.last_hello = now.max(1);
            }
        } else if now.saturating_sub(self.last_roster) >= 500 || self.last_roster == 0 {
            self.roster_seq = self.roster_seq.wrapping_add(1);
            let mut b = packed::header(self.session, self.local, ROSTER, self.roster_seq);
            b.push(self.actors.len() as u8);
            write_info(&mut b, self.local_info());
            for (&id, a) in &self.actors {
                if id != self.local {
                    write_info(&mut b, a.info);
                }
            }
            for peer in self.links.keys().copied().collect::<Vec<_>>() {
                self.queue(peer, b.clone());
            }
            self.last_roster = now.max(1);
        }
        let mut output = std::mem::take(&mut self.pending);
        let mut ids: Vec<_> = self.actors.keys().copied().collect();
        if !ids.is_empty() {
            let offset = self.round % ids.len();
            ids.rotate_left(offset);
        }
        self.round = self.round.wrapping_add(1);
        let mut peers: Vec<_> = self.links.keys().copied().collect();
        if !peers.is_empty() {
            let offset = self.round % peers.len();
            peers.rotate_left(offset);
        }
        for peer in peers {
            let link = self.links.get_mut(&peer).unwrap();
            if now.saturating_sub(link.last_ack) >= 100 {
                let mut b = packed::header(self.session, self.local, ACK, 0);
                b.push(self.actors.len() as u8);
                for (&id, a) in &self.actors {
                    b.extend(id.to_le_bytes());
                    b.extend(a.body.latest().map_or(0, |r| r.seq).to_le_bytes());
                    b.extend(a.pose.latest().map_or(0, |r| r.seq).to_le_bytes());
                }
                output.push(Outgoing { peer, data: b });
                link.last_ack = now;
            }
            if now.saturating_sub(link.last_ping) >= 1000 {
                let mut b = packed::header(self.session, self.local, PING, 0);
                b.extend(now.to_le_bytes());
                output.push(Outgoing { peer, data: b });
                link.last_ping = now;
            }
            // Essential physics first; optional pose detail consumes only remaining budget.
            for kind in [BODY, POSE] {
                for &id in &ids {
                    if id == link.actor || (self.host.is_some() && id != self.local) {
                        continue;
                    }
                    let actor = &self.actors[&id];
                    let stream = actor.stream(kind);
                    let Some(latest) = stream.latest() else {
                        continue;
                    };
                    let distance = if self.host.is_some() {
                        0.
                    } else {
                        self.actors
                            .get(&link.actor)
                            .and_then(|a| a.body.latest())
                            .zip(actor.body.latest())
                            .map_or(0., |(a, b)| {
                                // Detached boards also need frequent updates near another player.
                                let roots = distance(a.interest[0], b.interest[0]);
                                a.interest
                                    .iter()
                                    .map(|&p| distance(p, b.interest[0]))
                                    .chain(b.interest.iter().map(|&p| distance(p, a.interest[0])))
                                    .fold(roots, f32::min)
                            })
                    };
                    let mut interval = match (kind, distance) {
                        (BODY, d) if d < 30. => 50,
                        (BODY, d) if d < 100. => 100,
                        (BODY, _) => 500,
                        (POSE, d) if d < 30. => {
                            if self.loopback {
                                50
                            } else {
                                100
                            }
                        }
                        (POSE, d) if d < 100. => 250,
                        _ => 1000,
                    };
                    if !self.loopback
                        && now.saturating_sub(stream.changed) > 250
                        && (kind == POSE || distance >= 8.)
                    {
                        interval = interval.max(200);
                    }
                    if let Some(&(sent, seq)) = link.sent.get(&(id, kind)) {
                        if seq == latest.seq || now.saturating_sub(sent) < interval {
                            continue;
                        }
                    }
                    let keyframe = link
                        .keys
                        .get(&(id, kind))
                        .is_none_or(|at| now.saturating_sub(*at) >= 2000);
                    let baseline = link
                        .acks
                        .get(&(id, kind))
                        .and_then(|&seq| stream.at(seq).map(|s| (seq, s)))
                        .filter(|_| !keyframe);
                    let data =
                        packed::delta(self.session, id, kind, latest.seq, &latest.state, baseline);
                    if data.len() > packed::MTU {
                        self.stats.invalid += 1;
                        continue;
                    }
                    let bytes = data.len() as f64;
                    if link.credits < bytes || self.credits < bytes {
                        self.stats.budget_skips += 1;
                        continue;
                    }
                    link.credits -= bytes;
                    self.credits -= bytes;
                    link.sent.insert((id, kind), (now, latest.seq));
                    if baseline.is_none() {
                        link.keys.insert((id, kind), now);
                    }
                    output.push(Outgoing { peer, data });
                }
            }
        }
        // Rotate every record, not just actors, so a busy mod cannot starve later keys.
        let mut app_peers: Vec<_> = self.links.keys().copied().collect();
        if !app_peers.is_empty() {
            let offset = self.round % app_peers.len();
            app_peers.rotate_left(offset);
        }
        for peer in app_peers {
            let link = self.links.get_mut(&peer).unwrap();
            link.app_sent
                .retain(|(id, _), _| self.actors.contains_key(id));
            link.app_acks
                .retain(|(id, _), _| self.actors.contains_key(id));
            let records: Vec<_> = self
                .actors
                .iter()
                .filter(|(id, _)| **id != link.actor && (self.host.is_none() || **id == self.local))
                .flat_map(|(&id, a)| {
                    a.application
                        .iter()
                        .map(move |(key, record)| (id, key, record))
                })
                .collect();
            let count = records.len();
            for offset in 0..count {
                let index = (link.app_round + offset) % count;
                let (id, key, record) = records[index];
                if link
                    .app_acks
                    .get(&(id, key.clone()))
                    .is_some_and(|&seq| seq == record.seq)
                {
                    continue;
                }
                let previous = link.app_sent.get(&(id, key.clone()));
                if previous.is_some_and(|&(at, seq)| {
                    now.saturating_sub(at)
                        < if seq == record.seq {
                            110 + ((self.round as u64 + id) * 17) % 130
                        } else {
                            50
                        }
                }) {
                    continue;
                }
                let mut data = packed::header(self.session, id, APPLICATION, record.seq);
                data.push(key.len() as u8);
                data.extend(key.as_bytes());
                data.extend(&record.value);
                let bytes = data.len() as f64;
                if bytes > link.credits || bytes > self.credits {
                    link.app_round = index;
                    break;
                }
                link.credits -= bytes;
                self.credits -= bytes;
                link.app_sent.insert((id, key.clone()), (now, record.seq));
                output.push(Outgoing { peer, data });
            }
        }
        let members = self.actors.keys().copied().collect();
        let peers: Vec<_> = self
            .links
            .iter()
            .map(|(&peer, link)| (peer, link.actor, link.rtt))
            .collect();
        output.extend(self.blobs.service(
            self.session,
            self.local,
            self.host.is_none(),
            &members,
            &peers,
            now,
        ));
        self.stats.rtt_ms = self.links.values().map(|l| l.rtt).max().unwrap_or(0);
        output
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f32>()
        .sqrt()
}
fn write_info(b: &mut Vec<u8>, i: Info) {
    for v in [i.id, i.map, i.rig, i.physics, i.appearance] {
        b.extend(v.to_le_bytes());
    }
}
fn read_info(r: &mut Reader) -> Option<Info> {
    Some(Info {
        id: r.u64()?,
        map: r.u64()?,
        rig: r.u64()?,
        physics: r.u64()?,
        appearance: r.u64()?,
    })
}
