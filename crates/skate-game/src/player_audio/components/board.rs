//! `SFXObj_SkateBoard` (owner vtable `0x822FC770`): the board's continuous sound. The grain
//! rolling bed and its per-player bus chains, the per-surface and held `Class_rolling` layers,
//! the rattle, skid, squeaks and loose-board scrape, the owner's push envelopes and its MixMap
//! controller inputs.
//!
//! Transliterated from the lifted retail code (`skate3_recomp.11.cpp` unless noted):
//!
//! | retail | here |
//! |---|---|
//! | `sub_824C5058` constructor, `sub_824C59C8` start (players, held layers) | [`Board::new`] |
//! | `sub_824C6A78` process (vtable slot 9) | [`Component::process`] |
//! | `sub_824CA738` slope inputs 2/3 → `+1508`/`+1512` | [`Board::slope_inputs`] |
//! | `sub_824C5CA8` surface routing, `sub_824C82A8` / `sub_82494CD8` surface of a truck | [`Board::route_surfaces`], [`Board::surface_of`] |
//! | `sub_824C6198` push input 4, push envelopes, rattle post, `+1164`/`+1168` slews | [`Board::push_and_rattle`] |
//! | `sub_824C9058` chain pushes | [`Board::push_chains`] |
//! | `sub_824C7438` / `sub_824C72F0` skid trigger, input 1, `sub_824AF678` packet | [`Board::skid_trigger`], [`skid_predicate`], [`skid_post`] |
//! | `sub_824C7738` squeaks trigger, `sub_824AFF48` packet | [`Board::squeak_trigger`], [`squeak_post`] |
//! | `sub_824C9F68` spidercrack rolling layer 5 | [`Board::crack_layer`] |
//! | `sub_824CA448` / `sub_824CA318` seam-pattern gain envelope (`+1340`) | [`Board::seam_envelope`] |
//! | `sub_824CAEC0` graph-3 send level by speed | [`Board::distortion_send`] |
//! | `sub_824CB180` / `sub_824CB078` graph-1/3 gain wobble | [`Board::gain_wobble`] |
//! | `sub_824CB3C8` loose-board trigger, `sub_824B0670` packet | [`Board::board_slide_trigger`] |
//! | `sub_824CBAC0` input 5 (rate of change of vfunc52(0)) | [`heading_rate`] |
//! | `sub_824C6BD8` update (vtable slot 10): grain records, per-surface rolling | [`Component::update`] |
//! | `sub_824C7A20` skid, `sub_824C7DD0` squeaks, `sub_824C80C0` rattle, `sub_824C9948` held layers, `sub_824CA038` layer 5, `sub_824CB4C0` board slide | the `*_update` functions |
//!
//! **Owner MixMap inputs** (controller vfunc 8, key `0x40010000`):
//!
//! - id 0: surface-change pulse;
//! - id 1: skid active;
//! - ids 2/3: downhill/uphill slope;
//! - id 4: push foot planted;
//! - id 5: heading rate;
//! - id 6: on metal.
//!
//! They are buffered in call order and taken with [`Component::take_owner_inputs`]. The retail
//! MixMap evaluates them later in the frame, so applying the buffer before the MixMap tick is
//! equivalent.
//!
//! **Not ported** (reported):
//!
//! - `sub_824CB828`: non-local skaters' pass-by;
//! - `sub_824C6BD8`'s store of vfunc52(0) × 360/65536 into the global manager's `+124`/`+160`.
//!
//! **Surfaces.** Retail picks each truck's surface from the audio state's wheel material through
//! the vault's `Sk8::AudioSurfaceMap` (`0x4CA607558B1CF440`, entry `+4`). [`SurfacePolicy::Default`]
//! (the current default, accepted by the user) replaces every in-contact result with surface 2
//! (concrete_rough grains) until the engine's materials are trusted; [`SurfacePolicy::Retail`] is
//! the full mapping.

use std::collections::HashMap;

use skate_audio_core::fp::{fmadd_single, nmsub_single};
use skate_audio_core::grain::board::{
    self as grain_board, BoardInputs, ChainInputs, GrainRecord, SlewInputs, SurfaceTuning,
};
use skate_audio_core::grain::chain::ChainConfig;
use skate_audio_core::grain::envelope::{Envelope, program_push};
use skate_data::audio::grains::{GrainMember, GrainVault};
use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::contacts::SurfaceMap;
use super::words::{
    KMH_PER_MS, SPEED_SCALE, TEN_THOUSAND, THOUSAND, board_slide_speed, fctiwz, rattle_speed, word,
};
use super::{Component, Controls, Tick, post, redeliver, release};

pub(crate) const ROLLING: &str = "Class_rolling";
pub(crate) const RATTLE: &str = "Rolling_Rattle_Class";
pub(crate) const SKID: &str = "Class_wheels_skid";
pub(crate) const SQUEAKS: &str = "Class_Squeaks";
pub(crate) const BOARD_SLIDE: &str = "c_board_slide";

/// hash64("default"), `sub_824C8370`'s key for surfaces without grains.
const DEFAULT_KEY: u64 = 0xD7ED_BD36_2D7D_2152;
/// `0x82256FE4`: the rolling max speed when no layer is given (`sub_824C6B30(−1)`).
const DEFAULT_ROLLING_KMH: f32 = 45.0;
/// `0x821747FC`.
const LEVEL: f32 = 32_767.0;
/// `lis -32241 ; lfs -10884`: 100.0; `lis -32243 ; lfs 29160`: 0.01; `0x822F8B3C`: −0.01.
const HUNDRED: f32 = 100.0;
const HUNDREDTH: f32 = f32::from_bits(0x3C23_D70A);
const MINUS_HUNDREDTH: f32 = f32::from_bits(0xBC23_D70A);
/// `0x822F9414`: 114.5916, radians to the squeak angle word.
const SQUEAK_ANGLE: f32 = f32::from_bits(0x42E5_2EE0);
/// `0x822F9018`: −90.0, the skid slip scale.
const MINUS_NINETY: f32 = -90.0;
/// `0x82256FD8`: 90.0.
const NINETY: f32 = 90.0;
/// `0x82165A00`: 0.05, the `+1168` step.
const BRAKE_STEP: f32 = f32::from_bits(0x3D4C_CCCD);
/// The title generator's state in the dumped image (`0x82FD7D74`).
const RNG_SEED: [u32; 6] = [
    0xF22D_0E56,
    0x8831_26E9,
    0xC624_DD2F,
    0x0702_C49C,
    0x9E35_3F7D,
    0x6FDF_3B64,
];

/// How [`Board::surface_of`] turns a truck's contact into a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfacePolicy {
    /// `sub_824C82A8` through the vault's AudioSurfaceMap.
    Retail,
    /// Retail's no-contact result (14) is kept; every other result becomes this surface. The
    /// accepted default is 2 (concrete_rough). Kept as the `SKATE_AUDIO_SURFACES=concrete`
    /// fallback so the two can be A/B'd in one session.
    Default(u32),
}

/// Owner facts retail reads from the character and the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoardConfig {
    /// `[[owner+16]+72]`: the local player (graph 3, both trucks, skid w14/w15/w16).
    pub local: bool,
    /// `[[owner+16]+64] == 0` (skid w14; 0 for the first player).
    pub first_player: bool,
    /// Skid w13: `[[0x830CFDC4]+564]` and its collection flag (0 in every capture post).
    pub global_flag: bool,
    pub surfaces: SurfacePolicy,
}

impl Default for BoardConfig {
    fn default() -> Self {
        Self {
            local: true,
            first_player: true,
            global_flag: false,
            surfaces: SurfacePolicy::Retail,
        }
    }
}

impl BoardConfig {
    /// The live configuration, with the surface policy taken from the environment.
    ///
    /// Retail routes each truck's rolling grain, seam pattern, skid and grind timbre through the
    /// vault's `Sk8::AudioSurfaceMap` from the per-wheel material. This port pinned every
    /// in-contact surface to 2 (concrete_rough) while the engine's materials were not trusted;
    /// they now are. `WorldTriangle.tag` carries the retail RWCM per-triangle `surface: u16`
    /// verbatim (`skate-data/src/retail_collision.rs:186-190` →
    /// `skate-game/src/skate_world.rs:172`), and `SKATE_AUDIO_OBSERVE=1` traces on University show
    /// a dozen distinct ids — 1, 3, 4, 5, 8, 16, 17, 41, 42, 53, 66, 79 — with 41 and 3 dominant.
    ///
    /// `SKATE_AUDIO_SURFACES=concrete` restores the pinned behaviour for an A/B.
    ///
    /// **Portable `.skate` maps are a known exception** and still wrong: `skate_world.rs:324`
    /// packs the real id into `packed_surfaces` but the triangle's `tag` at `:336` takes
    /// `source.surface`, which the Blender exporter writes as a per-object counter. Riding one of
    /// those gives surfaces derived from an object index. Recorded in `docs/engine-defects.md` 5.
    pub(crate) fn from_env() -> Self {
        let surfaces = match std::env::var("SKATE_AUDIO_SURFACES").as_deref() {
            Ok("concrete") => SurfacePolicy::Default(2),
            _ => SurfacePolicy::Retail,
        };
        Self {
            surfaces,
            ..Self::default()
        }
    }
}

/// A seam pattern's gain wobble (`0x7242F32831ED3332`, `sub_824CA448`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Wobble {
    gain_low: f32,
    gain_high: f32,
    ms_low: i32,
    ms_high: i32,
}

/// Every vault value the owner reads. Loaded once; a missing field is an error.
#[derive(Clone, Debug)]
pub(crate) struct BoardVault {
    /// `Sk8::AudioSurfaceMap` (holder `+64`): `+4` rolling surface, `+12` skid surface.
    surfaces: SurfaceMap,
    /// `0x880C82E8EF647EC4`, rolling max speed per layer (holder `+56`).
    rolling_kmh: Vec<f32>,
    /// Grain-class collections by `sub_824C8370` key, plus `default`.
    tunings: HashMap<u64, SurfaceTuning>,
    /// Per key: rattle divisor `0x12275AA8AC4A63FB`, slope divisors `0x57A78D3BE8D47BB3` /
    /// `0x8DD4C3FC8DAF4059`.
    rattle_kmh: HashMap<u64, f32>,
    slope: HashMap<u64, (f32, f32)>,
    /// Holder `+4` (grain `default`): squeak threshold `0xA129B33B4A2C7961`, divisor
    /// `0xAC87D592E7601134`.
    squeak_threshold: i32,
    squeak_divisor: f32,
    /// Holder `+84`: skid w11 `0xFB10048CCDD6ADFA`, w12 `0x8CE42E5A9388A4C8`.
    skid_gain: i32,
    skid_level: i32,
    /// Holder `+140` eEQChain: rattle `0xC04832978CDED925`, skid `0xF52450E504250254`, squeak
    /// `0x22D0D4A5A14FFF7D`, board slide `0xF2B44F93BD91662E`.
    rattle_tweak: i32,
    skid_tweak: i32,
    squeak_tweak: i32,
    slide_tweak: i32,
    /// Holder `+96`: `0x9635B780C7472A6E`, `0x662CEE73D2E3FE2F`, `0xAB87C3D1EDDDDCBC`.
    slide_kmh: f32,
    slide_level: i32,
    slide_level_loose: i32,
    /// Holder key `0x1C20B475CE218459`: input 5's max and slew rate (`sub_824CBAC0`).
    heading_max: f32,
    heading_step: f32,
    /// Holder `+132` (`sub_824CA938`): `+1520..+1552`, `+1564`, `+1568`, wobble records.
    chain: ChainConfig,
    send_ramp: [f32; 4],
    level_ramp: [f32; 3],
    wobble_ramp: [f32; 2],
    wobbles: [Wobble; 2],
    /// Seam-pattern wobbles by pattern 1..=15 (index 0 = `default`).
    seam: [Wobble; 16],
}

fn hash_name(key: u64) -> String {
    if key == DEFAULT_KEY {
        "default".to_owned()
    } else {
        format!("Hash_{key:016X}")
    }
}

impl BoardVault {
    pub(crate) fn load(assets: &std::path::Path) -> Result<Self, String> {
        let collections = Collections::load(assets)?;
        let grains = GrainVault::load(assets).map_err(|e| e.to_string())?;
        Self::from(&collections, &grains)
    }

