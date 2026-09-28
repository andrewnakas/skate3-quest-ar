//! Playback of the game's own audio streams.
//!
//! The engine had no game audio at all: the only sounds were synthesized. This plays the real
//! thing, out of the retail archives, through the decode path in `skate-data`.
//!
//! Decoding is not cheap and it spawns an external process, so a request never runs on a schedule
//! thread. It goes to the async compute pool and the finished PCM is picked up on a later frame.
//! A stream that fails to decode is reported once and dropped rather than retried, because every
//! failure here is a property of the asset or the host's decoder and retrying would only repeat
//! the process spawn every frame.
//!
//! ## Verified by recording the sound card
//!
//! `skate3-audio-check` (`src/audio_check_main.rs`) runs this plugin with no window, renderer or
//! assets, so the audio path can be exercised on a checkout the game itself will not boot on.
//! Playing 60 blocks of the five-channel ambience bed through it, while recording the output
//! device's monitor:
//!
//! * the plugin decoded 306,816 frames of 5 channels (6.39 s) in 0.19 s and the stream ended
//!   after 6.6 s;
//! * the recorded level rose from about -58 dBFS before playback to about -45 dBFS during it;
//! * cross-correlating the decoded waveform against the recording peaks at **lag 1.370 s** --
//!   exactly the recorder's one-second head start plus this program's startup -- with a
//!   peak-to-mean ratio of **58.8**, against **5.7** for a time-reversed control whose peak lands
//!   at a lag outside the overlap.
//!
//! The absolute correlation is 0.22 rather than near 1 because the mixer downmixes five channels
//! to two and the device has its own response. The sharpness of the peak and the lag being right
//! are what carry the claim.
//!
//! ## What has been verified, and what has not
//!
//! The decode underneath this is checked byte-for-byte against an independently produced
//! reference on real archive data, and the logic in this file is unit-tested. But **no sound from
//! this path has been heard yet**: it was written on a checkout with no set-up asset pipeline, so
//! the game does not boot there to reach the startup hook. Treat the first run on a working
//! install as the real test, and start it with `SKATE_AUDIO_PLAY`.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use bevy::audio::{AddAudioSource, Source, Volume};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};
use skate_audio_formats::{banks, eaac};
use skate_core::physics::phase::PhysicsEvent;
use skate_data::audio::{self, ffmpeg::FfmpegDecoder};

#[path = "player_audio.rs"]
pub mod player;

/// One completed local-player physics tick, ready for the player-audio runtime.
///
/// This is transport only: it preserves the physics event order and raw surface vote without
/// claiming that a physics material id is an authored audio-material id. Patch-message creation,
/// random selection, and archive lookup remain the audio runtime's responsibilities.
/// A front-end one-shot to play, by the 64-bit AttribSys id retail passes to `GlobalFEPlaySound`
/// (`sub_825DFAF0`).
///
/// This lives here rather than beside its producer because `player_audio` is shared by every
/// binary in this crate while `session_marker` is declared only by the game binary.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontEndSoundRequest(pub u64);

#[derive(Message, Clone, Debug, PartialEq)]
pub struct PlayerAudioObservation {
    pub tick: u64,
    pub state: u32,
    pub board_speed: f32,
    pub rider_speed: f32,
    pub grounded: bool,
    /// The authoritative physical state is a grind. This is distinct from a deck scrape or a
    /// wheel contact, which can occur without an admitted grind.
    pub grinding: bool,
    /// Retained native Grind PhysOut fields. These are the inputs consumed by the retail grind
    /// audio updater; a ground-wheel material is not a substitute for its authored surface id.
    pub grind_family: u32,
    pub grind_substate: u32,
    pub grind_audio_surface: u32,
    pub grind_impact_speed: f32,
    pub wiping_out: bool,
    pub landed: bool,
    /// Downward speed immediately before a landing contact, in metres per second.
    pub landing_impact_speed: f32,
    pub landing_clean: bool,
    pub landing_sketchy: bool,
    pub landing_type: u32,
    pub landing_spin: f32,
    pub landing_sideways_speed: f32,
    pub footstep_strength: f32,
    pub footstep_bone: i32,
    pub foot_push_speed: f32,
    /// Per-foot ground support while walking (left, right), from the offboard foot manager.
    pub feet_supported: [bool; 2],
    /// Packed foot-query audio material (low seven bits), independent of the board wheels.
    pub foot_surface: u32,
    pub contact_count: u32,
    /// The stock MotionGraph's powerslide flag. This is the retail trigger for `Class_Squeaks`;
    /// ordinary speed loss must not be treated as a powerslide.
    pub powersliding: bool,
    /// The authored scoring descriptor active on this tick, resolved through the retail scoring
    /// catalog. The identifier is retained because its family (ollie, kickflip, shuv, and so on)
    /// is one of the inputs used by the trick-foley selectors.
    pub trick_id: Option<usize>,
    pub trick_identifier: Option<String>,
    pub animation_name: Option<String>,
    pub riding_switch: bool,
    pub riding_fakie: bool,
    pub nollie: bool,
    /// The retail audio-material vote from the low seven bits of the four wheel-query tags.
    /// This is intentionally not the friction/physics surface stored in bits 7..11.
    pub wheel_surface: u32,
    pub events: Vec<PhysicsEvent>,
    /// Front-end one-shot ids to play this frame, retail's `GlobalFEPlaySound` (`sub_825DFAF0`)
    /// arguments. `session_marker` emits these and the forwarder attaches them here; they are not
    /// physics events, so they do not belong in `events`.
    pub frontend_sounds: Vec<u64>,
    /// The native PhysOut fields the retail audio-state bridge `sub_824B0DA8` reads.
    pub retail: RetailAudioInputs,
}

