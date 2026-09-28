//! Class_Seams (component vtable 0x822FC6E0, Cracks controller 40010040): the per-wheel crack and
//! seam hits.
//!
//! - Component constructor `sub_824C1198`: the pattern instance at `+36`/`+40` starts on the
//!   pattern class's `default` collection, grid cells `+72..+100` = `0x7FFFFFFF`, front/rear
//!   distance accumulators `+104` = 0.0 and `+108` = 999.0 (`0x82324510`), previous wheel
//!   materials `+112..+124` = 143, the frame counter `+132` and last-fire frames `+136..+148` = 0.
//! - Create (slot 7) `sub_824C13D0`: enable the Cracks controller (`[[this+12]+12]+60 = 1`), then
//!   per wheel construct the 84-byte packet (`sub_824AFDD0`, 20 words, message slot
//!   `0x8302EE58`), hold it at `+52 + 4i` and set its toggle byte `+68 + i` to 1. The four
//!   messages are held until teardown (`sub_824C1378`).
//! - Process (slot 9) `sub_824C14C8`, gated on the owner's active byte `[[this+16]+52]`: controller
//!   input 0 := 0, w7 := 0 on every packet, clear the material history while airborne (+332), fire
//!   the material-change trigger `sub_824C1DF8(i, 0, 1)` for every wheel whose material (+620)
//!   changed, else above the pattern's speed threshold run the pattern trigger `sub_824C1698` for
//!   axis 0 then axis 1; then redeliver all four.
//! - Update (slot 10) `sub_824C1F18`, same gate: rewrite and redeliver each packet in turn.
//!
//! So each packet is delivered twice per frame, which the retail capture shows (update first,
//! then process, within one capture frame).
//!
//! Patterns (`sub_82497A58`, names at `0x8224DD08..0x8224DDDC`, keyed by AttribSys hash): the seam
//! pattern of wheel 0 (+636), or of wheel 3 (+648) while manualling (`+339 || +340`) with wheel 0's
//! landed latch (+464) clear, selects the collection of class `7242F32831ED3332`. Its layout
//! (`+0` gain, `+4` grid angle, `+8` grid z, `+12` grid x, `+16` class = w13) and attributes (mode
//! `CA81764BF5A85E34`, minimum frames between hits `DAE803A0CBD286D1`, speed threshold
//! `F7BCA67F0A1FC92E`, distance spacing `A3BC1976039FA00A`, level `2F29F40384863C8C`) are read
//! from the vault, inheritance included. Pattern 0 (the surface map supplies no pattern) posts no
//! hits and zeroes w13 on each pattern check; the held messages keep running with the other
//! words (w8 speed, w15 turn, gains).
//!
//! Engine interface notes (not guesses; the lead wires these):
//! - Retail writes the Cracks controller's input 0 (vtable `+8`, `Set(0, v)`): 0 at the start of
//!   every process and 32767 on every hit; [`Component::take_owner_inputs`] hands them over in
//!   order. Create's controller enable (`+60 = 1`) is already set by the MixMap build.
//! - The owner's active byte `[[this+16]+52]` has no engine equivalent; the component runs
//!   whenever it is ticked.
//! - Distance mode (pattern 10 `slats` only) scales by the live global time scale `[*(0x82083C38)
//!   + 0x2F070]` (the value the bridge copies into +220; this port reads +220) and the owner's
//!   frame time `[[this+16]+60]` (`sub_824B95A0` accumulates it as seconds; this port uses the
//!   tick's `dt`). The capture never visits a slats surface, so this path is not capture-checked.
//! - `sub_824B23C8` (w11 and the w1 source) returns +684 for the local skater (`[[state+16]+72]`
//!   set); the remote branch (first local player's `+84 == 0`) is not ported.

use skate_data::attrib_hash::hash;
use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::contacts::{SurfaceMap, clamp_word, vault_word};
use super::words::{fctiwz, seam_speed};
use super::{Component, Controls, Tick, post, redeliver};

/// Packet length: the constructor's 84-byte object minus the 4-byte header.
pub(crate) const SEAM_WORDS: usize = 20;
const OBJECT: &str = "Class_Seams";

/// Seam pattern class (`sub_824C1698`, `sub_824C1198`; also the audio tuning holder `+28`).
const PATTERN_CLASS: &str = "Hash_7242F32831ED3332";
const DEFAULT_KEY: &str = "Hash_D7EDBD362D7D2152";
/// Layout fields (skaterschema class definition): +0, +4, +8, +12, +16.
const FIELD_GAIN_0: &str = "Hash_199170BB1C52EE64";
const FIELD_ANGLE_4: &str = "Hash_D18F436B5764F260";
const FIELD_GRID_8: &str = "Hash_534B0A719762E2E8";
const FIELD_GRID_12: &str = "Hash_107A78BA11A2B813";
const FIELD_CLASS_16: &str = "Hash_5FAD918A2DE5459A";
/// Attributes (`sub_82B72420` lookups on the current pattern collection).
const FIELD_MODE: &str = "Hash_CA81764BF5A85E34";
const FIELD_MIN_FRAMES: &str = "Hash_DAE803A0CBD286D1";
const FIELD_SPEED_THRESHOLD: &str = "Hash_F7BCA67F0A1FC92E";
const FIELD_SPACING: &str = "Hash_A3BC1976039FA00A";
const FIELD_LEVEL: &str = "Hash_2F29F40384863C8C";
/// Holder `+28` (the pattern class's `default`), `sub_824C1CA0`: the grid multiplier on surface
/// 3 for class-10 patterns.
const FIELD_SURFACE3_SCALE: &str = "Hash_1911176187FB9B1F";
/// Holder `+140` eEQChain `default`, field `5A837C613E3F41DC` (w19, `sub_824C13D0`).
const EQ_CLASS: &str = "Hash_42AFE160E647167C";
const EQ_FIELD: &str = "Hash_5A837C613E3F41DC";

