//! The retail player-sound frame: the order `sub_82485190` runs the audio game side in, once per
//! 60 Hz physics tick.
//!
//! Retail alternates the two halves of `sub_82485190` only while dt ≤ 0.02 s (`0x822F8DE8`); at a
//! fixed 60 Hz it runs both every frame with dt = 1/60, which is what this does:
//!
//! 1. the audio-state bridge `sub_824B0DA8` ([`AudioState::update`]) and the state controller's
//!    inputs (`sub_824B19C8`, key `0x60010000`);
//! 2. every component's `process` (+36), which also writes its own controller inputs;
//! 3. the MixMap evaluation (`sub_8294BAE8` → `sub_8294F5E8`);
//! 4. every component's `update` (+40), reading this evaluation's outputs.

use skate_audio_core::authored::AuthoredRuntime;
use skate_audio_core::mixmap::{self, inputs};

use super::audio_state::AudioState;
use super::components::{Component, ControlSnapshot, Tick};
use super::contact_voices::{ContactVoicePlayer, SharedContactVoices};
use super::tuning::AudioTuning;
use crate::skate_audio::PlayerAudioObservation;

/// The game runs its audio game side at the physics rate.
pub(crate) const FRAME_SECONDS: f32 = 1.0 / 60.0;

/// SFXCTL_PlayerPhysics, the audio state's `[state+12]` controller (local player, group 0).
const STATE_CONTROLLER: u32 = mixmap::key(3, 1, 0, 0);
/// OffBoard (the footsteps' owner) controller, group 0.
const OFF_BOARD_CONTROLLER: u32 = mixmap::key(2, 1, 0, 9);
/// The state's two 3DObjPos children (`sub_824B0C48`): the skater's and the board's position
/// controllers, which feed every component's distance/pan lookups.
const SKATER_POSITION_CONTROLLER: u32 = mixmap::key(3, 1, 0, 1);
const BOARD_POSITION_CONTROLLER: u32 = mixmap::key(3, 1, 0, 2);
/// SFXObj_Contacts (landing inputs 1/2/6, `sub_824B90D8` / `sub_824BA630`), group 0.
const CONTACTS_CONTROLLER: u32 = mixmap::key(2, 1, 0, 1);

/// A component and the controller its owner readers use.
pub(crate) struct Entry {
    pub name: &'static str,
    pub controller: u32,
    pub component: Box<dyn Component>,
    /// The last evaluation's outputs: what a process slot (first half, before this frame's
    /// evaluation) reads.
    pub last: ControlSnapshot,
}

impl Entry {
    pub(crate) fn new(name: &'static str, controller: u32, component: Box<dyn Component>) -> Self {
        Self {
            name,
            controller,
            component,
            last: ControlSnapshot::default(),
        }
    }
}

pub(crate) struct PlayerSound {
    audio: AudioState,
    tuning: AudioTuning,
    /// The state's `+784` listener-distance slew (id 13).
    distance_784: f32,
    state_controller: u32,
    off_board_controller: u32,
    contacts_controller: u32,
    contacts: inputs::Contacts,
    jitter: inputs::Jitter,
    music: inputs::MusicEmphasis,
    music_controller: u32,
    pause_controller: u32,
    listener: inputs::Listener,
    positions: [(u32, inputs::ObjPos); 2],
    entries: Vec<Entry>,
    /// The Contacts component's recorded one-shot plays and the player that opens them.
    contact_voices: Option<(SharedContactVoices, ContactVoicePlayer)>,
    tick: u64,
}

fn controller(runtime: &AuthoredRuntime, key: u32, name: &str) -> Result<u32, String> {
    runtime
        .mixmap_controller(key)
        .ok_or_else(|| format!("MixMapSK8.mxb has no {name} controller {key:#010x}"))
}