    pub(crate) fn from(c: &Collections, grains: &GrainVault) -> Result<Self, String> {
        const HOLDER: &str = "Hash_C1831BDB6CB1B1EA";
        const GRAIN: &str = "Hash_7AB23C11B6ADA2DE";
        const EQ: &str = "Hash_42AFE160E647167C";
        const OWNER: &str = "Hash_6E878344774A7999";
        let surfaces = SurfaceMap::load(c)?;
        let rolling_kmh =
            c.float_items(HOLDER, "Hash_7B0ED922C779B74C", "Hash_880C82E8EF647EC4")?;
        let int = |class: &str, key: &str, name: &str| -> Result<i32, String> {
            Ok(c.words::<1>(class, key, name)?[0] as i32)
        };
        let mut keys = vec![DEFAULT_KEY];
        for surface in 1..=9 {
            for soft in [false, true] {
                if let Some(choice) = grain_board::grain_for_surface(surface, soft) {
                    keys.push(choice.key);
                }
            }
        }
        let mut tunings = HashMap::new();
        let mut rattle_kmh = HashMap::new();
        let mut slope = HashMap::new();
        for key in keys {
            tunings.insert(key, grains.tuning(key).map_err(|e| e.to_string())?);
            let name = hash_name(key);
            rattle_kmh.insert(key, c.float(GRAIN, &name, "Hash_12275AA8AC4A63FB")?);
            slope.insert(
                key,
                (
                    c.float(GRAIN, &name, "Hash_57A78D3BE8D47BB3")?,
                    c.float(GRAIN, &name, "Hash_8DD4C3FC8DAF4059")?,
                ),
            );
        }
        let owner = |name: &str| c.float(OWNER, "default", name);
        let seam_names = [
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
        const SEAM: &str = "Hash_7242F32831ED3332";
        let seam_of = |key: &str| -> Result<Wobble, String> {
            Ok(Wobble {
                gain_low: c.float(SEAM, key, "Hash_FA3A57801765A2F8")?,
                gain_high: c.float(SEAM, key, "Hash_32A9692F1B826274")?,
                ms_low: c.words::<1>(SEAM, key, "Hash_F713CB547B1DF920")?[0] as i32,
                ms_high: c.words::<1>(SEAM, key, "Hash_0608B3129FF81F12")?[0] as i32,
            })
        };
        let mut seam = [seam_of("default")?; 16];
        for (i, name) in seam_names.iter().enumerate() {
            let key = format!("Hash_{:016X}", skate_data::attrib_hash::hash(name));
            seam[i + 1] = seam_of(&key)?;
        }
        Ok(Self {
            surfaces,
            rolling_kmh,
            tunings,
            rattle_kmh,
            slope,
            squeak_threshold: int(GRAIN, "default", "Hash_A129B33B4A2C7961")?,
            squeak_divisor: c.float(GRAIN, "default", "Hash_AC87D592E7601134")?,
            skid_gain: int(HOLDER, "Hash_BA9837A6CF4C26ED", "Hash_FB10048CCDD6ADFA")?,
            skid_level: int(HOLDER, "Hash_BA9837A6CF4C26ED", "Hash_8CE42E5A9388A4C8")?,
            rattle_tweak: int(EQ, "default", "Hash_C04832978CDED925")?,
            skid_tweak: int(EQ, "default", "Hash_F52450E504250254")?,
            squeak_tweak: int(EQ, "default", "Hash_22D0D4A5A14FFF7D")?,
            slide_tweak: int(EQ, "default", "Hash_F2B44F93BD91662E")?,
            slide_kmh: c.float(HOLDER, "Hash_621090620F4F936A", "Hash_9635B780C7472A6E")?,
            slide_level: int(HOLDER, "Hash_621090620F4F936A", "Hash_662CEE73D2E3FE2F")?,
            slide_level_loose: int(HOLDER, "Hash_621090620F4F936A", "Hash_AB87C3D1EDDDDCBC")?,
            heading_max: c.float(HOLDER, "Hash_1C20B475CE218459", "Hash_780F5C816E00BDFC")?,
            heading_step: c.float(HOLDER, "Hash_1C20B475CE218459", "Hash_68756CFEB1FF3428")?,
            chain: ChainConfig {
                local: true,
                create: 0,
                eq_chain: int(EQ, "default", "Hash_A5D3ADA63608617F")? as u32,
                clip: owner("Hash_E64C04ED542DABC8")?,
                shelf_corner: owner("Hash_55BEB30353F244A9")?,
                shelf_gain: owner("Hash_45516395725ED16B")?,
                local_send: 0.0,
            },
            send_ramp: [
                owner("Hash_0D665393E2EDC605")?,
                owner("Hash_28E708782445747F")?,
                owner("Hash_D900C07BE7C5450F")?,
                0.0,
            ],
            level_ramp: [
                owner("Hash_88AA96B08FD16914")?,
                owner("Hash_3FFB5107C82BA3E0")?,
                owner("Hash_D3E8894CA25A4F71")?,
            ],
            wobble_ramp: [
                owner("Hash_281A501B22B6CCDF")?,
                owner("Hash_54CDE019E31FC04E")?,
            ],
            wobbles: [
                Wobble {
                    ms_low: int(OWNER, "default", "Hash_36AE41817640FE04")?,
                    ms_high: int(OWNER, "default", "Hash_71EE27313BD30F21")?,
                    gain_low: owner("Hash_437D128B53669C34")?,
                    gain_high: owner("Hash_02885338DD5D7DCA")?,
                },
                Wobble {
                    ms_low: int(OWNER, "default", "Hash_2055BBF39C152FA9")?,
                    ms_high: int(OWNER, "default", "Hash_F5240AFADA3B3FFC")?,
                    gain_low: owner("Hash_F916E153393C5F24")?,
                    gain_high: owner("Hash_0A36F90732016D85")?,
                },
            ],
            seam,
        })
    }

    /// Every key the owner can hold is loaded (the grain keys, wood_ramp_soft and `default`).
    fn rattle_kmh(&self, key: u64) -> f32 {
        self.rattle_kmh
            .get(&key)
            .copied()
            .unwrap_or_else(|| self.rattle_kmh[&DEFAULT_KEY])
    }

    fn slope(&self, key: u64) -> (f32, f32) {
        self.slope
            .get(&key)
            .copied()
            .unwrap_or_else(|| self.slope[&DEFAULT_KEY])
    }

    fn tuning(&self, key: u64) -> &SurfaceTuning {
        self.tunings
            .get(&key)
            .unwrap_or_else(|| &self.tunings[&DEFAULT_KEY])
    }

    /// `sub_824C97B8(layer)`; −1 is the image's 45 km/h.
    fn rolling_kmh(&self, layer: i32) -> f32 {
        if layer < 0 {
            return DEFAULT_ROLLING_KMH;
        }
        // `sub_824C6B30(-1)`'s own fallback, not 0.0: a zero divisor makes `speed_word`'s
        // `v / kmh` infinite and pegs the speed word at its 10000 clamp for every speed. The
        // owner's vault array has 16 entries so every selector is covered, but the fallback has
        // to be the retail one for a shorter array not to fail silently.
        self.rolling_kmh
            .get(layer as usize)
            .copied()
            .unwrap_or(DEFAULT_ROLLING_KMH)
    }
}

/// The title generator (`sub_82A8AF10`), a private instance seeded with the dumped image's state.
/// Retail shares one generator with about a hundred call sites, so its sequence cannot be
/// reproduced; the recurrence is the retail one.
#[derive(Clone, Debug)]
pub(crate) struct TitleRng([u32; 6]);

impl TitleRng {
    fn carry(sum: u32, old: u32, carry_in: u32) -> u32 {
        u32::from(sum < old || (sum == old && carry_in != 0))
    }

    pub(crate) fn next(&mut self) -> u32 {
        let s = &mut self.0;
        let s5 = s[5];
        let r10 = s[4].wrapping_add(s5);
        let c1 = u32::from(r10 < s5);
        let r30 = s[3].wrapping_add(r10).wrapping_add(c1);
        let c2 = Self::carry(r30, r10, c1);
        let r31 = s[2].wrapping_add(r30).wrapping_add(c2);
        let c3 = Self::carry(r31, r30, c2);
        let r5 = s[1].wrapping_add(r31).wrapping_add(c3);
        let c4 = Self::carry(r5, r31, c3);
        let r3 = s[0].wrapping_add(r5).wrapping_add(c4);
        *s = [r3, r5, r31, r30, r10, s5.wrapping_add(1)];
        if s[5] == 0 {
            for k in [4usize, 3, 2, 1, 0] {
                s[k] = s[k].wrapping_add(1);
                if s[k] != 0 {
                    break;
                }
            }
        }
        s[0]
    }
}

// ------------------------------------------------------------------ packet words (pure)

fn clamp(value: i32, high: i32) -> u32 {
    word(value, 0, high)
}

/// `fctiwz(clamp01(x) × scale)` with the updaters' two `fsel`s.
fn unit_word(ratio: f32, scale: f32) -> i32 {
    let clamped = if !(-ratio >= 0.0) { ratio } else { 0.0 };
    let clamped = if 1.0 - clamped >= 0.0 { clamped } else { 1.0 };
    fctiwz(clamped * scale)
}

/// `sub_824C6B30` (and `sub_824C9830`'s form): `fctiwz(clamp01(v / kmh × 3.6) × 10000)`.
pub(crate) fn speed_word(speed: f32, kmh: f32) -> i32 {
    unit_word(speed / kmh * KMH_PER_MS, TEN_THOUSAND)
}

/// `sub_824C4C18`: a `Class_rolling` post.
pub(crate) fn rolling_post(speed: i32, selector: i32, surface: i32) -> [u32; 12] {
    [
        0,
        0,
        4096,
        clamp(speed, 10_000),
        clamp(selector, 15),
        0,
        clamp(surface, 13),
        0,
        0,
        25_000,
        0,
        32_767,
    ]
}

/// `sub_824C9948` / `sub_824CA038` for one held layer; `surface` is `None` when both trucks are
/// off the ground and not grinding (the word keeps its value).
pub(crate) fn held_rolling_update(
    words: &mut [u32; 12],
    c: &dyn Controls,
    gain_id: u32,
    speed: i32,
    surface: Option<i32>,
    audio: &AudioState,
) {
    words[0] = 32_767;
    words[11] = clamp(c.level(gain_id) as i32, 32_767);
    words[1] = clamp(c.raw(0) as i32, 65_536);
    words[2] = clamp(c.pitch(8), 8192);
    words[3] = clamp(speed, 10_000);
    words[5] = 0;
    if let Some(surface) = surface {
        words[6] = clamp(surface, 13);
    }
    words[7] = u32::from(audio.balance_340 || audio.manual_brake_339);
    words[8] = clamp(c.level(19) as i32, 32_767);
    words[9] = clamp(c.level(17) as i32, 25_000);
    words[10] = clamp(c.level(18) as i32, 25_000);
}

/// `sub_824C6BD8`'s per-surface `Class_rolling` rewrite.
pub(crate) fn surface_rolling_update(
    words: &mut [u32; 12],
    c: &dyn Controls,
    speed: i32,
    selector: i32,
    audio: &AudioState,
) {
    words[0] = 32_767;
    words[11] = clamp(c.level(1) as i32, 32_767);
    words[1] = clamp(c.raw(0) as i32, 65_536);
    words[2] = clamp(c.pitch(3), 8192);
    words[3] = clamp(speed, 10_000);
    words[5] = 0;
    words[8] = if selector == 1 {
        0
    } else {
        clamp(c.level(13) as i32, 32_767)
    };
    words[9] = clamp(c.level(11) as i32, 25_000);
    words[10] = clamp(c.level(12) as i32, 25_000);
    words[7] = u32::from(audio.balance_340 || audio.manual_brake_339);
}

/// `sub_824B0248`: a rattle post.
pub(crate) fn rattle_post(speed: i32, code: i32, tweak: i32) -> [u32; 12] {
    [
        0,
        0,
        4096,
        clamp(speed, 10_000),
        clamp(code, 8),
        0,
        1,
        0,
        25_000,
        0,
        32_767,
        clamp(tweak, 32_767),
    ]
}

/// `sub_824C80C0`.
pub(crate) fn rattle_update(words: &mut [u32; 12], c: &dyn Controls) {
    let pitch = c.pitch(3);
    words[0] = 32_767;
    words[10] = clamp(c.level(6) as i32, 32_767);
    words[1] = clamp(c.raw(0) as i32, 65_535);
    words[2] = clamp(pitch, 8192);
    words[7] = clamp(c.level(16) as i32, 32_767);
    words[8] = clamp(c.level(14) as i32, 25_000);
    words[9] = clamp(c.level(15) as i32, 25_000);
}

/// `sub_824C72F0`, the skid predicate. While the slip (`+232`) is above 0, or unordered (`fcmpu ;
/// ble`): grinding (`+341`) gives `+192 == 4`; walking with the board held (`+716`, `+308`) gives
/// false; otherwise the audio trick (`+348`) must be −1 or 35. At no slip: reverting (`+690`) or
/// the `+1516` counter above 0.
pub(crate) fn skid_predicate(audio: &AudioState, counter: i32) -> bool {
    if !(audio.slip_232 <= 0.0) {
        if audio.grinding_341 {
            return audio.grind_family_192 == 4;
        }
        if audio.walking_716 && audio.board_held_308 {
            return false;
        }
        let trick = audio.audio_trick_348 as i32;
        return trick == -1 || trick == 35;
    }
    audio.revert_690 || counter > 0
}

/// `sub_824C7A20`'s `+1516` step, run while the skid packet stays held: +5 up to 45 while
/// reverting (`+690`), otherwise −15 down to 0.
pub(crate) fn skid_counter_step(counter: i32, revert: bool) -> i32 {
    if revert {
        (counter + 5).min(45)
    } else if counter == 0 {
        0
    } else {
        let counter = if counter > 0 { counter - 15 } else { counter };
        counter.max(0)
    }
}

/// `sub_824C7738`'s decision while its gate holds (both feet in the deck box, more than one
/// wheel down) and the tilt angle reaches the threshold: `(release, post)`. `sign` is `+1296`,
/// the tilt sign of the held squeak. A held squeak is released when the sign flips, then
/// re-posted.
pub(crate) fn squeak_step(tilt: f32, sign: &mut bool, held: bool) -> (bool, bool) {
    let now = tilt >= 0.0;
    let release = *sign != now && held;
    *sign = now;
    (release, !held || release)
}

/// `sub_824C7738`'s gate and threshold: `None` when the gate fails (the squeak is released);
/// `Some(false)` below the threshold (nothing happens).
pub(crate) fn squeak_gate(audio: &AudioState, threshold: i32) -> Option<bool> {
    if !(audio.foot_in_deck_box_615
        && audio.foot_in_deck_box_616
        && audio.wheel_count_200 as i32 > 1)
    {
        return None;
    }
    Some(fctiwz(audio.deck_tilt_264.abs() * SQUEAK_ANGLE) >= threshold)
}

/// `sub_824C7388`: the skid surface, map entry `+12` of wheel 0's material (0 with none).
fn skid_surface(vault: &BoardVault, audio: &AudioState) -> i32 {
    let material = audio.wheel_material_620[0];
    if material as i32 >= 143 {
        0
    } else {
        vault.surfaces.lookup(material as i32, 12) as i32
    }
}

/// `sub_824AF678`: the skid post (18 words).
#[allow(clippy::too_many_arguments)]
pub(crate) fn skid_post(
    speed: i32,
    soft: i32,
    surface: i32,
    slip: i32,
    gain: i32,
    level: i32,
    global: i32,
    first_local: i32,
    local: i32,
    local_level: i32,
    tweak: i32,
) -> [u32; 18] {
    [
        0,
        32_767,
        0,
        0,
        0,
        25_000,
        0,
        clamp(speed, 10_000),
        clamp(soft, 1),
        clamp(surface, 4),
        clamp(slip, 90),
        clamp(gain, 32_767),
        clamp(level, 32_767),
        clamp(global, 1),
        clamp(first_local, 1),
        clamp(local, 1),
        clamp(local_level, 32_767),
        clamp(tweak, 32_767),
    ]
}

/// `fctiwz(clamp01(v × 0.08) × scale)`, the skid (10000) and squeak (1000) speed words.
fn slow_speed(speed: f32, scale: f32) -> i32 {
    unit_word(speed * SPEED_SCALE, scale)
}

/// `sub_824C7A20`'s words (the counter is updated by the caller first).
pub(crate) fn skid_update(
    words: &mut [u32; 18],
    c: &dyn Controls,
    audio: &AudioState,
    counter: i32,
    surface: i32,
    local: bool,
) {
    words[7] = clamp(slow_speed(audio.ground_speed_208, TEN_THOUSAND), 10_000);
    words[4] = clamp(c.pitch(3), 8192);
    words[0] = 32_767;
    words[1] = clamp(c.level(4) as i32, 32_767);
    words[3] = clamp(c.raw(0) as i32, 65_536);
    words[2] = clamp(c.level(13) as i32, 32_767);
    words[5] = clamp(c.level(11) as i32, 25_000);
    words[6] = clamp(c.level(12) as i32, 25_000);
    let slip = fctiwz(audio.slip_232 * MINUS_NINETY);
    words[10] = clamp(counter.wrapping_sub(slip).min(90), 90);
    words[9] = clamp(surface, 4);
    words[16] = clamp(if local { c.level(20) as i32 } else { 0 }, 32_767);
}

/// The squeak angle word `|z| / 1.5 × 1000`, 1000 at most, 0 below 50.
fn squeak_turn(z: f32, divisor: f32) -> i32 {
    let value = fctiwz(z.abs() / divisor * THOUSAND);
    if value > 1_000 {
        1_000
    } else if value < 50 {
        0
    } else {
        value
    }
}

/// `sub_824AFF48`: the squeak post (11 words).
pub(crate) fn squeak_post(speed: i32, turn: i32, tweak: i32) -> [u32; 11] {
    [
        0,
        32_767,
        0,
        0,
        4096,
        25_000,
        0,
        clamp(speed, 1_000),
        15,
        clamp(turn, 1_000),
        clamp(tweak, 32_767),
    ]
}

/// `sub_824C7DD0`.
pub(crate) fn squeak_update(
    words: &mut [u32; 11],
    c: &dyn Controls,
    audio: &AudioState,
    divisor: f32,
) {
    let speed = slow_speed(audio.ground_speed_208, THOUSAND);
    let turn = squeak_turn(audio.deck_angular_velocity_480[2], divisor);
    words[7] = clamp(speed, 1_000);
    words[9] = clamp(turn, 1_000);
    words[4] = clamp(c.pitch(3), 8192);
    words[0] = 32_767;
    words[1] = clamp(c.level(5) as i32, 32_767);
    words[2] = 0;
    words[5] = clamp(c.level(11) as i32, 25_000);
    words[6] = clamp(c.level(12) as i32, 25_000);
    words[3] = clamp(c.raw(0) as i32, 65_536);
}

/// `sub_824B0670`: the loose-board scrape post (12 words).
pub(crate) fn board_slide_post(tweak: i32, loose: i32) -> [u32; 12] {
    [
        0,
        0,
        4096,
        0,
        25_000,
        0,
        0,
        0,
        clamp(tweak, 32_767),
        0,
        clamp(loose, 3),
        0,
    ]
}

/// `sub_824CB4C0`.
pub(crate) fn board_slide_update(
    words: &mut [u32; 12],
    c: &dyn Controls,
    audio: &AudioState,
    vault: &BoardVault,
) {
    words[0] = 32_767;
    words[1] = clamp(c.raw(0) as i32, 65_535);
    words[2] = clamp(c.pitch(23), 8192);
    words[3] = board_slide_speed(audio.ground_speed_208, vault.slide_kmh);
    words[4] = clamp(c.level(25) as i32, 25_000);
    words[5] = clamp(c.level(26) as i32, 25_000);
    words[6] = clamp(c.level(27) as i32, 32_767);
    words[7] = clamp(c.level(24) as i32, 32_767);
    let level = if audio.loose_board_780 == 2 {
        vault.slide_level_loose
    } else {
        vault.slide_level
    };
    words[11] = clamp(level, 32_767);
}

/// `sub_824C82A8` as a pure function. `latch` is `+1504` after `sub_824CA688` on the local
/// player (false otherwise). `primary` is `[owner+1500]`: that truck reads wheel 0, the other one
/// wheel 3.
pub(crate) fn truck_surface(
    vault: &BoardVault,
    policy: SurfacePolicy,
    latch: bool,
    primary: usize,
    audio: &AudioState,
    truck: usize,
) -> u32 {
    let wheel = if truck == primary { 0 } else { 3 };
    if latch && !audio.wheel_landed_464[wheel] {
        return 14;
    }
    if audio.grinding_341 {
        return 14;
    }
    let material = audio.wheel_material_620[wheel];
    let retail = if material as i32 >= 143 {
        3
    } else {
        vault.surfaces.lookup(material as i32, 4)
    };
    match (policy, retail) {
        (SurfacePolicy::Retail, _) | (_, 14) => retail,
        (SurfacePolicy::Default(surface), _) => surface,
    }
}

/// `sub_824C5CA8`'s selector for a surface without grains (7, 8, 10..=13). `None` for a grain
/// surface (1..=6, 9).
pub(crate) fn rolling_selector(surface: u32) -> Option<i32> {
    Some(match surface {
        7 => 1,
        8 => 2,
        10 => 10,
        11 => 12,
        12 => 11,
        13 => 9,
        _ => return None,
    })
}

/// The routing bookkeeping of `sub_824C5CA8`:
///
/// - `+768+4t`: the truck's surface (14 = none);
/// - `+1320+4t`: the surface plays grains;
/// - `+1328+t`: grains running;
/// - `+1496+t`: the truck has a live sound;
/// - `+1500`: the primary truck.
///
/// It is pure, so the capture replay drives the same code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Routing {
    pub surface: [u32; 2],
    pub grain_surface: [bool; 2],
    pub grains: [bool; 2],
    pub live: [bool; 2],
    pub primary: usize,
}

