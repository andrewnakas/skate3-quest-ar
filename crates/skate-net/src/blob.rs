//! Reliable, bounded appearance streams. Movement is serviced first by the lobby.
//! Selective acknowledgements slide continuously; no page-sized round trips.
use crate::{lobby::Outgoing, packed};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const META: u8 = 12;
pub const DATA: u8 = 13;
pub const ACK: u8 = 14;
pub const MAX_BLOB: usize = 64 * 1024 * 1024;
const MEMORY: usize = (crate::lobby::MAX_PLAYERS + 2) * MAX_BLOB;
const CHUNK: usize = 1024;
const WINDOW: usize = 1024;
const MASK: usize = WINDOW / 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub hash: [u8; 32],
    pub size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(bytes: &[u8]) -> Vec<u8> {
        let mut b = blake3::hash(bytes).as_bytes().to_vec();
        b.extend((bytes.len() as u32).to_le_bytes());
        b
    }
    #[test]
    fn blob_bounds_hash_validation_and_obsolete_chunks() {
        let mut b = Blobs::default();
        let mut oversized = vec![0; 32];
        oversized.extend(((MAX_BLOB + 1) as u32).to_le_bytes());
        b.receive(1, 10, META, 1, &oversized);
        assert!(b.cache.is_empty());
        b.receive(1, 10, META, 1, &manifest(&[1, 2]));
        let mut wrong = 0u32.to_le_bytes().to_vec();
        wrong.extend([3, 4]);
        b.receive(1, 10, DATA, 1, &wrong);
        assert!(
            b.ready(10).is_none(),
            "corrupt content must never be loaded"
        );
        b.receive(1, 10, META, 2, &manifest(&[5, 6]));
        b.receive(1, 10, DATA, 1, &wrong);
        b.receive(1, 10, META, 1, &manifest(&[1, 2]));
        assert_eq!(b.actors[&10].0, 2);
        let mut valid = 0u32.to_le_bytes().to_vec();
        valid.extend([5, 6]);
        b.receive(1, 10, DATA, 2, &valid);
        assert_eq!(b.ready(10).unwrap().1, &[5, 6]);
        // A later peer can consume verified cached bytes without data retransmission.
        b.receive(2, 11, META, 1, &manifest(&[5, 6]));
        assert_eq!(b.ready(11).unwrap().1, &[5, 6]);
    }
    #[test]
    fn lobby_rejects_forged_blob_origins_and_unregistered_senders() {
        use crate::lobby::{Info, Session};
        let info = |id| Info {
            id,
            map: 1,
            rig: 1,
            physics: 1,
            appearance: 1,
        };
        let mut host = Session::new(88, info(10), None);
        let mut client = Session::new(88, info(11), Some(1));
        for p in client.service(1) {
            host.receive(2, &p.data, 1);
        }
        let mut forged = packed::header(88, 12, META, 1);
        forged.extend(manifest(&[9]));
        host.receive(2, &forged, 2);
        assert!(host.blobs.actors.is_empty());
        let mut valid = packed::header(88, 11, META, 1);
        valid.extend(manifest(&[9]));
        host.receive(3, &valid, 3);
        assert!(host.blobs.actors.is_empty());
        host.receive(2, &valid, 4);
        assert!(host.blobs.actors.contains_key(&11));
    }
}
struct Blob {
    id: Identity,
    bytes: Arc<Vec<u8>>,
    received: Vec<bool>,
    prefix: usize,
    complete: bool,
    failed: bool,
}
impl Blob {
    fn count(&self) -> usize {
        self.id.size.div_ceil(CHUNK)
    }
}
struct Sending {
    seq: u32,
    meta_at: Option<u64>,
    accepted: bool,
    prefix: usize,
    mask: [u8; MASK],
    sent: BTreeMap<usize, u64>,
    window: usize,
    last_loss: u64,
}
impl Sending {
    fn new(seq: u32) -> Self {
        Self {
            seq,
            meta_at: None,
            accepted: false,
            prefix: 0,
            mask: [0; MASK],
            sent: BTreeMap::new(),
            window: 64,
            last_loss: 0,
        }
    }
    fn has(&self, i: usize) -> bool {
        i < self.prefix
            || (i - self.prefix < WINDOW
                && self.mask[(i - self.prefix) / 8] & (1 << ((i - self.prefix) % 8)) != 0)
    }
}
#[derive(Default)]
pub struct Blobs {
    actors: BTreeMap<u64, (u32, [u8; 32])>,
    cache: BTreeMap<[u8; 32], Blob>,
    sending: BTreeMap<(u64, u64), Sending>,
    acknowledgements: BTreeSet<(u64, u64)>,
    round: usize,
}
impl Blobs {
    fn reserve(&mut self, size: usize) -> bool {
        let mut used: usize = self.cache.values().map(|b| b.id.size).sum();
        let unused: Vec<_> = self
            .cache
            .keys()
            .filter(|hash| !self.actors.values().any(|(_, h)| h == *hash))
            .copied()
            .collect();
        for hash in unused {
            if used + size <= MEMORY {
                break;
            }
            used -= self.cache.remove(&hash).unwrap().id.size;
        }
        used + size <= MEMORY
    }
    pub fn publish(&mut self, actor: u64, bytes: Vec<u8>) -> bool {
        if bytes.is_empty() || bytes.len() > MAX_BLOB {
            return false;
        }
        let hash = *blake3::hash(&bytes).as_bytes();
        if self.actors.get(&actor).is_some_and(|(_, h)| *h == hash) {
            return true;
        }
        if self.cache.get(&hash).is_none_or(|b| !b.complete) {
            if !self.cache.contains_key(&hash) && !self.reserve(bytes.len()) {
                return false;
            }
            let count = bytes.len().div_ceil(CHUNK);
            self.cache.insert(
                hash,
                Blob {
                    id: Identity {
                        hash,
                        size: bytes.len(),
                    },
                    bytes: Arc::new(bytes),
                    received: vec![],
                    prefix: count,
                    complete: true,
                    failed: false,
                },
            );
        }
        let seq = self
            .actors
            .get(&actor)
            .map_or(1, |(s, _)| s.saturating_add(1));
        self.actors.insert(actor, (seq, hash));
        true
    }
    pub fn ready(&self, actor: u64) -> Option<(&Identity, &[u8])> {
        let (_, hash) = self.actors.get(&actor)?;
        let b = self.cache.get(hash)?;
        b.complete.then_some((&b.id, b.bytes.as_slice()))
    }
    pub fn progress(&self, local: u64) -> (usize, usize) {
        self.actors
            .iter()
            .filter(|(id, _)| **id != local)
            .filter_map(|(_, (_, h))| self.cache.get(h))
            .filter(|b| !b.complete)
            .fold((0, 0), |(done, total), b| {
                (
                    done + b.received.iter().filter(|v| **v).count(),
                    total + b.count(),
                )
            })
    }
    // The lobby authenticates origin/peer and bounds membership before calling this.
    pub(crate) fn receive(&mut self, peer: u64, actor: u64, kind: u8, seq: u32, data: &[u8]) {
        if kind == META {
            if data.len() != 36 || seq == 0 {
                return;
            }
            let hash: [u8; 32] = data[..32].try_into().unwrap();
            let size = u32::from_le_bytes(data[32..].try_into().unwrap()) as usize;
            if size == 0 || size > MAX_BLOB {
                return;
            }
            if self
                .actors
                .get(&actor)
                .is_some_and(|(old, h)| seq < *old || (seq == *old && hash != *h))
            {
                return;
            }
            if let Some(b) = self.cache.get(&hash) {
                if b.id.size != size {
                    return;
                }
            } else {
                if !self.reserve(size) {
                    return;
                }
                self.cache.insert(
                    hash,
                    Blob {
                        id: Identity { hash, size },
                        bytes: Arc::new(vec![0; size]),
                        received: vec![false; size.div_ceil(CHUNK)],
                        prefix: 0,
                        complete: false,
                        failed: false,
                    },
                );
            }
            self.actors.insert(actor, (seq, hash));
            self.acknowledgements.insert((peer, actor));
        } else if kind == DATA {
            let Some((current, hash)) = self.actors.get(&actor) else {
                return;
            };
            if seq != *current || data.len() < 5 {
                return;
            }
            let b = self.cache.get_mut(hash).unwrap();
            let index = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
            if index >= b.count() || data.len() - 4 != CHUNK.min(b.id.size - index * CHUNK) {
                return;
            }
            self.acknowledgements.insert((peer, actor));
            if b.complete || b.failed || b.received[index] {
                return;
            }
            Arc::get_mut(&mut b.bytes).unwrap()[index * CHUNK..index * CHUNK + data.len() - 4]
                .copy_from_slice(&data[4..]);
            b.received[index] = true;
            while b.prefix < b.count() && b.received[b.prefix] {
                b.prefix += 1;
            }
            if b.prefix == b.count() {
                b.complete = blake3::hash(&b.bytes).as_bytes() == &b.id.hash;
                b.failed = !b.complete;
                // Bad bytes never reach the asset loader or trigger an endless retry.
            }
        } else if kind == ACK {
            if data.len() != 12 + MASK {
                return;
            }
            let origin = u64::from_le_bytes(data[..8].try_into().unwrap());
            let prefix = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
            let Some((current, hash)) = self.actors.get(&origin) else {
                return;
            };
            let count = self.cache[hash].count();
            let Some(tx) = self.sending.get_mut(&(peer, origin)) else {
                return;
            };
            if seq != *current || seq != tx.seq || prefix > count || prefix < tx.prefix {
                return;
            }
            if prefix == tx.prefix {
                for (a, b) in tx.mask.iter_mut().zip(&data[12..]) {
                    *a |= *b;
                }
            } else {
                tx.mask.copy_from_slice(&data[12..]);
                tx.prefix = prefix;
            }
            tx.accepted = true;
            let remove: Vec<_> = tx.sent.keys().filter(|i| tx.has(**i)).copied().collect();
            tx.window = (tx.window + remove.len()).min(WINDOW);
            for i in remove {
                tx.sent.remove(&i);
            }
        }
    }
    pub(crate) fn service(
        &mut self,
        session: u64,
        local: u64,
        host: bool,
        members: &BTreeSet<u64>,
        peers: &[(u64, u64, u64)],
        now: u64,
    ) -> Vec<Outgoing> {
        self.actors.retain(|id, _| members.contains(id));
        self.sending.retain(|(peer, id), _| {
            members.contains(id) && peers.iter().any(|(p, _, _)| p == peer)
        });
        let mut out = vec![];
        for (peer, origin) in std::mem::take(&mut self.acknowledgements) {
            if !peers.iter().any(|(p, _, _)| *p == peer) {
                continue;
            }
            let Some((seq, hash)) = self.actors.get(&origin) else {
                continue;
            };
            let b = &self.cache[hash];
            let mut data = packed::header(session, local, ACK, *seq);
            data.extend(origin.to_le_bytes());
            data.extend((b.prefix as u32).to_le_bytes());
            let mut mask = [0u8; MASK];
            for i in b.prefix..(b.prefix + WINDOW).min(b.count()) {
                if b.complete || b.received[i] {
                    mask[(i - b.prefix) / 8] |= 1 << ((i - b.prefix) % 8);
                }
            }
            data.extend(mask);
            out.push(Outgoing { peer, data });
        }
        // ACKs and the loss-responsive window control throughput, rather than
        // a fixed bytes/second cap. Bound each service call so model uploads
        // cannot monopolize the game thread or overflow the local IPC queues.
        let mut global = 2_000_000usize;
        let mut peers = peers.to_vec();
        if !peers.is_empty() {
            let n = peers.len();
            peers.rotate_left(self.round % n);
        }
        self.round = self.round.wrapping_add(1);
        'peers: for (peer, recipient, rtt) in peers {
            if recipient == 0 {
                continue;
            }
            let mut budget = 512_000usize;
            let mut origins: Vec<_> = self
                .actors
                .iter()
                .filter(|(id, _)| **id != recipient && (host || **id == local))
                .map(|(&id, _)| id)
                .collect();
            if !origins.is_empty() {
                let n = origins.len();
                origins.rotate_left(self.round % n);
            }
            for origin in &origins {
                let (seq, hash) = self.actors[origin];
                let b = &self.cache[&hash];
                let tx = self
                    .sending
                    .entry((peer, *origin))
                    .or_insert_with(|| Sending::new(seq));
                if tx.seq != seq {
                    *tx = Sending::new(seq);
                }
                if !tx.accepted && tx.meta_at.is_none_or(|at| now.saturating_sub(at) >= 500) {
                    let mut data = packed::header(session, *origin, META, seq);
                    data.extend(hash);
                    data.extend((b.id.size as u32).to_le_bytes());
                    out.push(Outgoing { peer, data });
                    tx.meta_at = Some(now);
                }
            }
            // Round robin chunks across actors, with a sliding selective-ACK window.
            let mut candidates = vec![];
            for origin in origins {
                let (_, hash) = self.actors[&origin];
                let b = &self.cache[&hash];
                let tx = self.sending.get_mut(&(peer, origin)).unwrap();
                if !tx.accepted || b.failed {
                    continue;
                }
                let retry = (rtt * 3 + 200).clamp(400, 2000);
                if now.saturating_sub(tx.last_loss) >= retry
                    && tx.sent.values().any(|at| now.saturating_sub(*at) >= retry)
                {
                    tx.window = (tx.window / 2).max(32);
                    tx.last_loss = now;
                }
                let chunks: Vec<_> = (tx.prefix..(tx.prefix + tx.window).min(b.count()))
                    .filter(|&i| {
                        !tx.has(i)
                            && (b.complete || b.received[i])
                            && tx
                                .sent
                                .get(&i)
                                .is_none_or(|at| now.saturating_sub(*at) >= retry)
                    })
                    .collect();
                candidates.push((origin, chunks.into_iter()));
            }
            loop {
                let mut sent = false;
                for (origin, indices) in &mut candidates {
                    let Some(i) = indices.next() else {
                        continue;
                    };
                    let (seq, hash) = self.actors[origin];
                    let b = &self.cache[&hash];
                    let start = i * CHUNK;
                    let end = (start + CHUNK).min(b.id.size);
                    let size = packed::HEADER + 4 + end - start;
                    if global < size {
                        return out;
                    }
                    if budget < size {
                        continue 'peers;
                    }
                    let mut data = packed::header(session, *origin, DATA, seq);
                    data.extend((i as u32).to_le_bytes());
                    data.extend(&b.bytes[start..end]);
                    self.sending
                        .get_mut(&(peer, *origin))
                        .unwrap()
                        .sent
                        .insert(i, now);
                    budget -= size;
                    global -= size;
                    out.push(Outgoing { peer, data });
                    sent = true;
                }
                if !sent {
                    break;
                }
            }
        }
        out
    }
}
