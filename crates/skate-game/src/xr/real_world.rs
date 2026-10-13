//! Real-world collision: the Scene floor, walls and furniture become surfaces
//! the skater rides, mapped into the game world through the rig. Furniture
//! tops (and table planes) are ledges. The room's scanned triangle mesh fills
//! in everything else (chairs, shelves, clutter), but only where no clean
//! plane or box already covers it: the mesh is lumpy, so the flat faces are
//! what gets ridden wherever they exist. The player's placed pieces
//! (`xr::build`) are added on top. Rebuilt whenever the set of located Scene
//! entities changes or the layout is edited.
use super::planes::{LocatedMesh, RoomMesh, SceneRevision, SceneShapes, Shape};
use super::build::PlacedPieces;
use super::plane_view::MeshPlanes;
use super::build::pieces::PieceKind;
use crate::skate_audio::clean::SurfaceHints;
use crate::physics::ground::PlacedSurfaces;
use crate::physics::SkaterRuntime;
use super::rig::Rig;
use crate::physics::GamePhysics;
use bevy::prelude::*;
use skate_core::math::Vector3;

const MIN_HALF_M: f32 = 0.01;
const UPWARD_MIN_COS: f32 = 0.9;
/// A mesh triangle this close to a Scene plane or box surface is that surface.
const COVER_DISTANCE_M: f32 = 0.08;
/// Scan edges overhang the Scene rectangles a little.
const COVER_MARGIN_M: f32 = 0.05;
/// Ceilings and the undersides of things.
const MAX_DOWN_COS: f32 = -0.7;
/// Twice the area of the smallest kept triangle, m^2.
const MIN_DOUBLE_AREA_M2: f32 = 1e-6;

pub(crate) struct RealWorldPlugin;

impl Plugin for RealWorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, rebuild_collision);
    }
}

fn rebuild_collision(
    rig: Res<Rig>,
    shapes: Res<SceneShapes>,
    room: Res<RoomMesh>,
    revision: Res<SceneRevision>,
    placed: Res<PlacedPieces>,
    mesh_planes: Res<MeshPlanes>,
    physics: Option<ResMut<GamePhysics>>,
    skater: Option<ResMut<SkaterRuntime>>,
    hints: Option<ResMut<SurfaceHints>>,
    mut built: Local<Option<((Vec3, f32), u64, u64)>>,
) {
    let (Some(anchor), Some(root), Some(mut physics), Some(mut skater)) =
        (rig.anchor, rig.root_transform(), physics, skater)
    else {
        return;
    };
    // Re-located poses wobble by tracking noise every frame; only a new
    // anchor, a different set of entities or an edited layout is worth a
    // rebuild. A piece being dragged rebuilds when it is dropped, and grind
    // state refers to the current rails, so a grind finishes first.
    let key = (anchor, revision.0, placed.revision);
    if shapes.0.is_empty() || *built == Some(key) || placed.busy() || skater.is_grinding() {
        return;
    }
    *built = Some(key);
    let started = std::time::Instant::now();
    // Flat surfaces found in the room mesh ride like Scene planes, with
    // grindable edges, and replace the lumpy triangles under them.
    let all_shapes: Vec<Shape> = shapes.0.iter().copied().chain(mesh_planes.solid_shapes()).collect();
    let (quads, real_edges) = real_geometry(&all_shapes);
    let triangles = room
        .0
        .as_ref()
        .map(|mesh| mesh_triangles(mesh, &all_shapes))
        .unwrap_or_default();
    let v = |w: Vec3| Vector3::new(w.x, w.y, w.z);
    let to_world = |p: Vec3| v(root.transform_point(p));
    let (quad_count, triangle_count) = (quads.len(), triangles.len());
    let quads = quads.into_iter().map(|q| q.map(to_world)).collect();
    let triangles = triangles.into_iter().map(|t| t.map(to_world)).collect();
    let edges: Vec<_> = real_edges.iter().map(|&(a, b)| (to_world(a), to_world(b))).collect();
    // Table and furniture tops are grindable ledges.
    let mut rails: Vec<Vec<Vector3>> = edges.iter().map(|&(a, b)| vec![a, b]).collect();
    let mut pieces = Vec::with_capacity(placed.items.len());
    let mut sound = SurfaceHints::default();
    for item in &placed.items {
        let at = item.world_transform(&root);
        let local = |p: Vec3| v(at.transform_point(p));
        let geometry = item.piece.geometry();
        rails.extend(geometry.rails.iter().map(|r| r.iter().map(|&p| local(p)).collect()));
        // Rails, handrails and ramp copings are metal; ramps and boxes are
        // wood; stairs roll like the concrete floor.
        if matches!(
            item.piece.kind,
            PieceKind::Rail | PieceKind::QuarterPipe | PieceKind::HalfPipe | PieceKind::Spine | PieceKind::Stairs
        ) {
            for rail in &geometry.rails {
                let rail: Vec<Vec3> = rail.iter().map(|&p| at.transform_point(p)).collect();
                sound.metal_rails.extend(rail.windows(2).map(|w| (w[0], w[1])));
            }
        }
        if !matches!(item.piece.kind, PieceKind::Rail | PieceKind::Stairs) {
            let corners = geometry.quads.iter().flatten().map(|&p| at.transform_point(p));
            let (min, max) = corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| {
                (lo.min(p), hi.max(p))
            });
            if min.x <= max.x {
                sound.wood_boxes.push((min, max));
            }
        }
        pieces.push(PlacedSurfaces {
            quads: geometry.quads.iter().map(|q| q.map(local)).collect(),
            rail_quads: geometry.rail_quads.iter().map(|q| q.map(local)).collect(),
            edges: geometry.edges.iter().map(|&(a, b)| (local(a), local(b))).collect(),
        });
    }
    let piece_count = pieces.len();
    if let Err(e) = physics.set_real_surfaces(quads, triangles, edges, pieces) {
        error!("AR collision rebuild failed: {e}");
        return;
    }
    if let Err(e) = physics.set_grind_rails(&rails, &mut skater) {
        error!("AR grind rails rebuild failed: {e}");
    }
    if let Some(mut hints) = hints {
        *hints = sound;
    }
    info!(
        "AR collision: {quad_count} Scene faces + {triangle_count} room-mesh triangles from {} shapes ({} mesh planes), {piece_count} placed pieces, {} grind rails ({:.1} ms)",
        shapes.0.len(),
        all_shapes.len() - shapes.0.len(),
        rails.len(),
        started.elapsed().as_secs_f32() * 1000.0
    );
}

