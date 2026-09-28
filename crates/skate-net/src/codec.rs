//! Fixed little-endian scalars, bounded counts, and no generic deserializer allocations.
use super::*;
struct Writer(Vec<u8>);
impl Writer {
    fn byte(&mut self, v: u8) {
        self.0.push(v);
    }
    fn floats<const N: usize>(&mut self, v: [f32; N]) {
        for f in v {
            self.0.extend(f.to_le_bytes());
        }
    }
    fn pose(&mut self, p: Pose) {
        self.floats(p.p);
        self.floats(p.q);
    }
}
pub(super) fn encode(f: &Frame) -> Vec<u8> {
    let mut w = Writer(Vec::with_capacity(8192));
    for v in [f.map, f.rig, f.tick] {
        w.0.extend(v.to_le_bytes());
    }
    w.byte(f.appearance.len() as u8);
    w.0.extend(f.appearance.as_bytes());
    w.pose(f.root);
    w.byte(f.bones.len() as u8);
    for b in &f.bones {
        w.0.extend(b.index.to_le_bytes());
        w.pose(b.pose);
    }
    w.byte(f.bodies.len() as u8);
    for b in &f.bodies {
        w.pose(b.pose);
        w.floats(b.velocity);
        w.floats(b.angular);
    }
    w.byte(f.volumes.len() as u8);
    for v in &f.volumes {
        w.byte(v.body);
        match v.shape {
            Shape::Sphere { center, radius } => {
                w.byte(0);
                w.floats(center);
                w.floats([radius]);
            }
            Shape::Capsule {
                center,
                axis,
                half,
                radius,
            } => {
                w.byte(1);
                w.floats(center);
                w.floats(axis);
                w.floats([half, radius]);
            }
            Shape::Box { pose, half, radius } => {
                w.byte(2);
                w.pose(pose);
                w.floats(half);
                w.floats([radius]);
            }
            Shape::Triangle { vertices, radius } => {
                w.byte(3);
                for p in vertices {
                    w.floats(p);
                }
                w.floats([radius]);
            }
        }
    }
    w.0
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let v = self.0.get(..N)?.try_into().ok()?;
        self.0 = &self.0[N..];
        Some(v)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.bytes::<1>()?[0])
    }
    fn floats<const N: usize>(&mut self) -> Option<[f32; N]> {
        let mut v = [0.; N];
        for f in &mut v {
            *f = f32::from_le_bytes(self.bytes()?);
        }
        Some(v)
    }
    fn pose(&mut self) -> Option<Pose> {
        Some(Pose {
            p: self.floats()?,
            q: self.floats()?,
        })
    }
}
pub(super) fn decode(data: &[u8]) -> Option<Frame> {
    if data.len() > MAX_FRAME {
        return None;
    }
    let mut r = Reader(data);
    let map = u64::from_le_bytes(r.bytes()?);
    let rig = u64::from_le_bytes(r.bytes()?);
    let tick = u64::from_le_bytes(r.bytes()?);
    let count = r.byte()? as usize;
    if count > 128 {
        return None;
    }
    let appearance = std::str::from_utf8(r.0.get(..count)?).ok()?.to_string();
    r.0 = &r.0[count..];
    let root = r.pose()?;
    let count = r.byte()?;
    if count > 32 {
        return None;
    }
    let mut bones = Vec::with_capacity(count as usize);
    for _ in 0..count {
        bones.push(Bone {
            index: u16::from_le_bytes(r.bytes()?),
            pose: r.pose()?,
        });
    }
    let count = r.byte()?;
    if count as usize != BODY_COUNT {
        return None;
    }
    let mut bodies = Vec::with_capacity(BODY_COUNT);
    for _ in 0..count {
        bodies.push(Body {
            pose: r.pose()?,
            velocity: r.floats()?,
            angular: r.floats()?,
        });
    }
    let count = r.byte()?;
    if count > 64 {
        return None;
    }
    let mut volumes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let body = r.byte()?;
        let shape = match r.byte()? {
            0 => Shape::Sphere {
                center: r.floats()?,
                radius: r.floats::<1>()?[0],
            },
            1 => Shape::Capsule {
                center: r.floats()?,
                axis: r.floats()?,
                half: r.floats::<1>()?[0],
                radius: r.floats::<1>()?[0],
            },
            2 => Shape::Box {
                pose: r.pose()?,
                half: r.floats()?,
                radius: r.floats::<1>()?[0],
            },
            3 => Shape::Triangle {
                vertices: [r.floats()?, r.floats()?, r.floats()?],
                radius: r.floats::<1>()?[0],
            },
            _ => return None,
        };
        volumes.push(Volume { body, shape });
    }
    if !r.0.is_empty() {
        return None;
    }
    Some(Frame {
        map,
        rig,
        tick,
        appearance,
        root,
        bones,
        bodies,
        volumes,
    })
}
