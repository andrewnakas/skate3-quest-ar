//! Class_Treatment (component vtable 0x822FCDA0, Treatments controller 40010070).
//!
//! - Process `sub_824DD408` (slot 9), gated on the owner's local byte `[this+28]+72`: while
//!   component `+36` is empty, allocate the 92-byte object and run its constructor
//!   `sub_824B0080`, which posts itself (message slot `0x8302EE70`). Never released.
//! - Update `sub_824DD6F0` (slot 10): rewrite w0, w3, w4, w7..w14 of the held packet in place
//!   and redeliver. The object is the packet, so words the updater leaves alone keep their last
//!   value (w13 in particular, see [`TreatmentGlobal`]).
//!
//! Not ported (out of scope): the `hall_of_meat_slo_mo` companion at `+40` (15 words,
//! `sub_824AF368`; posted by the process while the game mode `[0x830CFDC4]+1060` is 7 and
//! redelivered by the second half of the updater).
//!
//! Tuning: the audio tuning holder `*(0x830CFDA4)+68` = class `C1831BDB6CB1B1EA`, key
//! `EE7B8A8A893A4E30`.

use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::contacts::clamp_word;
use super::words::{THOUSAND, fctiwz};
use super::{Component, Controls, Tick, post, redeliver};

const TUNING_CLASS: &str = "Hash_C1831BDB6CB1B1EA";
const TUNING_KEY: &str = "Hash_EE7B8A8A893A4E30";

/// Packet length: the constructor's 92-byte object minus the 4-byte header.
pub(crate) const TREATMENT_WORDS: usize = 22;
const OBJECT: &str = "Class_Treatment";

/// `0x821161A0` (10000), `0x822F9408` (166.66667, bits read from the image; the decimal
/// 166.667 rounds to a different float), `0x820BD5C4` (500).
const TEN_THOUSAND: f32 = f32::from_bits(0x461C_4000);
const HEIGHT_SCALE: f32 = f32::from_bits(0x4326_AAAB);
const TIME_SCALE: f32 = f32::from_bits(0x43FA_0000);

/// PowerPC `fsel`: `a >= 0 ? b : c` (NaN selects `c`).
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// The unidentified global `G = *(*(0x83083C38) + 0x2FCB4)` the updater reads for w11..w13.
///
/// What is known: `sub_8279E800` resets `G+16..` with `sub_827AB6E0` (every field below to
/// 0 / false); the audio-state MixMap input 11 (`sub_824B19C8`) and the rolling-SFX path
/// `sub_824BC188` read the same byte `G+16` (the latter also `G+24 × 32767` into input 8);
/// `sub_827EDE58` adds `G+168` to a skater float at `+3140`. Its writer was not found. The engine
/// has no equivalent, so it stays at the reset values: retail's default path (w11 = w12 = 0,
/// w13 keeps the constructor's 0). The retail capture shows `G+164` set with `G+168 ≈ 1.0` for
/// most of play (w13 = 10000), which this cannot reproduce.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TreatmentGlobal {
    pub flag_16: bool,
    pub value_24: f32,
    pub flag_164: bool,
    pub value_168: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TreatmentTuning {
    /// `32F9111CBF746F34` (7000), `E4BC6FE030A553C4` (28000), `1F11951C2AF58CC7` (32767):
    /// constructor arguments r4..r6 → w15..w17.
    pub levels: [i32; 3],
}

impl TreatmentTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let int = |name: &str| {
            vault
                .integer(TUNING_CLASS, TUNING_KEY, name)
                .map(|v| v as i32)
        };
        Ok(Self {
            levels: [
                int("Hash_32F9111CBF746F34")?,
                int("Hash_E4BC6FE030A553C4")?,
                int("Hash_1F11951C2AF58CC7")?,
            ],
        })
    }
}

