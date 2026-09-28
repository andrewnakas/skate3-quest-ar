//! The front-end one-shots, which today means the session marker's three cellphone sounds.
//!
//! Retail plays these through `GlobalFEPlaySound` (`sub_825DFAF0`): `PlayerUI::UpdateSessionMarker`
//! (`sub_82898FC8`) builds a 64-bit AttribSys id inline and calls it, and that function looks the
//! id up in the vault and hands it to the front-end sound manager. `session_marker` already
//! emitted those three ids; the message simply had no consumer, which is why the marker has been
//! silent. It now travels as [`FrontEndSoundRequest`](crate::skate_audio::FrontEndSoundRequest).
//!
//! The ids are the AttribSys hashes of the vault keys, confirmed by hashing the names back:
//!
//! | id | `fe` key | sample |
//! |---|---|---|
//! | `0x0D6C88A3B91C828F` | `cellphone_place_marker` | 237 |
//! | `0x7F135F9FD28F7F21` | `cellphone_goto_marker` | 236 |
//! | `0x66B3AFE3B602918C` | `cellphone_marker_error` | 209 |
//! | `0x1A67C3E0088AC43D` | `multiplyer_2` | 249 |
//! | `0xE7F826661375333F` | `multiplyer_3` | 248 |
//!
//! The last two are the score multiplier's sounds (retail's own misspelling). They are played
//! from the TrickDisplay HUD module's update, `sub_82666BC0`, which inlines the tail of
//! `GlobalFEPlaySound` -- same singleton at `0x830CFDC4`, same vtable slot `+44` -- and so skips
//! the front-end gate `825DFAF0` applies. It polls the published sequence multiplier and, on any
//! change, selects by exact float equality:
//!
//! ```text
//! lfs f0,52(r31)      ; cached multiplier
//! lfs f31,140(r30)    ; current
//! fcmpu cr6,f0,f31
//! beq  cr6,0x82666c90 ; unchanged -> nothing
//! ...                 ; HUD event 10
//! lfs f0,3152(r9)     ; 0x82060C50 = 2.0
//! fcmpu cr6,f31,f0
//! bne  cr6,0x82666c50 ; -> try 3.0, else silent
//! ```
//!
//! Three consequences are retail behaviour, not omissions: **x1.5 makes no sound** (1.5 matches
//! neither constant), the test is **direction-agnostic** so a drop from x3 to x2 replays
//! `multiplyer_2`, and **losing the multiplier is silent** because 1.0 matches neither. There is
//! no `multiplyer_1`, no line-banked and no multiplier-lost record anywhere in the vault, and
//! `sub_82666BC0` is the only function in the executable that builds a `multiplyer_*` id.
//!
//! Each record is class `fe`, parent `fe_sfx`, and carries its sample in a field whose type name
//! is `sk8_menu` -- the same "a field's type name is its bank" convention the collision materials
//! use -- plus a `Volume`. Nothing here is hard-coded: the sample and gain are read from the
//! owner's own vault at load, so a modified install gets its own values.
//!
//! Not ported: retail gates `GlobalFEPlaySound` on three collection words (a front-end enable
//! state this engine has no equivalent for), so these always play.
use skate_data::collections::Collections;

/// `Volume`, and the sample field whose type name is the bank.
const VOLUME_FIELD: &str = "Hash_875BA75341DC8391";
const SAMPLE_FIELD: &str = "Hash_8FCC7EF9B9208858";
const FE_CLASS: &str = "fe";

/// One front-end sound, keyed by the id `session_marker` emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrontEndSound {
    /// `cellphone_place_marker`: the session marker was set.
    PlaceMarker,
    /// `cellphone_goto_marker`: the session returned to the marker.
    GotoMarker,
    /// `cellphone_marker_error`: the placement was refused.
    MarkerError,
    /// `multiplyer_2`: the sequence multiplier became exactly x2.
    MultiplierTwo,
    /// `multiplyer_3`: the sequence multiplier became exactly x3.
    MultiplierThree,
}

/// `multiplyer_2`, as `sub_82666BC0` builds it inline.
pub(crate) const MULTIPLIER_TWO_ID: u64 = 0x1a67_c3e0_088a_c43d;
/// `multiplyer_3`, as `sub_82666BC0` builds it inline.
pub(crate) const MULTIPLIER_THREE_ID: u64 = 0xe7f8_2666_1375_333f;

