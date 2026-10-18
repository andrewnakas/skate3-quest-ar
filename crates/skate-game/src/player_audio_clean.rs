//! Clean player sounds: a small sample driver for builds without the retail audio runtime.
//!
//! The retail-exact player audio (`player_audio.rs`) needs the decoded retail banks, grains and
//! the recovered authored graph. On the Quest that chain is fragile, so the AR build plays the
//! skate3-cleanroom web build's 31 sounds instead: real CC0 skateboard recordings plus a few
//! resynthesised ones (`clean-audio/`, extracted by `tools/extract_clean_audio.py`, credits in
//! `clean-audio/CREDITS.json`). They are compiled into the binary, so nothing has to be pushed
//! to the headset.
//!
//! The driver reads the same `PlayerAudioObservation` the retail worker does:
//! - loops (rolling per surface, grinds, powerslide) run all the time at zero volume and are
//!   faded and pitched by speed each frame;
//! - one-shots (pop, flip, catch, landing, push, footstep, bail, the multiplier chime) fire on
//!   state edges, with a random variant and a little pitch spread.
//!
//! `SKATE_AUDIO=clean|retail` overrides the default (clean on Android, retail elsewhere).
//!
//! In AR ([`SpatialSkateAudio`] present) every sound is a spatial emitter at the skater, heard
//! through the headset's listener (`xr::spatial_audio`), so it pans and fades as the skater
//! moves around the room.
use std::sync::Arc;

use bevy::audio::{AudioSinkPlayback, SpatialAudioSink, Volume};
use bevy::prelude::*;
use skate_core::physics::phase::PhysicsEvent;

use super::{FrontEndSoundRequest, PlayerAudioObservation, StreamPcm};

/// Whether this build plays the clean sounds instead of the retail player audio.
pub(crate) fn enabled() -> bool {
    match std::env::var("SKATE_AUDIO").as_deref() {
        Ok("clean") => true,
        Ok("retail") => false,
        _ => cfg!(target_os = "android"),
    }
}

macro_rules! clean_wavs {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/clean-audio/", $name, ".wav")) as &[u8])),*]
    };
}

const WAVS: &[(&str, &[u8])] = clean_wavs!(
    "bail_1", "bail_2", "catch_1", "catch_2", "catch_3", "chime", "flip_1", "flip_2", "flip_3",
    "footstep_1", "footstep_2", "footstep_3", "footstep_4", "grind_concrete", "grind_metal",
    "land_impact_1", "land_impact_2", "land_impact_3", "land_impact_4", "pop_1", "pop_2", "pop_3",
    "pop_4", "powerslide", "push_1", "push_2", "push_3", "roll_metal", "roll_rough",
    "roll_smooth", "roll_wood",
);

/// Overall level; the CC0 files are normalised per category (hits peak near -4 dBFS, loops sit
/// around -24 dBFS RMS).
const MASTER: f32 = 0.9;
/// Loops are mastered quiet so they sit under the hits; this restores them.
const LOOP_GAIN: f32 = 1.6;
/// Volume fade time constant, seconds.
const FADE_S: f32 = 0.06;
/// Below this board speed the wheels are silent, m/s.
const ROLL_MIN_SPEED: f32 = 0.25;
/// Board speed at which rolling is at full volume, m/s.
const ROLL_FULL_SPEED: f32 = 7.0;
/// How close to a hinted rail the skater must be for a metal grind, metres.
const METAL_RAIL_DISTANCE_M: f32 = 0.6;
/// Delay from a flip's start to its catch, seconds.
const CATCH_DELAY_S: f32 = 0.32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Loop {
    RollSmooth,
    RollRough,
    RollWood,
    RollMetal,
    GrindMetal,
    GrindConcrete,
    Powerslide,
}

impl Loop {
    const ALL: [Loop; 7] = [
        Loop::RollSmooth,
        Loop::RollRough,
        Loop::RollWood,
        Loop::RollMetal,
        Loop::GrindMetal,
        Loop::GrindConcrete,
        Loop::Powerslide,
    ];