/// The owner bytes and settings the process passes to the constructor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TreatmentOwner {
    /// Owner byte `[this+28]+72` (local player).
    pub local_72: bool,
    /// Owner word `[this+28]+64` (player index; 0 for the local player).
    pub index_64: i32,
    /// The game-settings SFX pack: `[0x830CFDC4]+564` enables a lookup of the pack record
    /// `+568` in class `11A631878B239355` (whose `default` record names
    /// `ID_GAMESETTINGS_SFXPACK_OFF`), Bool `DE65D9E08266B102` via `sub_82484460`. Off in the
    /// retail capture (w18 = 0 throughout) and not modelled by the engine.
    pub sfx_pack_564: bool,
}

impl TreatmentOwner {
    pub(crate) const LOCAL: Self = Self {
        local_72: true,
        index_64: 0,
        sfx_pack_564: false,
    };
}

/// `sub_824DD408` → `sub_824B0080`: the posted packet.
pub(crate) fn treatment_constructor(
    tuning: &TreatmentTuning,
    owner: TreatmentOwner,
) -> [u32; TREATMENT_WORDS] {
    let mut words = [0; TREATMENT_WORDS];
    words[1] = 32_767;
    words[4] = 4_096;
    words[5] = 25_000;
    words[10] = 500;
    words[15] = clamp_word(tuning.levels[0], 0, 32_767);
    words[16] = clamp_word(tuning.levels[1], 0, 32_767);
    words[17] = clamp_word(tuning.levels[2], 0, 32_767);
    words[18] = clamp_word(i32::from(owner.sfx_pack_564), 0, 1);
    words[19] = clamp_word(i32::from(owner.local_72 && owner.index_64 == 0), 0, 1);
    words[20] = clamp_word(i32::from(owner.local_72), 0, 1);
    words[21] = 8;
    words
}

/// `sub_824DD6F0`'s rewrite of the held packet (first half; the second half is the
/// hall_of_meat companion).
pub(crate) fn treatment_update(
    words: &mut [u32; TREATMENT_WORDS],
    audio: &AudioState,
    controls: &dyn Controls,
    global: &TreatmentGlobal,
) {
    words[0] = clamp_word(controls.level(2) as i32, 0, 32_767);
    words[3] = clamp_word(controls.raw(0) as i32, 0, 0xFFFF);
    words[4] = clamp_word(controls.pitch(1), 0, 8_192);
    words[10] = clamp_word(fctiwz(audio.time_scale_220 * TIME_SCALE), 0, 1_000);
    // cntlzw(byte224) >> 5: 1 when +224 is clear.
    words[14] = clamp_word(i32::from(!audio.paused_224), 0, 1);
    words[7] = clamp_word(fctiwz(audio.air_time_236 * THOUSAND), 0, 10_000);
    words[8] = clamp_word(fctiwz(audio.air_until_landing_240 * THOUSAND), 0, 10_000);
    // f11 = h × 166.667; f9 = fsel(−f11, 0, f11); f7 = fsel(1000 − f9, f9, 1000).
    let height = audio.air_jump_height_260 * HEIGHT_SCALE;
    let low = fsel(-height, 0.0, height);
    words[9] = clamp_word(fctiwz(fsel(THOUSAND - low, low, THOUSAND)), 0, 10_000);
    words[12] = 0;
    words[11] = 0;
    if global.flag_16 {
        words[11] = clamp_word(fctiwz(global.value_24 * THOUSAND), 0, 1_000);
    }
    if global.flag_164 {
        words[12] = 1;
        words[13] = clamp_word(fctiwz(global.value_168 * TEN_THOUSAND), 0, 10_000);
    }
}

pub(crate) struct Treatment {
    tuning: TreatmentTuning,
    owner: TreatmentOwner,
    /// See [`TreatmentGlobal`]; stays at its reset values.
    pub global: TreatmentGlobal,
    /// Component `+36`.
    held: Option<(u32, [u32; TREATMENT_WORDS])>,
}

impl Treatment {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            tuning: TreatmentTuning::load(vault)?,
            owner: TreatmentOwner::LOCAL,
            global: TreatmentGlobal::default(),
            held: None,
        })
    }
}

