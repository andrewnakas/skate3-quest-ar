//! Skate sounds in 3D: the headset is the listener and the skater the emitter
//! (`skate_audio::clean` plays every sound at the skater when
//! [`SpatialSkateAudio`] is present).
//!
//! Bevy's spatial audio pans between two ears and falls off with the inverse
//! square of distance, clamped to full volume within one unit. Positions are
//! game-world metres, which are `k` times real metres (see [`rig`]), so the
//! spatial scale is set to `1 / (k * REFERENCE_M)`: one unit is
//! [`REFERENCE_M`] of real room, whatever size the world is scaled to. The
//! listener sits between the eye cameras, so its ear gap is also real metres.
//!
//! [`rig`]: super::rig
use super::rig::Rig;
use crate::skate_audio::clean::SpatialSkateAudio;
use bevy::audio::{DefaultSpatialScale, SpatialListener, SpatialScale};
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;

/// Real distance from the head inside which the skater is at full volume, m (a skater across the
/// room still heard well).
/// Twice as far is a quarter of the level.
const REFERENCE_M: f32 = 3.0;
/// Real distance between the ears, m.
const EAR_GAP_M: f32 = 0.18;

#[derive(Component)]
struct HeadListener;

pub(crate) struct SpatialAudioPlugin;

impl Plugin for SpatialAudioPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SpatialSkateAudio)
            .add_systems(Startup, |mut commands: Commands| {
                commands.spawn((HeadListener, SpatialListener::new(EAR_GAP_M), Transform::default()));
            })
            .add_systems(Update, follow_head);
    }
}

/// Put the listener at the head (between the eyes) in the game world, and keep
/// the spatial scale in real metres.
fn follow_head(
    rig: Res<Rig>,
    eyes: Query<&Transform, (With<XrCamera>, Without<HeadListener>)>,
    mut listener: Query<&mut Transform, With<HeadListener>>,
    mut scale: ResMut<DefaultSpatialScale>,
) {
    let (Some(root), Ok(mut listener)) = (rig.root_transform(), listener.single_mut()) else {
        return;
    };
    let mut count = 0.0;
    let mut position = Vec3::ZERO;
    let mut rotation = None;
    for eye in &eyes {
        position += eye.translation;
        rotation.get_or_insert(eye.rotation);
        count += 1.0;
    }
    let Some(rotation) = rotation else { return };
    // Eye cameras live under the tracking root (tracking space); the root maps
    // tracking space into the game world, scale included, so the ear gap
    // scales with the world like the room does.
    let head = Transform::from_translation(position / count).with_rotation(rotation);
    let target = root.mul_transform(head);
    if *listener != target {
        *listener = target;
    }
    let units = SpatialScale::new(1.0 / (rig.scale * REFERENCE_M));
    if scale.0.0 != units.0 {
        scale.0 = units;
    }
}
