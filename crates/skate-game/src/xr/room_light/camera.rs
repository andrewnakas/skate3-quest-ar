//! The headset's passthrough camera (Meta's Passthrough Camera API, through
//! the NDK's Camera2), feeding the capture: Quest 3 and 3S, Horizon OS v74
//! or later, with the player's `horizonos.permission.HEADSET_CAMERA`.
//!
//! One thread. While room lighting is wanted and allowed it streams the left
//! passthrough camera at its smallest square size and lowest frame rate,
//! and five times a second reduces the latest frame to a 32x32 grid of
//! colour, which goes to Roomtone with the frame's exposure and the head's
//! pose. The camera shares passthrough's auto exposure, which is how the
//! light's level follows what the player sees. Frames never leave this
//! thread; only the grids do, and nothing is stored.
use super::feed::{CameraState, Feed};
use super::reduce::{self, GRID, Plane};
use bevy::log::{info, warn};
use ndk_sys as sys;
use roomtone_sys::{Camera, Frame, Quat, Room};
use std::ffi::{CString, c_int, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[link(name = "camera2ndk")]
unsafe extern "C" {}

/// Between frames: Roomtone wants a look around the room, not video.
const FRAME_EVERY: Duration = Duration::from_millis(200);
/// Frames are left out this long after the camera opens or the headset comes
/// back (on the face, out of the system menu): auto exposure is still
/// settling, and the first images can be black or far off.
const SETTLE: Duration = Duration::from_secs(1);
/// At most one line this often about black frames.
const BLACK_LOG_EVERY: Duration = Duration::from_secs(10);
const IDLE_EVERY: Duration = Duration::from_millis(500);
/// A camera that keeps failing is left alone until room lighting is turned
/// off and on again.
const MAX_FAILURES: u32 = 3;
const AIMAGE_FORMAT_YUV_420_888: i32 = 0x23;

pub(super) fn spawn(feed: Arc<Feed>, room: Arc<Mutex<Room>>) {
    let result = std::thread::Builder::new().name("sk8-room-light".into()).spawn(move || run(&feed, &room));
    if let Err(e) = result {
        warn!("Room light: no capture thread: {e}");
    }
}

fn run(feed: &Feed, room: &Mutex<Room>) {
    let linear = reduce::linear_table();
    let mut cells = vec![0.0f32; GRID * GRID * 4];
    let mut stream: Option<Stream> = None;
    let mut failures = 0;
    let started = Instant::now();
    // Frames count from here (the camera opened, the head came back).
    let mut usable_from = Instant::now();
    let mut black_logged: Option<Instant> = None;
    loop {
        if !feed.wanted() {
            if stream.take().is_some() {
                info!("Room light: camera closed");
            }
            failures = 0;
            feed.set_state(CameraState::Off);
            std::thread::sleep(IDLE_EVERY);
            continue;
        }
        if stream.as_ref().is_some_and(|s| s.shared.failed.load(Ordering::Relaxed)) {
            warn!("Room light: the camera stopped");
            stream = None;
            failures += 1;
        }
        let Some(open) = stream.as_mut() else {
            if !crate::camera_access() {
                feed.set_state(CameraState::NoPermission);
            } else if failures >= MAX_FAILURES {
                feed.set_state(CameraState::NoCamera);
            } else {
                feed.set_state(CameraState::Starting);
                match Stream::open() {
                    Ok(opened) => {
                        info!("Room light: camera {} at {}x{}", opened.id, opened.camera.width, opened.camera.height);
                        room.lock().unwrap().set_camera(&opened.camera);
                        stream = Some(opened);
                        usable_from = Instant::now() + SETTLE;
                    }
                    Err(why) => {
                        warn!("Room light: no camera: {why}");
                        failures += 1;
                    }
                }
            }
            std::thread::sleep(IDLE_EVERY);
            continue;
        };
        std::thread::sleep(FRAME_EVERY);
        // No head while the headset isn't showing the game (off the face, the
        // system menu): what the camera sees then isn't the room as played in.
        let Some(head) = feed.head() else {
            usable_from = Instant::now() + SETTLE;
            continue;
        };
        if !open.latest(&linear, &mut cells) || Instant::now() < usable_from {
            continue;
        }
        feed.set_state(CameraState::Running);
        let frame = Frame {
            cells: &cells,
            cols: GRID,
            rows: GRID,
            // A frame without them is left out by Roomtone.
            exposure_ms: open.shared.exposure_ns.load(Ordering::Relaxed) as f32 / 1e6,
            iso: open.shared.iso.load(Ordering::Relaxed) as f32,
            head,
            time_ns: started.elapsed().as_nanos() as i64,
        };
        let mut room = room.lock().unwrap();
        let result = feed.add_frame(&mut room, &frame);
        if result.restarted {
            info!("Room light: the room's light changed ({:.2}x), starting over", result.change);
        }
        if result.black && black_logged.is_none_or(|at| at.elapsed() >= BLACK_LOG_EVERY) {
            info!("Room light: the camera's image is black (covered or stopped): keeping the light");
            black_logged = Some(Instant::now());
        }
    }
}

/// What the camera's callbacks write.
#[derive(Default)]
struct Shared {
    /// The latest result's exposure and sensitivity (0: none yet).
    exposure_ns: AtomicI64,
    iso: AtomicI32,
    failed: AtomicBool,
}

/// An open camera streaming into an image reader. Dropping it closes
/// everything, in the order Camera2 wants.
struct Stream {
    id: String,
    camera: Camera,
    shared: Arc<Shared>,
    manager: *mut sys::ACameraManager,
    device: *mut sys::ACameraDevice,
    reader: *mut sys::AImageReader,
    session: *mut sys::ACameraCaptureSession,
    request: *mut sys::ACaptureRequest,
    target: *mut sys::ACameraOutputTarget,
    output: *mut sys::ACaptureSessionOutput,
    outputs: *mut sys::ACaptureSessionOutputContainer,
    // The NDK keeps pointers to these while the device and session live.
    _device_callbacks: Box<sys::ACameraDevice_StateCallbacks>,
    _session_callbacks: Box<sys::ACameraCaptureSession_stateCallbacks>,
    _capture_callbacks: Box<sys::ACameraCaptureSession_captureCallbacks>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            if !self.session.is_null() {
                sys::ACameraCaptureSession_close(self.session);
            }
            if !self.request.is_null() {
                sys::ACaptureRequest_free(self.request);
            }
            if !self.target.is_null() {
                sys::ACameraOutputTarget_free(self.target);
            }
            if !self.outputs.is_null() {
                sys::ACaptureSessionOutputContainer_free(self.outputs);
            }
            if !self.output.is_null() {
                sys::ACaptureSessionOutput_free(self.output);
            }
            if !self.device.is_null() {
                sys::ACameraDevice_close(self.device);
            }
            if !self.reader.is_null() {
                sys::AImageReader_delete(self.reader);
            }
            if !self.manager.is_null() {
                sys::ACameraManager_delete(self.manager);
            }
        }
    }
}

