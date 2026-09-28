//! `CSTATEMGR_Collision` — the contact-sound subsystem `sub_82486EF0` posts to, and the sink the
//! grind onset and the continuous per-material impact level both need (`docs/engine-defects.md`
//! #9).
//!
//! Retail's audio is a three-level, data-driven object model (`docs/audio-object-registry.md`):
//! a `CSTATEMGR_<X>` manager owns N `CSTATE_<X>` slots, and each slot owns one `SFXObj_<Y>`
//! component. For collision, `CSTATEMGR_Collision::vtable[+8]` (`sub_824F17B8`) builds **exactly
//! ten** `CSTATE_Collision` slots on `[manager+16]`, each with one `SFXObj_Collision`.
//!
//! The delivery `sub_82486EF0` performs is two hops:
//!
//! 1. `[manager+668]->vtable[+12]` = `sub_824F1818`, a *router*: it walks the ten slots and keeps
//!    the one with the lowest signed `[node+64]`, calls that node's `vtable[+28]`
//!    (`sub_824F8B58`, which frees the message the slot was still holding) and returns it.
//! 2. that node's `vtable[+12]` = `sub_824F8990`, five instructions: stamp `[node+64]` from a
//!    global counter, park the message at `[node+68]`, activate the state.
//!
//! `[node+64]` being a *timestamp* is what makes the router an LRU: the slot that has gone
//! longest without a message is the one reused. [`CollisionStates`] is that manager.
//!
//! Two things come out of the component per frame:
//!
//! * `sub_824D1E00` (`SFXObj_Collision::vtable[+36]`) rewrites two controller inputs from zero
//!   every frame — a gate and a weight picked from the message's two tier words. That weight is
//!   retail's *continuous* landing dynamic, the part §9 of `player-audio-retail-drivers.md`
//!   identified as missing. See [`CollisionStates::drive`] and [`contact_weight`].
//! * `sub_824D1F68` starts up to two voices, one per material in the message, and `sub_824D2318`
//!   keeps their properties up to date. Which sample a material plays is read out of the
//!   controller's own outputs — `sub_824D20E8` and `sub_824D22B8` pick *which output id* from the
//!   material's category. See [`CollisionMaterials`].
//!
//! **The sample choice** is `sub_824965D0` → `sub_824967F8`, and it is a table, not code. The
//! image table at `0x8302D6E8` gives each material a *kind* word and the AttribSys key of its
//! record; the kind selects which family of fields on that record the sample comes from, and an
//! AttribSys field's retail type name **is** its bank — `Skate_Collisions` ->
//! `Skate_Collisions.bnk`, `Skate_Metal` -> `Skate_Metal.bnk`, `HOM_Set_1` -> `HOM_Set_1.bnk`.
//! Within a family, `(tier, the paired material's class)` picks the field. See [`KindFields`] and
//! [`CollisionMaterials::sample`].
//!
//! That is why a rail grind is two sounds and not one: the board's family base resolves out of
//! `Skate_Collisions.bnk` while the rail's own material resolves out of `Skate_Metal.bnk`.
//!
//! **One hole is left deliberately.** Kind 2's `tier == 0` against paired class 0 goes through
//! `sub_824825D0`, a different accessor that is not decoded, so that one combination resolves to
//! no sample rather than to a guess. Nothing here invents a sample.

use skate_data::collections::Collections;

use super::components::contacts::SurfaceMap;

use super::collision_materials::{MATERIAL_COUNT, MATERIAL_SOUND_ID, MATERIAL_VAULT_KEY};

/// `sub_824F17B8` loops exactly ten times.
pub(crate) const SLOT_COUNT: usize = 10;

/// The contact code's "no material" sentinel — the value the table at `0x8302D6E8` stops at, and
/// the one `sub_824D1F68` skips a voice record on.
pub(crate) const NO_MATERIAL: i32 = MATERIAL_COUNT as i32;

/// The message's "no tier" sentinel. `sub_824D1F68` skips a voice record whose tier word is 3, and
/// `sub_824D1E00` treats 3 as "take the other one" when it picks the weight.
pub(crate) const NO_TIER: i32 = 3;

/// `sub_82496FD0` returns 8 for a negative material.
const NEGATIVE_MATERIAL_CATEGORY: u8 = 8;

/// The category `sub_824D20E8` forces for `material >= 143`. Note `sub_824D22B8` does *not* use
/// this one — it takes the `category == 9` branch instead, so the two must stay separate.
const HIGH_MATERIAL_LEVEL_CATEGORY: u8 = 8;

/// Where a material with no vault record lands. `sub_82496FD0` pre-seeds its lookup with the
/// default record at `0x830D0850` and reads `+72` back whether or not the lookup replaced it; that
/// record is all zeroes in the image, so an unresolvable material reads category 0.
const DEFAULT_CATEGORY: u8 = 0;

/// `sub_824D20E8`'s jump table at `0x824D212C`, category `0..=9` → the controller output id it
/// reads the material's level from.
const CATEGORY_LEVEL_OUTPUT: [u32; 10] = [13, 14, 15, 16, 17, 18, 12, 19, 20, 21];

/// `sub_824D20E8`'s `cmplwi r3,9; bgt` arm: any category past the table falls here.
const LEVEL_OUTPUT_FALLBACK: u32 = 20;

/// `sub_824D22B8`: category 9 (or `material >= 143`) reads output 22, everything else output 1.
/// Both are the MixMap's `t1` (signed, `× 4096`) outputs — see the `Controls::pitch` accessor
/// `sub_824C5910`, which is the same function the component calls here.
const PITCH_OUTPUT_CATEGORY_9: u32 = 22;
const PITCH_OUTPUT_OTHER: u32 = 1;

/// `sub_824D1E00`'s two controller inputs: id 0 is the gate, id 1 the weight.
const GATE_INPUT: u32 = 0;
const WEIGHT_INPUT: u32 = 1;

/// The gate's raised value, and the ceiling the weight is clamped to.
const FULL_SCALE: u32 = 32_767;

/// The vault class of a material record, and the field `sub_82496FD0` reads its category from.
/// The field's retail type is `Sk8::Audio::eMaterialNicotineType`; it takes exactly ten values
/// across the 145 records, which is what the `0..=9` jump table above indexes.
const MATERIAL_CLASS: &str = "Hash_D40CB4C0FFE45676";
const MATERIAL_CATEGORY_FIELD: &str = "Hash_D5EF686287A57AFE";

/// `sub_82497088`'s own RefSpec on the material record, and the class it points at.
const MATERIAL_IMPACT_REFSPEC: &str = "Hash_F64F891C312EFD5E";
const IMPACT_CLASS: &str = "Hash_7DAFF70B3A91CD5D";