    fn file(self) -> &'static str {
        match self {
            Loop::RollSmooth => "roll_smooth",
            Loop::RollRough => "roll_rough",
            Loop::RollWood => "roll_wood",
            Loop::RollMetal => "roll_metal",
            Loop::GrindMetal => "grind_metal",
            Loop::GrindConcrete => "grind_concrete",
            Loop::Powerslide => "powerslide",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shot {
    Pop,
    Flip,
    Catch,
    Land,
    Push,
    Footstep,
    Bail,
    Chime,
}

impl Shot {
    const ALL: [Shot; 8] = [
        Shot::Pop,
        Shot::Flip,
        Shot::Catch,
        Shot::Land,
        Shot::Push,
        Shot::Footstep,
        Shot::Bail,
        Shot::Chime,
    ];

    fn prefix(self) -> &'static str {
        match self {
            Shot::Pop => "pop",
            Shot::Flip => "flip",
            Shot::Catch => "catch",
            Shot::Land => "land_impact",
            Shot::Push => "push",
            Shot::Footstep => "footstep",
            Shot::Bail => "bail",
            Shot::Chime => "chime",
        }
    }
}

/// Where in the world the clean driver should hear something other than the default surface,
/// filled by whoever builds the collision (the AR build's placed pieces). World space, the same
/// space as `RetailAudioInputs::position`.
#[derive(Resource, Default, Clone, Debug)]
pub(crate) struct SurfaceHints {
    /// Metal rails and copings: a grind near one is a metal grind.
    pub metal_rails: Vec<(Vec3, Vec3)>,
    /// Wooden ramps (min, max): rolling inside one is rolling on wood.
    pub wood_boxes: Vec<(Vec3, Vec3)>,
}

impl SurfaceHints {
    fn near_metal(&self, p: Vec3) -> bool {
        self.metal_rails
            .iter()
            .any(|&(a, b)| segment_distance(p, a, b) <= METAL_RAIL_DISTANCE_M)
    }

    fn on_wood(&self, p: Vec3) -> bool {
        self.wood_boxes.iter().any(|&(min, max)| {
            let lo = min - Vec3::new(0.15, 0.1, 0.15);
            let hi = max + Vec3::new(0.15, 1.2, 0.15);
            p.cmpge(lo).all() && p.cmple(hi).all()
        })
    }
}

fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-8 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Present when sounds should come from the skater in 3D (the XR build inserts it).
#[derive(Resource, Default)]
pub(crate) struct SpatialSkateAudio;

#[derive(Resource)]
struct CleanAudio {
    spatial: bool,
    /// The skater's latest world position, where spatial sounds are emitted.
    source: Vec3,
    shots: Vec<(Shot, Vec<Handle<StreamPcm>>)>,
    loops: Vec<(Loop, Entity, f32)>,
    targets: [(f32, f32); 7],
    last_pick: [usize; 8],
    rng: u32,
    previous: Option<Edges>,
    pending_catch: Option<f32>,
    last_land: f32,
    last_wipe: f32,
    paused: bool,
    seen_surfaces: u128,
}

/// What the previous observation said, for edge detection.
#[derive(Clone, Default)]
struct Edges {
    state: u32,
    wiping_out: bool,
    pushing: bool,
    stepping: bool,
    trick: Option<String>,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<SurfaceHints>()
        .add_systems(Startup, setup)
        .add_systems(Update, (observe, frontend, mix, follow_skater).chain());
}

/// A one-shot from the clean-room engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Sk8Cue {
    Pop,
    /// A flip trick started: whoosh now, catch a moment later.
    Flip,
    /// Touchdown, with the speed into the surface.
    Land(f32),
    Push,
    Bail,
    /// A combo was banked.
    Chime,
}

/// What the clean-room engine did this frame; the clean tier (`crate::clean`)
/// fills it before the driver runs.
#[derive(Resource, Default, Debug)]
pub(crate) struct Sk8Sounds {
    pub position: Vec3,
    pub speed: f32,
    pub grounded: bool,
    pub grinding: bool,
    pub powersliding: bool,
    pub paused: bool,
    pub cues: Vec<Sk8Cue>,
}

