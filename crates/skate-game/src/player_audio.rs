//! Single-owner host for the recovered rider/board sound graph.
//!
//! Asset IO and XMA decoding happen once on this worker before the runtime is published. The
//! Bevy mixer sees one permanent stereo source; neither the mixer callback nor gameplay touches
//! guest memory.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
};
use std::thread;
use std::time::{Duration, Instant};

use bevy::audio::{AudioPlayer, PlaybackSettings, Volume};
use bevy::prelude::*;
use skate_audio_core::authored::{AuthoredRuntime, PCM_CHANNELS, PCM_FRAMES_PER_BLOCK};
use skate_audio_core::pcm::PcmSource;
use skate_data::audio::catalog::PlayerAudioCatalog;

use super::{LivePcm, PlayerAudioObservation};

#[path = "player_audio/audio_state.rs"]
mod audio_state;
#[path = "player_audio/collision_materials.rs"]
mod collision_materials;
#[path = "player_audio/collision_states.rs"]
mod collision_states;
#[path = "player_audio/components/mod.rs"]
mod components;
#[path = "player_audio/contact_voices.rs"]
mod contact_voices;
#[path = "player_audio/frontend.rs"]
mod frontend;
#[cfg(test)]
#[path = "player_audio/headless.rs"]
mod headless;
#[path = "player_audio/sound.rs"]
mod sound;
#[path = "player_audio/trace.rs"]
mod trace;
#[path = "player_audio/tuning.rs"]
mod tuning;

const OUTPUT_CHANNELS: u16 = 2;
const SAMPLE_RATE: u32 = 48_000;
const QUEUE_FRAMES: usize = 4096;
const OBSERVATION_QUEUE: usize = 16;
// The host edge is headroom, not balance — and it is now measured rather than guessed.
//
// With the recomp metered at its output pass (`OUT` lines) and its music off, retail's own player
// mix over a session of ollies sits at rolling peak p50 -22.9 dBFS with landings reaching +0.4
// dBFS: retail's native mix runs *past* unity on an impact and relies on a master/Dac stage this
// port does not have. The same session in this engine measures rolling -22.7 dBFS and, with the
// contact trim set from the same capture, landings at retail's level. So the levels themselves
// already match retail and must not be scaled again; what is left to supply is the headroom.
//
// `downmix` sums front + 0.707 centre + 0.5 surround, measured at +2.6 dB over the native peak on
// an impact, which would put a retail-level landing near +3 dBFS and into the clamp — the clamp is
// what made landings read as "gunshots". 0.6 (-4.4 dB) keeps a retail-level landing just under
// full scale while preserving retail's 23 dB of impact-over-bed dynamics.
//
// This edge still stands in for retail's master/Dac stage (`docs/player-audio-retail-drivers.md`
// section 7): it models that stage's headroom, not the console's own downmix or master level, so
// absolute loudness at the speakers may still differ from a retail console's.
const OUTPUT_GAIN: f32 = 0.6;

#[derive(Resource)]
struct PlayerAudioHost {
    input: SyncSender<PlayerAudioObservation>,
    /// The last forwarded observation, repeated while paused (see [`forward`]).
    last_observation: Option<PlayerAudioObservation>,
    paused_carry: f32,
    status: Mutex<Receiver<WorkerStatus>>,
    live: LivePcm,
    last_report: Instant,
    dropped_observations: u64,
    /// Front-end one-shot ids waiting for the next observation to carry them to the worker.
    pending_frontend: Vec<u64>,
}

#[derive(Resource)]
pub(crate) struct PlayerAudioAssets(pub PathBuf);

enum WorkerStatus {
    Ready { samples: usize, cache_hits: usize },
    Failed(String),
}

pub(super) fn install(app: &mut App) {
    // The session marker registers this too, but its plugin returns early when its validation
    // data will not load, and the multiplier sounds must not depend on that.
    app.add_message::<crate::skate_audio::FrontEndSoundRequest>()
        .add_systems(Startup, start)
        .add_systems(Update, (forward, report));
}

/// 82666BC0's sound for a sequence multiplier that has just changed, or `None` for the
/// multipliers retail is silent for. The caller owns the change detection, as the HUD module
/// does; this module is shared by binaries that have no simulation to poll.
pub(crate) fn multiplier_sound_id(multiplier: f32) -> Option<u64> {
    frontend::FrontEndSound::for_multiplier(multiplier).map(frontend::FrontEndSound::id)
}

fn start(
    mut commands: Commands,
    mut assets: ResMut<Assets<LivePcm>>,
    configured: Option<Res<PlayerAudioAssets>>,
) {
    let Some(configured) = configured else { return };
    let live = match LivePcm::new(OUTPUT_CHANNELS, SAMPLE_RATE, QUEUE_FRAMES) {
        Ok(live) => live,
        Err(error) => {
            error!("skate-audio player source: {error}");
            return;
        }
    };
    let handle = assets.add(live.clone());
    commands.spawn((
        AudioPlayer(handle),
        // LivePcm never ends. Loop mode would wrap it in Rodio's buffering repeater, which
        // eagerly captures startup silence and retains every streamed sample.
        PlaybackSettings::ONCE.with_volume(Volume::Linear(1.0)),
    ));

    let (input, observations) = mpsc::sync_channel(OBSERVATION_QUEUE);
    let (status_tx, status) = mpsc::channel();
    let assets = configured.0.clone();
    let worker_live = live.clone();
    thread::Builder::new()
        .name("skate-player-audio".into())
        .spawn(move || {
            if let Err(error) = run(assets, worker_live, observations, &status_tx) {
                eprintln!("SKATE_PLAYER_AUDIO failed={error}");
                let _ = status_tx.send(WorkerStatus::Failed(error));
            }
        })
        .map_err(|error| error!("skate-audio could not start player worker: {error}"))
        .ok();
    commands.insert_resource(PlayerAudioHost {
        input,
        last_observation: None,
        paused_carry: 0.0,
        status: Mutex::new(status),
        live,
        last_report: Instant::now(),
        dropped_observations: 0,
        pending_frontend: Vec::new(),
    });
}

fn forward(
    mut host: Option<ResMut<PlayerAudioHost>>,
    mut observations: MessageReader<PlayerAudioObservation>,
    mut marker_sounds: MessageReader<crate::skate_audio::FrontEndSoundRequest>,
    virtual_time: Res<Time<bevy::time::Virtual>>,
    real_time: Res<Time<bevy::time::Real>>,
) {
    let Some(host) = host.as_deref_mut() else {
        return;
    };
    // The session marker fires on its own schedule, not with the simulation, so its sounds are
    // buffered and ride out on the next observation rather than being dropped if this frame
    // produced none. Retail calls `GlobalFEPlaySound` straight from the UI; this is the closest
    // equivalent that still reaches the one audio worker.
    host.pending_frontend
        .extend(marker_sounds.read().map(|sound| sound.0));
    // Retail keeps publishing an audio state every frame while the game is paused — its physics
    // values simply stay frozen — and silences the player path through SFXObj_Pause's input. The
    // simulation schedule does not run while `Time<Virtual>` is paused, so repeat the last
    // observation at the simulation rate with `paused` set, which is what retail's frozen state is.
    if virtual_time.is_paused() {
        if let Some(last) = host.last_observation.clone() {
            host.paused_carry += real_time.delta_secs();
            while host.paused_carry >= sound::FRAME_SECONDS {
                host.paused_carry -= sound::FRAME_SECONDS;
                let mut frozen = last.clone();
                frozen.retail.paused = true;
                let _ = host.input.try_send(frozen);
            }
        }
        return;
    }
    host.paused_carry = 0.0;
    for observation in observations.read() {
        host.last_observation = Some(observation.clone());
        let mut observation = observation.clone();
        observation
            .frontend_sounds
            .append(&mut std::mem::take(&mut host.pending_frontend));
        match host.input.try_send(observation) {
            Ok(()) => {}
            Err(TrySendError::Full(lost)) => {
                host.dropped_observations += 1;
                eprintln!(
                    "SKATE_PLAYER_AUDIO observation_overflow tick={} footstep={} push={} landed={} events={:?}",
                    lost.tick,
                    lost.footstep_strength,
                    lost.foot_push_speed,
                    lost.landed,
                    lost.events
                );
            }
            Err(TrySendError::Disconnected(_)) => break,
        }
    }
}