impl PlayerSound {
    /// `runtime` must already have the MixMap loaded.
    pub(crate) fn new(
        runtime: &mut AuthoredRuntime,
        tuning: AudioTuning,
        entries: Vec<Entry>,
        contact_voices: Option<(SharedContactVoices, ContactVoicePlayer)>,
    ) -> Result<Self, String> {
        if !runtime.has_mixmap() {
            return Err("the MixMap is not loaded; player sound needs MixMapSK8.mxb".into());
        }
        // Retail's free-skate values of the global controllers (the capture's steady state; each
        // class's writer is cited in `mixmap::inputs::FREE_SKATE_GLOBALS`).
        for &(key, id, value) in inputs::FREE_SKATE_GLOBALS {
            let ctrl = controller(runtime, key, "global")?;
            runtime
                .mixmap_set(ctrl, id, value)
                .map_err(|e| e.to_string())?;
        }
        // APPROXIMATION: this runtime plays no retail music, so Music ids 0/3/6 and VU ids 0/1
        // hold their most frequent free-skate capture values (`FREE_SKATE_MUSIC_VU`). Retail feeds
        // them from the music player (`sub_824D1208`) and the output meters (`sub_824EDBE8`).
        for &(key, id, value) in inputs::FREE_SKATE_MUSIC_VU {
            let ctrl = controller(runtime, key, "music/VU")?;
            runtime
                .mixmap_set(ctrl, id, value)
                .map_err(|e| e.to_string())?;
        }
        Ok(Self {
            audio: AudioState::default(),
            tuning,
            distance_784: 0.0,
            state_controller: controller(runtime, STATE_CONTROLLER, "PlayerPhysics")?,
            off_board_controller: controller(runtime, OFF_BOARD_CONTROLLER, "OffBoard")?,
            contacts_controller: controller(runtime, CONTACTS_CONTROLLER, "Contacts")?,
            contacts: inputs::Contacts::default(),
            jitter: inputs::Jitter::retail(),
            music: inputs::MusicEmphasis::default(),
            music_controller: controller(runtime, 0x4000_0010, "Music")?,
            pause_controller: controller(runtime, 0x4000_0070, "Pause")?,
            listener: inputs::Listener::default(),
            positions: [
                (
                    controller(runtime, SKATER_POSITION_CONTROLLER, "skater position")?,
                    inputs::ObjPos::default(),
                ),
                (
                    controller(runtime, BOARD_POSITION_CONTROLLER, "board position")?,
                    inputs::ObjPos::default(),
                ),
            ],
            entries,
            contact_voices,
            tick: 0,
        })
    }

    pub(crate) fn audio(&self) -> &AudioState {
        &self.audio
    }

