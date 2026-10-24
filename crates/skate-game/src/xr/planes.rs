//! Real-world surfaces and objects from Meta's Scene model (`XR_FB_scene`).
//!
//! Quest's runtime does not offer `XR_EXT_plane_detection`; its planes and
//! objects are what the headset's Space Setup captured: floor and walls as 2D
//! planes, furniture (tables, couches, beds, storage...) as 3D boxes. They are
//! queried until results arrive (retried until the Scene permission is
//! granted), then re-queried every [`REFRESH_SECONDS`] so a new or edited
//! room scan shows up live, and re-located every frame. Flat things Space
//! Setup did not label are found in its room mesh (`mesh_planes`). The tracking space is STAGE, so the real
//! floor is always available at y = 0 even without a room scan.
use bevy::prelude::*;
use bevy_mod_openxr::{
    exts::OxrEnabledExtensions,
    openxr_session_running,
    poll_events::{OxrEventHandlerExt, OxrEventIn},
    resources::{OxrFrameState, OxrInstance},
    session::OxrSession,
};
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use openxr::sys::{self, Handle as _};
use std::ptr;
use std::sync::Arc;

/// One upward-facing real surface, in the same tracking-space frame as
/// [`super::actions::Hand`] transforms (pre-rig-scale, pre-root-transform).
/// Local +Y is the surface normal; the surface spans local X/Z.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DetectedPlane {
    pub pose: Transform,
    pub half_extent: Vec2,
    pub upward: bool,
}

#[derive(Resource, Default)]
pub(crate) struct DetectedPlanes(pub Vec<DetectedPlane>);

impl DetectedPlanes {
    /// The point on the upward-facing surface under/over `point` whose height
    /// is closest to `point.y`.
    pub(crate) fn snap(&self, point: Vec3) -> Option<Vec3> {
        self.0
            .iter()
            .filter(|p| p.upward)
            .filter_map(|p| {
                let local = p.pose.rotation.inverse() * (point - p.pose.translation);
                (local.x.abs() <= p.half_extent.x && local.z.abs() <= p.half_extent.y)
                    .then_some((p, local.y.abs()))
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(p, _)| {
                let normal = p.pose.rotation * Vec3::Y;
                point - normal * (point - p.pose.translation).dot(normal)
            })
    }
}

/// A located Scene entity in tracking space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shape {
    /// Local +Y is the normal; spans local X/Z by `half`.
    Plane { pose: Transform, half: Vec2, floor: bool },
    /// Centred box with local half extents.
    Box { pose: Transform, half: Vec3 },
}

/// Every usable Scene entity, located this frame. A real floor is always
/// present (Space Setup's, or STAGE y = 0).
#[derive(Resource, Default)]
pub(crate) struct SceneShapes(pub Vec<Shape>);

/// Bumped whenever [`SceneShapes`] or [`RoomMesh`] gains or loses entities,
/// so collision rebuilds when located entities arrive (not just on
/// placement).
#[derive(Resource, Default)]
pub(crate) struct SceneRevision(pub u64);

/// The Space Setup label of each [`SceneShapes`] entry, in the same order
/// ("FLOOR" for the fallback floor).
#[derive(Resource, Default)]
pub(crate) struct SceneLabels(pub Vec<String>);

/// Space Setup's triangle mesh of the whole room (`XR_META_spatial_entity_mesh`),
/// in its own space; `pose` places it in tracking space.
#[derive(Clone)]
pub(crate) struct LocatedMesh {
    pub pose: Transform,
    pub vertices: Arc<[Vec3]>,
    pub indices: Arc<[u32]>,
}

#[derive(Resource, Default)]
pub(crate) struct RoomMesh(pub Option<LocatedMesh>);

struct SceneSurface {
    space: sys::Space,
    rect: Option<sys::Rect2Df>,
    volume: Option<sys::Rect3DfFB>,
    mesh: Option<(Arc<[Vec3]>, Arc<[u32]>)>,
    label: String,
}

/// Space Setup ("scan your room") launched from inside the app. Set
/// `requested` to start one; when it completes, the Scene is queried again.
#[derive(Resource, Default)]
pub(crate) struct SceneCapture {
    pub requested: bool,
    pending: Option<sys::AsyncRequestIdFB>,
    /// The automatic capture for a room with no scan runs once per launch.
    auto_requested: bool,
}

impl SceneCapture {
    /// Space Setup is running (asked for, or not finished).
    pub(crate) fn busy(&self) -> bool {
        self.requested || self.pending.is_some()
    }
}