impl FrontEndSound {
    /// The AttribSys ids `PlayerUI::UpdateSessionMarker` and the TrickDisplay HUD update build
    /// inline.
    pub(crate) fn from_id(id: u64) -> Option<Self> {
        match id {
            0x0d6c_88a3_b91c_828f => Some(Self::PlaceMarker),
            0x7f13_5f9f_d28f_7f21 => Some(Self::GotoMarker),
            0x66b3_afe3_b602_918c => Some(Self::MarkerError),
            MULTIPLIER_TWO_ID => Some(Self::MultiplierTwo),
            MULTIPLIER_THREE_ID => Some(Self::MultiplierThree),
            _ => None,
        }
    }

    /// 82666BC0's selection, made only once the published multiplier has changed: exact float
    /// equality against the constants at `0x82060C50` (2.0) and `0x82063B08` (3.0). Every other
    /// value is silent in retail -- including x1.5, and the x1 a dropped line falls back to.
    ///
    /// Exact equality is retail's own test, so the multiplier must arrive bit-exact. It does:
    /// `ComboTimer::credit` assigns the authored level value straight through, and the authored
    /// levels are 1.5/2.0/3.0 exactly.
    pub(crate) fn for_multiplier(multiplier: f32) -> Option<Self> {
        match multiplier {
            m if m == 2.0 => Some(Self::MultiplierTwo),
            m if m == 3.0 => Some(Self::MultiplierThree),
            _ => None,
        }
    }

    /// The id its caller passes to the front-end sound manager.
    pub(crate) fn id(self) -> u64 {
        match self {
            Self::PlaceMarker => 0x0d6c_88a3_b91c_828f,
            Self::GotoMarker => 0x7f13_5f9f_d28f_7f21,
            Self::MarkerError => 0x66b3_afe3_b602_918c,
            Self::MultiplierTwo => MULTIPLIER_TWO_ID,
            Self::MultiplierThree => MULTIPLIER_THREE_ID,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::PlaceMarker => "cellphone_place_marker",
            Self::GotoMarker => "cellphone_goto_marker",
            Self::MarkerError => "cellphone_marker_error",
            Self::MultiplierTwo => "multiplyer_2",
            Self::MultiplierThree => "multiplyer_3",
        }
    }
}

/// What one front-end sound plays: a Splice sample in `sk8_menu.bnk`, and its authored gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrontEndVoice {
    pub sample: u16,
    pub gain: f32,
}

/// The front-end sounds as the owner's vault holds them.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrontEndSounds {
    place: Option<FrontEndVoice>,
    goto: Option<FrontEndVoice>,
    error: Option<FrontEndVoice>,
    multiplier_two: Option<FrontEndVoice>,
    multiplier_three: Option<FrontEndVoice>,
}

impl FrontEndSounds {
    /// A record this installation does not carry stays `None` and simply makes no sound, which is
    /// how retail's own lookup behaves when the id resolves to nothing.
    pub(crate) fn load(vault: &Collections) -> Self {
        let read = |sound: FrontEndSound| -> Option<FrontEndVoice> {
            let key = sound.key();
            let sample = vault
                .field(FE_CLASS, key, SAMPLE_FIELD)
                .ok()
                .and_then(|field| u32::from_str_radix(field.data.trim(), 16).ok())
                .and_then(|value| u16::try_from(value).ok())
                .filter(|sample| *sample != 0)?;
            // Retail treats the volume as an ordinary authored gain; a record without one plays
            // at unity rather than silently.
            let gain = vault.float(FE_CLASS, key, VOLUME_FIELD).unwrap_or(1.0);
            Some(FrontEndVoice { sample, gain })
        };
        Self {
            place: read(FrontEndSound::PlaceMarker),
            goto: read(FrontEndSound::GotoMarker),
            error: read(FrontEndSound::MarkerError),
            multiplier_two: read(FrontEndSound::MultiplierTwo),
            multiplier_three: read(FrontEndSound::MultiplierThree),
        }
    }