/// The four floats of an impact record, **in native offset order** `+0`, `+4`, `+8`, `+12`.
///
/// `sub_82497088` reads them by offset, not by name, so the mapping had to be recovered. It is
/// derived rather than assumed: the three bands below are contiguous and ascending, which forces
/// the order, and that holds for all 45 records of the class with no violations. The defaults are
/// `+12` 0.005, `+8` 0.5, `+0` 1.0, `+4` 2.0.
const IMPACT_FIELDS: [&str; 4] = [
    "Hash_C8DED1BC20B9D6A5", // +0  band 1/2 split
    "Hash_B8870D2001033E0F", // +4  ceiling
    "Hash_7D8DEDD338D45482", // +8  band 0/1 split
    "Hash_D660AC459139BDF4", // +12 floor; below it the contact is silent
];

/// One material's impact bands, `sub_82497088`'s record.
#[derive(Clone, Copy, Debug)]
struct ImpactBands {
    /// `[+12]`, `[+8]`, `[+0]`, `[+4]` in ascending order.
    floor: f32,
    mid: f32,
    high: f32,
    ceiling: f32,
}

impl Default for ImpactBands {
    /// Class `7DAFF70B3A91CD5D`'s own `default` record, used only when a material's RefSpec does
    /// not resolve. Against the owner's real vault every one of the 143 materials resolves, with
    /// five distinct floors across the table — see `the_vault_impact_bands_tile_for_every_material`.
    fn default() -> Self {
        Self {
            floor: 0.005,
            mid: 0.5,
            high: 1.0,
            ceiling: 2.0,
        }
    }
}

/// The controller key of one collision slot. `sub_828DEA00` packs a component's key as
/// `0x40000000 | category << 16 | index << 11 | kind << 4`; collision is category 3, kind 0, so
/// the ten slots are `40030000`, `40030800`, … `40034800` — which is exactly what the retail
/// MixMap holds (`cargo run -p skate-audio-core --example mixmap_dump`, ten `SFXObj_Collision`
/// controllers whose outputs run 0..=22 with 1 and 22 typed `t1`).
pub(crate) fn controller_key(slot: usize) -> u32 {
    0x4003_0000 | ((slot as u32) << 11)
}

/// `sub_824D20E8`: which Collision controller **output** scales a voice, chosen by the material's
/// category (`sub_82496FD0`). The jump table at `0x824D212C`, category 0..=9 in order.
///
/// `sub_824D2318` multiplies the voice's gain by this output — `fmuls f3,f5,f4` with `f4` from
/// vtable +60 — so a contact voice is *not* played at its material level alone. Leaving it out
/// made both landing collision voices play at full material level, which is loud enough to swamp
/// the class voice the landing is supposed to vary with.
pub(crate) fn category_output_id(category: u8) -> u32 {
    const IDS: [u32; 10] = [13, 14, 15, 16, 17, 18, 12, 19, 21, 20];
    IDS.get(usize::from(category)).copied().unwrap_or(13)
}

/// `sub_82486EF0`'s 48-byte message, field for field.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ContactMessage {
    /// `+0x00`: the first material — for a grind onset the family base (95 or 96).
    pub material_a: i32,
    /// `+0x04`: the second material — for a grind onset the grind material (143 → 10).
    pub material_b: i32,
    /// `+0x08`: the first material's impact tier, 3 for "none".
    pub tier_a: i32,
    /// `+0x0C`: the second material's impact tier, 3 for "none".
    pub tier_b: i32,
    /// `+0x10`: a `vec4` world position, copied wholesale from the audio state (`+48` for a grind
    /// onset, `+144` for a landing).
    pub position: [f32; 4],
    /// `+0x20` / `+0x24`: the two `0..=32767` levels from `sub_82496C58`'s interpolation.
    pub level_a: u32,
    pub level_b: u32,
    /// `+0x28` / `+0x29` / `+0x2A`.
    pub flags: [u8; 3],
}

impl ContactMessage {
    /// The material of voice record `i`, as `sub_824D1F68` indexes them (`msg+0`, `msg+4`).
    pub(crate) fn material(&self, record: usize) -> i32 {
        if record == 0 {
            self.material_a
        } else {
            self.material_b
        }
    }

    /// The tier of voice record `i` (`msg+8`, `msg+12`).
    pub(crate) fn tier(&self, record: usize) -> i32 {
        if record == 0 {
            self.tier_a
        } else {
            self.tier_b
        }
    }
}

/// `sub_824D1E00`'s weight: the message's two tier words choose one of three fixed levels.
///
/// 3 is a "none" sentinel — if one word is 3 the other is taken, and only when neither is 3 does
/// the larger win. The clamp is retail's own and is vacuous for these three constants, but it is
/// what the lifted code does so it is kept.
pub(crate) fn contact_weight(tier_a: i32, tier_b: i32) -> u32 {
    let selected = if tier_a == NO_TIER {
        tier_b
    } else if tier_b == NO_TIER {
        tier_a
    } else {
        tier_a.max(tier_b)
    };
    let level: i32 = match selected {
        1 => 20_000,
        2 => 32_767,
        _ => 10_000,
    };
    level.clamp(0, FULL_SCALE as i32) as u32
}

/// The per-material collision table: the baked image data plus the categories read from the
/// owner's vault.
pub(crate) struct CollisionMaterials {
    category: [u8; MATERIAL_COUNT],
    /// Everything `sub_824967F8` would read off the material's record, resolved once at load
    /// instead of per contact. Retail does the vault lookup on every message; the values are
    /// static, so the table is the same.
    entries: Vec<MaterialEntry>,
}

/// One material's row: the seven samples `sub_824967F8` can pick between, plus its level.
#[derive(Clone, Copy, Debug, Default)]
struct MaterialEntry {
    /// `[record+52]`, clamped as the original clamps it.
    level: u32,
    /// `sub_82496C58`'s `(low, high)` windows, `[tier][class]`.
    pairs: [[(i32, i32); 3]; 3],
    /// Whether the RefSpec resolved at all; when it did not, retail reads its zeroed default
    /// record and every window is `(0, 0)`, which makes the level 0 and suppresses the contact.
    pairs_resolved: bool,
    /// `sub_82497088`'s four impact thresholds.
    bands: ImpactBands,
    /// `tier == 2`.
    tier_two: u16,
    /// `tier == 0`, by the paired material's class.
    tier_zero: [u16; 3],
    /// Any other tier, by the paired material's class.
    tier_other: [u16; 3],
}