#[derive(Resource, Default)]
struct SceneQuery {
    pending: Option<sys::AsyncRequestIdFB>,
    surfaces: Vec<SceneSurface>,
    since_attempt: Option<f32>,
    attempt: usize,
    /// The query shape the runtime last accepted, reused for refreshes.
    accepted: Option<QueryShape>,
    /// A refresh is in flight; its results collect in `incoming`.
    refreshing: bool,
    incoming: Vec<SceneSurface>,
    since_refresh: f32,
    /// A refresh replaced `surfaces`; `locate_surfaces` bumps the revision.
    changed: bool,
}

/// Quest runtimes reject some query shapes with ERROR_VALIDATION_FAILURE
/// (seen for component filters on Oculus 206); try these in turn.
#[derive(Clone, Copy, Debug)]
enum QueryShape {
    /// No filter: every anchor the app may load (Meta's sample "query all").
    All,
    Bounded2d,
    LocatableLocal,
}
const QUERY_SHAPES: [QueryShape; 3] =
    [QueryShape::All, QueryShape::Bounded2d, QueryShape::LocatableLocal];

const RETRY_SECONDS: f32 = 5.0;
/// How often the Scene is queried again for changes.
pub(crate) const REFRESH_SECONDS: f32 = 15.0;
const MAX_RESULTS: u32 = 128;
/// Fallback floor when Space Setup has no FLOOR entity.
const FLOOR_HALF_EXTENT_M: f32 = 10.0;
/// Free roam: the floor is this big (half extent, real metres) and is moved
/// under the skater, a grid cell of this size at a time.
const FREE_FLOOR_HALF_M: f32 = 24.0;
const FREE_FLOOR_CELL_M: f32 = 8.0;
const UPWARD_MIN_COS: f32 = 0.9;
/// Anything bigger is the room-sized global mesh, not furniture.
const MAX_OBJECT_HALF_M: f32 = 2.0;
const RECOGNIZED_LABELS: &std::ffi::CStr = c"TABLE,COUCH,FLOOR,CEILING,WALL_FACE,WINDOW_FRAME,DOOR_FRAME,STORAGE,BED,SCREEN,LAMP,PLANT,WALL_ART,GLOBAL_MESH,INVISIBLE_WALL_FACE,OTHER";
/// Labels that are not something to skate on or bump into. (The global mesh
/// is used through its triangle mesh, never its room-sized box.)
const IGNORED_LABELS: [&str; 5] = [
    "CEILING",
    "INVISIBLE_WALL_FACE",
    "WINDOW_FRAME",
    "DOOR_FRAME",
    "WALL_ART",
];

pub(crate) struct PlanesPlugin;

impl Plugin for PlanesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DetectedPlanes>()
            .init_resource::<SceneShapes>()
            .init_resource::<RoomMesh>()
            .init_resource::<SceneRevision>()
            .init_resource::<SceneLabels>()
            .init_resource::<SceneQuery>()
            .init_resource::<SceneCapture>()
            .add_oxr_event_handler(on_scene_event)
            .add_systems(
                PreUpdate,
                (capture_scene, query_scene.run_if(scene_enabled), locate_surfaces)
                    .chain()
                    .run_if(openxr_session_running),
            );
    }
}

fn scene_enabled(exts: Option<Res<OxrEnabledExtensions>>) -> bool {
    exts.is_some_and(|e| e.fb_spatial_entity && e.fb_spatial_entity_query && e.fb_scene)
}

