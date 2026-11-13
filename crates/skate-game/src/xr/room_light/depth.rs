//! Depth for Roomtone from the room the app already knows: Space Setup's
//! planes, boxes and triangle mesh, the surfaces the skater rides.
//!
//! Roomtone places what the camera saw (and so lights the content from where
//! the room's surfaces are) when it is given depth images. The Depth API's
//! would need a GPU readback; the Scene's surfaces are already in memory, so
//! a small depth image is ray cast from them on the capture's thread, in
//! `XR_META_environment_depth`'s convention.
use super::super::spawn::ray_triangle;
use bevy::math::{Quat, Vec3};

/// The depth image's side, pixels (Roomtone samples about this many).
pub(crate) const SIZE: usize = 64;
/// Its half field of view as a tangent: wider than the camera's 73 degrees,
/// which is tilted down from the head's.
const TAN: f32 = 1.2;
const NEAR_M: f32 = 0.1;

/// The room's triangles (tracking space) under a bounding volume hierarchy.
pub(crate) struct Surfaces {
    triangles: Vec<[Vec3; 3]>,
    nodes: Vec<Node>,
}

struct Node {
    min: Vec3,
    max: Vec3,
    /// A leaf's triangles are `triangles[start..start + count]`; an inner
    /// node (count 0) has its children at `start` and `start + 1`.
    start: u32,
    count: u32,
}

const LEAF: usize = 4;

impl Surfaces {
    pub(crate) fn new(mut triangles: Vec<[Vec3; 3]>) -> Self {
        triangles.retain(|t| t.iter().all(|p| p.is_finite()));
        let mut nodes = Vec::new();
        if !triangles.is_empty() {
            nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, start: 0, count: 0 });
            let count = triangles.len();
            build(&mut triangles, &mut nodes, 0, 0, count);
        }
        Self { triangles, nodes }
    }

    pub(crate) fn len(&self) -> usize {
        self.triangles.len()
    }

    /// The distance along `direction` (unit) to the nearest surface.
    pub(crate) fn cast(&self, origin: Vec3, direction: Vec3) -> Option<f32> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut nearest = f32::INFINITY;
        let mut stack = [0u32; 64];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            // Slabs (an axis the ray runs along only has to hold its origin).
            let (mut enter, mut exit) = (0.0f32, nearest);
            for axis in 0..3 {
                let (o, d) = (origin[axis], direction[axis]);
                if d.abs() < 1e-12 {
                    if o < node.min[axis] || o > node.max[axis] {
                        exit = -1.0;
                    }
                    continue;
                }
                let (a, b) = ((node.min[axis] - o) / d, (node.max[axis] - o) / d);
                enter = enter.max(a.min(b));
                exit = exit.min(a.max(b));
            }
            if enter > exit {
                continue;
            }
            if node.count > 0 {
                for t in &self.triangles[node.start as usize..(node.start + node.count) as usize] {
                    if let Some(d) = ray_triangle(origin, direction, t[0], t[1], t[2]) {
                        nearest = nearest.min(d);
                    }
                }
            } else if top + 2 <= stack.len() {
                stack[top] = node.start;
                stack[top + 1] = node.start + 1;
                top += 2;
            }
        }
        nearest.is_finite().then_some(nearest)
    }

    /// A [`SIZE`] x [`SIZE`] depth image seen from `position` looking along
    /// `orientation`'s -Z: OpenGL depth over 65535 with an infinite far
    /// plane, rows from the bottom; 65535 where the room has no surface.
    pub(crate) fn depth_image(&self, position: Vec3, orientation: Quat, out: &mut Vec<u16>) -> View {
        out.clear();
        for row in 0..SIZE {
            for column in 0..SIZE {
                let (u, v) = ((column as f32 + 0.5) / SIZE as f32, (row as f32 + 0.5) / SIZE as f32);
                let along = Vec3::new(TAN * (2.0 * u - 1.0), TAN * (2.0 * v - 1.0), -1.0);
                let length = along.length();
                // The depth is along the view's axis, not the ray.
                let z = self.cast(position, orientation * (along / length)).map(|d| d / length);
                out.push(match z {
                    Some(z) if z > NEAR_M => ((1.0 - NEAR_M / z) * 65535.0).round().min(65534.0) as u16,
                    Some(_) => 0,
                    None => 65535,
                });
            }
        }
        View { near_z: NEAR_M, tan: TAN }
    }
}

/// A depth image's projection: symmetric, `tan` each way.
#[derive(Clone, Copy, Debug)]
pub(crate) struct View {
    pub near_z: f32,
    pub tan: f32,
}

fn build(triangles: &mut [[Vec3; 3]], nodes: &mut Vec<Node>, node: usize, start: usize, count: usize) {
    let slice = &mut triangles[start..start + count];
    let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for p in slice.iter().flatten() {
        min = min.min(*p);
        max = max.max(*p);
    }
    nodes[node].min = min;
    nodes[node].max = max;
    if count <= LEAF {
        nodes[node].start = start as u32;
        nodes[node].count = count as u32;
        return;
    }
    // Halve along the longest side, by the triangles' middles.
    let size = max - min;
    let axis = if size.x >= size.y && size.x >= size.z { 0 } else if size.y >= size.z { 1 } else { 2 };
    let middle = |t: &[Vec3; 3]| t[0][axis] + t[1][axis] + t[2][axis];
    let half = count / 2;
    slice.select_nth_unstable_by(half, |a, b| middle(a).total_cmp(&middle(b)));
    let children = nodes.len();
    for _ in 0..2 {
        nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, start: 0, count: 0 });
    }
    nodes[node].start = children as u32;
    build(triangles, nodes, children, start, half);
    build(triangles, nodes, children + 1, start + half, count - half);
}

