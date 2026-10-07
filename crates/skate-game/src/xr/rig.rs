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
//! Placement mode: hold the left menu button for a second (without using it
//! as a D-pad/Back chord). While placing, the miniature follows the right
//! controller and the pad is neutral; press A to set it down. A new session
//! starts in placement mode.
use super::actions::{Hand, LatestTouch};
use super::planes::DetectedPlanes;
use super::XrOptions;
use crate::input::ExternalPad;
use bevy::prelude::*;
use bevy_mod_openxr::openxr_session_running;
use bevy_mod_xr::session::{XrSessionCreated, XrTrackingRoot};
use skate_core::input::xbox::XboxState;

const HOLD_TO_PLACE_SECONDS: f32 = 1.0;
/// Left stick deflection that makes a menu hold a D-pad chord instead.
const CHORD_STICK: f32 = 0.3;

#[derive(Resource, Debug)]
pub(crate) struct Rig {
    pub scale: f32,
    /// World point shown at the anchor: the map spawn's ground point, or the
    /// test course's landing floor under the spawn.
    pub world_origin: Vec3,
    /// Tracking-space anchor position and yaw.
    pub anchor: Option<(Vec3, f32)>,
    pub placing: bool,
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
        let world_origin = spawn_ground(app.world().resource::<crate::config::Config>());
        app.insert_resource(Rig {
            scale: self.0.scale,
            world_origin,
            anchor: None,
            placing: true,
        })
        .add_systems(XrSessionCreated, |mut rig: ResMut<Rig>| rig.placing = true)
        .add_systems(
            PreUpdate,
            placement
                .after(super::actions::TouchRead)
                .before(crate::input::poll_controllers)
                .run_if(openxr_session_running),
        )
        .add_systems(
            PostUpdate,
            apply_root.before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

fn spawn_ground(config: &crate::config::Config) -> Vec3 {
    match &config.map {
        Some(map) => Vec3::from_array(map.spawn),
        // The test course's (hidden) landing floor sits on the real surface;
        // the spawn platform, ramp, half pipe, rails and ledge stand on it.
        None => Vec3::new(0.0, crate::physics::ground::FLOOR_HEIGHT, 0.0),
    }
}

fn placement(
    touch: Res<LatestTouch>,
    time: Res<Time<Real>>,
    hands: Query<(&Hand, &Transform)>,
    planes: Res<DetectedPlanes>,
    mut rig: ResMut<Rig>,
    mut pad: ResMut<ExternalPad>,
    mut hold: Local<Option<f32>>,
    mut toggled: Local<bool>,
    mut a_was_down: Local<bool>,
) {
    let touch = &touch.0;
    let chording = touch.x
        || touch.left_stick[0].abs() > CHORD_STICK
        || touch.left_stick[1].abs() > CHORD_STICK;
    *hold = match (*hold, touch.menu && !chording) {
        (_, false) => {
            *toggled = false;
            None
        }
        (None, true) => Some(0.0),
        (Some(t), true) => Some(t + time.delta_secs()),
    };
    if hold.is_some_and(|t| t >= HOLD_TO_PLACE_SECONDS) && !*toggled {
        *toggled = true;
        rig.placing = !rig.placing;
        info!("XR placement mode: {}", rig.placing);
    }

    let a_pressed = touch.a && !*a_was_down;
    *a_was_down = touch.a;
    if !rig.placing {
        return;
    }
    pad.set(XboxState {
        buttons: 0,
        triggers: [0, 0],
        left: [0, 0],
        right: [0, 0],
    });
    let Some((_, aim)) = hands.iter().find(|(hand, _)| matches!(hand, Hand::Right)) else {
        return;
    };
    // Height comes from the real surface under the controller (table or
    // floor), heading from where the controller points.
    let position = planes.snap(aim.translation).unwrap_or(aim.translation);
    rig.anchor = Some((position, heading(aim.rotation)));
    if a_pressed {
        rig.placing = false;
        info!("XR miniature placed at {:?}", rig.anchor);
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