fn query_scene(
    instance: Res<OxrInstance>,
    session: Res<OxrSession>,
    time: Res<Time<Real>>,
    mut query: ResMut<SceneQuery>,
) {
    if query.pending.is_some() {
        return;
    }
    let shape = if !query.surfaces.is_empty() {
        // Refresh: the same query again, to see whether the room changed.
        query.since_refresh += time.delta_secs();
        let Some(shape) = query.accepted else { return };
        if query.since_refresh < REFRESH_SECONDS {
            return;
        }
        query.since_refresh = 0.0;
        query.refreshing = true;
        shape
    } else {
        if let Some(t) = query.since_attempt.as_mut() {
            *t += time.delta_secs();
            if *t < RETRY_SECONDS {
                return;
            }
        }
        query.since_attempt = Some(0.0);
        query.refreshing = false;
        let shape = QUERY_SHAPES[query.attempt % QUERY_SHAPES.len()];
        query.attempt += 1;
        shape
    };
    let Some(fp) = instance.exts().fb_spatial_entity_query.as_ref() else {
        return;
    };
    let storage = sys::SpaceStorageLocationFilterInfoFB {
        ty: sys::SpaceStorageLocationFilterInfoFB::TYPE,
        next: ptr::null(),
        location: sys::SpaceStorageLocationFB::LOCAL,
    };
    // Planes and furniture boxes carry different bounding components, which
    // are read per space afterwards.
    let filter = sys::SpaceComponentFilterInfoFB {
        ty: sys::SpaceComponentFilterInfoFB::TYPE,
        next: match shape {
            QueryShape::LocatableLocal => (&storage as *const sys::SpaceStorageLocationFilterInfoFB).cast(),
            _ => ptr::null(),
        },
        component_type: match shape {
            QueryShape::Bounded2d => sys::SpaceComponentTypeFB::BOUNDED_2D,
            _ => sys::SpaceComponentTypeFB::LOCATABLE,
        },
    };
    let info = sys::SpaceQueryInfoFB {
        ty: sys::SpaceQueryInfoFB::TYPE,
        next: ptr::null(),
        query_action: sys::SpaceQueryActionFB::LOAD,
        max_result_count: MAX_RESULTS,
        timeout: sys::Duration::NONE,
        filter: match shape {
            QueryShape::All => ptr::null(),
            _ => (&filter as *const sys::SpaceComponentFilterInfoFB).cast(),
        },
        exclude_filter: ptr::null(),
    };
    let mut request = sys::AsyncRequestIdFB::from_raw(0);
    let result = unsafe {
        (fp.query_spaces)(
            session.as_raw(),
            (&info as *const sys::SpaceQueryInfoFB).cast(),
            &mut request,
        )
    };
    match cvt(result) {
        Ok(_) => {
            if !query.refreshing {
                info!("xrQuerySpacesFB accepted ({shape:?})");
            }
            query.accepted = Some(shape);
            query.pending = Some(request);
        }
        Err(e) => {
            query.refreshing = false;
            warn!("xrQuerySpacesFB ({shape:?}): {e:?}, retrying");
        }
    }
}

fn capture_scene(
    instance: Res<OxrInstance>,
    session: Res<OxrSession>,
    mut capture: ResMut<SceneCapture>,
) {
    if !capture.requested || capture.pending.is_some() {
        return;
    }
    capture.requested = false;
    let Some(fp) = instance.exts().fb_scene_capture.as_ref() else {
        warn!("Room scan unavailable: XR_FB_scene_capture is not enabled");
        return;
    };
    let info = sys::SceneCaptureRequestInfoFB {
        ty: sys::SceneCaptureRequestInfoFB::TYPE,
        next: ptr::null(),
        request_byte_count: 0,
        request: ptr::null(),
    };
    let mut request = sys::AsyncRequestIdFB::from_raw(0);
    match cvt(unsafe { (fp.request_scene_capture)(session.as_raw(), &info, &mut request) }) {
        Ok(_) => {
            info!("Room scan: Space Setup requested");
            capture.pending = Some(request);
        }
        Err(e) => warn!("Room scan: xrRequestSceneCaptureFB failed: {e:?}"),
    }
}

fn on_scene_event(
    event: OxrEventIn,
    instance: Res<OxrInstance>,
    session: Option<Res<OxrSession>>,
    mut query: ResMut<SceneQuery>,
    mut capture: ResMut<SceneCapture>,
    settings: Res<super::ar_settings::ArSettings>,
) {
    match *event {
        openxr::Event::SceneCaptureCompleteFB(e) if capture.pending == Some(e.request_id()) => {
            capture.pending = None;
            if let Err(err) = cvt(e.result()) {
                warn!("Room scan: Space Setup ended with {err:?}");
            }
            // Query again even on failure/cancel: the old scan may still be
            // there. The old handles are released first.
            for surface in query.surfaces.drain(..) {
                unsafe { (instance.fp().destroy_space)(surface.space) };
            }
            query.changed = true;
            query.since_attempt = None;
            query.attempt = 0;
            info!("Room scan: complete, re-querying the Scene");
        }
        openxr::Event::SpaceQueryResultsAvailableFB(e) if query.pending == Some(e.request_id()) => {
            let Some(session) = session else { return };
            let started = std::time::Instant::now();
            let surfaces = retrieve_surfaces(&instance, &session, e.request_id());
            if query.refreshing {
                debug!("Scene refresh: {} spaces in {:.1} ms", surfaces.len(), started.elapsed().as_secs_f32() * 1000.0);
                query.incoming.extend(surfaces);
                return;
            }
            for s in &surfaces {
                info!(
                    "Scene: {:?} plane={} box={} mesh_triangles={}",
                    s.label,
                    s.rect.is_some(),
                    s.volume.is_some(),
                    s.mesh.as_ref().map_or(0, |(_, i)| i.len() / 3)
                );
            }
            query.surfaces.extend(surfaces);
        }
        openxr::Event::SpaceQueryCompleteFB(e) if query.pending == Some(e.request_id()) => {
            query.pending = None;
            if query.refreshing {
                query.refreshing = false;
                let incoming = std::mem::take(&mut query.incoming);
                if cvt(e.result()).is_err() || incoming.is_empty() {
                    release_unshared(&instance, &incoming, &query.surfaces);
                    return;
                }
                if same_scene(&query.surfaces, &incoming) {
                    release_unshared(&instance, &incoming, &query.surfaces);
                } else {
                    info!(
                        "PLANES live update: Scene changed ({} -> {} entities)",
                        query.surfaces.len(),
                        incoming.len()
                    );
                    let old = std::mem::replace(&mut query.surfaces, incoming);
                    release_unshared(&instance, &old, &query.surfaces);
                    query.changed = true;
                }
                return;
            }
            if let Err(err) = cvt(e.result()) {
                warn!("Scene query failed: {err:?}");
            } else if query.surfaces.is_empty() {
                warn!("Scene has no surfaces (run Space Setup); using the floor only");
            } else {
                info!("Scene: {} entities", query.surfaces.len());
            }
            let scanned = query.surfaces.iter().any(|s| s.label.contains("FLOOR"));
            // Free roam (outside) never wants a room scan.
            if !scanned && !capture.auto_requested && !settings.free_roam() {
                capture.auto_requested = true;
                capture.requested = true;
                info!("Room scan: no room scan found, starting Space Setup");
            }
        }
        _ => {}
    }
}