    /// The outputs a component read at its last update (diagnostics).
    pub(crate) fn controls(&self, name: &str) -> Option<(u32, &ControlSnapshot)> {
        self.entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| (entry.controller, &entry.last))
    }

    /// One game frame.
    pub(crate) fn frame(
        &mut self,
        runtime: &mut AuthoredRuntime,
        observation: &PlayerAudioObservation,
    ) -> Result<(), String> {
        let set = |runtime: &mut AuthoredRuntime, ctrl: u32, pairs: &[(u32, u32)]| {
            pairs
                .iter()
                .try_for_each(|&(id, value)| runtime.mixmap_set(ctrl, id, value))
                .map_err(|e| e.to_string())
        };

        // 1. Bridge, then `sub_824B19C8`.
        super::trace::frame(self.tick);
        self.audio.update(&observation.retail, &self.tuning);
        super::trace::state(&self.audio, &observation.retail);
        let audio = &self.audio;
        let fields = inputs::StateFields {
            wheel_count_200: audio.wheel_count_200,
            ground_speed_208: audio.ground_speed_208,
            com_speed_212: audio.com_speed_212,
            brake_336: audio.brake_336,
            manual_brake_339: audio.manual_brake_339,
            trick_active_343: audio.trick_active_343,
            local_player_72: true,
            // APPROXIMATION: the G+16 writer (retail sets it with each bail for ~110 frames) is
            // not yet traced; id 11 stays off.
            bail_camera_g16: false,
            vector_96: [
                audio.com_velocity_96[0],
                audio.com_velocity_96[1],
                audio.com_velocity_96[2],
                0.0,
            ],
            // APPROXIMATION: the engine does not yet forward the listener (camera) to the audio
            // worker; without it id 13 measures from the origin.
            listener_32: None,
        };
        let state_inputs = inputs::state_inputs(
            &fields,
            &mut self.distance_784,
            FRAME_SECONDS,
            inputs::DISTANCE_RATE,
            inputs::DISTANCE_CAP,
        );
        // The bridge writes id 3 (`sub_824B2088`, listener facing) and id 12 (`sub_824B23C8`)
        // before `sub_824B19C8`.
        let retail = &observation.retail;
        if let Some((camera_at, _)) = retail.camera {
            let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
            // `a` = record +496 = [B+0]+80 (the effective deck up); `b` = record +64 = [B+0]+128.
            // UNVERIFIED: B+0 +128 is not traced; the deck's effective forward stands in.
            let (facing, factor_680) = inputs::listener_facing(
                inputs::normalize3(v4(retail.effective_deck_up)),
                inputs::normalize3(v4(retail.deck_forward)),
                inputs::normalize3(v4(camera_at)),
            );
            self.audio.listener_facing_680 = factor_680;
            set(runtime, self.state_controller, &[(3, facing)])?;
        }
        let flag = inputs::player_flag(true, self.audio.soft_wheels_684, &[]);
        set(
            runtime,
            self.state_controller,
            &[(12, if flag == 1 { 32767 } else { 0 })],
        )?;
        set(runtime, self.state_controller, &state_inputs)?;

        // The listener (`sub_8248CC08`, first in the first half) and the two position
        // controllers (`sub_824AEC70`, the state's children's vfunc 16).
        if let Some((camera_at, camera_position)) = retail.camera {
            let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
            let record = inputs::PlayerRecord {
                position_0: v4(retail.position),
                // UNVERIFIED: record +16 = [B+20]+80 is row 1 (the up axis) of a skeleton transform
                // (SkeletonState+9488) the engine does not publish; world up stands in. For the
                // skater's own controller the emitter is the followed point, so it has no effect.
                facing_16: [0.0, 1.0, 0.0, 0.0],
                velocity_32: v4(retail.com_velocity),
                // B+0 +128 is row 2 of the effective deck transform (`82C02A80` stores the
                // `82C01BF8` result's +32 there): the deck's effective forward, verified.
                // UNVERIFIED: B+0 +144 is `[[..]+652]+16` of an unnamed body object in
                // `82C02A80`; the deck position stands in.
                position_48: v4(retail.deck_position),
                facing_64: v4(retail.deck_forward),
                velocity_80: v4(retail.linear_velocity),
            };
            self.listener.update(
                v4(camera_at),
                v4(camera_position),
                true,
                FRAME_SECONDS,
                Some(&record),
            );
            let (skater, board) = record.emitters();
            for ((ctrl, object), emitter) in self.positions.iter_mut().zip([skater, board]) {
                let word15 = runtime.mixmap_get(*ctrl, 15).map_err(|e| e.to_string())?;
                let writes = object.update(&self.listener, &emitter, word15);
                set(runtime, *ctrl, &writes)?;
            }
        }

        // 2. Owner inputs written from the process slots.
        // Rail's ids 0/1 come from the Grind component's own process (`sub_824C28B0`).
        set(
            runtime,
            self.off_board_controller,
            &[inputs::off_board_input(self.audio.walking_716)],
        )?;
        let contacts = self.contacts.process(&inputs::ContactsFields {
            airborne_332: self.audio.in_known_air_332,
            grinding_341: self.audio.grinding_341,
            wheel_contact_464: self.audio.wheel_landed_464,
            wheel_word_448: self.audio.wheel_landing_bucket_448.map(|w| w as i32),
            wheel_material_620: self.audio.wheel_material_620,
            local_player: true,
        });
        set(runtime, self.contacts_controller, &contacts)?;
        // The combo multiplier: `sub_827A2E88`'s tier flags, then the Music process
        // (`sub_824D1208`) — id 3 while x3, id 6 slewed toward the tier's emphasis. Every player
        // controller reads these; x3 lifts e.g. the board grain chain's level 1267 -> 28343.
        // `sub_824898C8`'s non-free-skate mode terms are false here.
        let flags = inputs::multiplier_flags(Some(observation.retail.combo_multiplier));
        self.audio.multiplier_flags_2f0d0 = flags;
        let music = self.music.process(flags, false, FRAME_SECONDS);
        set(runtime, self.music_controller, &music)?;
        // SFXObj_Pause (`sub_824E1D00`, a slot-9 process): retail keeps the bridge, the components
        // and the voices running while paused and ducks every player level through input 0 (it
        // reaches silence in ~175 ms and recovers in ~88 ms). Written every frame, paused or not.
        set(
            runtime,
            self.pause_controller,
            &inputs::pause_inputs(observation.retail.paused),
        )?;
        // Jitter (`sub_824EF378`): 24 random-walk channels drawn from the shared retail generator.
        runtime
            .mixmap_jitter_tick(&mut self.jitter)
            .map_err(|e| e.to_string())?;
        for entry in &mut self.entries {
            let mut tick = Tick {
                runtime: &mut *runtime,
                audio: &self.audio,
                controls: &entry.last,
                dt: FRAME_SECONDS,
                tick: self.tick,
            };
            entry
                .component
                .process(&mut tick)
                .map_err(|e| format!("{} process: {e}", entry.name))?;
            let owner = entry.component.take_owner_inputs();
            set(runtime, entry.controller, &owner)?;
        }

        // The Contacts component recorded its one-shot plays during its process; open them now,
        // before the evaluation, as retail's process does.
        if let Some((shared, player)) = &mut self.contact_voices {
            player.drain(runtime, shared, &self.audio, FRAME_SECONDS)?;
            // Retail's `GlobalFEPlaySound` is not part of the component graph at all -- the UI
            // calls it directly -- so these ride in on the observation and open here, alongside
            // the other one-shots, rather than going through a component.
            for &id in &observation.frontend_sounds {
                match super::frontend::FrontEndSound::from_id(id) {
                    Some(sound) => player.play_frontend(runtime, sound),
                    None => eprintln!("SKATE_PLAYER_AUDIO frontend_sound_unknown id={id:#018x}"),
                }
            }
        }

        // 3. Evaluate.
        runtime
            .mixmap_tick(FRAME_SECONDS)
            .map_err(|e| e.to_string())?;

        // 4. Updates read this evaluation.
        for entry in &mut self.entries {
            entry.last = ControlSnapshot::read(runtime, entry.controller);
            let mut tick = Tick {
                runtime: &mut *runtime,
                audio: &self.audio,
                controls: &entry.last,
                dt: FRAME_SECONDS,
                tick: self.tick,
            };
            entry
                .component
                .update(&mut tick)
                .map_err(|e| format!("{} update: {e}", entry.name))?;
            // Inputs written during an update are consumed by the next evaluation.
            let owner = entry.component.take_owner_inputs();
            set(runtime, entry.controller, &owner)?;
        }
        self.tick += 1;
        Ok(())
    }
}

