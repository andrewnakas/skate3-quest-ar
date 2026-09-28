//! Resolving a one-shot ("Splice") sound: which bank samples a sample id plays, and the vault
//! selection the wheel pops and the landing impact use.
//!
//! The bank format is `skate_audio_formats::splc`; the randomisation and the playback are
//! `skate_audio_core::authored::oneshot`. This module is the part between them: it walks a sample
//! id to its members exactly as `sub_82975700` and `sub_829757D0` do, in the retail draw order
//! (each child's probability first, then the container value, then each voice's pitch, gain and
//! delay), and it reads the pops and landing selection out of the vault.
//!
//! **Group state.** Retail keeps each group's and container's selection state in the bank's own
//! memory and mutates it in place (`sub_82976DD8`'s third argument). The banks here are placed in
//! guest memory read-only, so [`SpliceState`] holds those words host-side, keyed by the group's
//! offset in its bank. The effect is the same: the state persists per group for as long as the
//! bank is loaded.

use std::collections::HashMap;
use std::path::Path;

use skate_audio_core::authored::oneshot::{
    CONTACT_TRIM, MemberValues, Rand, VoiceValues, container_value, pick, plays, voice_values,
};
use skate_audio_formats::splc::{Group, Resolved, Splc};
use skate_audio_formats::eb;

use super::Error;
use crate::collections::Collections;

/// The vault class the pops and the landing read (`[[0x830CFDA4]+24]`).
pub const CONTACTS_CLASS: &str = "Hash_C26949FCB638A2CA";
/// `Sk8::Audio::eEQChain`.
pub const EQ_CLASS: &str = "Hash_42AFE160E647167C";
/// The bank the pops and the landing name through the vault field types.
pub const COLLISIONS_BANK: &str = "Skate_Collisions.bnk";
/// The pops' DLC bank, absent from a base installation.
pub const DLC_COLLISIONS_BANK: &str = "DLC_Cartoon_Collisions.bnk";

/// One member a sample id resolved to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedMember {
    /// The record the id resolved to.
    pub record: usize,
    /// Index into the bank's sample table.
    pub sample: u16,
    /// The sample's EAAC header, as an offset into the bank file: add the installed bank's base for
    /// the address the play command takes.
    pub stream_offset: u32,
    pub values: VoiceValues,
    /// The member's pan angle in degrees (`+16`); 0 when it is the unpanned marker −127.0.
    pub pan: f32,
}

/// The selection state of every group and container, by bank name and offset.
#[derive(Clone, Debug, Default)]
pub struct SpliceState(HashMap<(String, usize), u32>);

impl SpliceState {
    fn at(&mut self, bank: &str, offset: usize) -> &mut u32 {
        self.0.entry((bank.to_owned(), offset)).or_insert(0)
    }
}

/// The parsed `SPLC` banks a one-shot can play from.
#[derive(Debug)]
pub struct SpliceBanks {
    banks: Vec<(String, Vec<u8>, Splc)>,
}

impl SpliceBanks {
    /// Parse the named banks out of `audiofiles.big`; a name the archive does not hold is skipped
    /// when `required` does not list it.
    pub fn load(archive: &Path, names: &[&str], required: &[&str]) -> Result<Self, Error> {
        let data = std::fs::read(archive)
            .map_err(|e| Error::Format(format!("{}: {e}", archive.display())))?;
        let parsed = eb::Archive::parse(&data)?;
        let mut banks = Vec::new();
        for &name in names {
            let member = parsed.entries.iter().find(|e| {
                e.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            });
            let Some(member) = member else {
                if required.contains(&name) {
                    return Err(Error::Format(format!("missing sound bank {name}")));
                }
                continue;
            };
            let range = member.range();
            let bytes = data
                .get(range)
                .ok_or_else(|| Error::Format(format!("{name}: member out of bounds")))?
                .to_vec();
            let bank = Splc::parse(&bytes)?;
            banks.push((name.to_owned(), bytes, bank));
        }
        Ok(Self { banks })
    }