    pub(crate) fn voice(&self, sound: FrontEndSound) -> Option<FrontEndVoice> {
        match sound {
            FrontEndSound::PlaceMarker => self.place,
            FrontEndSound::GotoMarker => self.goto,
            FrontEndSound::MarkerError => self.error,
            FrontEndSound::MultiplierTwo => self.multiplier_two,
            FrontEndSound::MultiplierThree => self.multiplier_three,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three ids are the AttribSys hashes of the three vault keys. If either side is ever
    /// edited, this is what catches the mismatch — the ids are built inline in retail's code and
    /// cannot be re-derived from the vault alone.
    #[test]
    fn every_session_marker_id_maps_to_its_own_sound() {
        for (id, expected) in [
            (0x0d6c_88a3_b91c_828f, FrontEndSound::PlaceMarker),
            (0x7f13_5f9f_d28f_7f21, FrontEndSound::GotoMarker),
            (0x66b3_afe3_b602_918c, FrontEndSound::MarkerError),
            (MULTIPLIER_TWO_ID, FrontEndSound::MultiplierTwo),
            (MULTIPLIER_THREE_ID, FrontEndSound::MultiplierThree),
        ] {
            assert_eq!(FrontEndSound::from_id(id), Some(expected));
        }
        assert_eq!(FrontEndSound::from_id(0), None);
        // Every id is the AttribSys hash of its own key, so a typo in either half is caught
        // here rather than by a sound that silently never plays.
        for sound in [
            FrontEndSound::PlaceMarker,
            FrontEndSound::GotoMarker,
            FrontEndSound::MarkerError,
            FrontEndSound::MultiplierTwo,
            FrontEndSound::MultiplierThree,
        ] {
            let key = sound.key();
            assert_eq!(
                FrontEndSound::from_id(skate_data::attrib_hash::hash(key)),
                Some(sound),
                "{key}"
            );
        }
        // Distinct keys, so two sounds can never collapse onto one record.
        let keys = [
            FrontEndSound::PlaceMarker.key(),
            FrontEndSound::GotoMarker.key(),
            FrontEndSound::MarkerError.key(),
            FrontEndSound::MultiplierTwo.key(),
            FrontEndSound::MultiplierThree.key(),
        ];
        let unique: std::collections::BTreeSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len());
    }

    /// 82666BC0 compares the new multiplier with 2.0 and 3.0 by `fcmpu`, so anything else --
    /// x1, x1.5, or a value that merely rounds to 2.0 -- plays nothing. Retail has no x1.5
    /// sound and no "multiplier lost" sound; this test is what stops one being invented.
    #[test]
    fn only_exactly_two_and_three_select_a_multiplier_sound() {
        assert_eq!(
            FrontEndSound::for_multiplier(2.0),
            Some(FrontEndSound::MultiplierTwo)
        );
        assert_eq!(
            FrontEndSound::for_multiplier(3.0),
            Some(FrontEndSound::MultiplierThree)
        );
        for silent in [1.0, 1.5, 0.0, 2.5, 4.0, 1.9999999, 2.0000002, f32::NAN] {
            assert_eq!(
                FrontEndSound::for_multiplier(silent),
                None,
                "x{silent} should be silent"
            );
        }
        // The authored levels the combo timer assigns are the ones retail tests against.
        assert_eq!(f32::from_bits(0x4000_0000), 2.0);
        assert_eq!(f32::from_bits(0x4040_0000), 3.0);
        // Every id round-trips, so `id` and `from_id` cannot drift apart.
        for sound in [
            FrontEndSound::PlaceMarker,
            FrontEndSound::GotoMarker,
            FrontEndSound::MarkerError,
            FrontEndSound::MultiplierTwo,
            FrontEndSound::MultiplierThree,
        ] {
            assert_eq!(FrontEndSound::from_id(sound.id()), Some(sound));
        }
    }

    /// Against the owner's real vault: all three must resolve, to the samples the `fe` records
    /// carry, out of the `sk8_menu` bank.
    ///
    ///     cargo test -p skate-game --bin skate3rust -- --ignored the_vault_resolves_every_session_marker --nocapture
    #[test]
    #[ignore = "needs the owner's assets"]
    fn the_vault_resolves_every_session_marker_sound() {
        let assets =
            std::path::Path::new("C:/s3/installations/70eda9dc4644496d81ae73af95ff4285/assets");
        let vault = Collections::load(assets).expect("vault");
        let sounds = FrontEndSounds::load(&vault);
        for (sound, sample) in [
            (FrontEndSound::PlaceMarker, 237),
            (FrontEndSound::GotoMarker, 236),
            (FrontEndSound::MarkerError, 209),
            (FrontEndSound::MultiplierTwo, 249),
            (FrontEndSound::MultiplierThree, 248),
        ] {
            let voice = sounds
                .voice(sound)
                .unwrap_or_else(|| panic!("{sound:?} did not resolve"));
            assert_eq!(voice.sample, sample, "{sound:?}");
            assert!(voice.gain > 0.0, "{sound:?} gain {}", voice.gain);
            // The field's type name is the bank, which is what puts these in `sk8_menu.bnk`.
            let field = vault
                .field(FE_CLASS, sound.key(), SAMPLE_FIELD)
                .expect("sample field");
            assert_eq!(field.type_name, "sk8_menu", "{sound:?}");
        }
    }

    /// The bank half: `sk8_menu.bnk` must actually be in the archive and its three containers must
    /// resolve to real members. Without this the wiring would be silently inert -- an unresolvable
    /// optional bank is tolerated by design, so nothing else would complain.
    ///
    ///     cargo test -p skate-game --bin skate3rust -- --ignored the_menu_bank_resolves --nocapture
    #[test]
    #[ignore = "needs the owner's assets"]
    fn the_menu_bank_resolves_every_session_marker_sample() {
        use skate_audio_core::authored::oneshot::Rand;
        use skate_data::audio::splice::{SpliceBanks, SpliceState};
        let assets =
            std::path::Path::new("C:/s3/installations/70eda9dc4644496d81ae73af95ff4285/assets");
        let vault = Collections::load(assets).expect("vault");
        let sounds = FrontEndSounds::load(&vault);
        let bank = skate_data::audio::catalog::MENU_BANK;
        let banks = SpliceBanks::load(
            &assets.join("private/stock/data/audio/audiofiles.big"),
            &[bank],
            &[bank],
        )
        .expect("sk8_menu.bnk is in the archive");
        let mut state = SpliceState::default();
        let mut rand = Rand::new(1);
        for sound in [
            FrontEndSound::PlaceMarker,
            FrontEndSound::GotoMarker,
            FrontEndSound::MarkerError,
            FrontEndSound::MultiplierTwo,
            FrontEndSound::MultiplierThree,
        ] {
            let voice = sounds.voice(sound).expect("resolved");
            let members = banks
                .resolve(bank, voice.sample, &mut state, &mut rand)
                .unwrap_or_else(|e| panic!("{sound:?} sample {:#x}: {e}", voice.sample));
            assert!(
                !members.is_empty(),
                "{sound:?} sample {:#x} resolved to no members",
                voice.sample
            );
            println!(
                "{sound:?} sample {:#x} -> {} member(s), gain {:.3}",
                voice.sample,
                members.len(),
                voice.gain
            );
        }
    }

    /// Dump every member of the three containers, to see what actually plays.
    ///
    ///     cargo test -p skate-game --bin skate3rust -- --ignored dump_session_marker_members --nocapture
    #[test]
    #[ignore = "needs the owner's assets"]
    fn dump_session_marker_members() {
        use skate_audio_core::authored::oneshot::Rand;
        use skate_data::audio::splice::{SpliceBanks, SpliceState};
        let assets =
            std::path::Path::new("C:/s3/installations/70eda9dc4644496d81ae73af95ff4285/assets");
        let vault = Collections::load(assets).expect("vault");
        let sounds = FrontEndSounds::load(&vault);
        let bank = skate_data::audio::catalog::MENU_BANK;
        let banks = SpliceBanks::load(
            &assets.join("private/stock/data/audio/audiofiles.big"),
            &[bank],
            &[bank],
        )
        .expect("bank");
        for sound in [FrontEndSound::PlaceMarker, FrontEndSound::GotoMarker] {
            let voice = sounds.voice(sound).expect("resolved");
            // Resolve several times: a Splice container can be a random one-of, in which case the
            // membership changes between passes and playing "all members" would be wrong.
            for pass in 0..4 {
                let mut state = SpliceState::default();
                let mut rand = Rand::new(pass + 1);
                let members = banks
                    .resolve(bank, voice.sample, &mut state, &mut rand)
                    .expect("resolve");
                let summary: Vec<String> = members
                    .iter()
                    .map(|m| {
                        format!(
                            "rec{} s{:#x} off{:#x} gain{:.2} pitch{:.2} delay{:.3} pan{:.0}",
                            m.record,
                            m.sample,
                            m.stream_offset,
                            m.values.gain,
                            m.values.pitch,
                            m.values.delay,
                            m.pan
                        )
                    })
                    .collect();
                println!("{sound:?} pass{pass}: {} member(s)", members.len());
                for line in summary {
                    println!("    {line}");
                }
            }
        }
    }
}