impl Default for Routing {
    /// `sub_824C59C8`: both trucks on 14, `+1320` = 1, nothing live, truck 0 primary.
    fn default() -> Self {
        Self {
            surface: [14; 2],
            grain_surface: [true; 2],
            grains: [false; 2],
            live: [false; 2],
            primary: 0,
        }
    }
}

/// One action of [`Routing::route`], in retail order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteStep {
    /// Owner input 0 := 32767 (a truck left a surface). Input 0 is set to 0 before any step.
    Pulse,
    /// Stop the truck's sound. If its surface had no grains, release its per-surface
    /// `Class_rolling`; otherwise stop its grain players if they were running.
    Stop {
        truck: usize,
        grain_surface: bool,
        grains: bool,
    },
    /// The truck is now on `surface` (never 14). `sound` is false when the other truck is already
    /// on it; retail then only updates the collection key and `+1320`.
    Start {
        truck: usize,
        surface: u32,
        sound: bool,
    },
}

impl Routing {
    /// `sub_824C5CA8`'s loop: the primary truck, then (local player only) the other one.
    /// `surface_of(truck, primary)` is `sub_824C82A8`.
    pub(crate) fn route(
        &mut self,
        local: bool,
        surface_of: &mut dyn FnMut(usize, usize) -> u32,
    ) -> Vec<RouteStep> {
        let mut steps = Vec::new();
        for iteration in 0..2 {
            let mut t = self.primary;
            if iteration == 1 {
                t = usize::from(t == 0);
            }
            let mut other = usize::from(t == 0);
            let surface = surface_of(t, self.primary);
            let current = self.surface[t];
            if surface != current {
                if current != 14 {
                    steps.push(RouteStep::Pulse);
                    if local && !self.live[other] {
                        // Hand the sound to the other truck: `[1500]` flips and the new surface
                        // is stored for it. Nothing is stopped, and this truck's `+1496` stays.
                        std::mem::swap(&mut t, &mut other);
                        self.primary = usize::from(self.primary == 0);
                    } else {
                        steps.push(RouteStep::Stop {
                            truck: t,
                            grain_surface: self.grain_surface[t],
                            grains: self.grains[t],
                        });
                        if self.grain_surface[t] {
                            self.grains[t] = false;
                        }
                        self.live[t] = false;
                    }
                }
                self.surface[t] = surface;
                if surface != 14 {
                    let grain = rolling_selector(surface).is_none();
                    let sound = surface != self.surface[other];
                    steps.push(RouteStep::Start {
                        truck: t,
                        surface,
                        sound,
                    });
                    if sound {
                        self.live[t] = true;
                        if grain {
                            self.grains[t] = true;
                        }
                    }
                    self.grain_surface[t] = grain;
                }
            }
            if !local {
                break;
            }
        }
        steps
    }
}

// ------------------------------------------------------------------ the owner

/// One truck's side of the owner (`owner + 4t`, `+16t`, `+48t` fields).
#[derive(Clone, Debug)]
struct Truck {
    /// `+184+16t`: the grain-class collection key this truck's attribute instance holds.
    key: Option<u64>,
    /// `+1312`: the per-surface `Class_rolling` handle and words.
    rolling: Option<(u32, [u32; 12])>,
    /// `+1488`: its selector.
    selector: u32,
    /// Players A and B (`+1176`, `+1180`).
    players: [u32; 2],
}

pub(crate) struct Board {
    config: BoardConfig,
    vault: BoardVault,
    trucks: [Truck; 2],
    routing: Routing,
    /// `+760`: the last `sub_824C8370` key.
    key_760: u64,
    /// `+1288`, `+1292`/`+1296`, `+1300`, `+1304`, `+1308`, `+1332`, `+1884`.
    skid: Option<(u32, [u32; 18])>,
    squeak: Option<(u32, [u32; 11])>,
    squeak_sign: bool,
    rattle: Option<(u32, [u32; 12])>,
    layer0: Option<(u32, [u32; 12])>,
    layer3: Option<(u32, [u32; 12])>,
    layer5: Option<(u32, [u32; 12])>,
    board_slide: Option<(u32, [u32; 12])>,
    /// `+1516`.
    skid_counter: i32,
    latch_1504: bool,
    latch_1505: bool,
    /// `+912`, `+1036`, `+1340`.
    scale_envelope: Envelope,
    shift_envelope: Envelope,
    seam_envelope: Envelope,
    /// `+1336`, `+1464`, `+1468`, `+1472..+1484`.
    seam_pattern: u32,
    seam_active: bool,
    seam_target: f32,
    seam_wobble: Wobble,
    f1160: f32,
    f1164: f32,
    f1168: f32,
    f1508: f32,
    f1512: f32,
    f1556: f32,
    f1560: f32,
    /// The two `+1572`/`+1728` wobble records: envelope, target, last posted, gains.
    wobble_envelopes: [Envelope; 2],
    wobble_targets: [f32; 2],
    wobble_last: [f32; 2],
    rng: TitleRng,
    owner_inputs: Vec<(u32, u32)>,
    /// `+1892`, `+1896`: vfunc52(0) at the last two updates; `+1900`: input 5's slewed rate.
    raw_1892: u32,
    raw_1896: u32,
    f1900: f32,
}

impl Board {
    /// `sub_824C5058` + `sub_824C59C8`: load the grain members, create the four players, and (local)
    /// post the held `Class_rolling` layers 0 and 3 (`sub_824C9830`).
    pub(crate) fn new(
        tick: &mut Tick,
        vault: BoardVault,
        members: &[GrainMember],
        config: BoardConfig,
    ) -> Result<Self, String> {
        let mut players = [[0u32; 2]; 2];
        {
            let mut grains = tick.runtime.grains();
            for m in members {
                grains
                    .load(&m.name, &m.bytes, m.samples.clone(), m.channels, m.rate)
                    .map_err(|e| e.to_string())?;
            }
            for truck in &mut players {
                for player in truck.iter_mut() {
                    *player = grains.create_player().map_err(|e| e.to_string())?;
                }
            }
        }
        let truck = |players: [u32; 2], key| Truck {
            key,
            rolling: None,
            selector: 0,
            players,
        };
        let mut board = Self {
            config,
            // The constructor's last `sub_824C5BF8` leaves truck 0's instance on wood_ramp_soft.
            trucks: [
                truck(players[0], Some(0x382B_1263_6ED9_D8DA)),
                truck(players[1], None),
            ],
            vault,
            routing: Routing::default(),
            key_760: DEFAULT_KEY,
            skid: None,
            squeak: None,
            squeak_sign: false,
            rattle: None,
            layer0: None,
            layer3: None,
            layer5: None,
            board_slide: None,
            skid_counter: 0,
            latch_1504: false,
            latch_1505: false,
            scale_envelope: Envelope::default(),
            shift_envelope: Envelope::default(),
            seam_envelope: Envelope::default(),
            seam_pattern: 0,
            seam_active: false,
            seam_target: 0.0,
            seam_wobble: Wobble {
                gain_low: 1.0,
                gain_high: 1.0,
                ms_low: 0,
                ms_high: 0,
            },
            f1160: 0.0,
            f1164: 0.0,
            f1168: 0.0,
            f1508: 0.0,
            f1512: 0.0,
            f1556: 0.0,
            f1560: 1.0,
            wobble_envelopes: [Envelope::default(), Envelope::default()],
            wobble_targets: [0.0; 2],
            wobble_last: [0.0; 2],
            rng: TitleRng(RNG_SEED),
            owner_inputs: Vec::new(),
            raw_1892: 0,
            raw_1896: 0,
            f1900: 0.0,
        };
        // sub_824C5058 fills the wobble records' +16 with 0 and +20..+28 with 1.0 (sub_824C59C8).
        board.wobble_last = [1.0; 2];
        if board.config.local {
            board.post_held_layers(tick)?;
        }
        Ok(board)
    }