unsafe extern "C" fn on_disconnected(context: *mut c_void, _: *mut sys::ACameraDevice) {
    unsafe { &*(context as *const Shared) }.failed.store(true, Ordering::Relaxed);
}

unsafe extern "C" fn on_error(context: *mut c_void, _: *mut sys::ACameraDevice, _: c_int) {
    unsafe { &*(context as *const Shared) }.failed.store(true, Ordering::Relaxed);
}

unsafe extern "C" fn on_session(_: *mut c_void, _: *mut sys::ACameraCaptureSession) {}

unsafe extern "C" fn on_completed(
    context: *mut c_void,
    _: *mut sys::ACameraCaptureSession,
    _: *mut sys::ACaptureRequest,
    result: *const sys::ACameraMetadata,
) {
    let shared = unsafe { &*(context as *const Shared) };
    if let Some(&[ns]) = unsafe { entry_i64(result, sys::acamera_metadata_tag::ACAMERA_SENSOR_EXPOSURE_TIME) } {
        shared.exposure_ns.store(ns, Ordering::Relaxed);
    }
    if let Some(&[iso]) = unsafe { entry_i32(result, sys::acamera_metadata_tag::ACAMERA_SENSOR_SENSITIVITY) } {
        shared.iso.store(iso, Ordering::Relaxed);
    }
}

unsafe fn entry(metadata: *const sys::ACameraMetadata, tag: sys::acamera_metadata_tag) -> Option<sys::ACameraMetadata_const_entry> {
    let mut entry = unsafe { std::mem::zeroed::<sys::ACameraMetadata_const_entry>() };
    let status = unsafe { sys::ACameraMetadata_getConstEntry(metadata, tag.0, &mut entry) };
    (status == sys::camera_status_t::ACAMERA_OK && entry.count > 0).then_some(entry)
}

// The entry types Camera2 gives these tags (ACAMERA_TYPE_*): 1 int32, 2 float, 3 int64.
unsafe fn entry_i32<'a>(metadata: *const sys::ACameraMetadata, tag: sys::acamera_metadata_tag) -> Option<&'a [i32]> {
    let e = unsafe { entry(metadata, tag) }.filter(|e| e.type_ == 1)?;
    Some(unsafe { std::slice::from_raw_parts(e.data.i32_, e.count as usize) })
}

