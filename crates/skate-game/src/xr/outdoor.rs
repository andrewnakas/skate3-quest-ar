//! Outdoor park mode without a room: the tracking space and the Guardian boundary.
//!
//! In a room the app tracks in STAGE space (the Guardian's floor centre), so a layout comes back
//! on the same tables every launch. Outdoors there is no room to set up and a stationary
//! boundary is a couple of metres across, so park mode tracks in LOCAL_FLOOR space (the floor
//! where the headset started, no boundary needed) and asks the runtime to hide the boundary while
//! passthrough shows the real park (`XR_META_boundary_visibility`, bound by hand: openxr-sys has
//! no binding for it). The manifest's `com.oculus.feature.BOUNDARYLESS_APP` lets the app start
//! with no boundary set up at all.
use super::ar_settings::ArSettings;
use super::scan::ParkScan;
use bevy::math::Isometry3d;
use bevy::prelude::*;
use bevy_mod_openxr::{openxr_session_running, resources::OxrInstance, session::OxrSession};
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use openxr::sys;

/// `XR_META_boundary_visibility`.
pub(crate) const BOUNDARY_VISIBILITY_EXT: &[u8] = b"XR_META_boundary_visibility";
/// `XrBoundaryVisibilityMETA`: 1 not suppressed, 2 suppressed.
const BOUNDARY_NOT_SUPPRESSED: i32 = 1;
const BOUNDARY_SUPPRESSED: i32 = 2;
type RequestBoundaryVisibility = unsafe extern "system" fn(sys::Session, i32) -> sys::Result;

pub(crate) struct OutdoorPlugin;

impl Plugin for OutdoorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OutdoorState>()
            .add_systems(Update, (tracking_space, boundary).chain().run_if(openxr_session_running));
    }
}

#[derive(Resource, Default)]
struct OutdoorState {
    /// The space type the primary space was last made as by us (None: the plugin's STAGE).
    space: Option<sys::ReferenceSpaceType>,
    /// LOCAL_FLOOR couldn't be made: stay in STAGE.
    no_local_floor: bool,
    /// The boundary visibility last asked for (and whether the call exists at all).
    boundary: Option<i32>,
    boundary_fn: Option<Option<RequestBoundaryVisibility>>,
}

/// Park mode tracks in LOCAL_FLOOR, a room in STAGE; switched live when the mode changes.
fn tracking_space(
    settings: Res<ArSettings>,
    session: Res<OxrSession>,
    primary: Option<Res<XrPrimaryReferenceSpace>>,
    mut state: ResMut<OutdoorState>,
    mut scan: ResMut<ParkScan>,
    mut commands: Commands,
) {
    // (wait for the plugin's own STAGE space, made when the session is)
    if primary.is_none() {
        return;
    }
    let want = if settings.free_roam() && !state.no_local_floor {
        sys::ReferenceSpaceType::LOCAL_FLOOR
    } else {
        sys::ReferenceSpaceType::STAGE
    };
    let current = state.space.unwrap_or(sys::ReferenceSpaceType::STAGE);
    if want == current {
        return;
    }
    match session.create_reference_space(want, Isometry3d::IDENTITY) {
        Ok(space) => {
            commands.insert_resource(XrPrimaryReferenceSpace(space));
            state.space = Some(want);
            // (the walked scan was in the old space's coordinates)
            scan.clear_depth();
            info!("OUTDOOR tracking space now {want:?}");
        }
        Err(e) => {
            warn!("OUTDOOR can't make a {want:?} space ({e}): staying in {current:?}");
            if want == sys::ReferenceSpaceType::LOCAL_FLOOR {
                state.no_local_floor = true;
            }
        }
    }
}

/// Park mode hides the Guardian boundary (passthrough shows the real park); a room shows it.
fn boundary(
    settings: Res<ArSettings>,
    instance: Res<OxrInstance>,
    session: Res<OxrSession>,
    mut state: ResMut<OutdoorState>,
) {
    let want = if settings.free_roam() { BOUNDARY_SUPPRESSED } else { BOUNDARY_NOT_SUPPRESSED };
    if state.boundary == Some(want) {
        return;
    }
    let f = *state.boundary_fn.get_or_insert_with(|| {
        let mut f: Option<sys::pfn::VoidFunction> = None;
        let name = c"xrRequestBoundaryVisibilityMETA";
        let r = unsafe { (instance.entry().fp().get_instance_proc_addr)(instance.as_raw(), name.as_ptr(), &mut f) };
        if r != sys::Result::SUCCESS || f.is_none() {
            info!("OUTDOOR no boundary visibility control ({r:?}): the boundary stays as the system has it");
            return None;
        }
        // SAFETY: the runtime hands out this name's function, whose signature is the extension's.
        Some(unsafe { std::mem::transmute::<sys::pfn::VoidFunction, RequestBoundaryVisibility>(f.unwrap()) })
    });
    state.boundary = Some(want);
    let Some(f) = f else { return };
    let r = unsafe { f(session.as_raw(), want) };
    info!("OUTDOOR boundary {} ({r:?})", if want == BOUNDARY_SUPPRESSED { "hidden" } else { "shown" });
}
