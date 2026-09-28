//! Class_foot_drag (Contacts controller 40010010).
//!
//! The Contacts component's process (`sub_824B8218`, gated on `[[this+16]+52]`) calls the trigger
//! `sub_824BB540`; its update (`sub_824BE130`, same gate) calls the updater `sub_824BEEE8`. The
//! held message lives at component `+128`; the constructor is `sub_824AF498` (64-byte object,
//! 15 words, message slot 0 = `0x8302EE28`).
//!
//! - Trigger: while `+336 || +339 || (local && +310)` and nothing is held, post
//!   w7 = speed (or 500 on the local hold path), w8 = foot surface (`sub_824BA390`), w9..w12 vault
//!   levels, w13 = `!+336`, w14 = vault eEQChain (the manual-brake one while `+339`).
//! - Updater: once none of the three holds, release; otherwise rewrite w0..w8 and redeliver.
//!
//! Every tuning value is read from the vault at construction through the audio tuning holder
//! `*(0x830CFDA4)`: +24 = class `C26949FCB638A2CA`/`default`, +64 = the AudioSurfaceMap
//! (`C1831BDB6CB1B1EA`/`C489459A0C07D154`, field `4CA607558B1CF440`), +140 = eEQChain class
//! `42AFE160E647167C`/`default`.

use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::words::{KMH_PER_MS, TEN_THOUSAND, fctiwz};
use super::{Component, Controls, Tick, post, redeliver, release};

/// Holder +24 (`sub_8279C948` class) and holder +140 (eEQChain) collections.
const TUNING_CLASS: &str = "Hash_C26949FCB638A2CA";
const EQ_CLASS: &str = "Hash_42AFE160E647167C";
const DEFAULT_KEY: &str = "Hash_D7EDBD362D7D2152";
/// Holder +64: `Sk8::AudioSurfaceMap`, 95 elements of 72 bytes.
const SURFACE_CLASS: &str = "Hash_C1831BDB6CB1B1EA";
const SURFACE_KEY: &str = "Hash_C489459A0C07D154";
const SURFACE_FIELD: &str = "Hash_4CA607558B1CF440";

/// `0x8209975C`: the updater's speed offset (the trigger reads the vault's instead).
const UPDATER_SPEED_OFFSET: f32 = f32::from_bits(0x3F00_0000);
/// Packet length: the constructor's 64-byte object minus the 4-byte header.
pub(crate) const FOOT_DRAG_WORDS: usize = 15;
const OBJECT: &str = "Class_foot_drag";

/// PowerPC `fsel`: `a >= 0 ? b : c` (NaN selects `c`).
pub(crate) fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// The updaters' `cmpwi`/`li` clamp of a signed word.
pub(crate) fn clamp_word(value: i32, low: i32, high: i32) -> u32 {
    (if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }) as u32
}

/// The first 32-bit lane of a vault field, whatever its reflection type (eEQChain enums are
/// 32-bit values the code reads with `lwz`).
pub(crate) fn vault_word(
    vault: &Collections,
    class: &str,
    key: &str,
    name: &str,
) -> Result<u32, String> {
    let data = &vault.field(class, key, name)?.data;
    let hex: String = data
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(8)
        .collect();
    u32::from_str_radix(&hex, 16).map_err(|e| format!("{class}/{key}/{name}: {e}"))
}

/// `Sk8::AudioSurfaceMap` (holder +64). `sub_82484198` indexes the array; materials outside
/// 0..94 read element 94 (`sub_82494E18`, `sub_82494EB8`, `sub_824DC2B0`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SurfaceMap {
    entries: Vec<[u32; 18]>,
}

impl SurfaceMap {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let field = vault.field(SURFACE_CLASS, SURFACE_KEY, SURFACE_FIELD)?;
        let array = field
            .array
            .as_ref()
            .ok_or("AudioSurfaceMap 4CA607558B1CF440 has no array payload")?;
        let entries = array
            .items
            .iter()
            .map(|item| {
                let hex: String = item.chars().filter(|c| !c.is_whitespace()).collect();
                if hex.len() != 144 {
                    return Err(format!(
                        "AudioSurfaceMap element of {} hex digits",
                        hex.len()
                    ));
                }
                let mut words = [0; 18];
                for (i, word) in words.iter_mut().enumerate() {
                    *word = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16)
                        .map_err(|e| e.to_string())?;
                }
                Ok(words)
            })
            .collect::<Result<Vec<_>, String>>()?;
        if entries.len() < 95 {
            return Err(format!(
                "AudioSurfaceMap has {} elements, retail reads 95",
                entries.len()
            ));
        }
        Ok(Self { entries })
    }

    /// The 32-bit word at byte `offset` of the material's element.
    pub(crate) fn lookup(&self, material: i32, offset: usize) -> u32 {
        let index = if (0..94).contains(&material) {
            material as usize
        } else {
            94
        };
        self.entries[index][offset / 4]
    }

    #[cfg(test)]
    pub(crate) fn from_entries(entries: Vec<[u32; 18]>) -> Self {
        Self { entries }
    }
}

/// The audio-state fields the foot drag trigger and updater read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FootDragInputs {
    pub ground_speed_208: f32,
    pub hold_expired_310: bool,
    pub brake_336: bool,
    pub manual_brake_339: bool,
    /// +620 / +628: wheel 0 and wheel 2 materials (signed compares in `sub_824BA390`).
    pub wheel_material_620: i32,
    pub wheel_material_628: i32,
}

