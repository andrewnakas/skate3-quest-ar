//! Version 5: one datagram per independently decodable stream update.
//! Position error <= 0.5 mm/axis within 32 m; larger offsets use full floats.
//! Quaternion uses smallest-three 10-bit components. Velocities use binary16.
use crate::{Body, Bone, Pose};
use half::f16;
pub const MTU: usize = 1200;
pub const MAGIC: &[u8; 8] = b"SK8NET05";
pub const HEADER: usize = 29;
pub const BODY: u8 = 1;
pub const POSE: u8 = 2;
#[derive(Clone, Debug, PartialEq)]
pub struct Packed {
    /// Source capture time in milliseconds; forwarding never retimestamps it.
    pub captured: u64,
    pub root: [u8; 16],
    pub enabled: u64,
    pub rows: Vec<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub struct BodyState {
    pub root: Pose,
    pub enabled: u64,
    pub bodies: Vec<Body>,
}
#[derive(Clone, Debug)]
pub struct PoseState {
    pub root: Pose,
    pub bones: Vec<Bone>,
}
pub fn header(session: u64, actor: u64, kind: u8, sequence: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(MTU);
    b.extend(MAGIC);
    b.extend(session.to_le_bytes());
    b.extend(actor.to_le_bytes());
    b.push(kind);
    b.extend(sequence.to_le_bytes());
    b
}
pub fn envelope(b: &[u8]) -> Option<(u64, u64, u8, u32)> {
    if b.len() < HEADER || b.len() > MTU || &b[..8] != MAGIC {
        return None;
    }
    Some((
        u64::from_le_bytes(b[8..16].try_into().ok()?),
        u64::from_le_bytes(b[16..24].try_into().ok()?),
        b[24],
        u32::from_le_bytes(b[25..29].try_into().ok()?),
    ))
}
pub struct Reader<'a>(pub &'a [u8]);
impl Reader<'_> {
    pub fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b = self.0.get(..N)?.try_into().ok()?;
        self.0 = &self.0[N..];
        Some(b)
    }
    pub fn byte(&mut self) -> Option<u8> {
        Some(self.take::<1>()?[0])
    }
    pub fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take()?))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take()?))
    }
}
fn quat(q: [f32; 4]) -> [u8; 4] {
    let largest = (0..4)
        .max_by(|&a, &b| q[a].abs().total_cmp(&q[b].abs()))
        .unwrap();
    let sign = if q[largest] < 0. { -1. } else { 1. };
    let mut bits = largest as u32;
    let mut shift = 2;
    for (i, v) in q.iter().enumerate() {
        if i != largest {
            let n = ((v * sign * std::f32::consts::SQRT_2 * 0.5 + 0.5) * 1023.)
                .round()
                .clamp(0., 1023.) as u32;
            bits |= n << shift;
            shift += 10;
        }
    }
    bits.to_le_bytes()
}
fn unquat(b: [u8; 4]) -> [f32; 4] {
    let bits = u32::from_le_bytes(b);
    let largest = (bits & 3) as usize;
    let mut q = [0.; 4];
    let mut shift = 2;
    let mut sum = 0.;
    for (i, v) in q.iter_mut().enumerate() {
        if i != largest {
            *v = (((bits >> shift) & 1023) as f32 / 1023. - 0.5) * std::f32::consts::SQRT_2;
            sum += *v * *v;
            shift += 10;
        }
    }
    q[largest] = (1. - sum).max(0.).sqrt();
    let norm = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    q.map(|v| v / norm)
}
fn root(p: Pose) -> [u8; 16] {
    let mut b = [0; 16];
    for i in 0..3 {
        b[i * 4..i * 4 + 4].copy_from_slice(&p.p[i].to_le_bytes());
    }
    b[12..].copy_from_slice(&quat(p.q));
    b
}
fn unroot(b: [u8; 16]) -> Option<Pose> {
    let p = Pose {
        p: std::array::from_fn(|i| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())),
        q: unquat(b[12..].try_into().unwrap()),
    };
    p.valid().then_some(p)
}
fn row_pose(b: &mut Vec<u8>, p: Pose, origin: [f32; 3]) {
    let relative = std::array::from_fn::<_, 3, _>(|i| p.p[i] - origin[i]);
    let small = relative.iter().all(|v| v.abs() <= 32.767);
    b.push(u8::from(!small));
    for v in relative {
        if small {
            b.extend(((v * 1000.).round() as i16).to_le_bytes());
        } else {
            b.extend(v.to_le_bytes());
        }
    }
    b.extend(quat(p.q));
}
fn read_pose(r: &mut Reader, origin: [f32; 3]) -> Option<Pose> {
    let wide = r.byte()?;
    if wide > 1 {
        return None;
    }
    let mut p = [0.; 3];
    for i in 0..3 {
        p[i] = origin[i]
            + if wide == 0 {
                i16::from_le_bytes(r.take()?) as f32 / 1000.
            } else {
                f32::from_le_bytes(r.take()?)
            };
    }
    let p = Pose {
        p,
        q: unquat(r.take()?),
    };
    p.valid().then_some(p)
}
fn rates(b: &mut Vec<u8>, v: [f32; 3]) {
    for f in v {
        b.extend(f16::from_f32(f).to_bits().to_le_bytes());
    }
}
fn read_rates(r: &mut Reader, bound: f32) -> Option<[f32; 3]> {
    let mut v = [0.; 3];
    for f in &mut v {
        *f = f16::from_bits(u16::from_le_bytes(r.take()?)).to_f32();
        if !f.is_finite() || f.abs() > bound {
            return None;
        }
    }
    Some(v)
}
impl Packed {
    pub fn same_state(&self, other: &Self) -> bool {
        self.root == other.root && self.enabled == other.enabled && self.rows == other.rows
    }
    pub fn body(s: &BodyState) -> Option<Self> {
        if !s.root.valid()
            || s.bodies.len() != 33
            || (s.enabled & !((1 << 63) | (1 << 62))) >> 33 != 0
        {
            return None;
        }
        let mut rows = Vec::with_capacity(33);
        for body in &s.bodies {
            if !body.pose.valid()
                || !body
                    .velocity
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 250.)
                || !body
                    .angular
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 500.)
            {
                return None;
            }
            let mut row = Vec::with_capacity(29);
            row_pose(&mut row, body.pose, s.root.p);
            rates(&mut row, body.velocity);
            rates(&mut row, body.angular);
            rows.push(row);
        }
        Some(Self {
            captured: 0,
            root: root(s.root),
            enabled: s.enabled,
            rows,
        })
    }
    pub fn pose(s: &PoseState) -> Option<Self> {
        if !s.root.valid() || s.bones.len() > 32 {
            return None;
        }
        let mut rows = Vec::with_capacity(s.bones.len());
        let mut seen = vec![];
        for bone in &s.bones {
            if bone.index >= 256 || !bone.pose.valid() || seen.contains(&bone.index) {
                return None;
            }
            seen.push(bone.index);
            let mut row = vec![bone.index as u8];
            row_pose(&mut row, bone.pose, [0.; 3]);
            rows.push(row);
        }
        Some(Self {
            captured: 0,
            root: root(s.root),
            enabled: 0,
            rows,
        })
    }
    pub fn unpack_body(&self) -> Option<BodyState> {
        let root = unroot(self.root)?;
        let mut bodies = vec![];
        if self.rows.len() != 33 || (self.enabled & !((1 << 63) | (1 << 62))) >> 33 != 0 {
            return None;
        }
        for row in &self.rows {
            let mut r = Reader(row);
            let body = Body {
                pose: read_pose(&mut r, root.p)?,
                velocity: read_rates(&mut r, 250.)?,
                angular: read_rates(&mut r, 500.)?,
            };
            if !r.0.is_empty() {
                return None;
            }
            bodies.push(body);
        }
        Some(BodyState {
            root,
            enabled: self.enabled,
            bodies,
        })
    }
    pub fn unpack_pose(&self) -> Option<PoseState> {
        let root = unroot(self.root)?;
        if self.rows.len() > 32 || self.enabled != 0 {
            return None;
        }
        let mut bones = vec![];
        for row in &self.rows {
            let mut r = Reader(row);
            let index = u16::from(r.byte()?);
            if bones.iter().any(|b: &Bone| b.index == index) {
                return None;
            }
            bones.push(Bone {
                index,
                pose: read_pose(&mut r, [0.; 3])?,
            });
            if !r.0.is_empty() {
                return None;
            }
        }
        Some(PoseState { root, bones })
    }
    pub fn position(&self) -> [f32; 3] {
        unroot(self.root).map_or([0.; 3], |p| p.p)
    }
}
pub fn delta(
    session: u64,
    actor: u64,
    kind: u8,
    seq: u32,
    state: &Packed,
    base: Option<(u32, &Packed)>,
) -> Vec<u8> {
    let base = base.filter(|(_, b)| b.rows.len() == state.rows.len());
    let mut out = header(session, actor, kind, seq);
    out.extend(base.map_or(0, |b| b.0).to_le_bytes());
    out.extend(state.captured.to_le_bytes());
    out.extend(state.root);
    out.extend(state.enabled.to_le_bytes());
    out.push(state.rows.len() as u8);
    let mask = state.rows.iter().enumerate().fold(0u64, |m, (i, row)| {
        if base.is_none_or(|(_, b)| b.rows[i] != *row) {
            m | (1 << i)
        } else {
            m
        }
    });
    out.extend(mask.to_le_bytes());
    for (i, row) in state.rows.iter().enumerate() {
        if mask & (1 << i) != 0 {
            out.extend(row);
        }
    }
    debug_assert!(out.len() <= MTU);
    out
}
pub fn baseline(data: &[u8]) -> Option<u32> {
    u32::from_le_bytes(data.get(HEADER..HEADER + 4)?.try_into().ok()?).into()
}
pub fn apply(data: &[u8], base: Option<&Packed>) -> Option<Packed> {
    let (_, _, kind, _) = envelope(data)?;
    if kind != BODY && kind != POSE {
        return None;
    }
    let mut r = Reader(&data[HEADER..]);
    let base_seq = r.u32()?;
    let captured = r.u64()?;
    let root = r.take()?;
    unroot(root)?;
    let enabled = r.u64()?;
    let count = r.byte()? as usize;
    if (kind == BODY && count != 33) || (kind == POSE && count > 32) {
        return None;
    }
    let mask = r.u64()?;
    if mask >> count != 0 {
        return None;
    }
    let mut rows = if base_seq != 0 {
        let b = base?;
        if b.rows.len() != count {
            return None;
        }
        b.rows.clone()
    } else {
        if mask != (1u64 << count) - 1 {
            return None;
        }
        vec![vec![]; count]
    };
    for (i, row) in rows.iter_mut().enumerate() {
        if mask & (1 << i) == 0 {
            continue;
        }
        let start = r.0;
        if kind == POSE {
            r.byte()?;
        }
        let wide = *r.0.first()?;
        if wide > 1 {
            return None;
        }
        let len = 1 + if wide == 0 { 6 } else { 12 } + 4 + if kind == BODY { 12 } else { 0 };
        r.0 = r.0.get(len..)?;
        *row = start[..start.len() - r.0.len()].to_vec();
    }
    if !r.0.is_empty() {
        return None;
    }
    let result = Packed {
        captured,
        root,
        enabled,
        rows,
    };
    if kind == BODY {
        result.unpack_body()?;
    } else {
        result.unpack_pose()?;
    }
    Some(result)
}
