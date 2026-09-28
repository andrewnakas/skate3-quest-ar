//! c_body_slide and c_cloth_falls (Clothing controller 40010060).
//!
//! Both live on the Clothing component. Its process `sub_824DBB68` runs the cloth-falls trigger
//! `sub_824DBF10`, then `sub_824DBBB8` (push-driven streamed foley, not a message family, not
//! ported here), then the body-slide trigger `sub_824DC0E8`; its update `sub_824DCB98` runs the
//! body-slide updater `sub_824DC578`, the cloth-falls updater `sub_824DCA48`, then
//! `sub_824DC7D8` (the streamed foley's). Component fields: `+36` previous `+676`, `+40` the
//! cloth-falls message, `+44` the cloth-falls level, `+60` the body-slide message.
//!
//! - c_body_slide (`sub_824B7070`, 52-byte object, 12 words, slot 21 = `0x8302EED0`): posts
//!   while a ragdoll body part slides (`+528..+548`) faster than the vault start speed, holds
//!   while above the keep speed.
//! - c_cloth_falls (`sub_824B72D8`, 44-byte object, 10 words, slot 23 = `0x8302EEE0`): posts
//!   when `+676` (bail) rises, releases on `+677` (end of bail) or when `+676` clears.
//!
//! They read the ragdoll fields +328 (|Skeleton+288|), +528..+548 / +560..+580 / +593
//! (`sub_82773298` body contacts from Collision+80..195, written by `sub_82BD60C8`) and +672
//! (0.25 × Σ Skeleton+560..572), all carried by [`AudioState`]. The packet logic is checked
//! against the retail capture with the captured state words.
//!
//! Tuning through the audio tuning holder `*(0x830CFDA4)`: +36 = class `6EBA5BCD3E38A98A` /
//! `default`, +64 = AudioSurfaceMap, +136 = class `A867FBE3454326FF` / `default`, +140 =
//! eEQChain class `42AFE160E647167C` / `default`.

use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::contacts::{SurfaceMap, clamp_word, vault_word};
use super::words::{THOUSAND, fctiwz};
use super::{Component, Controls, Tick, post, redeliver, release};

const SLIDE_CLASS: &str = "Hash_6EBA5BCD3E38A98A";
const FALLS_CLASS: &str = "Hash_A867FBE3454326FF";
const EQ_CLASS: &str = "Hash_42AFE160E647167C";
const DEFAULT_KEY: &str = "Hash_D7EDBD362D7D2152";

pub(crate) const BODY_SLIDE_WORDS: usize = 12;
pub(crate) const CLOTH_FALLS_WORDS: usize = 10;
const BODY_SLIDE: &str = "c_body_slide";
const CLOTH_FALLS: &str = "c_cloth_falls";

/// The audio-state fields `sub_824DC2B0`, `sub_824DC0E8` and `sub_824DC578` read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BodySlideInputs {
    pub com_speed_212: f32,
    /// +328: |Skeleton+288| (ragdoll body speed).
    pub body_speed_328: f32,
    /// +528..+548: per body part slide speed (record from Collision+80.., `sub_82773298`).
    pub slide_528: [f32; 6],
    /// +560..+580: per body part raw contact material (0 = none, else material + 1).
    pub material_560: [i32; 6],
    /// +593: body contact flag (type 4).
    pub flag_593: bool,
    pub bail_676: bool,
}