impl FootDragInputs {
    pub(crate) fn from_state(state: &AudioState) -> Option<Self> {
        Some(Self {
            ground_speed_208: state.ground_speed_208,
            hold_expired_310: state.hold_expired_310,
            brake_336: state.brake_336,
            manual_brake_339: state.manual_brake_339,
            wheel_material_620: state.wheel_material_620[0] as i32,
            wheel_material_628: state.wheel_material_620[2] as i32,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_capture(words: &[u32]) -> Self {
        use capture::{byte, float, int};
        Self {
            ground_speed_208: float(words, 208),
            hold_expired_310: byte(words, 310) != 0,
            brake_336: byte(words, 336) != 0,
            manual_brake_339: byte(words, 339) != 0,
            wheel_material_620: int(words, 620),
            wheel_material_628: int(words, 628),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FootDragTuning {
    /// Holder +24 `E5A6D8AC6EB9B5AB` (0.5): the trigger's speed offset.
    pub speed_offset: f32,
    /// Holder +24 `B2C81577820408BE` (50 km/h): the speed range.
    pub top_kmh: f32,
    /// Holder +24 `97171DE6035D6069`, `194EF41DA2254153`, `6242BC0480B09A33`,
    /// `9FF88541CAB461C7`: constructor words 9..12 (4000, 4500, 4000, 22500).
    pub levels: [i32; 4],
    /// Holder +140 `C58CE169C13320CA` (brake) / `2DBD9ED0AD824844` (manual brake): w14.
    pub eq_brake: i32,
    pub eq_manual: i32,
    pub surfaces: SurfaceMap,
}

impl FootDragTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let int = |name: &str| {
            vault
                .integer(TUNING_CLASS, DEFAULT_KEY, name)
                .map(|v| v as i32)
        };
        Ok(Self {
            speed_offset: vault.float(TUNING_CLASS, DEFAULT_KEY, "Hash_E5A6D8AC6EB9B5AB")?,
            top_kmh: vault.float(TUNING_CLASS, DEFAULT_KEY, "Hash_B2C81577820408BE")?,
            levels: [
                int("Hash_97171DE6035D6069")?,
                int("Hash_194EF41DA2254153")?,
                int("Hash_6242BC0480B09A33")?,
                int("Hash_9FF88541CAB461C7")?,
            ],
            eq_brake: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_C58CE169C13320CA")? as i32,
            eq_manual: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_2DBD9ED0AD824844")? as i32,
            surfaces: SurfaceMap::load(vault)?,
        })
    }
}

/// `sub_824BB540` / `sub_824BEEE8`: `+336 || +339 || (local && +310)`; `local` is owner byte
/// `[this+28]+72`.
pub(crate) fn foot_drag_active(inputs: &FootDragInputs, local: bool) -> bool {
    inputs.brake_336 || inputs.manual_brake_339 || (local && inputs.hold_expired_310)
}

/// The speed word both functions compute: `fctiwz(min(fsel(−x, 0, x), 1) × 10000)` with
/// x = (v − offset) / top × 3.6, or 500 on the local hold path.
fn drag_speed(inputs: &FootDragInputs, offset: f32, top_kmh: f32, local: bool) -> i32 {
    let x = (inputs.ground_speed_208 - offset) / top_kmh * KMH_PER_MS;
    let low = fsel(-x, 0.0, x);
    let unit = fsel(1.0 - low, low, 1.0);
    let speed = fctiwz(unit * TEN_THOUSAND);
    if local && inputs.hold_expired_310 {
        500
    } else {
        speed
    }
}

/// `sub_824BA390`: the foot surface from wheel 2's material while `+339`, else wheel 0's;
/// material ≥ 143 (none) → 0; AudioSurfaceMap `+20`; with `+339`, surface 1 → 0.
pub(crate) fn foot_surface(inputs: &FootDragInputs, surfaces: &SurfaceMap) -> i32 {
    let manual = inputs.manual_brake_339;
    let material = if manual {
        inputs.wheel_material_628
    } else {
        inputs.wheel_material_620
    };
    if material >= 143 {
        return 0;
    }
    let surface = surfaces.lookup(material, 20) as i32;
    if manual && surface == 1 { 0 } else { surface }
}

/// `sub_824BB540` → `sub_824AF498`: the posted packet.
pub(crate) fn foot_drag_constructor(
    tuning: &FootDragTuning,
    inputs: &FootDragInputs,
    local: bool,
) -> [u32; FOOT_DRAG_WORDS] {
    let speed = drag_speed(inputs, tuning.speed_offset, tuning.top_kmh, local);
    let eq = if inputs.manual_brake_339 {
        tuning.eq_manual
    } else {
        tuning.eq_brake
    };
    let mut words = [0; FOOT_DRAG_WORDS];
    words[1] = 32_767;
    words[5] = 25_000;
    words[7] = clamp_word(speed, 0, 10_000);
    words[8] = clamp_word(foot_surface(inputs, &tuning.surfaces), 0, 10);
    for (i, level) in tuning.levels.iter().enumerate() {
        words[9 + i] = clamp_word(*level, 0, 32_767);
    }
    words[13] = clamp_word(i32::from(!inputs.brake_336), 0, 2);
    words[14] = clamp_word(eq, 0, 32_767);
    words
}

/// `sub_824BEEE8`'s rewrite of a held packet (words 9..14 keep the constructor's values).
pub(crate) fn foot_drag_update(
    words: &mut [u32; FOOT_DRAG_WORDS],
    tuning: &FootDragTuning,
    inputs: &FootDragInputs,
    controls: &dyn Controls,
    local: bool,
) {
    let speed = drag_speed(inputs, UPDATER_SPEED_OFFSET, tuning.top_kmh, local);
    words[7] = clamp_word(speed, 0, 10_000);
    words[0] = 32_767;
    words[1] = clamp_word(
        controls.level(if inputs.brake_336 { 4 } else { 5 }) as i32,
        0,
        32_767,
    );
    words[2] = clamp_word(controls.level(18) as i32, 0, 32_767);
    words[5] = clamp_word(controls.level(16) as i32, 0, 25_000);
    words[6] = clamp_word(controls.level(17) as i32, 0, 25_000);
    words[3] = clamp_word(controls.raw(0) as i32, 0, 0x1_0000);
    words[4] = clamp_word(controls.pitch(22), 0, 8_192);
    words[8] = clamp_word(foot_surface(inputs, &tuning.surfaces), 0, 10);
}

pub(crate) struct FootDrag {
    tuning: FootDragTuning,
    /// Owner byte +72: the engine drives only the local skater.
    local: bool,
    /// Component +128.
    held: Option<(u32, [u32; FOOT_DRAG_WORDS])>,
}

impl FootDrag {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            tuning: FootDragTuning::load(vault)?,
            local: true,
            held: None,
        })
    }
}

impl Component for FootDrag {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let Some(inputs) = FootDragInputs::from_state(tick.audio) else {
            return Ok(());
        };
        if foot_drag_active(&inputs, self.local) && self.held.is_none() {
            let words = foot_drag_constructor(&self.tuning, &inputs, self.local);
            let handle = post(tick.runtime, OBJECT, &words)?;
            self.held = Some((handle, words));
        }
        Ok(())
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        let Some(inputs) = FootDragInputs::from_state(tick.audio) else {
            return Ok(());
        };
        if self.held.is_none() {
            return Ok(());
        }
        if !foot_drag_active(&inputs, self.local) {
            let mut handle = self.held.take().map(|(handle, _)| handle);
            return release(tick.runtime, &mut handle);
        }
        if let Some((handle, words)) = self.held.as_mut() {
            foot_drag_update(words, &self.tuning, &inputs, tick.controls, self.local);
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

/// Retail recomp capture access for the tests (`.local/captures/extract`).

// =====================================================================================
// SFXObj_Contacts one-shot contact voices: pops, landing, grind onset.
// =====================================================================================
//
// `sub_824B90D8` is not a separate object: the Contacts component process `sub_824B8218`
// (slot 9, gated `[[this+16]+52]`) calls it in the middle of its chain, so the pops, landing
// and grind-onset sounds live on the same object as [`FootDrag`]. Retail order inside
// `sub_824B8218`: `sub_824B95A0` (deck-box timers), `sub_824B9948`, `sub_824BB330`,
// `sub_824B86E0`, **`sub_824B90D8`** (this port), `sub_824BB540` (the foot-drag trigger,
// [`FootDrag`]), `sub_824BBB28`, `sub_824BC188`, `sub_824BD000`, `sub_824BD358`,
// `sub_824B85B0`, `sub_824BFA48`, `sub_824C01E8`, `sub_824C07D8`, `sub_824C0DA8`, the `+440`
// countdown, `sub_824C0AB8`.
//
// These sounds are **not** authored messages. Each routine creates a generic `"Splice"` voice
// container through the object factory `sub_828AAC28` (the same allocator as the authored
// `sub_828AAE90`, plus flag bit 0x0100_0000) and drives a bank-sample voice with
// `sub_82975700` / `sub_82975A60`, freeing it again with `sub_824836B8`. They therefore have
// no message slot, no packet and no redelivery, and they appear nowhere in the retail capture
// (no object in posts.tsv / releases.tsv / updates/, and vf.tsv only records the controller
// reads 52/56/60). What can be checked against the capture is the *trigger frames*, which
// [`ContactsOwner`] derives from state.tsv edges; that is what `contacts_owner_edges_match_the_capture`
// does.
//
// The one-shot output itself is behind [`ContactVoices`]. The bank-voice side (the `"Splice"`
// factory, `sub_82975700`/`sub_82975A60`, hold-and-free and the bank pick `sub_824B9AD8`) is
// being ported separately in `skate-audio-core`; this file supplies every game-side input that
// port needs and holds the voice handles exactly where retail holds them.
//
// Ported here (decoded from `sub_824B90D8`, `sub_824B9CC8`, `sub_824BA630`, `sub_824BB0E0`):
// - the edges and gates that fire each sound, and the five owner latches;
// - the pop strength selector ladder (+468 against two vault thresholds, forced to 0 on
//   audio trick 33/34);
// - the pop roll voice's speed-tiered bank sample (+208 against three vault speeds);
// - the landing voice's vault bank sample and eEQChain, and its local-player gate;
// - the grind-onset gate, material and family sample base;
// - which slot holds which voice (`+60` pop, `+96` pop roll, `+56` landing) and when retail
//   frees it.
//
// NOT ported (undecoded, and not observable in the capture) — these are labelled at each use:
// - the bank pick `sub_824B9AD8` for the pop voice: retail turns the selector into a (bank,
//   sample) pair. The selector is handed to [`ContactVoices`] instead.
// - the pop voice gain: vtable slot 60 called with id 14, scaled by `[0x8220__+664]` and two
//   further vault fields (`E34B48082B5BF185`, `C3C25A37D00712A8`).
// - `sub_82975A60`'s spatialisation, and `sub_82489058`'s reset of the `+72` sub-object.
// - the landing voices past the first (vault fields `85FDC8BF696BCA5C` = 95,
//   `F262042EAA295711` = 860, `1E86469556ACD80A` = 862, thresholds 0.1/0.3/0.65/0.75/1.0).
// - the grind-onset variant behind its second threshold (`B2ACAFDBCD963C93` = 0.5).
// - the rolling/scrape contacts `sub_824BC188` and the eight further paths listed above.
// - whatever frees the pop roll slot `+96`. `sub_824B9CC8` only ever starts that voice when the
//   slot is empty and never clears it, so one of the unported paths in `sub_824B8218` must
//   release it; until that is found the roll voice starts once and then stays held (the capture
//   edge replay shows 1 roll against 28 pops for this reason).
//
// Controller inputs are **not** written here: `sub_824B90D8` resets ids 0/1/6 and the pop and
// landing routines raise ids 0 and 1, all of which the MixMap `inputs::Contacts` port owns.

/// Which retail routine asked for a voice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContactSound {
    /// `sub_824B9CC8` first voice, held at `+60`.
    Pop,
    /// `sub_824B9CC8` second voice, held at `+96`.
    PopRoll,
    /// `sub_824BA630` first voice, held at `+56`.
    Landing,
    /// `sub_824BA630`'s companion `sub_824B8D48(this, 0, max_class, 0)`, held at `+52`. Its sample
    /// is chosen by the landing class (`sub_824BA3F0`), which is how retail makes a hard landing
    /// sound unlike a soft one — the levels of both voices are fixed.
    LandingClass,
    /// `sub_824BA630`'s second landing voice, held at `+496` (`sub_82497F48` @ 0x824BAF18).
    ///
    /// Its sample comes from a 2×2 ladder — the deck-material test against time in air, split at
    /// `0x224B06562D9D5E0E` (0.75 s in the owner's vault) — and, like the other two, its level is
    /// fixed. See [`skate_data::audio::splice::LandingTuning::ladder_sample`].
    LandingLadder,
    /// `sub_824BB0E0`.
    GrindOnset,
}

/// Everything the game side resolves before the bank voice starts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct VoiceRequest {
    /// The vault bank-sample index, where the routine reads one directly.
    pub sample: Option<u32>,
    /// Retail pop strength 0/1/2, the input to the undecoded bank pick `sub_824B9AD8`.
    pub selector: Option<i32>,
    /// eEQChain from the vault (class `42AFE160E647167C`).
    pub eq_chain: Option<u32>,
    /// `sub_824BB0E0`: the grind material (+692, 143 → 10).
    pub material: Option<u32>,
    /// `sub_824BB0E0`: 95 for grind families 1/2/5, else 96.
    pub family_base: Option<u32>,
    /// `sub_824BB0E0`: 1 once the impact passes the first vault threshold.
    pub tier: Option<i32>,
    /// Contacts controller output 15, the landing voice's owner send level.
    pub send_level: Option<u32>,
    /// `sub_824BA630`: the latched air time (`[this+340]`) the contact-sound manager's landing
    /// message interpolates its level over — retail's "how far did you actually fall".
    pub air_time: Option<f32>,
    /// `sub_824BB0E0`: the grind impact (`+228`) its own contact level interpolates over, the
    /// grind's counterpart to the landing's air time.
    pub impact: Option<f32>,
    /// `sub_824BA630` @ 0x824BAC50: the deck material (`+660`) the ladder voice's column test
    /// runs on, or `None` when the deck-contact byte (`+614`) is clear — retail passes 0 to
    /// `sub_82494D78` in that case, which is the same as taking column 0.
    pub deck_material: Option<u32>,
    /// The landing class `sub_824BA630` resolved (largest `+448` bucket over the landed wheels).
    /// Retail derives the sample and the controller-input word from this one value.
    pub landing_class: Option<u32>,
}

/// The one-shot bank-voice sink. Implemented by the `skate-audio-core` bank-voice port; the
/// component holds whatever handle `play` returns exactly where retail holds it and hands it
/// back to `free` when retail calls `sub_824836B8`.
pub(crate) trait ContactVoices {
    /// Start a one-shot voice. `None` when no voice could be started (retail keeps its slot 0).
    fn play(&mut self, sound: ContactSound, request: &VoiceRequest) -> Option<u32>;
    /// Retail `sub_824836B8`: free a voice this component still holds.
    fn free(&mut self, handle: u32);
}

/// A sink that starts nothing, for hosts without the bank-voice path yet.
// Constructed by the worker once `ContactsOwner` is registered.
#[allow(dead_code)]
pub(crate) struct NoContactVoices;

impl ContactVoices for NoContactVoices {
    fn play(&mut self, _sound: ContactSound, _request: &VoiceRequest) -> Option<u32> {
        None
    }
    fn free(&mut self, _handle: u32) {}
}

/// Vault tuning. Every field is read through the audio tuning holder `*(0x830CFDA4)`:
/// `+24` = class `C26949FCB638A2CA`/`default` (the class [`FootDragTuning`] already uses),
/// `+40` = grind material class `049861E8F9A8D16B`/`default`, `+140` = eEQChain class
/// `42AFE160E647167C`/`default`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ContactsTuning {
    /// `F2A1E273ABB8E9AB` (0.42) and `D7758385CDB8DC26` (0.25): +468 → selector 2 / 1.
    pub pop_hard: f32,
    pub pop_medium: f32,
    /// `E34B48082B5BF185` on the eEQChain class (0).
    pub pop_eq_chain: u32,
    /// `58523180E1AD61B4` (12), `A73073D3A33E35AE` (8), `FB71DF2C85928859` (4): the +208 tiers.
    pub roll_speed_high: f32,
    pub roll_speed_mid: f32,
    pub roll_speed_low: f32,
    /// `7A745D81E4BCABC3`, `537C97E4F64A0EE2`, `9C4CDCF0DD84C281`, `537C97E4F64A0EE2`: the bank
    /// sample per tier. The stock vault sets all four to 1111, so which field belongs to which
    /// tier is not observable with stock data; the order here is the order the routine reads them.
    pub roll_sample_high: u32,
    pub roll_sample_mid: u32,
    pub roll_sample_low: u32,
    pub roll_sample_idle: u32,
    /// `633FA94E39C1AE8F` (1095) and `8B0E030799CBDD00` (1): the first landing voice.
    pub landing_sample: u32,
    pub landing_eq_chain: u32,
    /// `086B66C3D4FFEE8F` (0.25) on the grind material class: the +228 impact gate.
    pub grind_impact_threshold: f32,
}