impl CollisionMaterials {
    /// Resolve every material's category through the vault, the way `sub_82496FD0` does one at a
    /// time. A material whose record is absent from this installation falls back the way retail's
    /// pre-seeded default record does; the count of those is returned so a caller can report it.
    pub(crate) fn load(vault: &Collections) -> (Self, usize) {
        let mut category = [DEFAULT_CATEGORY; MATERIAL_COUNT];
        let mut unresolved = 0;
        for (material, slot) in category.iter_mut().enumerate() {
            let key = format!("Hash_{:016X}", MATERIAL_VAULT_KEY[material]);
            match vault.field(MATERIAL_CLASS, &key, MATERIAL_CATEGORY_FIELD) {
                Ok(field) => match u32::from_str_radix(field.data.trim(), 16) {
                    Ok(value) => *slot = value as u8,
                    Err(_) => unresolved += 1,
                },
                Err(_) => unresolved += 1,
            }
        }
        let entries = (0..MATERIAL_COUNT)
            .map(|material| Self::entry(vault, material))
            .collect();
        (Self { category, entries }, unresolved)
    }

    /// Read one material's row: `sub_824967F8`'s seven candidate fields and `sub_824965D0`'s level.
    fn entry(vault: &Collections, material: usize) -> MaterialEntry {
        let Some(fields) = kind_fields(MATERIAL_SOUND_ID[material]) else {
            return MaterialEntry::default();
        };
        let key = format!("Hash_{:016X}", MATERIAL_VAULT_KEY[material]);
        let word = |name: &str| -> u32 {
            if name.is_empty() {
                return 0;
            }
            vault
                .field(MATERIAL_CLASS, &key, name)
                .ok()
                .and_then(|field| u32::from_str_radix(field.data.trim(), 16).ok())
                .unwrap_or(0)
        };
        // The original clamps the level into `0..=32767` before it reaches the voice record.
        let level = word(MATERIAL_LEVEL_FIELD).min(FULL_SCALE);
        let sample = |name: &str| u16::try_from(word(name)).unwrap_or(0);
        let (pairs, pairs_resolved) = Self::level_pairs(vault, &key);
        let bands = Self::impact_bands(vault, &key);
        MaterialEntry {
            bands,
            level,
            pairs,
            pairs_resolved,
            tier_two: sample(fields.tier_two),
            tier_zero: fields.tier_zero.map(sample),
            tier_other: fields.tier_other.map(sample),
        }
    }

    /// `sub_824840E0`: follow the material's `Attrib::RefSpec` to its `13E20D398E385A56` record and
    /// read `sub_82496C58`'s fourteen window bounds off it. The RefSpec's payload is
    /// `class (8 bytes) | key (8 bytes) | padding`, so the key is bytes 8..16.
    fn level_pairs(vault: &Collections, material_key: &str) -> ([[(i32, i32); 3]; 3], bool) {
        let mut pairs = [[DEFAULT_LEVEL_PAIR; 3]; 3];
        let Ok(spec) = vault.field(MATERIAL_CLASS, material_key, MATERIAL_LEVELS_REFSPEC) else {
            return (pairs, false);
        };
        let data = spec.data.trim();
        if data.len() < 32 {
            return (pairs, false);
        }
        let key = format!("Hash_{}", &data[16..32]);
        let mut resolved = false;
        for (tier, row) in LEVEL_PAIRS.iter().enumerate() {
            for (class, (low, high)) in row.iter().enumerate() {
                let read = |name: &str| -> Option<i32> {
                    let field = vault.field(LEVEL_CLASS, &key, name).ok()?;
                    u32::from_str_radix(field.data.trim(), 16)
                        .ok()
                        .map(|v| v as i32)
                };
                if let (Some(low), Some(high)) = (read(low), read(high)) {
                    pairs[tier][class] = (low, high);
                    resolved = true;
                }
            }
        }
        (pairs, resolved)
    }

    /// Follow the material's second RefSpec to its `7DAFF70B3A91CD5D` record, `sub_82497088`'s
    /// thresholds. A material with no record of its own keeps the class default, which is what
    /// retail's own lookup returns for it.
    fn impact_bands(vault: &Collections, material_key: &str) -> ImpactBands {
        let Ok(spec) = vault.field(MATERIAL_CLASS, material_key, MATERIAL_IMPACT_REFSPEC) else {
            return ImpactBands::default();
        };
        let data = spec.data.trim();
        if data.len() < 32 {
            return ImpactBands::default();
        }
        // The payload is `class (8 bytes) | key (8 bytes) | padding`, as `level_pairs` reads it.
        let key = format!("Hash_{}", &data[16..32]);
        let read = |name: &str| -> Option<f32> {
            let field = vault.field(IMPACT_CLASS, &key, name).ok()?;
            u32::from_str_radix(field.data.trim(), 16)
                .ok()
                .map(f32::from_bits)
        };
        let [high, ceiling, mid, floor] = IMPACT_FIELDS.map(read);
        match (floor, mid, high, ceiling) {
            (Some(floor), Some(mid), Some(high), Some(ceiling)) => ImpactBands {
                floor,
                mid,
                high,
                ceiling,
            },
            _ => ImpactBands::default(),
        }
    }

    /// `sub_82497088`: which band an impact falls in, and the window that band interpolates over.
    ///
    /// `None` is retail's category 3 — the impact is under the material's floor and makes no
    /// sound at all. Otherwise the band doubles as `contact_level`'s `tier`, which is why that
    /// function already takes one.
    pub(crate) fn impact_band(&self, material: i32, impact: f32) -> Option<(i32, f32, f32)> {
        let bands = usize::try_from(material)
            .ok()
            .and_then(|m| self.entries.get(m))
            .map_or_else(ImpactBands::default, |entry| entry.bands);
        if impact < bands.floor {
            return None;
        }
        Some(if impact > bands.high {
            (2, bands.high, bands.ceiling)
        } else if impact > bands.mid {
            (1, bands.mid, bands.high)
        } else {
            (0, bands.floor, bands.mid)
        })
    }

    /// `sub_82496C58`: the `0..=32767` level a contact between `material` and `other` carries.
    ///
    /// `value` is the quantity the window interpolates over — for a landing the latched air time
    /// (`[this+340]`), which is why retail's landing weight follows how far you actually fell.
    /// `lo`/`hi` are that tier's window. A negative material is 0, and a tier with no window keeps
    /// the `(0, 32767)` default.
    pub(crate) fn contact_level(
        &self,
        material: i32,
        other: i32,
        tier: i32,
        lo: f32,
        hi: f32,
        value: f32,
        surfaces: &SurfaceMap,
    ) -> u32 {
        if material < 0 {
            return 0;
        }
        // `sub_82496C58` stops at material 97 and at "no material", both of which keep class 0.
        let class = if material >= CLASS_LOOKUP_CEILING || other == NO_MATERIAL {
            0
        } else {
            self.other_class(other, surfaces).clamp(0, 2) as usize
        };
        let (low, high) = match usize::try_from(tier) {
            Ok(tier) if tier < LEVEL_PAIRS.len() => self
                .entries
                .get(material as usize)
                .map_or(DEFAULT_LEVEL_PAIR, |entry| entry.pairs[tier][class]),
            _ => DEFAULT_LEVEL_PAIR,
        };
        // `f30 = min(value, hi)`, then the interpolation — or just `high` when the window is empty.
        let value = value.min(hi);
        if hi <= lo {
            return high.clamp(0, FULL_SCALE as i32) as u32;
        }
        let scaled = low as f32 + (high - low) as f32 / (hi - lo) * (value - lo);
        // `fctiwz` truncates.
        (scaled as i32).clamp(0, FULL_SCALE as i32) as u32
    }