/// `sub_82497A58`: pattern `p` (1..15) → the name at `0x8224DDDC + {-212, -200, …, 0}`.
const PATTERN_NAMES: [&str; 15] = [
    "spidercrack",
    "square_2_x_2",
    "square_4_x_4",
    "square_8_x_8",
    "square_12_x_12",
    "square_24_x_24",
    "irregular_small",
    "irregular_medium",
    "irregular_large",
    "slats",
    "sidewalk",
    "brick_tile_random_size",
    "mini_tile",
    "special_1",
    "special_2",
];

/// Image constants.
const REAR_IDLE: f32 = f32::from_bits(0x4479_C000); // 0x82324510 = 999.0
const DISTANCE_SCALE: f32 = f32::from_bits(0x42C8_0000); // 0x820ED57C = 100.0
const TIME_SCALE_CAP: f32 = f32::from_bits(0x4000_0000); // 0x82060C50 = 2.0
const REAR_TRAVEL: f32 = f32::from_bits(0x4248_0000); // 0x8220E13C = 50.0
const DEG_TO_RAD: f32 = f32::from_bits(0x3C8E_FA35); // 0x8206D110
const LEVEL_SCALE: f32 = f32::from_bits(0x46FF_FE00); // 0x821747FC = 32767.0
const TURN_SCALE: f32 = f32::from_bits(0x447A_0000); // 0x82256FE8 = 1000.0

/// One pattern collection as the component reads it through its `+36` instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PatternRecord {
    pub gain_0: f32,
    pub angle_4: i32,
    pub grid_8: f32,
    pub grid_12: f32,
    pub class_16: i32,
    /// 0 none, 1 grid, 2 distance.
    pub mode: i32,
    pub min_frames: i32,
    pub speed_threshold: f32,
    pub spacing: f32,
    pub level: f32,
}

impl PatternRecord {
    fn load(vault: &Collections, key: &str) -> Result<Self, String> {
        let float = |name: &str| vault.float(PATTERN_CLASS, key, name);
        let int = |name: &str| vault.integer(PATTERN_CLASS, key, name).map(|v| v as i32);
        Ok(Self {
            gain_0: float(FIELD_GAIN_0)?,
            angle_4: int(FIELD_ANGLE_4)?,
            grid_8: float(FIELD_GRID_8)?,
            grid_12: float(FIELD_GRID_12)?,
            class_16: int(FIELD_CLASS_16)?,
            // `SeamPatternStyle` enum, a 32-bit value read with `lwz`.
            mode: vault_word(vault, PATTERN_CLASS, key, FIELD_MODE)? as i32,
            min_frames: int(FIELD_MIN_FRAMES)?,
            speed_threshold: float(FIELD_SPEED_THRESHOLD)?,
            spacing: float(FIELD_SPACING)?,
            level: float(FIELD_LEVEL)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeamTuning {
    /// The pattern class's `default` collection (the instance the constructor selects).
    pub default: PatternRecord,
    /// Patterns 1..15.
    pub patterns: [PatternRecord; 15],
    pub surfaces: SurfaceMap,
    /// Holder `+28` field `1911176187FB9B1F`.
    pub surface3_scale: f32,
    /// w19.
    pub eq_chain: u32,
}

impl SeamTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let default = PatternRecord::load(vault, DEFAULT_KEY)?;
        let mut patterns = [default; 15];
        for (record, name) in patterns.iter_mut().zip(PATTERN_NAMES) {
            *record = PatternRecord::load(vault, &format!("Hash_{:016X}", hash(name)))?;
        }
        Ok(Self {
            default,
            patterns,
            surfaces: SurfaceMap::load(vault)?,
            surface3_scale: vault.float(PATTERN_CLASS, DEFAULT_KEY, FIELD_SURFACE3_SCALE)?,
            eq_chain: vault_word(vault, EQ_CLASS, DEFAULT_KEY, EQ_FIELD)?,
        })
    }
}

/// The audio-state fields the component reads (all published by [`AudioState`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SeamInputs {
    pub turn_204: f32,
    pub ground_speed_208: f32,
    pub time_scale_220: f32,
    pub in_known_air_332: bool,
    pub push_left_333: bool,
    pub push_right_334: bool,
    pub manual_brake_339: bool,
    pub balance_340: bool,
    pub grinding_341: bool,
    pub wheel_position_384: [[f32; 3]; 4],
    pub wheel_landed_464: bool,
    pub wheel_material_620: [i32; 4],
    pub wheel_seam_636: [u32; 4],
    /// `sub_824B23C8` for the local skater: +684.
    pub soft_wheels: i32,
}

impl SeamInputs {
    pub(crate) fn from_state(state: &AudioState) -> Self {
        Self {
            turn_204: state.turn_204,
            ground_speed_208: state.ground_speed_208,
            time_scale_220: state.time_scale_220,
            in_known_air_332: state.in_known_air_332,
            push_left_333: state.push_left_333,
            push_right_334: state.push_right_334,
            manual_brake_339: state.manual_brake_339,
            balance_340: state.balance_340,
            grinding_341: state.grinding_341,
            wheel_position_384: state.wheel_position_384,
            wheel_landed_464: state.wheel_landed_464[0],
            wheel_material_620: state.wheel_material_620.map(|m| m as i32),
            wheel_seam_636: state.wheel_seam_636,
            soft_wheels: state.soft_wheels_684 as i32,
        }
    }