impl ContactsTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let f = |name: &str| vault.float(TUNING_CLASS, DEFAULT_KEY, name);
        let i = |name: &str| vault_word(vault, TUNING_CLASS, DEFAULT_KEY, name);
        Ok(Self {
            pop_hard: f("Hash_F2A1E273ABB8E9AB")?,
            pop_medium: f("Hash_D7758385CDB8DC26")?,
            pop_eq_chain: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_E34B48082B5BF185")?,
            roll_speed_high: f("Hash_58523180E1AD61B4")?,
            roll_speed_mid: f("Hash_A73073D3A33E35AE")?,
            roll_speed_low: f("Hash_FB71DF2C85928859")?,
            roll_sample_high: i("Hash_7A745D81E4BCABC3")?,
            roll_sample_mid: i("Hash_537C97E4F64A0EE2")?,
            roll_sample_low: i("Hash_9C4CDCF0DD84C281")?,
            roll_sample_idle: i("Hash_537C97E4F64A0EE2")?,
            landing_sample: i("Hash_633FA94E39C1AE8F")?,
            landing_eq_chain: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_8B0E030799CBDD00")?,
            grind_impact_threshold: vault.float(
                "Hash_049861E8F9A8D16B",
                DEFAULT_KEY,
                "Hash_086B66C3D4FFEE8F",
            )?,
        })
    }
}

/// The audio-state fields `sub_824B90D8` and its three routines read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ContactsInputs {
    pub grind_family_192: u32,
    pub ground_speed_208: f32,
    pub grind_impact_228: f32,
    pub air_time_236: f32,
    pub in_known_air_332: bool,
    pub grinding_341: bool,
    pub trick_active_343: bool,
    pub audio_trick_348: u32,
    pub jump_velocity_468: f32,
    pub bail_676: bool,
    pub grind_material_692: u32,
    /// `+614`: the deck-contact byte that gates the ladder voice's material test.
    pub deck_contact_614: bool,
    /// `+660`: the deck material that test runs on.
    pub deck_material_660: u32,
    /// `+464` / `+448`: which wheels are down and each one's landing bucket. `sub_824BA630`
    /// reduces them to the single class it uses for both `sub_824B8D48`'s sample and the
    /// controller-input 2 write.
    pub wheel_landed_464: [bool; 4],
    pub wheel_landing_bucket_448: [u32; 4],
    /// Contacts controller output 15, the owner send the landing voice rides.
    /// Input 2 (the landing class) drives it: probing the real MixMap with the
    /// retail pre-roll gives 2584 / 3103 / 3650 for classes 0 / 1 / 2, a 3.0 dB
    /// spread. `Component::process` fills it; `from_state` alone cannot.
    pub landing_send_level_15: u32,
}

impl ContactsInputs {
    pub(crate) fn from_state(state: &AudioState) -> Self {
        Self {
            grind_family_192: state.grind_family_192,
            ground_speed_208: state.ground_speed_208,
            grind_impact_228: state.grind_impact_228,
            air_time_236: state.air_time_236,
            in_known_air_332: state.in_known_air_332,
            grinding_341: state.grinding_341,
            trick_active_343: state.trick_active_343,
            audio_trick_348: state.audio_trick_348,
            jump_velocity_468: state.jump_velocity_468,
            bail_676: state.bail_676,
            grind_material_692: state.grind_material_692,
            deck_contact_614: state.deck_contact_614,
            deck_material_660: state.deck_material_660,
            wheel_landed_464: state.wheel_landed_464,
            wheel_landing_bucket_448: state.wheel_landing_bucket_448,
            landing_send_level_15: 0,
        }
    }

    /// The capture's 160 state words from +192.
    #[cfg(test)]
    pub(crate) fn from_capture(words: &[u32]) -> Self {
        let word = |offset: usize| words[(offset - 192) / 4];
        let float = |offset: usize| f32::from_bits(word(offset));
        let byte = |offset: usize| (word(offset & !3) >> (8 * (3 - (offset & 3)))) & 0xFF != 0;
        Self {
            grind_family_192: word(192),
            ground_speed_208: float(208),
            grind_impact_228: float(228),
            air_time_236: float(236),
            in_known_air_332: byte(332),
            grinding_341: byte(341),
            trick_active_343: byte(343),
            audio_trick_348: word(348),
            jump_velocity_468: float(468),
            bail_676: byte(676),
            grind_material_692: word(692),
            deck_contact_614: byte(614),
            deck_material_660: word(660),
            wheel_landed_464: std::array::from_fn(|i| byte(464 + i)),
            wheel_landing_bucket_448: std::array::from_fn(|i| word(448 + 4 * i)),
            // The capture carries audio-state words, not controller outputs; the
            // send level is supplied by `Component::process` at runtime.
            landing_send_level_15: 0,
        }
    }
}

