//! playercharacter_footstep ×2 (OffBoard controller 40010090), owned by the player's off-board
//! sound object.
//!
//! Retail (lifted C++, `skate3_recomp.11/12.cpp`):
//!
//! | Function | Role | Rust |
//! |---|---|---|
//! | `sub_824E9270` | owner process (`+36` tick): curves, foot-down copies, countdowns, then the poster, `sub_824E9678`, prev-frame stores | [`FootstepOwner::process`] |
//! | `sub_82481E10` | 16-point piecewise-linear curve | [`Curve::evaluate`] |
//! | `sub_824E9FD8` | poster: foot surfaces, creates both packets once (holder `+220` first, then `+36`) | [`FootstepOwner::process`], [`constructor_words`] |
//! | `sub_824B73E0` | constructor: 104-byte object, 25 words, clamps, posts object+4 to slot `0x8302EEE8` | [`constructor_words`] |
//! | `sub_824E9678` | jump / landing sample voices; its `+444` voice pointer feeds word 11 | [`FootstepOwner::jump_voice`] |
//! | `sub_824EAEA8` | updater (`+40` tick): rewrites and redelivers `+36`, then `+220` | [`update_words`] |
//! | `sub_82494F58` | AudioSurfaceMap entry `+24` of a material (entry 94 outside 0..93) | [`FootstepTuning::surface_step`] |
//! | `sub_824ADF90` | vault int array `636464FBAD0D71A3` element *i* | [`FootstepTuning::gains`] |
//!
//! Holder `+36` ("foot A") is driven by audio-state `+724` (right foot down), foot material
//! `+732`, foot speeds `+288` / `+296`; holder `+220` ("foot B") by `+725`, `+728`, `+284` / `+292`.
//!
//! Not ported (no packet effect): the bank sample voices the owner plays beside the packets
//! (`sub_82975700` objects at owner `+60/+100/+140/+180/+244/+284/+324/+364` via
//! `sub_82493E60`/`sub_82493690`, walking voices `+428..+440` in `sub_824E9D10`, the landing
//! voice `+448`, `sub_824EBA08`, `sub_824EBB58`). Only the *existence* of the jump voice `+444`
//! matters to the packets (word 11) and is ported as a latch.
//!
//! `sub_824E9270` also writes the OffBoard controller *input* id 0 (`[owner+12]` vfunc `+8`,
//! Set) = `+716 ? 32767 : 0` every processed frame, before the packet logic;
//! [`Component::take_owner_inputs`] hands those writes to the worker.

use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::words::fctiwz;
use super::{Component, Controls, Tick, post, redeliver};

/// The patch object both packets post to.
pub(crate) const OBJECT: &str = "playercharacter_footstep";
/// `sub_824B73E0` allocates 104 bytes and posts object+4: 25 words.
pub(crate) const WORDS: usize = 25;

/// Holder `*(0x830CFDA4)` +92 (see `tuning.rs`): the curves, the gain array.
const TUNING_CLASS: &str = "Hash_C1831BDB6CB1B1EA";
const TUNING_KEY: &str = "Hash_1ABD2984D7248589";
/// `sub_824E9270`: `Sk8::PointNegGraphData16` curves, read by `sub_82481E10` with 16 points
/// (x at +16, y at +80).
const SPEED_CURVE: &str = "Hash_C3CD069BB1B16B58"; // of +212 → owner+408
const FOOT_SPEED_CURVE: &str = "Hash_CF844597AB96EAF8"; // of +288 → +412, +284 → +416
const FOOT_VERTICAL_CURVE: &str = "Hash_236311604A3C1FB5"; // of +296 → +420, +292 → +424
/// `sub_824EAEA8` words 18..23: `sub_824ADF90(holder+92, i)`.
const GAINS: &str = "Hash_636464FBAD0D71A3";
/// Holder +64 AudioSurfaceMap (`sub_82484198` / `sub_82494F58`): 95 entries × 72 bytes.
const SURFACE_CLASS: &str = "Hash_C1831BDB6CB1B1EA";
const SURFACE_KEY: &str = "Hash_C489459A0C07D154";
const SURFACE_FIELD: &str = "Hash_4CA607558B1CF440";
/// Holder +140 eEQChain, `sub_824E9FD8`: word 24 = this + 10.
const EQ_CLASS: &str = "Hash_42AFE160E647167C";
const EQ_KEY: &str = "default";
const EQ_FIELD: &str = "Hash_C014A21D0FF6EDBA";