/// Raw native PhysOut fields for the retail audio-state bridge `sub_824B0DA8`, which copies them
/// from the per-skater record `sub_827A1B78` builds, and for the PhysOut audio conditioner
/// (`sub_82772748`, PhysOut bundle slot B+40) whose output that record also packs. Transport
/// only: the player-audio worker runs the conditioner, the record packing and the bridge the way
/// retail does (`player_audio/audio_state.rs`). Field docs name the native PhysOut slot and
/// offset (B = `[[player+1808]]->vfunc92()`).
#[derive(Clone, Debug, PartialEq)]
pub struct RetailAudioInputs {
    /// The physics step (Processed+2604). The bridge's `f1` (hold clock +312) is the audio frame
    /// time; the engine runs the bridge once per physics observation.
    pub dt: f32,
    /// SkateboardMotion+164, the board's ground speed in m/s (audio state +208).
    pub ground_speed: f32,
    /// SystemReckoning+16, the COM velocity (audio state +96; its length is +212; its Y is the
    /// conditioner's landing sample, `sub_82772B88`).
    pub com_velocity: [f32; 3],
    /// SystemReckoning+64, the skater position (audio state +48).
    pub position: [f32; 3],
    /// Collision+0 wheel count (audio state +200 = `(R152 >> 20) & 7`).
    pub wheel_count: u32,
    /// KnownAir+176 time in state (audio state +236).
    pub air_time_in_state: f32,
    /// Air+184 time until landing (audio state +240).
    pub air_time_until_landing: f32,
    /// KnownAir+200 jump height (audio state +260).
    pub air_jump_height: f32,
    /// Player state bytes 52..87 as published by the state output (index = offset - 52).
    pub state_flags: [bool; 36],
    /// OffBoard bytes 306 (left) and 307 (right).
    pub offboard_feet: [bool; 2],
    /// Air FootPlantManager bytes 449 (left) and 450 (right).
    pub footplant: [bool; 2],
    /// Interaction+0 `AudibleFootStepStrength` (audio state +796).
    pub footstep_strength: f32,

    /// State+12 physical category (`physical.state.category_12`).
    pub state_category: u32,
    /// State+16 physical state id (`physical.state.state_16`).
    pub state: u32,
    /// Filtered+0 (`[B+64]+0`, `physical.filtered_state_0`); 7 = OffboardAir.
    pub filtered_state: u32,
    /// Air byte438, written by ProcessOutput `sub_82DB6EC0` (82DB7620..764C): `200 <= State+16 < 300 &&
    /// Collision+0 == 0`. The board FillPhysOut `82C02A80` has already run in that
    /// ProcessOutput, so this is the current tick's wheel count.
    pub in_known_air: bool,
    /// Grinds byte316 (`grinds.grinding_316`).
    pub grinding: bool,
    /// Grinds+136 physical grind family (`grinds.words_136_140[0]`).
    pub grind_family: u32,
    /// Grinds+216 grind audio material (`grinds.audio_surface_216`).
    pub grind_audio_surface: u32,
    /// Grinds+128 grind impact speed (`grinds.impact_speed_128`).
    pub grind_impact_speed: f32,
    /// Grinds byte323 (`grinds.flag_323`).
    pub grind_flag_323: bool,
    /// Air byte440 = Processed2468 bit 22 (ProcessOutput `sub_82DB6EC0`).
    pub air_440: bool,
    /// Air+112 jump velocity delta (`air.jump_velocity_delta_112`).
    pub jump_velocity_delta: [f32; 3],
    /// Air byte448 (`air.flag_448`): the foot materials come from Air+224 instead of OffBoard.
    pub footplant_448: bool,
    /// Air+224 footplant surface tag (`air.footplant_surface_224`).
    pub footplant_surface: u32,
    /// OffBoard+52 and +56. ProcessOutput `sub_82DB6EC0` stores Processed+2596 into both.
    pub offboard_surface: u32,
    /// OffBoard bytes 309 / 310: ProcessOutput `sub_82DB6EC0` stores Processed2480 bits 18
    /// and 8.
    pub offboard_309: bool,
    pub offboard_310: bool,
    /// OffBoard byte311, the board possession owner's held flag (`off_board.flag_311`).
    pub offboard_311: bool,
    /// Skeleton byte599 end of bail (`skeleton.over_599`).
    pub skeleton_599: bool,
    /// Skeleton bytes 600/601, feet inside the deck box (`82BF22A0`).
    pub feet_in_deck_box: [bool; 2],
    /// Skeleton+192 / +208: local toe velocities (`82BF22A0`, `foot_physical.output`).
    pub foot_local_velocity: [[f32; 3]; 2],
    /// Skeleton+224 / +240: world foot velocities (`82BF22A0`).
    pub foot_world_velocity: [[f32; 3]; 2],
    /// Collision bytes 3296..3299: per-wheel contact (`82C02A80` from CollisionInfo+844).
    pub wheel_contacts: [bool; 4],
    /// Collision bytes 3473 / 3474: front / back truck contact (CollisionInfo+848/+849).
    pub truck_contacts: [bool; 2],
    /// Collision byte3475: deck contact (CollisionInfo+850).
    pub deck_contact: bool,
    /// Collision+3376+16i: the wheel's contact normal, zero when the wheel is not in contact
    /// (`82C02A80`, loc_82C02D9C..2DA8 for wheel 0).
    pub wheel_contact_normals: [[f32; 3]; 4],
    /// Collision+3440+4i: wheel audio material, `tag & 0x7F` (0 = no hit).
    pub wheel_audio_surfaces: [u32; 4],
    /// Collision+3456+4i: wheel seam pattern, `(tag >> 12) & 0xF`.
    pub wheel_seam_patterns: [u32; 4],
    /// Collision+4/+8/+12: front truck / back truck / deck audio material (0 = no report).
    pub part_audio_surfaces: [u32; 3],
    /// Collision+20: deck slide speed (`82C02A80` 82C034E8..3570).
    pub deck_slide_speed: f32,
    /// Collision+24: deck scrape (`82C02A80` 82C03478..34D8).
    pub deck_scrape: f32,
    /// SkateboardMotion+64 / +80: deck angular / linear velocity.
    pub angular_velocity: [f32; 3],
    pub linear_velocity: [f32; 3],
    /// SkateboardMotion+184 = SkateboardBody+256, the filtered signed deck tilt.
    pub deck_tilt: f32,
    /// SkateboardMotion+200 = Processed+2764.
    pub motion_200: f32,
    /// SkateboardMotion+208+16i: wheel body linear velocities.
    pub wheel_velocities: [[f32; 3]; 4],
    /// PhysOut slot B+0, +0/+16/+32: rows of the deck part transform (`82585CB0`, part 6).
    pub deck_rows: [[f32; 3]; 3],
    /// PhysOut slot B+0, +80: row 1 of the effective deck transform (`82C01BF8`).
    pub effective_deck_up: [f32; 3],
    /// PhysOut slot B+0, +208+16i: wheel body positions (audio state +384+16i).
    pub wheel_positions: [[f32; 3]; 4],
    /// The camera the listener `*(0x830CFDD4)` follows (`sub_8248CC08`): its world matrix row 2
    /// (native At, the view direction) and row 3 (position). `None` before the first camera frame.
    pub camera: Option<([f32; 3], [f32; 3])>,
    /// The deck position and effective forward axis, standing in for the board PhysOut record's
    /// `+144` / `+128` that the second position controller (`60010020`) is bound to. UNVERIFIED:
    /// the writer of B+0 +128/+144 was not traced.
    pub deck_position: [f32; 3],
    pub deck_forward: [f32; 3],
    /// The score module's combo multiplier (`sub_82DA4238` stores it at the score output +56;
    /// `sub_827A2E88` turns it into the frame record's x1.5/x2/x3 tier flags).
    pub combo_multiplier: f32,
    /// Gameplay is paused (the pause menu). Retail keeps the bridge, the components and the voices
    /// running and ducks the mix through SFXObj_Pause (`sub_824E1D00`) input 0 instead.
    pub paused: bool,
    /// Ground+80: the wheel-contact normal (`ground.vector_80`).
    pub ground_normal: [f32; 3],
    /// Ground+264 = Processed+2676, the turn attribute.
    pub turn: f32,
    /// Ground+300 = Processed+2624, the jump strength attribute.
    pub jump_strength: f32,
    /// PhysOutScoring2 (B+60)+152: the EScorableID `sub_82DAC498` looks up (`sub_82DA5AC8`)
    /// while the score packet carries flag bit 24 or 25, else -1.
    pub scorable_id: i32,
    /// Skeleton+288 / +304 / +320: body+48 (angular velocity) of ragdoll parts 23 / 20 / 16,
    /// copied by Skeleton Fill `sub_82BE1AE8` from `[[[SkeletonState+6448]+24]+8]` +2284 /
    /// +1996 / +1612 (part record +76 → body).
    pub ragdoll_spin: [[f32; 3]; 3],
    /// Skeleton+560 / +564 / +568 / +572 (`sub_82BE1AE8`): |physical-record velocity of parts
    /// 17 / 21 / 4 / 8 (SkeletonState+10368 / +10432 / +10160 / +10224) − the COM velocity
    /// (SkeletonState+16176 = Reckoning+16)|.
    pub limb_speeds: [f32; 4],
    /// Skeleton+144 / +160 (`sub_82BE1AE8`): translations of physical parts 15 / 19
    /// (SkeletonState+9024 / +9280, the physical pose at +8016).
    pub toe_positions: [[f32; 3]; 2],
    /// Collision+80..+195, the ragdoll contact publication `sub_82BD60C8`.
    pub body_contacts: BodyContacts,
    /// Skeleton+516 = Pumping+56 absorption (−speed × angular speed), stored by ProcessOutput
    /// `sub_82DB6EC0` from `[player+1820]+56` right after Skeleton Fill (audio state +712).
    pub pump_absorption: f32,
}

