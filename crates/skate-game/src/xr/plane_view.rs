//! Plane detection on top of Space Setup, and a view of what was detected.
//!
//! The Quest runtime has no `XR_EXT_plane_detection`. Detected planes here
//! are Space Setup's labelled planes and furniture tops ([`SceneShapes`]) plus
//! the flat patches found in its room mesh ([`MeshPlanes`], fitted off the
//! main thread whenever the mesh or the Scene changes). Mesh planes that fill
//! their rectangle become collision with grindable edges (`real_world`).
//!
//! The overlay draws every detected plane as a translucent fill with a bright
//! outline, coloured by what it is: floor blue, furniture green, walls grey,
//! mesh-found surfaces orange (yellow when only shown, not ridden). It is on
//! in build mode, flashes for a few seconds whenever the detected set
//! changes, and A in build mode keeps it on everywhere.
use super::actions::LatestTouch;
use super::build::BuildMode;
use super::mesh_planes::{FitPlane, fit_planes};
use super::planes::{RoomMesh, SceneLabels, SceneRevision, SceneShapes, Shape, up_axis};
use super::real_world::covers;
use super::rig::Rig;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

/// How long the overlay shows after the detected planes change, seconds.
const FLASH_SECONDS: f32 = 4.0;
/// Overlays float this far above their surface, metres (no z-fighting with
/// pieces or the real-surface depth).
const LIFT_M: f32 = 0.004;
const OUTLINE_M: f32 = 0.012;

/// Flat patches of the room mesh that Space Setup did not already label.
#[derive(Resource, Default)]
pub(crate) struct MeshPlanes {
    pub planes: Vec<FitPlane>,
    pub revision: u64,
}

impl MeshPlanes {
    /// The ridable ones, as Scene-style planes (grindable outline).
    pub(crate) fn solid_shapes(&self) -> impl Iterator<Item = Shape> + '_ {
        self.planes.iter().filter(|p| p.solid).map(|p| Shape::Plane {
            pose: p.pose(),
            half: p.half,
            floor: false,
        })
    }
}

#[derive(Resource, Default)]
struct PlaneOverlay {
    always: bool,
    flash_until: f32,
    built: Option<(u64, u64)>,
    previous_a: bool,
}

#[derive(Component)]
struct OverlayRoot;

pub(crate) struct PlaneViewPlugin;

impl Plugin for PlaneViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MeshPlanes>()
            .init_resource::<PlaneOverlay>()
            .add_systems(Startup, spawn_root)
            .add_systems(Update, (fit_room_planes, toggle, rebuild_overlay, place_overlay).chain());
    }
}

type FitJob = Task<(Vec<FitPlane>, f32)>;

/// Refits whenever a different room mesh arrives or the Scene entities change.
fn fit_room_planes(
    room: Res<RoomMesh>,
    shapes: Res<SceneShapes>,
    mut out: ResMut<MeshPlanes>,
    mut revision: ResMut<SceneRevision>,
    mut job: Local<Option<FitJob>>,
    mut fitted: Local<Option<(usize, usize)>>,
) {
    if let Some(task) = job.as_mut() {
        let Some((planes, ms)) = block_on(future::poll_once(task)) else {
            return;
        };
        *job = None;
        let solid = planes.iter().filter(|p| p.solid).count();
        info!(
            "PLANES scene={} mesh_fit={} solid={solid} rev={} ({ms:.1} ms)",
            shapes.0.len(),
            planes.len(),
            revision.0
        );
        if out.planes != planes {
            out.planes = planes;
            out.revision += 1;
            // Collision is keyed on the Scene revision.
            revision.0 += 1;
        }
    }
    let Some(mesh) = room.0.as_ref() else { return };
    let key = (mesh.vertices.as_ptr() as usize, shapes.0.len());
    if *fitted == Some(key) || job.is_some() {
        return;
    }
    *fitted = Some(key);
    let vertices: Vec<Vec3> = mesh.vertices.iter().map(|&v| mesh.pose.transform_point(v)).collect();
    let indices = mesh.indices.clone();
    let scene = shapes.0.clone();
    *job = Some(AsyncComputeTaskPool::get().spawn(async move {
        let started = std::time::Instant::now();
        let planes = fit_planes(&vertices, &indices)
            .into_iter()
            // Space Setup's planes and boxes win where they exist.
            .filter(|p| {
                let c = p.hull.iter().copied().sum::<Vec2>() / p.hull.len() as f32;
                !scene.iter().any(|s| covers(s, Vec3::new(c.x, p.height, c.y)))
            })
            .collect();
        (planes, started.elapsed().as_secs_f32() * 1000.0)
    }));
}

fn toggle(
    touch: Res<LatestTouch>,
    mode: Res<BuildMode>,
    mut overlay: ResMut<PlaneOverlay>,
) {
    let a = touch.0.a;
    if mode.active && a && !overlay.previous_a {
        overlay.always = !overlay.always;
        info!("PLANES overlay always on: {}", overlay.always);
    }
    overlay.previous_a = a;
}

fn spawn_root(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.spawn((
        OverlayRoot,
        Mesh3d(Handle::default()),
        MeshMaterial3d(material),
        Transform::default(),
        Visibility::Hidden,
    ));
}

#[derive(Clone, Copy)]
enum Look {
    Floor,
    Furniture,
    Wall,
    MeshSolid,
    MeshShown,
}

