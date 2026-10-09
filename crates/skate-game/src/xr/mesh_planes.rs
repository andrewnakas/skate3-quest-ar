//! Flat surfaces found in the room's scanned triangle mesh.
//!
//! Space Setup labels the floor, walls and the furniture it recognises, but a
//! room has more flat things than that: steps, benches, boxes, shelves, the
//! seat of a chair. They are all in the GLOBAL_MESH, as lumpy triangles. This
//! groups the near-horizontal triangles into connected patches of one height
//! and describes each patch by its outline (convex hull) and the smallest
//! rectangle around it, which is what the skater rides and grinds.
//!
//! Pure geometry over tracking-space points, so it is tested off-device.
use bevy::prelude::*;
use std::collections::HashMap;

/// Normals within ~18 degrees of vertical count as horizontal.
const HORIZONTAL_MIN_COS: f32 = 0.95;
/// Triangles meeting at a vertex join one patch when their heights differ by
/// no more than this, metres.
const JOIN_HEIGHT_M: f32 = 0.03;
/// Vertices closer than this are the same vertex (the scan's mesh is not
/// always shared-vertex), metres.
const WELD_M: f32 = 0.01;
/// Smallest patch kept: about a 25 cm square.
pub(crate) const MIN_AREA_M2: f32 = 0.06;
/// A patch this close below another one that covers it is that thing's
/// underside (the scan winding is not reliably outward), metres.
const UNDERSIDE_GAP_M: f32 = 0.25;
/// A patch fills at least this much of its rectangle to be ridden as that
/// rectangle; emptier ones (L-shapes, scattered clutter) are only shown.
const SOLID_FILL: f32 = 0.7;
/// More than this many patches is clutter; the biggest are kept.
const MAX_PLANES: usize = 64;

/// One flat patch of the room mesh, in tracking space.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FitPlane {
    /// Area-weighted mean height of the patch.
    pub height: f32,
    /// Convex outline in the horizontal plane (x, z), counter-clockwise.
    pub hull: Vec<Vec2>,
    /// Summed triangle area, m^2.
    pub area: f32,
    /// Smallest enclosing rectangle: centre (x, z), yaw about +Y, half sizes
    /// along the rotated X and Z.
    pub centre: Vec2,
    pub yaw: f32,
    pub half: Vec2,
    /// Fills its rectangle well enough to ride as that rectangle.
    pub solid: bool,
}

impl FitPlane {
    /// The rectangle as a plane pose (+Y normal) for collision and overlays.
    pub(crate) fn pose(&self) -> Transform {
        Transform {
            translation: Vec3::new(self.centre.x, self.height, self.centre.y),
            rotation: Quat::from_rotation_y(self.yaw),
            scale: Vec3::ONE,
        }
    }

    pub(crate) fn contains(&self, p: Vec2) -> bool {
        inside_convex(&self.hull, p)
    }
}

