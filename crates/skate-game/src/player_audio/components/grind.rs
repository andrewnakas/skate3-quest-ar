//! Class_grind (component vtable 0x822FC728, Rail controller 40010030).
//!
//! Component constructor `sub_824C2698`: holders `+36` (main layer) and `+40` (the layer-1
//! companion) empty, `+56` grind family −1, `+140` speed word 0.
//!
//! - Process `sub_824C28B0(dt)` (slot 9). Controller inputs (vfunc 8 on `[this+12]`): id 0 =
//!   32767 while `+341` (grinding) else 0; id 1 = 32767 on the frame grinding ends (`+342 &&
//!   !+341`) else 0. While `+341` the speed word `+140` is recomputed. With nothing held and
//!   `+341`: surface = 4 when `+692` is 143 (none), else the AudioSurfaceMap `+16` lane of the
//!   material (`sub_82494E18`); a map result of 14 posts nothing (and skips the rest of the
//!   call). `+56 = +192`; layer = family 1/2/4 → 0, 5 → 3, else 2; post the 72-byte object
//!   (`sub_824AF8C8`, 17 words) into `+36`, and for family 0 a layer-1 companion into `+40`.
//!   While held and grinding stops: release both, `+56 = −1`.
//! - Update `sub_824C39E0` (slot 10), only while `+341` and `+36` is held: a family change to 0
//!   posts the companion (no surface-14 skip here), a change away from 0 releases it; then both
//!   held packets are rewritten and redelivered (the main packet's w10 = the current layer; the
//!   companion's w10 = 1 while the family is 0).
//!
//! Not ported (not message traffic): `sub_824C3FC8` / `sub_824C4138` (per-layer emitter objects
//! at `+60`/`+68`, gated on audio-state byte `+810`), `sub_824C45D0` (grind events through
//! `sub_824AA858`, id 24674 …) and `sub_824C42A8` (the update's tail).
//!
//! Tuning: holder `*(0x830CFDA4)+40` = grind material class `049861E8F9A8D16B`: `default` for the
//! speed range, and per surface the collection named by the image table `0x82249F90`
//! (`sub_824C2E48` V lookup, `sub_824C2D00` F lookup); holder `+64` = AudioSurfaceMap; holder
//! `+140` = eEQChain.
//!
//! **Surfaces.** [`SurfacePolicy::Retail`] is the mapping above. The user-accepted interim
//! [`SurfacePolicy::Default`] (2, as for the board) keeps a map result of 14 (no post) and
//! replaces every other surface, including the material-143 result 4, until the engine's grind
//! materials (`Grinds+216` → state `+692`) are trusted.

use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::board::SurfacePolicy;
use super::contacts::{SurfaceMap, clamp_word, vault_word};
use super::words::{HALF, KMH_PER_MS, TEN_THOUSAND, fctiwz};
use super::{Component, Controls, Tick, post, redeliver, release};

const MATERIAL_CLASS: &str = "Hash_049861E8F9A8D16B";
const DEFAULT_KEY: &str = "Hash_D7EDBD362D7D2152";
const EQ_CLASS: &str = "Hash_42AFE160E647167C";

/// Image table `0x82249F90`: the grind material collection key for surfaces 0..13 (surfaces
/// ≥ 14 read entry 4).
const SURFACE_KEYS: [u64; 14] = [
    0x72766EB54205429A,
    0x849358CD45882044,
    0x8DB0436F6B3405D5,
    0xA36240FD856EEBA2,
    0x8212F939B27CB441,
    0x8710D7DAA28B90E7,
    0x80AD71A0341B4AEB,
    0x29FC132BB19BE7C9,
    0xFD4F9F04AE61311B,
    0x7CBD549861AE837E,
    0xBDDCF1000271E0F7,
    0x0C4C7B1701C3EA1B,
    0x3696D249613957DD,
    0xEDB51E711C25AA58,
];
/// `sub_824C2E48` (V, per layer 0..3) and `sub_824C2D00` (F) field keys.
const V_FIELDS: [&str; 4] = [
    "Hash_0ECECDAC28B2B979",
    "Hash_58070BF511809903",
    "Hash_72BA0780A8FA25D6",
    "Hash_721A50C80028AD69",
];
const F_FIELDS: [&str; 4] = [
    "Hash_69969AF1BE6BB367",
    "Hash_0555484D6D4A3128",
    "Hash_C21983A2160ED3AC",
    "Hash_69A5FC53091ED1F8",
];