    /// For tests and for callers that already know the table.
    #[cfg(test)]
    pub(crate) fn from_categories(category: [u8; MATERIAL_COUNT]) -> Self {
        Self {
            category,
            entries: vec![MaterialEntry::default(); MATERIAL_COUNT],
        }
    }

    /// `sub_824965D0` + `sub_824967F8`: what this material plays against `other` at `tier`.
    ///
    /// `None` is retail's own "no sound": a material outside the table, the one whose kind word is
    /// `-1`, a paired class the kind has no field for, or a sample of 0.
    pub(crate) fn sample(&self, material: i32, other: i32, tier: i32) -> Option<CollisionSample> {
        if !(0..MATERIAL_COUNT as i32).contains(&material) {
            return None;
        }
        let fields = kind_fields(self.kind(material))?;
        let entry = self.entries.get(material as usize)?;
        let class = usize::try_from(other).ok()?;
        let sample = match tier {
            2 => entry.tier_two,
            0 => *entry.tier_zero.get(class)?,
            _ => *entry.tier_other.get(class)?,
        };
        // Retail leaves its output word at 0 when the field holds nothing, and the caller reads
        // that as "no voice".
        (sample != 0).then_some(CollisionSample {
            bank: fields.bank,
            sample,
            level: entry.level,
        })
    }

    /// `sub_82497910`: the paired material's class, `0..=2`. Materials 95..=113 answer from
    /// retail's own jump table; everything else reads the `AudioSurfaceMap` word at `+28`, which
    /// clamps out-of-range materials to element 94 exactly as `SurfaceMap::lookup` does.
    pub(crate) fn other_class(&self, material: i32, surfaces: &SurfaceMap) -> i32 {
        if material == NO_MATERIAL {
            // `sub_824965D0` never calls the classifier for "no material"; it passes 0.
            return 0;
        }
        special_other_class(material).unwrap_or_else(|| surfaces.lookup(material, 28) as i32)
    }

    /// `sub_82496FD0`.
    pub(crate) fn category(&self, material: i32) -> u8 {
        if material < 0 {
            return NEGATIVE_MATERIAL_CATEGORY;
        }
        self.category
            .get(material as usize)
            .copied()
            .unwrap_or(DEFAULT_CATEGORY)
    }

    /// The kind word at `+0` of the material's entry: which family of fields — and so which bank —
    /// the material's samples come from. `-1` means the material is silent, and `sub_824965D0`
    /// returns "no sound" without consulting the chooser at all.
    pub(crate) fn kind(&self, material: i32) -> i32 {
        if !(0..MATERIAL_COUNT as i32).contains(&material) {
            return -1;
        }
        MATERIAL_SOUND_ID[material as usize]
    }

    /// `sub_824D20E8`: which controller output carries this material's level.
    pub(crate) fn level_output(&self, material: i32) -> u32 {
        let category = if material >= NO_MATERIAL {
            HIGH_MATERIAL_LEVEL_CATEGORY
        } else {
            self.category(material)
        };
        CATEGORY_LEVEL_OUTPUT
            .get(category as usize)
            .copied()
            .unwrap_or(LEVEL_OUTPUT_FALLBACK)
    }

    /// `sub_824D22B8`: which controller output carries this material's pitch. `material >= 143`
    /// takes the category-9 branch here, *not* the category-8 one `level_output` forces.
    pub(crate) fn pitch_output(&self, material: i32) -> u32 {
        if material >= NO_MATERIAL || self.category(material) == 9 {
            PITCH_OUTPUT_CATEGORY_9
        } else {
            PITCH_OUTPUT_OTHER
        }
    }
}