fn report(mut host: Option<ResMut<PlayerAudioHost>>) {
    let Some(host) = host.as_deref_mut() else {
        return;
    };
    let statuses: Vec<_> = host
        .status
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .try_iter()
        .collect();
    for status in statuses {
        match status {
            WorkerStatus::Ready {
                samples,
                cache_hits,
            } => {
                info!("skate-audio player graph ready: {samples} samples ({cache_hits} from cache)")
            }
            WorkerStatus::Failed(error) => error!("skate-audio player graph stopped: {error}"),
        }
    }
    if host.dropped_observations >= 60 {
        warn!(
            "skate-audio dropped {} stale physics observations",
            host.dropped_observations
        );
        host.dropped_observations = 0;
    }
    if host.last_report.elapsed() >= Duration::from_secs(2) {
        let stats = host.live.snapshot();
        eprintln!(
            "SKATE_PLAYER_AUDIO stream pushed={} consumed={} nonzero={} underflow={} queued_frames={}",
            stats.pushed,
            stats.consumed,
            stats.consumed_nonzero,
            stats.underflow,
            host.live.queued_frames(),
        );
        host.last_report = Instant::now();
    }
}

fn cache_directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("Skate3RustEngine/audio-pcm-cache"))
}

/// Everything the worker builds before its first block: the authored runtime with its banks and
/// PCM, the boot utilities, the MixMap and every player-sound component.
pub(crate) struct Prepared {
    pub runtime: AuthoredRuntime,
    pub sound: sound::PlayerSound,
    pub samples: usize,
    pub cache_hits: usize,
}