/// Audio trick ids that suppress the pop (`sub_824B90D8`: -1, 31, 32, 35, 36).
const POP_BLOCKING_TRICKS: [i32; 5] = [-1, 31, 32, 35, 36];
/// `sub_824B9CC8` skips the pop voice while `+424 < 2`. Since `sub_824BA630` zeroes `+424`, this
/// also suppresses a pop on the frame straight after any landing.
const POP_WARMUP_FRAMES: i32 = 2;
/// `sub_824BB0E0`: no grind material (+692 = 143) falls back to 10.
const GRIND_MATERIAL_FALLBACK: u32 = 10;
/// `sub_824BB0E0`: grind families 1/2/5 use sample base 95, every other family 96.
const GRIND_FAMILY_BASE_LOW: u32 = 95;
const GRIND_FAMILY_BASE_HIGH: u32 = 96;

/// The owner latches `sub_824B90D8` keeps across frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ContactLatches {
    /// `+120`: previous frame's +332.
    pub airborne_120: bool,
    /// `+122`: previous frame's +341.
    pub grinding_122: bool,
    /// `+340`: +236 sampled while airborne.
    pub air_time_340: f32,
    /// `+344`: previous frame's +676.
    pub bail_344: bool,
    /// `+424`: processed-frame counter, incremented every process and reset by the landing.
    pub frames_424: i32,
    /// `+124`: the grind-onset gate. Written by a path that is not ported, so it stays 0 and
    /// the grind onset is never suppressed here.
    pub grind_gate_124: f32,
    /// `+64`: the pop selector the routine stores beside its voice.
    pub pop_selector_64: i32,
}

/// `sub_824B90D8` and the three voice routines it drives.
pub(crate) struct ContactsOwner {
    tuning: ContactsTuning,
    /// `[this+28]+72`: the local skater. Only the local player gets the landing voice.
    local: bool,
    latches: ContactLatches,
    /// `+60` pop, `+96` pop roll, `+56` landing.
    held_pop_60: Option<u32>,
    held_roll_96: Option<u32>,
    held_landing_56: Option<u32>,
    /// `sub_824B8D48`'s voice, retail slot `+52`.
    held_landing_class_52: Option<u32>,
    /// `sub_82497F48`'s ladder voice, retail slot `+496`.
    held_landing_ladder_496: Option<u32>,
    /// The Contacts controller inputs (vfunc 8 on `[this+12]`) this process set, in retail call
    /// order. `sub_824BA630`'s tail writes input 2 from the landing class; the worker applies
    /// them before the next MixMap evaluation, exactly as it already does for Rail.
    inputs: Vec<(u32, u32)>,
    voices: Box<dyn ContactVoices>,
}

impl ContactsOwner {
    // Called by the worker once `ContactsOwner` is registered on the Contacts controller.
    #[allow(dead_code)]
    pub(crate) fn new(
        vault: &Collections,
        local: bool,
        voices: Box<dyn ContactVoices>,
    ) -> Result<Self, String> {
        Ok(Self {
            tuning: ContactsTuning::load(vault)?,
            local,
            latches: ContactLatches::default(),
            held_pop_60: None,
            held_roll_96: None,
            held_landing_56: None,
            held_landing_class_52: None,
            held_landing_ladder_496: None,
            inputs: Vec::new(),
            voices,
        })
    }

    pub(crate) fn latches(&self) -> ContactLatches {
        self.latches
    }

    /// `sub_824B9CC8(this, audio_trick)`.
    fn pops(&mut self, inputs: &ContactsInputs) {
        let warm = self.latches.frames_424 < POP_WARMUP_FRAMES;
        // Retail frees the held voice and clears +60..+76 (including `sub_82489058` on the +72
        // sub-object, which is not ported) before deciding whether to start a new one.
        if let Some(handle) = self.held_pop_60.take() {
            self.voices.free(handle);
        }
        self.latches.pop_selector_64 = 0;
        if !warm {
            let mut selector = 0;
            if inputs.jump_velocity_468 > self.tuning.pop_hard {
                selector = 2;
            } else if inputs.jump_velocity_468 > self.tuning.pop_medium {
                selector = 1;
            }
            let trick = inputs.audio_trick_348 as i32;
            if trick == 33 || trick == 34 {
                selector = 0;
            }
            self.latches.pop_selector_64 = selector;
            // The (bank, sample) pair comes from the undecoded `sub_824B9AD8`; the sink resolves
            // it from the selector. The gain (vtable 60 id 14 × constant × vault) is not ported.
            let request = VoiceRequest {
                selector: Some(selector),
                eq_chain: Some(self.tuning.pop_eq_chain),
                ..VoiceRequest::default()
            };
            self.held_pop_60 = self.voices.play(ContactSound::Pop, &request);
        }
        if self.held_roll_96.is_none() {
            let speed = inputs.ground_speed_208;
            let sample = if speed > self.tuning.roll_speed_high {
                self.tuning.roll_sample_high
            } else if speed > self.tuning.roll_speed_mid {
                self.tuning.roll_sample_mid
            } else if speed > self.tuning.roll_speed_low {
                self.tuning.roll_sample_low
            } else {
                self.tuning.roll_sample_idle
            };
            let request = VoiceRequest {
                sample: Some(sample),
                ..VoiceRequest::default()
            };
            self.held_roll_96 = self.voices.play(ContactSound::PopRoll, &request);
        }
    }

    /// `sub_824BA630`.
    fn landing(&mut self, inputs: &ContactsInputs) {
        // `sub_824BA630` @ 0x824BA66C, before any voice: the landing pulse on input 1.
        self.latches.frames_424 = 0;
        self.inputs.push((1, 32_767));
        if !self.local {
            return;
        }
        if let Some(handle) = self.held_landing_56.take() {
            self.voices.free(handle);
        }
        let request = VoiceRequest {
            sample: Some(self.tuning.landing_sample),
            eq_chain: Some(self.tuning.landing_eq_chain),
            ..VoiceRequest::default()
        };
        self.held_landing_56 = self.voices.play(ContactSound::Landing, &request);
        // `sub_824BA630` @ 0x824BAC48: the second landing voice, held at `+496`. Retail releases
        // whatever is in the slot (`[[+496]]` vtable 0 with r4 = 1) before zeroing it, which is
        // the same free-then-play the other slots do here.
        //
        // Its sample is a 2×2 ladder — `sub_82494D78` of the deck material against time in air,
        // split at 0.75 s — and its level is fixed, so this is the third and last way retail
        // makes one landing sound unlike another. It was the one this port never played.
        if let Some(handle) = self.held_landing_ladder_496.take() {
            self.voices.free(handle);
        }
        self.held_landing_ladder_496 = self.voices.play(
            ContactSound::LandingLadder,
            &VoiceRequest {
                air_time: Some(self.latches.air_time_340),
                // Retail passes 0 to `sub_82494D78` when the deck-contact byte is clear.
                deck_material: inputs.deck_contact_614.then_some(inputs.deck_material_660),
                ..VoiceRequest::default()
            },
        );
        // `sub_824BA630` @ 0x824BB074 then calls `sub_824B8D48(this, 0, max_class, 0)`. Its sample
        // is picked from the landing class, so it — not the fixed-level impact above — is what
        // makes a heavy landing sound different from a light one. The sink resolves the class and
        // the surface category from the audio state, the way it already does for the pops.
        if let Some(handle) = self.held_landing_class_52.take() {
            self.voices.free(handle);
        }
        // `sub_824BA630` @ 0x824BAF08: the class is the largest `+448` bucket over the wheels
        // that are actually down (`+464`). Retail computes it **once** and uses it for both the
        // sample and the controller write, so it is computed here rather than in the sink.
        let mut class = 0;
        let mut landed = false;
        for wheel in 0..4 {
            if inputs.wheel_landed_464[wheel] {
                landed = true;
                class = class.max(inputs.wheel_landing_bucket_448[wheel]);
            }
        }
        if landed {
            self.held_landing_class_52 = self.voices.play(
                ContactSound::LandingClass,
                &VoiceRequest {
                    send_level: Some(inputs.landing_send_level_15),
                    air_time: Some(self.latches.air_time_340),
                    landing_class: Some(class),
                    ..VoiceRequest::default()
                },
            );
        }
        // `sub_824BA630`'s tail @ 0x824BAF44: the class as a controller word, written to input 2
        // of `[this+12]`. **This is the landing's level mechanism.** Input 2 drives Contacts
        // output 15 (the landing voice's owner send) and output 3; probing the real MixMap under
        // the retail pre-roll gives output 15 = 2584 / 3103 / 3650 for classes 0 / 1 / 2.
        //
        // Unlike inputs 0, 1 and 6 — which `sub_824B90D8` zeroes every frame — input 2 is **not**
        // reset, so the class latches until the next landing and the send holds for the voice's
        // whole life. Retail writes it even when no wheel landed, as 0.
        //
        // This port never wrote it at all, so output 15 sat at its class-0 value forever and the
        // landing class changed the sample but never the level.
        let word = match class {
            1 => 16_000,
            2 => 32_767,
            _ => 0,
        };
        self.inputs.push((2, if landed { word } else { 0 }));
    }