/// The driver for the clean-room engine: same sounds, loops and mixing, fed
/// by [`Sk8Sounds`] instead of the retail physics observation.
pub(crate) fn install_sk8(app: &mut App) {
    use bevy::audio::AddAudioSource;
    app.add_audio_source::<StreamPcm>()
        .init_resource::<SurfaceHints>()
        .init_resource::<Sk8Sounds>()
        .add_systems(Startup, setup)
        .add_systems(Update, (observe_sk8, mix, follow_skater).chain());
}

fn observe_sk8(
    mut commands: Commands,
    audio: Option<ResMut<CleanAudio>>,
    hints: Res<SurfaceHints>,
    sounds: Res<Sk8Sounds>,
    real_time: Res<Time<bevy::time::Real>>,
) {
    let Some(mut audio) = audio else { return };
    let now = real_time.elapsed_secs();
    audio.paused = sounds.paused;
    audio.source = sounds.position;
    if let Some(at) = audio.pending_catch {
        if now >= at {
            audio.pending_catch = None;
            audio.play(&mut commands, Shot::Catch, 0.8);
        }
    }
    for cue in &sounds.cues {
        match *cue {
            Sk8Cue::Pop => audio.play(&mut commands, Shot::Pop, 0.9),
            Sk8Cue::Flip => {
                audio.play(&mut commands, Shot::Flip, 0.7);
                audio.pending_catch = Some(now + CATCH_DELAY_S);
            }
            Sk8Cue::Land(impact) => {
                if now - audio.last_land > 0.15 {
                    audio.last_land = now;
                    audio.pending_catch = None;
                    let gain = (0.45 + impact.abs() / 6.0).clamp(0.45, 1.0);
                    audio.play(&mut commands, Shot::Land, gain);
                }
            }
            Sk8Cue::Push => audio.play(&mut commands, Shot::Push, 0.75),
            Sk8Cue::Bail => {
                if now - audio.last_wipe > 0.8 {
                    audio.last_wipe = now;
                    audio.play(&mut commands, Shot::Bail, 1.0);
                }
            }
            Sk8Cue::Chime => audio.play(&mut commands, Shot::Chime, 0.5),
        }
    }
    let speed = sounds.speed.max(0.0);
    let level = ((speed - ROLL_MIN_SPEED) / (ROLL_FULL_SPEED - ROLL_MIN_SPEED)).clamp(0.0, 1.0);
    let level = level.powf(0.6);
    let pitch = 0.85 + 0.35 * (speed / 10.0).min(1.0);
    for kind in Loop::ALL {
        audio.set(kind, 0.0, pitch);
    }
    let position = sounds.position;
    if sounds.grinding {
        let kind = if hints.near_metal(position) { Loop::GrindMetal } else { Loop::GrindConcrete };
        audio.set(kind, (0.35 + 0.65 * level).min(1.0), 0.9 + 0.2 * (speed / 8.0).min(1.0));
    } else if sounds.grounded {
        let kind = if hints.on_wood(position) { Loop::RollWood } else { Loop::RollSmooth };
        if sounds.powersliding {
            audio.set(kind, level * 0.5, pitch);
            audio.set(Loop::Powerslide, (0.4 + 0.6 * level).min(1.0), 0.9 + 0.2 * level);
        } else {
            audio.set(kind, level, pitch);
        }
    }
}

/// Keep the looping emitters on the skater.
fn follow_skater(audio: Option<Res<CleanAudio>>, mut transforms: Query<&mut Transform>) {
    let Some(audio) = audio.filter(|a| a.spatial) else { return };
    for (_, entity, _) in &audio.loops {
        if let Ok(mut transform) = transforms.get_mut(*entity) {
            if transform.translation != audio.source {
                transform.translation = audio.source;
            }
        }
    }
}

/// Mono or stereo 16-bit PCM from a canonical RIFF/WAVE file.
fn parse_wav(bytes: &[u8]) -> Option<(Vec<i16>, u16, u32)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut at, mut format) = (12, None);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..(at + 8 + size).min(bytes.len()))?;
        if id == b"fmt " && body.len() >= 16 {
            let tag = u16::from_le_bytes([body[0], body[1]]);
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag != 1 || bits != 16 || channels == 0 {
                return None;
            }
            format = Some((channels, rate));
        } else if id == b"data" {
            let (channels, rate) = format?;
            let samples = body
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]))
                .collect();
            return Some((samples, channels, rate));
        }
        at += 8 + size + (size & 1);
    }
    None
}

