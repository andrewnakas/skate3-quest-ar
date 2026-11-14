//! What the capture's thread shares with the game: the head's pose and the
//! room's surfaces go in, and each camera frame is added with them.
use super::depth::{self, Surfaces};
use roomtone_sys::{DepthImage, Frame, FrameResult, Pose, Room};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// How the capture is doing, for the settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum CameraState {
    /// Not asked for.
    Off = 0,
    /// The player hasn't allowed the cameras (yet).
    NoPermission = 1,
    /// This headset has no passthrough camera an app can open.
    NoCamera = 2,
    Starting = 3,
    Running = 4,
}

#[derive(Default)]
pub(crate) struct Feed {
    /// The game wants frames (room lighting is on and allowed).
    wanted: AtomicBool,
    state: AtomicU8,
    /// The head now, tracking space.
    head: Mutex<Option<Pose>>,
    surfaces: Mutex<Option<Arc<Surfaces>>>,
    depth: Mutex<Vec<u16>>,
}

// (The camera's side of it is the headset's.)
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl Feed {
    pub(crate) fn set_wanted(&self, wanted: bool) {
        self.wanted.store(wanted, Ordering::Relaxed);
    }

    pub(crate) fn wanted(&self) -> bool {
        self.wanted.load(Ordering::Relaxed)
    }

    pub(crate) fn set_state(&self, state: CameraState) {
        self.state.store(state as u8, Ordering::Relaxed);
    }

    pub(crate) fn state(&self) -> CameraState {
        match self.state.load(Ordering::Relaxed) {
            1 => CameraState::NoPermission,
            2 => CameraState::NoCamera,
            3 => CameraState::Starting,
            4 => CameraState::Running,
            _ => CameraState::Off,
        }
    }

    pub(crate) fn set_head(&self, head: Option<Pose>) {
        *self.head.lock().unwrap() = head;
    }

    pub(crate) fn head(&self) -> Option<Pose> {
        *self.head.lock().unwrap()
    }

    pub(crate) fn set_surfaces(&self, surfaces: Option<Arc<Surfaces>>) {
        *self.surfaces.lock().unwrap() = surfaces;
    }

    /// Adds a camera frame (its `head` already set) to the capture, behind
    /// a depth image of the room from the same pose when the room is known.
    pub(crate) fn add_frame(&self, room: &mut Room, frame: &Frame) -> FrameResult {
        let surfaces = self.surfaces.lock().unwrap().clone();
        if let Some(surfaces) = surfaces {
            let mut depth = self.depth.lock().unwrap();
            let p = frame.head.position;
            let q = frame.head.orientation;
            let view = surfaces.depth_image(
                bevy::math::Vec3::new(p.x, p.y, p.z),
                bevy::math::Quat::from_xyzw(q.x, q.y, q.z, q.w),
                &mut depth,
            );
            room.add_depth(&DepthImage {
                depth: &depth,
                width: depth::SIZE,
                height: depth::SIZE,
                stride: depth::SIZE,
                near_z: view.near_z,
                far_z: 0.0,
                tan_left: -view.tan,
                tan_right: view.tan,
                tan_up: view.tan,
                tan_down: -view.tan,
                view: frame.head,
                time_ns: frame.time_ns,
            });
        }
        room.add_frame(frame)
    }
}