    /// The bank's raw member bytes, which a [`ResolvedMember`]'s `stream_offset` indexes into.
    /// Callers that want to decode a sample rather than play it need these.
    pub fn bank_bytes(&self, name: &str) -> Option<&[u8]> {
        self.banks
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, bytes, _)| bytes.as_slice())
    }

    pub fn bank(&self, name: &str) -> Option<&Splc> {
        self.banks
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, _, bank)| bank)
    }

    fn entry(&self, name: &str) -> Result<(&str, &[u8], &Splc), Error> {
        self.banks
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(n, bytes, bank)| (n.as_str(), bytes.as_slice(), bank))
            .ok_or_else(|| Error::Format(format!("sound bank {name} is not loaded")))
    }

    /// `sub_82975700` then `sub_829757D0`: the members a sample id plays, with the values
    /// `sub_82975A60`/`sub_82975CC8` give them. Members whose probability draw fails are left out,
    /// exactly as the retail walk skips them.
    pub fn resolve(
        &self,
        bank_name: &str,
        sample_id: u16,
        state: &mut SpliceState,
        rand: &mut Rand,
    ) -> Result<Vec<ResolvedMember>, Error> {
        let (name, bytes, bank) = self.entry(bank_name)?;
        let record = match bank.resolve(sample_id) {
            Resolved::Record(index) => index,
            Resolved::Container(index) => {
                let container = &bank.containers[index];
                let count = container.ids.len() as u8;
                let offset = 60 + 36 * bank.records.len() + 72 * index;
                let choice = pick(container.mode, count, state.at(name, offset), rand);
                let id = *container
                    .ids
                    .get(usize::from(choice))
                    .ok_or_else(|| Error::Format(format!("{name}: empty container {index}")))?;
                let index = usize::from(id);
                if index >= bank.records.len() {
                    return Err(Error::Format(format!(
                        "{name}: container id {id} is not a record"
                    )));
                }
                index
            }
        };

        // First pass, `sub_829757D0`: one probability draw per child, and the member it picked.
        let groups: Vec<Group> = bank.groups_of(record).to_vec();
        let mut chosen = Vec::new();
        for group in &groups {
            let choice = pick(group.mode, group.count, state.at(name, group.members_offset), rand);
            let member = bank.member(bytes, group, choice)?;
            let values = MemberValues {
                gain: member.gain,
                gain_range: member.gain_range,
                delay: member.delay,
                delay_range: member.delay_range,
                pitch_spread: member.pitch_spread,
                probability: member.probability,
            };
            if plays(&values, rand) {
                // `sub_82976020` treats −127.0 as "unpanned".
                let pan = if member.unknown_16 == -127.0 { 0.0 } else { member.unknown_16 };
                chosen.push((member.sample, values, pan));
            }
        }
        // `sub_82975A60` draws the container value before the voices' own values, and
        // `sub_82975B08` multiplies the caller's level word by it every frame: it scales the gain
        // the voice's `Gain` receives (`sub_82976360` posts `[voice+64] × level`).
        let level = container_value(
            bank.records[record].value_base,
            bank.records[record].value_range,
            rand,
        );
        let mut out = Vec::with_capacity(chosen.len());
        for (sample, values, pan) in chosen {
            let stream_offset = bank
                .stream_offset(sample)
                .ok_or_else(|| Error::Format(format!("{name}: sample {sample} is not in the table")))?;
            let mut values = voice_values(&values, rand);
            // The retail gain (member × the container's value), then the host trim the playtest
            // needs while the Splice graph's own levels are unrecoverable (`CONTACT_TRIM`).
            values.gain *= level * CONTACT_TRIM;
            out.push(ResolvedMember {
                record,
                sample,
                stream_offset: stream_offset as u32,
                values,
                pan,
            });
        }
        Ok(out)
    }
}