/// `sub_824967F8`'s field table for one material kind.
///
/// The kind is the `+0` word of the material's entry in the image table, and it selects which
/// *family* of fields on the material record the sample is read from — which in AttribSys is the
/// same thing as which bank, because the field's retail type name **is** the bank's file name
/// (`Skate_Collisions` → `Skate_Collisions.bnk`, and so on).
///
/// Within a kind, `(tier, other class)` picks the field. `tier == 2` ignores the other class
/// entirely; `tier == 0` and everything else take one of three fields by the paired material's
/// class. The offsets in the comments are the ones the lifted code uses, and they come straight
/// out of the AttribSys class layout in `skaterschema.vlt` — retail's own field order.
struct KindFields {
    bank: &'static str,
    /// `tier == 2`.
    tier_two: &'static str,
    /// `tier == 0`, by other class 0/1/2.
    tier_zero: [&'static str; 3],
    /// Any other tier, by other class 0/1/2.
    tier_other: [&'static str; 3],
}

/// Kind 0 — the `Skate_Collisions` fields, offsets `+112`, `+64`/`+108`/`+120`, `+60`/`+104`/`+116`.
const COLLISION_FIELDS: KindFields = KindFields {
    bank: skate_data::audio::splice::COLLISIONS_BANK,
    tier_two: "Hash_9203DF6FD029B377",
    tier_zero: [
        "Hash_BFABF634D2B1E45A",
        "Hash_9ABFC64574AB2F9F",
        "Hash_BCD5E888294F7B15",
    ],
    tier_other: [
        "Hash_EF9BD81F9CFF725F",
        "Hash_A3ADCA7B19287B5D",
        "Hash_C676C87F862C0490",
    ],
};

/// Kind 1 — the `Skate_Metal` fields, offsets `+92`, `+80`/`+88`/`+100`, `+76`/`+84`/`+96`. This is
/// the set a metal rail lands on, which is why a rail grind does not sound like concrete.
const METAL_FIELDS: KindFields = KindFields {
    bank: METAL_BANK,
    tier_two: "Hash_F54277A83E0170FD",
    tier_zero: [
        "Hash_66A95889604DED36",
        "Hash_595537EBA0196BE7",
        "Hash_79DD0E6659793D0E",
    ],
    tier_other: [
        "Hash_B722B88FE44B046E",
        "Hash_1411108A7E9CC74A",
        "Hash_50796F92F3DE449B",
    ],
};

/// Kind 2 — the `HOM_Set_1` fields, materials 102..=106.
///
/// These are the only fields retail looks up **by hash** (`sub_82B72420`) rather than by offset,
/// and the class layout says exactly why: they are the ones with no static offset at all. The one
/// hole is `tier == 0` with other class 0, which goes through `sub_824825D0` — a different
/// accessor that is not decoded, so that combination resolves to no sample rather than a guess.
const HOM_FIELDS: KindFields = KindFields {
    bank: HOM_BANK,
    tier_two: "Hash_3EA2579C2F3BB23B",
    tier_zero: ["", "Hash_2A2830137430BB02", "Hash_79BDE00B04DF51B9"],
    tier_other: [
        "Hash_5432B35224E4B1C1",
        "Hash_3FCBE0407F833721",
        "Hash_53311C6761F135A1",
    ],
};

/// The two banks beside `Skate_Collisions.bnk` the collision materials name. Both are in a stock
/// `audiofiles.big`.
pub(crate) const METAL_BANK: &str = "Skate_Metal.bnk";
pub(crate) const HOM_BANK: &str = "HOM_Set_1.bnk";

/// `sub_82496C58`'s `(low, high)` pairs. They do not live on the material record: the material's
/// `Attrib::RefSpec` at `82B1451A90152514` points at a record of class `13E20D398E385A56`, which
/// `sub_824840E0` dereferences, and the pairs are that record's fourteen `Int32`s. The offsets in
/// the lifted code (`+0/+4`, `+8/+12`, …) are its AttribSys class layout, read from
/// `skaterschema.vlt`.
const MATERIAL_LEVELS_REFSPEC: &str = "Hash_82B1451A90152514";
/// The class the RefSpec points at.
const LEVEL_CLASS: &str = "Hash_13E20D398E385A56";
/// `(tier, paired class) -> (low field, high field)`. `tier == 2` ignores the class; `tier >= 3`
/// never reaches the table and keeps the `(0, 32767)` default.
const LEVEL_PAIRS: [[(&str, &str); 3]; 3] = [
    // tier 0: class 0 -> +8/+12, class 1 -> +24/+28, class >= 2 -> +52/+56
    [
        ("Hash_E24D9CB4000A53AC", "Hash_1A83AD0330976744"),
        ("Hash_75DC915E876A9DC9", "Hash_711F1F54903E76C9"),
        ("Hash_166BAB2FD5B60560", "Hash_0D6EF57A39AF0C93"),
    ],
    // tier 1: class 0 -> +0/+4, class 1 -> +16/+20, class >= 2 -> +44/+48
    [
        ("Hash_036F313CEBC664FD", "Hash_8001982DA2E91D6A"),
        ("Hash_24B725E05CEB027E", "Hash_CBBB19E302CE1A17"),
        ("Hash_41F2E2456A97752A", "Hash_2638FF12C8FBBCCB"),
    ],
    // tier 2: +32/+36 whatever the class
    [
        ("Hash_C8DED1BC20B9D6A5", "Hash_B8870D2001033E0F"),
        ("Hash_C8DED1BC20B9D6A5", "Hash_B8870D2001033E0F"),
        ("Hash_C8DED1BC20B9D6A5", "Hash_B8870D2001033E0F"),
    ],
];

/// `sub_82496C58`'s defaults when the tier has no window (`tier >= 3`).
const DEFAULT_LEVEL_PAIR: (i32, i32) = (0, FULL_SCALE as i32);

/// `sub_82496C58` stops consulting the paired material's class at material 97 and treats the
/// "no material" pair as class 0.
const CLASS_LOOKUP_CEILING: i32 = 97;

/// `sub_824965D0`'s per-material level, `[record+52]` — clamped to `0..=32767` by the original.
const MATERIAL_LEVEL_FIELD: &str = "Hash_875BA75341DC8391";

/// `sub_824967F8`'s three-way switch on the material's kind word. Anything other than 1 or 2 takes
/// the `Skate_Collisions` arm, which is the original's `default:`; `-1` is the silent material and
/// never reaches here.
fn kind_fields(kind: i32) -> Option<&'static KindFields> {
    match kind {
        -1 => None,
        1 => Some(&METAL_FIELDS),
        2 => Some(&HOM_FIELDS),
        _ => Some(&COLLISION_FIELDS),
    }
}

/// The paired material's class, `sub_82497910`, for the materials it answers from its own jump
/// table at `0x82497944` instead of the surface map. Materials 110..=112 fall through to the
/// surface-map path with everything else, so they are absent here.
fn special_other_class(material: i32) -> Option<i32> {
    Some(match material {
        96 => 2,
        98 | 103 | 107 | 108 | 109 => 0,
        95 | 97 | 99..=102 | 104..=106 | 113 => 1,
        _ => return None,
    })
}

/// What one material contributes to a contact: which bank, which sample, and at what level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CollisionSample {
    pub bank: &'static str,
    /// The Splice sample index. Retail treats 0 as "no sample", so this is never 0.
    pub sample: u16,
    /// `[record+52]`, `0..=32767`.
    pub level: u32,
}

/// One `CSTATE_Collision` slot and the `SFXObj_Collision` hanging off it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Slot {
    /// `[node+64]`: the stamp the router compares. Retail takes it from a global counter, so the
    /// lowest is the least recently used.
    pub stamp: i32,
    /// `[node+68]` (and `SFXObj_Collision+36`): the parked message.
    pub message: Option<ContactMessage>,
    /// `[state+52]`: raised by `sub_828DF6F0` on activation, cleared when both voice records go
    /// idle (`sub_824D2318`'s tail calls the state's `vtable[+28]`).
    pub active: bool,
    /// `SFXObj_Collision+40` / `+72`: whether each of the two voice records holds a voice. These
    /// are set from the conditions `sub_824D1F68` checks before it calls the sample chooser; the
    /// chooser's own refusal is the undecoded part (see the module note).
    pub started: [bool; 2],
}

/// The ten-slot LRU manager.
#[derive(Clone, Debug, Default)]
pub(crate) struct CollisionStates {
    slots: [Slot; SLOT_COUNT],
    /// The global counter `sub_824F8990` stamps from (`[[0x830CFD94]+16]`).
    clock: i32,
}

impl CollisionStates {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn slots(&self) -> &[Slot; SLOT_COUNT] {
        &self.slots
    }

    /// `sub_82486EF0`'s two hops: route to the least recently used slot, evict what it was
    /// holding, then stamp, park and activate. Returns the slot the message landed on.
    pub(crate) fn post(
        &mut self,
        message: ContactMessage,
        materials: &CollisionMaterials,
    ) -> usize {
        // `sub_824F1818` walks from the head and keeps a node only when it is *strictly* lower,
        // so the first slot at the minimum wins a tie — which is what `min_by_key` does too.
        let slot = (0..SLOT_COUNT)
            .min_by_key(|&index| self.slots[index].stamp)
            .expect("SLOT_COUNT is not zero");
        // `vtable[+28]` (`sub_824F8B58`) frees the message the slot still held before reuse.
        let state = &mut self.slots[slot];
        state.message = None;
        state.started = [false; 2];
        // `sub_824F8990`, then `sub_828DF6F0`.
        self.clock = self.clock.wrapping_add(1);
        let state = &mut self.slots[slot];
        state.stamp = self.clock;
        state.message = Some(message);
        state.active = true;
        // `sub_824D1F68`: one voice record per material, each skipped on the two sentinels.
        for record in 0..2 {
            let material = message.material(record);
            state.started[record] = material != NO_MATERIAL
                && message.tier(record) != NO_TIER
                && materials.kind(material) != -1;
        }
        // `sub_824D2318`'s tail: a component that started nothing deactivates its state again.
        if !state.started.iter().any(|started| *started) {
            state.active = false;
            state.message = None;
        }
        slot
    }