/// Destroys the spaces in `spaces` whose handles `kept` does not also hold
/// (a re-query may hand back the handle of an already-loaded space).
fn release_unshared(instance: &OxrInstance, spaces: &[SceneSurface], kept: &[SceneSurface]) {
    for s in spaces {
        if !kept.iter().any(|k| k.space == s.space) {
            unsafe { (instance.fp().destroy_space)(s.space) };
        }
    }
}

/// Labels, bounds (to the millimetre) and mesh sizes, order-independent.
fn same_scene(a: &[SceneSurface], b: &[SceneSurface]) -> bool {
    fn key(s: &SceneSurface) -> (String, [i64; 10], usize) {
        let mm = |v: f32| (v * 1000.0).round() as i64;
        let r = s.rect.map_or([0; 4], |r| [r.offset.x, r.offset.y, r.extent.width, r.extent.height].map(mm));
        let v = s.volume.map_or([0; 6], |v| {
            [v.offset.x, v.offset.y, v.offset.z, v.extent.width, v.extent.height, v.extent.depth].map(mm)
        });
        let mut k = [0; 10];
        k[..4].copy_from_slice(&r);
        k[4..].copy_from_slice(&v);
        (s.label.clone(), k, s.mesh.as_ref().map_or(0, |(v, i)| v.len() + i.len()))
    }
    let mut ka: Vec<_> = a.iter().map(key).collect();
    let mut kb: Vec<_> = b.iter().map(key).collect();
    ka.sort();
    kb.sort();
    ka == kb
}

fn retrieve_surfaces(
    instance: &OxrInstance,
    session: &OxrSession,
    request: sys::AsyncRequestIdFB,
) -> Vec<SceneSurface> {
    let (Some(query_fp), Some(scene_fp)) = (
        instance.exts().fb_spatial_entity_query.as_ref(),
        instance.exts().fb_scene.as_ref(),
    ) else {
        return Vec::new();
    };
    let mut results = sys::SpaceQueryResultsFB {
        ty: sys::SpaceQueryResultsFB::TYPE,
        next: ptr::null_mut(),
        result_capacity_input: 0,
        result_count_output: 0,
        results: ptr::null_mut(),
    };
    let retrieve = query_fp.retrieve_space_query_results;
    if let Err(e) = cvt(unsafe { retrieve(session.as_raw(), request, &mut results) }) {
        error!("xrRetrieveSpaceQueryResultsFB (count): {e:?}");
        return Vec::new();
    }
    let mut buf = vec![
        sys::SpaceQueryResultFB {
            space: sys::Space::NULL,
            uuid: sys::UuidEXT { data: [0; 16] },
        };
        results.result_count_output as usize
    ];
    results.result_capacity_input = buf.len() as u32;
    results.results = buf.as_mut_ptr();
    if let Err(e) = cvt(unsafe { retrieve(session.as_raw(), request, &mut results) }) {
        error!("xrRetrieveSpaceQueryResultsFB: {e:?}");
        return Vec::new();
    }
    buf.truncate(results.result_count_output as usize);
    for r in &buf {
        enable_locatable(instance, r.space);
    }
    let session = session.as_raw();
    buf.into_iter()
        .filter_map(|r| {
            let mut rect = sys::Rect2Df::default();
            let rect = cvt(unsafe { (scene_fp.get_space_bounding_box2_d)(session, r.space, &mut rect) })
                .ok()
                .map(|_| rect);
            let mut volume = sys::Rect3DfFB {
                offset: sys::Offset3DfFB { x: 0.0, y: 0.0, z: 0.0 },
                extent: sys::Extent3DfFB { width: 0.0, height: 0.0, depth: 0.0 },
            };
            let volume =
                cvt(unsafe { (scene_fp.get_space_bounding_box3_d)(session, r.space, &mut volume) })
                    .ok()
                    .map(|_| volume);
            let mesh = instance
                .exts()
                .meta_spatial_entity_mesh
                .as_ref()
                .and_then(|fp| triangle_mesh(fp, r.space));
            (rect.is_some() || volume.is_some() || mesh.is_some()).then(|| SceneSurface {
                space: r.space,
                rect,
                volume,
                mesh,
                label: semantic_label(scene_fp, session, r.space),
            })
        })
        .collect()
}