pub(crate) fn prepare(assets: &std::path::Path) -> Result<Prepared, String> {
    let mut catalog = PlayerAudioCatalog::from_assets(assets, cache_directory().as_deref())
        .map_err(|error| error.to_string())?;
    // The Splice collision banks (`Skate_Collisions.bnk`) hold the wheel pops and the landing
    // impact one-shots; they install exactly like the patch banks' samples.
    catalog
        .load_splice_banks(
            &assets.join("private/stock/data/audio/audiofiles.big"),
            cache_directory().as_deref(),
        )
        .map_err(|error| error.to_string())?;
    let sample_count = catalog.samples.len();
    let cache_hits = catalog.cache_hits;
    let mixmap = catalog
        .mixmap
        .clone()
        .ok_or("MixMapSK8.mxb is not staged in the assets (rerun setup)")?;
    let mut runtime = AuthoredRuntime::new(catalog.guest, catalog.projects, catalog.banks)
        .map_err(|error| error.to_string())?;
    for sample in catalog.samples {
        let base = runtime
            .bank_base(&sample.bank)
            .ok_or_else(|| format!("installed bank {} disappeared", sample.bank))?;
        runtime
            .insert_pcm(base + sample.header_offset, sample.pcm)
            .map_err(|error| error.to_string())?;
    }
    // Retail's audio boot creates these three persistent utilities before any rider/board
    // message. They all use the same eight packet-local control-block references; their order
    // is observable in the retail boot trace and establishes the broadcaster bindings consumed
    // later by rolling-surface programs.
    let utility_relocations = [
        (3, 0x1c),
        (7, 0x2c),
        (11, 0x3c),
        (15, 0x4c),
        (19, 0x5c),
        (23, 0x6c),
        (27, 0x7c),
    ];
    let utility_packet = [0u32; 28];
    let _emitter_utility = if runtime.has_object("c_emitter_utility") {
        Some(
            runtime
                .post_relocated("c_emitter_utility", &utility_packet, &utility_relocations)
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let _startup_play = if runtime.has_object("Start_up_Play_ctl") {
        Some(
            runtime
                .post_relocated("Start_up_Play_ctl", &utility_packet, &utility_relocations)
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    // `c_foley_utility` owns the foley-side subscriptions. Its eight
    // packet-local control blocks own the table-1 subscriptions used by dependent foley
    // programs; posting a rolling input without it lets the selector open then immediately
    // tear down its voices. Preserve every self-relative block exactly from the boot trace.
    let _foley_utility = if runtime.has_object("c_foley_utility") {
        let handle = runtime
            .post_relocated("c_foley_utility", &utility_packet, &utility_relocations)
            .map_err(|error| error.to_string())?;
        eprintln!("SKATE_PLAYER_AUDIO foley_utility_start handle={handle:#010x}");
        Some(handle)
    } else {
        None
    };
    runtime
        .load_mixmap(&mixmap)
        .map_err(|error| error.to_string())?;
    let sound = sound::build(&mut runtime, assets, cache_directory().as_deref())?;
    eprintln!("SKATE_PLAYER_AUDIO player_sound ready");
    Ok(Prepared {
        runtime,
        sound,
        samples: sample_count,
        cache_hits,
    })
}

fn run(
    assets: PathBuf,
    live: LivePcm,
    observations: Receiver<PlayerAudioObservation>,
    status: &mpsc::Sender<WorkerStatus>,
) -> Result<(), String> {
    let Prepared {
        mut runtime,
        mut sound,
        samples: sample_count,
        cache_hits,
    } = prepare(&assets)?;
    let _ = status.send(WorkerStatus::Ready {
        samples: sample_count,
        cache_hits,
    });
    eprintln!("SKATE_PLAYER_AUDIO ready samples={sample_count} cache_hits={cache_hits}");

    let block = Duration::from_secs_f64(f64::from(PCM_FRAMES_PER_BLOCK) / f64::from(SAMPLE_RATE));
    let mut deadline = Instant::now();
    let trace = std::env::var_os("SKATE_AUDIO_OBSERVE").is_some();
    // Diagnostics: one line per second with the rendered level and voice counts, plus the audio
    // state that drives the board, so a playtest log shows what the retail components saw.
    let mut window_peak = 0.0f32;
    let mut window_blocks = 0u32;
    loop {
        // Every observation may contain a one-tick contact or landing. Consume FIFO, allowing
        // the evaluator to run between observations; draining to the newest packet loses edges
        // and can overwrite a post's controls before its first audio block. At 187.5 blocks/s
        // this catches up with the normal 60 Hz simulation without discarding intermediate ticks.
        if let Some(observation) = next_observation(&observations)? {
            sound.frame(&mut runtime, &observation)?;
        }
        let native = runtime.pump_once().map_err(|error| error.to_string())?;
        if native.len() != PCM_FRAMES_PER_BLOCK as usize * usize::from(PCM_CHANNELS) {
            return Err(format!(
                "authored graph returned {} samples for one block",
                native.len()
            ));
        }
        // Player audio reaches the host only through the recovered retail message and graph
        // path.  Do not substitute decoded-bank one-shots here: that bypasses the authored
        // selector, pitch, envelope, filter, layer, and bus controls and therefore cannot be
        // considered a Skate 3 match.
        let stereo = downmix(&native);
        trace::output(&native, usize::from(PCM_CHANNELS), &stereo);
        window_peak = native
            .iter()
            .fold(window_peak, |peak, sample| peak.max(sample.abs()));
        window_blocks += 1;
        if trace && window_blocks as u64 * u64::from(PCM_FRAMES_PER_BLOCK) >= u64::from(SAMPLE_RATE)
        {
            let stats = runtime.stats();
            // The graph arena: blocks live must plateau. It climbing without bound was the
            // voice-graph leak that ended a long session in "guest heap exhausted".
            let arena = runtime.arena_usage();
            let audio = sound.audio();
            eprintln!(
                "SKATE_PLAYER_AUDIO second block={} peak_dbfs={:.1} opened={} live={} arena={}k/{} speed={:.2} wheels={} air={} grind={} walk={}",
                stats.blocks,
                20.0 * window_peak.max(1.0e-9).log10(),
                stats.voices_opened,
                stats.live_voices,
                arena.0 / 1024,
                arena.1,
                audio.ground_speed_208,
                audio.wheel_count_200,
                audio.in_known_air_332,
                audio.grinding_341,
                audio.walking_716,
            );
            window_peak = 0.0;
            window_blocks = 0;
        }
        live.push(&stereo)?;
        deadline += block;
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline - now);
        } else if now.duration_since(deadline) > block * 4 {
            deadline = now;
        }
    }
}

fn next_observation<T>(observations: &Receiver<T>) -> Result<Option<T>, String> {
    match observations.try_recv() {
        Ok(observation) => Ok(Some(observation)),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => {
            Err("player audio observation source disconnected".into())
        }
    }
}

#[derive(Clone)]
struct Clip {
    index: usize,
    source: PcmSource,
    sample_rate: u32,
}

struct DirectVoice {
    tag: Option<&'static str>,
    clip: Clip,
    frame: f64,
    gain: f32,
    target_gain: f32,
    repeat: bool,
    releasing: bool,
}

/// Mixes decoded retail samples for gameplay families whose patch payloads are not recovered yet.
/// The authored runtime remains authoritative for grinds; this host fallback makes the rest of the
/// rider and board audible from the correct shipped banks instead of substituting synthetic audio.
struct DirectMixer {
    banks: HashMap<String, Vec<Clip>>,
    voices: Vec<DirectVoice>,
    serial: usize,
    last_airborne: bool,
    last_wipeout: bool,
    last_speed: f32,
    last_footstep: f32,
    last_push: f32,
    next_footstep_tick: u64,
    next_push_tick: u64,
    next_skid_tick: u64,
    next_squeak_tick: u64,
    landing_skid_suppressed_until_tick: u64,
    offboard_step_distance: f32,
    offboard_step_side: usize,
}

impl DirectMixer {
    fn new(samples: &[skate_data::audio::catalog::BankPcm]) -> Self {
        let mut banks: HashMap<String, Vec<Clip>> = HashMap::new();
        for sample in samples {
            banks.entry(sample.bank.clone()).or_default().push(Clip {
                index: sample.index,
                source: sample.pcm.source.clone(),
                sample_rate: sample.pcm.sample_rate,
            });
        }
        Self {
            banks,
            voices: Vec::new(),
            serial: 0,
            last_airborne: false,
            last_wipeout: false,
            last_speed: 0.0,
            last_footstep: 0.0,
            last_push: 0.0,
            next_footstep_tick: 0,
            next_push_tick: 0,
            next_skid_tick: 0,
            next_squeak_tick: 0,
            landing_skid_suppressed_until_tick: 0,
            offboard_step_distance: 0.0,
            offboard_step_side: 0,
        }
    }

    /// Retail `Class_Flips` and `cloth_trick` divide the clip banks by trick family. The exact
    /// variants below are the groups opened by the recorded ollie, flip and shuv payloads; the
    /// counter still varies the member within each authored family.
    fn trick_groups(identifier: Option<&str>) -> (&'static [usize], &'static [usize]) {
        let name = identifier.unwrap_or_default().to_ascii_lowercase();
        if name.contains("shuv") {
            (&[6, 16, 17], &[24, 25, 26, 27])
        } else if name.contains("kickflip")
            || name.contains("heelflip")
            || name.contains("hardflip")
            || name.contains("laserflip")
            || name.contains("360flip")
            || name.contains("varial")
        {
            (&[5, 9, 18, 21], &[15, 16, 17, 18, 19])
        } else {
            (&[10, 11, 12, 13], &[8, 9, 10, 11, 12, 13, 14])
        }
    }

    fn choose(&mut self, bank: &str, salt: usize) -> Option<Clip> {
        let clips = self.banks.get(bank)?;
        let clip = clips
            .get((self.serial.wrapping_add(salt)) % clips.len())?
            .clone();
        self.serial = self.serial.wrapping_add(1);
        Some(clip)
    }

    fn choose_from(&mut self, bank: &str, indices: &[usize], salt: usize) -> Option<Clip> {
        let clips = self.banks.get(bank)?;
        let start = self.serial.wrapping_add(salt);
        let wanted = indices.get(start % indices.len())?;
        let clip = clips.iter().find(|clip| clip.index == *wanted)?.clone();
        self.serial = self.serial.wrapping_add(1);
        Some(clip)
    }

    fn play(&mut self, bank: &str, salt: usize, gain: f32) {
        if let Some(clip) = self.choose(bank, salt) {
            self.voices.push(DirectVoice {
                tag: None,
                clip,
                frame: 0.0,
                gain,
                target_gain: gain,
                repeat: false,
                releasing: false,
            });
        }
    }

    fn play_from(&mut self, bank: &str, indices: &[usize], salt: usize, gain: f32) {
        if let Some(clip) = self.choose_from(bank, indices, salt) {
            self.voices.push(DirectVoice {
                tag: None,
                clip,
                frame: 0.0,
                gain,
                target_gain: gain,
                repeat: false,
                releasing: false,
            });
        }
    }

    fn set_loop(&mut self, tag: &'static str, bank: &str, salt: usize, gain: f32, enabled: bool) {
        if !enabled {
            if let Some(voice) = self.voices.iter_mut().find(|voice| voice.tag == Some(tag)) {
                voice.target_gain = 0.0;
                voice.releasing = true;
            }
            return;
        }
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.tag == Some(tag)) {
            voice.target_gain = gain;
            voice.releasing = false;
            return;
        }
        if let Some(clip) = self.choose(bank, salt) {
            self.voices.push(DirectVoice {
                tag: Some(tag),
                clip,
                frame: 0.0,
                gain: 0.0,
                target_gain: gain,
                repeat: true,
                releasing: false,
            });
        }
    }

    fn set_loop_from(
        &mut self,
        tag: &'static str,
        bank: &str,
        indices: &[usize],
        salt: usize,
        gain: f32,
        enabled: bool,
    ) {
        if !enabled {
            if let Some(voice) = self.voices.iter_mut().find(|voice| voice.tag == Some(tag)) {
                voice.target_gain = 0.0;
                voice.releasing = true;
            }
            return;
        }
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.tag == Some(tag)) {
            voice.target_gain = gain;
            voice.releasing = false;
            return;
        }
        if let Some(clip) = self.choose_from(bank, indices, salt) {
            self.voices.push(DirectVoice {
                tag: Some(tag),
                clip,
                frame: 0.0,
                gain: 0.0,
                target_gain: gain,
                repeat: true,
                releasing: false,
            });
        }
    }

    fn observe(&mut self, observation: &PlayerAudioObservation) {
        let speed = observation.board_speed.max(observation.rider_speed);
        let airborne = matches!(observation.state, 103 | 200 | 201 | 202);
        if observation.landed {
            // A vertical landing commonly sheds enough reported horizontal speed to satisfy the
            // fallback deceleration heuristic. Retail does not turn that impact into a wheel-skid
            // post: Class_Treatment owns the landing, while Class_wheels_skid is reserved for
            // actual stops/reverts/slip. Cover the settling frames as well as the landing tick.
            self.landing_skid_suppressed_until_tick = observation.tick.saturating_add(12);
        }
        let rolling = observation.grounded
            && !observation.grinding
            && !observation.wiping_out
            && speed > 0.35;
        let surface = observation.wheel_surface as usize;
        let foot_surface = if observation.state / 100 == 5 {
            observation.foot_surface as usize
        } else {
            surface
        };
        let footstep_slots = material_slots::<8>(foot_surface);
        let foot_drag_slots = material_slots::<7>(surface);
        let wheel_skid_slots = material_slots::<4>(surface);
        if observation.landed {
            // `Class_Treatment` is a held retail message: its patch picks the impact sample
            // from the ten following re-deliveries. The authored treatment graph is posted and
            // updated above, but its voice path is not yet reliably audible on the host mixer.
            // Keep the authored path alive and supply the matching shipped-bank one-shot until
            // that path can replace this fallback. This must be keyed to the physics landing
            // edge, never to speed loss (which would turn ordinary braking into a landing).
            let strength = landing_treatment_strength(
                observation.landing_impact_speed,
                observation.landing_clean,
                observation.landing_sketchy,
                observation.landing_type,
            );
            let gain = treatment_fallback_gain(strength);
            // The recorded ordinary ollie landing opens Treatments slot 17.  Do not turn every
            // impact into that slot: the heavier/sketchy branches have not yet been exercised
            // through the retail selector, so retain the shipped bank's variation for those.
            let ordinary_retail_landing = (600..=750).contains(&strength)
                && !observation.landing_sketchy
                && observation.landing_type == 0;
            if ordinary_retail_landing {
                self.play_from("Treatments.abk", &[17], observation.tick as usize, gain);
            } else {
                self.play(
                    "Treatments.abk",
                    observation.tick as usize + observation.landing_type as usize,
                    gain,
                );
            }
            eprintln!(
                "SKATE_PLAYER_AUDIO landing_fallback tick={} impact={:.3} strength={} clean={} sketchy={} type={} source={}",
                observation.tick,
                observation.landing_impact_speed,
                strength,
                observation.landing_clean,
                observation.landing_sketchy,
                observation.landing_type,
                if ordinary_retail_landing {
                    "treatments:17"
                } else {
                    "treatments:severity-fallback"
                },
            );
        }
        let roll_gain = (0.015 + speed * 0.0035).min(0.065);
        self.set_loop_from(
            "rolling",
            "PatchBank_Rolling_Surfaces.abk",
            // These are the four surface-loop slots actually opened by Class_rolling across the
            // retail captures. One material loop is active at a time; layering two fixed slots was
            // the source of the uniform board hum in the previous build.
            &[11, 12, 14, 15],
            surface,
            roll_gain,
            rolling,
        );
        self.set_loop_from(
            "rattle",
            "Rolling_Rattles.abk",
            // The captured Rolling_Rattle_Class opens its looping speed layers 6/7/8. Audio
            // material is not an input to this family; selecting it from the surface ID made a
            // concrete change sound like a different set of trucks.
            &[6, 7, 8],
            (speed / 2.5) as usize,
            (speed * 0.0045).min(0.05),
            rolling && speed > 1.5,
        );
        self.set_loop_from(
            "wind",
            "sense_of_speed.abk",
            &[0, 1, 2],
            0,
            ((speed - 5.0) * 0.005).clamp(0.0, 0.055),
            speed > 5.0 && !observation.wiping_out,
        );
        self.set_loop_from(
            "speed-rattle",
            "sense_of_speed.abk",
            &[15, 16],
            0,
            ((speed - 7.0) * 0.004).clamp(0.0, 0.04),
            speed > 7.0 && !observation.wiping_out,
        );

        // `c_board_slide` belongs to a loose/deck scrape, not the rail-grind loop. Layering it on
        // every boardslide/tipslide/darkslide doubled the authored Class_grind scrape and made all
        // three families sound alike. The retail traces post it beside bail/ground-slide activity.
        let board_slide = observation.wiping_out && observation.contact_count > 0 && speed > 0.4;
        self.set_loop_from(
            "board-slide",
            "board_scrapes.abk",
            &[10, 11, 12, 13, 14],
            surface,
            0.11,
            board_slide,
        );
        self.set_loop_from(
            "body-slide",
            "Bodyslide.abk",
            &[0, 5, 15],
            surface,
            0.13,
            observation.wiping_out && observation.contact_count > 0 && speed > 0.4,
        );
        self.set_loop_from(
            "foot-drag",
            "FOOT_DRAG.abk",
            &foot_drag_slots,
            surface,
            (observation.foot_push_speed.abs() * 0.03).clamp(0.04, 0.12),
            matches!(observation.state, 101 | 102) && observation.foot_push_speed.abs() > 0.35,
        );

        // The animation signal can remain above zero through an entire planted-foot interval.
        // A rising-edge-only trigger consequently missed later steps when two contacts overlapped.
        // Keep the retail variants, but permit another contact after a short gait cooldown.
        if observation.footstep_strength > 0.05 && observation.tick >= self.next_footstep_tick {
            self.play_from(
                "fstep_skateshoe1_sm.abk",
                &footstep_slots,
                observation.footstep_bone.unsigned_abs() as usize + observation.tick as usize,
                (0.14 + observation.footstep_strength * 0.08).min(0.22),
            );
            self.next_footstep_tick = observation.tick + 18;
            self.offboard_step_distance = 0.0;
        } else if observation.footstep_strength <= 0.05 && observation.state == 500 {
            if advance_offboard_stride(&mut self.offboard_step_distance, observation.rider_speed)
                && observation.tick >= self.next_footstep_tick
            {
                self.play_from(
                    "fstep_skateshoe1_sm.abk",
                    &footstep_slots,
                    observation.tick as usize + self.offboard_step_side,
                    (0.13 + observation.rider_speed * 0.018).min(0.21),
                );
                self.offboard_step_side ^= 1;
                self.next_footstep_tick = observation.tick + 8;
            }
        } else {
            self.offboard_step_distance = 0.0;
        }
        if observation.foot_push_speed.abs() > 0.35 && observation.tick >= self.next_push_tick {
            self.play_from(
                "fstep_skateshoe1_sm.abk",
                &footstep_slots,
                surface + observation.tick as usize,
                0.16,
            );
            self.next_push_tick = observation.tick + 16;
        }
        if !self.last_airborne && airborne && !observation.wiping_out {
            let (_, cloth_clips) = Self::trick_groups(observation.trick_identifier.as_deref());
            let stance_salt = usize::from(observation.riding_switch)
                + usize::from(observation.riding_fakie) * 2
                + usize::from(observation.nollie) * 4;
            self.play_from(
                "Foley_Cloth.abk",
                cloth_clips,
                observation.tick as usize + stance_salt,
                0.10,
            );
            // The bank is 24 authored audio materials by four variants. Selecting from the
            // current material group preserves the retail surface identity at pop time.
            self.play_from(
                "WHEEL_SKID_BANK.abk",
                &wheel_skid_slots,
                observation.tick as usize + stance_salt,
                0.024,
            );
        }
        if observation.wiping_out && !self.last_wipeout {
            // `c_cloth_falls` occupies the fall/impact families left outside the trick selector's
            // ollie (8..14), flip (15..19), and shuv (24..27) groups.
            self.play_from(
                "Foley_Cloth.abk",
                &[0, 1, 2, 3, 4, 5, 6, 7],
                observation.tick as usize,
                0.20,
            );
        }
        let deceleration = self.last_speed - speed;
        if deceleration_skid_allowed(
            rolling,
            observation.powersliding,
            self.last_airborne,
            observation.tick,
            self.landing_skid_suppressed_until_tick,
            self.next_skid_tick,
            deceleration,
        ) {
            self.play_from(
                "WHEEL_SKID_BANK.abk",
                &wheel_skid_slots,
                surface + observation.tick as usize,
                (0.050 + deceleration * 0.016).min(0.10),
            );
            self.next_skid_tick = observation.tick + 8;
        }
        let sliding_or_reverting =
            observation.powersliding || matches!(observation.state, 101 | 102);
        if sliding_or_reverting
            && observation.contact_count > 0
            && speed > 0.5
            && observation.tick >= self.landing_skid_suppressed_until_tick
            && observation.tick >= self.next_squeak_tick
        {
            self.play_from(
                "Brd_Squeaks.abk",
                &[2, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
                surface + observation.tick as usize,
                (0.13 + speed * 0.012).min(0.24),
            );
            self.next_squeak_tick = observation.tick + (13.0 - speed * 0.5).clamp(6.0, 11.0) as u64;
        }
        if matches!(observation.state, 101 | 102)
            && observation.contact_count > 0
            && speed > 0.5
            && observation.tick >= self.landing_skid_suppressed_until_tick
            && observation.tick >= self.next_skid_tick
        {
            self.play_from(
                "WHEEL_SKID_BANK.abk",
                &wheel_skid_slots,
                observation.tick as usize + surface,
                (0.085 + speed * 0.007).min(0.15),
            );
            self.next_skid_tick = observation.tick + 9;
        }

        self.last_airborne = airborne;
        self.last_wipeout = observation.wiping_out;
        self.last_speed = speed;
        self.last_footstep = observation.footstep_strength;
        self.last_push = observation.foot_push_speed;
    }

    fn mix(&mut self, stereo: &mut [f32]) {
        for frame in stereo.chunks_exact_mut(2) {
            let mut left = 0.0;
            let mut right = 0.0;
            for voice in &mut self.voices {
                voice.gain += (voice.target_gain - voice.gain) * 0.0025;
                let source_frame = voice.frame as usize;
                let (source_left, source_right) =
                    direct_stereo_frame(&voice.clip.source, source_frame);
                left += source_left * voice.gain;
                right += source_right * voice.gain;
                voice.frame += f64::from(voice.clip.sample_rate) / f64::from(SAMPLE_RATE);
                if voice.frame as usize >= voice.clip.source.frames() && voice.repeat {
                    let (start, end) = voice
                        .clip
                        .source
                        .loop_range()
                        .unwrap_or((0, voice.clip.source.frames()));
                    let length = end.saturating_sub(start).max(1);
                    voice.frame =
                        (start + (voice.frame as usize).saturating_sub(start) % length) as f64;
                }
            }
            frame[0] = (frame[0] + left).clamp(-1.0, 1.0);
            frame[1] = (frame[1] + right).clamp(-1.0, 1.0);
        }
        self.voices.retain(|voice| {
            (!voice.releasing || voice.gain > 0.0005)
                && (voice.repeat || (voice.frame as usize) < voice.clip.source.frames())
        });
    }
}

/// The first live family uses the recovered `Class_grind` constructor/update payload. Additional
/// families stay out until their gameplay constructors have typed inputs; inventing payload words
/// here would make the graph sound plausible while bypassing the authored selection programs.
#[derive(Default)]
struct EventProducer {
    treatment: Option<u32>,
    treatment_phase: usize,
    rolling: Option<[u32; 2]>,
    rattle: Option<u32>,
    rattle_next_tick: u64,
    rattle_speed: u32,
    footsteps: Option<[u32; 2]>,
    foot_down: [bool; 2],
    foot_value: [u32; 2],
    grind: Option<u32>,
    flip: Option<u32>,
    flip_started: u64,
    last_airborne: bool,
    last_trick: Option<String>,
}

impl EventProducer {
    /// Retail `sub_824C9830` posts two held `Class_rolling` messages (layers 0 and 3) once, and
    /// `sub_824C9948` rewrites and re-delivers both every frame for the rest of the session. They
    /// are never released while the rider owns the board; silence comes from the gain word.
    fn update_rolling(
        &mut self,
        runtime: &mut AuthoredRuntime,
        observation: &PlayerAudioObservation,
    ) -> Result<(), String> {
        let speed = rolling_speed_word(observation.board_speed);
        let handles = match self.rolling {
            Some(handles) => handles,
            None => {
                let mut handles = [0; 2];
                for (handle, layer) in handles.iter_mut().zip(ROLLING_LAYERS) {
                    *handle = runtime
                        .post(
                            "Class_rolling",
                            &rolling_constructor(speed, layer.selector, DEFAULT_ROLLING_SURFACE),
                        )
                        .map_err(|error| error.to_string())?;
                }
                self.rolling = Some(handles);
                eprintln!(
                    "SKATE_PLAYER_AUDIO rolling_start tick={} handles={:#010x},{:#010x}",
                    observation.tick, handles[0], handles[1]
                );
                handles
            }
        };
        let contact = observation.grounded && !observation.grinding && !observation.wiping_out;
        for (handle, layer) in handles.into_iter().zip(ROLLING_LAYERS) {
            let gain = if contact {
                rolling_gain_word(speed, layer.gain_cap)
            } else {
                0
            };
            runtime
                .redeliver(
                    handle,
                    &rolling_update(speed, layer.selector, DEFAULT_ROLLING_SURFACE, gain),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Retail `sub_824C6198`: while the wheels are down, each rattle trigger releases the previous
    /// `Rolling_Rattle_Class` message and posts a fresh one carrying the current speed. The retail
    /// traces show those triggers about every 1.07 s, including at rest (speed word 0).
    fn update_rattle(
        &mut self,
        runtime: &mut AuthoredRuntime,
        observation: &PlayerAudioObservation,
    ) -> Result<(), String> {
        let contact = observation.grounded && !observation.grinding && !observation.wiping_out;
        if !contact {
            self.rattle_next_tick = 0;
        } else if observation.tick >= self.rattle_next_tick {
            self.rattle_next_tick = observation.tick + RATTLE_RETRIGGER_TICKS;
            if let Some(handle) = self.rattle.take() {
                runtime.release(handle).map_err(|error| error.to_string())?;
            }
            let speed = rattle_speed_word(observation.board_speed);
            self.rattle_speed = speed;
            let handle = runtime
                .post(
                    "Rolling_Rattle_Class",
                    &rattle_constructor(speed, DEFAULT_RATTLE_SURFACE, RATTLE_TWEAK),
                )
                .map_err(|error| error.to_string())?;
            self.rattle = Some(handle);
        }
        // `sub_824C80C0` re-delivers the held rattle every frame, contact or not; its gain words
        // follow the live wheel contact, so the voice is silent in the air and between triggers.
        if let Some(handle) = self.rattle {
            let contact_gain = if contact {
                rolling_gain_word(
                    rolling_speed_word(observation.board_speed),
                    ROLLING_LAYERS[0].gain_cap,
                )
            } else {
                0
            };
            runtime
                .redeliver(
                    handle,
                    &rattle_update(
                        self.rattle_speed,
                        DEFAULT_RATTLE_SURFACE,
                        RATTLE_TWEAK,
                        contact_gain,
                    ),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Retail `sub_824E9FD8` posts two held `playercharacter_footstep` messages (object +36 first
    /// updated, +220 second) and `sub_824EAEA8` rewrites and re-delivers both every frame. Word 8
    /// is each foot's *down level* (`sub_824E9270` copies audio-state bytes +724/+725 into +52/+236
    /// every frame); the authored patch starts its heel/toe/scuff layers on the 0->1 edge, so a
    /// foot held down must not re-trigger. On the board the +36 foot is down for the duration of
    /// the animation's `push_contact` (all 193 push plants in the recorded `play1` session used
    /// it); walking uses the native offboard foot manager's per-foot support flags.
    fn update_footsteps(
        &mut self,
        runtime: &mut AuthoredRuntime,
        observation: &PlayerAudioObservation,
    ) -> Result<(), String> {
        let handles = match self.footsteps {
            Some(handles) => handles,
            None => {
                let constructor = footstep_constructor();
                let handles = [
                    runtime
                        .post("playercharacter_footstep", &constructor)
                        .map_err(|error| error.to_string())?,
                    runtime
                        .post("playercharacter_footstep", &constructor)
                        .map_err(|error| error.to_string())?,
                ];
                self.footsteps = Some(handles);
                handles
            }
        };
        let walking = observation.state == 500 && !observation.wiping_out;
        let pushing = observation.foot_push_speed > 0.0 && !observation.wiping_out;
        let down = [
            pushing || (walking && observation.feet_supported[0]),
            walking && observation.feet_supported[1],
        ];
        for foot in 0..2 {
            if down[foot] && !self.foot_down[foot] {
                self.foot_value[foot] = if pushing && foot == 0 {
                    push_foot_value(observation.foot_push_speed)
                } else {
                    WALKING_FOOT_VALUE
                };
                eprintln!(
                    "SKATE_PLAYER_AUDIO foot_down tick={} foot={foot} {} value={}",
                    observation.tick,
                    if pushing && foot == 0 { "push" } else { "step" },
                    self.foot_value[foot]
                );
            }
        }
        self.foot_down = down;
        let body_speed = (observation.rider_speed * 100.0).round().clamp(0.0, 1000.0) as u32;
        // Word 13 is `fctiwz` of the animation's AudibleFootStepStrength, clamped to 1..99.
        let strength = (observation.footstep_strength as i32).clamp(1, 99) as u32;
        for (foot, handle) in handles.into_iter().enumerate() {
            runtime
                .redeliver(
                    handle,
                    &footstep_update(down[foot], self.foot_value[foot], body_speed, strength),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn update_treatment(
        &mut self,
        runtime: &mut AuthoredRuntime,
        observation: &PlayerAudioObservation,
    ) -> Result<(), String> {
        let handle = match self.treatment {
            Some(handle) => handle,
            None => {
                // Exact Class_Treatment constructor message from the retail session trace. The
                // object is posted once when the player-audio owner starts and remains held.
                let post = [
                    0, 0x7fff, 0, 0, 0x1000, 0x61a8, 0, 0, 0, 0, 0x01f4, 0, 0, 0, 0, 0x1b58,
                    0x6d60, 0x7fff, 0, 1, 1, 8,
                ];
                let handle = runtime
                    .post("Class_Treatment", &post)
                    .map_err(|error| error.to_string())?;
                self.treatment = Some(handle);
                eprintln!(
                    "SKATE_PLAYER_AUDIO treatment_start tick={} handle={handle:#010x}",
                    observation.tick
                );
                handle
            }
        };

        if observation.landed {
            self.treatment_phase = 1;
        }
        let update = treatment_update(self.treatment_phase);
        runtime
            .redeliver(handle, &update)
            .map_err(|error| error.to_string())?;
        if self.treatment_phase != 0 {
            self.treatment_phase += 1;
            if self.treatment_phase > TREATMENT_LANDING_UPDATES.len() {
                self.treatment_phase = 0;
            }
        }
        Ok(())
    }

    fn flip_selector(observation: &PlayerAudioObservation) -> u32 {
        let name = observation
            .trick_identifier
            .as_deref()
            .unwrap_or_default()
            .trim_start_matches("N_")
            .trim_start_matches("90_")
            .to_ascii_lowercase();
        let selector = if name.contains("360inwardheelflip") {
            14
        } else if name.contains("360hardflip") {
            13
        } else if name.contains("laserflip") {
            12
        } else if name.contains("360flip") {
            11
        } else if name.contains("fs360popshuv") {
            10
        } else if name.contains("360popshuv") {
            9
        } else if name.contains("inwardheelflip") {
            8
        } else if name.contains("hardflip") {
            7
        } else if name.contains("varialheelflip") {
            6
        } else if name.contains("varialkickflip") {
            5
        } else if name.contains("fspopshuv") {
            4
        } else if name.contains("popshuv") || name.contains("shuv") {
            3
        } else if name.contains("heelflip") {
            2
        } else if name.contains("kickflip") {
            1
        } else if observation.nollie || name == "nollie" {
            29
        } else {
            28
        };
        selector
    }

    fn start_flip(
        &mut self,
        runtime: &mut AuthoredRuntime,
        observation: &PlayerAudioObservation,
    ) -> Result<(), String> {
        if let Some(handle) = self.flip.take() {
            runtime.release(handle).map_err(|error| error.to_string())?;
        }
        let selector = Self::flip_selector(observation);
        let speed = observation.board_speed.max(observation.rider_speed);
        let strength = (650.0 + speed * 45.0).round().clamp(0.0, 1000.0) as u32;
        // Recovered Class_Flips constructor layout. Words 7..11 are its five clamped gameplay
        // inputs; words 16..27 are the stock voice-property defaults captured from retail.
        let post = [
            0, 32_767, 0, 0, 0x1000, 0x61a8, 0, 0, 0, strength, 500, selector, 0, 0, 0, 0, 1,
            0x6bfe, 0x3a00, 0x1a00, 0, 0x2710, 0, 0, 1, 1, 0, 6,
        ];
        let handle = runtime
            .post("Class_Flips", &post)
            .map_err(|error| error.to_string())?;
        self.flip = Some(handle);
        self.flip_started = observation.tick;
        eprintln!(
            "SKATE_PLAYER_AUDIO flip_start tick={} trick={:?} selector={} strength={} handle={handle:#010x}",
            observation.tick, observation.trick_identifier, selector, strength
        );
        Ok(())
    }

    fn apply(
        &mut self,
        runtime: &mut AuthoredRuntime,
        observation: &PlayerAudioObservation,
    ) -> Result<(), String> {
        self.update_treatment(runtime, observation)?;
        self.update_rolling(runtime, observation)?;
        self.update_rattle(runtime, observation)?;
        self.update_footsteps(runtime, observation)?;
        let airborne = matches!(observation.state, 103 | 200 | 201 | 202);
        let trick_changed = observation.trick_identifier.is_some()
            && observation.trick_identifier != self.last_trick;
        let took_off = !self.last_airborne && airborne;
        if !observation.wiping_out && airborne && (took_off || trick_changed) {
            self.start_flip(runtime, observation)?;
        }
        if let Some(handle) = self.flip {
            if !airborne
                || observation.wiping_out
                || observation.tick.saturating_sub(self.flip_started) > 36
            {
                runtime.release(handle).map_err(|error| error.to_string())?;
                self.flip = None;
            } else {
                let speed = observation.board_speed.max(observation.rider_speed);
                let strength = (650.0 + speed * 45.0).round().clamp(0.0, 1000.0) as u32;
                let selector = Self::flip_selector(observation);
                // Retail re-delivery replaces the constructor header and drives the patch's gain,
                // pitch and trick-selector ramps. Without this update, the selected voice is
                // intentionally opened at zero gain.
                let update = [
                    0x38d7, 32_767, 0, 0x0999, 0x0fe6, 0x618b, 0, 0, 0, strength, 500, selector, 0,
                    0, 0, 0, 1,
                ];
                runtime
                    .redeliver(handle, &update)
                    .map_err(|error| error.to_string())?;
            }
        }

        if observation.grinding {
            let physical_speed = observation.board_speed.max(observation.rider_speed);
            let (speed, level) = grind_controls(physical_speed);
            let surface = observation.grind_audio_surface.min(14);
            // sub_824C39E0 maps the six retained physical grind families to the authored patch
            // layers before posting/updating: boardslide=2, 50-50/tipslide/backslash=0,
            // five-o=2, darkslide=3. Previously every grind was forced through layer 2.
            let layer = grind_layer(observation.grind_family);
            if let Some(handle) = self.grind {
                // Recorded Class_grind updates replace the constructor's first word and publish
                // the live level, contact scalar, filters, speed, surface and grind layer.
                let update = [
                    32_767, level, 0x0434, 0, 0x1000, 0x618b, 0x004d, speed, 0x0400, surface,
                    layer, 0x628e, 0, 0, 0, 0, 5,
                ];
                runtime
                    .redeliver(handle, &update)
                    .map_err(|error| error.to_string())?;
            } else {
                // Exact 17-word constructor payload from the retail Class_grind trace. Layer 2 is
                // the sustained rail voice; the old shifted payload selected near-silent impulses.
                let post = [
                    0, 32_767, 0, 0, 0, 25_000, 0, speed, 0x0400, surface, layer, 0x628e, 0, 0, 0,
                    0, 5,
                ];
                let handle = runtime
                    .post("Class_grind", &post)
                    .map_err(|error| error.to_string())?;
                eprintln!(
                    "SKATE_PLAYER_AUDIO grind_start tick={} speed={physical_speed:.3} family={} substate={} surface={} impact={:.3} layer={} handle={handle:#010x}",
                    observation.tick,
                    observation.grind_family,
                    observation.grind_substate,
                    observation.grind_audio_surface,
                    observation.grind_impact_speed,
                    layer
                );
                self.grind = Some(handle);
            }
        } else if let Some(handle) = self.grind.take() {
            runtime.release(handle).map_err(|error| error.to_string())?;
            eprintln!(
                "SKATE_PLAYER_AUDIO grind_stop tick={} handle={handle:#010x}",
                observation.tick
            );
        }
        self.last_airborne = airborne;
        self.last_trick = observation.trick_identifier.clone();
        Ok(())
    }
}

// Wheel packets, from the retail constructors `sub_824C4C18` (Class_rolling) and `sub_824B0248`
// (Rolling_Rattle_Class). Each constructor fills a 52-byte object and posts object+4, so a packet
// is exactly 12 words. Retail trace lines print 28 words; everything past word 11 is the next heap
// block (including the next object's message node, which is not a linked control graph).

/// Surface word for every wheel packet until per-material mapping exists. 2 is the surface retail
/// reports on ~71% of rolling updates in the recorded sessions.
const DEFAULT_ROLLING_SURFACE: u32 = 2;
/// Rattle's surface code for rolling surface 2, as observed in every retail surface-2 rattle post.
const DEFAULT_RATTLE_SURFACE: u32 = 2;
/// Rattle's sixth tuning input; constant (8) in every recorded retail rattle post.
const RATTLE_TWEAK: u32 = 8;
/// Retail rattle triggers: median 1.09 s, p10 0.97 s apart while the wheels are down (60 Hz ticks).
const RATTLE_RETRIGGER_TICKS: u64 = 64;

struct RollingLayer {
    selector: u32,
    /// Retail gain word (vfunc 7) once rolling faster than ~0.92 m/s.
    gain_cap: u32,
}

/// `sub_824C9830` posts layer 0 into holder +1304 and layer 3 into +1308.
const ROLLING_LAYERS: [RollingLayer; 2] = [
    RollingLayer {
        selector: 0,
        gain_cap: 12_999,
    },
    RollingLayer {
        selector: 3,
        gain_cap: 4_913,
    },
];

/// `sub_824C9948`: `clamp(v * 3.6 / maxSpeed[layer], 0, 1) * 10000`, truncated (`fctiwz`). The 3.6
/// and 10000 are image constants (0x822F8628, 0x821161A0). `maxSpeed` is the AttribSys float array
/// 0x880C82E8EF647EC4 read by `sub_824C97B8(layer)`; the owner's skatercollections vault holds
/// [70, 65, 65, 70, 100, 45, ...], so both held layers (0 and 3) use 70 km/h.
fn rolling_speed_word(board_speed: f32) -> u32 {
    const MAX_SPEED_KMH: f32 = 70.0;
    ((board_speed * 3.6 / MAX_SPEED_KMH).clamp(0.0, 1.0) * 10_000.0) as u32
}

/// `sub_824C6198`: `clamp((v - 1) * 3.6 / D, 0, 1) * 10000`, truncated. `D` is the AttribSys float
/// 0x12275AA8AC4A63FB, 30 km/h in the owner's vault (independently regressed from 69 recorded
/// rattle posts as 30.5).
fn rattle_speed_word(board_speed: f32) -> u32 {
    const DIVISOR_KMH: f32 = 30.0;
    (((board_speed - 1.0) * 3.6 / DIVISOR_KMH).clamp(0.0, 1.0) * 10_000.0) as u32
}

/// Recorded retail gain rises linearly with the speed word and saturates at 460 (≈0.92 m/s).
fn rolling_gain_word(speed: u32, cap: u32) -> u32 {
    ((u64::from(speed) * u64::from(cap)) / 460).min(u64::from(cap)) as u32
}

/// `sub_824C4C18` constructor fields, object +4..+48.
fn rolling_constructor(speed: u32, selector: u32, surface: u32) -> [u32; 12] {
    [
        0,
        0,
        0x1000,
        speed.min(10_000),
        selector.min(15),
        0,
        surface.min(13),
        0,
        0,
        25_000,
        0,
        32_767,
    ]
}

/// Footstep packets: 25 words, from the retail constructor `sub_824B73E0` (as called by
/// `sub_824E9FD8`) and updater `sub_824EAEA8`. Retail update trace lines print only words 0..16;
/// words 17..24 come from the updater itself.
///
/// Word 10 on a walking step: the median of 20,000+ recorded retail step frames.
const WALKING_FOOT_VALUE: u32 = 48;
/// Words 18..23: AttribSys Int32 array 0x636464FBAD0D71A3 (six entries, read through
/// `sub_824ADF90` every frame) from the owner's skatercollections vault. Word 23 multiplies the
/// footstep patch's start gain, so it is required for any audible step.
const FOOTSTEP_TUNING: [u32; 6] = [32_767, 10_000, 15_000, 25_000, 32_767, 28_000];

fn footstep_constructor() -> [u32; 25] {
    let mut words = [0u32; 25];
    words[2] = 0x1000;
    words[4] = 25_000;
    words[7] = 32_767;
    words[12] = 1;
    words[13] = 1;
    words[16] = 1;
    words[17] = 1;
    words[24] = 12;
    words
}

/// Retail push plants carry word 10 at p10 169 / p50 513 / p90 527 against engine foot push speeds
/// of p10 0.68 / p50 3.25 / p90 7.45 m/s; this linear map reproduces that distribution.
fn push_foot_value(foot_push_speed: f32) -> u32 {
    (foot_push_speed * 160.0).round().clamp(0.0, 527.0) as u32
}

/// `sub_824EAEA8` per-foot rewrite. Words 1 (azimuth), 2, 4..7 are the owner's spatial virtuals,
/// constant with the follow camera; 12, 15, 16, 17 are constant in the recorded sessions
/// (16 = default surface); 13 is the step strength; 14 is the rider speed in cm/s.
fn footstep_update(planted: bool, foot_value: u32, body_speed: u32, strength: u32) -> [u32; 25] {
    let mut words = [0u32; 25];
    words[0] = 32_767;
    words[2] = 4_086;
    words[4] = 24_971;
    words[5] = 77;
    words[6] = 1_789;
    words[7] = 9_202;
    words[8] = u32::from(planted);
    words[9] = if planted { 2 } else { 0 };
    words[10] = foot_value.min(1_000);
    words[12] = 1;
    words[13] = strength.clamp(1, 99);
    words[14] = body_speed.min(1_000);
    words[15] = 1;
    words[16] = 2;
    words[17] = 1;
    words[18..24].copy_from_slice(&FOOTSTEP_TUNING);
    words[24] = 12;
    words
}

/// `sub_824C9948` per-frame rewrite. Words 1, 2, 8, 9 and 10 come from the owner's spatial
/// virtuals; with the follow camera they are constant in every recorded session except word 1, a
/// small azimuth, which is left centred.
fn rolling_update(speed: u32, selector: u32, surface: u32, gain: u32) -> [u32; 12] {
    [
        32_767,
        0,
        4_086,
        speed.min(10_000),
        selector.min(15),
        0,
        surface.min(13),
        0,
        0,
        24_971,
        77,
        gain.min(32_767),
    ]
}

/// `sub_824B0248` constructor fields, object +4..+48.
fn rattle_constructor(speed: u32, surface: u32, tweak: u32) -> [u32; 12] {
    [
        0,
        0,
        0x1000,
        speed.min(10_000),
        surface.min(8),
        0,
        1,
        0,
        25_000,
        0,
        32_767,
        tweak.min(32_767),
    ]
}

/// `sub_824C80C0` per-frame rewrite (object +4, +8, +12, +32, +36, +40, +44); the speed, surface and
/// tweak words keep their constructor values. Across 60,863 recorded frames word 10 is 0.92x and
/// word 7 0.131x the concurrent rolling layer-0 gain, both zero whenever that gain is zero.
fn rattle_update(speed: u32, surface: u32, tweak: u32, contact_gain: u32) -> [u32; 12] {
    let gain = u64::from(contact_gain);
    [
        32_767,
        0,
        4_055,
        speed.min(10_000),
        surface.min(8),
        0,
        1,
        (gain * 131 / 1000) as u32,
        24_971,
        77,
        (gain * 9226 / 10_000).min(32_767) as u32,
        tweak.min(32_767),
    ]
}

// These are ordered 60 Hz re-deliveries captured from the held retail Class_Treatment during a
// normal ollie landing. Replaying the sequence through the authored Treatments patch opens one
// treatment voice; directly choosing sample slots 13/14 bypassed its severity/variation graph.
const TREATMENT_LANDING_UPDATES: [[u32; 3]; 10] = [
    [0x0010, 0x02ae, 0x0025],
    [0x0032, 0x028c, 0x0039],
    [0x0064, 0x025a, 0x005d],
    [0x00c8, 0x01f6, 0x0098],
    [0x010a, 0x01b4, 0x00b2],
    [0x013c, 0x0182, 0x00b7],
    [0x0190, 0x012e, 0x00b7],
    [0x01f4, 0x00ca, 0x00b7],
    [0x028a, 0x0034, 0x00b7],
    [0x02ab, 0x0013, 0x00b7],
];

/// Retail updater `sub_824DD6F0`. Word 10 is `timeScale * 500` (image constant 0x820BD5C4 = 500,
/// applied to the global slow-motion timer that also gates Hall of Meat), so it stays 500 at
/// normal game speed. Writing a landing strength here played the Treatments patch up to 2x fast.
fn treatment_update(phase: usize) -> [u32; 17] {
    let mut update = [
        0x2caf, 0x7fff, 0, 0x097d, 0x0ff6, 0x61a8, 0, 0, 0, 0, 0x01f4, 0, 0, 0, 1, 0x1b58, 0x6d60,
    ];
    if let Some(values) = phase
        .checked_sub(1)
        .and_then(|index| TREATMENT_LANDING_UPDATES.get(index))
    {
        update[7..10].copy_from_slice(values);
    }
    update
}

fn landing_treatment_strength(
    impact_speed: f32,
    clean: bool,
    sketchy: bool,
    landing_type: u32,
) -> u32 {
    let severity = ((impact_speed - 1.5) / 5.0).clamp(0.0, 1.0);
    let quality = if sketchy {
        140.0
    } else if !clean && landing_type != 0 {
        70.0
    } else {
        0.0
    };
    (350.0 + severity * 650.0 + quality)
        .round()
        .clamp(1.0, 1_000.0) as u32
}

/// Temporary direct-mixer level for the shipped treatment bank while the recovered authored
/// `Class_Treatment` graph is still silent on the host output. Its input is the same 1..=1000
/// strength sent to the retail-style patch above, so clean landings remain quiet and hard or
/// sketchy landings remain clearly distinct.
fn treatment_fallback_gain(strength: u32) -> f32 {
    0.055 + strength.clamp(1, 1_000) as f32 * 0.000_145
}

fn grind_controls(physical_speed: f32) -> (u32, u32) {
    let speed = (physical_speed * 1_000.0).round().clamp(0.0, 10_000.0) as u32;
    // An admitted rail contact keeps a quiet texture at a stall. Motion speed still reaches zero,
    // so the authored patch can settle pitch without muting contact entirely.
    let level = (physical_speed * 700.0).round().clamp(650.0, 32_767.0) as u32;
    (speed, level)
}

/// Exact family-to-patch-layer switch from retail `sub_824C39E0`.
fn grind_layer(family: u32) -> u32 {
    match family {
        1 | 2 | 4 => 0,
        5 => 3,
        _ => 2,
    }
}

/// The retail shoe and wheel banks are laid out as 24 audio materials with a fixed number of
/// variants per material: footsteps 24x8, foot drag 24x7, wheel skid 24x4. Clamp custom-map IDs to
/// the final stock material instead of wrapping them into a different sounding surface.
fn material_slots<const VARIANTS: usize>(surface: usize) -> [usize; VARIANTS] {
    let first = surface.min(23) * VARIANTS;
    std::array::from_fn(|variant| first + variant)
}

fn advance_offboard_stride(distance: &mut f32, speed: f32) -> bool {
    if speed <= 0.25 {
        *distance = 0.0;
        return false;
    }
    *distance += speed * (1.0 / 60.0);
    let stride = (0.74 + speed * 0.035).clamp(0.74, 0.92);
    if *distance < stride {
        return false;
    }
    *distance -= stride;
    true
}

fn deceleration_skid_allowed(
    rolling: bool,
    powersliding: bool,
    was_airborne: bool,
    tick: u64,
    landing_suppressed_until: u64,
    next_skid_tick: u64,
    deceleration: f32,
) -> bool {
    rolling
        && !powersliding
        && !was_airborne
        && tick >= landing_suppressed_until
        && tick >= next_skid_tick
        && deceleration > 0.35
}

fn downmix(native: &[f32]) -> Vec<f32> {
    let mut stereo = Vec::with_capacity(native.len() / 3);
    for frame in native.chunks_exact(usize::from(PCM_CHANNELS)) {
        // Retail Dac order is front L/R, centre, LFE, surround L/R. Leave LFE out of this
        // full-range source and retain headroom for simultaneously authored layers.
        // Restore the missing device/master level at the host edge and contain rare overshoots.
        stereo
            .push(((frame[0] + 0.707 * frame[2] + 0.5 * frame[4]) * OUTPUT_GAIN).clamp(-1.0, 1.0));
        stereo
            .push(((frame[1] + 0.707 * frame[2] + 0.5 * frame[5]) * OUTPUT_GAIN).clamp(-1.0, 1.0));
    }
    stereo
}

/// Fold decoded resident bank material into the same stereo convention as the recovered retail
/// output path.  The old direct fallback averaged all channels and copied the result to both
/// speakers, which discarded the treatment bank's four-channel separation and made every
/// stereo board/cloth source sound centred and thinner than retail.
fn direct_stereo_frame(source: &PcmSource, frame: usize) -> (f32, f32) {
    let sample = |channel| source.sample(frame, channel).unwrap_or(0.0);
    match source.channels() {
        0 => (0.0, 0.0),
        1 => {
            let mono = sample(0);
            (mono, mono)
        }
        2 => (sample(0), sample(1)),
        3 => (sample(0) + 0.707 * sample(2), sample(1) + 0.707 * sample(2)),
        // EAAC/XMA four-channel material is front L/R followed by surround L/R.  In particular,
        // the traced normal Treatments voice is four-channel; preserve its rear impact layers.
        4 => (sample(0) + 0.5 * sample(2), sample(1) + 0.5 * sample(3)),
        _ => (
            sample(0) + 0.707 * sample(2) + 0.5 * sample(4),
            sample(1) + 0.707 * sample(2) + 0.5 * sample(5),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected words below are copied from the recorded retail session traces (sk8Audio
    // probe/traces/sessions/play1 and play4), not recomputed from these helpers.

    #[test]
    fn wheel_constructors_match_the_recorded_retail_posts() {
        // play1 msg 3: Class_rolling payload 40C884E4, words 0..11 (later words are heap).
        assert_eq!(
            rolling_constructor(0, 0, 3),
            [0, 0, 0x1000, 0, 0, 0, 3, 0, 0, 0x61A8, 0, 0x7FFF]
        );
        // play4 msg 16: Rolling_Rattle_Class payload 40C88564, words 0..11.
        assert_eq!(
            rattle_constructor(0, 2, 8),
            [0, 0, 0x1000, 0, 2, 0, 1, 0, 0x61A8, 0, 0x7FFF, 8]
        );
    }

    #[test]
    fn footstep_constructor_matches_the_recorded_retail_post() {
        // play1: every playercharacter_footstep post (payloads 40C93AA4, 40C936A4, ...).
        assert_eq!(
            footstep_constructor(),
            [
                0, 0, 0x1000, 0, 0x61A8, 0, 0, 0x7FFF, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0,
                0, 0x0C
            ]
        );
    }

    #[test]
    fn rolling_update_reproduces_recorded_retail_frames() {
        // play1 node 40C02F10 rows: (speed word, gain word) pairs from the ramp-in region.
        for (speed, gain) in [(59, 1666), (125, 3542), (182, 5150)] {
            let computed = rolling_gain_word(speed, 12_999);
            assert!(
                computed.abs_diff(gain) <= 20,
                "speed {speed}: {computed} vs recorded {gain}"
            );
        }
        assert_eq!(rolling_gain_word(4_000, 12_999), 12_999);
        assert_eq!(rolling_gain_word(4_000, 4_913), 4_913);
        let update = rolling_update(2_999, 0, DEFAULT_ROLLING_SURFACE, 12_999);
        assert_eq!(update[0], 32_767);
        assert_eq!((update[2], update[9], update[10]), (4_086, 24_971, 77));
        assert_eq!(
            (update[3], update[4], update[6], update[11]),
            (2_999, 0, 2, 12_999)
        );
    }

    #[test]
    fn wheel_speed_words_use_the_vault_tuning() {
        // 70 km/h rolling maximum and 30 km/h rattle divisor; truncation as fctiwz.
        assert_eq!(rolling_speed_word(0.0), 0);
        assert_eq!(rolling_speed_word(25.0), 10_000);
        assert!(rolling_speed_word(35.0 / 3.6).abs_diff(5_000) <= 1);
        assert_eq!(rattle_speed_word(1.0), 0);
        assert_eq!(rattle_speed_word(0.5), 0);
        assert_eq!(rattle_speed_word(20.0), 10_000);
        assert!(rattle_speed_word(1.0 + 15.0 / 3.6).abs_diff(5_000) <= 1);
    }

    #[test]
    fn footstep_update_carries_the_gain_tuning_the_patch_multiplies() {
        let idle = footstep_update(false, 0, 0, 0);
        assert_eq!((idle[8], idle[9], idle[13]), (0, 0, 1));
        assert_eq!(idle[23], 28_000, "word 23 zero would silence every step");
        assert_eq!(idle[24], 12, "voice property 9 reads word 24");
        let plant = footstep_update(true, 513, 297, 3);
        assert_eq!(
            (plant[8], plant[9], plant[10], plant[13], plant[14]),
            (1, 2, 513, 3, 297)
        );
    }

    #[test]
    fn queued_contact_and_landing_survive_a_following_idle_tick() {
        let (sender, receiver) = mpsc::sync_channel(16);
        // One presentation frame can forward multiple completed physics ticks together.
        sender.send((10, "foot contact")).unwrap();
        sender.send((11, "landing")).unwrap();
        sender.send((12, "idle")).unwrap();
        assert_eq!(
            next_observation(&receiver).unwrap(),
            Some((10, "foot contact"))
        );
        assert_eq!(next_observation(&receiver).unwrap(), Some((11, "landing")));
        assert_eq!(next_observation(&receiver).unwrap(), Some((12, "idle")));
        assert_eq!(next_observation(&receiver).unwrap(), None);
    }

    #[test]
    fn grind_families_select_the_retail_patch_layers() {
        assert_eq!(grind_layer(0), 2);
        assert_eq!(grind_layer(1), 0);
        assert_eq!(grind_layer(2), 0);
        assert_eq!(grind_layer(3), 2);
        assert_eq!(grind_layer(4), 0);
        assert_eq!(grind_layer(5), 3);
    }

    #[test]
    fn material_banks_select_only_the_current_retail_surface_group() {
        assert_eq!(material_slots::<8>(0), [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(material_slots::<4>(14), [56, 57, 58, 59]);
        assert_eq!(material_slots::<7>(23), [161, 162, 163, 164, 165, 166, 167]);
        assert_eq!(material_slots::<4>(127), [92, 93, 94, 95]);
    }

    #[test]
    fn landing_speed_loss_does_not_post_a_wheel_skid() {
        assert!(!deceleration_skid_allowed(
            true, false, true, 100, 112, 0, 3.0
        ));
        assert!(!deceleration_skid_allowed(
            true, false, false, 108, 112, 0, 3.0
        ));
        assert!(deceleration_skid_allowed(
            true, false, false, 112, 112, 0, 0.5
        ));
    }

    #[test]
    fn hard_landings_drive_a_stronger_treatment_payload() {
        assert_eq!(landing_treatment_strength(0.5, true, false, 0), 350);
        assert!(landing_treatment_strength(4.0, true, false, 0) > 500);
        assert_eq!(landing_treatment_strength(8.0, true, false, 0), 1_000);
        assert!(
            landing_treatment_strength(3.0, false, true, 1)
                > landing_treatment_strength(3.0, true, false, 0)
        );
    }

    #[test]
    fn direct_treatment_fallback_preserves_landing_severity() {
        let clean = landing_treatment_strength(0.5, true, false, 0);
        let hard = landing_treatment_strength(8.0, true, false, 0);
        let sketchy = landing_treatment_strength(3.0, false, true, 1);
        assert!(treatment_fallback_gain(hard) > treatment_fallback_gain(clean));
        assert!(treatment_fallback_gain(sketchy) > treatment_fallback_gain(clean));
        assert!(treatment_fallback_gain(hard) <= 0.20);
    }

    #[test]
    fn direct_bank_downmix_preserves_the_treatment_rear_layers() {
        let source =
            PcmSource::new(std::sync::Arc::from([10_000i16, -8_000, 6_000, -4_000]), 4).unwrap();
        let (left, right) = direct_stereo_frame(&source, 0);
        assert!(left > 0.39, "left keeps the front and rear-left layers");
        assert!(right < -0.30, "right keeps the front and rear-right layers");
        assert_ne!(left, right, "four-channel source must not collapse to mono");
    }

    #[test]
    fn admitted_grind_retains_quiet_contact_at_a_stall() {
        assert_eq!(grind_controls(0.0), (0, 650));
        assert_eq!(grind_controls(2.0), (2_000, 1_400));
    }

    #[test]
    fn offboard_stride_continues_without_animation_contact_markers() {
        let mut distance = 0.0;
        let steps = (0..120)
            .filter(|_| advance_offboard_stride(&mut distance, 2.0))
            .count();
        assert!((4..=5).contains(&steps));
        assert!(!advance_offboard_stride(&mut distance, 0.0));
        assert_eq!(distance, 0.0);
    }

    #[test]
    fn downmix_keeps_frames_and_separates_sides() {
        let input = [0.000_005, 0.000_01, 0.000_02, 100.0, 0.000_04, 0.000_08];
        let output = downmix(&input);
        assert_eq!(output.len(), 2);
        assert!(output[1] > output[0]);
        assert!(
            output.iter().all(|sample| *sample < 1.0),
            "LFE must not enter full-range output"
        );
    }
}