    /// `sub_824BB0E0`.
    fn grind_onset(&mut self, inputs: &ContactsInputs) {
        if self.latches.grind_gate_124 > 0.0 {
            return;
        }
        let material = if inputs.grind_material_692 == 143 {
            GRIND_MATERIAL_FALLBACK
        } else {
            inputs.grind_material_692
        };
        let family_base = if matches!(inputs.grind_family_192, 1 | 2 | 5) {
            GRIND_FAMILY_BASE_LOW
        } else {
            GRIND_FAMILY_BASE_HIGH
        };
        // Retail reads the 0.25 threshold, then a second 0.5 threshold whose variant is not
        // decoded; only the first tier is ported.
        let tier = i32::from(inputs.grind_impact_228 > self.tuning.grind_impact_threshold);
        let request = VoiceRequest {
            material: Some(material),
            family_base: Some(family_base),
            tier: Some(tier),
            impact: Some(inputs.grind_impact_228),
            ..VoiceRequest::default()
        };
        // This routine does not hold its voice in a slot the process reads back.
        let _ = self.voices.play(ContactSound::GrindOnset, &request);
    }

    /// `sub_824B90D8`.
    pub(crate) fn step(&mut self, inputs: &ContactsInputs) {
        // The head of `sub_824B90D8` zeroes inputs 0, 1 and 6 on its own controller every frame,
        // before anything else. Input **2** is pointedly not among them: the landing class stays
        // latched until the next landing.
        self.inputs.push((0, 0));
        self.inputs.push((1, 0));
        self.inputs.push((6, 0));
        self.latches.frames_424 = self.latches.frames_424.wrapping_add(1);
        let was_airborne = self.latches.airborne_120;
        let airborne = inputs.in_known_air_332;
        let grinding = inputs.grinding_341;
        if !was_airborne {
            let trick = inputs.audio_trick_348 as i32;
            if airborne
                && inputs.trick_active_343
                && !POP_BLOCKING_TRICKS.contains(&trick)
                && !self.latches.grinding_122
            {
                self.pops(inputs);
            }
        } else if !airborne && !grinding {
            self.landing(inputs);
        }
        if airborne {
            self.latches.air_time_340 = inputs.air_time_236;
        }
        self.latches.bail_344 = inputs.bail_676;
        self.latches.airborne_120 = airborne;
        if grinding && !self.latches.grinding_122 {
            self.grind_onset(inputs);
        }
        self.latches.grinding_122 = grinding;
    }
}