/// Queried spaces can't be located until their LOCATABLE component is
/// enabled: until then `xrLocateSpace` reports no valid pose, and every Scene
/// entity was dropped (only the fallback floor survived). The enable is
/// asynchronous; `locate_surfaces` picks the entities up once it lands.
fn enable_locatable(instance: &OxrInstance, space: sys::Space) {
    let Some(fp) = instance.exts().fb_spatial_entity.as_ref() else {
        return;
    };
    let mut status = sys::SpaceComponentStatusFB {
        ty: sys::SpaceComponentStatusFB::TYPE,
        next: ptr::null_mut(),
        enabled: sys::FALSE,
        change_pending: sys::FALSE,
    };
    let known = cvt(unsafe {
        (fp.get_space_component_status)(space, sys::SpaceComponentTypeFB::LOCATABLE, &mut status)
    });
    if known.is_ok() && (status.enabled == sys::TRUE || status.change_pending == sys::TRUE) {
        return;
    }
    let info = sys::SpaceComponentStatusSetInfoFB {
        ty: sys::SpaceComponentStatusSetInfoFB::TYPE,
        next: ptr::null(),
        component_type: sys::SpaceComponentTypeFB::LOCATABLE,
        enabled: sys::TRUE,
        timeout: sys::Duration::NONE,
    };
    let mut request = sys::AsyncRequestIdFB::from_raw(0);
    match unsafe { (fp.set_space_component_status)(space, &info, &mut request) } {
        sys::Result::ERROR_SPACE_COMPONENT_STATUS_ALREADY_SET_FB
        | sys::Result::ERROR_SPACE_COMPONENT_STATUS_PENDING_FB => {}
        result => {
            if let Err(e) = cvt(result) {
                warn!("Enabling LOCATABLE on a Scene space failed: {e:?}");
            }
        }
    }
}

fn triangle_mesh(
    fp: &openxr::raw::SpatialEntityMeshMETA,
    space: sys::Space,
) -> Option<(Arc<[Vec3]>, Arc<[u32]>)> {
    let info = sys::SpaceTriangleMeshGetInfoMETA {
        ty: sys::SpaceTriangleMeshGetInfoMETA::TYPE,
        next: ptr::null(),
    };
    let mut mesh = sys::SpaceTriangleMeshMETA {
        ty: sys::SpaceTriangleMeshMETA::TYPE,
        next: ptr::null_mut(),
        vertex_capacity_input: 0,
        vertex_count_output: 0,
        vertices: ptr::null_mut(),
        index_capacity_input: 0,
        index_count_output: 0,
        indices: ptr::null_mut(),
    };
    // Errors mean this space has no mesh component; that's most of them.
    cvt(unsafe { (fp.get_space_triangle_mesh)(space, &info, &mut mesh) }).ok()?;
    if mesh.vertex_count_output == 0 || mesh.index_count_output < 3 {
        return None;
    }
    let mut vertices = vec![sys::Vector3f::default(); mesh.vertex_count_output as usize];
    let mut indices = vec![0u32; mesh.index_count_output as usize];
    mesh.vertex_capacity_input = vertices.len() as u32;
    mesh.vertices = vertices.as_mut_ptr();
    mesh.index_capacity_input = indices.len() as u32;
    mesh.indices = indices.as_mut_ptr();
    if let Err(e) = cvt(unsafe { (fp.get_space_triangle_mesh)(space, &info, &mut mesh) }) {
        error!("xrGetSpaceTriangleMeshMETA: {e:?}");
        return None;
    }
    vertices.truncate(mesh.vertex_count_output as usize);
    indices.truncate(mesh.index_count_output as usize - mesh.index_count_output as usize % 3);
    let count = vertices.len() as u32;
    if indices.iter().any(|&i| i >= count) {
        error!("Room mesh has out-of-range indices; ignoring it");
        return None;
    }
    let vertices: Arc<[Vec3]> = vertices.iter().map(|v| Vec3::new(v.x, v.y, v.z)).collect();
    Some((vertices, indices.into()))
}