    fn set_input(&mut self, id: u32, value: u32) {
        self.owner_inputs.push((id, value));
    }

    fn tuning_of(&self, truck: usize) -> &SurfaceTuning {
        self.vault
            .tuning(self.trucks[truck].key.unwrap_or(DEFAULT_KEY))
    }

    /// `sub_824C9830`.
    fn post_held_layers(&mut self, tick: &mut Tick) -> Result<(), String> {
        let speed = tick.audio.ground_speed_208;
        let zero = speed_word(speed, self.vault.rolling_kmh(0));
        let words = rolling_post(zero, 0, 3);
        self.layer0 = Some((post(tick.runtime, ROLLING, &words)?, words));
        let three = speed_word(speed, self.vault.rolling_kmh(3));
        let words = rolling_post(three, 3, 3);
        self.layer3 = Some((post(tick.runtime, ROLLING, &words)?, words));
        Ok(())
    }

    /// `sub_824CA688`, updating the `+1504` latch.
    fn manual_latch(&mut self, audio: &AudioState) -> bool {
        self.latch_1504 =
            grain_board::manual_latch(self.latch_1504, audio.balance_340, audio.wheel_count_200);
        self.latch_1504
    }

    /// `sub_824CA6E0`, updating the `+1505` latch.
    fn trick_latch(&mut self, audio: &AudioState) -> bool {
        self.latch_1505 = grain_board::trick_latch(
            self.latch_1505,
            audio.hippy_jump_372,
            audio.foot_in_deck_box_615,
            audio.foot_in_deck_box_616,
        );
        self.latch_1505
    }

    fn special(&mut self, audio: &AudioState) -> bool {
        self.manual_latch(audio) || self.trick_latch(audio)
    }

    /// `sub_824C82A8`: truck `t`'s surface, 14 when it has none (updates the `+1504` latch on
    /// the local player, as retail's `sub_824CA688` call does).
    pub(crate) fn surface_of(&mut self, audio: &AudioState, truck: usize) -> u32 {
        let latch = self.config.local && self.manual_latch(audio);
        truck_surface(
            &self.vault,
            self.config.surfaces,
            latch,
            self.routing.primary,
            audio,
            truck,
        )
    }

    /// `sub_824C6B30(layer)`: the speed word, scaled by the push envelope while it runs.
    fn scaled_speed_word(&self, audio: &AudioState, layer: i32) -> i32 {
        let mut speed = audio.ground_speed_208;
        if !self.scale_envelope.idle {
            speed *= self.scale_envelope.value;
        }
        speed_word(speed, self.vault.rolling_kmh(layer))
    }

    /// `sub_824CA738`: `+1508`/`+1512` from the state's slope `+712` and inputs 2 and 3.
    pub(crate) fn slope_inputs(&mut self, slope_712: f32) {
        let primary = self.routing.primary;
        let key = self.trucks[primary].key.unwrap_or(DEFAULT_KEY);
        let levels = slope_levels(
            slope_712,
            self.routing.grain_surface[primary],
            self.vault.slope(key),
        );
        (self.f1508, self.f1512) = levels;
        let [a, b] = slope_words(levels);
        self.set_input(2, a);
        self.set_input(3, b);
    }

    /// `sub_824C5CA8`: route each truck's surface ([`Routing::route`]) and run the steps.
    pub(crate) fn route_surfaces(&mut self, tick: &mut Tick) -> Result<(), String> {
        let audio = tick.audio;
        let local = self.config.local;
        let mut routing = self.routing.clone();
        let mut latch_1504 = self.latch_1504;
        let (vault, policy) = (&self.vault, self.config.surfaces);
        let steps = routing.route(local, &mut |truck, primary| {
            let latch = local && {
                latch_1504 =
                    grain_board::manual_latch(latch_1504, audio.balance_340, audio.wheel_count_200);
                latch_1504
            };
            truck_surface(vault, policy, latch, primary, audio, truck)
        });
        self.routing = routing;
        self.latch_1504 = latch_1504;
        self.set_input(0, 0);
        for step in steps {
            match step {
                RouteStep::Pulse => self.set_input(0, 32_767),
                RouteStep::Stop {
                    truck,
                    grain_surface,
                    grains,
                } => self.stop_truck_sound(tick, truck, grain_surface, grains)?,
                RouteStep::Start {
                    truck,
                    surface,
                    sound,
                } => self.start_truck_sound(tick, truck, surface, sound)?,
            }
        }
        let first = self.surface_of(audio, 0);
        self.set_input(6, if first == 9 { 32_767 } else { 0 });
        Ok(())
    }

    fn stop_truck_sound(
        &mut self,
        tick: &mut Tick,
        t: usize,
        grain_surface: bool,
        grains: bool,
    ) -> Result<(), String> {
        if !grain_surface {
            let mut holder = self.trucks[t].rolling.take().map(|(h, _)| h);
            release(tick.runtime, &mut holder)?;
        } else if grains {
            let mut g = tick.runtime.grains();
            for &player in &self.trucks[t].players {
                g.stop(player).map_err(|e| e.to_string())?;
            }
            // sub_824C4D50 (chain teardown) is not run: chains are kept (grain::chain note).
        }
        Ok(())
    }