    /// The retail capture's audio-state words (160 words from state+192).
    #[cfg(test)]
    pub(crate) fn from_capture(words: &[u32]) -> Self {
        let word = |offset: usize| words[(offset - 192) / 4];
        let float = |offset: usize| f32::from_bits(word(offset));
        let byte = |offset: usize| (word(offset & !3) >> (8 * (3 - (offset & 3)))) & 0xFF != 0;
        Self {
            turn_204: float(204),
            ground_speed_208: float(208),
            time_scale_220: float(220),
            in_known_air_332: byte(332),
            push_left_333: byte(333),
            push_right_334: byte(334),
            manual_brake_339: byte(339),
            balance_340: byte(340),
            grinding_341: byte(341),
            wheel_position_384: std::array::from_fn(|i| {
                std::array::from_fn(|axis| float(384 + 16 * i + 4 * axis))
            }),
            wheel_landed_464: byte(464),
            wheel_material_620: std::array::from_fn(|i| word(620 + 4 * i) as i32),
            wheel_seam_636: std::array::from_fn(|i| word(636 + 4 * i)),
            soft_wheels: word(684) as i32,
        }
    }

    fn manual(&self) -> bool {
        self.balance_340 || self.manual_brake_339
    }
}

/// `sub_824AFDD0`: the constructor's packet.
pub(crate) fn seam_post(surface: i32, soft: i32, wheel: i32, eq_chain: i32) -> [u32; SEAM_WORDS] {
    let mut words = [0; SEAM_WORDS];
    words[1] = 32_767;
    words[4] = 4_096;
    words[5] = 25_000;
    words[10] = clamp_word(surface, 0, 8);
    words[11] = clamp_word(soft, 0, 1);
    words[14] = clamp_word(wheel, 0, 3);
    words[19] = clamp_word(eq_chain, 0, 32_767);
    words
}

/// `sub_824C1BA8`: rotate the wheel position about Y by the pattern angle (degrees). `cos`/`sin`
/// are the title's double-precision CRT routines (`sub_82F4DFB0` / `sub_82F4DED0`), rounded to
/// single; the products use the recomp's `fma` in double.
fn rotate(position: [f32; 3], degrees: i32) -> (f32, f32) {
    let theta = degrees as f32 * DEG_TO_RAD;
    let cos = (theta as f64).cos() as f32;
    let sin = (theta as f64).sin() as f32;
    let (x, z) = (position[0], position[2]);
    let sz = sin * z;
    let rx = ((cos as f64) * (x as f64) - sz as f64) as f32;
    let sx = sin * x;
    let rz = ((cos as f64) * (z as f64) + sx as f64) as f32;
    (rx, rz)
}

/// The component's retail members, driven by the pure process/update steps.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeamState {
    /// The four packets (object `+4..`).
    pub packets: [[u32; SEAM_WORDS]; 4],
    /// `+68..+71`.
    toggles: [bool; 4],
    /// `+72..+100`: stored grid cells, `[axis][wheel]`.
    cells: [[i32; 4]; 2],
    /// `+104` / `+108`.
    front_104: f32,
    rear_108: f32,
    /// `+112..+124` / `+128`.
    materials_112: [i32; 4],
    history_128: bool,
    /// `+132` / `+136..+148`.
    counter_132: i32,
    last_136: [i32; 4],
    /// `+152`: slewed turn.
    turn_152: i32,
    /// The `+36` instance's collection.
    record: PatternRecord,
    /// Writes to the Cracks controller's input 0, in order.
    controller_writes: Vec<u32>,
}