/// `0x82165A10`: the curve's minimum segment width.
const ZERO: f32 = 0.0;
/// `sub_824E9FD8`: surface used when the foot material is none (143).
const DEFAULT_SURFACE: u32 = 3;
/// `sub_824E9270`: frames word 11 stays up after a footplant ends.
const FOOTPLANT_COUNTDOWN: i32 = 10;
const NO_MATERIAL: u32 = 143;

/// A `Sk8::PointNegGraphData16`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Curve {
    pub xs: [f32; 16],
    pub ys: [f32; 16],
}

impl Curve {
    /// `sub_82481E10(n = 16, xs, ys, x)`.
    pub(crate) fn evaluate(&self, x: f32) -> f32 {
        let (xs, ys) = (&self.xs, &self.ys);
        let n = xs.len();
        if x < xs[0] {
            return ys[0];
        }
        if !(x < xs[n - 1]) {
            return ys[n - 1];
        }
        for i in 1..n {
            if x < xs[i] {
                let width = xs[i] - xs[i - 1];
                return if width > ZERO {
                    let slope = (ys[i] - ys[i - 1]) / width;
                    slope.mul_add(x - xs[i - 1], ys[i - 1])
                } else {
                    ys[i]
                };
            }
        }
        // loc_82481E68 (unreachable with x < xs[n-1] unless NaN compares fall through).
        ys[0]
    }

    fn from_vault(vault: &Collections, field: &str) -> Result<Self, String> {
        let entry = vault.field(TUNING_CLASS, TUNING_KEY, field)?;
        if entry.type_name != "Sk8::PointNegGraphData16" {
            return Err(format!("footstep curve {field} is {}", entry.type_name));
        }
        let words = vault.words::<36>(TUNING_CLASS, TUNING_KEY, field)?;
        Ok(Self {
            xs: std::array::from_fn(|i| f32::from_bits(words[4 + i])),
            ys: std::array::from_fn(|i| f32::from_bits(words[20 + i])),
        })
    }
}

/// Vault values the footstep code reads, all at runtime.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FootstepTuning {
    pub speed_curve: Curve,
    pub foot_speed_curve: Curve,
    pub foot_vertical_curve: Curve,
    /// Words 18..23, `636464FBAD0D71A3` elements 0..5 (signed).
    pub gains: [i32; 6],
    /// AudioSurfaceMap entry `+24` for entries 0..94.
    pub surface_step: Vec<u32>,
    /// eEQChain `C014A21D0FF6EDBA`.
    pub eq_chain: u32,
}

impl FootstepTuning {
    pub(crate) fn from_vault(vault: &Collections) -> Result<Self, String> {
        let gains = array_words(vault, TUNING_CLASS, TUNING_KEY, GAINS, 4)?;
        let gains: [i32; 6] = gains
            .iter()
            .map(|w| w[0] as i32)
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| format!("footstep gains {GAINS} need 6 elements"))?;
        let surfaces = array_words(vault, SURFACE_CLASS, SURFACE_KEY, SURFACE_FIELD, 72)?;
        if surfaces.len() < 95 {
            return Err(format!(
                "AudioSurfaceMap has {} entries, need 95",
                surfaces.len()
            ));
        }
        Ok(Self {
            speed_curve: Curve::from_vault(vault, SPEED_CURVE)?,
            foot_speed_curve: Curve::from_vault(vault, FOOT_SPEED_CURVE)?,
            foot_vertical_curve: Curve::from_vault(vault, FOOT_VERTICAL_CURVE)?,
            gains,
            surface_step: surfaces.iter().map(|entry| entry[6]).collect(),
            eq_chain: vault.words::<1>(EQ_CLASS, EQ_KEY, EQ_FIELD)?[0],
        })
    }

    /// `sub_82494F58`: entry `material` if 0..=93, else entry 94; field `+24`.
    pub(crate) fn surface_step(&self, material: u32) -> u32 {
        let material = material as i32;
        let index = if (0..94).contains(&material) {
            material as usize
        } else {
            94
        };
        self.surface_step[index]
    }
}

/// An array attribute's elements as big-endian words.
fn array_words(
    vault: &Collections,
    class: &str,
    key: &str,
    name: &str,
    element_size: usize,
) -> Result<Vec<Vec<u32>>, String> {
    let field = vault.field(class, key, name)?;
    let array = field
        .array
        .as_ref()
        .ok_or_else(|| format!("{class}/{key}/{name} is not an array"))?;
    array
        .items
        .iter()
        .map(|item| {
            let hex: String = item.chars().filter(|c| !c.is_whitespace()).collect();
            if hex.len() != element_size * 2 {
                return Err(format!("{name}: element of {} hex digits", hex.len()));
            }
            (0..element_size / 4)
                .map(|i| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).map_err(|e| e.to_string()))
                .collect()
        })
        .collect()
}