fn setup(
    mut commands: Commands,
    mut assets: ResMut<Assets<StreamPcm>>,
    spatial: Option<Res<SpatialSkateAudio>>,
) {
    let spatial = spatial.is_some();
    let mut load = |name: &str, looping: bool| -> Option<Handle<StreamPcm>> {
        let (_, bytes) = WAVS.iter().find(|(n, _)| *n == name)?;
        let Some((samples, channels, rate)) = parse_wav(bytes) else {
            error!("SKATE_CLEAN_AUDIO {name}.wav is not 16-bit PCM");
            return None;
        };
        Some(assets.add(StreamPcm::from_i16(Arc::new(samples), channels, rate, looping)))
    };
    let mut loops = Vec::new();
    for kind in Loop::ALL {
        if let Some(handle) = load(kind.file(), true) {
            // Inline-looping source played once: it never ends, and Bevy's whole-source loop
            // (a buffering repeater) is not needed.
            let entity = commands
                .spawn((
                    AudioPlayer(handle),
                    PlaybackSettings::ONCE
                        .with_volume(Volume::Linear(0.0))
                        .with_spatial(spatial),
                    Transform::default(),
                ))
                .id();
            loops.push((kind, entity, 0.0));
        }
    }
    let mut shots = Vec::new();
    let mut count = loops.len();
    for shot in Shot::ALL {
        let prefix = shot.prefix();
        let variants: Vec<_> = WAVS
            .iter()
            .filter(|(n, _)| {
                *n == prefix
                    || n.strip_prefix(prefix)
                        .and_then(|rest| rest.strip_prefix('_'))
                        .is_some_and(|i| i.chars().all(|c| c.is_ascii_digit()))
            })
            .filter_map(|(n, _)| load(n, false))
            .collect();
        count += variants.len();
        shots.push((shot, variants));
    }
    info!("SKATE_CLEAN_AUDIO loaded {count} sounds (spatial={spatial})");
    commands.insert_resource(CleanAudio {
        spatial,
        source: Vec3::ZERO,
        shots,
        loops,
        targets: [(0.0, 1.0); 7],
        last_pick: [usize::MAX; 8],
        rng: 0x9E37_79B9,
        previous: None,
        pending_catch: None,
        last_land: -1.0,
        last_wipe: -1.0,
        paused: false,
        seen_surfaces: 0,
    });
}

impl CleanAudio {
    fn random(&mut self) -> f32 {
        // xorshift32: plenty for picking variants.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn play(&mut self, commands: &mut Commands, shot: Shot, gain: f32) {
        let slot = Shot::ALL.iter().position(|&s| s == shot).unwrap_or(0);
        let Some(count) = self
            .shots
            .iter()
            .find(|(s, _)| *s == shot)
            .map(|(_, v)| v.len())
            .filter(|&n| n > 0)
        else {
            return;
        };
        // A random variant, never the same one twice in a row.
        let mut pick = (self.random() * count as f32) as usize % count;
        if count > 1 && pick == self.last_pick[slot] {
            pick = (pick + 1) % count;
        }
        self.last_pick[slot] = pick;
        let pitch = 0.96 + 0.08 * self.random();
        let handle = self
            .shots
            .iter()
            .find(|(s, _)| *s == shot)
            .map(|(_, v)| v[pick].clone())
            .expect("checked above");
        commands.spawn((
            AudioPlayer(handle),
            PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear((gain * MASTER).clamp(0.0, 1.5)))
                .with_speed(pitch)
                .with_spatial(self.spatial),
            Transform::from_translation(self.source),
        ));
        debug!("SKATE_CLEAN_AUDIO {} gain={gain:.2}", shot.prefix());
    }

    fn set(&mut self, kind: Loop, volume: f32, pitch: f32) {
        let i = Loop::ALL.iter().position(|&k| k == kind).unwrap_or(0);
        self.targets[i] = (volume, pitch);
    }
}