impl SeamState {
    /// `sub_824C1198` then `sub_824C13D0` (create).
    pub(crate) fn create(tuning: &SeamTuning, inputs: &SeamInputs) -> Self {
        let mut state = Self {
            packets: [[0; SEAM_WORDS]; 4],
            toggles: [true; 4],
            cells: [[i32::MAX; 4]; 2],
            front_104: 0.0,
            rear_108: REAR_IDLE,
            materials_112: [143; 4],
            history_128: false,
            counter_132: 0,
            last_136: [0; 4],
            turn_152: 0,
            record: tuning.default,
            controller_writes: Vec::new(),
        };
        for wheel in 0..4 {
            let surface = state.surface(tuning, inputs, wheel, false);
            state.packets[wheel] = seam_post(
                surface,
                inputs.soft_wheels,
                wheel as i32,
                tuning.eq_chain as i32,
            );
        }
        state
    }

    /// `sub_824C2428`: the wheel's AudioSurfaceMap seams entry (`+32`), or the transition entry
    /// (`+36`) plus 5; 0 without a material.
    fn surface(
        &self,
        tuning: &SeamTuning,
        inputs: &SeamInputs,
        wheel: usize,
        transition: bool,
    ) -> i32 {
        let material = inputs.wheel_material_620[wheel];
        if material >= 143 {
            return 0;
        }
        if transition {
            tuning.surfaces.lookup(material, 36) as i32 + 5
        } else {
            tuning.surfaces.lookup(material, 32) as i32
        }
    }

    /// `sub_824C1DF8`: fire wheel `wheel`'s packet.
    fn trigger(
        &mut self,
        tuning: &SeamTuning,
        inputs: &SeamInputs,
        wheel: usize,
        single: bool,
        transition: bool,
    ) {
        let packet = &mut self.packets[wheel];
        packet[9] = u32::from(single);
        let toggle = self.toggles[wheel];
        packet[7] = if toggle { 1 } else { 2 };
        self.toggles[wheel] = !toggle;
        let surface = self.surface(tuning, inputs, wheel, transition);
        let packet = &mut self.packets[wheel];
        packet[10] = clamp_word(surface, 0, 8);
        packet[11] = clamp_word(inputs.soft_wheels, 0, 1);
        self.controller_writes.push(32_767);
    }

    /// `sub_824C1CA0`: has wheel `wheel` crossed a grid line on `axis` since its last check?
    fn crossed(
        &mut self,
        tuning: &SeamTuning,
        inputs: &SeamInputs,
        wheel: usize,
        axis: usize,
    ) -> bool {
        if inputs.in_known_air_332 || inputs.grinding_341 {
            return false;
        }
        let (x, z) = rotate(inputs.wheel_position_384[wheel], self.record.angle_4);
        let mut scale = 1.0;
        if self.surface(tuning, inputs, wheel, false) == 3 && self.record.class_16 == 10 {
            scale = tuning.surface3_scale;
        }
        let (coordinate, grid) = if axis == 1 {
            (z, self.record.grid_8 * scale)
        } else {
            (x, self.record.grid_12 * scale)
        };
        let cell = fctiwz(((coordinate / grid) as f64).floor() as f32);
        let stored = &mut self.cells[axis][wheel];
        if *stored != cell {
            *stored = cell;
            true
        } else {
            false
        }
    }

    fn clear_class(&mut self) {
        for packet in &mut self.packets {
            packet[13] = 0;
        }
    }

    /// `sub_824C1698`: the pattern trigger for one axis.
    fn pattern(
        &mut self,
        tuning: &SeamTuning,
        inputs: &SeamInputs,
        axis: usize,
        dt: f32,
    ) -> Result<(), String> {
        let mut pattern = inputs.wheel_seam_636[0];
        if inputs.manual() && !inputs.wheel_landed_464 {
            pattern = inputs.wheel_seam_636[3];
        }
        if pattern == 0 {
            self.clear_class();
            return Ok(());
        }
        // The bridge's pattern is a 4-bit field (Collision+3456, `(tag >> 12) & 0xF`).
        self.record = *tuning
            .patterns
            .get(pattern as usize - 1)
            .ok_or_else(|| format!("seam pattern {pattern} outside the 4-bit field"))?;
        if self.record.mode == 0 {
            self.clear_class();
            return Ok(());
        }
        self.counter_132 += 1;
        if self.counter_132 > 32_767 {
            self.counter_132 = 0;
            self.last_136 = [0; 4];
        }
        let counter = self.counter_132;
        let gap = self.record.min_frames;
        if self.record.mode == 1 {
            let a = self.crossed(tuning, inputs, 0, axis);
            let b = self.crossed(tuning, inputs, 1, axis);
            let (c, d) = if inputs.manual() {
                (false, false)
            } else {
                (
                    self.crossed(tuning, inputs, 2, axis),
                    self.crossed(tuning, inputs, 3, axis),
                )
            };
            let both = a && b;
            if a {
                if counter.wrapping_sub(self.last_136[1]) > gap {
                    self.trigger(tuning, inputs, 0, !both, false);
                    self.last_136[0] = counter;
                }
            } else if b && counter.wrapping_sub(self.last_136[0]) > gap {
                self.trigger(tuning, inputs, 1, !both, false);
                self.last_136[1] = counter;
            }
            let both = c && d;
            if c {
                if counter.wrapping_sub(self.last_136[3]) > gap {
                    self.trigger(tuning, inputs, 2, !both, false);
                    self.last_136[2] = counter;
                }
            } else if d && counter.wrapping_sub(self.last_136[2]) > gap {
                self.trigger(tuning, inputs, 3, !both, false);
                self.last_136[3] = counter;
            }
        } else if self.record.mode == 2 && !inputs.in_known_air_332 && axis == 0 {
            let speed = inputs.ground_speed_208 * DISTANCE_SCALE;
            let mut time_scale = inputs.time_scale_220;
            if time_scale > TIME_SCALE_CAP {
                time_scale = TIME_SCALE_CAP;
            }
            let distance = time_scale * speed * dt;
            self.front_104 += distance;
            if !(self.rear_108 > REAR_TRAVEL) {
                self.rear_108 -= distance;
            }
            if self.front_104 > self.record.spacing {
                self.trigger(tuning, inputs, 0, false, false);
                self.trigger(tuning, inputs, 1, false, false);
                self.front_104 = 0.0;
                self.rear_108 = REAR_TRAVEL;
            }
            if !(self.rear_108 > 0.0) {
                self.trigger(tuning, inputs, 2, false, false);
                self.trigger(tuning, inputs, 3, false, false);
                self.rear_108 = REAR_IDLE;
            }
        }
        Ok(())
    }