impl BodySlideInputs {
    /// Every field is in [`AudioState`]; `Option` is kept for the call sites.
    pub(crate) fn from_state(state: &AudioState) -> Option<Self> {
        Some(Self {
            com_speed_212: state.com_speed_212,
            body_speed_328: state.ragdoll_spin_328,
            slide_528: std::array::from_fn(|i| state.body_slide_528[i]),
            material_560: std::array::from_fn(|i| state.body_material_560[i] as i32),
            flag_593: state.face_contact_593,
            bail_676: state.bail_676,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_capture(words: &[u32]) -> Self {
        use super::contacts::capture::{byte, float, int};
        Self {
            com_speed_212: float(words, 212),
            body_speed_328: float(words, 328),
            slide_528: std::array::from_fn(|i| float(words, 528 + 4 * i)),
            material_560: std::array::from_fn(|i| int(words, 560 + 4 * i)),
            flag_593: byte(words, 593) != 0,
            bail_676: byte(words, 676) != 0,
        }
    }
}

/// The audio-state fields `sub_824DBF10` and `sub_824DCA48` read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ClothFallsInputs {
    /// +328: |Skeleton+288|.
    pub body_speed_328: f32,
    /// +672: 0.25 × Σ Skeleton+560..572 (limb speed relative to the COM).
    pub limb_speed_672: f32,
    pub bail_676: bool,
    pub bail_over_677: bool,
}

impl ClothFallsInputs {
    /// Every field is in [`AudioState`]; `Option` is kept for the call sites.
    pub(crate) fn from_state(state: &AudioState) -> Option<Self> {
        Some(Self {
            body_speed_328: state.ragdoll_spin_328,
            limb_speed_672: state.limb_speed_672,
            bail_676: state.bail_676,
            bail_over_677: state.bail_over_677,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_capture(words: &[u32]) -> Self {
        use super::contacts::capture::{byte, float};
        Self {
            body_speed_328: float(words, 328),
            limb_speed_672: float(words, 672),
            bail_676: byte(words, 676) != 0,
            bail_over_677: byte(words, 677) != 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BodySlideTuning {
    /// Holder +36 `787171ECD02DBBC3` (4.5): COM speed divisor of the speed word.
    pub speed_divisor: f32,
    /// Holder +36 `B021DB338B89D0F2` (350): start when the speed word is above it.
    pub start_speed: i32,
    /// Holder +36 `746EA8EF187E1571` (150): keep while above it.
    pub keep_speed: i32,
    /// Holder +36 `5D2F244E82CF7255` (8.0): body speed divisor of w10.
    pub body_divisor: f32,
    /// Holder +140 `4A022FEF9905D8F4` (7): w11.
    pub eq: i32,
    pub surfaces: SurfaceMap,
}

impl BodySlideTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            speed_divisor: vault.float(SLIDE_CLASS, DEFAULT_KEY, "Hash_787171ECD02DBBC3")?,
            start_speed: vault.integer(SLIDE_CLASS, DEFAULT_KEY, "Hash_B021DB338B89D0F2")? as i32,
            keep_speed: vault.integer(SLIDE_CLASS, DEFAULT_KEY, "Hash_746EA8EF187E1571")? as i32,
            body_divisor: vault.float(SLIDE_CLASS, DEFAULT_KEY, "Hash_5D2F244E82CF7255")?,
            eq: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_4A022FEF9905D8F4")? as i32,
            surfaces: SurfaceMap::load(vault)?,
        })
    }
}

/// `sub_824DC2B0`'s three outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BodyContact {
    /// Body slide type 0..4 (default 2).
    pub kind: i32,
    /// `fctiwz(+212 / divisor × 1000)` (unclamped).
    pub speed: i32,
    /// Any part's |slide| > 0.
    pub sliding: bool,
}

/// `sub_824DC2B0`. A sliding part sets the sliding flag and, with +593, the type-4 flag; its
/// material is taken if non-zero — parts 0 and 1 unconditionally (part 1 overrides part 0),
/// parts 2..5 only while none was taken. The material (raw − 1; none or out of 0..143 → 143)
/// selects the AudioSurfaceMap `+40` body slide type, 4 when flagged, 2 without a material.
pub(crate) fn body_contact(inputs: &BodySlideInputs, tuning: &BodySlideTuning) -> BodyContact {
    let mut material = 0;
    let mut flagged = false;
    let mut sliding = false;
    for part in 0..6 {
        if inputs.slide_528[part].abs() > 0.0 {
            sliding = true;
            if inputs.flag_593 {
                flagged = true;
            }
            let raw = inputs.material_560[part];
            if raw != 0 && (part < 2 || material == 0) {
                material = raw;
            }
        }
    }
    let speed = fctiwz(inputs.com_speed_212 / tuning.speed_divisor * THOUSAND);
    let material = if material == 0 {
        143
    } else {
        let value = material - 1;
        if value > 143 || value < 0 { 143 } else { value }
    };
    let kind = if material == 143 {
        2
    } else if flagged {
        4
    } else {
        tuning.surfaces.lookup(material, 40) as i32
    };
    BodyContact {
        kind,
        speed,
        sliding,
    }
}

/// `sub_824DC0E8` → `sub_824B7070`: the posted packet.
pub(crate) fn body_slide_constructor(
    tuning: &BodySlideTuning,
    inputs: &BodySlideInputs,
    contact: &BodyContact,
) -> [u32; BODY_SLIDE_WORDS] {
    let mut words = [0; BODY_SLIDE_WORDS];
    words[3] = clamp_word(contact.speed, 0, 1_000);
    words[4] = 25_000;
    words[8] = clamp_word(contact.kind, 0, 4);
    words[9] = clamp_word(i32::from(inputs.bail_676), 0, 1);
    words[11] = clamp_word(tuning.eq, 0, 32_767);
    words
}

/// `sub_824DC578`'s rewrite of the held packet.
pub(crate) fn body_slide_update(
    words: &mut [u32; BODY_SLIDE_WORDS],
    tuning: &BodySlideTuning,
    inputs: &BodySlideInputs,
    controls: &dyn Controls,
) {
    let contact = body_contact(inputs, tuning);
    let body = fctiwz(inputs.body_speed_328 / tuning.body_divisor * THOUSAND);
    words[0] = 32_767;
    words[1] = clamp_word(controls.raw(0) as i32, 0, 0xFFFF);
    words[2] = clamp_word(controls.pitch(6), 0, 8_192);
    words[3] = clamp_word(contact.speed, 0, 1_000);
    words[7] = clamp_word(controls.level(5) as i32, 0, 32_767);
    words[8] = clamp_word(contact.kind, 0, 4);
    words[10] = clamp_word(body, 0, 1_000);
    words[9] = clamp_word(i32::from(inputs.bail_676), 0, 1);
}

/// What `sub_824DC0E8` does to the body-slide message this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlideAction {
    Keep,
    Release,
    Post,
    Nothing,
}

/// `sub_824DC0E8`: held → keep while speed > keep threshold and sliding, else release; not
/// held → post when sliding and speed > start threshold.
pub(crate) fn body_slide_action(
    tuning: &BodySlideTuning,
    contact: &BodyContact,
    held: bool,
) -> SlideAction {
    if held {
        if contact.speed > tuning.keep_speed && contact.sliding {
            SlideAction::Keep
        } else {
            SlideAction::Release
        }
    } else if contact.sliding && contact.speed > tuning.start_speed {
        SlideAction::Post
    } else {
        SlideAction::Nothing
    }
}

pub(crate) struct BodySlide {
    tuning: BodySlideTuning,
    /// Component +60.
    held: Option<(u32, [u32; BODY_SLIDE_WORDS])>,
}

impl BodySlide {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            tuning: BodySlideTuning::load(vault)?,
            held: None,
        })
    }
}