/// The audio-state fields the footstep code reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FootstepInputs {
    /// +100: COM velocity y (`sub_824E9678`, the `+456` falling edge).
    pub com_velocity_y_100: f32,
    pub com_speed_212: f32,
    pub foot_world_speed_xz_284: f32,
    pub foot_world_speed_xz_288: f32,
    /// +292 / +296: |Skeleton+324| / |Skeleton+308|: |Y| of the angular velocity (body+48) of
    /// ragdoll parts 16 / 20 (`AudioState::ragdoll_spin_y_*`).
    pub foot_vertical_speed_292: f32,
    pub foot_vertical_speed_296: f32,
    /// +300 landing bucket, compared signed.
    pub landing_bucket_300: i32,
    pub hippy_jump_372: bool,
    pub walking_716: bool,
    pub offboard_air_718: bool,
    pub foot_down_right_724: bool,
    pub foot_down_left_725: bool,
    pub foot_material_728: u32,
    pub foot_material_732: u32,
    /// +740 step code 1..5 (`sub_827729B8`), compared signed.
    pub step_code_740: i32,
    pub footplant_768: bool,
    pub footstep_strength_796: f32,
}

impl FootstepInputs {
    /// Every field retail reads is in [`AudioState`]; `Option` is kept for the call sites.
    pub(crate) fn from_state(state: &AudioState) -> Option<Self> {
        Some(Self {
            com_velocity_y_100: state.com_velocity_96[1],
            com_speed_212: state.com_speed_212,
            foot_world_speed_xz_284: state.foot_world_speed_xz_284,
            foot_world_speed_xz_288: state.foot_world_speed_xz_288,
            foot_vertical_speed_292: state.ragdoll_spin_y_292,
            foot_vertical_speed_296: state.ragdoll_spin_y_296,
            landing_bucket_300: state.landing_bucket_300 as i32,
            hippy_jump_372: state.hippy_jump_372,
            walking_716: state.walking_716,
            offboard_air_718: state.offboard_air_718,
            foot_down_right_724: state.foot_down_right_724,
            foot_down_left_725: state.foot_down_left_725,
            foot_material_728: state.foot_material_728,
            foot_material_732: state.foot_material_732,
            step_code_740: state.step_code_740 as i32,
            footplant_768: state.footplant_768,
            footstep_strength_796: state.footstep_strength_796,
        })
    }

    /// The capture's 160 state words from +192. +100 is below the dump, so the COM velocity y
    /// is supplied separately (`None` = unknown: see the replay test).
    #[cfg(test)]
    pub(crate) fn from_capture(words: &[u32], com_velocity_y_100: f32) -> Self {
        let word = |offset: usize| words[(offset - 192) / 4];
        let float = |offset: usize| f32::from_bits(word(offset));
        let byte = |offset: usize| (word(offset & !3) >> (8 * (3 - (offset & 3)))) & 0xFF != 0;
        Self {
            com_velocity_y_100,
            com_speed_212: float(212),
            foot_world_speed_xz_284: float(284),
            foot_world_speed_xz_288: float(288),
            foot_vertical_speed_292: float(292),
            foot_vertical_speed_296: float(296),
            landing_bucket_300: word(300) as i32,
            hippy_jump_372: byte(372),
            walking_716: byte(716),
            offboard_air_718: byte(718),
            foot_down_right_724: byte(724),
            foot_down_left_725: byte(725),
            foot_material_728: word(728),
            foot_material_732: word(732),
            step_code_740: word(740) as i32,
            footplant_768: byte(768),
            footstep_strength_796: float(796),
        }
    }
}

/// `sub_824E9FD8`'s two `sub_824B73E0` calls (identical arguments: w0..w6 = 0, 0, 4096, 0,
/// 25000, 0, 0; w7 32767; w12 1; w16 1; w17 1; w24 eEQChain + 10; the rest 0), after the
/// constructor's clamps. Word 13 is the constructor's constant 1.
pub(crate) fn constructor_words(eq_chain: u32) -> [u32; WORDS] {
    let clamp = |v: i32, lo: i32, hi: i32| v.clamp(lo, hi) as u32;
    let mut w = [0u32; WORDS];
    w[0] = clamp(0, 0, 32767);
    w[1] = clamp(0, 0, 65535);
    w[2] = clamp(4096, 0, 8192);
    w[3] = clamp(0, 0, 1000);
    w[4] = clamp(25000, 0, 25001);
    w[5] = clamp(0, 0, 25001);
    w[6] = clamp(0, 0, 32767);
    w[7] = clamp(32767, 0, 32767);
    w[8] = clamp(0, 0, 1);
    w[9] = clamp(0, 0, 1000);
    w[10] = clamp(0, 0, 1000);
    w[11] = clamp(0, 0, 1);
    w[12] = clamp(1, 1, 4);
    w[13] = 1;
    w[14] = clamp(0, 0, 1000);
    w[15] = clamp(0, 0, 5);
    w[16] = clamp(1, 1, 7);
    w[17] = clamp(1, 1, 5);
    for word in &mut w[18..24] {
        *word = clamp(0, 0, 32767);
    }
    w[24] = clamp((eq_chain as i32).wrapping_add(10), 0, 32767);
    w
}