impl Look {
    /// (fill, outline) colours, linear RGBA.
    fn colors(self) -> ([f32; 4], [f32; 4]) {
        match self {
            Look::Floor => ([0.15, 0.5, 1.0, 0.06], [0.3, 0.7, 1.0, 0.7]),
            Look::Furniture => ([0.2, 1.0, 0.45, 0.16], [0.35, 1.0, 0.55, 0.85]),
            Look::Wall => ([0.0; 4], [0.8, 0.8, 0.85, 0.35]),
            Look::MeshSolid => ([1.0, 0.55, 0.1, 0.18], [1.0, 0.6, 0.15, 0.9]),
            Look::MeshShown => ([1.0, 0.9, 0.2, 0.08], [1.0, 0.9, 0.3, 0.5]),
        }
    }
}

#[derive(Default)]
struct OverlayMesh {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl OverlayMesh {
    /// A planar polygon (convex, in order) with its outline, lifted along
    /// `normal`.
    fn polygon(&mut self, points: &[Vec3], normal: Vec3, look: Look) {
        if points.len() < 3 {
            return;
        }
        let (fill, line) = look.colors();
        let lifted: Vec<Vec3> = points.iter().map(|&p| p + normal * LIFT_M).collect();
        let centre = lifted.iter().copied().sum::<Vec3>() / lifted.len() as f32;
        if fill[3] > 0.0 {
            let base = self.positions.len() as u32;
            self.positions.extend(lifted.iter().map(|p| p.to_array()));
            self.colors.extend(std::iter::repeat_n(fill, lifted.len()));
            for i in 1..lifted.len() as u32 - 1 {
                self.indices.extend([base, base + i, base + i + 1]);
            }
        }
        // Outline: a thin band just inside each edge.
        for i in 0..lifted.len() {
            let (a, b) = (lifted[i], lifted[(i + 1) % lifted.len()]);
            let inward = |p: Vec3| {
                let d = centre - p;
                p + d.normalize_or_zero() * OUTLINE_M.min(d.length() * 0.5)
            };
            let base = self.positions.len() as u32;
            self.positions.extend([a, b, inward(b), inward(a)].map(|p| p.to_array()));
            self.colors.extend([line; 4]);
            self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    fn rect(&mut self, pose: Transform, half: Vec2, look: Look) {
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, z)| pose.transform_point(Vec3::new(x * half.x, 0.0, z * half.y)));
        self.polygon(&corners, pose.rotation * Vec3::Y, look);
    }

    fn into_mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
            .with_inserted_indices(Indices::U32(self.indices))
    }
}

fn scene_look(shape: &Shape, label: &str) -> Look {
    match shape {
        Shape::Plane { floor: true, .. } => Look::Floor,
        Shape::Plane { pose, .. } if (pose.rotation * Vec3::Y).y < 0.9 => Look::Wall,
        _ if label.contains("FLOOR") => Look::Floor,
        _ => Look::Furniture,
    }
}

/// Rebuilds the overlay mesh when the detected planes change, in tracking
/// space (the root entity carries the rig transform).
fn rebuild_overlay(
    shapes: Res<SceneShapes>,
    labels: Res<SceneLabels>,
    mesh_planes: Res<MeshPlanes>,
    revision: Res<SceneRevision>,
    time: Res<Time<Real>>,
    mut overlay: ResMut<PlaneOverlay>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut root: Query<&mut Mesh3d, With<OverlayRoot>>,
) {
    let key = (revision.0, mesh_planes.revision);
    if overlay.built == Some(key) || shapes.0.is_empty() {
        return;
    }
    let Ok(mut handle) = root.single_mut() else { return };
    let first = overlay.built.is_none();
    overlay.built = Some(key);
    if !first {
        overlay.flash_until = time.elapsed_secs() + FLASH_SECONDS;
    }
    let mut out = OverlayMesh::default();
    for (i, shape) in shapes.0.iter().enumerate() {
        let look = scene_look(shape, labels.0.get(i).map_or("", String::as_str));
        match *shape {
            Shape::Plane { pose, half, .. } => out.rect(pose, half, look),
            Shape::Box { pose, half } => {
                // The top face, full size.
                let (axis, sign) = up_axis(pose.rotation);
                let normal = pose.rotation * (axis * sign);
                let (u, w) = match axis {
                    a if a == Vec3::X => (Vec3::Y * half.y, Vec3::Z * half.z),
                    a if a == Vec3::Y => (Vec3::X * half.x, Vec3::Z * half.z),
                    _ => (Vec3::X * half.x, Vec3::Y * half.y),
                };
                let top = pose.translation + normal * axis.dot(half).abs();
                let (u, w) = (pose.rotation * u, pose.rotation * w);
                out.polygon(&[top - u - w, top + u - w, top + u + w, top - u + w], normal, look);
            }
        }
    }
    for plane in &mesh_planes.planes {
        let look = if plane.solid { Look::MeshSolid } else { Look::MeshShown };
        if plane.solid {
            out.rect(plane.pose(), plane.half, look);
        } else {
            let points: Vec<Vec3> = plane.hull.iter().map(|p| Vec3::new(p.x, plane.height, p.y)).collect();
            out.polygon(&points, Vec3::Y, look);
        }
    }
    handle.0 = meshes.add(out.into_mesh());
}

fn place_overlay(
    rig: Res<Rig>,
    mode: Res<BuildMode>,
    overlay: Res<PlaneOverlay>,
    time: Res<Time<Real>>,
    mut root: Query<(&mut Transform, &mut Visibility), With<OverlayRoot>>,
) {
    let Ok((mut transform, mut visibility)) = root.single_mut() else { return };
    let show = overlay.built.is_some()
        && (mode.active || overlay.always || time.elapsed_secs() < overlay.flash_until);
    let target = match rig.root_transform() {
        Some(root) if show => {
            if *transform != root {
                *transform = root;
            }
            Visibility::Visible
        }
        _ => Visibility::Hidden,
    };
    if *visibility != target {
        *visibility = target;
    }
}