impl Component for BodySlide {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let Some(inputs) = BodySlideInputs::from_state(tick.audio) else {
            return Ok(());
        };
        let contact = body_contact(&inputs, &self.tuning);
        match body_slide_action(&self.tuning, &contact, self.held.is_some()) {
            SlideAction::Release => {
                let mut handle = self.held.take().map(|(handle, _)| handle);
                release(tick.runtime, &mut handle)?;
            }
            SlideAction::Post => {
                let words = body_slide_constructor(&self.tuning, &inputs, &contact);
                let handle = post(tick.runtime, BODY_SLIDE, &words)?;
                self.held = Some((handle, words));
            }
            SlideAction::Keep | SlideAction::Nothing => {}
        }
        Ok(())
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        let Some(inputs) = BodySlideInputs::from_state(tick.audio) else {
            return Ok(());
        };
        if let Some((handle, words)) = self.held.as_mut() {
            body_slide_update(words, &self.tuning, &inputs, tick.controls);
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClothFallsTuning {
    /// Holder +136 `0A9A9BD1150FE838` (5.0): the speed range (used as its reciprocal).
    pub speed_range: f32,
    /// Holder +140 `E633C8F009CAEFFC` (5): w9.
    pub eq: i32,
}

impl ClothFallsTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            speed_range: vault.float(FALLS_CLASS, DEFAULT_KEY, "Hash_0A9A9BD1150FE838")?,
            eq: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_E633C8F009CAEFFC")? as i32,
        })
    }
}