    /// `sub_824C14C8` up to its redelivery loop.
    pub(crate) fn process(
        &mut self,
        tuning: &SeamTuning,
        inputs: &SeamInputs,
        dt: f32,
    ) -> Result<(), String> {
        self.controller_writes.push(0);
        for packet in &mut self.packets {
            packet[7] = 0;
        }
        if inputs.in_known_air_332 {
            self.history_128 = false;
        }
        let mut changed = false;
        if self.history_128 {
            for wheel in 0..4 {
                if self.materials_112[wheel] != inputs.wheel_material_620[wheel] {
                    self.trigger(tuning, inputs, wheel, false, true);
                    changed = true;
                }
            }
        }
        self.materials_112 = inputs.wheel_material_620;
        self.history_128 = true;
        if !changed && inputs.ground_speed_208 > self.record.speed_threshold {
            self.pattern(tuning, inputs, 0, dt)?;
            self.pattern(tuning, inputs, 1, dt)?;
        }
        Ok(())
    }

    /// One iteration of `sub_824C1F18`'s wheel loop, before its redelivery.
    pub(crate) fn update(&mut self, inputs: &SeamInputs, controls: &dyn Controls, wheel: usize) {
        let mut level = controls.level(1) as i32;
        if inputs.soft_wheels == 1 {
            level = controls.level(6) as i32;
        }
        let raw = controls.raw(0) as i32;
        let pitch = controls.pitch(2);
        let level5 = controls.level(5) as i32;
        let spatial3 = controls.level(3) as i32;
        let spatial4 = controls.level(4) as i32;
        let packet = &mut self.packets[wheel];
        packet[0] = 32_767;
        packet[1] = clamp_word(level, 0, 32_767);
        packet[2] = clamp_word(level5, 0, 32_767);
        packet[5] = clamp_word(spatial3, 0, 25_000);
        packet[6] = clamp_word(spatial4, 0, 25_000);
        packet[3] = clamp_word(raw, 0, 65_536);
        packet[4] = clamp_word(pitch, 0, 8_192);
        packet[8] = seam_speed(inputs.ground_speed_208);
        packet[11] = clamp_word(inputs.soft_wheels, 0, 1);
        let target = fctiwz(inputs.turn_204 * TURN_SCALE);
        let previous = self.turn_152;
        let mut turn = target;
        if target > previous {
            if target.wrapping_sub(previous) > 100 {
                turn = previous + 100;
            }
        } else if target < previous && previous.wrapping_sub(target) > 100 {
            turn = previous - 100;
        }
        self.turn_152 = turn;
        let packet = &mut self.packets[wheel];
        packet[15] = clamp_word(if turn < 0 { turn.wrapping_neg() } else { turn }, 0, 1_000);
        packet[16] = clamp_word(fctiwz(self.record.gain_0 * LEVEL_SCALE), 0, 32_767);
        packet[17] = u32::from(inputs.manual());
        packet[18] = clamp_word(fctiwz(self.record.level * LEVEL_SCALE), 0, 32_767);
        packet[12] = u32::from(inputs.push_left_333 || inputs.push_right_334);
        packet[13] = clamp_word(self.record.class_16, 0, 15);
    }
}

pub(crate) struct Seams {
    tuning: SeamTuning,
    state: Option<SeamState>,
    handles: [Option<u32>; 4],
}

