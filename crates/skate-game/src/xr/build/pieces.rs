//! The skate pieces the player can place, as collision/render geometry in
//! piece-local game metres: +Y up, the base on y = 0, centred on the origin.
//!
//! Every quad uses the course winding (triangles 0,2,1 and 0,3,2; normal
//! `(q2 - q0) x (q1 - q0)`) facing out of the solid, because course collision
//! is one-sided. Ramps are height fields over z, extruded across x, so their
//! end caps are simple trapezoids even where the profile is concave.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::f32::consts::FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum PieceKind {
    Kicker,
    QuarterPipe,
    HalfPipe,
    Rail,
    Ledge,
    FunBox,
    Platform,
}

impl PieceKind {
    pub(crate) const ALL: [Self; 7] = [
        Self::Kicker,
        Self::QuarterPipe,
        Self::HalfPipe,
        Self::Rail,
        Self::Ledge,
        Self::FunBox,
        Self::Platform,
    ];

    pub(crate) fn color(self) -> Color {
        match self {
            Self::Kicker => Color::srgb(0.85, 0.55, 0.25),
            Self::QuarterPipe | Self::HalfPipe => Color::srgb(0.35, 0.6, 0.9),
            Self::Rail => Color::srgb(0.85, 0.85, 0.9),
            Self::Ledge => Color::srgb(0.6, 0.6, 0.62),
            Self::FunBox => Color::srgb(0.9, 0.35, 0.4),
            Self::Platform => Color::srgb(0.45, 0.75, 0.45),
        }
    }
}

/// Resize range for the piece's main dimension.
pub(crate) const MIN_SIZE: f32 = 0.5;
pub(crate) const MAX_SIZE: f32 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Piece {
    pub kind: PieceKind,
    /// Multiplier on the kind's main dimension (ramp height, rail length...).
    pub size: f32,
}

#[derive(Debug, Default)]
pub(crate) struct PieceGeometry {
    /// Solid faces.
    pub quads: Vec<[Vec3; 4]>,
    /// Faces of grind tubes: convex rail collision, see `ground`.
    pub rail_quads: Vec<[Vec3; 4]>,
    /// Grindable polylines (a tube's top centre, a ledge or coping edge).
    pub rails: Vec<Vec<Vec3>>,
    /// Off-board ledge edges (grab/climb/respawn queries).
    pub edges: Vec<(Vec3, Vec3)>,
}

impl PieceGeometry {
    pub(crate) fn all_quads(&self) -> impl Iterator<Item = &[Vec3; 4]> {
        self.quads.iter().chain(&self.rail_quads)
    }
}

impl Piece {
    pub(crate) fn new(kind: PieceKind) -> Self {
        Self { kind, size: 1.0 }
    }

    pub(crate) fn geometry(&self) -> PieceGeometry {
        let s = self.size.clamp(MIN_SIZE, MAX_SIZE);
        let mut g = PieceGeometry::default();
        match self.kind {
            PieceKind::Kicker => {
                let (length, height) = (2.5 * s, 0.8 * s);
                let profile: Vec<Vec2> = (0..=16)
                    .map(|i| {
                        let t = i as f32 / 16.0;
                        Vec2::new(-length * 0.5 + length * t, height * (1.0 - t) * (1.0 - t))
                    })
                    .collect();
                extrude(&mut g, &profile, 2.0);
                g.edges.push((Vec3::new(-1.0, height, -length * 0.5), Vec3::new(1.0, height, -length * 0.5)));
            }
            PieceKind::QuarterPipe => {
                let (radius, deck, width) = (2.0 * s, 1.0, 4.0);
                let z0 = -(radius + deck) * 0.5;
                let mut profile = vec![Vec2::new(z0, radius)];
                profile.extend(transition(z0 + deck, radius, 1.0));
                extrude(&mut g, &profile, width);
                coping(&mut g, z0 + deck, radius, width);
            }
            PieceKind::HalfPipe => {
                let (radius, deck, flat, width) = (2.0 * s, 1.0, 3.0, 5.0);
                let half = deck + radius + flat * 0.5;
                let mut profile = vec![Vec2::new(-half, radius)];
                profile.extend(transition(-half + deck, radius, 1.0));
                profile.extend(transition(half - deck, radius, -1.0).into_iter().rev());
                profile.push(Vec2::new(half, radius));
                extrude(&mut g, &profile, width);
                coping(&mut g, -half + deck, radius, width);
                coping(&mut g, half - deck, radius, width);
            }
            PieceKind::Rail => {
                let (length, top, tube) = (6.0 * s, 0.5, 0.08);
                let x = length * 0.5;
                let mut tube_faces = Vec::new();
                box_quads(&mut tube_faces, Vec3::new(-x, top - tube, -tube * 0.5), Vec3::new(x, top, tube * 0.5));
                g.rail_quads = tube_faces;
                for px in [-x + 0.6, x - 0.6] {
                    box_quads(&mut g.quads, Vec3::new(px - 0.04, 0.0, -0.04), Vec3::new(px + 0.04, top - tube, 0.04));
                }
                g.rails.push(vec![Vec3::new(-x, top, 0.0), Vec3::new(x, top, 0.0)]);
            }
            PieceKind::Ledge => grind_box(&mut g, Vec3::new(5.0 * s, 0.5, 0.8), [true, false, true, false]),
            PieceKind::FunBox => grind_box(&mut g, Vec3::new(3.0 * s, 0.5, 2.0), [true; 4]),
            PieceKind::Platform => grind_box(&mut g, Vec3::new(4.0 * s, 1.2, 4.0), [true; 4]),
        }
        g
    }
}