/// `sub_824DBF10`'s two speed words: `fctiwz(1/range × +328 × 1000)` (the constructor's w3) and
/// the level `max(that, fctiwz(+672 × 1/range × 1000))` it stores at component +44.
pub(crate) fn cloth_falls_speeds(
    tuning: &ClothFallsTuning,
    inputs: &ClothFallsInputs,
) -> (i32, i32) {
    let reciprocal = 1.0 / tuning.speed_range;
    let body = fctiwz(reciprocal * inputs.body_speed_328 * THOUSAND);
    let limbs = fctiwz(inputs.limb_speed_672 * reciprocal * THOUSAND);
    (body, if body > limbs { body } else { limbs })
}

/// `sub_824B72D8`: the posted packet.
pub(crate) fn cloth_falls_constructor(
    tuning: &ClothFallsTuning,
    body: i32,
) -> [u32; CLOTH_FALLS_WORDS] {
    let mut words = [0; CLOTH_FALLS_WORDS];
    words[3] = clamp_word(body, 0, 1_000);
    words[4] = 25_000;
    words[9] = clamp_word(tuning.eq, 0, 32_767);
    words
}

/// `sub_824DCA48`'s rewrite of the held packet; `level` is component +44.
pub(crate) fn cloth_falls_update(
    words: &mut [u32; CLOTH_FALLS_WORDS],
    level: i32,
    controls: &dyn Controls,
) {
    let gain = controls.level(2) as i32;
    words[1] = clamp_word(controls.raw(0) as i32, 0, 0xFFFF);
    words[2] = clamp_word(controls.pitch(1), 0, 8_192);
    words[3] = clamp_word(level, 0, 1_000);
    words[0] = 32_767;
    words[7] = clamp_word(gain, 0, 32_767);
}

pub(crate) struct ClothFalls {
    tuning: ClothFallsTuning,
    /// Component +36: the previous frame's +676.
    previous_bail: bool,
    /// Component +44.
    level: i32,
    /// Component +40.
    held: Option<(u32, [u32; CLOTH_FALLS_WORDS])>,
}

/// What `sub_824DBF10` does to the cloth-falls message this frame.
pub(crate) fn cloth_falls_action(
    inputs: &ClothFallsInputs,
    previous_bail: bool,
    held: bool,
) -> SlideAction {
    if held {
        if inputs.bail_over_677 || !inputs.bail_676 {
            SlideAction::Release
        } else {
            SlideAction::Keep
        }
    } else if !previous_bail && inputs.bail_676 {
        SlideAction::Post
    } else {
        SlideAction::Nothing
    }
}

impl ClothFalls {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            tuning: ClothFallsTuning::load(vault)?,
            previous_bail: false,
            level: 0,
            held: None,
        })
    }
}

