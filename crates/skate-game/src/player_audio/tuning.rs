//! Vault tuning read by the retail PhysOut audio conditioner (`sub_82772748` and helpers), the
//! record builder `sub_827A1B78` and the audio-state bridge `sub_824B0DA8`.
//!
//! Every value is read at runtime from the owner's AttribSys vault through [`Collections`]. The
//! collections are the ones the retail code reaches through the audio tuning holder
//! `*(0x830CFDA4)` (built by `sub_8289D5C8`) or through the builder's own lookup; their
//! class/key identities were recovered from that constructor:
//!
//! | Holder slot | Class | Key | Reader |
//! |---|---|---|---|
//! | +24 | `C26949FCB638A2CA` | `default` (`D7EDBD362D7D2152`) | `sub_8279C948` |
//! | +36 | `6EBA5BCD3E38A98A` | `default` | holder |
//! | +84 | `C1831BDB6CB1B1EA` | `BA9837A6CF4C26ED` | holder |
//! | +92 | `C1831BDB6CB1B1EA` | `1ABD2984D7248589` | holder |
//! | +136 | `A867FBE3454326FF` | `default` | holder |
//! | +268 | `physics_collision` | `default` | holder (layout +164 read by `sub_82BD60C8`) |
//!
//! A missing field is an error, never a default: retail's fallback (`0x820D0850`) is not tuning.

use std::collections::HashMap;
use std::path::Path;

use skate_data::attrib_hash::{hash, numeric_name};
use skate_data::collections::{Collection, Collections};

/// Holder +24 (`sub_8279C948`): wheel landing tuning.
const WHEEL_CLASS: &str = "Hash_C26949FCB638A2CA";
const WHEEL_KEY: &str = "Hash_D7EDBD362D7D2152";
/// Holder +36: body slide tuning.
const BODY_SLIDE_CLASS: &str = "Hash_6EBA5BCD3E38A98A";
const BODY_SLIDE_KEY: &str = "Hash_D7EDBD362D7D2152";
/// Holder +84: slip tuning.
const SLIP_CLASS: &str = "Hash_C1831BDB6CB1B1EA";
const SLIP_KEY: &str = "Hash_BA9837A6CF4C26ED";
/// Holder +92: landing thresholds.
const LANDING_CLASS: &str = "Hash_C1831BDB6CB1B1EA";
const LANDING_KEY: &str = "Hash_1ABD2984D7248589";
/// Holder +136: jump thresholds.
const JUMP_CLASS: &str = "Hash_A867FBE3454326FF";
const JUMP_KEY: &str = "Hash_D7EDBD362D7D2152";
/// `eSk8AudioTricks` records, one collection per lowercase trick name (`sub_827A1B78`
/// before loc_827A1FE0: `sub_82B69B08(0x6918469984A8C596, hash64(lowercase name))`).
const TRICK_CLASS: &str = "Hash_6918469984A8C596";
/// Layout (`sub_82B6CC78`) +164, +172 (both `eSk8AudioTricks`) and byte +176 (Bool), per the
/// `skaterschema.vlt` class definition.
const TRICK_FIELD_164: &str = "Hash_8C3025DB4D1761AF";
const TRICK_FIELD_172: &str = "Hash_A2C5C22C5BE725F8";
const TRICK_FIELD_176: &str = "Hash_7B9298C7883E2FA2";

/// The builder's three reads of one `eSk8AudioTricks` collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AudioTrick {
    /// Layout +164 → record +180 → audio state +348.
    pub id_164: u32,
    /// Layout +172 → record +184 → audio state +352.
    pub id_172: u32,
    /// Layout byte +176 ≠ 0 → record +148 bit 23 → audio state +344.
    pub flag_176: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AudioTuning {
    /// `sub_82772E18` K1, holder +84 field `56D931D202881C2D`: slip divisor.
    pub slip_divisor: f32,
    /// `sub_82772E18` K2, holder +84 field `E6C3FFE8AA70F944`: slip offset.
    pub slip_offset: f32,
    /// `sub_82772B88`, holder +92 field `7385078DD3C063BA` elements 0, 1, 2: landing thresholds.
    pub landing_thresholds: [f32; 3],
    /// `sub_82772D30`, holder +136 field `468752B0BEE65CDB` elements 0, 1: jump thresholds.
    pub jump_thresholds: [f32; 2],
    /// `sub_82772FD8`, holder +24 field `3D399FE04952B425`: wheel touchdown speed divisor.
    pub wheel_impact_divisor: f32,
    /// `sub_824B2268`, holder +24 fields `6D3D91A9BA7ADCDC` / `2A70BB8A382574E4`: the per-wheel
    /// landing bucket thresholds (2 at or above `high`, 1 at or above `low`).
    pub wheel_bucket_high: f32,
    pub wheel_bucket_low: f32,
    /// `sub_82BD60C8` f4: layout +164 of `*(0x830CFDA4)`+268 (physics_collision/default,
    /// field `1430BD50F0A33475`), the ragdoll contact impact scale.
    pub body_impact_scale: f32,
    /// Bridge loop loc_824B18A8: holder +36 (class `6EBA5BCD3E38A98A`/default) layout +16 / +48,
    /// the eight x / y points of `8B164823E008749C` (PointNegGraphData8) that `sub_82481E10`
    /// evaluates at the previous COM speed to scale +496..+524.
    pub body_impact_speed_x: [f32; 8],
    pub body_impact_speed_y: [f32; 8],
    /// Audio trick records by EScorableID (0..332); `None` where the vault has no collection
    /// for that name (the builder then leaves +348/+352 at −1 and +344 clear).
    pub tricks: Vec<Option<AudioTrick>>,
}