impl Component for Treatment {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        if !self.owner.local_72 || self.held.is_some() {
            return Ok(());
        }
        let words = treatment_constructor(&self.tuning, self.owner);
        let handle = post(tick.runtime, OBJECT, &words)?;
        self.held = Some((handle, words));
        Ok(())
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        if let Some((handle, words)) = self.held.as_mut() {
            treatment_update(words, tick.audio, tick.controls, &self.global);
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::capture::{self, Captured, Matches};
    use super::*;

    fn tuning() -> TreatmentTuning {
        TreatmentTuning {
            levels: [7_000, 28_000, 32_767],
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
    fn treatment_constructor_matches_the_retail_post() {
        // Retail post, frame 2708.
        assert_eq!(
            treatment_constructor(&tuning(), TreatmentOwner::LOCAL),
            [
                0, 32767, 0, 0, 4096, 25000, 0, 0, 0, 0, 500, 0, 0, 0, 0, 7000, 28000, 32767, 0, 1,
                1, 8
            ]
        );
    }

    #[test]
    fn treatment_update_matches_a_retail_row_and_keeps_w13() {
        // Frame 2710 (state 2709): reads 60:2 = 0x130F, 52:0 = 0xFFFF, 56:1 = 0xFF6.
        let mut words = treatment_constructor(&tuning(), TreatmentOwner::LOCAL);
        words[13] = 10_000;
        let audio = {
            let mut s = AudioState::default();
            s.time_scale_220 = 1.0;
            s.air_time_236 = f32::from_bits(0x3F33_3333);
            s
        };
        let controls = Fixed(&[(60, 2, 0x130F), (52, 0, 0xFFFF), (56, 1, 0xFF6)]);
        treatment_update(&mut words, &audio, &controls, &TreatmentGlobal::default());
        assert_eq!(words[0], 0x130F);
        assert_eq!(words[3], 0xFFFF);
        assert_eq!(words[4], 0xFF6);
        // 0.7f × 1000 is 700 in single precision.
        assert_eq!(words[7], 700);
        assert_eq!(words[10], 500);
        assert_eq!(words[14], 1);
        // The updater writes w13 only while G+164 is set.
        assert_eq!((words[11], words[12], words[13]), (0, 0, 10_000));
        let global = TreatmentGlobal {
            flag_16: true,
            value_24: 0.11,
            flag_164: true,
            value_168: 0.9758,
        };
        treatment_update(&mut words, &audio, &controls, &global);
        assert_eq!((words[11], words[12], words[13]), (110, 1, 9_758));
    }

    /// Replays the retail capture (`.local/captures/extract`): every Class_Treatment update
    /// against the one-frame-earlier audio state and the update's own controller reads.
    #[test]
    #[ignore = "needs the retail capture in .local"]
    fn treatment_replays_the_retail_capture() {
        let Some(root) = capture::root() else { return };
        let states = capture::states(&root);
        let mut matches = Matches::new("Class_Treatment", TREATMENT_WORDS);
        let mut words = None;
        for row in capture::rows(&root, "Class_Treatment") {
            match row.kind.as_str() {
                "PO" => {
                    let ours = treatment_constructor(&tuning(), TreatmentOwner::LOCAL);
                    assert_eq!(
                        &row.words[..TREATMENT_WORDS],
                        &ours,
                        "post at {}",
                        row.frame
                    );
                    words = Some(ours);
                }
                "UP" => {
                    let (Some(held), Some(state)) = (words.as_mut(), states.get(&(row.frame - 1)))
                    else {
                        continue;
                    };
                    let audio = AudioState::from_capture(state);
                    let controls = Captured::from_reads(&row.reads, 0x824D_D6F0..0x824D_DC10);
                    treatment_update(held, &audio, &controls, &TreatmentGlobal::default());
                    matches.add(row.frame, &row.words[..TREATMENT_WORDS], held);
                    // Carry retail's G-driven words forward so later rows compare the rest.
                    held[11..14].copy_from_slice(&row.words[11..14]);
                }
                _ => {}
            }
        }
        matches.print();
    }
}