/// The SFXObj controllers of the local player (slot 1, group 0), by object index.
fn object_key(object: u32) -> u32 {
    mixmap::key(2, 1, 0, object)
}

/// Build every local-player component on a runtime whose MixMap is loaded: the constructor side
/// of `sub_824C5058` and its siblings.
pub(crate) fn build(
    runtime: &mut AuthoredRuntime,
    assets: &std::path::Path,
    cache: Option<&std::path::Path>,
) -> Result<PlayerSound, String> {
    use super::components::{
        board::{Board, BoardConfig, BoardVault},
        clothing::Clothing,
        contacts::{ContactsOwner, FootDrag},
        footsteps::Footsteps,
        grind::Grind,
        seams::Seams,
        speed::SenseOfSpeed,
        treatment::Treatment,
        tricks::Tricks,
        wheels::{self, Wheels},
    };
    use skate_data::collections::Collections;

    let vault = Collections::load(assets)?;
    let tuning = AudioTuning::from_collections(&vault)?;
    let grains = skate_data::audio::grains::load_grains(
        &assets.join("private/stock/data/audio/grains.big"),
        &[],
        cache,
    )
    .map_err(|e| e.to_string())?;
    let board = {
        let audio = AudioState::default();
        let controls = ControlSnapshot::default();
        let mut tick = Tick {
            runtime: &mut *runtime,
            audio: &audio,
            controls: &controls,
            dt: FRAME_SECONDS,
            tick: 0,
        };
        Board::new(
            &mut tick,
            BoardVault::load(assets)?,
            &grains,
            BoardConfig::from_env(),
        )?
    };
    let (wheels_vault, wheel_members) = wheels::load(assets, cache)?;
    let (wheels_component, speed) = {
        let audio = AudioState::default();
        let controls = ControlSnapshot::default();
        let mut tick = Tick {
            runtime: &mut *runtime,
            audio: &audio,
            controls: &controls,
            dt: FRAME_SECONDS,
            tick: 0,
        };
        (
            Wheels::new(&mut tick, wheels_vault, &wheel_members, true)?,
            SenseOfSpeed::with_rocket(&mut tick, &vault, &grains)?,
        )
    };
    let ctrl = |runtime: &AuthoredRuntime, object: u32, name: &str| {
        controller(runtime, object_key(object), name)
    };
    // `sub_824B90D8` runs inside the Contacts process before the foot-drag trigger
    // (`sub_824BB540`), so the owner ticks first and shares the Contacts controller.
    let voices = SharedContactVoices::default();
    let contacts_owner = ContactsOwner::new(&vault, true, Box::new(voices.clone()))?;
    let contact_player = ContactVoicePlayer::new(assets, cache)?;
    let entries = vec![
        Entry::new(
            "ContactsOwner",
            ctrl(runtime, 1, "Contacts")?,
            Box::new(contacts_owner),
        ),
        Entry::new(
            "SkateBoard",
            ctrl(runtime, 0, "SkateBoard")?,
            Box::new(board),
        ),
        Entry::new(
            "Contacts",
            ctrl(runtime, 1, "Contacts")?,
            Box::new(FootDrag::new(&vault)?),
        ),
        Entry::new(
            "Wheels",
            ctrl(runtime, 2, "Wheels")?,
            Box::new(wheels_component),
        ),
        Entry::new(
            "Rail",
            ctrl(runtime, 3, "Rail")?,
            Box::new(Grind::new(&vault)?),
        ),
        Entry::new(
            "Cracks",
            ctrl(runtime, 4, "Cracks")?,
            Box::new(Seams::new(&vault)?),
        ),
        Entry::new(
            "Tricks",
            ctrl(runtime, 5, "Tricks")?,
            Box::new(Tricks::new(&vault)?),
        ),
        Entry::new(
            "Clothing",
            ctrl(runtime, 6, "Clothing")?,
            Box::new(Clothing::new(&vault)?),
        ),
        Entry::new(
            "Treatments",
            ctrl(runtime, 7, "Treatments")?,
            Box::new(Treatment::new(&vault)?),
        ),
        Entry::new(
            "SenseOfSpeed",
            ctrl(runtime, 8, "SenseOfSpeed")?,
            Box::new(speed),
        ),
        Entry::new(
            "OffBoard",
            ctrl(runtime, 9, "OffBoard")?,
            Box::new(Footsteps::new(&vault)?),
        ),
    ];
    PlayerSound::new(runtime, tuning, entries, Some((voices, contact_player)))
}