/// The vault values the wheel pops read (class `0xC26949FCB638A2CA`, key `default`).
#[derive(Clone, Debug, PartialEq)]
pub struct PopsTuning {
    /// `0xF2A1E273ABB8E9AB` (0.42) and `0xD7758385CDB8DC26` (0.25): the audio state's `+468`
    /// thresholds for classes 2 and 1.
    pub high: f32,
    pub low: f32,
    /// `0x3C1E3B965A93594A` (surface categories 0 and 1) and `0x84DFABF76D821DEC` (2 and 3), both
    /// `Skate_Collisions` sample ids, indexed by the class.
    pub samples: [Vec<u16>; 2],
    /// `0x056E414053859F6E`, the `DLC_Cartoon_Collisions` ids of the DLC variant.
    pub dlc_samples: Vec<u16>,
    /// `0xE34B48082B5BF185`, the eEQChain bus.
    pub bus: u32,
}

/// The vault values the landing impact reads.
#[derive(Clone, Debug, PartialEq)]
pub struct LandingTuning {
    /// `0x633FA94E39C1AE8F` — `Skate_Collisions` sample `0x447`. The impact itself, opened on the
    /// default output bus at a fixed level: retail passes `block[1] = 1.0`, and `sub_82975B08`
    /// (which would re-scale a voice by its container value each frame) is never called for a
    /// Contacts voice, so this one-shot's gain really is just its member gain. A landing is *not*
    /// made louder by being harder — see [`Self::ladder_sample`].
    pub sample: u16,
    /// `sub_824BA630` @ 0x824BAC54's second landing voice: a 2×2 ladder over the deck material
    /// test and time in air, which is how retail makes a hard landing *sound* different rather
    /// than louder. Indexed `[material_test][hard]`:
    /// `0x35C`/`0x35D` when the test is 0 (`0xF262042EAA295711` / `0x3A2F1C788E21D92D`),
    /// `0x35E`/`0x35F` otherwise (`0x1E86469556ACD80A` / `0xBF22DD8BC69DDE53`).
    pub ladder: [[u16; 2]; 2],
    /// `0x224B06562D9D5E0E` = 0.75 s — the time-in-air split between the soft and hard columns.
    pub ladder_seconds: f32,
    /// `sub_824BA3F0`'s four sample arrays, one per mode: `0x5A93802D11B00173`,
    /// `0xA0F86FEEA9C2412F`, `0x797EC34502499EC3`, `0xAB0D92058B4293A7` (13 ids each). This is the
    /// voice `sub_824B8D48` opens, and it is what actually makes one landing sound unlike another:
    /// the *level* is fixed, the *sample* changes. See [`Self::class_sample`].
    pub class_modes: [Vec<u16>; 4],
}

fn ids(vault: &Collections, class: &str, name: &str) -> Result<Vec<u16>, Error> {
    let field = vault
        .field(class, "default", name)
        .map_err(Error::Format)?;
    let array = field
        .array
        .as_ref()
        .ok_or_else(|| Error::Format(format!("{name} is not an array")))?;
    array
        .items
        .iter()
        .map(|item| {
            u32::from_str_radix(item.trim(), 16)
                .map(|value| value as u16)
                .map_err(|e| Error::Format(format!("{name}: {e}")))
        })
        .collect()
}

impl PopsTuning {
    pub fn load(vault: &Collections) -> Result<Self, Error> {
        let float = |name: &str| {
            vault
                .float(CONTACTS_CLASS, "default", name)
                .map_err(Error::Format)
        };
        Ok(Self {
            high: float("Hash_F2A1E273ABB8E9AB")?,
            low: float("Hash_D7758385CDB8DC26")?,
            samples: [
                ids(vault, CONTACTS_CLASS, "Hash_3C1E3B965A93594A")?,
                ids(vault, CONTACTS_CLASS, "Hash_84DFABF76D821DEC")?,
            ],
            dlc_samples: ids(vault, CONTACTS_CLASS, "Hash_056E414053859F6E")?,
            bus: vault
                .words::<1>(EQ_CLASS, "default", "Hash_E34B48082B5BF185")
                .map_err(Error::Format)?[0],
        })
    }