impl Component for ClothFalls {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let Some(inputs) = ClothFallsInputs::from_state(tick.audio) else {
            return Ok(());
        };
        let (body, level) = cloth_falls_speeds(&self.tuning, &inputs);
        self.level = level;
        match cloth_falls_action(&inputs, self.previous_bail, self.held.is_some()) {
            SlideAction::Release => {
                let mut handle = self.held.take().map(|(handle, _)| handle);
                release(tick.runtime, &mut handle)?;
            }
            SlideAction::Post => {
                let words = cloth_falls_constructor(&self.tuning, body);
                let handle = post(tick.runtime, CLOTH_FALLS, &words)?;
                self.held = Some((handle, words));
            }
            SlideAction::Keep | SlideAction::Nothing => {}
        }
        self.previous_bail = inputs.bail_676;
        Ok(())
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        if let Some((handle, words)) = self.held.as_mut() {
            cloth_falls_update(words, self.level, tick.controls);
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

/// The Clothing component in retail call order (process: cloth falls, then body slide; update:
/// body slide, then cloth falls). The streamed foley between them (`sub_824DBBB8` /
/// `sub_824DC7D8`) is not a message family and is not part of this port.
pub(crate) struct Clothing {
    pub cloth_falls: ClothFalls,
    pub body_slide: BodySlide,
}

impl Clothing {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self {
            cloth_falls: ClothFalls::new(vault)?,
            body_slide: BodySlide::new(vault)?,
        })
    }
}

impl Component for Clothing {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        self.cloth_falls.process(tick)?;
        self.body_slide.process(tick)
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        self.body_slide.update(tick)?;
        self.cloth_falls.update(tick)
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::capture::{self, Captured, Matches};
    use super::*;

    fn slide_tuning() -> BodySlideTuning {
        // Vault values; AudioSurfaceMap +40 lane: material 15 → 1, everything else 2 (entry 94).
        let mut entries = vec![[0u32; 18]; 95];
        for entry in &mut entries {
            entry[10] = 2;
        }
        entries[15][10] = 1;
        BodySlideTuning {
            speed_divisor: 4.5,
            start_speed: 350,
            keep_speed: 150,
            body_divisor: 8.0,
            eq: 7,
            surfaces: SurfaceMap::from_entries(entries),
        }
    }

    fn falls_tuning() -> ClothFallsTuning {
        ClothFallsTuning {
            speed_range: 5.0,
            eq: 5,
        }
    }

    #[test]
    fn body_contact_prefers_part_one_then_the_first_later_part() {
        let tuning = slide_tuning();
        let mut inputs = BodySlideInputs {
            com_speed_212: 2.25,
            ..Default::default()
        };
        assert_eq!(
            body_contact(&inputs, &tuning),
            BodyContact {
                kind: 2,
                speed: 500,
                sliding: false
            }
        );
        // Parts 0 and 1 slide: part 1's material (16 → 15 → type 1) wins.
        inputs.slide_528 = [0.5, -0.25, 0.0, 0.0, 0.0, 0.0];
        inputs.material_560 = [4, 16, 0, 0, 0, 0];
        assert_eq!(body_contact(&inputs, &tuning).kind, 1);
        // Parts 2 and 3 slide: the first non-zero one wins.
        inputs.slide_528 = [0.0, 0.0, 0.5, 0.5, 0.0, 0.0];
        inputs.material_560 = [0, 0, 16, 4, 0, 0];
        assert_eq!(body_contact(&inputs, &tuning).kind, 1);
        // +593 with a material → 4; without a material → 2.
        inputs.flag_593 = true;
        assert_eq!(body_contact(&inputs, &tuning).kind, 4);
        inputs.material_560 = [0; 6];
        assert_eq!(body_contact(&inputs, &tuning).kind, 2);
        // Out of range materials are none.
        inputs.flag_593 = false;
        inputs.material_560 = [0, 0, 145, 0, 0, 0];
        assert_eq!(body_contact(&inputs, &tuning).kind, 2);
    }

