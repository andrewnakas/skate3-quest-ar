//! Depth image to tracking-space points (`depth`), kept apart from the capture so it builds and
//! is tested off the headset.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]
use bevy::prelude::*;
use openxr::sys;

/// Every STRIDE-th pixel, both ways.
const STRIDE: usize = 4;
/// Depth past this is too noisy to trust, metres (the error grows with the square of it).
const MAX_RANGE_M: f32 = 4.5;
const MIN_RANGE_M: f32 = 0.35;

/// One depth frame's view: the runtime's near and far planes, and the left view.
pub(crate) struct InFlight {
    pub near: f32,
    pub far: f32,
    pub view: sys::EnvironmentDepthImageViewMETA,
}

pub(crate) fn pose_position(p: sys::Posef) -> Vec3 {
    Vec3::new(p.position.x, p.position.y, p.position.z)
}

pub(crate) fn pose_rotation(p: sys::Posef) -> Quat {
    Quat::from_xyzw(p.orientation.x, p.orientation.y, p.orientation.z, p.orientation.w)
}

/// Tracking-space points from the left view's depth, every [`STRIDE`]th pixel. The depth is the
/// runtime's 16-bit [0, 1] buffer depth of an OpenGL-style projection (an infinite far plane when
/// `far` is infinite); `flip` reads the rows bottom-up.
pub(crate) fn unproject(pixels: &[u16], (w, h): (u32, u32), job: &InFlight, flip: bool) -> Vec<Vec3> {
    let fov = job.view.fov;
    let (l, r, u, d) = (fov.angle_left.tan(), fov.angle_right.tan(), fov.angle_up.tan(), fov.angle_down.tan());
    let (pos, rot) = (pose_position(job.view.pose), pose_rotation(job.view.pose));
    let (n, f) = (job.near, job.far);
    let mut out = Vec::with_capacity((w as usize / STRIDE) * (h as usize / STRIDE));
    for row in (STRIDE / 2..h as usize).step_by(STRIDE) {
        for col in (STRIDE / 2..w as usize).step_by(STRIDE) {
            let raw = pixels[row * w as usize + col];
            if raw == 0 || raw == u16::MAX {
                continue;
            }
            let ndc = raw as f32 / 65535.0 * 2.0 - 1.0;
            let z = if f.is_finite() && f > n { 2.0 * f * n / ((f + n) - ndc * (f - n)) } else { 2.0 * n / (1.0 - ndc) };
            if !(MIN_RANGE_M..=MAX_RANGE_M).contains(&z) {
                continue;
            }
            let x = (col as f32 + 0.5) / w as f32;
            let y = (row as f32 + 0.5) / h as f32;
            let tx = l + x * (r - l);
            let ty = if flip { d + y * (u - d) } else { u + y * (d - u) };
            out.push(pos + rot * (Vec3::new(tx, ty, -1.0) * z));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    fn job(pitch: f32) -> InFlight {
        let rot = Quat::from_rotation_x(pitch);
        let fov = 0.8f32;
        InFlight {
            near: 0.1,
            far: f32::INFINITY,
            view: sys::EnvironmentDepthImageViewMETA {
                ty: sys::EnvironmentDepthImageViewMETA::TYPE,
                next: ptr::null(),
                fov: sys::Fovf { angle_left: -fov, angle_right: fov, angle_up: fov, angle_down: -fov },
                pose: sys::Posef {
                    orientation: sys::Quaternionf { x: rot.x, y: rot.y, z: rot.z, w: rot.w },
                    position: sys::Vector3f { x: 0.0, y: 1.6, z: 0.0 },
                },
            },
        }
    }

    #[test]
    fn a_floor_depth_image_unprojects_onto_the_floor() {
        // Render the floor (y = 0) from 1.6 m up, looking 50 degrees down, rows top-down.
        let (w, h) = (64u32, 64u32);
        let j = job(-0.87);
        let rot = pose_rotation(j.view.pose);
        let t = 0.8f32.tan();
        let mut pixels = vec![0u16; (w * h) as usize];
        for row in 0..h {
            for col in 0..w {
                let tx = -t + (col as f32 + 0.5) / w as f32 * 2.0 * t;
                let ty = t - (row as f32 + 0.5) / h as f32 * 2.0 * t;
                let dir = rot * Vec3::new(tx, ty, -1.0);
                if dir.y >= -1e-3 {
                    continue;
                }
                let along = 1.6 / -dir.y; // view-space z of the floor hit
                let ndc = 1.0 - 2.0 * 0.1 / along;
                pixels[(row * w + col) as usize] = (((ndc + 1.0) * 0.5) * 65535.0).round() as u16;
            }
        }
        let right = unproject(&pixels, (w, h), &j, false);
        assert!(right.len() > 50);
        assert!(right.iter().all(|p| p.y.abs() < 0.03), "{:?}", right.iter().map(|p| p.y).fold(0f32, f32::max));
        let wrong = unproject(&pixels, (w, h), &j, true);
        assert!(wrong.iter().filter(|p| p.y.abs() < 0.12).count() < right.len() / 4);
    }
}