    /// `sub_824B9CC8`'s class: the audio state's `+468` against the two thresholds, with trick ids
    /// 33 and 34 forcing class 0.
    pub fn class(&self, jump_velocity_468: f32, trick: i32) -> usize {
        if trick == 33 || trick == 34 {
            return 0;
        }
        if jump_velocity_468 > self.high {
            2
        } else if jump_velocity_468 > self.low {
            1
        } else {
            0
        }
    }

    /// `sub_824B9AD8`: the bank and sample for a class and surface category. `dlc` is retail's
    /// special case (its global flag, a local player and player index 0), which plays the DLC bank's
    /// variant at bank index 8.
    pub fn sample(&self, class: usize, category: u8, dlc: bool) -> Option<(&'static str, u16)> {
        if dlc {
            return self
                .dlc_samples
                .get(class)
                .copied()
                .map(|id| (DLC_COLLISIONS_BANK, id));
        }
        let table = usize::from(category >= 2);
        self.samples[table]
            .get(class)
            .copied()
            .map(|id| (COLLISIONS_BANK, id))
    }
}

/// One `Skate_Collisions` id from the Contacts collection.
fn word(vault: &Collections, name: &str) -> Result<u16, Error> {
    Ok(vault
        .words::<1>(CONTACTS_CLASS, "default", name)
        .map_err(Error::Format)?[0] as u16)
}

impl LandingTuning {
    pub fn load(vault: &Collections) -> Result<Self, Error> {
        Ok(Self {
            sample: vault
                .words::<1>(CONTACTS_CLASS, "default", "Hash_633FA94E39C1AE8F")
                .map_err(Error::Format)?[0] as u16,
            ladder: [
                [
                    word(vault, "Hash_F262042EAA295711")?,
                    word(vault, "Hash_3A2F1C788E21D92D")?,
                ],
                [
                    word(vault, "Hash_1E86469556ACD80A")?,
                    word(vault, "Hash_BF22DD8BC69DDE53")?,
                ],
            ],
            ladder_seconds: f32::from_bits(
                vault
                    .words::<1>(CONTACTS_CLASS, "default", "Hash_224B06562D9D5E0E")
                    .map_err(Error::Format)?[0],
            ),
            class_modes: [
                ids(vault, CONTACTS_CLASS, "Hash_5A93802D11B00173")?,
                ids(vault, CONTACTS_CLASS, "Hash_A0F86FEEA9C2412F")?,
                ids(vault, CONTACTS_CLASS, "Hash_797EC34502499EC3")?,
                ids(vault, CONTACTS_CLASS, "Hash_AB0D92058B4293A7")?,
            ],
        })
    }

    /// `sub_824BA3F0`: the sample for the `sub_824B8D48` voice.
    ///
    /// The index is `3 × kind + class`; the mode is the surface category
    /// ([`surface_category`]) **except on a class-2 landing, where it is forced back to 0**. A
    /// landing calls `sub_824B8D48(this, 0, max_class, 0)`, i.e. `kind = 0`.
    ///
    /// The polarity is the retail one, read off the lifted code at `0x824BA420`
    /// (`skate3_recomp.11.cpp`), because it was inverted here until 2026-09-23 and that inversion
    /// silenced every heavy landing on three surface categories out of four:
    ///
    /// ```text
    /// cmpwi  cr6,r31,2        ; r31 = kind
    /// bgt    cr6,loc_824BA444 ; kind > 2 skips the mask entirely
    /// li     r11,2
    /// subfc  r9,r11,r25       ; r25 = class; carry = (class >= 2)
    /// subfe  r7,r10,r8        ; r7 = ~0 + carry -> 0 when carry, 0xFFFFFFFF when not
    /// and    r3,r7,r3         ; category &= r7
    /// ```
    ///
    /// `and` with 0 *clears* the category, so class 2 reads mode 0 and the lighter classes read
    /// their category — the opposite of what the comment here used to claim. It matters because
    /// `Skate_Collisions.bnk` authors the class-2 slot only in mode 0: modes 1, 2 and 3 point at
    /// containers 204, 193 and 215, whose records (`0x02a0..=0x02a2`, `0x0277..=0x0279`) carry
    /// **zero groups and a zero duration** — 27 such placeholder records in nine runs of three.
    /// Reading them for a heavy landing yields no members at all, which measured as a 1.2 s drop
    /// landing 10.5 dB below retail and *quieter than* a 0.8 s one.
    pub fn class_sample(&self, kind: u32, class: u32, category: u8) -> Option<u16> {
        let mode = if kind > 2 || class < 2 {
            usize::from(category)
        } else {
            0
        };
        let index = 3 * kind as usize + class as usize;
        self.class_modes.get(mode)?.get(index).copied()
    }