/// One foot's owner-side values `sub_824EAEA8` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FootUpdate {
    /// Owner +52 / +236: the foot-down byte copied from +724 / +725.
    pub down: bool,
    /// Owner +420 / +424: vertical speed curve.
    pub vertical: i32,
    /// Owner +412 / +416: horizontal speed curve.
    pub horizontal: i32,
    /// Owner +464 / +468: frames left after a footplant ended.
    pub countdown: i32,
    /// Owner +444 ≠ 0: the jump sample voice exists.
    pub jump_voice: bool,
    /// Owner +408: COM speed curve.
    pub speed: i32,
    /// Owner +56 / +240: foot surface material.
    pub surface: u32,
    pub landing_bucket_300: i32,
    /// `fctiwz(+796)`.
    pub strength: i32,
    pub step_code_740: i32,
}

/// `sub_824EAEA8` for one holder: words 3 and 24 keep their constructor values.
pub(crate) fn update_words(
    packet: &mut [u32; WORDS],
    foot: &FootUpdate,
    controls: &dyn Controls,
    tuning: &FootstepTuning,
) {
    let clamp = |v: i32, lo: i32, hi: i32| v.clamp(lo, hi) as u32;
    packet[0] = 32767;
    packet[1] = clamp(controls.raw(0) as i32, 0, 65535);
    packet[2] = clamp(controls.pitch(1), 0, 8192);
    packet[4] = clamp(controls.level(4) as i32, 0, 25001);
    packet[5] = clamp(controls.level(5) as i32, 0, 25001);
    packet[6] = clamp(controls.level(6) as i32, 0, 32767);
    packet[7] = clamp(controls.level(2) as i32, 0, 32767);
    packet[8] = u32::from(foot.down);
    packet[9] = clamp(foot.vertical, 0, 1000);
    packet[10] = clamp(foot.horizontal, 0, 1000);
    packet[11] = u32::from(foot.countdown > 0 || foot.jump_voice);
    packet[12] = clamp(foot.landing_bucket_300, 1, 4);
    packet[13] = clamp(foot.strength, 1, 99);
    packet[14] = clamp(foot.speed, 0, 1000);
    packet[15] = 1;
    packet[16] = clamp(tuning.surface_step(foot.surface) as i32, 1, 7);
    packet[17] = clamp(foot.step_code_740, 1, 5);
    for (i, gain) in tuning.gains.iter().enumerate() {
        packet[18 + i] = clamp(*gain, 0, 32767);
    }
}

/// The owner-side state `sub_824E9270` and its callees keep (fields named by owner offset).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FootstepOwner {
    speed_408: i32,
    horizontal_412: i32,
    horizontal_416: i32,
    vertical_420: i32,
    vertical_424: i32,
    /// +52 / +236 this frame's foot-down copies; +54 / +238 the previous frame's.
    down_52: bool,
    down_236: bool,
    down_prev_54: bool,
    down_prev_238: bool,
    /// +56 / +240: foot surfaces (poster).
    surface_56: u32,
    surface_240: u32,
    /// +460: previous +768.
    footplant_prev_460: bool,
    countdown_464: i32,
    countdown_468: i32,
    /// +444: jump sample voice pointer ≠ 0 (`sub_824E9678`).
    jump_voice_444: bool,
    /// +456: COM velocity y ≤ 0 last frame.
    falling_456: bool,
    /// +452 / +405: previous +372 / +718.
    hippy_prev_452: bool,
    offboard_air_prev_405: bool,
    /// `fctiwz(+796)` and the +300 / +740 reads of the updater, kept from the process pass's
    /// state (both ticks see the same state).
    strength: i32,
    landing_bucket_300: i32,
    step_code_740: i32,
}