impl AudioTuning {
    pub(crate) fn load(assets: &Path) -> Result<Self, String> {
        Self::from_collections(&Collections::load(assets)?)
    }

    pub(crate) fn from_collections(data: &Collections) -> Result<Self, String> {
        // The bridge (loc_824B11BC..11F8) reads Bool `642CF9BFEC6BE988` from holder +24. When set,
        // +448..+460 come from `sub_824B2350` (per-wheel air factor buckets); otherwise from the
        // builder's 2-bit touchdown buckets (`sub_827A0F58` over the builder's own +16
        // collection, whose identity is not recovered). Only the first path is ported.
        if !data.boolean(WHEEL_CLASS, WHEEL_KEY, "Hash_642CF9BFEC6BE988")? {
            return Err(
                "audio wheel landing path 642CF9BFEC6BE988 = false needs the builder's +16 \
                 collection, which is not recovered"
                    .into(),
            );
        }
        let graph = data
            .words::<20>(BODY_SLIDE_CLASS, BODY_SLIDE_KEY, "Hash_8B164823E008749C")?
            .map(f32::from_bits);
        let landing = data.float_items(LANDING_CLASS, LANDING_KEY, "Hash_7385078DD3C063BA")?;
        let jump = data.float_items(JUMP_CLASS, JUMP_KEY, "Hash_468752B0BEE65CDB")?;
        let landing_thresholds: [f32; 3] = landing
            .get(..3)
            .and_then(|items| items.try_into().ok())
            .ok_or("landing thresholds 7385078DD3C063BA need three elements")?;
        let jump_thresholds: [f32; 2] = jump
            .get(..2)
            .and_then(|items| items.try_into().ok())
            .ok_or("jump thresholds 468752B0BEE65CDB need two elements")?;
        Ok(Self {
            slip_divisor: data.float(SLIP_CLASS, SLIP_KEY, "Hash_56D931D202881C2D")?,
            slip_offset: data.float(SLIP_CLASS, SLIP_KEY, "Hash_E6C3FFE8AA70F944")?,
            landing_thresholds,
            jump_thresholds,
            wheel_impact_divisor: data.float(WHEEL_CLASS, WHEEL_KEY, "Hash_3D399FE04952B425")?,
            wheel_bucket_high: data.float(WHEEL_CLASS, WHEEL_KEY, "Hash_6D3D91A9BA7ADCDC")?,
            wheel_bucket_low: data.float(WHEEL_CLASS, WHEEL_KEY, "Hash_2A70BB8A382574E4")?,
            body_impact_scale: data.float(
                "physics_collision",
                "default",
                "Hash_1430BD50F0A33475",
            )?,
            body_impact_speed_x: graph[4..12].try_into().unwrap(),
            body_impact_speed_y: graph[12..20].try_into().unwrap(),
            tricks: audio_tricks(data)?,
        })
    }

    /// The builder's audio trick lookup for B60+152 (`sub_827A1B78`, before loc_827A2018):
    /// names outside 0..332 resolve to the empty string (`sub_82DA4E98`), which has no collection.
    pub(crate) fn trick(&self, scorable_id: i32) -> Option<AudioTrick> {
        usize::try_from(scorable_id)
            .ok()
            .and_then(|id| self.tricks.get(id).copied().flatten())
    }
}