/// Collision+80..+195 as the ragdoll contact publisher `sub_82BD60C8` writes them from the
/// SkeletonCollision object (`[SkeletonState+6448]+160`, updated by `sub_82BD4A30`). Slot i is
/// contact region i (table `0x820CFCB0`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyContacts {
    /// Region part ≠ −1 (SkeletonCollision+1200+4i).
    pub contact: [bool; 8],
    /// |physical-record velocity change of the part (record+4048+16p) · region normal
    /// (SkeletonCollision+1008+16i)| × part weight (record+4560+4p), 0 without a contact. The
    /// worker finishes Collision+80+4i = clamp(max(this × K164, 0.001), 0, 1) with the vault's
    /// physics_collision layout +164 (`sub_82BD60C8` 82BD68F0 loop).
    pub weighted_change: [f32; 8],
    /// Collision+112+4i: the region's tangential speed (SkeletonCollision+944+4i), 0 without.
    pub slide: [f32; 8],
    /// Collision+144+4i: the region's material, `tag & 0x7F` (SkeletonCollision+976+4i),
    /// written only with a contact; the PhysOut reset leaves 0 otherwise.
    pub material: [u32; 8],
    /// Collision bytes 176 / 177: the groin (part 23) / face (part 1) specific contact is current
    /// this frame (SkeletonCollision bytes 4008 / 4009).
    pub specific_current: [bool; 2],
    /// Collision+180 / +184 / +188 / +192: SkeletonCollision+4056 (maximum group-8 force),
    /// +4048 (maximum skater force), +4052 (other skater, −1 none), +4060 (maximum group-11
    /// force).
    pub group_8_force: f32,
    pub skater_force: f32,
    pub other_skater: i32,
    pub group_11_force: f32,
}

impl Default for RetailAudioInputs {
    fn default() -> Self {
        Self {
            camera: None,
            deck_position: [0.0; 3],
            deck_forward: [0.0; 3],
            combo_multiplier: 1.0,
            paused: false,
            dt: 0.0,
            ground_speed: 0.0,
            com_velocity: [0.0; 3],
            position: [0.0; 3],
            wheel_count: 0,
            air_time_in_state: 0.0,
            air_time_until_landing: 0.0,
            air_jump_height: 0.0,
            state_flags: [false; 36],
            offboard_feet: [false; 2],
            footplant: [false; 2],
            footstep_strength: 0.0,
            state_category: 0,
            state: 0,
            filtered_state: 0,
            in_known_air: false,
            grinding: false,
            grind_family: u32::MAX,
            grind_audio_surface: 0,
            grind_impact_speed: 0.0,
            grind_flag_323: false,
            air_440: false,
            jump_velocity_delta: [0.0; 3],
            footplant_448: false,
            footplant_surface: 0,
            offboard_surface: 0,
            offboard_309: false,
            offboard_310: false,
            offboard_311: false,
            skeleton_599: false,
            feet_in_deck_box: [false; 2],
            foot_local_velocity: [[0.0; 3]; 2],
            foot_world_velocity: [[0.0; 3]; 2],
            wheel_contacts: [false; 4],
            truck_contacts: [false; 2],
            deck_contact: false,
            wheel_contact_normals: [[0.0; 3]; 4],
            wheel_audio_surfaces: [0; 4],
            wheel_seam_patterns: [0; 4],
            part_audio_surfaces: [0; 3],
            deck_slide_speed: 0.0,
            deck_scrape: 0.0,
            angular_velocity: [0.0; 3],
            linear_velocity: [0.0; 3],
            deck_tilt: 0.0,
            motion_200: 0.0,
            wheel_velocities: [[0.0; 3]; 4],
            deck_rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            effective_deck_up: [0.0, 1.0, 0.0],
            wheel_positions: [[0.0; 3]; 4],
            ground_normal: [0.0, 1.0, 0.0],
            turn: 0.0,
            jump_strength: 0.0,
            scorable_id: -1,
            ragdoll_spin: [[0.0; 3]; 3],
            limb_speeds: [0.0; 4],
            toe_positions: [[0.0; 3]; 2],
            body_contacts: BodyContacts {
                other_skater: -1,
                ..BodyContacts::default()
            },
            pump_absorption: 0.0,
        }
    }
}