/// Horizontal patches of a triangle mesh whose vertices are already in
/// tracking space, biggest first.
pub(crate) fn fit_planes(vertices: &[Vec3], indices: &[u32]) -> Vec<FitPlane> {
    // Weld coincident vertices so neighbouring triangles find each other.
    let mut welded = HashMap::new();
    let ids: Vec<u32> = vertices
        .iter()
        .map(|v| {
            let key = (v / WELD_M).round().as_ivec3();
            let next = welded.len() as u32;
            *welded.entry(key).or_insert(next)
        })
        .collect();

    // Horizontal triangles: (welded corners, height, area).
    let mut flat = Vec::new();
    for t in indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|i| vertices.get(t[i] as usize).copied());
        let (Some(a), Some(b), Some(c)) = (a, b, c) else { continue };
        let n = (b - a).cross(c - a);
        let double_area = n.length();
        if double_area < 1e-8 || n.y.abs() / double_area < HORIZONTAL_MIN_COS {
            continue;
        }
        flat.push((
            [ids[t[0] as usize], ids[t[1] as usize], ids[t[2] as usize]],
            [a, b, c],
            (a.y + b.y + c.y) / 3.0,
            double_area * 0.5,
        ));
    }

    // Union triangles that share a welded vertex at (nearly) the same height.
    let mut parent: Vec<usize> = (0..flat.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut by_vertex: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, (corners, ..)) in flat.iter().enumerate() {
        for &v in corners {
            by_vertex.entry(v).or_default().push(i);
        }
    }
    for tris in by_vertex.values() {
        for (k, &i) in tris.iter().enumerate() {
            for &j in &tris[k + 1..] {
                if (flat[i].2 - flat[j].2).abs() <= JOIN_HEIGHT_M {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
    }

    // Gather each patch.
    let mut patches: HashMap<usize, (f32, f32, Vec<Vec2>)> = HashMap::new();
    for i in 0..flat.len() {
        let root = find(&mut parent, i);
        let (_, corners, height, area) = flat[i];
        let entry = patches.entry(root).or_insert((0.0, 0.0, Vec::new()));
        entry.0 += area;
        entry.1 += area * height;
        entry.2.extend(corners.iter().map(|p| Vec2::new(p.x, p.z)));
    }
    let mut planes: Vec<FitPlane> = patches
        .into_values()
        .filter(|(area, ..)| *area >= MIN_AREA_M2)
        .filter_map(|(area, weighted, points)| {
            let hull = convex_hull(points);
            if hull.len() < 3 {
                return None;
            }
            let (centre, yaw, half) = min_area_rect(&hull);
            let rect_area = 4.0 * half.x * half.y;
            Some(FitPlane {
                height: weighted / area,
                solid: rect_area > 0.0 && area / rect_area >= SOLID_FILL,
                hull,
                area,
                centre,
                yaw,
                half,
            })
        })
        .collect();
    planes.sort_by(|a, b| b.area.total_cmp(&a.area));

    // Drop undersides: a patch with another one just above that covers it.
    let undersides: Vec<bool> = planes
        .iter()
        .map(|p| {
            let c = polygon_centroid(&p.hull);
            planes.iter().any(|q| {
                let gap = q.height - p.height;
                gap > JOIN_HEIGHT_M && gap <= UNDERSIDE_GAP_M && q.contains(c)
            })
        })
        .collect();
    let mut kept: Vec<FitPlane> = planes
        .into_iter()
        .zip(undersides)
        .filter_map(|(p, under)| (!under).then_some(p))
        .collect();
    kept.truncate(MAX_PLANES);
    kept
}

/// Andrew's monotone chain; counter-clockwise, no repeated end point.
pub(crate) fn convex_hull(mut points: Vec<Vec2>) -> Vec<Vec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<Vec2> = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Smallest rectangle around a convex polygon (one side lies on a hull
/// edge): centre, yaw about +Y (the rectangle's X axis is `(cos, -sin)` in
/// x/z, matching `Quat::from_rotation_y`), half sizes.
pub(crate) fn min_area_rect(hull: &[Vec2]) -> (Vec2, f32, Vec2) {
    let mut best = (f32::MAX, Vec2::ZERO, 0.0, Vec2::ZERO);
    for i in 0..hull.len() {
        let edge = hull[(i + 1) % hull.len()] - hull[i];
        if edge.length_squared() < 1e-12 {
            continue;
        }
        let u = edge.normalize();
        let w = u.perp();
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for &p in hull {
            let q = Vec2::new(p.dot(u), p.dot(w));
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let size = hi - lo;
        let area = size.x * size.y;
        if area < best.0 {
            let mid = (lo + hi) * 0.5;
            // Rotation about +Y by yaw maps local X (1,0,0) to (cos, 0, -sin).
            let yaw = (-u.y).atan2(u.x);
            best = (area, u * mid.x + w * mid.y, yaw, size * 0.5);
        }
    }
    (best.1, best.2, best.3)
}

fn polygon_centroid(poly: &[Vec2]) -> Vec2 {
    poly.iter().copied().sum::<Vec2>() / poly.len().max(1) as f32
}

fn inside_convex(hull: &[Vec2], p: Vec2) -> bool {
    hull.len() >= 3
        && (0..hull.len()).all(|i| (hull[(i + 1) % hull.len()] - hull[i]).perp_dot(p - hull[i]) >= -1e-6)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of horizontal quads (two triangles each) at `y`.
    fn slab(min: Vec2, max: Vec2, y: f32, cells: usize, verts: &mut Vec<Vec3>, idx: &mut Vec<u32>) {
        let step = (max - min) / cells as f32;
        let base = verts.len() as u32;
        for j in 0..=cells {
            for i in 0..=cells {
                let p = min + step * Vec2::new(i as f32, j as f32);
                verts.push(Vec3::new(p.x, y, p.y));
            }
        }
        let row = cells as u32 + 1;
        for j in 0..cells as u32 {
            for i in 0..cells as u32 {
                let a = base + j * row + i;
                idx.extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
            }
        }
    }

    #[test]
    fn finds_a_step_and_a_bench() {
        let (mut v, mut i) = (Vec::new(), Vec::new());
        slab(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.4), 0.18, 4, &mut v, &mut i);
        slab(Vec2::new(2.0, 2.0), Vec2::new(3.5, 2.4), 0.45, 6, &mut v, &mut i);
        // A tiny patch of clutter that should be ignored.
        slab(Vec2::new(5.0, 5.0), Vec2::new(5.1, 5.1), 0.3, 1, &mut v, &mut i);
        let planes = fit_planes(&v, &i);
        assert_eq!(planes.len(), 2, "{planes:?}");
        let bench = &planes[0];
        assert!((bench.height - 0.45).abs() < 1e-4);
        assert!((bench.area - 0.6).abs() < 1e-3);
        assert!(bench.solid);
        let long = bench.half.max_element();
        let short = bench.half.min_element();
        assert!((long - 0.75).abs() < 1e-3 && (short - 0.2).abs() < 1e-3, "{:?}", bench.half);
        assert!((bench.centre - Vec2::new(2.75, 2.2)).length() < 1e-3);
        assert!((planes[1].height - 0.18).abs() < 1e-4);
    }

    #[test]
    fn separate_heights_stay_separate() {
        let (mut v, mut i) = (Vec::new(), Vec::new());
        // Two touching slabs 10 cm apart in height: a step, not one plane.
        slab(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.5), 0.0, 4, &mut v, &mut i);
        slab(Vec2::new(0.0, 0.5), Vec2::new(1.0, 1.0), 0.1, 4, &mut v, &mut i);
        assert_eq!(fit_planes(&v, &i).len(), 2);
    }

    #[test]
    fn walls_and_undersides_are_not_planes() {
        let (mut v, mut i) = (Vec::new(), Vec::new());
        slab(Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), 0.75, 4, &mut v, &mut i);
        // The table's underside, 3 cm lower.
        slab(Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0), 0.72, 4, &mut v, &mut i);
        // A wall.
        let b = v.len() as u32;
        v.extend([Vec3::new(3.0, 0.0, 0.0), Vec3::new(3.0, 2.0, 0.0), Vec3::new(3.0, 0.0, 2.0), Vec3::new(3.0, 2.0, 2.0)]);
        i.extend([b, b + 1, b + 2, b + 2, b + 1, b + 3]);
        let planes = fit_planes(&v, &i);
        assert_eq!(planes.len(), 1, "{planes:?}");
        assert!((planes[0].height - 0.75).abs() < 1e-4);
    }

    #[test]
    fn rotated_rectangle_and_pose() {
        let yaw = 0.5f32;
        let r = Quat::from_rotation_y(yaw);
        let corners = [(-0.6, -0.2), (0.6, -0.2), (0.6, 0.2), (-0.6, 0.2)]
            .map(|(x, z)| r * Vec3::new(x, 0.0, z) + Vec3::new(1.0, 0.0, 2.0))
            .map(|p| Vec2::new(p.x, p.z));
        let (centre, got_yaw, half) = min_area_rect(&convex_hull(corners.to_vec()));
        assert!((centre - Vec2::new(1.0, 2.0)).length() < 1e-4);
        let plane = FitPlane { height: 0.0, hull: vec![], area: 0.0, centre, yaw: got_yaw, half, solid: true };
        // Every corner lies on the rectangle's boundary.
        for c in corners {
            let local = plane.pose().rotation.inverse() * (Vec3::new(c.x, 0.0, c.y) - plane.pose().translation);
            assert!((local.x.abs() - half.x).abs() < 1e-3 && (local.z.abs() - half.y).abs() < 1e-3, "{local:?} {half:?}");
        }
    }

    #[test]
    fn l_shape_is_not_solid() {
        let (mut v, mut i) = (Vec::new(), Vec::new());
        slab(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.25), 0.4, 4, &mut v, &mut i);
        slab(Vec2::new(0.0, 0.25), Vec2::new(0.25, 1.0), 0.4, 4, &mut v, &mut i);
        let planes = fit_planes(&v, &i);
        assert_eq!(planes.len(), 1);
        assert!(!planes[0].solid);
    }
}