/// A box room's walls, floor and ceiling as triangles (tests, the desktop preview).
pub(crate) fn box_room(min: Vec3, max: Vec3) -> Vec<[Vec3; 3]> {
    let corner = |i: usize| Vec3::new(
        if i & 1 == 0 { min.x } else { max.x },
        if i & 2 == 0 { min.y } else { max.y },
        if i & 4 == 0 { min.z } else { max.z },
    );
    [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]]
        .into_iter()
        .flat_map(|q| [[corner(q[0]), corner(q[1]), corner(q[2])], [corner(q[0]), corner(q[2]), corner(q[3])]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> Surfaces {
        // 4 x 2.5 x 6 m, the head's origin 1.5 m up in it; and a table.
        let mut triangles = box_room(Vec3::new(-2.0, -1.5, -3.0), Vec3::new(2.0, 1.0, 3.0));
        triangles.extend(box_room(Vec3::new(-0.5, -1.5, -2.0), Vec3::new(0.5, -0.8, -1.0)));
        Surfaces::new(triangles)
    }

    #[test]
    fn casts_find_the_nearest_surface() {
        let room = room();
        assert_eq!(room.len(), 24);
        let hit = |d: Vec3| room.cast(Vec3::ZERO, d.normalize()).unwrap();
        assert!((hit(Vec3::NEG_Z) - 3.0).abs() < 1e-4);
        assert!((hit(Vec3::X) - 2.0).abs() < 1e-4);
        assert!((hit(Vec3::Y) - 1.0).abs() < 1e-4);
        assert!((hit(Vec3::NEG_Y) - 1.5).abs() < 1e-4);
        // Down and ahead: the table's top, before the floor.
        let d = Vec3::new(0.0, -0.8, -1.5).normalize();
        assert!((room.cast(Vec3::ZERO, d).unwrap() - 0.8 / -d.y).abs() < 1e-4);
        assert!(Surfaces::new(Vec::new()).cast(Vec3::ZERO, Vec3::X).is_none());
    }

    #[test]
    fn many_triangles_agree_with_brute_force() {
        // A bumpy floor of 2 x 40 x 40 triangles.
        let height = |x: i32, z: i32| ((x * 7 + z * 13) % 5) as f32 * 0.05 - 1.5;
        let p = |x: i32, z: i32| Vec3::new(x as f32 * 0.2 - 4.0, height(x, z), z as f32 * 0.2 - 4.0);
        let mut triangles = Vec::new();
        for x in 0..40 {
            for z in 0..40 {
                triangles.push([p(x, z), p(x + 1, z), p(x + 1, z + 1)]);
                triangles.push([p(x, z), p(x + 1, z + 1), p(x, z + 1)]);
            }
        }
        let surfaces = Surfaces::new(triangles.clone());
        for i in 0..200 {
            let d = Vec3::new((i % 20) as f32 * 0.1 - 1.0, -1.0, (i / 20) as f32 * 0.2 - 1.0).normalize();
            let brute = triangles
                .iter()
                .filter_map(|t| ray_triangle(Vec3::ZERO, d, t[0], t[1], t[2]))
                .min_by(f32::total_cmp);
            let cast = surfaces.cast(Vec3::ZERO, d);
            assert_eq!(cast.is_some(), brute.is_some(), "ray {i}");
            if let (Some(a), Some(b)) = (cast, brute) {
                assert!((a - b).abs() < 1e-4, "ray {i}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn roomtone_places_the_room_from_the_depth_image() {
        let surfaces = room();
        let mut depth = Vec::new();
        let view = surfaces.depth_image(Vec3::ZERO, Quat::IDENTITY, &mut depth);
        assert_eq!(depth.len(), SIZE * SIZE);
        // The middle looks at the far wall, 3 m along the axis.
        let middle = depth[SIZE / 2 * SIZE + SIZE / 2] as f32 / 65535.0;
        assert!((view.near_z / (1.0 - middle) - 3.0).abs() < 0.05);
        // The bottom rows come first: the table's front 1 m ahead, then the ceiling 1 m up.
        let z = |row: usize| view.near_z / (1.0 - depth[row * SIZE + SIZE / 2] as f32 / 65535.0);
        let slope = view.tan * (1.0 - 1.0 / SIZE as f32);
        assert!((z(0) - 1.0).abs() < 0.02 && (z(SIZE - 1) - 1.0 / slope).abs() < 0.02, "{} {}", z(0), z(SIZE - 1));

        // A capture fed these depth images places the room's surfaces.
        let mut capture = roomtone_sys::Room::new();
        let feed = super::super::feed::Feed::default();
        feed.set_surfaces(Some(std::sync::Arc::new(surfaces)));
        roomtone_sys::synthetic::sweep_with(&mut capture, |_| [0.2, 0.2, 0.2], |capture, frame| {
            feed.add_frame(capture, frame);
        });
        assert!(capture.voxels() > 500, "{} voxels: {}", capture.voxels(), capture.summary());
    }
}