    /// The ladder sample for a landing: `test` is `sub_82494D78` of the deck material (0 or not),
    /// `air_seconds` the latched audio-state `+236`.
    pub fn ladder_sample(&self, test: bool, air_seconds: f32) -> u16 {
        self.ladder[usize::from(test)][usize::from(air_seconds >= self.ladder_seconds)]
    }

    /// The landing impact plays on the default output bus (`sub_824BA630` passes
    /// `[[[0x830CFDBC]+44]]`).
    pub fn bank(&self) -> &'static str {
        COLLISIONS_BANK
    }
}

/// `sub_824BA310`: the pops' surface category — the wheel material's `AudioSurfaceMap` lane `+8`
/// (`sub_82494D78`) and the soft-wheel test (`sub_824B23C8`).
pub fn surface_category(map_lane_8: u32, soft_wheels: bool) -> u8 {
    u8::from(soft_wheels) + if map_lane_8 != 0 { 2 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_category_combines_the_surface_lane_and_the_wheels() {
        assert_eq!(surface_category(0, false), 0);
        assert_eq!(surface_category(0, true), 1);
        assert_eq!(surface_category(1, false), 2);
        assert_eq!(surface_category(7, true), 3);
    }

    fn tuning() -> PopsTuning {
        // The owner's vault values.
        PopsTuning {
            high: 0.42,
            low: 0.25,
            samples: [vec![0x449, 0x44A, 0x44B], vec![0x44F, 0x450, 0x451]],
            dlc_samples: vec![0x15, 0x16, 0x16],
            bus: 0,
        }
    }

    #[test]
    fn the_pop_class_follows_the_thresholds_and_the_trick() {
        let t = tuning();
        assert_eq!(t.class(0.0, -1), 0);
        assert_eq!(t.class(0.25, -1), 0);
        assert_eq!(t.class(0.30, -1), 1);
        assert_eq!(t.class(0.42, -1), 1);
        assert_eq!(t.class(0.50, -1), 2);
        // Tricks 33 and 34 always take the quietest pop.
        assert_eq!(t.class(0.99, 33), 0);
        assert_eq!(t.class(0.99, 34), 0);
        assert_eq!(t.class(0.99, 35), 2);
    }

    #[test]
    /// `sub_824BA3F0`'s mode mask, in the direction the lifted code actually has it: the surface
    /// category selects the mode for a light or medium landing, and a class-2 landing is forced
    /// back to mode 0. The inverse — which this port shipped until 2026-09-23 — reads
    /// `Skate_Collisions.bnk`'s empty placeholder records for a heavy landing on categories 1..3
    /// and plays nothing at all.
    #[test]
    fn a_class_two_landing_reads_mode_zero_and_the_lighter_classes_read_their_category() {
        let landing = LandingTuning {
            sample: 0,
            ladder: [[0; 2]; 2],
            ladder_seconds: 0.75,
            class_modes: [
                (0..13).map(|i| 0x100 + i).collect(),
                (0..13).map(|i| 0x200 + i).collect(),
                (0..13).map(|i| 0x300 + i).collect(),
                (0..13).map(|i| 0x400 + i).collect(),
            ],
        };
        // kind 0, classes 0 and 1: the category picks the row.
        for category in 0..4u8 {
            let row = 0x100 + 0x100 * u16::from(category);
            assert_eq!(landing.class_sample(0, 0, category), Some(row));
            assert_eq!(landing.class_sample(0, 1, category), Some(row + 1));
        }
        // kind 0, class 2: every category collapses to mode 0.
        for category in 0..4u8 {
            assert_eq!(
                landing.class_sample(0, 2, category),
                Some(0x102),
                "class 2 must read mode 0 whatever the category"
            );
        }
        // `bgt cr6,loc_824BA444`: a kind above 2 skips the mask, so the category survives.
        assert_eq!(landing.class_sample(3, 2, 1), Some(0x20b));
    }

    fn the_sample_follows_the_class_the_category_and_the_dlc_case() {
        let t = tuning();
        assert_eq!(t.sample(0, 0, false), Some((COLLISIONS_BANK, 0x449)));
        assert_eq!(t.sample(2, 1, false), Some((COLLISIONS_BANK, 0x44B)));
        assert_eq!(t.sample(0, 2, false), Some((COLLISIONS_BANK, 0x44F)));
        assert_eq!(t.sample(1, 3, false), Some((COLLISIONS_BANK, 0x450)));
        assert_eq!(t.sample(1, 0, true), Some((DLC_COLLISIONS_BANK, 0x16)));
    }

    /// The vault must hold what the selection expects, including the field types that name the
    /// banks.
    #[test]
    #[ignore = "needs the installed assets"]
    fn the_vault_holds_the_pops_and_landing_selection() {
        let assets =
            std::path::PathBuf::from(r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets");
        let Ok(vault) = Collections::load(&assets) else { return };
        let pops = PopsTuning::load(&vault).unwrap();
        let landing = LandingTuning::load(&vault).unwrap();
        println!("pops {pops:?}\nlanding {landing:?}");
        assert_eq!(pops, tuning());
        assert_eq!(landing.sample, 0x447);
        for name in [
            "Hash_3C1E3B965A93594A",
            "Hash_84DFABF76D821DEC",
            "Hash_633FA94E39C1AE8F",
        ] {
            let field = vault.field(CONTACTS_CLASS, "default", name).unwrap();
            assert_eq!(field.type_name, "Skate_Collisions", "{name} names its bank");
        }
        let dlc = vault
            .field(CONTACTS_CLASS, "default", "Hash_056E414053859F6E")
            .unwrap();
        assert_eq!(dlc.type_name, "DLC_Cartoon_Collisions");
    }

    /// Resolving the pops' and the landing's ids against the real bank.
    #[test]
    #[ignore = "needs the installed assets"]
    fn resolves_the_pops_and_landing_samples() {
        let assets =
            std::path::PathBuf::from(r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets");
        let archive = assets.join("private/stock/data/audio/audiofiles.big");
        let Ok(banks) = SpliceBanks::load(&archive, &[COLLISIONS_BANK], &[COLLISIONS_BANK]) else {
            return;
        };
        let bank = banks.bank(COLLISIONS_BANK).unwrap();
        println!(
            "{}: {} records, {} containers, {} samples",
            bank.name,
            bank.records.len(),
            bank.containers.len(),
            bank.samples.len()
        );
        let mut state = SpliceState::default();
        let mut rand = Rand::new(1);
        for id in [0x447u16, 0x449, 0x44A, 0x44B, 0x44F, 0x450, 0x451] {
            let mut samples = std::collections::BTreeSet::new();
            for _ in 0..16 {
                let voices = banks
                    .resolve(COLLISIONS_BANK, id, &mut state, &mut rand)
                    .unwrap();
                assert!(!voices.is_empty(), "id {id:#x} resolved to nothing");
                for voice in &voices {
                    assert!(voice.values.gain > 0.0, "id {id:#x} gain {}", voice.values.gain);
                    assert!(voice.values.pitch > 0.0);
                    assert!(
                        usize::from(voice.sample) < bank.samples.len(),
                        "id {id:#x} sample {}",
                        voice.sample
                    );
                    samples.insert(voice.sample);
                }
            }
            println!("  {id:#x} -> samples {samples:?}");
        }
    }

    /// Plays a pop and a landing on the real runtime, each measured on its own: the samples must be
    /// the vault's, the levels must sit in the retail relationship (a pop is a quiet layer through
    /// the owner send, a landing is unity into the default output bus), and no voice may leak.
    #[test]
    #[ignore = "needs the installed assets"]
    fn plays_a_pop_and_a_landing_headlessly() {
        use skate_audio_core::authored::AuthoredRuntime;
        use skate_audio_core::authored::oneshot::{OneshotBus, OneshotVoice, RETAIL_POPS_LEVEL};
        let assets =
            std::path::PathBuf::from(r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets");
        let archive = assets.join("private/stock/data/audio/audiofiles.big");
        if !archive.exists() {
            return;
        }
        let cache = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("Skate3RustEngine/audio-pcm-cache"));
        let mut catalog =
            super::super::catalog::PlayerAudioCatalog::from_assets(&assets, cache.as_deref()).unwrap();
        catalog.load_splice_banks(&archive, cache.as_deref()).unwrap();
        let splice_samples = catalog
            .samples
            .iter()
            .filter(|s| s.bank.eq_ignore_ascii_case(COLLISIONS_BANK))
            .count();
        assert!(splice_samples > 1000, "only {splice_samples} collision samples decoded");
        // The decoded peak of every collision sample, by its offset in the bank.
        let mut peaks: HashMap<u32, f32> = HashMap::new();
        for sample in &catalog.samples {
            if sample.bank.eq_ignore_ascii_case(COLLISIONS_BANK) {
                let source = &sample.pcm.source;
                let mut peak = 0.0f32;
                for frame in 0..source.frames() {
                    for channel in 0..usize::from(source.channels()) {
                        peak = peak.max(source.sample(frame, channel).unwrap_or(0.0).abs());
                    }
                }
                peaks.insert(sample.header_offset, peak);
            }
        }
        let mut runtime =
            AuthoredRuntime::new(catalog.guest, catalog.projects, catalog.banks).unwrap();
        for sample in catalog.samples {
            let base = runtime.bank_base(&sample.bank).unwrap();
            runtime.insert_pcm(base + sample.header_offset, sample.pcm).unwrap();
        }
        let base = runtime.bank_base(COLLISIONS_BANK).unwrap();
        let banks = SpliceBanks::load(&archive, &[COLLISIONS_BANK], &[COLLISIONS_BANK]).unwrap();
        let vault = Collections::load(&assets).unwrap();
        let pops = PopsTuning::load(&vault).unwrap();
        let landing = LandingTuning::load(&vault).unwrap();
        let mut state = SpliceState::default();
        let mut rand = Rand::new(1);
        // The owner send bus the wheel pops play into, at the retail capture's level.
        let owner_send = runtime.build_owner_send(pops.bus, RETAIL_POPS_LEVEL).unwrap();
        runtime.pump_once().unwrap();
        let idle = runtime.stats().live_voices;

        let db = |x: f64| if x > 0.0 { 20.0 * x.log10() } else { -999.0 };
        let mut measure = |runtime: &mut AuthoredRuntime,
                           state: &mut SpliceState,
                           rand: &mut Rand,
                           label: &str,
                           sample_id: u16,
                           bus: OneshotBus| {
            let voices = banks.resolve(COLLISIONS_BANK, sample_id, state, rand).unwrap();
            assert!(!voices.is_empty(), "{label} resolved to nothing");
            let mut sample_peak = 0.0f32;
            for member in &voices {
                let peak = peaks.get(&member.stream_offset).copied().unwrap_or(0.0);
                sample_peak = sample_peak.max(peak);
                println!(
                    "  {label} sample {:<5} gain {:.3} pitch {:.3} delay {:.3} pan {:.0} sample peak {:.1} dBFS",
                    member.sample,
                    member.values.gain,
                    member.values.pitch,
                    member.values.delay,
                    member.pan,
                    db(f64::from(peak))
                );
            }
            let mut handles: Vec<_> = voices
                .iter()
                .map(|member| {
                    runtime
                        .play_oneshot(&OneshotVoice {
                            sample: base + member.stream_offset,
                            gain: member.values.gain.min(1.0),
                            pitch: member.values.pitch,
                            delay: member.values.delay,
                            pan: member.pan,
                            bus,
                        })
                        .unwrap()
                })
                .collect();
            let (mut peak, mut rms, mut frames) = (0.0f32, 0.0f64, 0usize);
            let mut blocks = 0.0f64;
            let mut live_peak = 0usize;
            for _ in 0..120 {
                for handle in handles.iter_mut() {
                    runtime.tick_oneshot(handle, 1.0 / 60.0).unwrap();
                }
                blocks += 48_000.0 / 256.0 / 60.0;
                while blocks >= 1.0 {
                    blocks -= 1.0;
                    let pcm = runtime.pump_once().unwrap();
                    for s in &pcm {
                        peak = peak.max(s.abs());
                        rms += f64::from(*s) * f64::from(*s);
                    }
                    frames += pcm.len();
                }
                live_peak = live_peak.max(runtime.stats().live_voices);
            }
            let rms = (rms / frames.max(1) as f64).sqrt();
            println!(
                "  {label}: {} voices, output peak {:.1} dBFS, rms {:.1} dBFS (sample peak {:.1} dBFS)",
                voices.len(),
                db(f64::from(peak)),
                db(rms),
                db(f64::from(sample_peak))
            );
            assert!(peak > 1e-4, "{label} is silent");
            assert!(
                handles.iter().all(|h| h.voice == 0),
                "{label} did not retire every voice"
            );
            (db(f64::from(peak)), db(rms))
        };

        // A hard pop (class 2 at +468 = 0.5) on a plain surface, through the owner send.
        let class = pops.class(0.5, -1);
        assert_eq!(class, 2);
        let (_, pop_id) = pops.sample(class, surface_category(0, false), false).unwrap();
        assert_eq!(pop_id, 0x44B);
        let (pop_peak, _) = measure(
            &mut runtime,
            &mut state,
            &mut rand,
            "pop",
            pop_id,
            OneshotBus::Module(owner_send),
        );
        // The landing impact, straight to the default output bus as `sub_824BA630` plays it.
        let (landing_peak, _) = measure(
            &mut runtime,
            &mut state,
            &mut rand,
            "landing",
            landing.sample,
            OneshotBus::Default,
        );
        println!("live voices idle {idle}, now {}", runtime.stats().live_voices);
        assert_eq!(runtime.stats().live_voices, idle, "the one-shots leaked voices");
        // The pops sit far below the landing, as the retail send level puts them.
        // The landing must carry over the rolling bed (−20..−24 dBFS peak in the game path) without
        // clipping. The pop stacks five layers, so it is only checked for clipping: its retail level
        // is MixMap-driven (see `RETAIL_POPS_LEVEL`) and cannot be reproduced yet.
        assert!(landing_peak < -6.0, "the landing clips: {landing_peak:.1} dBFS");
        assert!(landing_peak > -18.0, "the landing is lost under the bed: {landing_peak:.1} dBFS");
        assert!(pop_peak < -3.0, "the pop clips: {pop_peak:.1} dBFS");
    }
}