fn is_ground(state: u32) -> bool {
    (100..=105).contains(&state)
}

fn is_air(state: u32) -> bool {
    (200..=202).contains(&state)
}

fn is_grind(state: u32) -> bool {
    (400..=405).contains(&state)
}

fn is_flip(trick: &str) -> bool {
    let t = trick.to_ascii_lowercase();
    ["flip", "heel", "shuv", "varial", "tre", "360", "impossible", "casper", "dolphin"]
        .iter()
        .any(|k| t.contains(k))
}

fn observe(
    mut commands: Commands,
    audio: Option<ResMut<CleanAudio>>,
    hints: Res<SurfaceHints>,
    mut observations: MessageReader<PlayerAudioObservation>,
    virtual_time: Res<Time<bevy::time::Virtual>>,
    real_time: Res<Time<bevy::time::Real>>,
) {
    let Some(mut audio) = audio else { return };
    let now = real_time.elapsed_secs();
    audio.paused = virtual_time.is_paused();
    if let Some(at) = audio.pending_catch {
        if now >= at {
            audio.pending_catch = None;
            audio.play(&mut commands, Shot::Catch, 0.8);
        }
    }
    let mut latest = None;
    for obs in observations.read() {
        let prev = audio.previous.clone().unwrap_or(Edges {
            state: obs.state,
            ..Default::default()
        });
        audio.source = Vec3::from_array(obs.retail.position);
        let wiped = obs.wiping_out || obs.events.iter().any(|e| matches!(e, PhysicsEvent::Wipeout));
        // Pop: leaving the ground or a grind for the air, by the skater's own doing.
        if is_air(obs.state) && (is_ground(prev.state) || is_grind(prev.state)) && !wiped {
            audio.play(&mut commands, Shot::Pop, 0.9);
        }
        // Landing: once per touchdown, louder the harder it hits.
        let landed = obs.landed || obs.events.iter().any(|e| matches!(e, PhysicsEvent::Landing));
        if landed && now - audio.last_land > 0.15 {
            audio.last_land = now;
            let gain = (0.45 + obs.landing_impact_speed.abs() / 6.0).clamp(0.45, 1.0);
            audio.play(&mut commands, Shot::Land, gain);
        }
        // Flip tricks: the board's whoosh now, the catch a moment later.
        if obs.trick_identifier != prev.trick && is_air(obs.state) {
            if let Some(trick) = obs.trick_identifier.as_deref().filter(|t| is_flip(t)) {
                debug!("SKATE_CLEAN_AUDIO trick {trick}");
                audio.play(&mut commands, Shot::Flip, 0.7);
                audio.pending_catch = Some(now + CATCH_DELAY_S);
            }
        }
        if !is_air(obs.state) {
            audio.pending_catch = None;
        }
        let pushing = obs.foot_push_speed.abs() > 0.35;
        if pushing && !prev.pushing {
            audio.play(&mut commands, Shot::Push, 0.75);
        }
        let stepping = obs.footstep_strength > 0.1 && (500..=503).contains(&obs.state);
        if stepping && !prev.stepping {
            audio.play(&mut commands, Shot::Footstep, 0.4 + 0.4 * obs.footstep_strength.min(1.0));
        }
        if wiped && !prev.wiping_out && now - audio.last_wipe > 0.8 {
            audio.last_wipe = now;
            audio.play(&mut commands, Shot::Bail, 1.0);
        }
        audio.previous = Some(Edges {
            state: obs.state,
            wiping_out: wiped,
            pushing,
            stepping,
            trick: obs.trick_identifier.clone(),
        });
        latest = Some(obs);
    }
    let Some(obs) = latest else { return };

    // Loops follow the latest observation.
    let speed = obs.board_speed.max(0.0);
    let level = ((speed - ROLL_MIN_SPEED) / (ROLL_FULL_SPEED - ROLL_MIN_SPEED)).clamp(0.0, 1.0);
    let level = level.powf(0.6);
    let pitch = 0.85 + 0.35 * (speed / 10.0).min(1.0);
    let position = Vec3::from_array(obs.retail.position);
    for kind in Loop::ALL {
        audio.set(kind, 0.0, pitch);
    }
    if obs.grinding || is_grind(obs.state) {
        let metal = hints.near_metal(position);
        let kind = if metal { Loop::GrindMetal } else { Loop::GrindConcrete };
        let grind_level = (0.35 + 0.65 * level).min(1.0);
        audio.set(kind, grind_level, 0.9 + 0.2 * (speed / 8.0).min(1.0));
    } else if obs.grounded && is_ground(obs.state) && !obs.wiping_out {
        // All AR real surfaces share one physics material, so the wheel material says nothing
        // there: placed ramps are wood, everything else rolls like smooth concrete. The rough and
        // metal loops are loaded for maps whose materials get mapped later.
        let kind = if hints.on_wood(position) { Loop::RollWood } else { Loop::RollSmooth };
        let surface = obs.wheel_surface & 0x7F;
        if audio.seen_surfaces & (1u128 << surface) == 0 {
            audio.seen_surfaces |= 1u128 << surface;
            info!("SKATE_CLEAN_AUDIO wheel audio material {surface} -> {kind:?}");
        }
        if obs.powersliding {
            audio.set(kind, level * 0.5, pitch);
            audio.set(Loop::Powerslide, (0.4 + 0.6 * level).min(1.0), 0.9 + 0.2 * level);
        } else {
            audio.set(kind, level, pitch);
        }
    }
}

