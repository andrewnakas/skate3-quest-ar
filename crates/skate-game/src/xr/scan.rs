//! Park scanning: the real ground as one smooth, rideable surface.
//!
//! Two scans feed it, both fused into 2.5D height grids (`sk8_terrain`), tracking space:
//! - the room mesh Space Setup captured (indoors, room sized), and
//! - depth frames from the headset (`depth`), fused while the player walks round a whole
//!   skatepark with "Scan the park" on. That is the outdoor scan: it covers as much ground as
//!   is walked, and it keeps growing (the depth scan wins where both have data).
//!
//! The grids are cleaned, smoothed (lumps and depth noise out, transitions kept round, ledges
//! and walls kept sharp), meshed with smooth normals and searched for grindable edges on a
//! background thread whenever the scan changes, at most every [`REBUILD_SECONDS`]. The result,
//! mapped into the game world through the rig, replaces the lumpy raw room mesh in collision
//! (`real_world`); the clean engine rides it with interpolated normals.
use super::ar_settings::ArSettings;
use super::planes::{RoomMesh, SceneShapes, Shape};
use super::real_world::{covers, layer_on};
use super::rig::Rig;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use sk8_terrain::{BuildConfig, Collider, FuseConfig, Fusion, HeightMap};
use std::sync::Arc;

/// Shortest time between terrain rebuilds while the scan is changing.
const REBUILD_SECONDS: f32 = 1.5;
/// Depth points this far above the floor are trees, roofs and people's heads, not park.
pub(crate) const MAX_PARK_HEIGHT_M: f32 = 3.2;
/// Floor planes sit this far under scanned terrain, so the scan is what gets ridden wherever it
/// exists (with no lip where the two meet) and the floor still catches the skater past it.
pub(crate) const FLOOR_UNDER_TERRAIN_M: f32 = 0.04;

/// The scanned terrain mapped into the game world.
pub(crate) struct TerrainWorld {
    pub collider: Arc<Collider>,
    /// Ledge tops and copings found in the scan, game metres.
    pub rails: Vec<Vec<Vec3>>,
    /// The terrain in tracking space (for drawing it over the real ground).
    pub tracking: sk8_terrain::TerrainMesh,
}

/// What a built terrain was built from: rig anchor, scan revision, Scene revision, settings.
pub(crate) type BuildKey = ((Vec3, f32), u64, u64, u64);

/// What the depth capture is doing, for the AR settings page (and the log).
#[derive(Resource, Default)]
pub(crate) struct ScanStatus {
    /// Short state: "off", "starting", "running", or why not.
    pub state: String,
    /// Depth frames taken / unprojected points kept from them, this session.
    pub frames: u64,
    pub points: u64,
    /// Points thrown out by the range / height filters (a wrong floor or row order shows here).
    pub rejected: u64,
}

impl ScanStatus {
    pub(crate) fn line(&self) -> String {
        if self.state.is_empty() {
            return "Depth: not started (needs the headset's depth sensor)".into();
        }
        format!("Depth: {} - {} frames, {} points kept, {} thrown out", self.state, self.frames, self.points, self.rejected)
    }
}

#[derive(Resource)]
pub(crate) struct ParkScan {
    /// The walked depth scan.
    pub depth: Fusion,
    /// The Space Setup room mesh as a height grid, and the mesh it was made from.
    room: Option<(Arc<HeightMap>, usize)>,
    room_job: Option<(usize, Task<HeightMap>)>,
    /// Bumped whenever an input of the terrain changes.
    pub revision: u64,
    pub built: Option<(BuildKey, Arc<TerrainWorld>)>,
    job: Option<(BuildKey, Task<TerrainWorld>)>,
    last_build: f32,
    pub depth_frames: u64,
    pub depth_rays: u64,
}

impl Default for ParkScan {
    fn default() -> Self {
        Self {
            depth: Fusion::new(FuseConfig::default()),
            room: None,
            room_job: None,
            revision: 0,
            built: None,
            job: None,
            last_build: f32::NEG_INFINITY,
            depth_frames: 0,
            depth_rays: 0,
        }
    }
}

impl ParkScan {
    /// The latest built terrain, if it was built for this rig anchor, Scene and settings (the
    /// walked scan may have grown since: that rebuild follows).
    pub(crate) fn terrain_for(&self, anchor: (Vec3, f32), scene: u64, settings: u64) -> Option<Arc<TerrainWorld>> {
        self.built.as_ref().filter(|(k, _)| (k.0, k.2, k.3) == (anchor, scene, settings)).map(|(_, t)| t.clone())
    }

    /// Forget the walked scan (the room mesh stays).
    pub(crate) fn clear_depth(&mut self) {
        self.depth.clear();
        self.depth_frames = 0;
        self.depth_rays = 0;
        self.revision += 1;
    }

    /// Square metres of ground the walked scan covers.
    pub(crate) fn depth_area_m2(&self) -> f32 {
        let cell = self.depth.config.cell;
        self.depth.heightmap(BuildConfig::default().min_weight).filled() as f32 * cell * cell
    }
}