    fn start_truck_sound(
        &mut self,
        tick: &mut Tick,
        t: usize,
        surface: u32,
        sound: bool,
    ) -> Result<(), String> {
        // sub_824C8370 tests softness with sub_824B23C8([owner+36]); for the local player that is
        // the audio state's +684.
        let soft = tick.audio.soft_wheels_684 != 0;
        let choice = grain_board::grain_for_surface(surface, soft);
        let key = choice.map_or(DEFAULT_KEY, |c| c.key);
        self.key_760 = key;
        self.trucks[t].key = Some(key);
        if !sound {
            return Ok(());
        }
        if let Some(selector) = rolling_selector(surface) {
            self.trucks[t].selector = selector as u32;
            let words = rolling_post(0, selector, surface as i32);
            self.trucks[t].rolling = Some((post(tick.runtime, ROLLING, &words)?, words));
        } else if let Some(choice) = choice {
            let tuning = self.vault.tuning(key).clone();
            let chain = ChainConfig {
                local: self.config.local,
                ..self.vault.chain
            };
            let mut g = tick.runtime.grains();
            for which in 0..2 {
                let player = self.trucks[t].players[which];
                let bus = g.chain(player, &chain).map_err(|e| e.to_string())?;
                g.bind(player, choice.member, tuning.params[which], bus)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// `sub_824C6198`.
    pub(crate) fn push_and_rattle(&mut self, tick: &mut Tick) -> Result<(), String> {
        let audio = tick.audio;
        let planted = audio.push_left_333 || audio.push_right_334;
        self.set_input(4, if planted { 32_767 } else { 0 });
        if audio.push_plant_335 {
            let primary_key = self.trucks[self.routing.primary].key.unwrap_or(DEFAULT_KEY);
            let push = self.vault.tuning(primary_key).push;
            program_push(
                &push,
                audio.ground_speed_208,
                &mut self.scale_envelope,
                &mut self.shift_envelope,
            );
            let mut holder = self.rattle.take().map(|(h, _)| h);
            release(tick.runtime, &mut holder)?;
            if self.routing.grain_surface[self.routing.primary] {
                let divisor = self.vault.rattle_kmh(primary_key);
                let speed = rattle_speed(audio.ground_speed_208, divisor) as i32;
                let code = match self.key_760 {
                    0x7C59_12FC_2DAB_F98C => 1,
                    0x0372_1D0F_A99A_03C8 => 2,
                    0xFFB5_E3E6_2E0B_4943 => 3,
                    0x7947_A259_F181_FDB4 => 4,
                    0xB303_AED8_2415_30E2 => 5,
                    _ => 0,
                };
                let words = rattle_post(speed, code, self.vault.rattle_tweak);
                self.rattle = Some((post(tick.runtime, RATTLE, &words)?, words));
            }
        }
        if !self.scale_envelope.idle {
            self.scale_envelope.advance(tick.dt)?;
        }
        if !self.shift_envelope.idle {
            self.shift_envelope.advance(tick.dt)?;
        }
        // sub_824C8588.
        let latch = self.manual_latch(audio);
        let ca6e0 = if latch {
            false
        } else {
            self.trick_latch(audio)
        };
        let v = audio.com_velocity_96;
        let tuning = self.tuning_of(self.routing.primary).clone();
        let slew = grain_board::slew(
            &tuning,
            &SlewInputs {
                vector: [v[0], v[1], v[2], 0.0],
                factor: audio.turn_204,
                state_200: audio.wheel_count_200 as i32,
                state_340: audio.balance_340,
                latch_1504: latch,
                ca6e0,
                previous: self.f1160,
            },
        );
        self.f1160 = slew.f1160;
        self.f1164 = slew.f1164;
        self.latch_1504 = slew.latch_1504;
        // The +1168 brake slew.
        // `fcmpu dot, 0.0 ; bge` → toward 0; an unordered dot falls through toward 1.
        let toward_one = audio.brake_336 && {
            let d = audio.com_velocity_delta_128;
            let d = grain_board::normalised_dot([d[0], d[1], d[2], 0.0], [v[0], v[1], v[2], 0.0]);
            !(d >= 0.0)
        };
        let f0 = self.f1168;
        self.f1168 = if toward_one {
            if 1.0 - f0 > BRAKE_STEP {
                f0 + BRAKE_STEP
            } else if f0 - 1.0 > BRAKE_STEP {
                f0 - BRAKE_STEP
            } else {
                1.0
            }
        } else if -f0 > BRAKE_STEP {
            f0 + BRAKE_STEP
        } else if f0 > BRAKE_STEP {
            f0 - BRAKE_STEP
        } else {
            0.0
        };
        Ok(())
    }

    /// `sub_824C9058`.
    pub(crate) fn push_chains(&mut self, tick: &mut Tick) -> Result<(), String> {
        let c = tick.controls;
        for t in 0..2 {
            if !self.routing.grains[t] || !self.routing.grain_surface[t] {
                continue;
            }
            let special = self.special(tick.audio);
            let tuning = self.tuning_of(t).clone();
            let primary = self.tuning_of(self.routing.primary).clone();
            let values = grain_board::chain_values(
                &tuning,
                &ChainInputs {
                    mix64_11: c.level(11) as i32,
                    mix64_12: c.level(12) as i32,
                    mix52_0: c.raw(0) as i32,
                    mix60_13: c.level(13) as i32,
                    local: self
                        .config
                        .local
                        .then(|| (c.level(21) as i32, c.level(22) as i32)),
                    special,
                    f1152: (!self.shift_envelope.idle).then_some(self.shift_envelope.value),
                    boost: self.f1508,
                    boost_shift_a: primary.shift_boost,
                    boost_shift_b: primary.shift_boost_b,
                },
            );
            let [a, b] = self.trucks[t].players;
            tick.runtime
                .grains()
                .push_chain(a, b, &values)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// `sub_824C7438`.
    pub(crate) fn skid_trigger(&mut self, tick: &mut Tick) -> Result<(), String> {
        let audio = tick.audio;
        if !skid_predicate(audio, self.skid_counter) {
            self.set_input(1, 0);
            return Ok(());
        }
        self.set_input(1, 32_767);
        if self.skid.is_some() {
            return Ok(());
        }
        let local_level = if self.config.local {
            tick.controls.level(20) as i32
        } else {
            0
        };
        let words = skid_post(
            slow_speed(audio.ground_speed_208, TEN_THOUSAND),
            i32::from(audio.soft_wheels_684 != 0),
            skid_surface(&self.vault, audio),
            fctiwz(audio.slip_232 * NINETY),
            self.vault.skid_gain,
            self.vault.skid_level,
            i32::from(self.config.global_flag),
            i32::from(self.config.local && self.config.first_player),
            i32::from(self.config.local),
            local_level,
            self.vault.skid_tweak,
        );
        self.skid = Some((post(tick.runtime, SKID, &words)?, words));
        Ok(())
    }

    /// `sub_824C7738`.
    pub(crate) fn squeak_trigger(&mut self, tick: &mut Tick) -> Result<(), String> {
        let audio = tick.audio;
        match squeak_gate(audio, self.vault.squeak_threshold) {
            None => {
                let mut holder = self.squeak.take().map(|(h, _)| h);
                release(tick.runtime, &mut holder)?;
            }
            Some(false) => {}
            Some(true) => {
                let (drop, start) = squeak_step(
                    audio.deck_tilt_264,
                    &mut self.squeak_sign,
                    self.squeak.is_some(),
                );
                if drop {
                    let mut holder = self.squeak.take().map(|(h, _)| h);
                    release(tick.runtime, &mut holder)?;
                }
                if start {
                    let words = squeak_post(
                        slow_speed(audio.ground_speed_208, THOUSAND),
                        squeak_turn(
                            audio.deck_angular_velocity_480[2],
                            self.vault.squeak_divisor,
                        ),
                        self.vault.squeak_tweak,
                    );
                    self.squeak = Some((post(tick.runtime, SQUEAKS, &words)?, words));
                }
            }
        }
        Ok(())
    }

    /// `sub_824C9F68`: rolling layer 5 while wheel 0 is on the spidercrack pattern.
    pub(crate) fn crack_layer(&mut self, tick: &mut Tick) -> Result<(), String> {
        let audio = tick.audio;
        let mut pattern = audio.wheel_seam_636[0];
        if self.manual_latch(audio) && !audio.wheel_landed_464[0] {
            pattern = audio.wheel_seam_636[3];
        }
        let speed = self.scaled_speed_word(audio, 5);
        match (&self.layer5, pattern == 1) {
            (None, true) => {
                let words = rolling_post(speed, 5, 3);
                self.layer5 = Some((post(tick.runtime, ROLLING, &words)?, words));
            }
            (Some(_), false) => {
                let mut holder = self.layer5.take().map(|(h, _)| h);
                release(tick.runtime, &mut holder)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// `sub_824CA318`: a random gain in `[+1472, +1476]` and duration in `[+1480, +1484]`.
    fn seam_random(&mut self) -> (f32, i32) {
        let w = self.seam_wobble;
        let mut gain = w.gain_low;
        if w.gain_low != w.gain_high {
            let range = fctiwz((w.gain_high - w.gain_low) * HUNDRED);
            let draw = self.rng.next();
            gain = if range > 0 {
                let m = draw % range as u32;
                m as i32 as f32 * HUNDREDTH
            } else {
                let range = range.unsigned_abs().max(1);
                let m = draw % range;
                m as i32 as f32 * MINUS_HUNDREDTH
            };
            gain += w.gain_low;
        }
        let mut ms = w.ms_low;
        if w.ms_high != w.ms_low {
            let range = w.ms_high.wrapping_sub(w.ms_low).unsigned_abs().max(1);
            ms = (self.rng.next() % range) as i32 + w.ms_low;
        }
        (gain, ms)
    }

    /// `sub_824CA448` (local only).
    pub(crate) fn seam_envelope(&mut self, tick: &mut Tick) -> Result<(), String> {
        if !self.config.local {
            return Ok(());
        }
        let pattern = tick.audio.wheel_seam_636[0];
        if pattern != self.seam_pattern {
            self.seam_envelope.reset();
            self.seam_pattern = pattern;
            self.seam_active = false;
            if pattern != 0 {
                // Patterns 1..=15 name a collection (`sub_82497A58`); any other has no key, the
                // lookup fails and every field reads the image's zero block at `0x830D0850`.
                self.seam_wobble = if (1..=15).contains(&pattern) {
                    self.vault.seam[pattern as usize]
                } else {
                    Wobble {
                        gain_low: 0.0,
                        gain_high: 0.0,
                        ms_low: 0,
                        ms_high: 0,
                    }
                };
                if self.seam_wobble.gain_low != 1.0 || self.seam_wobble.gain_high != 1.0 {
                    self.seam_active = true;
                    let (gain, ms) = self.seam_random();
                    self.seam_envelope.reset();
                    self.seam_envelope.add(1.0, gain, ms);
                    self.seam_target = gain;
                }
            }
        } else if !self.seam_envelope.idle {
            self.seam_envelope.advance(tick.dt)?;
        }
        Ok(())
    }

    /// `sub_824CAEC0` (local only): the graph-1 → graph-3 send level and `+1560` from speed.
    pub(crate) fn distortion_send(&mut self, tick: &mut Tick) -> Result<(), String> {
        if !self.config.local {
            return Ok(());
        }
        let kmh = tick.audio.ground_speed_208 * KMH_PER_MS;
        let [low, high, level, _] = self.vault.send_ramp;
        let mut send = 0.0;
        if kmh >= high {
            send = level;
        } else if kmh >= low {
            send = (kmh - low) / (high - low) * level;
        }
        let [start, end, floor] = self.vault.level_ramp;
        if kmh >= end {
            self.f1560 = floor;
        } else if kmh >= start {
            let frac = (kmh - start) / (end - start);
            // fnmsubs f5 = 1 − frac·(1 − floor), fused.
            self.f1560 = nmsub_single(f64::from(frac), f64::from(1.0 - floor), 1.0) as f32;
        }
        if send != self.f1556 {
            self.f1556 = send;
            let records: Vec<u32> = self
                .trucks
                .iter()
                .flat_map(|t| t.players)
                .filter_map(|p| tick.runtime.grains().chain_record(p))
                .collect();
            let grains = tick.runtime.grains();
            for record in records {
                let module = grains
                    .g
                    .u32(grains.g.u32(record).map_err(|e| e.to_string())? + 16)
                    .map_err(|e| e.to_string())?;
                skate_audio_core::device::post_property(grains.g, module, 0, f64::from(send))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// `sub_824CB078` + `sub_824CB180` (local only): the graph-1/3 `Gai0` gain wobble.
    pub(crate) fn gain_wobble(&mut self, tick: &mut Tick) -> Result<(), String> {
        if !self.config.local {
            return Ok(());
        }
        // sub_824CB078: a record whose envelope is idle draws a new segment (duration
        // `ms_low + rnd % (ms_high − ms_low)`, magnitude `gain_low + (rnd % fctiwz((high − low)·100))
        // · 0.01`, sign opposite to the last target's); a running one advances. `twllei` traps a
        // zero divisor in retail; the vault spans (15, 0.30 and 10, 0.15) are non-zero.
        for k in 0..2 {
            if self.wobble_envelopes[k].idle {
                let w = self.vault.wobbles[k];
                let span = w.ms_high.wrapping_sub(w.ms_low) as u32;
                let ms = (self.rng.next() % span.max(1)) as i32 + w.ms_low;
                let range = fctiwz((w.gain_high - w.gain_low) * HUNDRED) as u32;
                let m = self.rng.next() % range.max(1);
                let mut target = fmadd_single(
                    f64::from(m as i32 as f32),
                    f64::from(HUNDREDTH),
                    f64::from(w.gain_low),
                ) as f32;
                if !(self.wobble_targets[k] < 0.0) {
                    target *= -1.0;
                }
                self.wobble_envelopes[k].reset();
                self.wobble_envelopes[k].add(self.wobble_targets[k], target, ms);
                self.wobble_targets[k] = target;
            } else {
                self.wobble_envelopes[k].advance(tick.dt)?;
            }
        }
        let kmh = tick.audio.ground_speed_208 * KMH_PER_MS;
        let [low, high] = self.vault.wobble_ramp;
        let ramp = if kmh >= high {
            1.0
        } else if kmh >= low {
            (kmh - low) / (high - low)
        } else {
            0.0
        };
        let gains: [f32; 2] = std::array::from_fn(|k| {
            fmadd_single(
                f64::from(self.wobble_envelopes[k].value),
                f64::from(ramp),
                1.0,
            ) as f32
        });
        // Chain record `2r26 + k` (r26 = 0, 2) takes wobble k; its "last posted" (+1596 / +1752)
        // is shared by both trucks, so after truck 0 stores it truck 1 always compares equal and
        // is never posted (retail behaviour, kept).
        for t in 0..2 {
            for which in 0..2 {
                let current = gains[which];
                if current == self.wobble_last[which] {
                    continue;
                }
                self.wobble_last[which] = current;
                let player = self.trucks[t].players[which];
                let Some(record) = tick.runtime.grains().chain_record(player) else {
                    continue;
                };
                let grains = tick.runtime.grains();
                let mods1 = grains.g.u32(record).map_err(|e| e.to_string())?;
                let mods3 = grains.g.u32(record + 16).map_err(|e| e.to_string())?;
                let gain1 = grains.g.u32(mods1 + 20).map_err(|e| e.to_string())?;
                skate_audio_core::device::post_property(
                    grains.g,
                    gain1,
                    0,
                    f64::from(current * self.f1560),
                )
                .map_err(|e| e.to_string())?;
                if mods3 != 0 {
                    let gain3 = grains.g.u32(mods3 + 8).map_err(|e| e.to_string())?;
                    skate_audio_core::device::post_property(grains.g, gain3, 0, f64::from(current))
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }

    /// `sub_824CB3C8`.
    pub(crate) fn board_slide_trigger(&mut self, tick: &mut Tick) -> Result<(), String> {
        let loose = tick.audio.loose_board_780;
        match (&self.board_slide, loose) {
            (None, 0) | (Some(_), 1..) => {}
            (None, _) => {
                let words = board_slide_post(self.vault.slide_tweak, i32::from(loose == 2));
                self.board_slide = Some((post(tick.runtime, BOARD_SLIDE, &words)?, words));
            }
            (Some(_), 0) => {
                let mut holder = self.board_slide.take().map(|(h, _)| h);
                release(tick.runtime, &mut holder)?;
            }
        }
        Ok(())
    }

    /// `sub_824C7A20`.
    fn skid_updater(&mut self, tick: &mut Tick) -> Result<(), String> {
        if self.skid.is_none() {
            return Ok(());
        }
        let audio = tick.audio;
        if !skid_predicate(audio, self.skid_counter) {
            let mut holder = self.skid.take().map(|(h, _)| h);
            return release(tick.runtime, &mut holder);
        }
        self.skid_counter = skid_counter_step(self.skid_counter, audio.revert_690);
        let surface = skid_surface(&self.vault, audio);
        let counter = self.skid_counter;
        let local = self.config.local;
        if let Some((handle, words)) = self.skid.as_mut() {
            skid_update(words, tick.controls, audio, counter, surface, local);
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }

    /// The `w6` surface the held layers use: the primary truck's, else the other's; `None` when
    /// both are 14 and not grinding (grinding writes 13).
    fn layer_surface(&mut self, audio: &AudioState) -> Option<i32> {
        let latch = self.config.local && self.manual_latch(audio);
        layer_surface(
            &self.vault,
            self.config.surfaces,
            latch,
            self.routing.primary,
            audio,
        )
    }
}

/// `sub_824C9948`'s `w6`: `sub_824C82A8` of the primary truck, else of the other one. `None` when
/// both are 14 and the skater is not grinding (the word keeps its value); grinding writes 13.
pub(crate) fn layer_surface(
    vault: &BoardVault,
    policy: SurfacePolicy,
    latch: bool,
    primary: usize,
    audio: &AudioState,
) -> Option<i32> {
    let mut surface = truck_surface(vault, policy, latch, primary, audio, primary);
    if surface == 14 {
        let other = usize::from(primary == 0);
        surface = truck_surface(vault, policy, latch, primary, audio, other);
        if surface == 14 {
            return audio.grinding_341.then_some(13);
        }
    }
    Some(surface as i32)
}

fn unit_ratio(ratio: f32) -> f32 {
    let clamped = if !(-ratio >= 0.0) { ratio } else { 0.0 };
    if 1.0 - clamped >= 0.0 { clamped } else { 1.0 }
}

/// `sub_824CA738`'s `(+1508, +1512)`: with the primary truck on a grain surface, a downhill
/// slope (`+712` < 0) gives `clamp01(slope / down)` and an uphill one `clamp01(slope / up)`
/// (`down`, `up` = `0x57A78D3BE8D47BB3`, `0x8DD4C3FC8DAF4059` of the truck's grain collection:
/// −10 and 10); otherwise both are 0.
pub(crate) fn slope_levels(slope: f32, grain_surface: bool, (down, up): (f32, f32)) -> (f32, f32) {
    if !grain_surface {
        return (0.0, 0.0);
    }
    if slope < 0.0 {
        (unit_ratio(slope / down), 0.0)
    } else if slope > 0.0 {
        (0.0, unit_ratio(slope / up))
    } else {
        (0.0, 0.0)
    }
}

/// Owner inputs 2 and 3: `fctiwz(level × 32767)` clamped to 0..32767.
pub(crate) fn slope_words((down, up): (f32, f32)) -> [u32; 2] {
    [
        clamp(fctiwz(down * LEVEL), 32_767),
        clamp(fctiwz(up * LEVEL), 32_767),
    ]
}

/// `sub_824CBAC0`: owner input 5, the slewed rate of change of `vfunc52(0)` between the last two
/// updates. `f31 = min(|[1892] − [1896]| / dt, max)`, moved from `previous` (`+1900`) by at most
/// `step·dt`, then `fctiwz(f31 / max × 32767)` clamped to 0..32767. `max` = `0x780F5C816E00BDFC`
/// (10000, also what `sub_824ADE30` returns), `step` = `0x68756CFEB1FF3428` (3000), class
/// `0xC1831BDB6CB1B1EA` key `0x1C20B475CE218459`. Returns the new `+1900` and the input.
pub(crate) fn heading_rate(
    previous: f32,
    raw_1892: u32,
    raw_1896: u32,
    dt: f32,
    max: f32,
    step: f32,
) -> (f32, u32) {
    let delta = (raw_1892 as i32)
        .wrapping_sub(raw_1896 as i32)
        .wrapping_abs();
    let mut f31 = delta as f32 / dt;
    if f31 > max {
        f31 = max;
    }
    let limit = step * dt;
    if f31 > previous {
        if f31 - previous > limit {
            f31 = previous + limit;
        }
    } else if f31 < previous && previous - f31 > limit {
        f31 = previous - limit;
    }
    (f31, clamp(fctiwz(f31 / max * LEVEL), 32_767))
}

impl Component for Board {
    /// `sub_824C6A78`. Retail gates both ticks on `[[owner+28]+52]`; the host only ticks an
    /// enabled component.
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        self.slope_inputs(tick.audio.pump_absorption_712);
        self.route_surfaces(tick)?;
        self.push_and_rattle(tick)?;
        self.push_chains(tick)?;
        self.skid_trigger(tick)?;
        self.squeak_trigger(tick)?;
        self.crack_layer(tick)?;
        self.seam_envelope(tick)?;
        self.distortion_send(tick)?;
        self.gain_wobble(tick)?;
        self.board_slide_trigger(tick)?;
        // sub_824CB828 runs here for non-local skaters only (not ported, see the module note).
        let (f1900, input) = heading_rate(
            self.f1900,
            self.raw_1892,
            self.raw_1896,
            tick.dt,
            self.vault.heading_max,
            self.vault.heading_step,
        );
        self.f1900 = f1900;
        self.set_input(5, input);
        Ok(())
    }

    /// The owner inputs (ids 0..=6) `process` wrote, in retail call order.
    fn take_owner_inputs(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.owner_inputs)
    }

    /// `sub_824C6BD8`.
    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        let audio = tick.audio;
        let c = tick.controls;
        // The vfunc52(0) history sub_824CBAC0 reads. The same value, × 360/65536, is also stored
        // into the global manager's +124/+160 (not ported).
        self.raw_1896 = self.raw_1892;
        self.raw_1892 = c.raw(0);
        for t in 0..2 {
            if !self.routing.live[t] {
                continue;
            }
            if !self.routing.grain_surface[t] {
                let selector = self.trucks[t].selector as i32;
                let speed = self.scaled_speed_word(audio, selector);
                if let Some((handle, words)) = self.trucks[t].rolling.as_mut() {
                    surface_rolling_update(words, c, speed, selector, audio);
                    redeliver(tick.runtime, *handle, words)?;
                }
                continue;
            }
            if !self.routing.grains[t] {
                continue;
            }
            let special = self.special(audio);
            let tuning = self.tuning_of(t).clone();
            let primary = self.tuning_of(self.routing.primary).clone();
            let input = BoardInputs {
                speed: audio.ground_speed_208,
                speed_scale: (!self.scale_envelope.idle).then_some(self.scale_envelope.value),
                mix_gain_a: c.level(1) as i32,
                mix_gain_b: c.level(2) as i32,
                mix_pitch: c.pitch(3),
                f1164: self.f1164,
                f1168: self.f1168,
                f1456: self.seam_active.then_some(self.seam_envelope.value),
                special,
                boost: self.f1508,
                boost_gain: primary.boost_gain,
                boost_kmh: primary.boost_kmh,
            };
            let [a, b]: [GrainRecord; 2] = grain_board::board_records(&tuning, &input);
            super::super::trace::grain_records(
                t,
                (a.gain, a.pitch, a.position),
                (b.gain, b.pitch, b.position),
            );
            let [pa, pb] = self.trucks[t].players;
            let mut grains = tick.runtime.grains();
            grains.set_record(pa, a).map_err(|e| e.to_string())?;
            grains.set_record(pb, b).map_err(|e| e.to_string())?;
        }
        self.skid_updater(tick)?;
        if let Some((handle, words)) = self.squeak.as_mut() {
            squeak_update(words, c, audio, self.vault.squeak_divisor);
            redeliver(tick.runtime, *handle, words)?;
        }
        if let Some((handle, words)) = self.rattle.as_mut() {
            rattle_update(words, c);
            redeliver(tick.runtime, *handle, words)?;
        }
        let surface = self.layer_surface(audio);
        let speed0 = speed_word(audio.ground_speed_208, self.vault.rolling_kmh(0));
        let speed3 = speed_word(audio.ground_speed_208, self.vault.rolling_kmh(3));
        for (layer, gain_id, speed) in [(0usize, 7u32, speed0), (3, 9, speed3)] {
            let held = if layer == 0 {
                &mut self.layer0
            } else {
                &mut self.layer3
            };
            if let Some((handle, words)) = held.as_mut() {
                held_rolling_update(words, c, gain_id, speed, surface, audio);
                redeliver(tick.runtime, *handle, words)?;
            }
        }
        let speed5 = self.scaled_speed_word(audio, 5);
        if let Some((handle, words)) = self.layer5.as_mut() {
            held_rolling_update(words, c, 10, speed5, surface, audio);
            redeliver(tick.runtime, *handle, words)?;
        }
        // The seam envelope's next segment (sub_824C6BD8's tail).
        if self.seam_active && self.seam_envelope.idle {
            let (gain, ms) = self.seam_random();
            self.seam_envelope.reset();
            if self.seam_target < 1.0 {
                self.seam_envelope.add(self.seam_target, 1.0, ms);
                self.seam_target = 1.0;
            } else {
                self.seam_envelope.add(1.0, gain, ms);
                self.seam_target = gain;
            }
        }
        if let Some((handle, words)) = self.board_slide.as_mut() {
            board_slide_update(words, c, audio, &self.vault);
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::capture::{self, Captured, Matches};
    use super::*;
    use std::collections::{BTreeMap, HashMap};

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

    /// The vault's values (`skater-collections.json`), without its grain tunings. The surface
    /// map is the vault's for materials 0 (rolling 2, skid 1), 1 (rolling 9, skid 3) and 94
    /// (rolling 3, skid 0).
    fn vault() -> BoardVault {
        let mut entries = vec![[0u32; 18]; 95];
        entries[0][1] = 2;
        entries[0][3] = 1;
        entries[1][1] = 9;
        entries[1][3] = 3;
        entries[94][1] = 3;
        let wobble = |gain_low, gain_high, ms_low, ms_high| Wobble {
            gain_low,
            gain_high,
            ms_low,
            ms_high,
        };
        let mut seam = [wobble(1.0, 1.0, 0, 0); 16];
        seam[1] = wobble(0.8, 0.6, 30, 80);
        BoardVault {
            surfaces: SurfaceMap::from_entries(entries),
            rolling_kmh: vec![70.0, 65.0, 65.0, 70.0, 100.0, 45.0, 45.0, 45.0],
            tunings: HashMap::new(),
            rattle_kmh: HashMap::from([(DEFAULT_KEY, 30.0)]),
            slope: HashMap::from([(DEFAULT_KEY, (-10.0, 10.0))]),
            squeak_threshold: 15,
            squeak_divisor: 1.5,
            skid_gain: 23_000,
            skid_level: 32_767,
            rattle_tweak: 8,
            skid_tweak: 5,
            squeak_tweak: 0,
            slide_tweak: 7,
            slide_kmh: 15.0,
            slide_level: 15_000,
            slide_level_loose: 15_000,
            heading_max: 10_000.0,
            heading_step: 3_000.0,
            chain: ChainConfig {
                local: true,
                create: 0,
                eq_chain: 8,
                clip: f32::from_bits(0x3DB8_51EC),
                shelf_corner: 5000.0,
                shelf_gain: f32::from_bits(0x3F26_6666),
                local_send: 0.0,
            },
            send_ramp: [46.0, 70.0, 3.0, 0.0],
            level_ramp: [52.0, 74.0, f32::from_bits(0x3EE6_6666)],
            wobble_ramp: [42.0, 70.0],
            wobbles: [
                wobble(0.0, f32::from_bits(0x3E99_999A), 15, 30),
                wobble(f32::from_bits(0x3DCC_CCCD), 0.25, 5, 15),
            ],
            seam,
        }
    }

    #[test]
    fn title_rng_is_the_guest_generator() {
        use skate_audio_core::grain::rng;
        let mut g = skate_audio_core::Guest::single(rng::STATE & !0xFFF, 0x2000);
        for (i, w) in RNG_SEED.iter().enumerate() {
            g.set_u32(rng::STATE + 4 * i as u32, *w).unwrap();
        }
        let mut ours = TitleRng(RNG_SEED);
        for _ in 0..1000 {
            assert_eq!(ours.next(), rng::next(&mut g).unwrap());
        }
    }

    fn on_ground(material: u32) -> AudioState {
        let mut s = AudioState::default();
        s.wheel_material_620 = [material; 4];
        s.wheel_landed_464 = [true; 4];
        s.wheel_count_200 = 4;
        s
    }

    #[test]
    fn surfaces_follow_the_map_and_the_default_policy() {
        let v = vault();
        let s = on_ground(1);
        assert_eq!(truck_surface(&v, SurfacePolicy::Retail, false, 0, &s, 0), 9);
        assert_eq!(
            truck_surface(&v, SurfacePolicy::Default(2), false, 0, &s, 0),
            2
        );
        // 143 and above: 3. Above 93: element 94.
        assert_eq!(
            truck_surface(&v, SurfacePolicy::Retail, false, 0, &on_ground(143), 0),
            3
        );
        assert_eq!(
            truck_surface(&v, SurfacePolicy::Retail, false, 0, &on_ground(120), 0),
            3
        );
        // Grinding, or the manual latch with the wheel up: 14 (kept by the default policy).
        let mut grinding = on_ground(1);
        grinding.grinding_341 = true;
        assert_eq!(
            truck_surface(&v, SurfacePolicy::Default(2), false, 0, &grinding, 0),
            14
        );
        let mut manual = on_ground(0);
        manual.wheel_landed_464 = [false, true, true, true];
        assert_eq!(
            truck_surface(&v, SurfacePolicy::Retail, true, 0, &manual, 0),
            14
        );
        assert_eq!(
            truck_surface(&v, SurfacePolicy::Retail, true, 0, &manual, 1),
            2
        );
        // The non-primary truck reads wheel 3.
        assert_eq!(
            truck_surface(&v, SurfacePolicy::Retail, true, 1, &manual, 0),
            2
        );
        assert_eq!(
            layer_surface(&v, SurfacePolicy::Retail, true, 0, &manual),
            Some(2)
        );
        assert_eq!(
            layer_surface(&v, SurfacePolicy::Retail, false, 0, &grinding),
            Some(13)
        );
    }

    #[test]
    fn routing_starts_one_truck_and_hands_over_on_a_manual() {
        let mut r = Routing::default();
        let steps = r.route(true, &mut |_, _| 2);
        assert_eq!(
            steps,
            [
                RouteStep::Start {
                    truck: 0,
                    surface: 2,
                    sound: true
                },
                RouteStep::Start {
                    truck: 1,
                    surface: 2,
                    sound: false
                },
            ]
        );
        assert_eq!(
            (r.live, r.grains, r.primary),
            ([true, false], [true, false], 0)
        );
        // The primary truck lifts: the other has no live sound, so [1500] flips and nothing stops.
        let steps = r.route(true, &mut |truck, primary| {
            if truck == primary { 14 } else { 2 }
        });
        assert_eq!(steps, [RouteStep::Pulse]);
        assert_eq!((r.primary, r.surface, r.live), (1, [2, 14], [true, false]));
        // A non-local skater stops its truck instead.
        let mut r = Routing::default();
        r.route(false, &mut |_, _| 2);
        let steps = r.route(false, &mut |_, _| 7);
        assert_eq!(
            steps,
            [
                RouteStep::Pulse,
                RouteStep::Stop {
                    truck: 0,
                    grain_surface: true,
                    grains: true
                },
                RouteStep::Start {
                    truck: 0,
                    surface: 7,
                    sound: true
                },
            ]
        );
        assert_eq!(
            (r.grain_surface[0], r.grains[0], r.live[0]),
            (false, false, true)
        );
        assert_eq!(rolling_selector(7), Some(1));
        assert_eq!(rolling_selector(12), Some(11));
        assert_eq!(rolling_selector(9), None);
    }

    #[test]
    fn skid_predicate_and_counter() {
        let mut s = AudioState::default();
        s.slip_232 = 0.5;
        s.audio_trick_348 = u32::MAX;
        assert!(skid_predicate(&s, 0));
        s.audio_trick_348 = 35;
        assert!(skid_predicate(&s, 0));
        // While slipping, a trick id other than −1/35 is false whatever +690 and the counter say.
        s.audio_trick_348 = 7;
        s.revert_690 = true;
        assert!(!skid_predicate(&s, 5));
        s.grinding_341 = true;
        s.grind_family_192 = 4;
        assert!(skid_predicate(&s, 0));
        s.grind_family_192 = 3;
        assert!(!skid_predicate(&s, 0));
        s.grinding_341 = false;
        s.audio_trick_348 = u32::MAX;
        s.walking_716 = true;
        s.board_held_308 = true;
        assert!(!skid_predicate(&s, 0));
        // No slip: +690 or the counter.
        s.slip_232 = 0.0;
        assert!(skid_predicate(&s, 0));
        s.revert_690 = false;
        assert!(!skid_predicate(&s, 0));
        assert!(skid_predicate(&s, 5));
        // An unordered slip takes the slipping branch (`fcmpu ; ble`).
        s.slip_232 = f32::NAN;
        assert!(!skid_predicate(&s, 5));
        assert_eq!(skid_counter_step(43, true), 45);
        assert_eq!(skid_counter_step(40, false), 25);
        assert_eq!(skid_counter_step(10, false), 0);
        assert_eq!(skid_counter_step(0, false), 0);
    }

    #[test]
    fn squeak_gate_and_sign() {
        let mut s = AudioState::default();
        s.foot_in_deck_box_615 = true;
        s.foot_in_deck_box_616 = true;
        s.wheel_count_200 = 2;
        s.deck_tilt_264 = -0.14;
        // 0.14 × 114.5916 = 16.04 → 16 ≥ 15.
        assert_eq!(squeak_gate(&s, 15), Some(true));
        s.deck_tilt_264 = 0.13;
        assert_eq!(squeak_gate(&s, 15), Some(false));
        s.wheel_count_200 = 1;
        assert_eq!(squeak_gate(&s, 15), None);
        let mut sign = true;
        assert_eq!(squeak_step(-0.2, &mut sign, true), (true, true));
        assert!(!sign);
        assert_eq!(squeak_step(-0.2, &mut sign, true), (false, false));
        assert_eq!(squeak_step(0.2, &mut sign, false), (false, true));
        assert_eq!(squeak_turn(0.06, 1.5), 0);
        assert_eq!(squeak_turn(-0.75, 1.5), 500);
        assert_eq!(squeak_turn(3.0, 1.5), 1000);
    }

    #[test]
    fn heading_rate_slews_toward_the_capped_rate() {
        // |100 − 0| / 0.5 = 200, within the 1500 step.
        assert_eq!(
            heading_rate(0.0, 100, 0, 0.5, 10_000.0, 3_000.0),
            (200.0, 655)
        );
        // |1000 − 0| / 0.25 = 4000, slewed from 0 by 3000 × 0.25 = 750: fctiwz(750 / 10000 × 32767).
        assert_eq!(
            heading_rate(0.0, 0, 1000, 0.25, 10_000.0, 3_000.0),
            (750.0, 2457)
        );
        // A wrap of the raw pan saturates at the cap.
        let (f, word) = heading_rate(9_990.0, 0xFFF0, 0x10, 0.25, 10_000.0, 3_000.0);
        assert_eq!((f, word), (10_000.0, 32_767));
    }

    /// Rows from the retail recomp capture (2026-09-18, local board): the previous words, the
    /// updater's controller reads and the audio state one frame earlier, then retail's words.
    #[test]
    fn updaters_reproduce_pinned_capture_rows() {
        let mut s = AudioState::default();
        // Frame 2907, held layer 3 (sub_824C9948); w6 is sub_824C82A8's surface 2.
        s.ground_speed_208 = f32::from_bits(0x3B2C_5C40);
        let mut w = [0x7FFF, 5, 0xFF6, 1, 3, 0, 2, 0, 0, 0x618B, 0x4D, 9];
        let reads = Fixed(&[
            (60, 9, 0xC),
            (52, 0, 3),
            (56, 8, 0xFF6),
            (60, 19, 0),
            (60, 17, 0x618B),
            (60, 18, 0x4D),
        ]);
        held_rolling_update(
            &mut w,
            &reads,
            9,
            speed_word(s.ground_speed_208, 70.0),
            Some(2),
            &s,
        );
        assert_eq!(w, [0x7FFF, 3, 0xFF6, 1, 3, 0, 2, 0, 0, 0x618B, 0x4D, 0xC]);

        // Frame 3916, rattle (sub_824C80C0).
        let mut w = [
            0x7FFF, 0x18D, 0x11C3, 0x1DA4, 3, 0, 1, 0, 0x618B, 0x4D, 0, 8,
        ];
        let reads = Fixed(&[
            (56, 3, 0x11CB),
            (60, 6, 0),
            (52, 0, 0x1D6),
            (60, 16, 0),
            (60, 14, 0x618B),
            (60, 15, 0x4D),
        ]);
        rattle_update(&mut w, &reads);
        assert_eq!(
            w,
            [
                0x7FFF, 0x1D6, 0x11CB, 0x1DA4, 3, 0, 1, 0, 0x618B, 0x4D, 0, 8
            ]
        );

        // Frame 3218, skid (sub_824C7A20), counter 0, skid surface 0.
        s.ground_speed_208 = f32::from_bits(0x3C30_54B0);
        s.slip_232 = f32::from_bits(0x3C88_B326);
        let mut w = [
            0x7FFF, 0x48, 0xA1E, 0x33, 0xBDB, 0x618B, 0x4D, 8, 0, 0, 1, 0x59D8, 0x7FFF, 0, 1, 1,
            0x72D, 5,
        ];
        let reads = Fixed(&[
            (56, 3, 0xBD4),
            (60, 4, 0x51),
            (52, 0, 0x33),
            (60, 13, 0xA1E),
            (60, 11, 0x618B),
            (60, 12, 0x4D),
            (60, 20, 0x72D),
        ]);
        skid_update(&mut w, &reads, &s, 0, 0, true);
        assert_eq!(
            w,
            [
                0x7FFF, 0x51, 0xA1E, 0x33, 0xBD4, 0x618B, 0x4D, 8, 0, 0, 1, 0x59D8, 0x7FFF, 0, 1,
                1, 0x72D, 5
            ]
        );

        // Frame 4946, squeaks (sub_824C7DD0).
        s.ground_speed_208 = f32::from_bits(0x409F_F2C3);
        s.deck_angular_velocity_480 = [0.0, 0.0, f32::from_bits(0x3E21_EEEA)];
        let mut w = [
            0x7FFF, 0x907, 0, 0x21B, 0xFAE, 0x618B, 0x4D, 0x18F, 0xF, 0x55, 0,
        ];
        let reads = Fixed(&[
            (56, 3, 0xFC0),
            (60, 5, 0x907),
            (60, 11, 0x618B),
            (60, 12, 0x4D),
            (52, 0, 0x1ED),
        ]);
        squeak_update(&mut w, &reads, &s, 1.5);
        assert_eq!(
            w,
            [
                0x7FFF, 0x907, 0, 0x1ED, 0xFC0, 0x618B, 0x4D, 0x18F, 0xF, 0x69, 0
            ]
        );

        // Frame 4190, loose-board scrape (sub_824CB4C0), +780 = 2.
        s.ground_speed_208 = f32::from_bits(0x40BD_1B84);
        s.loose_board_780 = 2;
        let mut w = [0, 0, 0x1000, 0, 0x61A8, 0, 0, 0, 7, 0, 1, 0];
        let reads = Fixed(&[
            (52, 0, 0x3255),
            (56, 23, 0xFEA),
            (60, 25, 0x5A6F),
            (60, 26, 0x4D),
            (60, 27, 0xF04),
            (60, 24, 0x234A),
        ]);
        board_slide_update(&mut w, &reads, &s, &vault());
        assert_eq!(
            w,
            [
                0x7FFF, 0x3255, 0xFEA, 0x2710, 0x5A6F, 0x4D, 0xF04, 0x234A, 7, 0, 1, 0x3A98
            ]
        );
    }

    #[test]
    fn packets_have_the_retail_layouts() {
        assert_eq!(
            rolling_post(1234, 0, 3),
            [0, 0, 4096, 1234, 0, 0, 3, 0, 0, 25_000, 0, 32_767]
        );
        assert_eq!(
            rattle_post(20_000, 2, 8),
            [0, 0, 4096, 10_000, 2, 0, 1, 0, 25_000, 0, 32_767, 8]
        );
        assert_eq!(
            squeak_post(12, 600, 0),
            [0, 32_767, 0, 0, 4096, 25_000, 0, 12, 15, 600, 0]
        );
        assert_eq!(
            board_slide_post(7, 1),
            [0, 0, 4096, 0, 25_000, 0, 0, 0, 7, 0, 1, 0]
        );
        let skid = skid_post(314, 0, 2, 2, 23_000, 32_767, 0, 1, 1, 0x6B5, 5);
        // The capture's first local skid post (frame 2718).
        assert_eq!(
            skid,
            [
                0, 32_767, 0, 0, 0, 25_000, 0, 0x13A, 0, 2, 2, 0x59D8, 0x7FFF, 0, 1, 1, 0x6B5, 5
            ]
        );
        assert_eq!(speed_word(10.0, 70.0), 5142);
    }

    /// The local SkateBoard controller in the capture (board `0x40C3B020`, state `0x40C10E20`).
    const LOCAL: &str = "4A26A8A0";

    /// Return-address ranges of the owner functions whose controller reads each packet uses.
    const HELD_LAYERS: std::ops::Range<u32> = 0x824C_9948..0x824C_9F68;
    const RATTLE_UPDATE: std::ops::Range<u32> = 0x824C_80C0..0x824C_82A8;
    const SKID_UPDATE: std::ops::Range<u32> = 0x824C_7A20..0x824C_7DD0;
    const SQUEAK_UPDATE: std::ops::Range<u32> = 0x824C_7DD0..0x824C_80C0;
    const SLIDE_UPDATE: std::ops::Range<u32> = 0x824C_B4C0..0x824C_B828;
    const SKID_TRIGGER: std::ops::Range<u32> = 0x824C_7438..0x824C_7738;

    /// One captured packet instance: its post, the controller of its first update, its release.
    struct Packet {
        post: u32,
        words: Vec<u32>,
        local: Option<bool>,
        release: Option<u32>,
    }

    /// The packets of one object, and every local update row.
    fn packets(rows: &[capture::Row]) -> (Vec<Packet>, Vec<(usize, capture::Row)>, usize) {
        let mut packets: Vec<Packet> = Vec::new();
        let mut by_payload: HashMap<String, usize> = HashMap::new();
        let mut by_node: HashMap<String, usize> = HashMap::new();
        let mut updates = Vec::new();
        let mut stray = 0;
        for row in rows {
            match row.kind.as_str() {
                "PO" => {
                    by_payload.insert(row.payload.clone(), packets.len());
                    packets.push(Packet {
                        post: row.frame,
                        words: row.words.clone(),
                        local: None,
                        release: None,
                    });
                }
                "UP" => {
                    let Some(&index) = by_payload.get(&row.payload) else {
                        stray += 1;
                        continue;
                    };
                    let packet = &mut packets[index];
                    if packet.local.is_none() {
                        packet.local = Some(row.ctrl == LOCAL);
                    }
                    by_node.insert(row.node.clone(), index);
                    if packet.local == Some(true) {
                        updates.push((index, row.clone()));
                    }
                }
                "RL" => match by_node.remove(&row.node) {
                    Some(index) => packets[index].release = Some(row.frame),
                    None => stray += 1,
                },
                _ => {}
            }
        }
        (packets, updates, stray)
    }

    /// Multiset agreement of event frames: (retail, ours, same frame).
    fn agreement(retail: &[u32], ours: &[u32]) -> (usize, usize, usize) {
        let mut count: BTreeMap<u32, i32> = BTreeMap::new();
        for f in retail {
            *count.entry(*f).or_default() += 1;
        }
        let mut same = 0;
        for f in ours {
            if let Some(c) = count.get_mut(f) {
                if *c > 0 {
                    *c -= 1;
                    same += 1;
                }
            }
        }
        (retail.len(), ours.len(), same)
    }

    fn first_differences(retail: &[u32], ours: &[u32]) -> Vec<(Option<u32>, Option<u32>)> {
        let r: std::collections::BTreeSet<u32> = retail.iter().copied().collect();
        let o: std::collections::BTreeSet<u32> = ours.iter().copied().collect();
        r.symmetric_difference(&o)
            .take(6)
            .map(|f| (r.contains(f).then_some(*f), o.contains(f).then_some(*f)))
            .collect()
    }

    /// Replays the retail recomp capture through the component's pure pieces. Retail order in a
    /// frame F is update(F) on state F−1, then the bridge, then process(F) on state F. The
    /// replay steps the same bookkeeping (routing, latch, skid counter, squeak sign, held
    /// packets), compares every local update's words (previous words from the capture, controller
    /// reads from the capture), every local post's words, post and release frames, and the owner
    /// inputs against the controller's logged input table.
    #[test]
    #[ignore = "needs the retail capture in .local and the vault"]
    fn board_replays_the_retail_capture() {
        let Some(root) = capture::root() else { return };
        let assets = std::path::PathBuf::from(
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets",
        );
        let vault = BoardVault::load(&assets).expect("vault");
        let words = capture::states(&root);
        let states: BTreeMap<u32, AudioState> = words
            .iter()
            .map(|(f, w)| (*f, AudioState::from_capture(w)))
            .collect();
        let first = *states.keys().next().unwrap();
        let last = *states.keys().last().unwrap();
        let clock = |f: u32| words.get(&f).map(|w| capture::float(w, 312));

        let objects = [ROLLING, RATTLE, SKID, SQUEAKS, BOARD_SLIDE];
        let mut all: HashMap<&str, (Vec<Packet>, Vec<(usize, capture::Row)>, usize)> =
            HashMap::new();
        for object in objects {
            all.insert(object, packets(&capture::rows(&root, object)));
        }
        // The raw vfunc52(0) sub_824C6BD8 reads at its top (lr 0x824C6C14), per frame, from the
        // full controller-read log (the attributed rows carry it only when it precedes a packet).
        let mut raw_top: BTreeMap<u32, u32> = BTreeMap::new();
        {
            use std::io::BufRead;
            let file = std::fs::File::open(root.join("vf.tsv")).unwrap();
            for line in std::io::BufReader::new(file).lines() {
                let line = line.unwrap();
                if !line.ends_with("824C6C14") {
                    continue;
                }
                let mut columns = line.split('\t');
                let frame: u32 = columns.next().unwrap().parse().unwrap();
                let call: Vec<&str> = columns.nth(1).unwrap().split(' ').collect();
                if call[0] == "52" && call[2] == LOCAL && call[3] == "0" {
                    raw_top.insert(frame, u32::from_str_radix(call[4], 16).unwrap());
                }
            }
        }
        // Controller reads per frame for the skid trigger's level(20).
        let mut skid_reads: HashMap<u32, Captured> = HashMap::new();
        for object in objects {
            for row in capture::rows(&root, object) {
                if row.reads.iter().any(|r| SKID_TRIGGER.contains(&r.3)) {
                    skid_reads.insert(row.frame, Captured::from_reads(&row.reads, SKID_TRIGGER));
                }
            }
        }
        // The controller's logged inputs per frame.
        let mut inputs_log: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for line in std::fs::read_to_string(root.join("mixmap").join(format!("{LOCAL}.tsv")))
            .unwrap()
            .lines()
        {
            let frame: u32 = line.split('\t').next().unwrap().parse().unwrap();
            let table = line.split(" | ").nth(1).unwrap();
            inputs_log.insert(
                frame,
                table
                    .split(' ')
                    .map(|w| u32::from_str_radix(w, 16).unwrap())
                    .collect(),
            );
        }

        // --- the simulation: events and owner inputs per frame.
        let policy = SurfacePolicy::Retail;
        let mut routing = Routing::default();
        let mut latch = false;
        let mut counter = 0i32;
        let mut skid_held = false;
        let mut squeak_held = false;
        let mut squeak_sign = false;
        let mut slide_held = false;
        let mut layer5_held = false;
        let mut rattle_held = false;
        let mut key_760 = DEFAULT_KEY;
        let mut keys = [Some(0x382B_1263_6ED9_D8DAu64), None];
        let (mut raw_1892, mut raw_1896, mut f1900) = (0u32, 0u32, 0.0f32);
        let mut ours: HashMap<(&str, &str), Vec<u32>> = HashMap::new();
        let mut counters: BTreeMap<u32, i32> = BTreeMap::new();
        let mut routings: BTreeMap<u32, (Routing, bool)> = BTreeMap::new();
        let mut posts_ours: HashMap<(&str, u32), Vec<u32>> = HashMap::new();
        let mut inputs_ours: BTreeMap<u32, [Option<u32>; 7]> = BTreeMap::new();
        for frame in first + 1..=last {
            let (Some(before), Some(now)) = (states.get(&(frame - 1)), states.get(&frame)) else {
                continue;
            };
            // update(F), state F−1.
            if skid_held {
                if !skid_predicate(before, counter) {
                    skid_held = false;
                    ours.entry((SKID, "RL")).or_default().push(frame);
                } else {
                    counter = skid_counter_step(counter, before.revert_690);
                }
            }
            counters.insert(frame, counter);
            latch = grain_board::manual_latch(latch, before.balance_340, before.wheel_count_200);
            routings.insert(frame, (routing.clone(), latch));
            if let Some(&raw) = raw_top.get(&frame) {
                raw_1896 = raw_1892;
                raw_1892 = raw;
            }
            // process(F), state F.
            let a = now;
            let mut set = [None; 7];
            // sub_824CA738 runs before the routing, on +712 (not an AudioState field yet).
            let slope = capture::float(&words[&frame], 712);
            let key = keys[routing.primary].unwrap_or(DEFAULT_KEY);
            let levels = slope_levels(
                slope,
                routing.grain_surface[routing.primary],
                vault.slope(key),
            );
            let [down, up] = slope_words(levels);
            set[2] = Some(down);
            set[3] = Some(up);
            let mut l = latch;
            let steps = routing.route(true, &mut |truck, primary| {
                l = grain_board::manual_latch(l, a.balance_340, a.wheel_count_200);
                truck_surface(&vault, policy, l, primary, a, truck)
            });
            latch = l;
            set[0] = Some(0);
            for step in &steps {
                match *step {
                    RouteStep::Pulse => set[0] = Some(32_767),
                    RouteStep::Stop {
                        grain_surface: false,
                        ..
                    } => ours.entry((ROLLING, "RL")).or_default().push(frame),
                    RouteStep::Stop { .. } => {}
                    RouteStep::Start {
                        truck,
                        surface,
                        sound,
                    } => {
                        let soft = a.soft_wheels_684 != 0;
                        let key = grain_board::grain_for_surface(surface, soft)
                            .map_or(DEFAULT_KEY, |c| c.key);
                        key_760 = key;
                        keys[truck] = Some(key);
                        if sound && rolling_selector(surface).is_some() {
                            ours.entry((ROLLING, "PO")).or_default().push(frame);
                        }
                    }
                }
            }
            let metal = truck_surface(&vault, policy, latch, routing.primary, a, 0) == 9;
            set[6] = Some(if metal { 32_767 } else { 0 });
            set[4] = Some(if a.push_left_333 || a.push_right_334 {
                32_767
            } else {
                0
            });
            if a.push_plant_335 {
                if rattle_held {
                    ours.entry((RATTLE, "RL")).or_default().push(frame);
                    rattle_held = false;
                }
                if routing.grain_surface[routing.primary] {
                    let key = keys[routing.primary].unwrap_or(DEFAULT_KEY);
                    let code = match key_760 {
                        0x7C59_12FC_2DAB_F98C => 1,
                        0x0372_1D0F_A99A_03C8 => 2,
                        0xFFB5_E3E6_2E0B_4943 => 3,
                        0x7947_A259_F181_FDB4 => 4,
                        0xB303_AED8_2415_30E2 => 5,
                        _ => 0,
                    };
                    let speed = rattle_speed(a.ground_speed_208, vault.rattle_kmh(key)) as i32;
                    posts_ours.insert(
                        (RATTLE, frame),
                        rattle_post(speed, code, vault.rattle_tweak).to_vec(),
                    );
                    ours.entry((RATTLE, "PO")).or_default().push(frame);
                    rattle_held = true;
                }
            }
            let pred = skid_predicate(a, counter);
            set[1] = Some(if pred { 32_767 } else { 0 });
            if pred && !skid_held {
                skid_held = true;
                ours.entry((SKID, "PO")).or_default().push(frame);
                let level20 = skid_reads.get(&frame).map_or(0, |c| c.level(20) as i32);
                posts_ours.insert(
                    (SKID, frame),
                    skid_post(
                        slow_speed(a.ground_speed_208, TEN_THOUSAND),
                        i32::from(a.soft_wheels_684 != 0),
                        skid_surface(&vault, a),
                        fctiwz(a.slip_232 * NINETY),
                        vault.skid_gain,
                        vault.skid_level,
                        0,
                        1,
                        1,
                        level20,
                        vault.skid_tweak,
                    )
                    .to_vec(),
                );
            }
            match squeak_gate(a, vault.squeak_threshold) {
                None => {
                    if squeak_held {
                        squeak_held = false;
                        ours.entry((SQUEAKS, "RL")).or_default().push(frame);
                    }
                }
                Some(false) => {}
                Some(true) => {
                    let (drop, start) = squeak_step(a.deck_tilt_264, &mut squeak_sign, squeak_held);
                    if drop {
                        ours.entry((SQUEAKS, "RL")).or_default().push(frame);
                    }
                    if start {
                        squeak_held = true;
                        ours.entry((SQUEAKS, "PO")).or_default().push(frame);
                        posts_ours.insert(
                            (SQUEAKS, frame),
                            squeak_post(
                                slow_speed(a.ground_speed_208, THOUSAND),
                                squeak_turn(a.deck_angular_velocity_480[2], vault.squeak_divisor),
                                vault.squeak_tweak,
                            )
                            .to_vec(),
                        );
                    }
                }
            }
            let mut pattern = a.wheel_seam_636[0];
            if latch && !a.wheel_landed_464[0] {
                pattern = a.wheel_seam_636[3];
            }
            if !layer5_held && pattern == 1 {
                layer5_held = true;
                ours.entry((ROLLING, "PO")).or_default().push(frame);
            } else if layer5_held && pattern != 1 {
                layer5_held = false;
                ours.entry((ROLLING, "RL")).or_default().push(frame);
            }
            match (slide_held, a.loose_board_780) {
                (false, 0) | (true, 1..) => {}
                (false, loose) => {
                    slide_held = true;
                    ours.entry((BOARD_SLIDE, "PO")).or_default().push(frame);
                    posts_ours.insert(
                        (BOARD_SLIDE, frame),
                        board_slide_post(vault.slide_tweak, i32::from(loose == 2)).to_vec(),
                    );
                }
                (true, 0) => {
                    slide_held = false;
                    ours.entry((BOARD_SLIDE, "RL")).or_default().push(frame);
                }
            }
            if let (Some(c1), Some(c0)) = (clock(frame), clock(frame - 1)) {
                let (f, input) = heading_rate(
                    f1900,
                    raw_1892,
                    raw_1896,
                    c1 - c0,
                    vault.heading_max,
                    vault.heading_step,
                );
                f1900 = f;
                set[5] = Some(input);
            }
            inputs_ours.insert(frame, set);
        }

        // --- timing.
        println!(
            "post/release timing (retail events, ours, same frame), frames {}..={last}",
            first + 1
        );
        for object in objects {
            let (packets, _, stray) = &all[object];
            let in_range = |f: &u32| *f > first && *f <= last;
            let posts: Vec<u32> = packets
                .iter()
                .filter(|p| p.local == Some(true))
                .map(|p| p.post)
                .filter(in_range)
                .collect();
            let releases: Vec<u32> = packets
                .iter()
                .filter(|p| p.local == Some(true))
                .filter_map(|p| p.release)
                .filter(in_range)
                .collect();
            let unattributed = packets.iter().filter(|p| p.local.is_none()).count();
            for (kind, retail) in [("PO", &posts), ("RL", &releases)] {
                let ours = ours.get(&(object, kind)).cloned().unwrap_or_default();
                let (r, o, same) = agreement(retail, &ours);
                println!(
                    "  {object:<22} {kind}  retail {r:>4}  ours {o:>4}  same {same:>4}   first differences (retail, ours) {:?}",
                    first_differences(retail, &ours)
                );
            }
            println!(
                "  {object:<22}     packets never updated (unattributed): {unattributed}, stray rows {stray}"
            );
        }

        // --- post words.
        for object in [RATTLE, SKID, SQUEAKS, BOARD_SLIDE] {
            let n = match object {
                SKID => 18,
                SQUEAKS => 11,
                _ => 12,
            };
            let mut m = Matches::new(object, n);
            for p in all[object].0.iter().filter(|p| p.local == Some(true)) {
                if let Some(ours) = posts_ours.get(&(object, p.post)) {
                    m.add(p.post, &p.words[..n], ours);
                }
            }
            println!("post words, {object} (posts at frames both sides agree on)");
            m.print();
        }

        // --- update words.
        let mut held = Matches::new("Class_rolling held layers 0/3 (sub_824C9948)", 12);
        let mut rattle = Matches::new("Rolling_Rattle_Class (sub_824C80C0)", 12);
        let mut skid = Matches::new("Class_wheels_skid (sub_824C7A20)", 18);
        let mut squeak = Matches::new("Class_Squeaks (sub_824C7DD0)", 11);
        let mut slide = Matches::new("c_board_slide (sub_824CB4C0)", 12);
        let mut other_rolling = 0;
        for object in objects {
            let (packets, updates, _) = &all[object];
            let mut last_words: HashMap<usize, Vec<u32>> = HashMap::new();
            for (index, row) in updates {
                let frame = row.frame;
                let previous = last_words
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| packets[*index].words.clone());
                last_words.insert(*index, row.words.clone());
                let Some(audio) = states.get(&(frame - 1)) else {
                    continue;
                };
                match object {
                    ROLLING => {
                        let mut w: [u32; 12] = previous[..12].try_into().unwrap();
                        let layer = w[4];
                        let (gain, kmh_layer) = match layer {
                            0 => (7, 0),
                            3 => (9, 3),
                            _ => {
                                other_rolling += 1;
                                continue;
                            }
                        };
                        let c = Captured::from_reads(&row.reads, HELD_LAYERS);
                        let (r, l) = routings
                            .get(&frame)
                            .cloned()
                            .unwrap_or((Routing::default(), false));
                        let surface = layer_surface(&vault, policy, l, r.primary, audio);
                        let speed =
                            speed_word(audio.ground_speed_208, vault.rolling_kmh(kmh_layer));
                        held_rolling_update(&mut w, &c, gain, speed, surface, audio);
                        held.add(frame, &row.words[..12], &w);
                    }
                    RATTLE => {
                        let mut w: [u32; 12] = previous[..12].try_into().unwrap();
                        rattle_update(&mut w, &Captured::from_reads(&row.reads, RATTLE_UPDATE));
                        rattle.add(frame, &row.words[..12], &w);
                    }
                    SKID => {
                        let mut w: [u32; 18] = previous[..18].try_into().unwrap();
                        let count = counters.get(&frame).copied().unwrap_or(0);
                        let c = Captured::from_reads(&row.reads, SKID_UPDATE);
                        skid_update(&mut w, &c, audio, count, skid_surface(&vault, audio), true);
                        skid.add(frame, &row.words[..18], &w);
                    }
                    SQUEAKS => {
                        let mut w: [u32; 11] = previous[..11].try_into().unwrap();
                        let c = Captured::from_reads(&row.reads, SQUEAK_UPDATE);
                        squeak_update(&mut w, &c, audio, vault.squeak_divisor);
                        squeak.add(frame, &row.words[..11], &w);
                    }
                    _ => {
                        let mut w: [u32; 12] = previous[..12].try_into().unwrap();
                        let c = Captured::from_reads(&row.reads, SLIDE_UPDATE);
                        board_slide_update(&mut w, &c, audio, &vault);
                        slide.add(frame, &row.words[..12], &w);
                    }
                }
            }
        }
        for m in [&held, &rattle, &skid, &squeak, &slide] {
            m.print();
        }
        println!("other local Class_rolling updates (per-surface or layer 5): {other_rolling}");

        // --- owner inputs, against the logged table of the same frame and of the next.
        for lag in [0u32, 1] {
            let mut total = [0usize; 7];
            let mut exact = [0usize; 7];
            let mut bad: Vec<Vec<(u32, u32, u32)>> = vec![vec![]; 7];
            for (frame, set) in &inputs_ours {
                let Some(logged) = inputs_log.get(&(frame + lag)) else {
                    continue;
                };
                for id in 0..7 {
                    if let Some(v) = set[id] {
                        total[id] += 1;
                        if logged[id] == v {
                            exact[id] += 1;
                        } else if bad[id].len() < 4 {
                            bad[id].push((*frame, logged[id], v));
                        }
                    }
                }
            }
            println!("owner inputs vs the logged input table of frame F+{lag}:");
            for id in 0..7 {
                println!(
                    "  input {id}: {}/{}  bad (frame, retail, ours) {:?}",
                    exact[id], total[id], bad[id]
                );
            }
        }
        // Input 5 divides by dt, which the capture does not record: it is recovered from the
        // bridge clock (+312) difference, exact only to the clock's rounding.
        let mut spread = BTreeMap::<u32, usize>::new();
        for (frame, set) in &inputs_ours {
            if let (Some(v), Some(logged)) = (set[5], inputs_log.get(&(frame + 1))) {
                *spread.entry(logged[5].abs_diff(v).min(10)).or_default() += 1;
            }
        }
        println!("input 5 |retail − ours| (capped at 10) → frames: {spread:?}");
    }
}