fn semantic_label(scene_fp: &openxr::raw::SceneFB, session: sys::Session, space: sys::Space) -> String {
    let support = sys::SemanticLabelsSupportInfoFB {
        ty: sys::SemanticLabelsSupportInfoFB::TYPE,
        next: ptr::null(),
        flags: sys::SemanticLabelsSupportFlagsFB::MULTIPLE_SEMANTIC_LABELS
            | sys::SemanticLabelsSupportFlagsFB::ACCEPT_DESK_TO_TABLE_MIGRATION
            | sys::SemanticLabelsSupportFlagsFB::ACCEPT_INVISIBLE_WALL_FACE,
        recognized_labels: RECOGNIZED_LABELS.as_ptr(),
    };
    let mut labels = sys::SemanticLabelsFB {
        ty: sys::SemanticLabelsFB::TYPE,
        next: (&support as *const sys::SemanticLabelsSupportInfoFB).cast(),
        buffer_capacity_input: 0,
        buffer_count_output: 0,
        buffer: ptr::null_mut(),
    };
    let get = scene_fp.get_space_semantic_labels;
    if cvt(unsafe { get(session, space, &mut labels) }).is_err() {
        return String::new();
    }
    let mut buf = vec![0u8; labels.buffer_count_output as usize];
    labels.buffer_capacity_input = buf.len() as u32;
    labels.buffer = buf.as_mut_ptr().cast();
    if cvt(unsafe { get(session, space, &mut labels) }).is_err() {
        return String::new();
    }
    buf.retain(|&b| b != 0);
    String::from_utf8_lossy(&buf).into_owned()
}

