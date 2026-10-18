//! Triangle collision for the clean-room engine: a uniform grid over the
//! ground plane, answering the engine's segment casts.
use bevy::math::Vec3;
use bevy::platform::collections::HashMap;
use skate_lite_sys::{Collision, Hit};

/// Grid cell edge, game metres.
const CELL: f32 = 2.0;

/// Surface ids passed through to engine events (sounds pick by these).
pub(crate) const SURFACE_GROUND: i32 = 0;
pub(crate) const SURFACE_WOOD: i32 = 1;
pub(crate) const SURFACE_METAL: i32 = 2;

#[derive(Clone, Copy)]
struct Tri {
    a: Vec3,
    e1: Vec3,
    e2: Vec3,
    normal: Vec3,
    surface: i32,
}

#[derive(Default)]
pub(crate) struct Grid {
    tris: Vec<Tri>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn cell(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

impl Grid {
    pub(crate) fn add(&mut self, [a, b, c]: [Vec3; 3], surface: i32) {
        let (e1, e2) = (b - a, c - a);
        let normal = e1.cross(e2);
        if normal.length_squared() < 1e-12 {
            return;
        }
        let id = self.tris.len() as u32;
        self.tris.push(Tri {
            a,
            e1,
            e2,
            normal: normal.normalize(),
            surface,
        });
        let lo = a.min(b).min(c);
        let hi = a.max(b).max(c);
        for x in cell(lo.x)..=cell(hi.x) {
            for z in cell(lo.z)..=cell(hi.z) {
                self.cells.entry((x, z)).or_default().push(id);
            }
        }
    }

    pub(crate) fn add_quad(&mut self, [a, b, c, d]: [Vec3; 4], surface: i32) {
        self.add([a, b, c], surface);
        self.add([a, c, d], surface);
    }

    pub(crate) fn len(&self) -> usize {
        self.tris.len()
    }

    /// Nearest hit on the segment `origin .. origin + delta`; the normal faces the origin.
    pub(crate) fn cast(&self, origin: Vec3, delta: Vec3) -> Option<(f32, Vec3, i32)> {
        let end = origin + delta;
        let (lo, hi) = (origin.min(end), origin.max(end));
        let mut best: Option<(f32, Vec3, i32)> = None;
        for x in cell(lo.x)..=cell(hi.x) {
            for z in cell(lo.z)..=cell(hi.z) {
                let Some(ids) = self.cells.get(&(x, z)) else { continue };
                for &id in ids {
                    let t = &self.tris[id as usize];
                    let Some(f) = segment_triangle(origin, delta, t) else { continue };
                    if best.is_none_or(|(b, _, _)| f < b) {
                        let n = if t.normal.dot(delta) > 0.0 { -t.normal } else { t.normal };
                        best = Some((f, n, t.surface));
                    }
                }
            }
        }
        best
    }
}

/// Möller-Trumbore on a segment, both faces; returns the fraction 0..1.
fn segment_triangle(origin: Vec3, delta: Vec3, t: &Tri) -> Option<f32> {
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
    (0.0..=1.0).contains(&f).then_some(f)
}

impl Collision for Grid {
    fn raycast(&self, origin: [f32; 3], delta: [f32; 3]) -> Option<Hit> {
        let (o, d) = (Vec3::from_array(origin), Vec3::from_array(delta));
        self.cast(o, d).map(|(f, n, surface)| Hit {
            point: (o + d * f).to_array(),
            normal: n.to_array(),
            fraction: f,
            surface,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_nearest_face_and_faces_the_origin() {
        let mut g = Grid::default();
        g.add_quad(
            [
                Vec3::new(-10.0, 0.0, -10.0),
                Vec3::new(-10.0, 0.0, 10.0),
                Vec3::new(10.0, 0.0, 10.0),
                Vec3::new(10.0, 0.0, -10.0),
            ],
            SURFACE_GROUND,
        );
        g.add_quad(
            [
                Vec3::new(-1.0, 1.0, -1.0),
                Vec3::new(1.0, 1.0, -1.0),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(-1.0, 1.0, 1.0),
            ],
            SURFACE_WOOD,
        );
        let (f, n, s) = g.cast(Vec3::new(0.5, 3.0, 0.5), Vec3::new(0.0, -4.0, 0.0)).unwrap();
        assert!((f - 0.5).abs() < 1e-5 && n.y > 0.99 && s == SURFACE_WOOD);
        let (f, _, s) = g.cast(Vec3::new(5.0, 3.0, 5.0), Vec3::new(0.0, -4.0, 0.0)).unwrap();
        assert!((f - 0.75).abs() < 1e-5 && s == SURFACE_GROUND);
        assert!(g.cast(Vec3::new(50.0, 3.0, 5.0), Vec3::new(0.0, -4.0, 0.0)).is_none());
        let (_, n, _) = g.cast(Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 1.0, 0.0)).unwrap();
        assert!(n.y < -0.99, "normal faces a ray from below");
    }
}