/// Concave circular transition from a lip at (`lip_z`, `radius`) down to the
/// floor, heading towards +z (`dir` 1) or -z (`dir` -1). Includes the lip.
fn transition(lip_z: f32, radius: f32, dir: f32) -> Vec<Vec2> {
    // Circle centred level with the lip, one radius out over the floor.
    (0..=16)
        .map(|i| {
            let a = FRAC_PI_2 * i as f32 / 16.0;
            Vec2::new(lip_z + dir * radius * (1.0 - a.cos()), radius * (1.0 - a.sin()))
        })
        .collect()
}

fn coping(g: &mut PieceGeometry, z: f32, height: f32, width: f32) {
    let (a, b) = (Vec3::new(-width * 0.5, height, z), Vec3::new(width * 0.5, height, z));
    g.rails.push(vec![a, b]);
    g.edges.push((a, b));
}

/// A box whose top edges (in order -z, +x, +z, -x) are optionally grindable.
fn grind_box(g: &mut PieceGeometry, size: Vec3, grind: [bool; 4]) {
    let h = size * 0.5;
    box_quads(&mut g.quads, Vec3::new(-h.x, 0.0, -h.z), Vec3::new(h.x, size.y, h.z));
    let c = [
        Vec3::new(-h.x, size.y, -h.z),
        Vec3::new(h.x, size.y, -h.z),
        Vec3::new(h.x, size.y, h.z),
        Vec3::new(-h.x, size.y, h.z),
    ];
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        g.edges.push((a, b));
        if grind[i] {
            g.rails.push(vec![a, b]);
        }
    }
}

/// Extrude a height field `y(z)` (points in increasing z) across x in
/// [-width/2, width/2], solid down to y = 0. No bottom face: pieces sit on a
/// surface.
fn extrude(g: &mut PieceGeometry, profile: &[Vec2], width: f32) {
    let x = width * 0.5;
    let at = |x: f32, p: Vec2| Vec3::new(x, p.y, p.x);
    // Top surface, facing up (or towards the open side where vertical).
    for pair in profile.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        let edge = q - p;
        let outward = Vec3::new(0.0, edge.x, -edge.y);
        push(&mut g.quads, [at(-x, p), at(x, p), at(x, q), at(-x, q)], outward);
        // End caps: the trapezoid under this segment, on both sides.
        for (side, n) in [(-x, Vec3::NEG_X), (x, Vec3::X)] {
            let cap = [
                Vec3::new(side, 0.0, p.x),
                Vec3::new(side, 0.0, q.x),
                at(side, q),
                at(side, p),
            ];
            push(&mut g.quads, cap, n);
        }
    }
    // Back walls where the profile starts or ends above the base.
    let (first, last) = (profile[0], profile[profile.len() - 1]);
    push(&mut g.quads, [at(-x, first), at(x, first), at(x, first.with_y(0.0)), at(-x, first.with_y(0.0))], Vec3::NEG_Z);
    push(&mut g.quads, [at(-x, last), at(x, last), at(x, last.with_y(0.0)), at(-x, last.with_y(0.0))], Vec3::Z);
}

/// Six faces of an axis-aligned box, bottom included (rail tubes float).
fn box_quads(out: &mut Vec<[Vec3; 4]>, min: Vec3, max: Vec3) {
    let c = |x: bool, y: bool, z: bool| {
        Vec3::new(
            if x { max.x } else { min.x },
            if y { max.y } else { min.y },
            if z { max.z } else { min.z },
        )
    };
    let faces = [
        ([c(false, true, false), c(true, true, false), c(true, true, true), c(false, true, true)], Vec3::Y),
        ([c(false, false, false), c(true, false, false), c(true, false, true), c(false, false, true)], Vec3::NEG_Y),
        ([c(true, false, false), c(true, true, false), c(true, true, true), c(true, false, true)], Vec3::X),
        ([c(false, false, false), c(false, true, false), c(false, true, true), c(false, false, true)], Vec3::NEG_X),
        ([c(false, false, true), c(true, false, true), c(true, true, true), c(false, true, true)], Vec3::Z),
        ([c(false, false, false), c(true, false, false), c(true, true, false), c(false, true, false)], Vec3::NEG_Z),
    ];
    for (quad, n) in faces {
        push(out, quad, n);
    }
}

