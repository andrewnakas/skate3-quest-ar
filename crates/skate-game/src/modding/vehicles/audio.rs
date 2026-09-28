//! Shared synthesized engine voices; remote volume attenuates with distance.
use super::engine_sound::{Engine, Profile};
use bevy::{
    audio::{AddAudioSource, Volume},
    prelude::*,
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Component)]
struct EngineVoice(u64);
#[derive(Resource, Default)]
struct State {
    sources: BTreeMap<Profile, Handle<Engine>>,
    mix: BTreeMap<u64, (f32, f32)>,
}
pub(super) fn install(app: &mut App) {
    app.add_audio_source::<Engine>()
        .init_resource::<State>()
        .add_systems(Update, update.after(super::present));
}
fn update(
    mut commands: Commands,
    vehicles: Res<super::Vehicles>,
    time: Res<Time<Real>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    replay: Res<crate::replay::Replay>,
    skater: Res<crate::physics::SkaterRuntime>,
    mut state: ResMut<State>,
    mut sources: ResMut<Assets<Engine>>,
    mut voices: Query<(Entity, &EngineVoice, &mut AudioSink)>,
    pending: Query<&EngineVoice>,
) {
    let active = crate::graphics_menu::gameplay_active(menu) && !replay.active;
    let listener = vehicles
        .player_pose()
        .map(|p| Vec3::from_array(p.0))
        .unwrap_or_else(|| {
            crate::animation::native_matrix(skater.animated_skeleton.roots.animation_to_world)
                .w_axis
                .truncate()
        });
    let existing: BTreeSet<_> = pending.iter().map(|v| v.0).collect();
    let mut targets = BTreeMap::new();
    for (&id, car) in &vehicles.simulation.vehicles {
        let a = &car.definition.engine_audio;
        if !a.enabled || !super::network::occupied(&vehicles, id) {
            continue;
        }
        let Some((p, _)) = vehicles.simulation.pose(id) else {
            continue;
        };
        let distance = listener.distance(Vec3::from_array(p));
        if distance > 80. {
            continue;
        }
        let local = vehicles
            .driver
            .as_ref()
            .and_then(|d| vehicles.owned.get(&(d.owner.clone(), d.key.clone())))
            .is_some_and(|i| i.id == id);
        let attenuation = if local {
            1.
        } else {
            1. / (1. + distance * distance / 36.)
        };
        let speed =
            (car.controller.current_vehicle_speed.abs() / car.definition.max_speed).clamp(0., 1.);
        let throttle = car.controls.throttle.abs();
        // Chassis speed cannot describe a freestyle engine: the whole point of a
        // clutch and a rev in the air is that the bike is not moving. Take revs
        // from the driven wheel, which keeps turning off the ground and against
        // the brake, and let throttle rev it freely on top.
        let revs = if car.definition.bike.enabled {
            let wheel = vehicles.simulation.wheel_speed(id).abs()
                * car.definition.wheels.iter().find(|w| w.driven).map_or(0.3, |w| w.radius);
            let geared = (wheel / car.definition.max_speed).clamp(0., 1.);
            // A four-stroke single never sits at idle under load: the pull is
            // mostly the wheel, with the throttle free-revving over the top.
            (geared * 0.7 + throttle * 0.45).clamp(0., 1.)
        } else {
            (speed * 0.65 + throttle * 0.35).clamp(0., 1.)
        };
        let pitch = a.idle_pitch + (a.max_pitch - a.idle_pitch) * revs;
        let volume = if active {
            a.volume * (0.22 + 0.65 * throttle + 0.13 * speed) * attenuation
        } else {
            0.
        };
        targets.insert(id, (pitch, volume));
        if !existing.contains(&id) {
            let profile = Profile::parse(&a.profile).unwrap_or_default();
            let source = state
                .sources
                .entry(profile)
                .or_insert_with(|| sources.add(Engine(profile)))
                .clone();
            commands.spawn((
                EngineVoice(id),
                AudioPlayer(source),
                PlaybackSettings::ONCE
                    .with_volume(Volume::Linear(0.))
                    .with_speed(pitch),
            ));
        }
        state.mix.entry(id).or_insert((pitch, 0.));
    }
    let dt = time.delta_secs().min(0.1);
    for (&id, (pitch, volume)) in &mut state.mix {
        let (p, v) = targets.get(&id).copied().unwrap_or((*pitch, 0.));
        *pitch += (p - *pitch) * (1. - (-7. * dt).exp());
        *volume += (v - *volume) * (1. - (-12. * dt).exp());
    }
    for (entity, voice, mut sink) in &mut voices {
        let (pitch, volume) = state.mix.get(&voice.0).copied().unwrap_or((1., 0.));
        sink.set_speed(pitch.max(0.25));
        sink.set_volume(Volume::Linear(volume));
        if !targets.contains_key(&voice.0) && volume < 0.001 {
            commands.entity(entity).despawn();
            state.mix.remove(&voice.0);
        }
    }
}