impl FootstepOwner {
    /// `sub_824E9270` with `[owner+28]` present and its byte 52 set (the skater's sound owner
    /// is active), minus the poster's packet creation (done by the component).
    pub(crate) fn process(&mut self, state: &FootstepInputs, tuning: &FootstepTuning) {
        let footplant = state.footplant_768;
        let footplant_ended = !footplant && self.footplant_prev_460;

        self.speed_408 = fctiwz(tuning.speed_curve.evaluate(state.com_speed_212));
        self.horizontal_416 = fctiwz(
            tuning
                .foot_speed_curve
                .evaluate(state.foot_world_speed_xz_284),
        );
        self.horizontal_412 = fctiwz(
            tuning
                .foot_speed_curve
                .evaluate(state.foot_world_speed_xz_288),
        );
        self.vertical_424 = fctiwz(
            tuning
                .foot_vertical_curve
                .evaluate(state.foot_vertical_speed_292),
        );
        self.vertical_420 = fctiwz(
            tuning
                .foot_vertical_curve
                .evaluate(state.foot_vertical_speed_296),
        );

        self.down_52 = state.foot_down_right_724;
        self.down_236 = state.foot_down_left_725;
        if footplant_ended {
            if self.down_prev_54 {
                self.countdown_464 = FOOTPLANT_COUNTDOWN;
            } else if self.down_prev_238 {
                self.countdown_468 = FOOTPLANT_COUNTDOWN;
            }
        }
        if self.countdown_464 > 0 {
            self.countdown_464 -= 1;
        }
        if self.countdown_468 > 0 {
            self.countdown_468 -= 1;
        }

        // sub_824E9FD8: foot surfaces (the packet-relevant part; the rest plays bank samples).
        self.surface_240 = if state.foot_material_728 == NO_MATERIAL {
            DEFAULT_SURFACE
        } else {
            state.foot_material_728
        };
        self.surface_56 = if state.foot_material_732 == NO_MATERIAL {
            DEFAULT_SURFACE
        } else {
            state.foot_material_732
        };

        self.jump_voice(state, footplant_ended);

        self.footplant_prev_460 = footplant;
        self.down_prev_54 = self.down_52;
        self.down_prev_238 = self.down_236;

        self.strength = fctiwz(state.footstep_strength_796);
        self.landing_bucket_300 = state.landing_bucket_300;
        self.step_code_740 = state.step_code_740;
    }

    /// `sub_824E9678`'s `+444` jump voice: started when walking into OffboardAir (+716 && +718
    /// rising), on a rising hippy jump (+372), or when a footplant ends; replaced if running;
    /// stopped once the COM velocity turns downward (+100 ≤ 0 after > 0). Assumes the voice
    /// allocation (`sub_828AAC28`) succeeds.
    pub(crate) fn jump_voice(&mut self, state: &FootstepInputs, footplant_ended: bool) {
        let jump_off = state.walking_716 && state.offboard_air_718 && !self.offboard_air_prev_405;
        let rising = state.com_velocity_y_100 > 0.0;
        let turned_down = !rising && !self.falling_456;
        self.falling_456 = !rising;
        let stop = self.jump_voice_444 && turned_down;
        let hippy = !self.hippy_prev_452 && state.hippy_jump_372;
        if jump_off || hippy || footplant_ended {
            self.jump_voice_444 = true;
        }
        if stop {
            self.jump_voice_444 = false;
        }
        self.hippy_prev_452 = state.hippy_jump_372;
        self.offboard_air_prev_405 = state.offboard_air_718;
    }

    /// Holder `+36` (`foot == 0`) or `+220` (`foot == 1`).
    pub(crate) fn foot(&self, foot: usize) -> FootUpdate {
        let common = FootUpdate {
            jump_voice: self.jump_voice_444,
            speed: self.speed_408,
            landing_bucket_300: self.landing_bucket_300,
            strength: self.strength,
            step_code_740: self.step_code_740,
            ..FootUpdate::default()
        };
        if foot == 0 {
            FootUpdate {
                down: self.down_52,
                vertical: self.vertical_420,
                horizontal: self.horizontal_412,
                countdown: self.countdown_464,
                surface: self.surface_56,
                ..common
            }
        } else {
            FootUpdate {
                down: self.down_236,
                vertical: self.vertical_424,
                horizontal: self.horizontal_416,
                countdown: self.countdown_468,
                surface: self.surface_240,
                ..common
            }
        }
    }
}

/// The two playercharacter_footstep packets: `[0]` = holder `+36`, `[1]` = holder `+220`.
pub(crate) struct Footsteps {
    tuning: FootstepTuning,
    owner: FootstepOwner,
    holders: [Option<u32>; 2],
    packets: [[u32; WORDS]; 2],
    /// OffBoard controller writes since the last [`Component::take_owner_inputs`].
    owner_inputs: Vec<(u32, u32)>,
}