fn locate_surfaces(
    instance: Res<OxrInstance>,
    primary_space: Res<XrPrimaryReferenceSpace>,
    frame_state: Option<Res<OxrFrameState>>,
    mut query: ResMut<SceneQuery>,
    mut planes: ResMut<DetectedPlanes>,
    mut shapes: ResMut<SceneShapes>,
    mut labels: ResMut<SceneLabels>,
    mut room: ResMut<RoomMesh>,
    mut revision: ResMut<SceneRevision>,
    mut was_located: Local<Vec<Option<bool>>>,
    settings: Res<super::ar_settings::ArSettings>,
    rig: Res<super::rig::Rig>,
    link: Res<super::link::SkaterLink>,
    mut free_floor: Local<Option<(bool, [i32; 2], u32)>>,
) {
    let Some(frame_state) = frame_state else { return };
    if query.changed {
        query.changed = false;
        was_located.clear();
        revision.0 += 1;
    }
    if settings.free_roam() {
        // Outside: one big floor under the skater, nothing from the room.
        let to_tracking = rig.root_transform().map(|r| r.compute_affine().inverse());
        let at = to_tracking.zip(link.deck).map_or(Vec3::ZERO, |(t, (deck, _))| t.transform_point3(deck));
        let cell = [(at.x / FREE_FLOOR_CELL_M).round() as i32, (at.z / FREE_FLOOR_CELL_M).round() as i32];
        let key = (true, cell, settings.floor_offset.to_bits());
        if *free_floor != Some(key) {
            *free_floor = Some(key);
            revision.0 += 1;
            info!("PLANES free-roam floor at cell {cell:?}, height {:.2} m", settings.floor_offset);
        }
        let floor = Shape::Plane {
            pose: Transform::from_xyz(
                cell[0] as f32 * FREE_FLOOR_CELL_M,
                settings.floor_offset,
                cell[1] as f32 * FREE_FLOOR_CELL_M,
            ),
            half: Vec2::splat(FREE_FLOOR_HALF_M),
            floor: true,
        };
        room.0 = None;
        planes.0 = upward_surface(&floor).into_iter().collect();
        if labels.0.len() != 1 {
            labels.0 = vec!["FLOOR".into()];
        }
        if shapes.0 != [floor] {
            shapes.0 = vec![floor];
        }
        return;
    }
    if free_floor.take().is_some() {
        // Back from free roam: the room's surfaces again.
        was_located.clear();
        revision.0 += 1;
    }
    was_located.resize(query.surfaces.len(), None);
    let mut located_labels = Vec::new();
    let base = sys::Space::from_raw(primary_space.as_raw());
    let mut located = Vec::new();
    let mut room_mesh = None;
    for (i, surface) in query.surfaces.iter().enumerate() {
        if IGNORED_LABELS.iter().any(|l| surface.label.contains(l)) {
            continue;
        }
        let mut location = sys::SpaceLocation {
            ty: sys::SpaceLocation::TYPE,
            next: ptr::null_mut(),
            location_flags: sys::SpaceLocationFlags::EMPTY,
            pose: sys::Posef::IDENTITY,
        };
        let result = unsafe {
            (instance.fp().locate_space)(
                surface.space,
                base,
                frame_state.predicted_display_time,
                &mut location,
            )
        };
        let flags = location.location_flags;
        let ok = cvt(result).is_ok()
            && flags.contains(sys::SpaceLocationFlags::POSITION_VALID)
            && flags.contains(sys::SpaceLocationFlags::ORIENTATION_VALID);
        if was_located[i] != Some(ok) {
            was_located[i] = Some(ok);
            info!("Scene: {:?} located={ok} ({result:?}, {flags:?})", surface.label);
        }
        if !ok {
            continue;
        }
        if let Some((vertices, indices)) = &surface.mesh {
            room_mesh = Some(LocatedMesh {
                pose: pose_transform(location.pose),
                vertices: vertices.clone(),
                indices: indices.clone(),
            });
            continue;
        }
        if surface.label.contains("GLOBAL_MESH") {
            continue;
        }
        // Furniture carries both a box and its top plane; the box is the
        // whole object, so it wins.
        if let Some(volume) = surface.volume {
            if let Some(shape) = to_box(location.pose, volume) {
                located.push(shape);
                located_labels.push(surface.label.clone());
            }
        } else if let Some(rect) = surface.rect {
            let shape = to_plane(location.pose, rect, surface.label.contains("FLOOR"));
            if let Shape::Plane { pose, .. } = shape {
                // Ceilings without a label still face down; skip them.
                if (pose.rotation * Vec3::Y).y > -0.5 {
                    located.push(shape);
                    located_labels.push(surface.label.clone());
                }
            }
        }
    }
    let has_floor = located
        .iter()
        .any(|s| matches!(s, Shape::Plane { floor: true, .. }));
    if !has_floor {
        located.push(Shape::Plane {
            pose: Transform::IDENTITY,
            half: Vec2::splat(FLOOR_HALF_EXTENT_M),
            floor: true,
        });
        located_labels.push("FLOOR".into());
    }
    if labels.0 != located_labels {
        labels.0 = located_labels;
    }
    if shapes.0.len() != located.len() || room.0.is_some() != room_mesh.is_some() {
        revision.0 += 1;
    }
    room.0 = room_mesh;
    planes.0 = located.iter().filter_map(upward_surface).collect();
    if shapes.0 != located {
        shapes.0 = located;
    }
}

/// The upward face a skater could be placed on.
fn upward_surface(shape: &Shape) -> Option<DetectedPlane> {
    match *shape {
        Shape::Plane { pose, half, .. } => Some(DetectedPlane {
            pose,
            half_extent: half,
            upward: (pose.rotation * Vec3::Y).y >= UPWARD_MIN_COS,
        }),
        Shape::Box { pose, half } => {
            let (axis, sign) = up_axis(pose.rotation);
            let normal = pose.rotation * (axis * sign);
            // A plane frame on the top face: +Y along the face normal.
            let rotation = Quat::from_rotation_arc(pose.rotation * Vec3::Y, normal) * pose.rotation;
            Some(DetectedPlane {
                pose: Transform {
                    translation: pose.translation + normal * axis.dot(half).abs(),
                    rotation,
                    scale: Vec3::ONE,
                },
                half_extent: top_extent(half, axis),
                upward: normal.y >= UPWARD_MIN_COS,
            })
        }
    }
}

/// The box's local axis (and sign) whose direction is most upward.
pub(crate) fn up_axis(rotation: Quat) -> (Vec3, f32) {
    [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .map(|a| {
            let y = (rotation * a).y;
            (a, y.signum(), y.abs())
        })
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(a, s, _)| (a, s))
        .expect("three axes")
}

/// Conservative top-face half extents: the smaller of the two horizontal
/// half sizes on both axes, so a placement snap never lands past the edge
/// whatever the face's in-plane orientation is.
fn top_extent(half: Vec3, up: Vec3) -> Vec2 {
    let horizontal: Vec<f32> = [(Vec3::X, half.x), (Vec3::Y, half.y), (Vec3::Z, half.z)]
        .into_iter()
        .filter(|(a, _)| *a != up)
        .map(|(_, h)| h)
        .collect();
    Vec2::splat(horizontal[0].min(horizontal[1]))
}

