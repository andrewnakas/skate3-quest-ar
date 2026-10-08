//! Line pieces up with the furniture they are dropped on: when a piece's yaw
//! is within [`SNAP_ANGLE`] of a table's edge direction, it takes that
//! direction exactly, so a rail or ledge runs straight along the table.
use bevy::prelude::*;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

pub(crate) const SNAP_ANGLE: f32 = 10.0 * PI / 180.0;
/// How far outside a top face a point may be and still count as on it, m.
const MARGIN_M: f32 = 0.05;

/// A box-shaped surface in tracking space: rotation, centre, half extents.
pub(crate) type SurfaceBox = (Quat, Vec3, Vec3);

/// Heading (yaw about +Y, the convention of `rig::heading`) of a direction.
fn heading_of(direction: Vec3) -> f32 {
    (-direction.x).atan2(-direction.z)
}

/// `a - b` wrapped into (-π, π].
fn angle_between(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(TAU);
    if d > PI { d - TAU } else { d }
}

/// `yaw`, or the nearest edge direction of the box whose top `point` (tracking
/// space) is on, if that is within [`SNAP_ANGLE`].
pub(crate) fn snap_yaw(boxes: impl IntoIterator<Item = SurfaceBox>, point: Vec3, yaw: f32) -> f32 {
    for (rotation, centre, half) in boxes {
        let local = rotation.inverse() * (point - centre);
        if (local.abs() - half).max_element() > MARGIN_M {
            continue;
        }
        // A horizontal box axis: whichever of X/Z (in the box frame) is
        // least vertical. The other edges are this one plus quarter turns.
        let edge = [Vec3::X, Vec3::Y, Vec3::Z]
            .into_iter()
            .map(|a| rotation * a)
            .min_by(|a, b| a.y.abs().total_cmp(&b.y.abs()))
            .expect("three axes");
        let base = heading_of(edge.with_y(0.0));
        let nearest = (0..4)
            .map(|k| base + k as f32 * FRAC_PI_2)
            .min_by(|a, b| angle_between(*a, yaw).abs().total_cmp(&angle_between(*b, yaw).abs()))
            .expect("four edges");
        if angle_between(nearest, yaw).abs() <= SNAP_ANGLE {
            return yaw + angle_between(nearest, yaw);
        }
        return yaw;
    }
    yaw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(yaw: f32) -> SurfaceBox {
        (Quat::from_rotation_y(yaw), Vec3::new(1.0, 0.375, -1.0), Vec3::new(0.6, 0.375, 0.4))
    }

    #[test]
    fn close_yaws_line_up_with_the_table() {
        let top = Vec3::new(1.1, 0.75, -1.05);
        for table_yaw in [0.0, 0.3, -1.2] {
            for k in 0..4 {
                let edge = table_yaw + k as f32 * FRAC_PI_2;
                let snapped = snap_yaw([table(table_yaw)], top_of(table_yaw, top), edge + 0.1);
                assert!(angle_between(snapped, edge).abs() < 1e-4, "{table_yaw} {k}: {snapped}");
            }
        }
    }

    #[test]
    fn far_yaws_and_the_floor_are_left_alone() {
        let top = Vec3::new(1.1, 0.75, -1.05);
        assert_eq!(snap_yaw([table(0.0)], top, 0.5), 0.5, "30° off an edge stays");
        assert_eq!(snap_yaw([table(0.0)], Vec3::new(3.0, 0.0, 3.0), 0.05), 0.05, "not on the table");
    }

    fn top_of(table_yaw: f32, point: Vec3) -> Vec3 {
        let (rotation, centre, _) = table(table_yaw);
        let local = Quat::from_rotation_y(0.0).inverse() * (point - Vec3::new(1.0, 0.375, -1.0));
        centre + rotation * local
    }
}