    #[test]
    fn body_slide_starts_above_350_and_holds_above_150() {
        let tuning = slide_tuning();
        let contact = |speed, sliding| BodyContact {
            kind: 2,
            speed,
            sliding,
        };
        assert_eq!(
            body_slide_action(&tuning, &contact(350, true), false),
            SlideAction::Nothing
        );
        assert_eq!(
            body_slide_action(&tuning, &contact(351, true), false),
            SlideAction::Post
        );
        assert_eq!(
            body_slide_action(&tuning, &contact(900, false), false),
            SlideAction::Nothing
        );
        assert_eq!(
            body_slide_action(&tuning, &contact(151, true), true),
            SlideAction::Keep
        );
        assert_eq!(
            body_slide_action(&tuning, &contact(150, true), true),
            SlideAction::Release
        );
        assert_eq!(
            body_slide_action(&tuning, &contact(900, false), true),
            SlideAction::Release
        );
    }

    #[test]
    fn body_slide_packets() {
        let tuning = slide_tuning();
        let inputs = BodySlideInputs {
            com_speed_212: 9.0,
            body_speed_328: 4.0,
            slide_528: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            bail_676: true,
            ..Default::default()
        };
        let contact = body_contact(&inputs, &tuning);
        let mut words = body_slide_constructor(&tuning, &inputs, &contact);
        assert_eq!(words, [0, 0, 0, 1000, 25000, 0, 0, 0, 2, 1, 0, 7]);
        struct Reads;
        impl Controls for Reads {
            fn raw(&self, _: u32) -> u32 {
                0x1_0000
            }
            fn pitch(&self, id: u32) -> i32 {
                assert_eq!(id, 6);
                0x1004
            }
            fn level(&self, id: u32) -> u32 {
                assert_eq!(id, 5);
                0x2000
            }
        }
        body_slide_update(&mut words, &tuning, &inputs, &Reads);
        assert_eq!(
            words,
            [
                32767, 0xFFFF, 0x1004, 1000, 25000, 0, 0, 0x2000, 2, 1, 500, 7
            ]
        );
    }

    #[test]
    fn cloth_falls_posts_on_the_bail_edge_and_releases_at_its_end() {
        let tuning = falls_tuning();
        let mut inputs = ClothFallsInputs {
            body_speed_328: 1.0,
            limb_speed_672: 3.0,
            bail_676: true,
            ..Default::default()
        };
        assert_eq!(cloth_falls_speeds(&tuning, &inputs), (200, 600));
        assert_eq!(cloth_falls_action(&inputs, false, false), SlideAction::Post);
        assert_eq!(
            cloth_falls_action(&inputs, true, false),
            SlideAction::Nothing
        );
        assert_eq!(cloth_falls_action(&inputs, true, true), SlideAction::Keep);
        inputs.bail_over_677 = true;
        assert_eq!(
            cloth_falls_action(&inputs, true, true),
            SlideAction::Release
        );
        assert_eq!(
            cloth_falls_constructor(&tuning, 200),
            [0, 0, 0, 200, 25000, 0, 0, 0, 0, 5]
        );
        assert_eq!(cloth_falls_constructor(&tuning, 1600)[3], 1000);
    }

