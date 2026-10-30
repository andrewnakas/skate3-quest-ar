//! The smoothed surface as triangles with smooth normals, and a collider for them.
use crate::smooth::Surface;
use glam::{IVec2, Vec3};
use std::collections::HashMap;

/// Triangles, each with one normal per corner (smooth, except across creases).
#[derive(Clone, Default, Debug)]
pub struct TerrainMesh {
    pub triangles: Vec<[Vec3; 3]>,
    pub normals: Vec<[Vec3; 3]>,
}

/// One triangle per three valid cell centres, two per full 2x2 block, split along the diagonal
/// with the smaller height difference (so a ledge's edge follows the cells). A corner whose
/// smooth normal is more than `crease_deg` off the face's keeps the face normal: the top of a
/// ledge stays flat right up to its edge and the wall below it is a wall.
pub fn mesh(s: &Surface, crease_deg: f32) -> TerrainMesh {
    let m = &s.map;
    let crease_cos = crease_deg.to_radians().cos();
    let mut out = TerrainMesh::default();
    let at = |i: usize, j: usize| j * m.width + i;
    let point = |i: usize, j: usize| {
        let c = m.centre(i, j);
        Vec3::new(c.x, m.z[at(i, j)], c.y)
    };
    let mut push = |corners: [(usize, usize); 3]| {
        let p = corners.map(|(i, j)| point(i, j));
        let mut face = (p[1] - p[0]).cross(p[2] - p[0]);
        if face.length_squared() < 1e-12 {
            return;
        }
        face = face.normalize();
        if face.y < 0.0 {
            face = -face;
        }
        let n = corners.map(|(i, j)| {
            let v = s.normal(at(i, j));
            if v.dot(face) < crease_cos { face } else { v }
        });
        out.triangles.push(p);
        out.normals.push(n);
    };
    for j in 0..m.depth.saturating_sub(1) {
        for i in 0..m.width.saturating_sub(1) {
            let c = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)];
            let ok = c.map(|(i, j)| !m.z[at(i, j)].is_nan());
            let z = c.map(|(i, j)| m.z[at(i, j)]);
            let [a, b, cc, d] = c;
            match ok {
                [true, true, true, true] => {
                    if (z[0] - z[3]).abs() <= (z[1] - z[2]).abs() {
                        push([a, cc, d]);
                        push([a, d, b]);
                    } else {
                        push([a, cc, b]);
                        push([b, cc, d]);
                    }
                }
                [false, true, true, true] => push([b, cc, d]),
                [true, false, true, true] => push([a, cc, d]),
                [true, true, false, true] => push([a, d, b]),
                [true, true, true, false] => push([a, cc, b]),
                _ => {}
            }
        }
    }
    out
}

struct Tri {
    a: Vec3,
    e1: Vec3,
    e2: Vec3,
    face: Vec3,
    n: [Vec3; 3],
}

/// Segment casts against a [`TerrainMesh`], answering with the interpolated (smooth) normal.
pub struct Collider {
    tris: Vec<Tri>,
    buckets: HashMap<IVec2, Vec<u32>>,
    bucket: f32,
}

impl Collider {
    pub fn new(mesh: &TerrainMesh) -> Self {
        let bucket = 0.5;
        let mut tris = Vec::with_capacity(mesh.triangles.len());
        let mut buckets: HashMap<IVec2, Vec<u32>> = HashMap::new();
        for (t, n) in mesh.triangles.iter().zip(&mesh.normals) {
            let [a, b, c] = *t;
            let (e1, e2) = (b - a, c - a);
            let face = e1.cross(e2);
            if face.length_squared() < 1e-12 {
                continue;
            }
            let id = tris.len() as u32;
            tris.push(Tri { a, e1, e2, face: face.normalize(), n: *n });
            let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
            let (l, h) = (key(lo, bucket), key(hi, bucket));
            for x in l.x..=h.x {
                for z in l.y..=h.y {
                    buckets.entry(IVec2::new(x, z)).or_default().push(id);
                }
            }
        }
        Self { tris, buckets, bucket }
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    pub fn triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.tris.iter().map(|t| [t.a, t.a + t.e1, t.a + t.e2])
    }

    /// A long cast (a pointer ray, a drop search) in half-metre steps, so it only looks in the
    /// buckets it passes through.
    pub fn cast_far(&self, origin: Vec3, delta: Vec3) -> Option<(f32, Vec3)> {
        let steps = (delta.length() / self.bucket).ceil().max(1.0) as usize;
        (0..steps).find_map(|k| {
            let (a, b) = (k as f32 / steps as f32, (k + 1) as f32 / steps as f32);
            self.cast(origin + delta * a, delta * (b - a)).map(|(f, n)| (a + f * (b - a), n))
        })
    }

    /// The nearest hit on `origin .. origin + delta`: (fraction 0..1, unit normal facing the origin).
    pub fn cast(&self, origin: Vec3, delta: Vec3) -> Option<(f32, Vec3)> {
        let end = origin + delta;
        let (l, h) = (key(origin.min(end), self.bucket), key(origin.max(end), self.bucket));
        let mut best: Option<(f32, u32, f32, f32)> = None;
        for x in l.x..=h.x {
            for z in l.y..=h.y {
                let Some(ids) = self.buckets.get(&IVec2::new(x, z)) else { continue };
                for &id in ids {
                    let Some((f, u, v)) = segment_triangle(origin, delta, &self.tris[id as usize]) else { continue };
                    if best.is_none_or(|b| f < b.0) {
                        best = Some((f, id, u, v));
                    }
                }
            }
        }
        let (f, id, u, v) = best?;
        let t = &self.tris[id as usize];
        let mut n = (t.n[0] * (1.0 - u - v) + t.n[1] * u + t.n[2] * v).normalize_or(t.face);
        if t.face.dot(delta) > 0.0 {
            n = -n;
        }
        Some((f, n))
    }
}

fn key(p: Vec3, bucket: f32) -> IVec2 {
    IVec2::new((p.x / bucket).floor() as i32, (p.z / bucket).floor() as i32)
}

/// Möller-Trumbore on a segment, both faces: (fraction, u, v).
fn segment_triangle(origin: Vec3, delta: Vec3, t: &Tri) -> Option<(f32, f32, f32)> {
    let p = delta.cross(t.e2);
    let det = t.e1.dot(p);
    if det.abs() < 1e-10 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - t.a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(t.e1);
    let v = delta.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let f = t.e2.dot(q) * inv;
    (0.0..=1.0).contains(&f).then_some((f, u, v))
}