unsafe fn entry_f32<'a>(metadata: *const sys::ACameraMetadata, tag: sys::acamera_metadata_tag) -> Option<&'a [f32]> {
    let e = unsafe { entry(metadata, tag) }.filter(|e| e.type_ == 2)?;
    Some(unsafe { std::slice::from_raw_parts(e.data.f, e.count as usize) })
}

unsafe fn entry_i64<'a>(metadata: *const sys::ACameraMetadata, tag: sys::acamera_metadata_tag) -> Option<&'a [i64]> {
    let e = unsafe { entry(metadata, tag) }.filter(|e| e.type_ == 3)?;
    Some(unsafe { std::slice::from_raw_parts(e.data.i64_, e.count as usize) })
}

fn camera_ok(status: sys::camera_status_t, what: &str) -> Result<(), String> {
    if status == sys::camera_status_t::ACAMERA_OK { Ok(()) } else { Err(format!("{what}: camera status {}", status.0)) }
}

fn media_ok(status: sys::media_status_t, what: &str) -> Result<(), String> {
    if status == sys::media_status_t::AMEDIA_OK { Ok(()) } else { Err(format!("{what}: media status {}", status.0)) }
}

impl Stream {
    fn open() -> Result<Self, String> {
        use sys::acamera_metadata_tag as tag;
        // The left passthrough camera, by Meta's vendor tags (the entry point
        // looks it up: the NDK can't read a vendor tag by its name).
        let id = crate::passthrough_camera().ok_or("no passthrough camera on this headset")?;
        let c_id = CString::new(id.clone()).map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared::default());
        let context = Arc::as_ptr(&shared) as *mut c_void;
        let mut stream = Stream {
            id,
            camera: Camera::default(),
            shared,
            manager: unsafe { sys::ACameraManager_create() },
            device: null_mut(),
            reader: null_mut(),
            session: null_mut(),
            request: null_mut(),
            target: null_mut(),
            output: null_mut(),
            outputs: null_mut(),
            _device_callbacks: Box::new(sys::ACameraDevice_StateCallbacks {
                context,
                onDisconnected: Some(on_disconnected),
                onError: Some(on_error),
            }),
            _session_callbacks: Box::new(sys::ACameraCaptureSession_stateCallbacks {
                context,
                onClosed: Some(on_session),
                onReady: Some(on_session),
                onActive: Some(on_session),
            }),
            _capture_callbacks: Box::new(sys::ACameraCaptureSession_captureCallbacks {
                context,
                onCaptureStarted: None,
                onCaptureProgressed: None,
                onCaptureCompleted: Some(on_completed),
                onCaptureFailed: None,
                onCaptureSequenceCompleted: None,
                onCaptureSequenceAborted: None,
                onCaptureBufferLost: None,
            }),
        };
        if stream.manager.is_null() {
            return Err("no camera manager".into());
        }

        // Its calibration, its smallest square size and its lowest frame rate.
        let mut characteristics = null_mut();
        camera_ok(
            unsafe { sys::ACameraManager_getCameraCharacteristics(stream.manager, c_id.as_ptr(), &mut characteristics) },
            "characteristics",
        )?;
        let described = unsafe { describe(characteristics) };
        unsafe { sys::ACameraMetadata_free(characteristics) };
        let (camera, fps) = described?;
        stream.camera = camera;