    #[test]
    fn inputs_read_the_ragdoll_fields_of_the_audio_state() {
        let mut state = AudioState::default();
        state.ragdoll_spin_328 = 2.0;
        state.limb_speed_672 = 3.0;
        state.body_slide_528[5] = 1.5;
        state.body_slide_528[6] = 9.0;
        state.body_material_560[5] = 16;
        state.face_contact_593 = true;
        let slide = BodySlideInputs::from_state(&state).unwrap();
        assert_eq!(
            (
                slide.body_speed_328,
                slide.slide_528[5],
                slide.material_560[5]
            ),
            (2.0, 1.5, 16)
        );
        assert!(slide.flag_593);
        let falls = ClothFallsInputs::from_state(&state).unwrap();
        assert_eq!((falls.body_speed_328, falls.limb_speed_672), (2.0, 3.0));
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
    fn clothing_tuning_reads_the_vault() {
        let Some(vault) = vault() else { return };
        let slide = BodySlideTuning::load(&vault).unwrap();
        assert_eq!(
            (
                slide.speed_divisor,
                slide.start_speed,
                slide.keep_speed,
                slide.body_divisor,
                slide.eq
            ),
            (4.5, 350, 150, 8.0, 7)
        );
        // Element 94 (materials ≥ 94) has body slide type 2.
        assert_eq!(slide.surfaces.lookup(94, 40), 2);
        let falls = ClothFallsTuning::load(&vault).unwrap();
        assert_eq!((falls.speed_range, falls.eq), (5.0, 5));
    }

    const LOCAL: &str = "4A26A900";

    /// Frames whose packets belong to the local skater: UP rows by the local controller, and
    /// posts/releases whose payload/node a local UP row carries.
    /// The local skater's rows. Message objects and nodes are reused across skaters, so a post
    /// is local when the next update of its payload is, and a release when the last update of
    /// its node was.
    fn local_rows(
        rows: &[capture::Row],
    ) -> (Vec<&capture::Row>, Vec<&capture::Row>, Vec<&capture::Row>) {
        let updates: Vec<&capture::Row> = rows.iter().filter(|r| r.kind == "UP").collect();
        let posts = rows
            .iter()
            .filter(|r| r.kind == "PO")
            .filter(|r| {
                updates
                    .iter()
                    .find(|u| u.payload == r.payload && u.frame > r.frame)
                    .is_some_and(|u| u.ctrl == LOCAL)
            })
            .collect();
        let releases = rows
            .iter()
            .filter(|r| r.kind == "RL")
            .filter(|r| {
                updates
                    .iter()
                    .rev()
                    .find(|u| u.node == r.node && u.frame <= r.frame)
                    .is_some_and(|u| u.ctrl == LOCAL)
            })
            .collect();
        let updates = rows
            .iter()
            .filter(|r| r.kind == "UP" && r.ctrl == LOCAL)
            .collect();
        (posts, releases, updates)
    }

    /// Replays the capture through the pure functions in retail's observed frame order (the
    /// frame's update before its process: a post's first redelivery is on the next frame).
    #[test]
    #[ignore = "needs .local/captures"]
    fn clothing_replays_the_retail_capture() {
        let Some(root) = capture::root() else { return };
        let Some(vault) = vault() else { return };
        let slide_tuning = BodySlideTuning::load(&vault).unwrap();
        let falls_tuning = ClothFallsTuning::load(&vault).unwrap();
        let states = capture::states(&root);
        let slide_rows = capture::rows(&root, BODY_SLIDE);
        let falls_rows = capture::rows(&root, CLOTH_FALLS);
        let (slide_posts, slide_releases, slide_updates) = local_rows(&slide_rows);
        let (falls_posts, falls_releases, falls_updates) = local_rows(&falls_rows);
        let by_frame = |rows: &[&capture::Row]| -> std::collections::HashMap<u32, capture::Row> {
            rows.iter().map(|r| (r.frame, (*r).clone())).collect()
        };
        let (slide_up, falls_up) = (by_frame(&slide_updates), by_frame(&falls_updates));
        let (slide_po, falls_po) = (by_frame(&slide_posts), by_frame(&falls_posts));
        // The updaters read the previous frame's state row, the triggers the current one.
        for (update_lag, process_lag) in [(1u32, 0u32), (1, 1), (0, 0)] {
            let mut slide_post_m = Matches::new("body slide posts", BODY_SLIDE_WORDS);
            let mut slide_up_m = Matches::new("body slide updates", BODY_SLIDE_WORDS);
            let mut falls_post_m = Matches::new("cloth falls posts", CLOTH_FALLS_WORDS);
            let mut falls_up_m = Matches::new("cloth falls updates", CLOTH_FALLS_WORDS);
            let mut slide: Option<[u32; BODY_SLIDE_WORDS]> = None;
            let mut falls: Option<[u32; CLOTH_FALLS_WORDS]> = None;
            let (mut previous_bail, mut level) = (false, 0);
            let mut ours = [vec![], vec![], vec![], vec![], vec![], vec![]];
            for (&frame, _) in states.range(2709..) {
                let (Some(earlier), Some(state)) = (
                    states.get(&(frame - update_lag)),
                    states.get(&(frame - process_lag)),
                ) else {
                    continue;
                };
                let slide_in = BodySlideInputs::from_capture(earlier);
                // Update: body slide, then cloth falls.
                if let Some(words) = slide.as_mut() {
                    ours[2].push(frame);
                    if let Some(row) = slide_up.get(&frame) {
                        let controls = Captured::from_reads(&row.reads, 0x824D_C578..0x824D_C7D8);
                        body_slide_update(words, &slide_tuning, &slide_in, &controls);
                        slide_up_m.add(frame, &row.words, words);
                    }
                }
                if let Some(words) = falls.as_mut() {
                    ours[5].push(frame);
                    if let Some(row) = falls_up.get(&frame) {
                        let controls = Captured::from_reads(&row.reads, 0x824D_CA48..0x824D_CB98);
                        cloth_falls_update(words, level, &controls);
                        falls_up_m.add(frame, &row.words, words);
                    }
                }
                // Process: cloth falls, then body slide.
                let slide_in = BodySlideInputs::from_capture(state);
                let falls_in = ClothFallsInputs::from_capture(state);
                let (body, new_level) = cloth_falls_speeds(&falls_tuning, &falls_in);
                level = new_level;
                match cloth_falls_action(&falls_in, previous_bail, falls.is_some()) {
                    SlideAction::Release => {
                        falls = None;
                        ours[4].push(frame);
                    }
                    SlideAction::Post => {
                        let words = cloth_falls_constructor(&falls_tuning, body);
                        if let Some(row) = falls_po.get(&frame) {
                            falls_post_m.add(frame, &row.words, &words);
                        }
                        falls = Some(words);
                        ours[3].push(frame);
                    }
                    _ => {}
                }
                previous_bail = falls_in.bail_676;
                let contact = body_contact(&slide_in, &slide_tuning);
                match body_slide_action(&slide_tuning, &contact, slide.is_some()) {
                    SlideAction::Release => {
                        slide = None;
                        ours[1].push(frame);
                    }
                    SlideAction::Post => {
                        let words = body_slide_constructor(&slide_tuning, &slide_in, &contact);
                        if let Some(row) = slide_po.get(&frame) {
                            slide_post_m.add(frame, &row.words, &words);
                        }
                        slide = Some(words);
                        ours[0].push(frame);
                    }
                    _ => {}
                }
            }
            println!("==== update lag {update_lag}, process lag {process_lag}");
            for m in [&slide_post_m, &slide_up_m] {
                m.print();
            }
            for m in [&falls_post_m, &falls_up_m] {
                m.print();
            }
            let frames = |rows: &[&capture::Row]| rows.iter().map(|r| r.frame).collect::<Vec<_>>();
            let same = |a: &[u32], b: &[u32]| a.iter().filter(|f| b.contains(f)).count();
            let report = |name: &str, retail: Vec<u32>, ours: &[u32]| {
                println!(
                    "{name}: retail {} ours {} same-frame {}",
                    retail.len(),
                    ours.len(),
                    same(&retail, ours)
                );
                if retail.len() < 60 {
                    println!("   retail {retail:?}\n   ours   {ours:?}");
                }
            };
            report("body slide posts", frames(&slide_posts), &ours[0]);
            report("body slide releases", frames(&slide_releases), &ours[1]);
            report("body slide update frames", frames(&slide_updates), &ours[2]);
            report("cloth falls posts", frames(&falls_posts), &ours[3]);
            report("cloth falls releases", frames(&falls_releases), &ours[4]);
            report(
                "cloth falls update frames",
                frames(&falls_updates),
                &ours[5],
            );
        }
    }
}