/// Packet length: the constructor's 72-byte object minus the 4-byte header.
pub(crate) const GRIND_WORDS: usize = 17;
const OBJECT: &str = "Class_grind";
/// `0x821747FC` (32767.0), `0x8231A844` (1.0: layers above 3).
const FULL_SCALE: f32 = f32::from_bits(0x46FF_FE00);
const ONE: f32 = f32::from_bits(0x3F80_0000);
const NO_MATERIAL: u32 = 143;
const SKIP_SURFACE: u32 = 14;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GrindTuning {
    /// `default` `4890392C91829954` (45 km/h).
    pub top_kmh: f32,
    /// V and F per surface 0..13 and layer 0..3 (vault, with inheritance from `default`).
    pub v: [[f32; 4]; 14],
    pub f: [[f32; 4]; 14],
    /// eEQChain `D489344CEDEE5036` (w16).
    pub eq: i32,
    pub surfaces: SurfaceMap,
}

impl GrindTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let mut v = [[0.0; 4]; 14];
        let mut f = [[0.0; 4]; 14];
        for (surface, key) in SURFACE_KEYS.iter().enumerate() {
            let key = format!("Hash_{key:016X}");
            for layer in 0..4 {
                v[surface][layer] = vault.float(MATERIAL_CLASS, &key, V_FIELDS[layer])?;
                f[surface][layer] = vault.float(MATERIAL_CLASS, &key, F_FIELDS[layer])?;
            }
        }
        Ok(Self {
            top_kmh: vault.float(MATERIAL_CLASS, DEFAULT_KEY, "Hash_4890392C91829954")?,
            v,
            f,
            eq: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_D489344CEDEE5036")? as i32,
            surfaces: SurfaceMap::load(vault)?,
        })
    }

    /// `sub_824C2E48` (V) / `sub_824C2D00` (F): surfaces ≥ 14 read surface 4; layers > 3 give 1.0.
    fn value(table: &[[f32; 4]; 14], surface: u32, layer: u32) -> f32 {
        let surface = if surface >= 14 { 4 } else { surface as usize };
        if layer > 3 {
            ONE
        } else {
            table[surface][layer as usize]
        }
    }

    /// `sub_824C2E48`: `fctiwz(V × 32767)`.
    pub(crate) fn v_word(&self, surface: u32, layer: u32) -> i32 {
        fctiwz(Self::value(&self.v, surface, layer) * FULL_SCALE)
    }

    pub(crate) fn f_value(&self, surface: u32, layer: u32) -> f32 {
        Self::value(&self.f, surface, layer)
    }
}

/// Owner bytes and settings the constructor reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GrindOwner {
    /// `[this+16]+72` (local player) and `[this+16]+64` (player index).
    pub local_72: bool,
    pub index_64: i32,
    /// SFX-pack game setting `[0x830CFDC4]+564` (Bool `C317F2045035CD24` of the pack record via
    /// `sub_824843B0`); off in the retail capture (w12 = 0), not modelled.
    pub sfx_pack_564: bool,
}

impl GrindOwner {
    pub(crate) const LOCAL: Self = Self {
        local_72: true,
        index_64: 0,
        sfx_pack_564: false,
    };
}

/// `sub_824C28B0` speed: `min(fctiwz(clamp01((v − 0.5) / T × 3.6) × 10000), 9000)`, in retail's
/// operation order (`fsubs`, `fdivs`, `fmuls`; `words::grind_speed` multiplies first).
pub(crate) fn grind_speed_word(ground_speed: f32, top_kmh: f32) -> i32 {
    let x = (ground_speed - HALF) / top_kmh * KMH_PER_MS;
    let low = if -x >= 0.0 { 0.0 } else { x };
    let unit = if ONE - low >= 0.0 { low } else { ONE };
    fctiwz(unit * TEN_THOUSAND).min(9_000)
}