impl Component for ContactsOwner {
    /// The worker applies these to the Contacts controller before the next MixMap evaluation.
    fn take_owner_inputs(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.inputs)
    }

    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let mut inputs = ContactsInputs::from_state(tick.audio);
        inputs.landing_send_level_15 = tick.controls.level(15);
        self.step(&inputs);
        Ok(())
    }

    /// `sub_824B90D8` runs entirely in the process chain; there is no `+40` work and no packet
    /// to redeliver.
    fn update(&mut self, _tick: &mut Tick) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod capture {
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;

    use super::Controls;

    pub(crate) fn root() -> Option<PathBuf> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/captures/extract");
        root.join("state.tsv").exists().then_some(root)
    }

    fn slot(off: usize) -> usize {
        (off - 192) / 4
    }
    pub(crate) fn int(words: &[u32], off: usize) -> i32 {
        words[slot(off)] as i32
    }
    pub(crate) fn float(words: &[u32], off: usize) -> f32 {
        f32::from_bits(words[slot(off)])
    }
    pub(crate) fn byte(words: &[u32], off: usize) -> u8 {
        (words[slot(off)] >> (8 * (3 - (off - 192) % 4))) as u8
    }

    /// `state.tsv`: frame → the 160 words at state+192 (the last row of a frame wins).
    pub(crate) fn states(root: &PathBuf) -> BTreeMap<u32, Vec<u32>> {
        let text = std::fs::read_to_string(root.join("state.tsv")).unwrap();
        text.lines()
            .map(|line| {
                let mut parts = line.split('\t');
                let frame = parts.next().unwrap().parse().unwrap();
                parts.next();
                (
                    frame,
                    parts.map(|w| u32::from_str_radix(w, 16).unwrap()).collect(),
                )
            })
            .collect()
    }

    /// One `attributed/<object>.tsv` row.
    #[derive(Clone, Debug)]
    pub(crate) struct Row {
        pub kind: String,
        pub frame: u32,
        pub node: String,
        pub payload: String,
        pub words: Vec<u32>,
        pub ctrl: String,
        /// (fn, id, result, lr)
        pub reads: Vec<(u32, u32, u32, u32)>,
    }

    pub(crate) fn rows(root: &PathBuf, object: &str) -> Vec<Row> {
        let text =
            std::fs::read_to_string(root.join("attributed").join(format!("{object}.tsv"))).unwrap();
        text.lines()
            .map(|line| {
                let p: Vec<&str> = line.split('\t').collect();
                let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
                Row {
                    kind: p[0].into(),
                    frame: p[1].parse().unwrap(),
                    node: p[3].into(),
                    payload: p[4].into(),
                    words: if p[5] == "-" {
                        vec![]
                    } else {
                        p[5].split(' ').map(hex).collect()
                    },
                    ctrl: p[6].into(),
                    reads: p
                        .get(7)
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            s.split(',')
                                .map(|r| {
                                    let f: Vec<&str> = r.split(':').collect();
                                    (
                                        f[0].parse().unwrap(),
                                        f[1].parse().unwrap(),
                                        hex(f[2]),
                                        hex(f[3]),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect()
    }

    /// The controller reads one retail function made (return address inside `lr_range`).
    #[derive(Default)]
    pub(crate) struct Captured(pub HashMap<(u32, u32), u32>);

    impl Captured {
        pub(crate) fn from_reads(
            reads: &[(u32, u32, u32, u32)],
            lr_range: std::ops::Range<u32>,
        ) -> Self {
            Self(
                reads
                    .iter()
                    .filter(|r| lr_range.contains(&r.3))
                    .map(|&(f, id, result, _)| ((if f == 64 { 60 } else { f }, id), result))
                    .collect(),
            )
        }
    }

    impl Controls for Captured {
        fn raw(&self, id: u32) -> u32 {
            self.0.get(&(52, id)).copied().unwrap_or(0)
        }
        fn pitch(&self, id: u32) -> i32 {
            self.0.get(&(56, id)).copied().unwrap_or(0) as i32
        }
        fn level(&self, id: u32) -> u32 {
            self.0.get(&(60, id)).copied().unwrap_or(0)
        }
    }

    /// Per-word exact-match counters.
    pub(crate) struct Matches {
        pub name: &'static str,
        pub total: usize,
        pub exact: Vec<usize>,
        pub bad: Vec<Vec<(u32, u32, u32)>>,
    }

    impl Matches {
        pub(crate) fn new(name: &'static str, words: usize) -> Self {
            Self {
                name,
                total: 0,
                exact: vec![0; words],
                bad: vec![vec![]; words],
            }
        }
        pub(crate) fn add(&mut self, frame: u32, retail: &[u32], ours: &[u32]) {
            self.total += 1;
            for (i, ours) in ours.iter().enumerate() {
                if retail[i] == *ours {
                    self.exact[i] += 1;
                } else if self.bad[i].len() < 3 {
                    self.bad[i].push((frame, retail[i], *ours));
                }
            }
        }
        pub(crate) fn print(&self) {
            println!("{}: {} rows", self.name, self.total);
            for (i, exact) in self.exact.iter().enumerate() {
                println!(
                    "  w{i:<2} {exact}/{}  bad (frame, retail, ours) {:?}",
                    self.total, self.bad[i]
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::capture::{Captured, Matches};
    use super::*;

    /// Vault values (`skater-collections.json`), and an AudioSurfaceMap whose +20 lane is the
    /// vault's for the materials used below (0 → 1, 2 → 0, 94 → 0).
    fn tuning() -> FootDragTuning {
        let mut entries = vec![[0u32; 18]; 95];
        entries[0][5] = 1;
        FootDragTuning {
            speed_offset: 0.5,
            top_kmh: 50.0,
            levels: [4000, 4500, 4000, 22500],
            eq_brake: 7,
            eq_manual: 7,
            surfaces: SurfaceMap::from_entries(entries),
        }
    }

    struct Fixed(&'static [(u32, u32, u32)]);
    impl Controls for Fixed {
        fn raw(&self, id: u32) -> u32 {
            self.0
                .iter()
                .find(|r| r.0 == 52 && r.1 == id)
                .map_or(0, |r| r.2)
        }
        fn pitch(&self, id: u32) -> i32 {
            self.0
                .iter()
                .find(|r| r.0 == 56 && r.1 == id)
                .map_or(0, |r| r.2 as i32)
        }
        fn level(&self, id: u32) -> u32 {
            self.0
                .iter()
                .find(|r| r.0 == 60 && r.1 == id)
                .map_or(0, |r| r.2)
        }
    }

    #[test]
    fn foot_drag_posts_on_brake_and_hold_with_the_vault_levels() {
        let tuning = tuning();
        let mut inputs = FootDragInputs {
            brake_336: true,
            ground_speed_208: 0.25,
            ..Default::default()
        };
        assert!(foot_drag_active(&inputs, true));
        // Retail post at frame 2746: w7 0 (below the 0.5 m/s offset), w13 = !336 = 0.
        assert_eq!(
            foot_drag_constructor(&tuning, &inputs, true),
            [
                0, 32767, 0, 0, 0, 25000, 0, 0, 1, 4000, 4500, 4000, 22500, 0, 7
            ]
        );
        // Retail post at frame 6279: the local hold path, w7 = 500, w13 = 1.
        inputs = FootDragInputs {
            hold_expired_310: true,
            ground_speed_208: 9.0,
            wheel_material_620: 143,
            ..Default::default()
        };
        assert!(foot_drag_active(&inputs, true));
        assert!(!foot_drag_active(&inputs, false));
        assert_eq!(
            foot_drag_constructor(&tuning, &inputs, true),
            [
                0, 32767, 0, 0, 0, 25000, 0, 500, 0, 4000, 4500, 4000, 22500, 1, 7
            ]
        );
    }

    #[test]
    fn foot_drag_update_reads_the_contacts_controller() {
        let tuning = tuning();
        let inputs = FootDragInputs {
            hold_expired_310: true,
            wheel_material_620: 143,
            ..Default::default()
        };
        let mut words = foot_drag_constructor(&tuning, &inputs, true);
        // Retail update at frame 6280 (controller reads captured with it).
        let controls = Fixed(&[
            (60, 5, 0x2055),
            (60, 18, 0x332),
            (60, 16, 0x618B),
            (60, 17, 0x4D),
            (52, 0, 0xFDF4),
            (56, 22, 0x11FD),
        ]);
        foot_drag_update(&mut words, &tuning, &inputs, &controls, true);
        assert_eq!(
            words,
            [
                0x7FFF, 0x2055, 0x332, 0xFDF4, 0x11FD, 0x618B, 0x4D, 500, 0, 4000, 4500, 4000,
                22500, 1, 7
            ]
        );
    }

    #[test]
    fn foot_surface_uses_wheel_two_on_manual_brake_and_drops_surface_one() {
        let tuning = tuning();
        let mut inputs = FootDragInputs {
            wheel_material_620: 0,
            wheel_material_628: 0,
            ..Default::default()
        };
        assert_eq!(foot_surface(&inputs, &tuning.surfaces), 1);
        inputs.manual_brake_339 = true;
        assert_eq!(foot_surface(&inputs, &tuning.surfaces), 0);
        inputs.wheel_material_628 = 143;
        assert_eq!(foot_surface(&inputs, &tuning.surfaces), 0);
    }

    #[test]
    fn drag_speed_saturates_like_the_fsel_pair() {
        let tuning = tuning();
        let mut inputs = FootDragInputs {
            brake_336: true,
            ground_speed_208: 100.0,
            ..Default::default()
        };
        assert_eq!(foot_drag_constructor(&tuning, &inputs, true)[7], 10_000);
        inputs.ground_speed_208 = f32::NAN;
        // fsel(−NaN) takes NaN, then fsel(1 − NaN) takes 1.0.
        assert_eq!(foot_drag_constructor(&tuning, &inputs, true)[7], 10_000);
        inputs.ground_speed_208 = 5.0;
        // (5 − 0.5) / 50 × 3.6 = 0.324 → 3240.
        assert_eq!(foot_drag_constructor(&tuning, &inputs, true)[7], 3240);
    }

    fn vault() -> Option<Collections> {
        let root = std::path::PathBuf::from(
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets",
        );
        root.join("private/stock/skater-collections.json")
            .exists()
            .then(|| Collections::load(&root).unwrap())
    }

    #[test]
    fn foot_drag_tuning_reads_the_vault() {
        let Some(vault) = vault() else { return };
        let tuning = FootDragTuning::load(&vault).unwrap();
        assert_eq!((tuning.speed_offset, tuning.top_kmh), (0.5, 50.0));
        assert_eq!(tuning.levels, [4000, 4500, 4000, 22500]);
        assert_eq!((tuning.eq_brake, tuning.eq_manual), (7, 7));
        // Element 94 (every material ≥ 94) has foot drag surface 0.
        assert_eq!(tuning.surfaces.lookup(200, 20), 0);
        // Element 0 (material 0) is 0, element 2 is 1 (stock `4CA607558B1CF440`).
        assert_eq!(tuning.surfaces.lookup(0, 20), 0);
        assert_eq!(tuning.surfaces.lookup(2, 20), 1);
    }

    /// Replays the retail capture: runs the component's trigger and updater over every captured
    /// frame (state from `lag` frames earlier) in retail's observed order (a post's first
    /// redelivery is on the next frame, so the frame's update runs before its process) and
    /// compares posts, releases and redelivered words.
    #[test]
    #[ignore = "needs .local/captures"]
    fn foot_drag_replays_the_retail_capture() {
        let Some(root) = super::capture::root() else {
            return;
        };
        let Some(vault) = vault() else { return };
        let tuning = FootDragTuning::load(&vault).unwrap();
        let states = super::capture::states(&root);
        let rows = super::capture::rows(&root, OBJECT);
        const LOCAL: &str = "4A26A8B0";
        // The updater reads the previous frame's state row, the trigger the current one.
        for (update_lag, process_lag) in [(1u32, 0u32), (1, 1), (0, 0)] {
            let mut matches = Matches::new("foot drag updates", FOOT_DRAG_WORDS);
            let mut post_matches = Matches::new("foot drag posts", FOOT_DRAG_WORDS);
            let retail_posts: Vec<_> = rows.iter().filter(|r| r.kind == "PO").collect();
            let local_nodes: std::collections::HashSet<_> = rows
                .iter()
                .filter(|r| r.kind == "UP" && r.ctrl == LOCAL)
                .map(|r| r.node.clone())
                .collect();
            let retail_releases: Vec<u32> = rows
                .iter()
                .filter(|r| r.kind == "RL" && local_nodes.contains(&r.node))
                .map(|r| r.frame)
                .collect();
            let updates: std::collections::HashMap<u32, &super::capture::Row> = rows
                .iter()
                .filter(|r| r.kind == "UP" && r.ctrl == LOCAL)
                .map(|r| (r.frame, r))
                .collect();
            let mut held: Option<[u32; FOOT_DRAG_WORDS]> = None;
            let (mut our_posts, mut our_releases, mut our_updates) = (vec![], vec![], vec![]);
            for (&frame, _) in states.range(2709..) {
                let (Some(earlier), Some(state)) = (
                    states.get(&(frame - update_lag)),
                    states.get(&(frame - process_lag)),
                ) else {
                    continue;
                };
                let inputs = FootDragInputs::from_capture(earlier);
                // Update.
                if let Some(words) = held.as_mut() {
                    if !foot_drag_active(&inputs, true) {
                        held = None;
                        our_releases.push(frame);
                    } else {
                        our_updates.push(frame);
                        if let Some(row) = updates.get(&frame) {
                            let controls =
                                Captured::from_reads(&row.reads, 0x824B_EEE8..0x824B_F268);
                            foot_drag_update(words, &tuning, &inputs, &controls, true);
                            matches.add(frame, &row.words, words);
                        }
                    }
                }
                // Process.
                let inputs = FootDragInputs::from_capture(state);
                if held.is_none() && foot_drag_active(&inputs, true) {
                    let words = foot_drag_constructor(&tuning, &inputs, true);
                    if let Some(row) = retail_posts.iter().find(|r| r.frame == frame) {
                        post_matches.add(frame, &row.words, &words);
                    }
                    held = Some(words);
                    our_posts.push(frame);
                }
            }
            println!("update lag {update_lag}, process lag {process_lag}");
            post_matches.print();
            matches.print();
            let retail_post_frames: Vec<u32> = retail_posts.iter().map(|r| r.frame).collect();
            let retail_update_frames: Vec<u32> = {
                let mut f: Vec<u32> = updates.keys().copied().collect();
                f.sort();
                f
            };
            println!("posts   retail {retail_post_frames:?}\n        ours   {our_posts:?}");
            println!("releases retail {retail_releases:?}\n         ours   {our_releases:?}");
            println!(
                "update frames: retail {} ours {} common {}",
                retail_update_frames.len(),
                our_updates.len(),
                our_updates
                    .iter()
                    .filter(|f| updates.contains_key(f))
                    .count()
            );
        }
    }
}

#[cfg(test)]
mod owner_tests {
    use super::capture;
    use super::*;

    /// Records what the retail routines asked the bank-voice sink for.
    #[derive(Default)]
    struct Recorder {
        played: Vec<(ContactSound, VoiceRequest)>,
        freed: Vec<u32>,
        next: u32,
    }

    /// A sink shared with the test, since the component owns its sink.
    #[derive(Clone, Default)]
    struct SharedRecorder(std::rc::Rc<std::cell::RefCell<Recorder>>);

    impl ContactVoices for SharedRecorder {
        fn play(&mut self, sound: ContactSound, request: &VoiceRequest) -> Option<u32> {
            let mut r = self.0.borrow_mut();
            r.next += 1;
            let handle = r.next;
            r.played.push((sound, *request));
            Some(handle)
        }
        fn free(&mut self, handle: u32) {
            self.0.borrow_mut().freed.push(handle);
        }
    }

    impl SharedRecorder {
        fn sounds(&self) -> Vec<ContactSound> {
            self.0.borrow().played.iter().map(|(s, _)| *s).collect()
        }
        fn take(&self) -> Vec<(ContactSound, VoiceRequest)> {
            std::mem::take(&mut self.0.borrow_mut().played)
        }
        fn kinds(&self) -> Vec<ContactSound> {
            self.take().iter().map(|(s, _)| *s).collect()
        }
        fn freed(&self) -> Vec<u32> {
            self.0.borrow().freed.clone()
        }
    }

    /// The stock vault values, so the unit tests need no assets;
    /// `contacts_owner_tuning_reads_the_vault` checks them against the real vault.
    fn tuning() -> ContactsTuning {
        ContactsTuning {
            pop_hard: f32::from_bits(0x3ED7_0A3D),
            pop_medium: f32::from_bits(0x3E80_0000),
            pop_eq_chain: 0,
            roll_speed_high: 12.0,
            roll_speed_mid: 8.0,
            roll_speed_low: 4.0,
            roll_sample_high: 1111,
            roll_sample_mid: 1111,
            roll_sample_low: 1111,
            roll_sample_idle: 1111,
            landing_sample: 1095,
            landing_eq_chain: 1,
            grind_impact_threshold: 0.25,
        }
    }

    fn owner(local: bool) -> (ContactsOwner, SharedRecorder) {
        let sink = SharedRecorder::default();
        let owner = ContactsOwner {
            tuning: tuning(),
            local,
            latches: ContactLatches::default(),
            held_pop_60: None,
            held_roll_96: None,
            held_landing_56: None,
            held_landing_class_52: None,
            held_landing_ladder_496: None,
            inputs: Vec::new(),
            voices: Box::new(sink.clone()),
        };
        (owner, sink)
    }

    /// Airborne with a trick active and a pop-permitting trick id.
    /// A grounded frame with the wheels actually down. `sub_824BA630` resolves its landing
    /// class from the `+464` wheels, so a landing with no wheel down plays no class voice and
    /// writes 0 to controller input 2 — which is retail's behaviour, not a fixture accident.
    fn grounded() -> ContactsInputs {
        ContactsInputs {
            wheel_landed_464: [true; 4],
            ..ContactsInputs::default()
        }
    }

    fn air(trick: u32) -> ContactsInputs {
        ContactsInputs {
            in_known_air_332: true,
            trick_active_343: true,
            audio_trick_348: trick,
            ..ContactsInputs::default()
        }
    }

    #[test]
    fn pop_needs_the_airborne_rising_edge_the_trick_flag_and_a_permitted_trick() {
        // Warm-up: the first processed frame has +424 == 1, so the pop voice is skipped while
        // the roll voice still starts.
        let (mut o, sink) = owner(true);
        o.step(&air(5));
        assert_eq!(sink.sounds(), vec![ContactSound::PopRoll]);

        // A proper edge on a later frame plays both.
        let (mut o, sink) = owner(true);
        o.step(&ContactsInputs::default());
        o.step(&air(5));
        assert_eq!(sink.kinds(), vec![ContactSound::Pop, ContactSound::PopRoll]);
        // Still airborne: no new edge.
        o.step(&air(5));
        assert!(sink.take().is_empty());

        // Blocked trick ids fire nothing at all.
        for trick in [31u32, 32, 35, 36, u32::MAX] {
            let (mut o, sink) = owner(true);
            o.step(&ContactsInputs::default());
            o.step(&air(trick));
            assert!(sink.take().is_empty(), "trick {trick} should block the pop");
        }

        // +343 clear blocks it too.
        let (mut o, sink) = owner(true);
        o.step(&ContactsInputs::default());
        o.step(&ContactsInputs {
            trick_active_343: false,
            ..air(5)
        });
        assert!(sink.take().is_empty());

        // Previously grinding blocks it (latch +122).
        let (mut o, sink) = owner(true);
        o.step(&ContactsInputs {
            grinding_341: true,
            ..ContactsInputs::default()
        });
        sink.take();
        o.step(&air(5));
        assert!(sink.take().is_empty());
    }

    #[test]
    fn pop_selector_walks_the_jump_velocity_thresholds() {
        for (velocity, expected) in [(0.0f32, 0), (0.3, 1), (0.5, 2)] {
            let (mut o, sink) = owner(true);
            o.step(&ContactsInputs::default());
            o.step(&ContactsInputs {
                jump_velocity_468: velocity,
                ..air(5)
            });
            let played = sink.take();
            let (_, pop) = played
                .iter()
                .find(|(s, _)| *s == ContactSound::Pop)
                .unwrap();
            assert_eq!(pop.selector, Some(expected), "velocity {velocity}");
            assert_eq!(o.latches().pop_selector_64, expected);
            assert_eq!(pop.eq_chain, Some(0));
        }
        // Tricks 33 and 34 force the softest selector however hard the pop was.
        for trick in [33u32, 34] {
            let (mut o, sink) = owner(true);
            o.step(&ContactsInputs::default());
            o.step(&ContactsInputs {
                jump_velocity_468: 5.0,
                ..air(trick)
            });
            let played = sink.take();
            let (_, pop) = played
                .iter()
                .find(|(s, _)| *s == ContactSound::Pop)
                .unwrap();
            assert_eq!(pop.selector, Some(0));
        }
    }

    #[test]
    fn pop_roll_sample_walks_the_speed_tiers_and_is_held_once() {
        let mut t = tuning();
        t.roll_sample_high = 10;
        t.roll_sample_mid = 20;
        t.roll_sample_low = 30;
        t.roll_sample_idle = 40;
        for (speed, expected) in [(20.0f32, 10), (9.0, 20), (5.0, 30), (1.0, 40)] {
            let (mut o, sink) = owner(true);
            o.tuning = t;
            o.step(&ContactsInputs::default());
            o.step(&ContactsInputs {
                ground_speed_208: speed,
                ..air(5)
            });
            let played = sink.take();
            let (_, roll) = played
                .iter()
                .find(|(s, _)| *s == ContactSound::PopRoll)
                .unwrap();
            assert_eq!(roll.sample, Some(expected), "speed {speed}");
        }
        // The roll voice is held at +96: coming down lands, and the next pop frees only the
        // +60 pop voice (handle 1) and does not restart the roll. The landing zeroes +424, so
        // the pop needs one more grounded frame to clear the warm-up gate.
        let (mut o, sink) = owner(true);
        o.step(&ContactsInputs::default());
        o.step(&air(5));
        sink.take();
        o.step(&grounded());
        o.step(&ContactsInputs::default());
        o.step(&air(5));
        assert_eq!(
            sink.kinds(),
            vec![
                ContactSound::Landing,
                ContactSound::LandingLadder,
                ContactSound::LandingClass,
                ContactSound::Pop
            ]
        );
        assert_eq!(sink.freed(), vec![1]);
    }

    /// The landing's *level* mechanism, and the one this port was missing entirely: retail's
    /// `sub_824BA630` writes the landing class to Contacts controller input 2, which drives
    /// output 15 — the landing voice's owner send. With it unwritten, output 15 never left its
    /// class-0 value and every landing came out at the same level.
    #[test]
    fn a_landing_writes_its_class_to_controller_input_2_and_latches_it() {
        for (bucket, expected) in [(0u32, 0u32), (1, 16_000), (2, 32_767)] {
            let (mut o, _sink) = owner(true);
            o.step(&air(5));
            o.take_owner_inputs();
            o.step(&ContactsInputs {
                wheel_landing_bucket_448: [bucket; 4],
                ..grounded()
            });
            let inputs = o.take_owner_inputs();
            // The per-frame resets come first (`sub_824B90D8`'s head), then the landing pulse on
            // input 1, then the class on input 2.
            assert_eq!(inputs[0], (0, 0), "bucket {bucket}");
            assert_eq!(inputs[1], (1, 0), "bucket {bucket}");
            assert_eq!(inputs[2], (6, 0), "bucket {bucket}");
            assert!(
                inputs.contains(&(1, 32_767)),
                "landing pulse, bucket {bucket}"
            );
            assert_eq!(
                inputs.last(),
                Some(&(2, expected)),
                "class {bucket} should write {expected} to input 2"
            );

            // Input 2 is *not* in the per-frame reset list, so the next ordinary frame leaves the
            // class latched — that is what holds the send up for the voice's whole life.
            o.step(&ContactsInputs::default());
            let next = o.take_owner_inputs();
            assert_eq!(next, vec![(0, 0), (1, 0), (6, 0)], "bucket {bucket}");
        }
    }

    /// A landing with no wheel down plays no class voice and writes 0, rather than inventing a
    /// class from stale buckets.
    #[test]
    fn a_landing_with_no_wheel_down_writes_zero_and_plays_no_class_voice() {
        let (mut o, sink) = owner(true);
        o.step(&air(5));
        sink.take();
        o.take_owner_inputs();
        o.step(&ContactsInputs {
            wheel_landing_bucket_448: [2; 4],
            ..ContactsInputs::default()
        });
        assert!(!sink.kinds().contains(&ContactSound::LandingClass));
        assert_eq!(o.take_owner_inputs().last(), Some(&(2, 0)));
    }

    #[test]
    fn landing_resets_the_frame_counter_so_the_next_pop_is_gated() {
        // A pop on the frame straight after a landing is suppressed, because the landing set
        // +424 to 0 and `sub_824B9CC8` wants +424 >= 2.
        let (mut o, sink) = owner(true);
        o.step(&ContactsInputs::default());
        o.step(&air(5));
        sink.take();
        o.step(&grounded());
        o.step(&air(5));
        assert_eq!(
            sink.kinds(),
            vec![
                ContactSound::Landing,
                ContactSound::LandingLadder,
                ContactSound::LandingClass
            ]
        );
    }

    #[test]
    fn landing_fires_on_the_falling_edge_only_when_not_grinding_and_local() {
        let (mut o, sink) = owner(true);
        o.step(&air(5));
        sink.take();
        // Falling edge, not grinding: the landing voice, and the frame counter resets.
        o.step(&grounded());
        let played = sink.take();
        // The impact, the `+496` ladder voice and `sub_824B8D48`'s class voice, in retail's
        // order. The ladder one was missing entirely until it was ported.
        assert_eq!(played.len(), 3);
        assert_eq!(played[0].0, ContactSound::Landing);
        assert_eq!(played[0].1.sample, Some(1095));
        assert_eq!(played[0].1.eq_chain, Some(1));
        assert_eq!(played[1].0, ContactSound::LandingLadder);
        assert_eq!(played[2].0, ContactSound::LandingClass);
        assert_eq!(o.latches().frames_424, 0);

        // Landing into a grind plays no landing voice; the grind onset takes over.
        let (mut o, sink) = owner(true);
        o.step(&air(5));
        sink.take();
        o.step(&ContactsInputs {
            grinding_341: true,
            ..ContactsInputs::default()
        });
        assert_eq!(sink.kinds(), vec![ContactSound::GrindOnset]);

        // A remote skater gets no landing voice, but the counter still resets.
        let (mut o, sink) = owner(false);
        o.step(&air(5));
        sink.take();
        o.step(&ContactsInputs::default());
        assert!(sink.take().is_empty());
        assert_eq!(o.latches().frames_424, 0);
    }

    #[test]
    fn grind_onset_uses_the_material_family_base_and_impact_tier() {
        let (mut o, sink) = owner(true);
        o.step(&ContactsInputs {
            grinding_341: true,
            grind_material_692: 143,
            grind_family_192: 2,
            grind_impact_228: 0.5,
            ..ContactsInputs::default()
        });
        let played = sink.take();
        assert_eq!(played[0].0, ContactSound::GrindOnset);
        // 143 becomes 10, family 2 uses base 95, impact 0.5 over 0.25 gives tier 1.
        assert_eq!(played[0].1.material, Some(10));
        assert_eq!(played[0].1.family_base, Some(95));
        assert_eq!(played[0].1.tier, Some(1));

        // Another family and a quiet impact.
        let (mut o, sink) = owner(true);
        o.step(&ContactsInputs {
            grinding_341: true,
            grind_material_692: 7,
            grind_family_192: 4,
            grind_impact_228: 0.1,
            ..ContactsInputs::default()
        });
        let played = sink.take();
        assert_eq!(
            (
                played[0].1.material,
                played[0].1.family_base,
                played[0].1.tier
            ),
            (Some(7), Some(96), Some(0))
        );

        // Only the rising edge fires.
        o.step(&ContactsInputs {
            grinding_341: true,
            ..ContactsInputs::default()
        });
        assert!(sink.take().is_empty());
    }

    #[test]
    fn latches_track_the_retail_members() {
        let (mut o, _sink) = owner(true);
        o.step(&ContactsInputs {
            air_time_236: 1.5,
            bail_676: true,
            ..air(5)
        });
        let l = o.latches();
        assert_eq!(
            (l.airborne_120, l.bail_344, l.air_time_340),
            (true, true, 1.5)
        );
        assert_eq!(l.frames_424, 1);
        // On the ground the air-time latch keeps its last airborne value.
        o.step(&ContactsInputs {
            air_time_236: 9.0,
            ..ContactsInputs::default()
        });
        assert_eq!(o.latches().air_time_340, 1.5);
        assert!(!o.latches().airborne_120);
    }

    fn owner_vault() -> Option<Collections> {
        let root =
            std::path::PathBuf::from(r"C:\s3\installations8eda9dc4644496d81ae73af95ff4285ssets");
        root.join("private/stock/skater-collections.json")
            .exists()
            .then(|| Collections::load(&root).unwrap())
    }

    /// Skipped without the assets.
    #[test]
    fn contacts_owner_tuning_reads_the_vault() {
        let Some(v) = owner_vault() else { return };
        assert_eq!(ContactsTuning::load(&v).unwrap(), tuning());
    }

    /// The trigger frames are the only part of these sounds the capture can check, since they
    /// come purely from state.tsv edges. This replays every captured frame, reports the pop /
    /// landing / grind-onset frames and asserts each one sits on a real edge.
    #[test]
    #[ignore = "needs .local/captures"]
    fn contacts_owner_edges_match_the_capture() {
        let Some(root) = capture::root() else { return };
        let states = capture::states(&root);
        let sink = SharedRecorder::default();
        let mut o = ContactsOwner {
            tuning: tuning(),
            local: true,
            latches: ContactLatches::default(),
            held_pop_60: None,
            held_roll_96: None,
            held_landing_56: None,
            held_landing_class_52: None,
            held_landing_ladder_496: None,
            inputs: Vec::new(),
            voices: Box::new(sink.clone()),
        };
        let mut frames: Vec<(u32, ContactSound)> = Vec::new();
        for (&frame, words) in &states {
            let inputs = ContactsInputs::from_capture(words);
            o.step(&inputs);
            for (sound, _) in sink.take() {
                frames.push((frame, sound));
            }
        }
        let count = |what: ContactSound| frames.iter().filter(|(_, s)| *s == what).count();
        let list = |what: ContactSound| {
            frames
                .iter()
                .filter(|(_, s)| *s == what)
                .map(|(f, _)| *f)
                .collect::<Vec<_>>()
        };
        println!(
            "pops        {} {:?}",
            count(ContactSound::Pop),
            list(ContactSound::Pop)
        );
        println!("pop rolls   {}", count(ContactSound::PopRoll));
        println!(
            "landings    {} {:?}",
            count(ContactSound::Landing),
            list(ContactSound::Landing)
        );
        println!(
            "grind onset {} {:?}",
            count(ContactSound::GrindOnset),
            list(ContactSound::GrindOnset)
        );
        // Every trigger must sit on a real edge of the captured state.
        for (frame, sound) in &frames {
            let now = ContactsInputs::from_capture(&states[frame]);
            let previous = states
                .range(..frame)
                .next_back()
                .map(|(_, w)| ContactsInputs::from_capture(w))
                .unwrap_or_default();
            match sound {
                ContactSound::Pop | ContactSound::PopRoll => {
                    assert!(
                        now.in_known_air_332 && !previous.in_known_air_332,
                        "frame {frame}"
                    );
                    assert!(now.trick_active_343, "frame {frame}");
                }
                // `sub_824B8D48`'s voice is played from the same `sub_824BA630` call as the
                // impact, so it sits on the same landing edge of the captured state.
                ContactSound::Landing
                | ContactSound::LandingClass
                | ContactSound::LandingLadder => {
                    assert!(
                        !now.in_known_air_332 && previous.in_known_air_332,
                        "frame {frame}"
                    );
                    assert!(!now.grinding_341, "frame {frame}");
                }
                ContactSound::GrindOnset => {
                    assert!(now.grinding_341 && !previous.grinding_341, "frame {frame}");
                }
            }
        }
        assert!(
            count(ContactSound::Landing) > 0,
            "the capture should contain landings"
        );
    }
}
