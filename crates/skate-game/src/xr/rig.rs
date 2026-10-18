//! The tabletop rig: where the miniature sits in the room, and how big it is.
//!
//! The XR tracking root is the only thing that changes. For a tracking-space
//! point `p` the world point is `root.translation + root.rotation * (k * p)`,
//! so choosing
//!
//! ```text
//! rotation    = yaw(-anchor_yaw)
//! translation = W - rotation * (k * anchor_position)
//! ```
//!
//! puts the world point `W` (the spawn's ground point) exactly at the anchor
//! in the room, with world -Z along the anchor's forward direction, and the
//! whole world `k` times smaller than life.
//!
//! The anchor is the STAGE origin (the Guardian's floor centre) facing STAGE
//! -Z, so the world is pinned to the room the same way every launch: a saved
//! layout (`xr::build`) comes back on the same tables.
use super::XrOptions;
use bevy::prelude::*;
use bevy_mod_xr::session::XrTrackingRoot;

#[derive(Resource, Debug)]
pub(crate) struct Rig {
    pub scale: f32,
    /// World point shown at the anchor: the map spawn's ground point, or the
    /// test course's landing floor under the spawn.
    pub world_origin: Vec3,
    /// Tracking-space anchor position and yaw.
    pub anchor: Option<(Vec3, f32)>,
}

impl Rig {
    pub(crate) fn root_transform(&self) -> Option<Transform> {
        let (position, yaw) = self.anchor?;
        Some(root_transform(self.scale, self.world_origin, position, yaw))
    }
}

pub(crate) fn root_transform(scale: f32, world_origin: Vec3, anchor: Vec3, yaw: f32) -> Transform {
    let rotation = Quat::from_rotation_y(-yaw);
    Transform {
        translation: world_origin - rotation * (scale * anchor),
        rotation,
        scale: Vec3::splat(scale),
    }
}

/// Yaw about +Y that turns -Z onto the horizontal part of `rotation`'s forward.
pub(crate) fn heading(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::NEG_Z;
    (-forward.x).atan2(-forward.z)
}

pub(crate) struct RigPlugin(pub XrOptions);

impl Plugin for RigPlugin {
    fn build(&self, app: &mut App) {
        // The clean tier has no game config: its world floor is y = 0.
        let world_origin = app
            .world()
            .get_resource::<crate::config::Config>()
            .map_or(Vec3::ZERO, spawn_ground);
        app.insert_resource(Rig {
            scale: self.0.scale,
            world_origin,
            anchor: Some((Vec3::ZERO, 0.0)),
        })
        .add_systems(
            PostUpdate,
            apply_root.before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

fn spawn_ground(config: &crate::config::Config) -> Vec3 {
    match &config.map {
        Some(map) => Vec3::from_array(map.spawn),
        // The test course's landing-floor height sits on the real floor.
        None => Vec3::new(0.0, crate::physics::ground::FLOOR_HEIGHT, 0.0),
    }
}

fn apply_root(rig: Res<Rig>, mut root: Query<&mut Transform, With<XrTrackingRoot>>) {
    let (Some(target), Ok(mut transform)) = (rig.root_transform(), root.single_mut()) else {
        return;
    };
    if *transform != target {
        *transform = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_world(root: &Transform, tracking: Vec3) -> Vec3 {
        root.transform_point(tracking)
    }

    #[test]
    fn anchor_shows_the_world_origin() {
        let origin = Vec3::new(10.0, -0.035, -4.0);
        let anchor = Vec3::new(0.3, 0.75, -0.6);
        for yaw in [0.0, 1.0, -2.5] {
            let root = root_transform(12.0, origin, anchor, yaw);
            assert!(to_world(&root, anchor).distance(origin) < 1e-4);
        }
    }

    #[test]
    fn room_distances_are_scaled_up_in_the_world() {
        let root = root_transform(12.0, Vec3::ZERO, Vec3::ZERO, 0.7);
        let a = to_world(&root, Vec3::new(0.0, 0.0, 0.0));
        let b = to_world(&root, Vec3::new(0.1, 0.0, 0.0));
        assert!((a.distance(b) - 1.2).abs() < 1e-5, "10 cm of table is 1.2 m of world");
    }

    #[test]
    fn anchor_forward_becomes_world_negative_z() {
        let yaw = 0.9;
        let controller = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-0.4);
        assert!((heading(controller) - yaw).abs() < 1e-5, "pitch does not change heading");
        let root = root_transform(5.0, Vec3::ZERO, Vec3::ZERO, heading(controller));
        let forward = (root.rotation * (controller * Vec3::NEG_Z)).with_y(0.0).normalize();
        assert!(forward.distance(Vec3::NEG_Z) < 1e-4);
    }
}