impl Footsteps {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        let tuning = FootstepTuning::from_vault(vault)?;
        let packets = [constructor_words(tuning.eq_chain); 2];
        Ok(Self {
            tuning,
            owner: FootstepOwner::default(),
            holders: [None; 2],
            packets,
            owner_inputs: Vec::new(),
        })
    }
}

impl Component for Footsteps {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        self.owner_inputs
            .push((0, if tick.audio.walking_716 { 32_767 } else { 0 }));
        let Some(state) = FootstepInputs::from_state(tick.audio) else {
            return Ok(());
        };
        // sub_824E9270 runs the curves and copies before its poster; the poster creates the
        // packets (holder +220 first) when either is missing.
        if self.holders[0].is_none() || self.holders[1].is_none() {
            let words = constructor_words(self.tuning.eq_chain);
            for foot in [1, 0] {
                if self.holders[foot].is_none() {
                    self.packets[foot] = words;
                    self.holders[foot] = Some(post(tick.runtime, OBJECT, &words)?);
                }
            }
        }
        self.owner.process(&state, &self.tuning);
        Ok(())
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        let [Some(a), Some(b)] = self.holders else {
            return Ok(());
        };
        for (foot, handle) in [(0, a), (1, b)] {
            let update = self.owner.foot(foot);
            update_words(
                &mut self.packets[foot],
                &update,
                tick.controls,
                &self.tuning,
            );
            redeliver(tick.runtime, handle, &self.packets[foot])?;
        }
        Ok(())
    }

    fn take_owner_inputs(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.owner_inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed {
        raw: u32,
        pitch: i32,
        levels: [u32; 8],
    }

    impl Controls for Fixed {
        fn raw(&self, _id: u32) -> u32 {
            self.raw
        }
        fn pitch(&self, _id: u32) -> i32 {
            self.pitch
        }
        fn level(&self, id: u32) -> u32 {
            self.levels[id as usize]
        }
    }

    fn curve(xs: [u32; 16], ys: [u32; 16]) -> Curve {
        Curve {
            xs: xs.map(f32::from_bits),
            ys: ys.map(f32::from_bits),
        }
    }

    /// The stock vault (`skater-collections.json`), words copied from its hex.
    fn tuning() -> FootstepTuning {
        FootstepTuning {
            speed_curve: curve(
                [
                    0x00000000, 0x3EF1D2F8, 0x3FCCCCCD, 0x40000000, 0x40200000, 0x40600000,
                    0x408806AB, 0x40A42B5C, 0x40C0D57A, 0x40E01AAE, 0x40EB0C82, 0x4103DB4F,
                    0x410BEF53, 0x4112305E, 0x4117A947, 0x411FBD4A,
                ],
                [
                    0x00000000, 0x42D20000, 0x434D0000, 0x434D0000, 0x437A0000, 0x43988000,
                    0x43AF0000, 0x43B1AF9A, 0x43CC8BA3, 0x43CE9C8F, 0x44001964, 0x440121D9,
                    0x44001964, 0x44001964, 0x440121D9, 0x440121D9,
                ],
            ),
            foot_speed_curve: curve(
                [
                    0x00000000, 0x3E99999A, 0x3F856B90, 0x40088C15, 0x4057C3F8, 0x40888C17,
                    0x40A32086, 0x40C58640, 0x40E3C0A0, 0x40F7092D, 0x41073E8B, 0x41105D65,
                    0x41159399, 0x4118B41E, 0x411B0C82, 0x411F7A94,
                ],
                [
                    0x00000000, 0x44043B3D, 0x44022A51, 0x44001964, 0x44001964, 0x440121D9,
                    0x440121D9, 0x44022A51, 0x440121D9, 0x440121D9, 0x44022A51, 0x44001964,
                    0x44022A51, 0x44022A51, 0x44001964, 0x440332C7,
                ],
            ),
            foot_vertical_curve: curve(
                [
                    0x00000000, 0x3F000000, 0x3F800000, 0x401C5A0C, 0x405E0503, 0x409488C0,
                    0x40B49618, 0x40DAE47A, 0x40FEDA79, 0x411488C1, 0x41174535, 0x4122370D,
                    0x41311188, 0x413EBFD1, 0x414EC67E, 0x415CD8CD,
                ],
                [
                    0x40000000, 0x40000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
                    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x4060CB1D,
                    0x40819637, 0x40A15283, 0x40DA2E90, 0x40EE043A,
                ],
            ),
            gains: [32767, 10000, 15000, 25000, 32767, 28000],
            surface_step: {
                // Entries 0..94 `+24`; only the ones the tests use matter.
                let mut table = vec![1; 95];
                table[2] = 1;
                table[3] = 2;
                table
            },
            eq_chain: 2,
        }
    }

    #[test]
    fn curve_clamps_at_the_ends_and_interpolates_with_a_fused_multiply_add() {
        let t = tuning();
        assert_eq!(t.speed_curve.evaluate(-1.0), 0.0);
        assert_eq!(t.speed_curve.evaluate(100.0), f32::from_bits(0x440121D9));
        // Mid-segment: (205 - 105) / (1.6 - 0.4724) × (1.0 - 0.4724) + 105.
        let x0 = f32::from_bits(0x3EF1D2F8);
        let x1 = f32::from_bits(0x3FCCCCCD);
        let slope = (205.0f32 - 105.0) / (x1 - x0);
        assert_eq!(t.speed_curve.evaluate(1.0), slope.mul_add(1.0 - x0, 105.0));
        // Exactly on an interior point: the next segment's start.
        assert_eq!(t.speed_curve.evaluate(2.0), 205.0);
        // A zero-width segment returns the right point's y.
        let step = Curve {
            xs: [
                0.0, 1.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0,
            ],
            ys: [0.0; 16],
        };
        assert_eq!(step.evaluate(0.5), 0.0);
    }

    #[test]
    fn constructor_matches_the_retail_post() {
        // posts.tsv, frame 2708: both local packets.
        let posted = [
            0, 0, 0x1000, 0, 0x61A8, 0, 0, 0x7FFF, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0,
            0xC,
        ];
        assert_eq!(constructor_words(2), posted);
    }

    #[test]
    fn footplant_end_raises_word_11_for_nine_frames_on_the_previously_down_foot() {
        let t = tuning();
        let mut owner = FootstepOwner::default();
        let mut state = FootstepInputs {
            footplant_768: true,
            foot_down_right_724: true,
            com_velocity_y_100: -1.0,
            ..FootstepInputs::default()
        };
        owner.process(&state, &t);
        state.footplant_768 = false;
        owner.process(&state, &t);
        // The footplant end also starts the jump voice; the COM is already falling, so it
        // stays until the velocity turns downward again.
        assert_eq!(owner.foot(0).countdown, 9);
        assert_eq!(owner.foot(1).countdown, 0);
        for left in (0..9).rev() {
            owner.process(&state, &t);
            assert_eq!(owner.foot(0).countdown, left);
        }
    }

    #[test]
    fn jump_voice_holds_until_the_com_turns_downward() {
        let t = tuning();
        let mut owner = FootstepOwner::default();
        let mut state = FootstepInputs {
            walking_716: true,
            com_velocity_y_100: 0.0,
            ..FootstepInputs::default()
        };
        owner.process(&state, &t);
        assert!(!owner.foot(0).jump_voice);
        state.offboard_air_718 = true;
        state.com_velocity_y_100 = 3.0;
        owner.process(&state, &t);
        assert!(owner.foot(0).jump_voice);
        owner.process(&state, &t);
        assert!(owner.foot(1).jump_voice);
        state.com_velocity_y_100 = -0.5;
        owner.process(&state, &t);
        assert!(!owner.foot(0).jump_voice);
    }

    #[test]
    fn update_words_clamp_and_copy_like_sub_824eaea8() {
        // Capture frame 2709 (state 2708), holder +36: controller reads 52:0 = FF8F,
        // 56:1 = FF6, 60:4 = 618B, 60:5 = 4D, 60:6 = 291, 60:2 = D33.
        let t = tuning();
        let controls = Fixed {
            raw: 0xFF8F,
            pitch: 0xFF6,
            levels: [0, 0, 0xD33, 0, 0x618B, 0x4D, 0x291, 0],
        };
        let mut packet = constructor_words(2);
        let foot = FootUpdate {
            down: false,
            vertical: 2,
            horizontal: 0x1E4,
            countdown: 0,
            jump_voice: false,
            speed: 0x73,
            surface: 2,
            landing_bucket_300: 1,
            strength: 0,
            step_code_740: 1,
        };
        update_words(&mut packet, &foot, &controls, &t);
        let retail = [
            0x7FFF, 0xFF8F, 0xFF6, 0, 0x618B, 0x4D, 0x291, 0xD33, 0, 2, 0x1E4, 0, 1, 1, 0x73, 1, 1,
            1, 0x7FFF, 0x2710, 0x3A98, 0x61A8, 0x7FFF, 0x6D60, 0xC,
        ];
        assert_eq!(packet[..16], retail[..16]);
        assert_eq!(packet[17..], retail[17..]);
    }

    /// Replays the retail recomp capture: state from frame − 1, the controller reads each
    /// update made, compared word by word. Needs `.local/captures/extract` and the vault.
    #[test]
    #[ignore]
    fn capture_replay() {
        replay::run();
    }

    mod replay {
        use super::super::*;
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        struct Reads(BTreeMap<(u32, u32), u32>);

        impl Controls for Reads {
            fn raw(&self, id: u32) -> u32 {
                self.0[&(52, id)]
            }
            fn pitch(&self, id: u32) -> i32 {
                self.0[&(56, id)] as i32
            }
            fn level(&self, id: u32) -> u32 {
                self.0[&(60, id)]
            }
        }

        pub(super) fn run() {
            let root =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/captures/extract");
            let assets =
                PathBuf::from(r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets");
            let vault = Collections::load(&assets).expect("vault");
            let tuning = FootstepTuning::from_vault(&vault).expect("tuning");
            let mut states: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for line in std::fs::read_to_string(root.join("state.tsv"))
                .unwrap()
                .lines()
            {
                let p: Vec<&str> = line.split('\t').collect();
                let words = p[2..]
                    .iter()
                    .map(|w| u32::from_str_radix(w, 16).unwrap())
                    .collect();
                states.insert(p[0].parse().unwrap(), words);
            }
            let rows =
                std::fs::read_to_string(root.join("attributed/playercharacter_footstep.tsv"))
                    .unwrap();
            let mut owner = FootstepOwner::default();
            let mut packets = [constructor_words(tuning.eq_chain); 2];
            let (mut total, mut exact) = ([0u32; WORDS], [0u32; WORDS]);
            let mut bad: Vec<Vec<String>> = vec![Vec::new(); WORDS];
            let mut w11_voice_only = 0;
            let mut posts = 0;
            let mut releases = 0;
            for row in rows.lines() {
                let p: Vec<&str> = row.split('\t').collect();
                if p[6] != "4A26A930" && p[0] != "PO" {
                    continue;
                }
                let frame: u32 = p[1].parse().unwrap();
                match p[0] {
                    "PO" => {
                        if p[4] == "40C93724" || p[4] == "40C93924" {
                            posts += 1;
                            let words: Vec<u32> = p[5]
                                .split(' ')
                                .map(|w| u32::from_str_radix(w, 16).unwrap())
                                .collect();
                            assert_eq!(
                                words[..WORDS],
                                constructor_words(tuning.eq_chain),
                                "post frame {frame}"
                            );
                        }
                        continue;
                    }
                    "RL" => {
                        releases += 1;
                        continue;
                    }
                    _ => {}
                }
                let foot = match p[4] {
                    "40C93724" => 0,
                    "40C93924" => 1,
                    _ => continue,
                };
                let Some(state) = states.get(&(frame - 1)) else {
                    continue;
                };
                if foot == 0 {
                    // +100 (COM velocity y) is below the dump, so the jump-voice latch cannot be
                    // replayed; the replay drops it from word 11 and counts the rows it alone
                    // would explain.
                    let inputs = FootstepInputs::from_capture(state, 0.0);
                    owner.process(&inputs, &tuning);
                }
                let reads: BTreeMap<(u32, u32), u32> = p[7]
                    .split(',')
                    .filter(|r| !r.is_empty())
                    .map(|r| {
                        let f: Vec<&str> = r.split(':').collect();
                        (
                            (f[0].parse().unwrap(), f[1].parse().unwrap()),
                            u32::from_str_radix(f[2], 16).unwrap(),
                        )
                    })
                    .collect();
                let mut update = owner.foot(foot);
                update.jump_voice = false;
                update_words(&mut packets[foot], &update, &Reads(reads), &tuning);
                let retail: Vec<u32> = p[5]
                    .split(' ')
                    .map(|w| u32::from_str_radix(w, 16).unwrap())
                    .collect();
                for i in 0..WORDS {
                    total[i] += 1;
                    if packets[foot][i] == retail[i] {
                        exact[i] += 1;
                    } else {
                        if i == 11 && update.countdown <= 0 && retail[i] == 1 {
                            w11_voice_only += 1;
                            continue;
                        }
                        if bad[i].len() < 6 {
                            bad[i].push(format!(
                                "f{frame}/{foot}: ours {} retail {}",
                                packets[foot][i], retail[i]
                            ));
                        }
                    }
                }
            }
            println!("posts {posts} releases {releases}");
            for i in 0..WORDS {
                println!("w{i:2}: {}/{} exact {:?}", exact[i], total[i], bad[i]);
            }
            println!(
                "w11 mismatches where retail=1 without a countdown (jump voice +444): {w11_voice_only}"
            );
        }
    }
}