        unsafe {
            media_ok(
                sys::AImageReader_new(camera.width, camera.height, AIMAGE_FORMAT_YUV_420_888, 2, &mut stream.reader),
                "image reader",
            )?;
            let mut window = null_mut();
            media_ok(sys::AImageReader_getWindow(stream.reader, &mut window), "image reader window")?;
            camera_ok(
                sys::ACameraManager_openCamera(stream.manager, c_id.as_ptr(), &mut *stream._device_callbacks, &mut stream.device),
                "open",
            )?;
            camera_ok(
                sys::ACameraDevice_createCaptureRequest(
                    stream.device,
                    sys::ACameraDevice_request_template::TEMPLATE_PREVIEW,
                    &mut stream.request,
                ),
                "request",
            )?;
            camera_ok(sys::ACameraOutputTarget_create(window, &mut stream.target), "target")?;
            camera_ok(sys::ACaptureRequest_addTarget(stream.request, stream.target), "add target")?;
            if let Some(range) = fps {
                // Not fatal: the stream just runs faster than it needs to.
                let _ = sys::ACaptureRequest_setEntry_i32(
                    stream.request,
                    tag::ACAMERA_CONTROL_AE_TARGET_FPS_RANGE.0,
                    2,
                    range.as_ptr(),
                );
            }
            camera_ok(sys::ACaptureSessionOutputContainer_create(&mut stream.outputs), "outputs")?;
            camera_ok(sys::ACaptureSessionOutput_create(window, &mut stream.output), "output")?;
            camera_ok(sys::ACaptureSessionOutputContainer_add(stream.outputs, stream.output), "add output")?;
            camera_ok(
                sys::ACameraDevice_createCaptureSession(
                    stream.device,
                    stream.outputs,
                    &*stream._session_callbacks,
                    &mut stream.session,
                ),
                "session",
            )?;
            camera_ok(
                sys::ACameraCaptureSession_setRepeatingRequest(
                    stream.session,
                    &mut *stream._capture_callbacks,
                    1,
                    &mut stream.request,
                    null_mut(),
                ),
                "repeating request",
            )?;
        }
        Ok(stream)
    }

    /// Reduces the newest frame into `cells`; false when there is none yet.
    fn latest(&mut self, linear: &[f32; 256], cells: &mut [f32]) -> bool {
        unsafe {
            let mut image = null_mut();
            if sys::AImageReader_acquireLatestImage(self.reader, &mut image) != sys::media_status_t::AMEDIA_OK
                || image.is_null()
            {
                return false;
            }
            let plane = |index: c_int| -> Option<Plane<'static>> {
                let (mut data, mut length, mut row, mut pixel) = (null_mut(), 0, 0, 0);
                let ok = sys::AImage_getPlaneData(image, index, &mut data, &mut length) == sys::media_status_t::AMEDIA_OK
                    && sys::AImage_getPlaneRowStride(image, index, &mut row) == sys::media_status_t::AMEDIA_OK
                    && sys::AImage_getPlanePixelStride(image, index, &mut pixel) == sys::media_status_t::AMEDIA_OK;
                (ok && !data.is_null() && length > 0).then(|| Plane {
                    // (Valid until the image is deleted, below.)
                    data: std::slice::from_raw_parts(data, length as usize),
                    row_stride: row as usize,
                    pixel_stride: pixel as usize,
                })
            };
            let (mut width, mut height) = (0, 0);
            sys::AImage_getWidth(image, &mut width);
            sys::AImage_getHeight(image, &mut height);
            let planes = (plane(0), plane(1), plane(2));
            let reduced = if let ((Some(y), Some(u), Some(v)), true) = (planes, width > 0 && height > 0) {
                reduce::reduce(y, u, v, width as usize, height as usize, linear, cells);
                true
            } else {
                false
            };
            sys::AImage_delete(image);
            reduced
        }
    }
}

/// The camera's calibration with its smallest square stream size, and its
/// slowest frame rate range.
unsafe fn describe(characteristics: *const sys::ACameraMetadata) -> Result<(Camera, Option<[i32; 2]>), String> {
    use sys::acamera_metadata_tag as tag;
    let rotation = unsafe { entry_f32(characteristics, tag::ACAMERA_LENS_POSE_ROTATION) }.filter(|r| r.len() >= 4);
    let intrinsics = unsafe { entry_f32(characteristics, tag::ACAMERA_LENS_INTRINSIC_CALIBRATION) }.filter(|i| i.len() >= 4);
    // (left, top, width, height)
    let array = unsafe { entry_i32(characteristics, tag::ACAMERA_SENSOR_INFO_PRE_CORRECTION_ACTIVE_ARRAY_SIZE) }
        .filter(|a| a.len() >= 4);
    let (Some(rotation), Some(intrinsics), Some(array)) = (rotation, intrinsics, array) else {
        return Err("the camera has no calibration".into());
    };
    // (format, width, height, is input) each.
    let sizes = unsafe { entry_i32(characteristics, tag::ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS) }.unwrap_or(&[]);
    let side = sizes
        .chunks_exact(4)
        .filter(|c| c[0] == AIMAGE_FORMAT_YUV_420_888 && c[3] == 0 && c[1] == c[2] && c[1] >= (GRID * 5) as i32)
        .map(|c| c[1])
        .min()
        .ok_or("the camera has no square size")?;
    let fps = unsafe { entry_i32(characteristics, tag::ACAMERA_CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES) }
        .and_then(|ranges| ranges.chunks_exact(2).min_by_key(|r| r[1]).map(|r| [r[0], r[1]]));
    let camera = Camera {
        lens_rotation: Quat { x: rotation[0], y: rotation[1], z: rotation[2], w: rotation[3] },
        fx: intrinsics[0],
        fy: intrinsics[1],
        cx: intrinsics[2],
        cy: intrinsics[3],
        sensor_width: array[2],
        sensor_height: array[3],
        width: side,
        height: side,
    };
    Ok((camera, fps))
}