/// Decoded interleaved 16-bit PCM, ready to hand to the mixer.
#[derive(Asset, TypePath, Clone)]
pub struct StreamPcm {
    samples: Arc<Vec<i16>>,
    channels: u16,
    sample_rate: u32,
    /// Frame interval to repeat for an inline EAAC loop. Archive streams without an inline loop
    /// continue to use Bevy's whole-source looping setting.
    loop_range: Option<(usize, usize)>,
}

impl StreamPcm {
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        }
    }

    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.frames() as f64 / f64::from(self.sample_rate))
    }

    /// A short mono sine wave for verifying the host mixer and output device without game data.
    ///
    /// This deliberately lives beside decoded streams so the diagnostic exercises the exact same
    /// Bevy source, mixer and Windows device backend as retail audio does.
    fn tone(hertz: f32, duration: Duration) -> Self {
        const RATE: u32 = 48_000;
        let frames = (duration.as_secs_f64() * f64::from(RATE)).round() as usize;
        let angular_step = std::f32::consts::TAU * hertz / RATE as f32;
        let samples = (0..frames)
            // Keep headroom so the device test is not itself a clipping test.
            .map(|frame| (angular_step * frame as f32).sin() * 0.20 * i16::MAX as f32)
            .map(|sample| sample.round() as i16)
            .collect();
        Self {
            samples: Arc::new(samples),
            channels: 1,
            sample_rate: RATE,
            loop_range: None,
        }
    }
}

/// Plays a [`StreamPcm`] once, converting to the float samples the mixer wants.
pub struct PcmPlayback {
    samples: Arc<Vec<i16>>,
    at: usize,
    channels: u16,
    sample_rate: u32,
    loop_range: Option<(usize, usize)>,
}

impl Iterator for PcmPlayback {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some((start, end)) = self.loop_range {
            let channels = usize::from(self.channels);
            if self.at == end.saturating_mul(channels) {
                self.at = start.saturating_mul(channels);
            }
        }
        let s = *self.samples.get(self.at)?;
        self.at += 1;
        // i16::MIN has no positive counterpart, so dividing by 32768 keeps the full range inside
        // [-1, 1] instead of letting the one extreme sample clip a fraction above it.
        Some(f32::from(s) / 32768.0)
    }
}

impl Source for PcmPlayback {
    fn current_frame_len(&self) -> Option<usize> {
        self.loop_range
            .is_none()
            .then(|| self.samples.len().saturating_sub(self.at))
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        if self.loop_range.is_some() {
            return None;
        }
        let frames = if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        };
        (self.sample_rate != 0)
            .then(|| Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate)))
    }
}

impl Decodable for StreamPcm {
    type DecoderItem = f32;
    type Decoder = PcmPlayback;

    fn decoder(&self) -> PcmPlayback {
        PcmPlayback {
            samples: self.samples.clone(),
            at: 0,
            channels: self.channels,
            sample_rate: self.sample_rate,
            loop_range: self.loop_range,
        }
    }
}

/// A bounded, continuous float source for the recovered audio graph.
///
/// The audio thread never waits for the game thread. When a graph block has not arrived yet (or a
/// producer momentarily owns the queue), its decoder supplies silence and remains live for the
/// next block. Keeping the queue bounded also prevents a stalled output device from turning into
/// delayed gameplay audio.
#[derive(Asset, TypePath, Clone)]
pub struct LivePcm {
    samples: Arc<Mutex<VecDeque<f32>>>,
    stats: Arc<LivePcmStats>,
    channels: u16,
    sample_rate: u32,
    capacity: usize,
}

impl LivePcm {
    /// Hold at most `capacity_frames` rendered frames. A zero channel count, rate, or capacity is
    /// not a useful continuous audio source and is rejected at construction.
    pub fn new(channels: u16, sample_rate: u32, capacity_frames: usize) -> Result<Self, String> {
        if channels == 0 || sample_rate == 0 || capacity_frames == 0 {
            return Err("live PCM needs nonzero channels, sample rate, and capacity".into());
        }
        Ok(Self {
            samples: Arc::new(Mutex::new(VecDeque::with_capacity(
                capacity_frames.saturating_mul(usize::from(channels)),
            ))),
            stats: Arc::new(LivePcmStats::default()),
            channels,
            sample_rate,
            capacity: capacity_frames.saturating_mul(usize::from(channels)),
        })
    }

    /// Append complete interleaved frames. If the renderer gets ahead of output, discard the
    /// oldest complete frames so current gameplay remains current. The returned count is the
    /// number of samples discarded from the front.
    pub fn push(&self, samples: &[f32]) -> Result<usize, String> {
        if samples.len() % usize::from(self.channels) != 0 {
            return Err("live PCM input does not end on a frame boundary".into());
        }
        if samples.iter().any(|value| !value.is_finite()) {
            return Err("live PCM contains a non-finite sample".into());
        }
        let mut queue = self
            .samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let overflow = queue
            .len()
            .saturating_add(samples.len())
            .saturating_sub(self.capacity);
        let dropped = overflow.min(queue.len());
        for _ in 0..dropped {
            queue.pop_front();
        }
        // A block can be larger than this source's whole latency budget. Keep its newest complete
        // frames for the same reason as normal queue overflow.
        let keep = samples.len().min(self.capacity);
        let skip = samples.len() - keep;
        queue.extend(samples[skip..].iter().copied());
        self.stats.pushed.fetch_add(keep as u64, Ordering::Relaxed);
        Ok(dropped + skip)
    }

    /// Frames waiting in the shared queue, excluding the decoder's current small block.
    pub fn queued_frames(&self) -> usize {
        self.samples.lock().unwrap_or_else(|p| p.into_inner()).len() / usize::from(self.channels)
    }