/// Tracking-space room-mesh triangles not already covered by a Scene plane
/// or box, each in both windings (scan winding isn't guaranteed outward).
pub(crate) fn mesh_triangles(mesh: &LocatedMesh, shapes: &[Shape]) -> Vec<[Vec3; 3]> {
    let vertices: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|&v| mesh.pose.transform_point(v))
        .collect();
    mesh.indices
        .chunks_exact(3)
        .filter_map(|t| {
            let [a, b, c] = [0, 1, 2].map(|i| vertices[t[i] as usize]);
            let normal = (b - a).cross(c - a);
            let double_area = normal.length();
            if double_area < MIN_DOUBLE_AREA_M2 || normal.y / double_area < MAX_DOWN_COS {
                return None;
            }
            let centre = (a + b + c) / 3.0;
            (!shapes.iter().any(|s| covers(s, centre))).then_some([a, b, c])
        })
        .flat_map(|t| [t, [t[0], t[2], t[1]]])
        .collect()
}

pub(crate) fn covers(shape: &Shape, point: Vec3) -> bool {
    match *shape {
        Shape::Plane { pose, half, .. } => {
            let local = pose.rotation.inverse() * (point - pose.translation);
            local.y.abs() <= COVER_DISTANCE_M
                && local.x.abs() <= half.x + COVER_MARGIN_M
                && local.z.abs() <= half.y + COVER_MARGIN_M
        }
        Shape::Box { pose, half } => {
            let local = pose.rotation.inverse() * (point - pose.translation);
            (local.abs() - half).max_element() <= COVER_DISTANCE_M
        }
    }
}

/// Corners `c -u -w, c +u -w, c +u +w, c -u +w`; the collision face normal is
/// `w x u` (the same winding as the test course's floor).
fn quad(c: Vec3, u: Vec3, w: Vec3) -> [Vec3; 4] {
    [c - u - w, c + u - w, c + u + w, c - u + w]
}

fn outline(q: [Vec3; 4]) -> [(Vec3, Vec3); 4] {
    [(q[0], q[1]), (q[1], q[2]), (q[2], q[3]), (q[3], q[0])]
}

