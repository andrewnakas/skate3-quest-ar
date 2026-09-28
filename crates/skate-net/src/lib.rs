//! Transport-neutral, bounded snapshot protocol. No platform identity or SDK types.
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
pub mod blob;
mod codec;
pub mod directory;
pub mod interpolation;
pub mod lobby;
pub mod packed;
pub mod socket;

pub const MAGIC: &[u8; 8] = b"SK8NET01";
pub const MAX_FRAME: usize = 48_000;
const PAYLOAD: usize = 1_000;
const HEADER: usize = 36;
pub const BODY_COUNT: usize = 33;
pub const DEFAULT_APPEARANCE: &str = "default-skater-v1";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pose {
    pub p: [f32; 3],
    pub q: [f32; 4],
}
impl Pose {
    pub fn valid(&self) -> bool {
        self.p.iter().all(|v| v.is_finite() && v.abs() < 100_000.)
            && self.q.iter().all(|v| v.is_finite())
            && (0.98..1.02).contains(&self.q.iter().map(|v| v * v).sum::<f32>())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Body {
    pub pose: Pose,
    pub velocity: [f32; 3],
    pub angular: [f32; 3],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bone {
    pub index: u16,
    pub pose: Pose,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Shape {
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    Capsule {
        center: [f32; 3],
        axis: [f32; 3],
        half: f32,
        radius: f32,
    },
    Box {
        pose: Pose,
        half: [f32; 3],
        radius: f32,
    },
    Triangle {
        vertices: [[f32; 3]; 3],
        radius: f32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Volume {
    pub body: u8,
    pub shape: Shape,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub map: u64,
    pub rig: u64,
    pub tick: u64,
    pub appearance: String,
    pub root: Pose,
    /// Physical animation anchors and board parts, never the complete skin.
    pub bones: Vec<Bone>,
    pub bodies: Vec<Body>,
    pub volumes: Vec<Volume>,
}
fn vector(v: &[f32; 3], bound: f32) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() <= bound)
}
impl Frame {
    pub fn validate(&self) -> bool {
        if self.appearance.len() > 128
            || !self.root.valid()
            || self.bones.len() > 32
            || self.bodies.len() != BODY_COUNT
            || self.volumes.len() > 64
        {
            return false;
        }
        if !self.bones.iter().all(|b| b.index < 256 && b.pose.valid()) {
            return false;
        }
        if self
            .bones
            .iter()
            .enumerate()
            .any(|(i, b)| self.bones[..i].iter().any(|a| a.index == b.index))
        {
            return false;
        }
        if !self
            .bodies
            .iter()
            .all(|b| b.pose.valid() && vector(&b.velocity, 250.) && vector(&b.angular, 500.))
        {
            return false;
        }
        self.volumes.iter().all(|v| {
            let Some(body) = self.bodies.get(v.body as usize) else {
                return false;
            };
            let near = |p: &[f32; 3]| {
                vector(p, 100_000.)
                    && p.iter()
                        .zip(body.pose.p)
                        .map(|(a, b)| (a - b) * (a - b))
                        .sum::<f32>()
                        < 16.
            };
            let radius = |r: f32| r.is_finite() && (0.0..=1.0).contains(&r);
            match &v.shape {
                Shape::Sphere { center, radius: r } => near(center) && radius(*r),
                Shape::Capsule {
                    center,
                    axis,
                    half,
                    radius: r,
                } => {
                    near(center)
                        && vector(axis, 1.01)
                        && (0.98..1.02).contains(&axis.iter().map(|v| v * v).sum::<f32>())
                        && radius(*half)
                        && radius(*r)
                }
                Shape::Box {
                    pose,
                    half,
                    radius: r,
                } => pose.valid() && near(&pose.p) && half.iter().all(|v| radius(*v)) && radius(*r),
                Shape::Triangle {
                    vertices,
                    radius: r,
                } => vertices.iter().all(near) && radius(*r),
            }
        })
    }
}
pub fn appearance_or_default(requested: &str) -> &'static str {
    // Only the canonical rig/outfit is implemented. Never turn a peer identifier into a path.
    let _ = requested;
    DEFAULT_APPEARANCE
}
pub fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
pub fn packets(
    session: u64,
    node: u64,
    sequence: u64,
    frame: &Frame,
) -> Result<Vec<Vec<u8>>, String> {
    if !frame.validate() {
        return Err("Invalid snapshot".into());
    }
    let data = codec::encode(frame);
    if !frame.validate() || data.len() > MAX_FRAME {
        return Err("Snapshot exceeds protocol bounds".into());
    }
    let count = data.len().div_ceil(PAYLOAD) as u16;
    Ok(data
        .chunks(PAYLOAD)
        .enumerate()
        .map(|(index, chunk)| {
            let mut out = Vec::with_capacity(HEADER + chunk.len());
            out.extend(MAGIC);
            out.extend(session.to_le_bytes());
            out.extend(node.to_le_bytes());
            out.extend(sequence.to_le_bytes());
            out.extend((index as u16).to_le_bytes());
            out.extend(count.to_le_bytes());
            out.extend(chunk);
            out
        })
        .collect())
}
pub fn envelope(packet: &[u8]) -> Option<(u64, u64, u64)> {
    if packet.len() <= HEADER || packet.len() > HEADER + PAYLOAD || &packet[..8] != MAGIC {
        return None;
    }
    Some((
        u64::from_le_bytes(packet[8..16].try_into().ok()?),
        u64::from_le_bytes(packet[16..24].try_into().ok()?),
        u64::from_le_bytes(packet[24..32].try_into().ok()?),
    ))
}
struct Partial {
    sequence: u64,
    count: usize,
    chunks: Vec<Option<Vec<u8>>>,
    start: Instant,
}
#[derive(Default)]
pub struct Receiver {
    pending: Vec<Partial>,
    completed: u64,
}
impl Receiver {
    pub fn receive(&mut self, bytes: &[u8]) -> Option<Frame> {
        let (_, _, sequence) = envelope(bytes)?;
        if sequence <= self.completed {
            return None;
        }
        let index = u16::from_le_bytes(bytes[32..34].try_into().ok()?) as usize;
        let count = u16::from_le_bytes(bytes[34..36].try_into().ok()?) as usize;
        if count == 0 || count > MAX_FRAME / PAYLOAD || index >= count {
            return None;
        }
        self.pending.retain(|p| {
            p.start.elapsed() < Duration::from_millis(500) && p.sequence > self.completed
        });
        if !self.pending.iter().any(|p| p.sequence == sequence) {
            if self.pending.len() >= 4 {
                self.pending.remove(0);
            }
            self.pending.push(Partial {
                sequence,
                count,
                chunks: vec![None; count],
                start: Instant::now(),
            });
        }
        let p = self.pending.iter_mut().find(|p| p.sequence == sequence)?;
        if p.count != count {
            return None;
        }
        p.chunks[index] = Some(bytes[HEADER..].to_vec());
        if p.chunks.iter().any(Option::is_none) {
            return None;
        }
        let data: Vec<_> = p
            .chunks
            .iter()
            .flat_map(|p| p.as_ref().unwrap().iter().copied())
            .collect();
        let frame = codec::decode(&data)?;
        if !frame.validate() {
            return None;
        }
        self.completed = sequence;
        self.pending.retain(|p| p.sequence > sequence);
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Frame {
        let pose = Pose {
            p: [0.; 3],
            q: [0., 0., 0., 1.],
        };
        Frame {
            map: 1,
            rig: 2,
            tick: 3,
            appearance: "uninstalled/outfit".into(),
            root: pose,
            bones: vec![Bone { index: 0, pose }],
            bodies: vec![
                Body {
                    pose,
                    velocity: [0.; 3],
                    angular: [0.; 3]
                };
                BODY_COUNT
            ],
            volumes: vec![],
        }
    }
    #[test]
    fn reorder_duplicate_and_loss() {
        let packets = packets(1, 2, 1, &frame()).unwrap();
        let mut rx = Receiver::default();
        for p in packets.iter().rev().skip(1) {
            assert!(rx.receive(p).is_none());
            assert!(rx.receive(p).is_none());
        }
        assert!(rx.receive(packets.last().unwrap()).is_some());
        for p in &packets {
            assert!(rx.receive(p).is_none());
        }
        let incomplete = super::packets(1, 2, 2, &frame()).unwrap();
        rx.receive(&incomplete[0]);
        let next = super::packets(1, 2, 3, &frame()).unwrap();
        assert_eq!(next.iter().filter_map(|p| rx.receive(p)).count(), 1);
        for p in &incomplete {
            assert!(rx.receive(p).is_none());
        }
    }
    #[test]
    fn rejects_bad_numbers_sizes_and_indices() {
        let mut f = frame();
        f.root.q = [0.; 4];
        assert!(!f.validate());
        f = frame();
        f.bodies[0].velocity[0] = f32::NAN;
        assert!(!f.validate());
        f = frame();
        f.volumes.push(Volume {
            body: 255,
            shape: Shape::Sphere {
                center: [0.; 3],
                radius: 1.,
            },
        });
        assert!(!f.validate());
        f = frame();
        f.bones.push(f.bones[0].clone());
        assert!(!f.validate());
        assert!(envelope(&[0; 2048]).is_none());
        assert_eq!(appearance_or_default("../../bad.glb"), DEFAULT_APPEARANCE);
    }
    #[test]
    fn binary_geometry_round_trip_and_truncation() {
        let mut f = frame();
        f.volumes = vec![
            Volume {
                body: 0,
                shape: Shape::Sphere {
                    center: [0.; 3],
                    radius: 0.1,
                },
            },
            Volume {
                body: 1,
                shape: Shape::Capsule {
                    center: [0.; 3],
                    axis: [0., 1., 0.],
                    half: 0.2,
                    radius: 0.1,
                },
            },
            Volume {
                body: 2,
                shape: Shape::Box {
                    pose: f.root,
                    half: [0.1; 3],
                    radius: 0.01,
                },
            },
            Volume {
                body: 3,
                shape: Shape::Triangle {
                    vertices: [[0.; 3], [0.1, 0., 0.], [0., 0.1, 0.]],
                    radius: 0.01,
                },
            },
        ];
        let data = codec::encode(&f);
        assert!(data.len() < 3000);
        assert_eq!(codec::encode(&codec::decode(&data).unwrap()), data);
        for n in 0..data.len() {
            assert!(codec::decode(&data[..n]).is_none());
        }
        let mut trailing = data;
        trailing.push(0);
        assert!(codec::decode(&trailing).is_none());
    }
    #[test]
    fn actual_loopback_datagrams_reassemble() {
        use std::net::UdpSocket;
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        a.connect(b.local_addr().unwrap()).unwrap();
        b.connect(a.local_addr().unwrap()).unwrap();
        b.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let packets = packets(12, 34, 1, &frame()).unwrap();
        for p in packets.iter().rev() {
            a.send(p).unwrap();
        }
        let mut rx = Receiver::default();
        let mut buffer = [0; 1500];
        let mut complete = 0;
        for _ in &packets {
            let n = b.recv(&mut buffer).unwrap();
            assert_eq!(envelope(&buffer[..n]).unwrap(), (12, 34, 1));
            if let Some(f) = rx.receive(&buffer[..n]) {
                assert_eq!(f.tick, 3);
                complete += 1;
            }
        }
        assert_eq!(complete, 1);
    }
}