pub(crate) struct ScanPlugin;

impl Plugin for ScanPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ParkScan>()
            .init_resource::<ScanStatus>()
            .add_systems(Update, (grid_room_mesh, rebuild_terrain).chain());
        #[cfg(target_os = "android")]
        super::depth::build(app);
    }
}

/// The room mesh as a height grid, made off the main thread whenever a new mesh arrives.
fn grid_room_mesh(room: Res<RoomMesh>, mut scan: ResMut<ParkScan>) {
    let key = room.0.as_ref().map_or(0, |m| m.vertices.as_ptr() as usize);
    if let Some((job_key, task)) = scan.room_job.as_mut() {
        let job_key = *job_key;
        if let Some(map) = block_on(future::poll_once(task)) {
            scan.room_job = None;
            info!("SCAN room mesh gridded: {} cells", map.filled());
            scan.room = Some((Arc::new(map), job_key));
            scan.revision += 1;
        }
        return;
    }
    let current = scan.room.as_ref().map_or(0, |(_, k)| *k);
    if key == current {
        return;
    }
    let Some(mesh) = room.0.clone() else {
        scan.room = None;
        scan.revision += 1;
        return;
    };
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let mut fusion = Fusion::new(FuseConfig::default());
        let v: Vec<Vec3> = mesh.vertices.iter().map(|&p| mesh.pose.transform_point(p)).collect();
        for t in mesh.indices.chunks_exact(3) {
            let tri = [v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]];
            if tri.iter().all(|p| p.y < MAX_PARK_HEIGHT_M) {
                fusion.add_triangle(tri, 1.0);
            }
        }
        fusion.heightmap(1.0)
    });
    scan.room_job = Some((key, task));
}

#[allow(clippy::too_many_arguments)]
fn rebuild_terrain(
    mut scan: ResMut<ParkScan>,
    rig: Res<Rig>,
    shapes: Res<SceneShapes>,
    revision: Res<super::planes::SceneRevision>,
    settings: Res<ArSettings>,
    time: Res<Time<Real>>,
) {
    if let Some((key, task)) = scan.job.as_mut() {
        let key = *key;
        if let Some(world) = block_on(future::poll_once(task)) {
            scan.job = None;
            scan.built = Some((key, Arc::new(world)));
        }
        return;
    }
    let (Some(anchor), Some(root)) = (rig.anchor, rig.root_transform()) else { return };
    let key: BuildKey = (anchor, scan.revision, revision.0, settings.revision);
    if scan.built.as_ref().is_some_and(|(k, _)| *k == key) {
        return;
    }
    let now = time.elapsed_secs();
    // Only the walked scan changes all the time; anything else (a new anchor, the room, the
    // settings) rebuilds straight away.
    let only_scan_moved = scan.built.as_ref().is_some_and(|(k, _)| (k.0, k.2, k.3) == (key.0, key.2, key.3));
    if only_scan_moved && now - scan.last_build < REBUILD_SECONDS {
        return;
    }
    scan.last_build = now;
    let cfg = BuildConfig::default();
    let depth = scan.depth.heightmap(cfg.min_weight);
    let room = if settings.free_roam() || !settings.mesh.collide { None } else { scan.room.as_ref().map(|(m, _)| m.clone()) };
    if depth.z.is_empty() && room.is_none() {
        scan.built = None;
        return;
    }
    // Space Setup's planes and boxes win where they exist (floors excepted: they sit under the
    // scan, see FLOOR_UNDER_TERRAIN_M).
    let solid: Vec<Shape> = shapes
        .0
        .iter()
        .copied()
        .filter(|s| layer_on(s, &settings) && !matches!(s, Shape::Plane { floor: true, .. }))
        .collect();
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let started = std::time::Instant::now();
        let raw = match room {
            Some(room) => room.overlay(&depth),
            None => depth,
        };
        let terrain = sk8_terrain::build_from(&raw, &cfg);
        let mut world_mesh = sk8_terrain::TerrainMesh::default();
        for (t, n) in terrain.mesh.triangles.iter().zip(&terrain.mesh.normals) {
            let centre = (t[0] + t[1] + t[2]) / 3.0;
            if solid.iter().any(|s| covers(s, centre)) {
                continue;
            }
            world_mesh.triangles.push(t.map(|p| root.transform_point(p)));
            world_mesh.normals.push(n.map(|v| root.rotation * v));
        }
        let rails: Vec<Vec<Vec3>> =
            terrain.rails.iter().map(|r| r.iter().map(|&p| root.transform_point(p)).collect()).collect();
        let collider = Arc::new(Collider::new(&world_mesh));
        info!(
            "SCAN terrain: {} cells, {} triangles, {} edge rails ({:.0} ms)",
            terrain.surface.map.filled(),
            collider.len(),
            rails.len(),
            started.elapsed().as_secs_f32() * 1000.0
        );
        TerrainWorld { collider, rails, tracking: terrain.mesh }
    });
    scan.job = Some((key, task));
}