    /// Release a slot's voice record, as retail does when a voice finishes. When both go idle the
    /// state deactivates, exactly as `sub_824D2318`'s tail does.
    pub(crate) fn release(&mut self, slot: usize, record: usize) {
        let state = &mut self.slots[slot];
        state.started[record] = false;
        if !state.started.iter().any(|started| *started) {
            state.active = false;
            state.message = None;
        }
    }

    /// `sub_824D1E00`, for every slot: `(controller key, input id, value)`.
    ///
    /// Retail rewrites both inputs from zero on every frame before deciding whether to raise them,
    /// so the zeroes are emitted too and in the original's order — this is a continuously driven
    /// pair, not a one-shot, which is the whole reason a landing's weight can vary while it rings.
    pub(crate) fn drive(&self) -> Vec<(u32, u32, u32)> {
        let mut writes = Vec::with_capacity(SLOT_COUNT * 2);
        for (index, slot) in self.slots.iter().enumerate() {
            let key = controller_key(index);
            writes.push((key, GATE_INPUT, 0));
            writes.push((key, WEIGHT_INPUT, 0));
            if !slot.active || !slot.started.iter().any(|started| *started) {
                continue;
            }
            let Some(message) = slot.message else {
                continue;
            };
            writes.push((key, GATE_INPUT, FULL_SCALE));
            writes.push((
                key,
                WEIGHT_INPUT,
                contact_weight(message.tier_a, message.tier_b),
            ));
        }
        writes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn materials() -> CollisionMaterials {
        let mut category = [0u8; MATERIAL_COUNT];
        // Enough of the real spread to exercise both output pickers.
        category[10] = 1;
        category[7] = 3;
        category[5] = 4;
        category[60] = 9;
        CollisionMaterials::from_categories(category)
    }

    fn message(material_a: i32, material_b: i32, tier_a: i32, tier_b: i32) -> ContactMessage {
        ContactMessage {
            material_a,
            material_b,
            tier_a,
            tier_b,
            ..ContactMessage::default()
        }
    }

    #[test]
    fn the_ten_controller_keys_are_the_ones_the_mixmap_holds() {
        // `mixmap_dump` prints exactly these for SFXObj_Collision #0..#9.
        assert_eq!(controller_key(0), 0x4003_0000);
        assert_eq!(controller_key(1), 0x4003_0800);
        assert_eq!(controller_key(9), 0x4003_4800);
    }

    #[test]
    fn the_weight_treats_three_as_no_tier_and_otherwise_takes_the_larger() {
        // One word absent: the other decides.
        assert_eq!(contact_weight(NO_TIER, 2), 32_767);
        assert_eq!(contact_weight(1, NO_TIER), 20_000);
        // Neither absent: the larger wins.
        assert_eq!(contact_weight(0, 1), 20_000);
        assert_eq!(contact_weight(2, 1), 32_767);
        // Anything that is not 1 or 2 — including both words absent — is the floor.
        assert_eq!(contact_weight(0, 0), 10_000);
        assert_eq!(contact_weight(NO_TIER, NO_TIER), 10_000);
    }

    #[test]
    fn the_output_pickers_diverge_on_a_material_past_the_table() {
        let m = materials();
        // `sub_824D20E8`'s jump table, category 6 → 12 is the one that is out of order.
        assert_eq!(m.level_output(0), 13);
        assert_eq!(m.level_output(10), 14);
        assert_eq!(m.level_output(7), 16);
        assert_eq!(m.level_output(60), 21);
        // 143 forces category 8 here …
        assert_eq!(m.level_output(NO_MATERIAL), 20);
        // … but takes the category-9 branch there. The two are not the same test.
        assert_eq!(m.pitch_output(NO_MATERIAL), 22);
        assert_eq!(m.pitch_output(60), 22);
        assert_eq!(m.pitch_output(10), 1);
    }

    #[test]
    fn a_negative_material_is_category_eight() {
        assert_eq!(materials().category(-1), NEGATIVE_MATERIAL_CATEGORY);
    }

    #[test]
    fn material_ninety_four_is_the_silent_one() {
        let m = materials();
        // The only `-1` in the retail table, and the same slot `SurfaceMap` clamps to.
        assert_eq!(m.kind(94), -1);
        assert_ne!(m.kind(0), -1);
        assert_eq!(m.kind(NO_MATERIAL), -1);
        assert_eq!(m.kind(-1), -1);
    }

    #[test]
    fn the_router_reuses_the_least_recently_used_slot() {
        let m = materials();
        let mut states = CollisionStates::new();
        // Ten posts fill every slot in order, because every stamp starts at 0 and ties go to the
        // first slot.
        for expected in 0..SLOT_COUNT {
            assert_eq!(states.post(message(0, 1, 0, 0), &m), expected);
        }
        // The eleventh comes back round to slot 0, the oldest stamp.
        assert_eq!(states.post(message(0, 1, 0, 0), &m), 0);
        assert_eq!(states.post(message(0, 1, 0, 0), &m), 1);
    }

    #[test]
    fn a_post_evicts_the_message_the_slot_was_holding() {
        let m = materials();
        let mut states = CollisionStates::new();
        let first = message(0, 1, 0, 0);
        let second = message(10, 7, 1, 2);
        for _ in 0..SLOT_COUNT {
            states.post(first, &m);
        }
        assert_eq!(states.slots()[0].message, Some(first));
        states.post(second, &m);
        assert_eq!(states.slots()[0].message, Some(second));
    }

    #[test]
    fn a_message_with_no_playable_material_leaves_its_state_inactive() {
        let m = materials();
        let mut states = CollisionStates::new();
        // Both records refused: material 143 on one side, tier 3 on the other.
        let slot = states.post(message(NO_MATERIAL, 1, 0, NO_TIER), &m);
        assert_eq!(states.slots()[slot].started, [false, false]);
        assert!(!states.slots()[slot].active);
        // And the silent material is refused the same way.
        let slot = states.post(message(94, NO_MATERIAL, 0, NO_TIER), &m);
        assert_eq!(states.slots()[slot].started, [false, false]);
        assert!(!states.slots()[slot].active);
    }

    #[test]
    fn the_drive_zeroes_every_slot_and_raises_only_the_live_ones() {
        let m = materials();
        let mut states = CollisionStates::new();
        let slot = states.post(message(0, 1, 1, NO_TIER), &m);
        let writes = states.drive();
        let key = controller_key(slot);
        // Every slot is zeroed first, live or not.
        assert_eq!(
            writes
                .iter()
                .filter(|(_, id, v)| *id == GATE_INPUT && *v == 0)
                .count(),
            SLOT_COUNT
        );
        // The live slot then raises the gate and the weight, in that order, after its own zeroes.
        let mine: Vec<_> = writes.iter().filter(|(k, ..)| *k == key).copied().collect();
        assert_eq!(
            mine,
            vec![
                (key, GATE_INPUT, 0),
                (key, WEIGHT_INPUT, 0),
                (key, GATE_INPUT, FULL_SCALE),
                (key, WEIGHT_INPUT, 20_000)
            ]
        );
        // A slot nobody posted to is left at zero.
        let idle = controller_key(SLOT_COUNT - 1);
        let theirs: Vec<_> = writes
            .iter()
            .filter(|(k, ..)| *k == idle)
            .copied()
            .collect();
        assert_eq!(theirs, vec![(idle, GATE_INPUT, 0), (idle, WEIGHT_INPUT, 0)]);
    }

    /// Pins the categories against the owner's real vault rather than a synthetic table: the
    /// baked keys must actually resolve, and the values must be the `0..=9` the jump table
    /// indexes. Ignored because it needs the installation.
    ///
    ///     cargo test -p skate-game --bin skate3rust -- --ignored the_vault_resolves --nocapture
    #[test]
    #[ignore = "needs the owner's assets"]
    fn the_vault_resolves_every_material_category() {
        let assets =
            std::path::Path::new("C:/s3/installations/70eda9dc4644496d81ae73af95ff4285/assets");
        let vault = Collections::load(assets).expect("vault");
        let (materials, unresolved) = CollisionMaterials::load(&vault);
        // Exactly one material has no record: 94, whose key is all zeroes and whose sound id is
        // the table's only `-1`. (A JSON export of the vault appears to be missing five further
        // records — 77, 87, 92, 97, 98 — but the vault itself has them, which is why this test
        // reads the vault and not the export.)
        assert_eq!(
            unresolved, 1,
            "unexpected number of materials without a vault record"
        );
        assert_eq!(materials.kind(94), -1);
        let mut seen = [0usize; 10];
        for material in 0..MATERIAL_COUNT as i32 {
            let category = materials.category(material);
            assert!(category <= 9, "material {material} has category {category}");
            seen[category as usize] += 1;
        }
        // Every category is used, so the whole jump table is reachable from real data.
        assert!(
            seen.iter().all(|count| *count > 0),
            "unused categories: {seen:?}"
        );
        println!("category histogram {seen:?}, {unresolved} materials without a record");
    }

    /// The chooser against the real vault: the banks a material's kind selects, the holes retail
    /// leaves, and what a rail grind actually resolves to.
    ///
    ///     cargo test -p skate-game --bin skate3rust -- --ignored the_chooser --nocapture
    #[test]
    #[ignore = "needs the owner's assets"]
    fn the_chooser_picks_real_samples_out_of_the_right_banks() {
        let assets =
            std::path::Path::new("C:/s3/installations/70eda9dc4644496d81ae73af95ff4285/assets");
        let vault = Collections::load(assets).expect("vault");
        let (m, _) = CollisionMaterials::load(&vault);
        let surfaces = SurfaceMap::load(&vault).expect("surface map");

        // Material 94 is the silent one and never reaches the chooser.
        assert_eq!(m.sample(94, 0, 0), None);
        assert_eq!(m.sample(NO_MATERIAL, 0, 0), None);

        // Kind 1 is the Skate_Metal family — the reason a metal rail does not sound like concrete.
        let metal = m.sample(8, 1, 1).expect("material 8 is kind 1");
        assert_eq!(metal.bank, METAL_BANK);
        // Kind 0 is the Skate_Collisions family.
        let concrete = m.sample(0, 1, 1).expect("material 0 is kind 0");
        assert_eq!(concrete.bank, skate_data::audio::splice::COLLISIONS_BANK);
        // Kind 2 (materials 102..=106) is HOM_Set_1, and its one undecoded combination —
        // `tier == 0` against class 0, which goes through `sub_824825D0` — stays silent.
        assert_eq!(m.sample(104, 0, 0), None);
        let hom = m.sample(104, 1, 0).expect("material 104 is kind 2");
        assert_eq!(hom.bank, HOM_BANK);

        // The paired-material classifier: the grind family bases answer from retail's jump table.
        assert_eq!(m.other_class(95, &surfaces), 1);
        assert_eq!(m.other_class(96, &surfaces), 2);
        assert_eq!(m.other_class(98, &surfaces), 0);
        // Everything else comes off the surface map, and must stay inside the 0..=2 the chooser
        // indexes with — otherwise the material would silently resolve to nothing.
        for material in 0..NO_MATERIAL {
            let class = m.other_class(material, &surfaces);
            assert!(
                (0..=2).contains(&class),
                "material {material} has paired class {class}"
            );
        }

        // Every tier a real onset can produce, not just the one that happened to work.
        for tier in [0, 1] {
            for family_base in [95, 96] {
                let grind_material = 10;
                let a = m.sample(family_base, m.other_class(grind_material, &surfaces), tier);
                let b = m.sample(grind_material, m.other_class(family_base, &surfaces), tier);
                println!("tier {tier} family {family_base}: {a:?} / {b:?}");
            }
        }
        println!(
            "other_class(10)={} other_class(95)={} other_class(96)={}",
            m.other_class(10, &surfaces),
            m.other_class(95, &surfaces),
            m.other_class(96, &surfaces)
        );

        // What a rail grind actually posts: family base 95/96 as one material, the grind material
        // (143 → 10) as the other, and the same tier on both.
        for family_base in [95, 96] {
            let grind_material = 10;
            let first = m.sample(family_base, m.other_class(grind_material, &surfaces), 1);
            let second = m.sample(grind_material, m.other_class(family_base, &surfaces), 1);
            println!("family {family_base}: {first:?} / {second:?}");
            let (first, second) = (first.expect("board side"), second.expect("rail side"));
            // The claim this whole subsystem rests on: a rail grind is *two* voices out of *two*
            // banks — the board's family base from Skate_Collisions, the rail from Skate_Metal.
            assert_eq!(first.bank, skate_data::audio::splice::COLLISIONS_BANK);
            assert_eq!(second.bank, METAL_BANK);
            assert_ne!(first.sample, 0);
            assert_ne!(second.sample, 0);
        }
    }

    /// `sub_82496C58` against the real vault: a landing's level must actually vary with air time,
    /// and must be non-zero for ordinary surfaces — retail drops the whole message if either level
    /// is 0, so a zero here is silence.
    ///
    ///     cargo test -p skate-game --bin skate3rust -- --ignored the_landing_level --nocapture
    #[test]
    #[ignore = "needs the owner's assets"]
    fn the_landing_level_follows_the_air_time() {
        let assets =
            std::path::Path::new("C:/s3/installations/70eda9dc4644496d81ae73af95ff4285/assets");
        let vault = Collections::load(assets).expect("vault");
        let (m, _) = CollisionMaterials::load(&vault);
        let surfaces = SurfaceMap::load(&vault).expect("surface map");
        let board = 95;
        let (threshold, ceiling) = (0.1f32, 0.3f32);
        // `sub_824BA630` normalizes first — the windows are in `clamp(air / 0.4, 0, 1)` units,
        // not seconds, so the curve is spent by 0.12 s of air. An earlier version of this test
        // fed raw seconds and so measured a curve 2.5× wider than retail's.
        let divisor = 0.4f32;

        let level = |surface: i32, air: f32| {
            let air = (air / divisor).clamp(0.0, 1.0);
            let tier = i32::from(air >= threshold);
            let (lo, hi) = if tier == 0 {
                (0.0, threshold)
            } else {
                (threshold, ceiling)
            };
            (
                m.contact_level(surface, board, tier, lo, hi, air, &surfaces),
                m.contact_level(board, surface, tier, lo, hi, air, &surfaces),
            )
        };

        let mut varied = 0;
        for surface in [0, 2, 8, 10, 15, 16, 40] {
            // Seconds of air, chosen to straddle retail's real boundaries once normalized:
            // 0.04 s is the tier split and 0.12 s the ceiling, so 0.5 s is already saturated.
            let steps: Vec<_> = [0.0f32, 0.02, 0.04, 0.08, 0.12, 0.5]
                .iter()
                .map(|air| (*air, level(surface, *air)))
                .collect();
            println!("surface {surface}: {steps:?}");
            let surface_levels: Vec<u32> = steps.iter().map(|(_, (s, _))| *s).collect();
            if surface_levels.windows(2).any(|w| w[0] != w[1]) {
                varied += 1;
            }
        }
        // The whole point: a soft landing and a hard one must not produce the same level.
        assert!(varied > 0, "no surface varied its level with air time");
        println!("{varied} of 7 surfaces vary with air time");
    }

    #[test]
    fn releasing_both_records_deactivates_the_state() {
        let m = materials();
        let mut states = CollisionStates::new();
        let slot = states.post(message(0, 10, 0, 1), &m);
        assert_eq!(states.slots()[slot].started, [true, true]);
        states.release(slot, 0);
        assert!(states.slots()[slot].active);
        states.release(slot, 1);
        assert!(!states.slots()[slot].active);
        assert_eq!(states.slots()[slot].message, None);
        // And it stops driving its controller.
        let key = controller_key(slot);
        let mine: Vec<_> = states
            .drive()
            .iter()
            .filter(|(k, ..)| *k == key)
            .copied()
            .collect();
        assert_eq!(mine, vec![(key, GATE_INPUT, 0), (key, WEIGHT_INPUT, 0)]);
    }

    /// `sub_82497088`'s three bands, against class `7DAFF70B3A91CD5D`'s own `default` record
    /// (floor 0.005, mid 0.5, high 1.0, ceiling 2.0), the record used when a RefSpec does not
    /// resolve.
    ///
    /// The bands must be contiguous and ascending, because that property is what fixed the
    /// offset-to-field mapping in the first place.
    #[test]
    fn an_impact_chooses_the_band_and_window_sub_82497088_would() {
        let materials = CollisionMaterials::from_categories([0; MATERIAL_COUNT]);
        // Under the floor retail returns category 3, and the contact makes no sound at all.
        assert_eq!(materials.impact_band(0, 0.004), None);
        // The floor itself is not silent: the test is `impact < floor`, not `<=`.
        assert_eq!(materials.impact_band(0, 0.005), Some((0, 0.005, 0.5)));
        assert_eq!(materials.impact_band(0, 0.2), Some((0, 0.005, 0.5)));
        assert_eq!(materials.impact_band(0, 0.75), Some((1, 0.5, 1.0)));
        assert_eq!(materials.impact_band(0, 1.5), Some((2, 1.0, 2.0)));
        // Both splits are strict `>`, so a value sitting exactly on one stays in the lower band.
        assert_eq!(materials.impact_band(0, 0.5), Some((0, 0.005, 0.5)));
        assert_eq!(materials.impact_band(0, 1.0), Some((1, 0.5, 1.0)));
        // A material off the end of the table still answers, from the same default record.
        assert_eq!(materials.impact_band(NO_MATERIAL, 0.75), Some((1, 0.5, 1.0)));
        // The windows tile the range with no gap and no overlap.
        let bands: Vec<_> = [0.1, 0.75, 1.5]
            .iter()
            .map(|v| materials.impact_band(0, *v).expect("above the floor"))
            .collect();
        assert_eq!(bands[0].2, bands[1].1);
        assert_eq!(bands[1].2, bands[2].1);
    }

    /// Pins `sub_82497088`'s four thresholds against the owner's real vault.
    ///
    /// The offset-to-field mapping in [`IMPACT_FIELDS`] was recovered from the *structure* of the
    /// data — the three bands must tile ascending with no gap — so this is the assertion that
    /// keeps it honest against every material rather than against the one default record.
    ///
    ///     cargo test -p skate-game --bin skate3rust -- --ignored the_vault_impact_bands --nocapture
    #[test]
    #[ignore = "needs the owner's assets"]
    fn the_vault_impact_bands_tile_for_every_material() {
        let assets =
            std::path::Path::new("C:/s3/installations/70eda9dc4644496d81ae73af95ff4285/assets");
        let vault = Collections::load(assets).expect("vault");
        let (materials, _) = CollisionMaterials::load(&vault);
        let mut distinct = std::collections::BTreeSet::new();
        for material in 0..MATERIAL_COUNT as i32 {
            let bands = materials.entries[material as usize].bands;
            assert!(
                bands.floor < bands.mid && bands.mid < bands.high && bands.high <= bands.ceiling,
                "material {material} bands do not tile: {bands:?}"
            );
            // Just above the floor is band 0, and the top band must reach the ceiling.
            assert_eq!(materials.impact_band(material, bands.floor).map(|b| b.0), Some(0));
            assert_eq!(materials.impact_band(material, bands.ceiling).map(|b| b.0), Some(2));
            assert_eq!(materials.impact_band(material, bands.floor * 0.5), None);
            distinct.insert(bands.floor.to_bits());
        }
        // The body parts the on-board impact uses must all answer.
        for body in [97, 98, 99, 100, 107, 108, 109, 110, 111, 112] {
            assert!(
                materials.impact_band(body, 1.0).is_some(),
                "body material {body} has no impact band at all"
            );
        }
        println!("distinct impact floors across 143 materials: {}", distinct.len());
    }
}