    pub fn clear(&self) {
        self.samples
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    pub fn snapshot(&self) -> LivePcmSnapshot {
        LivePcmSnapshot {
            pushed: self.stats.pushed.load(Ordering::Relaxed),
            consumed: self.stats.consumed.load(Ordering::Relaxed),
            consumed_nonzero: self.stats.consumed_nonzero.load(Ordering::Relaxed),
            underflow: self.stats.underflow.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct LivePcmStats {
    pushed: AtomicU64,
    consumed: AtomicU64,
    consumed_nonzero: AtomicU64,
    underflow: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LivePcmSnapshot {
    pub pushed: u64,
    pub consumed: u64,
    pub consumed_nonzero: u64,
    pub underflow: u64,
}

/// The mixer-facing decoder for [`LivePcm`].
pub struct LivePlayback {
    samples: Arc<Mutex<VecDeque<f32>>>,
    stats: Arc<LivePcmStats>,
    channels: u16,
    sample_rate: u32,
    block: VecDeque<f32>,
}

impl Iterator for LivePlayback {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.block.is_empty() {
            // Transfer only whole frames. Locking once per sample could insert silence in the
            // middle of a stereo frame, permanently swapping L/R after contention or underflow.
            if let Ok(mut queue) = self.samples.try_lock() {
                let count = queue.len().min(256 * usize::from(self.channels));
                self.block.extend(queue.drain(..count));
            }
            if self.block.is_empty() {
                self.block.resize(usize::from(self.channels), 0.0);
                self.stats
                    .underflow
                    .fetch_add(u64::from(self.channels), Ordering::Relaxed);
            }
        }
        let sample = self.block.pop_front();
        if let Some(value) = sample {
            self.stats.consumed.fetch_add(1, Ordering::Relaxed);
            if value.abs() > 1.0e-8 {
                self.stats.consumed_nonzero.fetch_add(1, Ordering::Relaxed);
            }
        }
        sample
    }
}

impl Source for LivePlayback {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for LivePcm {
    type DecoderItem = f32;
    type Decoder = LivePlayback;

    fn decoder(&self) -> LivePlayback {
        LivePlayback {
            samples: self.samples.clone(),
            stats: self.stats.clone(),
            channels: self.channels,
            sample_rate: self.sample_rate,
            block: VecDeque::with_capacity(256 * usize::from(self.channels)),
        }
    }
}

/// Where one stream lives, and what it takes to read it.
///
/// The ambience and grain archives store bare block chains, so `channels` and `sample_rate` are
/// part of the request: they come from the metadata table, not from the audio.
#[derive(Clone, Debug)]
pub struct StreamRequest {
    pub archive: PathBuf,
    /// A member name, or its index in the entry table as a decimal string.
    pub entry: String,
    pub channels: u8,
    pub sample_rate: u32,
    /// Stop after this many blocks; 0 decodes the whole member.
    pub blocks: usize,
    pub volume: f32,
    /// Ambience beds run about two minutes and are meant to run under everything, so they repeat.
    pub looping: bool,
}

impl StreamRequest {
    pub fn new(
        archive: impl Into<PathBuf>,
        entry: impl Into<String>,
        channels: u8,
        sample_rate: u32,
    ) -> Self {
        Self {
            archive: archive.into(),
            entry: entry.into(),
            channels,
            sample_rate,
            blocks: 0,
            volume: 1.0,
            looping: false,
        }
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    pub fn blocks(mut self, blocks: usize) -> Self {
        self.blocks = blocks;
        self
    }

    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
}

/// Ask for a stream to be decoded and played once.
#[derive(Message, Clone, Debug)]
pub struct PlayStream(pub StreamRequest);

/// Decode one inline EAAC sample from an `.abk` member of an owned archive.
///
/// This is an opt-in audition mechanism for comparing individual rider-and-board sound variants.
/// It does not evaluate a bank program or map gameplay events to samples. An authored inline loop
/// repeats from its recorded loop frame, instead of repeating the whole decoded waveform.
#[derive(Message, Clone, Debug)]
pub struct PlayBankSample(pub BankSampleRequest);

#[derive(Clone, Debug)]
pub struct BankSampleRequest {
    pub archive: PathBuf,
    pub bank: String,
    pub sample: usize,
    pub volume: f32,
}

impl BankSampleRequest {
    pub fn new(archive: impl Into<PathBuf>, bank: impl Into<String>, sample: usize) -> Self {
        Self {
            archive: archive.into(),
            bank: bank.into(),
            sample,
            volume: 1.0,
        }
    }

    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
}

/// Ask for the ambience bed that suits a place, if the archive has one for it.
///
/// Resolution is by member name (`skate_data::audio::ambience`), which is a stand-in for the
/// undecoded audio metadata, so a place with no matching district plays nothing at all rather
/// than something plausible.
#[derive(Message, Clone, Debug)]
pub struct PlayAmbience {
    /// The archive of `.snr` headers, e.g. `ambienceresident.big`.
    pub resident: PathBuf,
    /// The archive of `.sns` payloads, e.g. `ambience.big`.
    pub payload: PathBuf,
    pub place: String,
    pub volume: f32,
}

/// Resolve a place to a bed, across the pair of archives a bed is stored in.
///
/// Ambience is **split in two**: `ambienceresident.big` holds one `.snr` header record per bed,
/// and `ambience.big` holds the matching `.sns` block chain, paired by the name's stem. Reading a
/// bed out of the resident archive alone gets a header and no audio -- the block walk then reads
/// the next member's bytes and reports "block size 0 does not advance", which looks like a corrupt
/// archive and is really the wrong file.
///
/// So the header supplies the channel count and rate, and the payload supplies the blocks.
pub fn resolve_ambience(
    resident: &std::path::Path,
    payload: &std::path::Path,
    place: &str,
) -> Result<StreamRequest, String> {
    let header_data =
        std::fs::read(resident).map_err(|e| format!("{}: {e}", resident.display()))?;
    let described = audio::describe_archive(&header_data).map_err(|e| e.to_string())?;
    let beds: Vec<audio::ambience::Bed> = described
        .iter()
        .filter_map(|(name, _)| audio::ambience::parse_bed(name))
        .collect();
    if beds.is_empty() {
        return Err(format!(
            "{} holds no named ambience beds",
            resident.display()
        ));
    }
    let bed = audio::ambience::pick(place, &beds).ok_or_else(|| {
        format!(
            "no bed matches {place:?} among {} in the archive",
            beds.len()
        )
    })?;
    let info = described
        .iter()
        .find(|(name, _)| *name == bed.member)
        .map(|(_, info)| info.clone())
        .ok_or_else(|| format!("{} has no header", bed.member))?;

    // The payload member carries the same stem with a .sns extension.
    let stem = bed.member.strip_suffix(".snr").unwrap_or(&bed.member);
    let entry = format!("{stem}.sns");
    let payload_data = std::fs::read(payload).map_err(|e| format!("{}: {e}", payload.display()))?;
    let archive =
        skate_audio_formats::eb::Archive::parse(&payload_data).map_err(|e| e.message.clone())?;
    if archive.find(&entry).is_none() {
        return Err(format!("{} has no member {entry}", payload.display()));
    }
    Ok(StreamRequest::new(payload, entry, info.channels, info.sample_rate).looping(true))
}

#[derive(Component)]
struct Decoding {
    task: Task<Result<StreamPcm, String>>,
    request: StreamRequest,
}

#[derive(Component)]
struct BankDecoding {
    task: Task<Result<StreamPcm, String>>,
    request: BankSampleRequest,
}

pub struct SkateAudioPlugin;

impl Plugin for SkateAudioPlugin {
    fn build(&self, app: &mut App) {
        player::install(app);
        app.add_audio_source::<StreamPcm>()
            .add_audio_source::<LivePcm>()
            .add_message::<PlayStream>()
            .add_message::<PlayBankSample>()
            .add_message::<PlayAmbience>()
            .add_message::<PlayerAudioObservation>()
            .add_systems(
                Startup,
                (
                    play_requested_at_startup,
                    play_bank_sample_at_startup,
                    play_tone_at_startup,
                ),
            )
            .add_systems(
                Update,
                (
                    resolve_ambience_requests,
                    start_decoding,
                    start_bank_decoding,
                    finish_decoding,
                    finish_bank_decoding,
                ),
            );
    }
}

/// `SKATE_AUDIO_PLAY=<archive>:<entry>:<channels>:<rate>[:<blocks>]` plays one stream at
/// startup. It exists so the decode path can be *heard* rather than only measured, on a build
/// that has no automatic ambience yet: which stream belongs to which map lives in the metadata
/// table, and that table is not decoded.
fn play_requested_at_startup(mut requests: MessageWriter<PlayStream>) {
    let Ok(spec) = std::env::var("SKATE_AUDIO_PLAY") else {
        return;
    };
    match parse_spec(&spec) {
        Ok(request) => {
            info!(
                "skate-audio: playing {} entry {} on request",
                request.archive.display(),
                request.entry
            );
            requests.write(PlayStream(request));
        }
        Err(e) => warn!("skate-audio: SKATE_AUDIO_PLAY={spec}: {e}"),
    }
}

/// `SKATE_AUDIO_BANK_SAMPLE=<archive>|<bank.abk>|<sample>` auditions one bank sample at startup.
///
/// Pipes leave Windows drive-letter paths intact. The requested bank must be an uncompressed
/// member of the owned archive; the parser rejects an absent sample slot and never reads into the
/// next member.
fn play_bank_sample_at_startup(mut requests: MessageWriter<PlayBankSample>) {
    let Ok(spec) = std::env::var("SKATE_AUDIO_BANK_SAMPLE") else {
        return;
    };
    match parse_bank_sample_spec(&spec) {
        Ok(request) => {
            info!(
                "skate-audio: auditioning {} sample {} from {}",
                request.bank,
                request.sample,
                request.archive.display()
            );
            requests.write(PlayBankSample(request));
        }
        Err(e) => warn!("skate-audio: SKATE_AUDIO_BANK_SAMPLE={spec}: {e}"),
    }
}

/// `SKATE_AUDIO_TONE[=<hertz>]` plays a one-second test tone without an archive or decoder.
///
/// It is deliberately opt-in: it is a Windows output-device diagnostic, not game content. A
/// malformed value keeps the conventional 440 Hz tone and reports the issue rather than making a
/// silent diagnostic run look successful.
fn play_tone_at_startup(mut commands: Commands, mut assets: ResMut<Assets<StreamPcm>>) {
    let Ok(value) = std::env::var("SKATE_AUDIO_TONE") else {
        return;
    };
    let hertz = match value.parse::<f32>() {
        Ok(hertz) if hertz.is_finite() && hertz > 0.0 => hertz,
        Ok(_) | Err(_) if value.is_empty() || value == "1" => 440.0,
        Ok(_) | Err(_) => {
            warn!(
                "skate-audio: SKATE_AUDIO_TONE={value:?} is not a positive frequency; using 440 Hz"
            );
            440.0
        }
    };
    info!("skate-audio: playing {hertz:.1} Hz output-device test tone");
    let handle = assets.add(StreamPcm::tone(hertz, Duration::from_secs(1)));
    commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN));
}

/// Parse `archive:entry:channels:rate[:blocks]`. The archive path is taken from the left, so a
/// Windows drive letter or any other colon inside the path would need the fields reordered; this
/// is a debug hook and says so rather than pretending to be a general parser.
fn parse_spec(spec: &str) -> Result<StreamRequest, String> {
    let parts: Vec<&str> = spec.rsplitn(5, ':').collect();
    // rsplitn yields the tail first, so the fields come back reversed.
    let (archive, entry, channels, rate, blocks) = match parts.len() {
        4 => (parts[3], parts[2], parts[1], parts[0], "0"),
        5 => (parts[4], parts[3], parts[2], parts[1], parts[0]),
        _ => return Err("expected archive:entry:channels:rate[:blocks]".into()),
    };
    Ok(StreamRequest::new(
        archive,
        entry,
        channels
            .parse::<u8>()
            .map_err(|e| format!("channels: {e}"))?,
        rate.parse::<u32>().map_err(|e| format!("rate: {e}"))?,
    )
    .blocks(
        blocks
            .parse::<usize>()
            .map_err(|e| format!("blocks: {e}"))?,
    ))
}

/// Turn a place into a stream request, or say why it could not be.
fn resolve_ambience_requests(
    mut asked: MessageReader<PlayAmbience>,
    mut streams: MessageWriter<PlayStream>,
) {
    for ask in asked.read() {
        match resolve_ambience(&ask.resident, &ask.payload, &ask.place) {
            Ok(request) => {
                info!(
                    "skate-audio: ambience for {:?}: {}",
                    ask.place, request.entry
                );
                streams.write(PlayStream(request.volume(ask.volume)));
            }
            // Not an error worth stopping for: a map with no bed simply has no ambience yet.
            Err(e) => info!("skate-audio: no ambience for {:?}: {e}", ask.place),
        }
    }
}

fn start_decoding(mut commands: Commands, mut requests: MessageReader<PlayStream>) {
    for PlayStream(request) in requests.read() {
        let job = request.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move { decode(&job) });
        commands.spawn(Decoding {
            task,
            request: request.clone(),
        });
    }
}

fn start_bank_decoding(mut commands: Commands, mut requests: MessageReader<PlayBankSample>) {
    for PlayBankSample(request) in requests.read() {
        let job = request.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move { decode_bank_sample(&job) });
        commands.spawn(BankDecoding {
            task,
            request: request.clone(),
        });
    }
}

fn finish_decoding(
    mut commands: Commands,
    mut assets: ResMut<Assets<StreamPcm>>,
    mut pending: Query<(Entity, &mut Decoding)>,
) {
    for (entity, mut job) in &mut pending {
        let Some(result) = block_on(future::poll_once(&mut job.task)) else {
            continue;
        };
        commands.entity(entity).despawn();
        match result {
            Ok(pcm) => {
                info!(
                    "skate-audio: {} entry {} decoded, {} frames of {} channels ({:.2} s)",
                    job.request.archive.display(),
                    job.request.entry,
                    pcm.frames(),
                    pcm.channels,
                    pcm.duration().as_secs_f64()
                );
                let volume = job.request.volume;
                let handle = assets.add(pcm);
                let settings = if job.request.looping {
                    PlaybackSettings::LOOP
                } else {
                    PlaybackSettings::DESPAWN
                };
                commands.spawn((
                    AudioPlayer(handle),
                    settings.with_volume(Volume::Linear(volume)),
                ));
            }
            Err(e) => warn!(
                "skate-audio: {} entry {}: {e}",
                job.request.archive.display(),
                job.request.entry
            ),
        }
    }
}

fn finish_bank_decoding(
    mut commands: Commands,
    mut assets: ResMut<Assets<StreamPcm>>,
    mut pending: Query<(Entity, &mut BankDecoding)>,
) {
    for (entity, mut job) in &mut pending {
        let Some(result) = block_on(future::poll_once(&mut job.task)) else {
            continue;
        };
        commands.entity(entity).despawn();
        match result {
            Ok(pcm) => {
                info!(
                    "skate-audio: {} sample {} decoded, {} frames of {} channels ({:.2} s)",
                    job.request.bank,
                    job.request.sample,
                    pcm.frames(),
                    pcm.channels,
                    pcm.duration().as_secs_f64()
                );
                let handle = assets.add(pcm);
                commands.spawn((
                    AudioPlayer(handle),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(job.request.volume)),
                ));
            }
            Err(e) => warn!(
                "skate-audio: {} sample {}: {e}",
                job.request.bank, job.request.sample
            ),
        }
    }
}

/// Read, split and decode one stream. Runs off the schedule threads: it spawns a process.
fn decode(request: &StreamRequest) -> Result<StreamPcm, String> {
    let data = std::fs::read(&request.archive).map_err(|e| format!("{e}"))?;
    let (at, end) = locate(&data, &request.entry)?;
    decode_range(
        &data,
        at,
        end,
        request.channels,
        request.sample_rate,
        request.blocks,
    )
}

fn decode_bank_sample(request: &BankSampleRequest) -> Result<StreamPcm, String> {
    use skate_audio_formats::eb;

    let data = std::fs::read(&request.archive).map_err(|e| format!("{e}"))?;
    let archive = eb::Archive::parse(&data).map_err(|e| e.message.clone())?;
    let member = archive
        .find(&request.bank)
        .ok_or_else(|| format!("no member named {}", request.bank))?;
    if member.is_compressed() {
        return Err(format!(
            "bank {} is a chunkref block, not stored audio",
            request.bank
        ));
    }
    let member_range = member.range();
    let bank_bytes = data
        .get(member_range.clone())
        .ok_or_else(|| format!("bank {} runs past archive end", request.bank))?;
    let bank = banks::Abk::parse(bank_bytes).map_err(|e| e.message)?;
    let sample_range = bank
        .sample_range(request.sample)
        .ok_or_else(|| format!("bank {} has no sample {}", request.bank, request.sample))?;
    let at = member_range.start + sample_range.start;
    let end = member_range.start + sample_range.end;
    decode_range(&data, at, end, 0, 0, 0)
}

/// Decode an EAAC chain whose enclosing range is already known. Keeping `end` explicit prevents
/// the final block of a bank sample from consuming the next sample's header as payload.
fn decode_range(
    data: &[u8],
    at: usize,
    end: usize,
    fallback_channels: u8,
    fallback_rate: u32,
    blocks_limit: usize,
) -> Result<StreamPcm, String> {
    if at >= end || end > data.len() {
        return Err("audio member has an invalid byte range".into());
    }
    // A member may carry its own stream header -- the named wheel and grain sounds do -- or be a
    // bare block chain whose format lives in the metadata, as the ambience beds are. Prefer the
    // header when there is one, and skip it: its low 24 bits are the sample rate, so reading it
    // as a block header looks like a 48,000-byte block and fails far from the real mistake.
    // Keep the header parser inside the member/sample boundary too: a malformed looping header
    // must not borrow its loop word from the next bank sample.
    let header = eaac::Header::parse(&data[..end], at).ok();
    let (chain, channels, rate) = match header {
        Some(header) => (at + header.size(), header.channels(), header.sample_rate),
        None => (at, fallback_channels, fallback_rate),
    };
    let widths = audio::context_widths(channels);
    let contexts = widths.len();
    if contexts == 0 {
        return Err("a stream with no channels".into());
    }

    let mut chains: Vec<Vec<Vec<u8>>> = vec![Vec::new(); contexts];
    let mut seen = 0usize;
    for block in eaac::blocks(&data[chain..end]).map_err(|e| e.message.clone())? {
        if blocks_limit != 0 && seen >= blocks_limit {
            break;
        }
        let range = block.data_range();
        let payload = &data[chain + range.start..chain + range.end];
        for (context, chunk) in eaac::split_block(payload, contexts)
            .map_err(|e| e.message.clone())?
            .into_iter()
            .enumerate()
        {
            chains[context].push(chunk.data.to_vec());
        }
        seen += 1;
    }

    let mut per_context = Vec::with_capacity(contexts);
    for (context, chunks) in chains.iter().enumerate() {
        per_context.push(
            audio::ffmpeg::decode_chain(chunks, widths[context], rate).map_err(|e| e.message)?,
        );
    }
    let samples = audio::interleave_contexts(&per_context, &widths);
    let frames = samples.len() / usize::from(channels);
    let loop_range = header.and_then(|header| {
        header.loop_start.map(|start| {
            let start = start as usize;
            (start, frames)
        })
    });
    if let Some((start, loop_end)) = loop_range
        && start >= loop_end
    {
        return Err(format!(
            "decoded loop start {start} is outside its {loop_end}-frame PCM"
        ));
    }
    Ok(StreamPcm {
        samples: Arc::new(samples),
        channels: u16::from(channels),
        sample_rate: rate,
        loop_range,
    })
}

fn parse_bank_sample_spec(spec: &str) -> Result<BankSampleRequest, String> {
    let mut fields = spec.rsplitn(3, '|');
    let sample = fields
        .next()
        .ok_or("expected archive|bank.abk|sample")?
        .parse::<usize>()
        .map_err(|e| format!("sample: {e}"))?;
    let bank = fields.next().ok_or("expected archive|bank.abk|sample")?;
    let archive = fields.next().ok_or("expected archive|bank.abk|sample")?;
    if archive.is_empty() || bank.is_empty() {
        return Err("archive and bank must not be empty".into());
    }
    Ok(BankSampleRequest::new(archive, bank, sample))
}

/// Resolve a member to a byte range. The range matters as much as the offset: a block walk that
/// runs past the member reads the next one's first bytes as a block header.
fn locate(data: &[u8], entry: &str) -> Result<(usize, usize), String> {
    use skate_audio_formats::eb;
    let archive = eb::Archive::parse(data).map_err(|e| e.message.clone())?;
    let member = match entry.parse::<usize>() {
        Ok(index) => archive
            .entries
            .get(index)
            .ok_or_else(|| format!("entry {index}: the archive has {}", archive.entries.len()))?,
        Err(_) => archive
            .find(entry)
            .ok_or_else(|| format!("no member named {entry}"))?,
    };
    if member.is_compressed() {
        return Err(format!(
            "member {entry} is a chunkref block, not stored audio"
        ));
    }
    let range = member.range();
    Ok((range.start, range.end.min(data.len())))
}

/// Is a decoder present? A host without one gets synthesized audio only.
pub fn decoder_available() -> bool {
    FfmpegDecoder::available()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_converts_full_scale_without_exceeding_unity() {
        // i16::MIN / 32767 would be 1.000031, which clips on a mixer that trusts the range.
        let pcm = StreamPcm {
            samples: Arc::new(vec![i16::MIN, i16::MAX, 0]),
            channels: 1,
            sample_rate: 48_000,
            loop_range: None,
        };
        let got: Vec<f32> = pcm.decoder().collect();
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|s| (-1.0..=1.0).contains(s)), "{got:?}");
        assert_eq!(got[0], -1.0);
        assert_eq!(got[2], 0.0);
    }

    #[test]
    fn live_pcm_is_continuous_and_keeps_the_newest_complete_frames() {
        let pcm = LivePcm::new(2, 48_000, 2).unwrap();
        let mut playback = pcm.decoder();
        assert_eq!(playback.next(), Some(0.0), "an empty live block is silence");
        assert_eq!(
            playback.next(),
            Some(0.0),
            "silence completes the stereo frame"
        );
        assert_eq!(playback.current_frame_len(), None);
        assert_eq!(playback.total_duration(), None);
        assert_eq!(pcm.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap(), 2);
        assert_eq!(
            [
                playback.next(),
                playback.next(),
                playback.next(),
                playback.next()
            ],
            [Some(3.0), Some(4.0), Some(5.0), Some(6.0)]
        );
        assert_eq!(playback.next(), Some(0.0));
        assert!(pcm.push(&[1.0]).is_err(), "partial frames are refused");
        assert!(pcm.push(&[f32::NAN, 0.0]).is_err());
    }

    #[test]
    fn live_pcm_underflow_cannot_swap_stereo_channels() {
        let pcm = LivePcm::new(2, 48_000, 2).unwrap();
        let mut playback = pcm.decoder();
        assert_eq!(playback.next(), Some(0.0));
        pcm.push(&[0.25, -0.5]).unwrap();
        assert_eq!(playback.next(), Some(0.0));
        assert_eq!(playback.next(), Some(0.25));
        assert_eq!(playback.next(), Some(-0.5));
    }

    #[test]
    fn duration_counts_frames_not_samples() {
        // Five channels at 48 kHz: 240,000 samples are one second, not five.
        let pcm = StreamPcm {
            samples: Arc::new(vec![0i16; 48_000 * 5]),
            channels: 5,
            sample_rate: 48_000,
            loop_range: None,
        };
        assert_eq!(pcm.frames(), 48_000);
        assert_eq!(pcm.duration(), Duration::from_secs(1));
        assert_eq!(pcm.decoder().total_duration(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn inline_loop_restarts_at_its_authored_frame_not_at_zero() {
        let pcm = StreamPcm {
            samples: Arc::new(vec![100i16, 200, 300]),
            channels: 1,
            sample_rate: 48_000,
            loop_range: Some((1, 3)),
        };
        let mut playback = pcm.decoder();
        let got: Vec<i16> = (0..6)
            .map(|_| (playback.next().unwrap() * 32767.0).round() as i16)
            .collect();
        assert_eq!(got, [100, 200, 300, 200, 300, 200]);
        assert_eq!(playback.current_frame_len(), None);
        assert_eq!(playback.total_duration(), None);
    }

    #[test]
    fn output_test_tone_is_a_bounded_one_second_mono_source() {
        let tone = StreamPcm::tone(440.0, Duration::from_secs(1));
        assert_eq!(tone.channels, 1);
        assert_eq!(tone.sample_rate, 48_000);
        assert_eq!(tone.frames(), 48_000);
        assert_eq!(tone.duration(), Duration::from_secs(1));
        assert!(tone.samples.iter().any(|&sample| sample != 0));
        assert!(
            tone.samples
                .iter()
                .all(|&sample| sample.unsigned_abs() <= (i16::MAX as u16) / 4)
        );
    }

    #[test]
    fn the_debug_spec_reads_its_fields_in_order() {
        let r = parse_spec("/a/ambience.big:0:5:48000").expect("four fields");
        assert_eq!(r.archive, PathBuf::from("/a/ambience.big"));
        assert_eq!(r.entry, "0");
        assert_eq!(r.channels, 5);
        assert_eq!(r.sample_rate, 48_000);
        assert_eq!(r.blocks, 0);
        let r = parse_spec("/a/b.big:name:2:44100:8").expect("five fields");
        assert_eq!(r.entry, "name");
        assert_eq!(r.blocks, 8);
        assert!(parse_spec("/a/b.big:0:5").is_err());
        assert!(parse_spec("/a/b.big:0:many:48000").is_err());
    }

    #[test]
    fn bank_sample_spec_keeps_a_windows_archive_path_intact() {
        let request =
            parse_bank_sample_spec(r"C:\\Skate 3\\audio\\audiofiles.big|GRINDS.abk|17").unwrap();
        assert_eq!(
            request.archive,
            PathBuf::from(r"C:\\Skate 3\\audio\\audiofiles.big")
        );
        assert_eq!(request.bank, "GRINDS.abk");
        assert_eq!(request.sample, 17);
        assert!(parse_bank_sample_spec("archive|bank.abk").is_err());
        assert!(parse_bank_sample_spec("archive||0").is_err());
        assert!(parse_bank_sample_spec("archive|bank.abk|many").is_err());
    }

    #[test]
    fn a_request_defaults_to_the_whole_member_at_full_volume() {
        let r = StreamRequest::new("/x/ambience.big", "0", 5, 48_000);
        assert_eq!(r.blocks, 0);
        assert_eq!(r.volume, 1.0);
        assert!(!r.looping, "a one-shot by default; only ambience repeats");
        assert_eq!(r.blocks(8).blocks, 8);
    }
}