impl Seams {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            tuning: SeamTuning::load(vault)?,
            state: None,
            handles: [None; 4],
        })
    }

    fn redeliver_all(&self, tick: &mut Tick) -> Result<(), String> {
        let Some(state) = &self.state else {
            return Ok(());
        };
        for (handle, packet) in self.handles.iter().zip(&state.packets) {
            if let Some(handle) = handle {
                redeliver(tick.runtime, *handle, packet)?;
            }
        }
        Ok(())
    }

    /// Create (slot 7) runs once before the first process.
    fn create(&mut self, tick: &mut Tick) -> Result<(), String> {
        let inputs = SeamInputs::from_state(tick.audio);
        let state = SeamState::create(&self.tuning, &inputs);
        for (holder, packet) in self.handles.iter_mut().zip(&state.packets) {
            *holder = Some(post(tick.runtime, OBJECT, packet)?);
        }
        self.state = Some(state);
        Ok(())
    }
}

impl Component for Seams {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        if self.state.is_none() {
            self.create(tick)?;
        }
        let inputs = SeamInputs::from_state(tick.audio);
        if let Some(state) = self.state.as_mut() {
            state.process(&self.tuning, &inputs, tick.dt)?;
        }
        self.redeliver_all(tick)
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        let inputs = SeamInputs::from_state(tick.audio);
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        for wheel in 0..4 {
            state.update(&inputs, tick.controls, wheel);
            if let Some(handle) = self.handles[wheel] {
                redeliver(tick.runtime, handle, &state.packets[wheel])?;
            }
        }
        Ok(())
    }

    /// The Cracks controller's input 0 writes (`Set(0, v)`), in retail order.
    fn take_owner_inputs(&mut self) -> Vec<(u32, u32)> {
        let writes = self
            .state
            .as_mut()
            .map(|state| std::mem::take(&mut state.controller_writes));
        writes
            .unwrap_or_default()
            .into_iter()
            .map(|value| (0, value))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::capture::{self, Captured, Matches};
    use super::*;

    /// The stock vault's pattern records (`skater-collections.json`, class `7242F32831ED3332`),
    /// so the unit tests need no assets; `vault_values_match_the_owner_vault` checks them.
    fn record(
        gain: u32,
        angle: i32,
        grid: u32,
        class: i32,
        mode: i32,
        spacing: u32,
    ) -> PatternRecord {
        PatternRecord {
            gain_0: f32::from_bits(gain),
            angle_4: angle,
            grid_8: f32::from_bits(grid),
            grid_12: f32::from_bits(grid),
            class_16: class,
            mode,
            min_frames: 1,
            speed_threshold: f32::from_bits(0x3D4C_CCCD),
            spacing: f32::from_bits(spacing),
            level: 1.0,
        }
    }

    fn tuning() -> SeamTuning {
        let default = record(0x3F21_47AE, 30, 0x4040_0000, 2, 1, 0x4296_0000);
        let mut patterns = [default; 15];
        patterns[3] = record(0x3F21_47AE, 14, 0x3F00_0000, 3, 1, 0x4296_0000); // square_8_x_8
        patterns[7] = record(0x3F80_0000, 30, 0x3E99_999A, 7, 1, 0x4296_0000); // irregular_medium
        patterns[9] = record(0x3F3A_E148, 30, 0x4040_0000, 9, 2, 0x4248_0000); // slats
        patterns[10] = record(0x3F66_6666, 85, 0x4040_0000, 10, 1, 0x437A_0000); // sidewalk
        // Synthetic AudioSurfaceMap: material 2 seams surface (+32) 2, material 3 transition
        // surface (+36) 3; entry 94 (the fallback) is the stock one.
        let mut entries = vec![[0u32; 18]; 95];
        entries[94] = [0x5E, 3, 0, 2, 4, 0, 1, 2, 2, 0, 2, 3, 8, 8, 8, 8, 8, 2];
        entries[2][8] = 2;
        entries[3][8] = 5;
        entries[3][9] = 3;
        SeamTuning {
            default,
            patterns,
            surfaces: SurfaceMap::from_entries(entries),
            surface3_scale: 2.0,
            eq_chain: 8,
        }
    }

    struct Fixed;
    impl Controls for Fixed {
        fn raw(&self, _: u32) -> u32 {
            10
        }
        fn pitch(&self, _: u32) -> i32 {
            4081
        }
        fn level(&self, id: u32) -> u32 {
            [0, 2896, 0, 24971, 77, 729, 49][id as usize]
        }
    }

    impl SeamState {
        fn take_writes(&mut self) -> Vec<u32> {
            std::mem::take(&mut self.controller_writes)
        }
    }

    #[test]
    fn create_posts_the_constructor_packet_per_wheel() {
        // Capture frame 2707: four posts, w10 = 0 (no material), w14 = wheel, w19 = 8.
        let inputs = SeamInputs {
            wheel_material_620: [143; 4],
            ..SeamInputs::default()
        };
        let state = SeamState::create(&tuning(), &inputs);
        for (wheel, packet) in state.packets.iter().enumerate() {
            let mut expected = [0u32; SEAM_WORDS];
            expected[1] = 0x7FFF;
            expected[4] = 0x1000;
            expected[5] = 0x61A8;
            expected[14] = wheel as u32;
            expected[19] = 8;
            assert_eq!(*packet, expected);
        }
    }

    #[test]
    fn update_rewrites_gain_speed_turn_and_pattern_words() {
        let tuning = tuning();
        let mut inputs = SeamInputs {
            ground_speed_208: 0.5,
            wheel_material_620: [2; 4],
            ..SeamInputs::default()
        };
        let mut state = SeamState::create(&tuning, &inputs);
        state.record = tuning.patterns[10];
        inputs.turn_204 = -0.35;
        for wheel in 0..4 {
            state.update(&inputs, &Fixed, wheel);
        }
        let packet = state.packets[0];
        assert_eq!(&packet[..7], &[0x7FFF, 2896, 729, 10, 4081, 24971, 77]);
        assert_eq!(packet[8], 0);
        assert_eq!(packet[13], 10);
        assert_eq!(
            packet[16],
            fctiwz(f32::from_bits(0x3F66_6666) * LEVEL_SCALE) as u32
        );
        assert_eq!(packet[18], 32_767);
        // The turn slews 100 per wheel iteration: wheels 0..3 see 100, 200, 300, 350.
        assert_eq!(state.packets.map(|p| p[15]), [100, 200, 300, 350]);
    }

    #[test]
    fn grid_crossing_alternates_w7_and_marks_single_wheel_hits() {
        let tuning = tuning();
        let mut inputs = SeamInputs {
            ground_speed_208: 5.0,
            wheel_material_620: [2; 4],
            wheel_seam_636: [4; 4],
            ..SeamInputs::default()
        };
        let mut state = SeamState::create(&tuning, &inputs);
        // First process: every cell differs from 0x7FFFFFFF. Wheel 1 is blocked by the gap
        // against wheel 0's hit this frame, wheel 3 by wheel 2's; both hits are pairs (w9 = 0).
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        assert_eq!(state.packets.map(|p| p[7]), [1, 0, 1, 0]);
        assert_eq!(state.packets[0][9], 0);
        assert_eq!(state.packets[0][10], 2);
        // Still: no crossing, w7 back to 0.
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        assert_eq!(state.packets.map(|p| p[7]), [0; 4]);
        // Wheel 0 alone crosses a 0.5 m cell: a single-wheel hit (w9 = 1), w7 alternates to 2.
        inputs.wheel_position_384[0] = [0.6, 0.0, 0.0];
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        assert_eq!(state.packets[0][7], 2);
        assert_eq!(state.packets[0][9], 1);
        assert_eq!(state.take_writes(), vec![0, 32_767, 32_767, 0, 0, 32_767]);
    }

    #[test]
    fn material_change_fires_the_transition_surface_and_skips_patterns() {
        let tuning = tuning();
        let mut inputs = SeamInputs {
            ground_speed_208: 5.0,
            wheel_material_620: [2; 4],
            ..SeamInputs::default()
        };
        let mut state = SeamState::create(&tuning, &inputs);
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        inputs.wheel_material_620[2] = 3;
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        // Transition entry +36 of material 3 is 3, plus 5.
        assert_eq!(state.packets[2][10], 8);
        assert_eq!(state.packets.map(|p| p[7]), [0, 0, 1, 0]);
        // Airborne clears the history: the next change is not a transition.
        inputs.in_known_air_332 = true;
        inputs.wheel_material_620[2] = 2;
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        assert_eq!(state.packets.map(|p| p[7]), [0; 4]);
    }

    #[test]
    fn pattern_zero_clears_w13_and_manual_reads_the_rear_pattern() {
        let tuning = tuning();
        let mut inputs = SeamInputs {
            ground_speed_208: 5.0,
            wheel_material_620: [2; 4],
            wheel_seam_636: [11, 11, 0, 0],
            ..SeamInputs::default()
        };
        let mut state = SeamState::create(&tuning, &inputs);
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        assert_eq!(state.record.class_16, 10);
        state.update(&inputs, &Fixed, 0);
        assert_eq!(state.packets[0][13], 10);
        // Manual with wheel 0's landed latch clear: wheel 3's pattern 0 applies.
        inputs.balance_340 = true;
        state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
        assert_eq!(state.packets[0][13], 0);
        assert_eq!(state.record.class_16, 10);
    }

    #[test]
    fn distance_mode_fires_front_then_rear_after_fifty() {
        let mut tuning = tuning();
        tuning.patterns[9].spacing = 100.0;
        let inputs = SeamInputs {
            ground_speed_208: 3.0,
            time_scale_220: 1.0,
            wheel_material_620: [2; 4],
            wheel_seam_636: [10; 4],
            ..SeamInputs::default()
        };
        let mut state = SeamState::create(&tuning, &inputs);
        let mut fired = Vec::new();
        for frame in 0..12 {
            state.process(&tuning, &inputs, 0.05).unwrap();
            if state.packets.iter().any(|p| p[7] != 0) {
                fired.push((frame, state.packets.map(|p| p[7] != 0)));
            }
        }
        // 15 per frame: the front passes 100 on the 7th frame and arms the rear at 50, which
        // reaches 0 four frames later. (With the stock slats spacing of 50 the front refires,
        // re-arming the rear, on the very frame the rear would have fired.)
        assert_eq!(
            fired,
            vec![
                (6, [true, true, false, false]),
                (10, [false, false, true, true])
            ]
        );
    }

    fn assets() -> Option<Collections> {
        let root = std::path::PathBuf::from(
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets",
        );
        root.join("private/stock/skater-collections.json")
            .exists()
            .then(|| Collections::load(&root).unwrap())
    }

    /// Skipped without the owner's assets.
    #[test]
    fn vault_values_match_the_owner_vault() {
        let Some(vault) = assets() else { return };
        let loaded = SeamTuning::load(&vault).unwrap();
        let fixture = tuning();
        assert_eq!(loaded.default, fixture.default);
        for index in [3, 7, 9, 10] {
            assert_eq!(
                loaded.patterns[index],
                fixture.patterns[index],
                "pattern {}",
                index + 1
            );
        }
        assert_eq!(loaded.patterns[0].mode, 0); // spidercrack
        assert_eq!((loaded.surface3_scale, loaded.eq_chain), (2.0, 8));
        assert_eq!(loaded.surfaces.lookup(200, 32), 2);
    }

    /// Replays the retail capture: the local Cracks component's creation, then per capture frame
    /// its update (with the MixMap values that update read) and its process, each against the
    /// delivered words. `cargo test ... seams::tests::capture_replay -- --ignored --nocapture`
    ///
    /// Result: every word of every update and process delivery matches (74176 + 74204 rows) when
    /// the update reads the previous frame's state row and the process the current one; with a
    /// single lag for both, either the process words (w7, w9, w10, w13, w16) or the update words
    /// (w8, w12, w15, w17) drift. The bridge runs between the two ticks.
    #[test]
    #[ignore]
    fn capture_replay() {
        let (Some(root), Some(vault)) = (capture::root(), assets()) else {
            return;
        };
        let tuning = SeamTuning::load(&vault).unwrap();
        let states = capture::states(&root);
        let rows = capture::rows(&root, OBJECT);
        const LOCAL: &str = "4A26A8E0";
        let local: std::collections::HashSet<String> = rows
            .iter()
            .filter(|r| r.ctrl == LOCAL)
            .map(|r| r.payload.clone())
            .collect();
        let posts: Vec<_> = rows
            .iter()
            .filter(|r| r.kind == "PO" && local.contains(&r.payload))
            .collect();
        let first = posts[0].frame;
        assert!(
            posts.iter().all(|p| p.frame == first),
            "local seams are posted once"
        );
        for (update_lag, process_lag) in [(1u32, 0u32), (1, 1), (0, 0)] {
            let inputs_at = |frame: u32, lag: u32| {
                states
                    .get(&(frame - lag))
                    .map(|w| SeamInputs::from_capture(w))
            };
            // The capture's state rows start the frame after the posts (no wheel material yet).
            let initial = SeamInputs {
                wheel_material_620: [143; 4],
                ..SeamInputs::default()
            };
            let mut state =
                SeamState::create(&tuning, &inputs_at(first, process_lag).unwrap_or(initial));
            let mut post = Matches::new("post", SEAM_WORDS);
            for row in &posts {
                post.add(
                    row.frame,
                    &row.words,
                    &state.packets[row.words[14] as usize],
                );
            }
            let mut update = Matches::new("update", SEAM_WORDS);
            let mut process = Matches::new("process", SEAM_WORDS);
            let mut by_frame: std::collections::BTreeMap<u32, Vec<&capture::Row>> =
                Default::default();
            for row in rows
                .iter()
                .filter(|r| r.kind == "UP" && local.contains(&r.payload) && r.frame > first)
            {
                by_frame.entry(row.frame).or_default().push(row);
            }
            let mut missing = 0;
            for (&frame, frame_rows) in &by_frame {
                let (Some(earlier), Some(inputs)) =
                    (inputs_at(frame, update_lag), inputs_at(frame, process_lag))
                else {
                    missing += 1;
                    continue;
                };
                let (updates, processes): (Vec<&&capture::Row>, Vec<&&capture::Row>) =
                    frame_rows.iter().partition(|r| r.ctrl == LOCAL);
                for row in &updates {
                    let wheel = row.words[14] as usize;
                    let controls = Captured::from_reads(&row.reads, 0x824C_1F18..0x824C_2428);
                    state.update(&earlier, &controls, wheel);
                    update.add(frame, &row.words, &state.packets[wheel]);
                }
                if !processes.is_empty() {
                    state.process(&tuning, &inputs, 1.0 / 60.0).unwrap();
                    for row in &processes {
                        process.add(frame, &row.words, &state.packets[row.words[14] as usize]);
                    }
                }
            }
            println!(
                "=== update lag {update_lag}, process lag {process_lag} (frames skipped: {missing})"
            );
            post.print();
            update.print();
            process.print();
            if (update_lag, process_lag) == (1, 0) {
                for m in [&post, &update, &process] {
                    assert!(
                        m.exact.iter().all(|&e| e == m.total),
                        "{} words drift",
                        m.name
                    );
                }
            }
        }
    }
}