fn pose_transform(pose: sys::Posef) -> Transform {
    let (p, o) = (pose.position, pose.orientation);
    Transform {
        translation: Vec3::new(p.x, p.y, p.z),
        rotation: Quat::from_xyzw(o.x, o.y, o.z, o.w).normalize(),
        scale: Vec3::ONE,
    }
}

/// Scene planes lie in their space's local X/Y with +Z as the normal; turn
/// that into this module's convention (+Y normal, extent in X/Z) and centre
/// the pose on the bounding rectangle.
fn to_plane(pose: sys::Posef, rect: sys::Rect2Df, floor: bool) -> Shape {
    let space = pose_transform(pose);
    let half = Vec2::new(rect.extent.width, rect.extent.height) * 0.5;
    let centre = Vec3::new(rect.offset.x + half.x, rect.offset.y + half.y, 0.0);
    Shape::Plane {
        pose: Transform {
            translation: space.transform_point(centre),
            rotation: space.rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        },
        half,
        floor,
    }
}

fn to_box(pose: sys::Posef, volume: sys::Rect3DfFB) -> Option<Shape> {
    let space = pose_transform(pose);
    let (o, e) = (volume.offset, volume.extent);
    let half = Vec3::new(e.width, e.height, e.depth) * 0.5;
    if half.min_element() <= 0.0 || half.max_element() > MAX_OBJECT_HALF_M {
        return None;
    }
    let centre = Vec3::new(o.x, o.y, o.z) + half;
    Some(Shape::Box {
        pose: Transform {
            translation: space.transform_point(centre),
            rotation: space.rotation,
            scale: Vec3::ONE,
        },
        half,
    })
}

fn cvt(x: sys::Result) -> Result<sys::Result, sys::Result> {
    if x.into_raw() >= 0 { Ok(x) } else { Err(x) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(rotation: Quat, position: Vec3) -> sys::Posef {
        sys::Posef {
            orientation: sys::Quaternionf { x: rotation.x, y: rotation.y, z: rotation.z, w: rotation.w },
            position: sys::Vector3f { x: position.x, y: position.y, z: position.z },
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> sys::Rect2Df {
        sys::Rect2Df {
            offset: sys::Offset2Df { x, y },
            extent: sys::Extent2Df { width: w, height: h },
        }
    }

    #[test]
    fn floor_with_up_facing_z_becomes_an_upward_plane() {
        // A Scene floor: +Z (the plane normal) rotated to point up.
        let rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let Shape::Plane { pose: p, half, .. } =
            to_plane(pose(rotation, Vec3::new(1.0, 0.0, -2.0)), rect(-0.6, -0.4, 1.2, 0.8), true)
        else {
            unreachable!()
        };
        assert!((p.rotation * Vec3::Y).distance(Vec3::Y) < 1e-5);
        assert!(p.translation.distance(Vec3::new(1.0, 0.0, -2.0)) < 1e-5);
        assert_eq!(half, Vec2::new(0.6, 0.4));
    }

    #[test]
    fn table_box_top_is_a_snap_target_whichever_axis_is_up() {
        // Box local Z up (Meta's convention for some volumes) and local Y up.
        for rotation in [Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2), Quat::IDENTITY] {
            let local_up = rotation.inverse() * Vec3::Y;
            let half = Vec3::splat(0.5) + local_up.abs() * (0.375 - 0.5);
            let shape = Shape::Box {
                pose: Transform::from_translation(Vec3::new(0.0, 0.375, 0.0)).with_rotation(rotation),
                half,
            };
            let top = upward_surface(&shape).unwrap();
            assert!(top.upward);
            assert!((top.pose.translation.y - 0.75).abs() < 1e-5, "{:?}", top.pose);
            let planes = DetectedPlanes(vec![top]);
            let snapped = planes.snap(Vec3::new(0.2, 0.9, -0.1)).unwrap();
            assert!(snapped.distance(Vec3::new(0.2, 0.75, -0.1)) < 1e-4);
        }
    }

    #[test]
    fn room_sized_boxes_are_not_furniture() {
        let volume = sys::Rect3DfFB {
            offset: sys::Offset3DfFB { x: -3.0, y: -3.0, z: 0.0 },
            extent: sys::Extent3DfFB { width: 6.0, height: 6.0, depth: 2.5 },
        };
        assert!(to_box(pose(Quat::IDENTITY, Vec3::ZERO), volume).is_none());
    }
}