/// `sub_82DA4E98` name (`*(0x820862A8 + 24·id + 20)`, the engine's `catalog::IDENTIFIERS`),
/// lowercased by `sub_82B69B68` before its AttribSys hash.
fn audio_tricks(data: &Collections) -> Result<Vec<Option<AudioTrick>>, String> {
    // One pass over the vault instead of a full scan per lookup; the resolution (class, key,
    // then parent chain) is the one `Collections::field` performs.
    let class = numeric_name(TRICK_CLASS);
    let by_key: HashMap<String, &Collection> = data
        .entries()
        .iter()
        .filter(|c| numeric_name(&c.class_name) == class)
        .map(|c| (numeric_name(&c.key), c))
        .collect();
    skate_core::scoring::catalog::IDENTIFIERS
        .iter()
        .map(|(name, _, _)| {
            let key = format!("Hash_{:016X}", hash(&name.to_ascii_lowercase()));
            let Some(collection) = by_key.get(&key) else {
                return Ok(None);
            };
            let lookup = |name: &str| field_data(trick_field(&by_key, collection, name)?, name);
            let word = |name: &str| -> Result<u32, String> {
                u32::from_str_radix(lookup(name)?.get(..8).ok_or("short trick word")?, 16)
                    .map_err(|e| e.to_string())
            };
            let flag = match lookup(TRICK_FIELD_176)?.get(..2) {
                Some("00") => false,
                Some("01") => true,
                _ => return Err(format!("invalid audio trick flag for {name}")),
            };
            Ok(Some(AudioTrick {
                id_164: word(TRICK_FIELD_164)?,
                id_172: word(TRICK_FIELD_172)?,
                flag_176: flag,
            }))
        })
        .collect()
}

fn trick_field<'a>(
    by_key: &HashMap<String, &'a Collection>,
    mut collection: &'a Collection,
    field: &str,
) -> Result<Option<&'a str>, String> {
    for _ in 0..=by_key.len() {
        if let Some(value) = collection.fields.get(field) {
            return Ok(Some(value.data.as_str()));
        }
        if collection.parent.is_empty() {
            return Ok(None);
        }
        collection = by_key
            .get(&numeric_name(&collection.parent))
            .ok_or_else(|| format!("missing audio trick parent {}", collection.parent))?;
    }
    Err("cyclic audio trick inheritance".into())
}

fn field_data<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, String> {
    value.ok_or_else(|| format!("audio trick collection lacks {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Option<std::path::PathBuf> {
        let root = std::path::PathBuf::from(
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets",
        );
        root.join("private/stock/skater-collections.json")
            .exists()
            .then_some(root)
    }

    /// Values recovered with `vlt.py` from the owner's vault; skipped without the assets.
    #[test]
    fn stock_vault_tuning_resolves_every_recovered_value() {
        let Some(root) = assets() else {
            return;
        };
        let tuning = AudioTuning::load(&root).unwrap();
        assert_eq!(tuning.slip_divisor, 45.0);
        assert_eq!(tuning.slip_offset, -0.75);
        assert_eq!(
            tuning.landing_thresholds.map(f32::to_bits),
            [0x3FC0_0000, 0x4026_6666, 0x405C_CCCD]
        );
        assert_eq!(
            tuning.jump_thresholds.map(f32::to_bits),
            [0x3EE6_6666, 0x3F40_0000]
        );
        assert_eq!(tuning.wheel_impact_divisor, 9.0);
        assert_eq!(tuning.wheel_bucket_high, 0.5);
        assert_eq!(tuning.wheel_bucket_low.to_bits(), 0x3E9E_B852);
        assert_eq!(tuning.body_impact_scale, 10.0);
        assert_eq!(tuning.body_impact_speed_x[1].to_bits(), 0x3EBB_9F41);
        assert_eq!(
            tuning.body_impact_speed_y,
            [
                1.0,
                1.2,
                f32::from_bits(0x3FBE_2BE0),
                f32::from_bits(0x3FF5_0753),
                f32::from_bits(0x401B_6DB5),
                3.6,
                f32::from_bits(0x4092_4921),
                5.0
            ]
        );
        assert_eq!(tuning.tricks.len(), 332);
        // Doc §4: ollie → 28, kickflip → 0.
        let id = |name: &str| {
            skate_core::scoring::catalog::IDENTIFIERS
                .iter()
                .position(|(n, _, _)| *n == name)
                .unwrap() as i32
        };
        assert_eq!(tuning.trick(id("ollie")).map(|t| t.id_164), Some(28));
        assert_eq!(tuning.trick(id("kickflip")).map(|t| t.id_164), Some(0));
        assert_eq!(tuning.trick(-1), None);
        assert_eq!(tuning.trick(332), None);
    }
}