fn frontend(
    mut commands: Commands,
    audio: Option<ResMut<CleanAudio>>,
    mut requests: MessageReader<FrontEndSoundRequest>,
) {
    let Some(mut audio) = audio else { return };
    if requests.read().count() > 0 {
        audio.play(&mut commands, Shot::Chime, 0.5);
    }
}

fn mix(
    audio: Option<ResMut<CleanAudio>>,
    mut sinks: Query<&mut AudioSink>,
    mut spatial_sinks: Query<&mut SpatialAudioSink>,
    time: Res<Time<bevy::time::Real>>,
) {
    let Some(mut audio) = audio else { return };
    let blend = (time.delta_secs() / FADE_S).min(1.0);
    let paused = audio.paused;
    let targets = audio.targets;
    for (i, (_, entity, volume)) in audio.loops.iter_mut().enumerate() {
        let (target, pitch) = targets[i];
        let target = if paused { 0.0 } else { target };
        *volume += (target - *volume) * blend;
        let level = Volume::Linear(*volume * LOOP_GAIN * MASTER);
        let speed = pitch.clamp(0.5, 2.0);
        if let Ok(mut sink) = sinks.get_mut(*entity) {
            sink.set_volume(level);
            sink.set_speed(speed);
        } else if let Ok(mut sink) = spatial_sinks.get_mut(*entity) {
            sink.set_volume(level);
            sink.set_speed(speed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_clean_sound_parses() {
        for (name, bytes) in WAVS {
            let (samples, channels, rate) = parse_wav(bytes).unwrap_or_else(|| panic!("{name}"));
            assert_eq!((channels, rate), (1, 32_000), "{name}");
            assert!(samples.len() > 1000, "{name}");
        }
    }

    #[test]
    fn hints_find_rails_and_ramps() {
        let hints = SurfaceHints {
            metal_rails: vec![(Vec3::ZERO, Vec3::X * 2.0)],
            wood_boxes: vec![(Vec3::new(5.0, 0.0, 0.0), Vec3::new(6.0, 0.5, 1.0))],
        };
        assert!(hints.near_metal(Vec3::new(1.0, 0.3, 0.0)));
        assert!(!hints.near_metal(Vec3::new(1.0, 2.0, 0.0)));
        assert!(hints.on_wood(Vec3::new(5.5, 1.0, 0.5)));
        assert!(!hints.on_wood(Vec3::new(3.0, 0.5, 0.5)));
    }

    #[test]
    fn flips_are_recognised() {
        assert!(is_flip("Kickflip"));
        assert!(is_flip("varial_heelflip"));
        assert!(!is_flip("Ollie"));
    }
}
