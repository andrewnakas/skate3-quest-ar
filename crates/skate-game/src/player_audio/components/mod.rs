//! The retail player-sound components, one per game-side sound object.
//!
//! Each mirrors its retail class: vtable slot 9 (`process`, the `+36` tick) posts and releases
//! messages, slot 10 (`update`, the `+40` tick) rewrites the held packets and redelivers them.
//! Gameplay words come from the retail audio state ([`AudioState`]), gain/pitch/pan words from the
//! component's MixMap controller ([`Controls`]), tuning from the owner's AttribSys vault. Word
//! formulas are in [`words`], each checked against the retail recomp capture.
//!
//! Per game frame, as retail runs it at a fixed 60 Hz (both halves of `sub_82485190` each frame,
//! because dt > 0.02 would otherwise alternate them): the bridge writes the audio state and the
//! state controller's inputs, every component's `process` writes its owner inputs, the MixMap
//! evaluates, then every component's `update` reads this evaluation's outputs. (The capture's
//! apparent one-frame lag between the state dump and the updates is only where its frame counter
//! increments. Retail's update (+40) reads the state from F−1 and its process (+36) reads F; running
//! bridge → process → evaluate → update each tick is equivalent, with posts landing one tick after
//! retail's frame label.)

pub(crate) mod board;
pub(crate) mod clothing;
pub(crate) mod contacts;
pub(crate) mod footsteps;
pub(crate) mod grind;
pub(crate) mod seams;
pub(crate) mod speed;
pub(crate) mod treatment;
pub(crate) mod tricks;
pub(crate) mod wheels;
pub(crate) mod words;

use skate_audio_core::authored::AuthoredRuntime;

use super::audio_state::AudioState;

/// A component's reads of its MixMap controller's output table (`[[owner+12]+12]`).
pub(crate) trait Controls {
    /// Owner vfunc 52, `sub_824C2870`: the raw 16-bit output.
    fn raw(&self, id: u32) -> u32;
    /// Owner vfunc 56, `sub_824C5910`: a cents output converted to `2^(c/1200) × 4096`.
    fn pitch(&self, id: u32) -> i32;
    /// Owner vfuncs 60/64, `sub_824AF240`: the 15-bit output.
    fn level(&self, id: u32) -> u32;
}

/// What one component tick sees.
pub(crate) struct Tick<'a> {
    pub runtime: &'a mut AuthoredRuntime,
    /// This frame's audio state (see the module note).
    pub audio: &'a AudioState,
    pub controls: &'a dyn Controls,
    /// Game frame time in seconds.
    pub dt: f32,
    pub tick: u64,
}

pub(crate) trait Component {
    /// Vtable slot 9 / `+36`: post, release and trigger logic.
    fn process(&mut self, tick: &mut Tick) -> Result<(), String>;
    /// Vtable slot 10 / `+40`: rewrite and redeliver the held packets.
    fn update(&mut self, tick: &mut Tick) -> Result<(), String>;
    /// The MixMap controller inputs (`id`, 32-bit word) this component wrote to its own controller
    /// (controller vfunc 8) since the last call, in retail call order. The worker applies them
    /// before the next evaluation.
    fn take_owner_inputs(&mut self) -> Vec<(u32, u32)> {
        Vec::new()
    }
}

/// Number of controller output ids a snapshot keeps. Output blocks are 15 words of packed int16
/// pairs plus the enable word, so ids stay below 30.
pub(crate) const CONTROLLER_OUTPUTS: usize = 30;

/// One controller's outputs, read once after the evaluation through the retail owner readers.
#[derive(Clone, Debug, Default)]
pub(crate) struct ControlSnapshot {
    raw: [u32; CONTROLLER_OUTPUTS],
    pitch: [i32; CONTROLLER_OUTPUTS],
    level: [u32; CONTROLLER_OUTPUTS],
}

impl ControlSnapshot {
    pub(crate) fn read(runtime: &AuthoredRuntime, controller: u32) -> Self {
        let mut snapshot = Self::default();
        for id in 0..CONTROLLER_OUTPUTS {
            snapshot.raw[id] = runtime.mixmap_raw(controller, id as u32);
            snapshot.pitch[id] = runtime.mixmap_pitch(controller, id as u32) as i32;
            snapshot.level[id] = runtime.mixmap_level(controller, id as u32);
        }
        snapshot
    }
}

impl Controls for ControlSnapshot {
    fn raw(&self, id: u32) -> u32 {
        self.raw.get(id as usize).copied().unwrap_or(0)
    }
    fn pitch(&self, id: u32) -> i32 {
        self.pitch.get(id as usize).copied().unwrap_or(0)
    }
    fn level(&self, id: u32) -> u32 {
        self.level.get(id as usize).copied().unwrap_or(0)
    }
}

/// `SKATE_AUDIO_POSTS=1` logs every post and release (diagnostics).
fn trace_posts() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SKATE_AUDIO_POSTS").is_some())
}

/// Release a held message, if any.
pub(crate) fn release(
    runtime: &mut AuthoredRuntime,
    holder: &mut Option<u32>,
) -> Result<(), String> {
    if let Some(handle) = holder.take() {
        if trace_posts() {
            eprintln!("SKATE_PLAYER_AUDIO release handle={handle:#010x}");
        }
        super::trace::release(handle);
        runtime.release(handle).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn post(
    runtime: &mut AuthoredRuntime,
    object: &str,
    words: &[u32],
) -> Result<u32, String> {
    let handle = runtime
        .post(object, words)
        .map_err(|error| error.to_string())?;
    if trace_posts() {
        eprintln!("SKATE_PLAYER_AUDIO post {object} handle={handle:#010x} words={words:x?}");
    }
    super::trace::post(object, handle, words);
    Ok(handle)
}

pub(crate) fn redeliver(
    runtime: &mut AuthoredRuntime,
    handle: u32,
    words: &[u32],
) -> Result<(), String> {
    super::trace::update(handle, words);
    runtime
        .redeliver(handle, words)
        .map_err(|error| error.to_string())
}