/// Course-winding normal of a quad (not normalised).
pub(crate) fn quad_normal(q: &[Vec3; 4]) -> Vec3 {
    (q[2] - q[0]).cross(q[1] - q[0])
}

/// Add `quad` facing `outward`, dropping degenerate (zero-area) quads.
fn push(out: &mut Vec<[Vec3; 4]>, quad: [Vec3; 4], outward: Vec3) {
    let n = quad_normal(&quad) + (quad[3] - quad[0]).cross(quad[2] - quad[0]);
    if n.length_squared() < 1e-10 {
        return;
    }
    out.push(if n.dot(outward) >= 0.0 {
        quad
    } else {
        [quad[0], quad[3], quad[2], quad[1]]
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centroid(g: &PieceGeometry) -> Vec3 {
        let points: Vec<Vec3> = g.all_quads().flatten().copied().collect();
        points.iter().sum::<Vec3>() / points.len() as f32
    }

    #[test]
    fn every_piece_has_upward_tops_and_no_downward_ramps() {
        for kind in PieceKind::ALL {
            for size in [MIN_SIZE, 1.0, MAX_SIZE] {
                let g = Piece { kind, size }.geometry();
                assert!(!g.quads.is_empty(), "{kind:?}");
                let up = g
                    .all_quads()
                    .filter(|q| quad_normal(q).normalize().y > 0.9)
                    .count();
                assert!(up > 0, "{kind:?} has something to land on");
                for q in g.all_quads() {
                    assert!(q.iter().all(|p| p.is_finite()));
                    assert!(q.iter().all(|p| p.y >= -1e-5), "{kind:?} stays above its base");
                }
            }
        }
    }

    #[test]
    fn box_faces_point_away_from_the_centre() {
        for kind in [PieceKind::Ledge, PieceKind::FunBox, PieceKind::Platform] {
            let g = Piece::new(kind).geometry();
            let c = centroid(&g);
            for q in &g.quads {
                let mid = q.iter().sum::<Vec3>() / 4.0;
                assert!(quad_normal(q).dot(mid - c) > 0.0, "{kind:?} face {q:?}");
            }
        }
    }

    #[test]
    fn ramp_surfaces_face_up_or_into_the_open() {
        let g = Piece::new(PieceKind::QuarterPipe).geometry();
        // Transition faces (not caps, not walls) have a +y or +z component.
        for q in &g.quads {
            let n = quad_normal(q).normalize();
            if n.x.abs() < 0.5 && n.y > -0.1 && q.iter().any(|p| p.y > 0.01) {
                assert!(n.y > -1e-3, "transition faces never point down: {n:?}");
            }
        }
        let kicker = Piece::new(PieceKind::Kicker).geometry();
        let ramp_up = kicker
            .quads
            .iter()
            .filter(|q| {
                let n = quad_normal(q).normalize();
                n.y > 0.1 && n.z > 0.05
            })
            .count();
        assert!(ramp_up >= 8, "kicker ramp faces lean towards the run-in");
    }

    #[test]
    fn grindable_pieces_have_rails_on_their_top() {
        for (kind, rails) in [
            (PieceKind::Rail, 1),
            (PieceKind::QuarterPipe, 1),
            (PieceKind::HalfPipe, 2),
            (PieceKind::Ledge, 2),
            (PieceKind::FunBox, 4),
            (PieceKind::Platform, 4),
            (PieceKind::Kicker, 0),
        ] {
            let g = Piece::new(kind).geometry();
            assert_eq!(g.rails.len(), rails, "{kind:?}");
            let top = g.all_quads().flatten().map(|p| p.y).fold(0.0, f32::max);
            for rail in &g.rails {
                assert!(rail.len() >= 2);
                assert!(rail.iter().all(|p| (p.y - top).abs() < 1e-4), "{kind:?} rail sits on top");
            }
        }
    }

    #[test]
    fn size_scales_the_main_dimension() {
        let small = Piece { kind: PieceKind::Rail, size: 1.0 }.geometry();
        let big = Piece { kind: PieceKind::Rail, size: 2.0 }.geometry();
        let len = |g: &PieceGeometry| g.rails[0][0].distance(g.rails[0][1]);
        assert!((len(&big) - 2.0 * len(&small)).abs() < 1e-4);
    }
}