/// Tracking-space collision faces and ledge edges for the Scene shapes.
pub(crate) fn real_geometry(shapes: &[Shape]) -> (Vec<[Vec3; 4]>, Vec<(Vec3, Vec3)>) {
    let mut quads = Vec::new();
    let mut edges = Vec::new();
    for shape in shapes {
        match *shape {
            Shape::Plane { pose, half, floor } => {
                if half.min_element() < MIN_HALF_M {
                    continue;
                }
                let q = quad(
                    pose.translation,
                    pose.rotation * Vec3::X * half.x,
                    pose.rotation * Vec3::Z * half.y,
                );
                if (pose.rotation * Vec3::Y).y >= UPWARD_MIN_COS {
                    quads.push(q);
                    if !floor {
                        edges.extend(outline(q));
                    }
                } else {
                    // Walls: solid from both sides, whichever way Scene faces them.
                    quads.push(q);
                    quads.push([q[0], q[3], q[2], q[1]]);
                }
            }
            Shape::Box { pose, half } => {
                if half.min_element() < MIN_HALF_M {
                    continue;
                }
                let axes = [Vec3::X, Vec3::Y, Vec3::Z];
                for i in 0..3 {
                    for sign in [1.0, -1.0] {
                        let n = pose.rotation * (axes[i] * sign);
                        if n.y < -0.5 {
                            continue; // the bottom sits on the floor
                        }
                        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                        let u_dir = pose.rotation * axes[j];
                        let w_dir = u_dir.cross(n);
                        let q = quad(pose.translation + n * half[i], u_dir * half[j], w_dir * half[k]);
                        quads.push(q);
                        if n.y >= UPWARD_MIN_COS {
                            edges.extend(outline(q));
                        }
                    }
                }
            }
        }
    }
    (quads, edges)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(q: [Vec3; 4]) -> Vec3 {
        // Matches ground::build_world's triangle (0, 2, 1).
        (q[1] - q[0]).cross(q[2] - q[0]).normalize() * -1.0
    }

    #[test]
    fn quad_winding_matches_the_course_floor() {
        let q = quad(Vec3::ZERO, Vec3::X * 50.0, Vec3::Z * 50.0);
        assert_eq!(q[0], Vec3::new(-50.0, 0.0, -50.0));
        assert_eq!(q[1], Vec3::new(50.0, 0.0, -50.0));
        assert_eq!(q[2], Vec3::new(50.0, 0.0, 50.0));
        assert_eq!(q[3], Vec3::new(-50.0, 0.0, 50.0));
    }

    #[test]
    fn box_faces_point_outwards_and_skip_the_bottom() {
        let shape = Shape::Box {
            pose: Transform::from_translation(Vec3::new(0.0, 0.4, 0.0)),
            half: Vec3::new(0.6, 0.4, 0.3),
        };
        let (quads, edges) = real_geometry(&[shape]);
        assert_eq!(quads.len(), 5);
        assert_eq!(edges.len(), 4, "only the top outline is a ledge");
        for q in &quads {
            let centre = (q[0] + q[1] + q[2] + q[3]) / 4.0;
            let outward = (centre - Vec3::new(0.0, 0.4, 0.0)).normalize();
            assert!(normal(*q).dot(outward) > 0.99, "{q:?}");
        }
        let top = quads.iter().find(|q| q[0].y > 0.79).unwrap();
        assert!(top.iter().all(|v| (v.y - 0.8).abs() < 1e-5));
    }

    #[test]
    fn room_mesh_keeps_only_what_scene_does_not_cover() {
        let floor = Shape::Plane { pose: Transform::IDENTITY, half: Vec2::splat(5.0), floor: true };
        let table = Shape::Box {
            pose: Transform::from_xyz(1.0, 0.375, 0.0),
            half: Vec3::new(0.6, 0.375, 0.4),
        };
        let mesh = LocatedMesh {
            pose: Transform::IDENTITY,
            vertices: [
                // Floor lump (2 cm up): covered by the floor plane.
                Vec3::new(-1.0, 0.02, -1.0),
                Vec3::new(-0.5, 0.02, -1.0),
                Vec3::new(-1.0, 0.02, -0.5),
                // Table top: covered by the box.
                Vec3::new(0.8, 0.76, 0.0),
                Vec3::new(1.2, 0.76, 0.0),
                Vec3::new(0.8, 0.76, 0.2),
                // A chair seat, 45 cm up, away from both: kept.
                Vec3::new(-2.0, 0.45, 1.0),
                Vec3::new(-1.6, 0.45, 1.0),
                Vec3::new(-2.0, 0.45, 1.4),
                // A ceiling triangle (faces down): dropped.
                Vec3::new(0.0, 2.5, 0.0),
                Vec3::new(0.0, 2.5, 0.5),
                Vec3::new(0.5, 2.5, 0.0),
            ]
            .into(),
            indices: [0, 2, 1, 3, 5, 4, 6, 8, 7, 9, 11, 10].into(),
        };
        let kept = mesh_triangles(&mesh, &[floor, table]);
        assert_eq!(kept.len(), 2, "the chair seat, in both windings");
        assert!(kept.iter().flatten().all(|v| (v.y - 0.45).abs() < 1e-6));
    }

    #[test]
    fn floor_has_no_ledges_but_a_table_plane_does() {
        let floor = Shape::Plane { pose: Transform::IDENTITY, half: Vec2::splat(2.0), floor: true };
        let table = Shape::Plane {
            pose: Transform::from_xyz(0.0, 0.75, 0.0),
            half: Vec2::new(0.6, 0.4),
            floor: false,
        };
        let (quads, edges) = real_geometry(&[floor, table]);
        assert_eq!(quads.len(), 2);
        assert_eq!(edges.len(), 4);
        assert!(normal(quads[0]).dot(Vec3::Y) > 0.99);
    }
}