/// Grind family → layer (`li r24,2`; 1/2/4 → 0; 5 → 3).
pub(crate) fn grind_layer(family: i32) -> u32 {
    match family {
        1 | 2 | 4 => 0,
        5 => 3,
        _ => 2,
    }
}

/// The grind surface of state `+692`: 4 for no material, else AudioSurfaceMap `+16`.
pub(crate) fn grind_surface(tuning: &GrindTuning, material_692: u32, policy: SurfacePolicy) -> u32 {
    let retail = if material_692 == NO_MATERIAL {
        4
    } else {
        tuning.surfaces.lookup(material_692 as i32, 16)
    };
    match policy {
        SurfacePolicy::Retail => retail,
        SurfacePolicy::Default(_) if retail == SKIP_SURFACE => retail,
        SurfacePolicy::Default(surface) => surface,
    }
}

/// `sub_824AF8C8`: a posted packet.
pub(crate) fn grind_constructor(
    tuning: &GrindTuning,
    speed_140: i32,
    surface: u32,
    layer: u32,
    owner: GrindOwner,
    level6: u32,
) -> [u32; GRIND_WORDS] {
    let mut words = [0; GRIND_WORDS];
    words[1] = 32_767;
    words[5] = 25_000;
    words[7] = clamp_word(speed_140, 0, 10_000);
    words[8] = 1_024;
    words[9] = clamp_word(surface as i32, 0, 14);
    words[10] = clamp_word(layer as i32, 0, 3);
    words[11] = clamp_word(tuning.v_word(surface, layer), 0, 32_767);
    words[12] = clamp_word(i32::from(owner.sfx_pack_564), 0, 1);
    words[13] = clamp_word(i32::from(owner.local_72 && owner.index_64 == 0), 0, 1);
    words[14] = clamp_word(i32::from(owner.local_72), 0, 1);
    words[15] = clamp_word(if owner.local_72 { level6 as i32 } else { 0 }, 0, 32_767);
    words[16] = clamp_word(tuning.eq, 0, 32_767);
    words
}

/// `sub_824C39E0`'s rewrite of held packet `index` (0 = `+36`, 1 = `+40`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn grind_update(
    words: &mut [u32; GRIND_WORDS],
    index: usize,
    tuning: &GrindTuning,
    audio: &AudioState,
    controls: &dyn Controls,
    owner: GrindOwner,
    speed_140: i32,
    policy: SurfacePolicy,
) {
    let family = audio.grind_family_192 as i32;
    let layer = grind_layer(family);
    let surface = grind_surface(tuning, audio.grind_material_692, policy);
    // extsw, fcfid, frsp, fmuls, fctiwz
    let gain = fctiwz(controls.level(1) as i32 as f32 * tuning.f_value(surface, layer));
    words[7] = clamp_word(speed_140, 0, 10_000);
    words[0] = 32_767;
    words[1] = clamp_word(gain, 0, 32_767);
    words[2] = clamp_word(controls.level(5) as i32, 0, 32_767);
    words[15] = clamp_word(
        if owner.local_72 {
            controls.level(6) as i32
        } else {
            0
        },
        0,
        32_767,
    );
    words[5] = clamp_word(controls.level(3) as i32, 0, 25_000);
    words[6] = clamp_word(controls.level(4) as i32, 0, 25_000);
    words[3] = clamp_word(controls.raw(0) as i32, 0, 0x1_0000);
    words[4] = clamp_word(controls.pitch(2), 0, 8_192);
    if index == 0 {
        words[10] = clamp_word(layer as i32, 0, 3);
    } else if family == 0 {
        words[10] = 1;
    }
}

/// What one call did, for the runtime glue and the capture replay.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GrindEvents {
    pub post: [Option<[u32; GRIND_WORDS]>; 2],
    pub release: [bool; 2],
}

pub(crate) struct Grind {
    tuning: GrindTuning,
    owner: GrindOwner,
    pub surfaces: SurfacePolicy,
    /// `+36`, `+40`.
    held: [Option<(u32, [u32; GRIND_WORDS])>; 2],
    family_56: i32,
    speed_140: i32,
    /// Controller inputs set this frame (id, value), in call order; see [`Grind::take_controller_inputs`].
    inputs: Vec<(u32, u32)>,
}

impl Grind {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self::with_tuning(
            GrindTuning::load(vault)?,
            SurfacePolicy::Default(2),
        ))
    }

    fn with_tuning(tuning: GrindTuning, surfaces: SurfacePolicy) -> Self {
        Self {
            tuning,
            owner: GrindOwner::LOCAL,
            surfaces,
            held: [None, None],
            family_56: -1,
            speed_140: 0,
            inputs: Vec::new(),
        }
    }

    /// The Rail controller inputs (vfunc 8 on `[this+12]`) the process set, in call order: id 0
    /// grinding, id 1 grind-end pulse. The retail MixMap evaluates them later in the frame.
    pub(crate) fn take_controller_inputs(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.inputs)
    }

    /// `sub_824C28B0` (without the emitter and event helpers).
    pub(crate) fn step_process(
        &mut self,
        audio: &AudioState,
        controls: &dyn Controls,
    ) -> GrindEvents {
        let mut events = GrindEvents::default();
        if audio.grinding_341 {
            self.inputs.push((0, 32_767));
            self.speed_140 = grind_speed_word(audio.ground_speed_208, self.tuning.top_kmh);
        } else {
            self.inputs.push((0, 0));
        }
        let end = audio.grinding_prev_342 && !audio.grinding_341;
        self.inputs.push((1, if end { 32_767 } else { 0 }));
        if self.held[0].is_some() {
            if !audio.grinding_341 {
                for index in 0..2 {
                    events.release[index] = self.held[index].is_some();
                }
                self.family_56 = -1;
            }
            return events;
        }
        if !audio.grinding_341 {
            return events;
        }
        let surface = grind_surface(&self.tuning, audio.grind_material_692, self.surfaces);
        if audio.grind_material_692 != NO_MATERIAL && surface == SKIP_SURFACE {
            return events;
        }
        let family = audio.grind_family_192 as i32;
        self.family_56 = family;
        let layer = grind_layer(family);
        let level6 = if self.owner.local_72 {
            controls.level(6)
        } else {
            0
        };
        events.post[0] = Some(grind_constructor(
            &self.tuning,
            self.speed_140,
            surface,
            layer,
            self.owner,
            level6,
        ));
        if family == 0 {
            events.post[1] = Some(grind_constructor(
                &self.tuning,
                self.speed_140,
                surface,
                1,
                self.owner,
                level6,
            ));
        }
        events
    }

    /// `sub_824C39E0`'s companion changes (the rewrite is [`Grind::rewrite`]).
    pub(crate) fn step_update(
        &mut self,
        audio: &AudioState,
        controls: &dyn Controls,
    ) -> GrindEvents {
        let mut events = GrindEvents::default();
        if !audio.grinding_341 || self.held[0].is_none() {
            return events;
        }
        let family = audio.grind_family_192 as i32;
        if family != self.family_56 {
            if family == 0 {
                if self.held[1].is_none() {
                    let surface =
                        grind_surface(&self.tuning, audio.grind_material_692, self.surfaces);
                    let level6 = if self.owner.local_72 {
                        controls.level(6)
                    } else {
                        0
                    };
                    events.post[1] = Some(grind_constructor(
                        &self.tuning,
                        self.speed_140,
                        surface,
                        1,
                        self.owner,
                        level6,
                    ));
                }
            } else if self.family_56 == 0 {
                events.release[1] = self.held[1].is_some();
            }
            self.family_56 = family;
        }
        events
    }

    /// `sub_824C39E0`'s loop: rewrite every held packet (after the companion changes applied).
    pub(crate) fn rewrite(&mut self, audio: &AudioState, controls: &dyn Controls) {
        if !audio.grinding_341 {
            return;
        }
        for (index, slot) in self.held.iter_mut().enumerate() {
            if let Some((_, words)) = slot.as_mut() {
                grind_update(
                    words,
                    index,
                    &self.tuning,
                    audio,
                    controls,
                    self.owner,
                    self.speed_140,
                    self.surfaces,
                );
            }
        }
    }

    fn apply(
        &mut self,
        runtime: &mut skate_audio_core::authored::AuthoredRuntime,
        events: &GrindEvents,
    ) -> Result<(), String> {
        for index in 0..2 {
            if events.release[index] {
                let mut handle = self.held[index].take().map(|(handle, _)| handle);
                release(runtime, &mut handle)?;
            }
            if let Some(words) = events.post[index] {
                self.held[index] = Some((post(runtime, OBJECT, &words)?, words));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn apply_local(&mut self, events: &GrindEvents) {
        for index in 0..2 {
            if events.release[index] {
                self.held[index] = None;
            }
            if let Some(words) = events.post[index] {
                self.held[index] = Some((0, words));
            }
        }
    }
}

impl Component for Grind {
    fn take_owner_inputs(&mut self) -> Vec<(u32, u32)> {
        self.take_controller_inputs()
    }

    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let events = self.step_process(tick.audio, tick.controls);
        self.apply(tick.runtime, &events)
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        if !tick.audio.grinding_341 || self.held[0].is_none() {
            return Ok(());
        }
        // A companion the update posts goes out with its constructor words and is then
        // rewritten and redelivered in the same pass, as retail's loop does.
        let events = self.step_update(tick.audio, tick.controls);
        self.apply(tick.runtime, &events)?;
        self.rewrite(tick.audio, tick.controls);
        for (handle, words) in self.held.iter().flatten() {
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::capture::{self, Captured, Matches};
    use super::*;

    fn surface_map() -> SurfaceMap {
        // Materials 5 → 8, 7 → 0, 94 (and 94..142) → 4, 18 → 13, 100 → 4.
        let mut entries = vec![[0u32; 18]; 95];
        entries[5][4] = 8;
        entries[18][4] = 13;
        entries[94][4] = 4;
        entries[20][4] = 14;
        SurfaceMap::from_entries(entries)
    }

    /// Vault values for the surfaces used here (default elsewhere).
    fn tuning() -> GrindTuning {
        let mut v = [[1.0; 4]; 14];
        let mut f = [[1.0; 4]; 14];
        v[4] = [
            f32::from_bits(0x3EB8_51EC),
            f32::from_bits(0x3F0F_5C29),
            f32::from_bits(0x3F28_F5C3),
            1.0,
        ];
        f[4] = [f32::from_bits(0x3F57_0A3D), 1.0, 1.0, 0.5];
        v[6] = [
            f32::from_bits(0x3F07_AE14),
            f32::from_bits(0x3F0C_CCCD),
            f32::from_bits(0x3F54_7AE1),
            f32::from_bits(0x3F33_3333),
        ];
        v[12] = [1.0, 0.5, 1.0, 1.0];
        f[12] = [
            f32::from_bits(0x3F47_AE14),
            1.0,
            1.0,
            f32::from_bits(0x3F4C_CCCD),
        ];
        GrindTuning {
            top_kmh: 45.0,
            v,
            f,
            eq: 5,
            surfaces: surface_map(),
        }
    }

    #[test]
    fn grind_speed_uses_retails_operation_order_and_caps() {
        assert_eq!(grind_speed_word(0.5, 45.0), 0);
        assert_eq!(grind_speed_word(50.0, 45.0), 9_000);
        assert_eq!(grind_speed_word(0.0, 45.0), 0);
    }

    #[test]
    fn grind_layers_and_surfaces_follow_retail() {
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 6, -1].map(grind_layer),
            [2, 0, 0, 2, 0, 3, 2, 2]
        );
        let t = tuning();
        assert_eq!(grind_surface(&t, 143, SurfacePolicy::Retail), 4);
        assert_eq!(grind_surface(&t, 5, SurfacePolicy::Retail), 8);
        assert_eq!(grind_surface(&t, 120, SurfacePolicy::Retail), 4);
        assert_eq!(grind_surface(&t, 5, SurfacePolicy::Default(2)), 2);
        assert_eq!(grind_surface(&t, 20, SurfacePolicy::Default(2)), 14);
    }

    #[test]
    fn grind_constructor_matches_retail_posts() {
        let t = tuning();
        // Frame 3894: layer 2 on surface 6 (V 0.83 → 27196), speed 0x1B11, level 6 = 0x664.
        assert_eq!(
            grind_constructor(&t, 0x1B11, 6, 2, GrindOwner::LOCAL, 0x664),
            [
                0, 32767, 0, 0, 0, 25000, 0, 0x1B11, 1024, 6, 2, 0x6A3C, 0, 1, 1, 0x664, 5
            ]
        );
        // Frame 20599 (second skater, family 0): the layer-1 companion on surface 6, V 0.55.
        assert_eq!(t.v_word(6, 1), 0x4665);
        // Frame 4594: surface 12, layer 0 → V 1.0 → 32767.
        let words = grind_constructor(&t, 0x84A, 12, 0, GrindOwner::LOCAL, 0x664);
        assert_eq!((words[9], words[10], words[11]), (12, 0, 32_767));
    }

    #[test]
    fn family_zero_posts_a_companion_and_the_main_packet_tracks_the_layer() {
        let mut grind = Grind::with_tuning(tuning(), SurfacePolicy::Retail);
        let controls = Captured::default();
        let audio = {
            let mut s = AudioState::default();
            s.grinding_341 = true;
            s.grind_family_192 = 0;
            s.grind_material_692 = 143;
            s.ground_speed_208 = 5.0;
            s
        };
        let events = grind.step_process(&audio, &controls);
        assert_eq!(events.post[0].map(|w| w[10]), Some(2));
        assert_eq!(events.post[1].map(|w| w[10]), Some(1));
        grind.apply_local(&events);
        grind.step_update(&audio, &controls);
        grind.rewrite(&audio, &controls);
        // Retail (second skater, frame 20600): main w10 2, companion w10 1.
        assert_eq!(grind.held[0].unwrap().1[10], 2);
        assert_eq!(grind.held[1].unwrap().1[10], 1);
        // Family 2: the companion goes, the main packet takes layer 0.
        let audio = {
            let mut s = audio.clone();
            s.grind_family_192 = 2;
            s
        };
        let events = grind.step_update(&audio, &controls);
        assert!(events.release[1]);
        grind.apply_local(&events);
        grind.rewrite(&audio, &controls);
        assert_eq!(grind.held[0].unwrap().1[10], 0);
        // Grinding stops: what is held is released.
        let events = grind.step_process(
            &{
                let mut s = AudioState::default();
                s.grinding_prev_342 = true;
                s
            },
            &controls,
        );
        assert_eq!(events.release, [true, false]);
        assert_eq!(
            grind.take_controller_inputs(),
            [(0, 32_767), (1, 0), (0, 0), (1, 32_767)]
        );
    }

    #[test]
    fn map_result_14_posts_nothing() {
        let mut grind = Grind::with_tuning(tuning(), SurfacePolicy::Retail);
        let audio = {
            let mut s = AudioState::default();
            s.grinding_341 = true;
            s.grind_family_192 = 2;
            s.grind_material_692 = 20;
            s
        };
        assert_eq!(
            grind.step_process(&audio, &Captured::default()),
            GrindEvents::default()
        );
    }

    /// Replays the retail capture (local player's rows: w14 = 1) with the real vault tuning.
    #[test]
    #[ignore = "needs the retail capture in .local and the owner's vault"]
    fn grind_replays_the_retail_capture() {
        let Some(root) = capture::root() else { return };
        let assets = std::env::var("SKATE_ASSETS").unwrap_or_else(|_| {
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets".into()
        });
        let vault = Collections::load(std::path::Path::new(&assets)).unwrap();
        let tuning = GrindTuning::load(&vault).unwrap();
        let states = capture::states(&root);
        let rows = capture::rows(&root, "Class_grind");
        let mut by_frame = std::collections::BTreeMap::<u32, Vec<capture::Row>>::new();
        for row in rows
            .iter()
            .filter(|r| r.kind == "RL" || r.words.get(14) == Some(&1))
        {
            by_frame.entry(row.frame).or_default().push(row.clone());
        }
        let local_nodes: std::collections::HashSet<String> = rows
            .iter()
            .filter(|r| r.kind == "UP" && r.words.get(14) == Some(&1))
            .map(|r| r.node.clone())
            .collect();
        let mut grind = Grind::with_tuning(tuning, SurfacePolicy::Retail);
        let mut updates = Matches::new("Class_grind update", GRIND_WORDS);
        let mut posts = Matches::new("Class_grind post", GRIND_WORDS);
        let (mut timing_ok, mut timing_bad) = (0, Vec::new());
        let first = *states.keys().next().unwrap();
        let last = *states.keys().last().unwrap();
        for frame in first + 1..=last {
            let empty = Vec::new();
            let here = by_frame.get(&frame).unwrap_or(&empty);
            let ups: Vec<_> = here.iter().filter(|r| r.kind == "UP").collect();
            let reads: Vec<_> = ups.iter().flat_map(|r| r.reads.clone()).collect();
            let mut ours_rl = 0;
            if let Some(state) = states.get(&(frame - 1)) {
                let audio = AudioState::from_capture(state);
                let controls = Captured::from_reads(&reads, 0x824C_39E0..0x824C_3FC8);
                let held = grind.held[0].is_some() && audio.grinding_341;
                let events = grind.step_update(&audio, &controls);
                ours_rl += events.release.iter().filter(|r| **r).count();
                grind.apply_local(&events);
                if held {
                    grind.rewrite(&audio, &controls);
                }
                for up in &ups {
                    // The companion is the held packet whose constructor V word (w11) matches.
                    let index = usize::from(
                        grind.held[1].is_some_and(|(_, w)| w[11] == up.words[11])
                            && grind.held[0].is_some_and(|(_, w)| w[11] != up.words[11]),
                    );
                    if let Some((_, words)) = grind.held[index].as_ref() {
                        updates.add(frame, &up.words[..GRIND_WORDS], words);
                    }
                }
            }
            let retail_rl = here
                .iter()
                .filter(|r| r.kind == "RL" && local_nodes.contains(&r.node))
                .count();
            let pos: Vec<_> = here.iter().filter(|r| r.kind == "PO").collect();
            let mut ours_po = 0;
            if let Some(state) = states.get(&frame) {
                let audio = AudioState::from_capture(state);
                let preads: Vec<_> = pos.iter().flat_map(|r| r.reads.clone()).collect();
                let controls = Captured::from_reads(&preads, 0x824C_28B0..0x824C_2D00);
                let events = grind.step_process(&audio, &controls);
                ours_rl += events.release.iter().filter(|r| **r).count();
                for (i, words) in events.post.iter().enumerate() {
                    if let Some(words) = words {
                        ours_po += 1;
                        if let Some(po) = pos.get(i) {
                            posts.add(frame, &po.words[..GRIND_WORDS], words);
                        }
                    }
                }
                grind.apply_local(&events);
            }
            if ours_po == pos.len() && ours_rl == retail_rl {
                timing_ok += 1;
            } else if timing_bad.len() < 12 {
                timing_bad.push((frame, ours_po, pos.len(), ours_rl, retail_rl));
            }
        }
        posts.print();
        updates.print();
        println!(
            "frames with matching post/release counts {timing_ok}; mismatches (frame, ours po, retail po, ours rl, retail rl) {timing_bad:?}"
        );
    }
}
