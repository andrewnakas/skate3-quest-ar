//! The Tricks component (vtable 0x822FC7B8, Tricks controller 40010050): Class_Flips and the two
//! cloth_trick messages.
//!
//! Component constructor `sub_824CBD98`: holders `+36` (Class_Flips), `+40` (cloth_trick A),
//! `+44` (cloth_trick B) empty; `+48` previous trick id, `+52` posted flip id, `+56` cloth A id,
//! `+60` latched `+352` id all −1; `+64` trick duration, `+68` cloth B countdown 0.0; `+72` the
//! w12 slew 0.
//!
//! Process `sub_824CBEB0(dt)` (slot 9), gated on the owner's local byte `[this+28]+72`:
//! `+60 = state+352` while `state+348 ≠ −1`; the flip poster `sub_824CBFB8`; cloth A
//! `sub_824CC590`; cloth B `sub_824CC680`; `+68` counts down by dt (to 0); `+48 = state+348`;
//! the w12 slew `sub_824CD170`; then `sub_824CD390`, which only raises trick events through
//! `sub_824AA858` (ids 24685..24690 to `[0x830CFDDC]`) and posts no message (not ported).
//!
//! Update `sub_824CBF78` (slot 10): `sub_824CC7D8` (flip release/rewrite), `sub_824CCE48`
//! (cloth A), `sub_824CCFE8` (cloth B).
//!
//! Tuning: holder `*(0x830CFDA4)+72` = class `C1831BDB6CB1B1EA`, key `tricks`
//! (`1FA8AC006CABEF59`); the slew's own lookup of `C1831BDB6CB1B1EA`/`47EC76B4F9FC79F6`;
//! holder `+140` = eEQChain class `42AFE160E647167C`/`default`.

use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::contacts::{clamp_word, vault_word};
use super::words::{fctiwz, flip_rate};
use super::{Component, Controls, Tick, post, redeliver, release};

const TUNING_CLASS: &str = "Hash_C1831BDB6CB1B1EA";
const TRICKS_KEY: &str = "Hash_1FA8AC006CABEF59";
const SLEW_KEY: &str = "Hash_47EC76B4F9FC79F6";
const EQ_CLASS: &str = "Hash_42AFE160E647167C";
const DEFAULT_KEY: &str = "Hash_D7EDBD362D7D2152";

/// Packet lengths: the 116-byte Class_Flips object (`sub_824AFAD8`) and the 48-byte
/// cloth_trick object (`sub_824B71C0`), minus the 4-byte header.
pub(crate) const FLIPS_WORDS: usize = 28;
pub(crate) const CLOTH_WORDS: usize = 11;
const FLIPS_OBJECT: &str = "Class_Flips";
const CLOTH_OBJECT: &str = "cloth_trick";

/// `0x820BD5C4`: time scale → word.
const TIME_SCALE: f32 = f32::from_bits(0x43FA_0000);
/// The hold path's forced trick id (`li r11,34`), and the ids the poster never posts.
const HOLD_TRICK: i32 = 34;
const UNPOSTED: [i32; 3] = [-1, 35, 36];
const NO_TRICK: i32 = -1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TricksTuning {
    /// Rotation-rate lanes x, y, z (state +480/+484/+488): divisor and dead-zone threshold,
    /// `1494BB20854C155C`/`5C73CF6A0D50C8D8`, `02D39586635FB1A3`/`EE157886DE5D3C97`,
    /// `8EDBACCBA6FE46AD`/`9A0316625B63CD99`.
    pub rate_divisor: [f32; 3],
    pub rate_threshold: [i32; 3],
    /// Constructor levels: w17 `99E6FF024834E4C7`, w18 `D2D0EBAC43842F6D`, w19
    /// `13C155A181A55BA4`, w21 `B565D4D763128252`.
    pub w17: i32,
    pub w18: i32,
    pub w19: i32,
    pub w21: i32,
    /// eEQChain `D9BE1F2F1A72FEE8` (flips w27) and `4B6E2D79A8452D9B` (cloth w10).
    pub eq_flips: i32,
    pub eq_cloth: i32,
    /// `sub_824CD170`: targets `B601DFAB3AF7DE66` (flag 0x8000), `08441EA8E8019665` (0x4000),
    /// `36F8D12486A929D1` (0x2000 or `sub_824898C8`); rates per second down
    /// `6B57BD44C0E0B267`, up `57AED5FBC374D8F1`.
    pub slew_targets: [i32; 3],
    pub slew_down: f32,
    pub slew_up: f32,
}

impl TricksTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let float = |name: &str| vault.float(TUNING_CLASS, TRICKS_KEY, name);
        let int = |name: &str| {
            vault
                .integer(TUNING_CLASS, TRICKS_KEY, name)
                .map(|v| v as i32)
        };
        let slew = |name: &str| {
            vault
                .integer(TUNING_CLASS, SLEW_KEY, name)
                .map(|v| v as i32)
        };
        Ok(Self {
            rate_divisor: [
                float("Hash_1494BB20854C155C")?,
                float("Hash_02D39586635FB1A3")?,
                float("Hash_8EDBACCBA6FE46AD")?,
            ],
            rate_threshold: [
                int("Hash_5C73CF6A0D50C8D8")?,
                int("Hash_EE157886DE5D3C97")?,
                int("Hash_9A0316625B63CD99")?,
            ],
            w17: int("Hash_99E6FF024834E4C7")?,
            w18: int("Hash_D2D0EBAC43842F6D")?,
            w19: int("Hash_13C155A181A55BA4")?,
            w21: int("Hash_B565D4D763128252")?,
            eq_flips: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_D9BE1F2F1A72FEE8")? as i32,
            eq_cloth: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_4B6E2D79A8452D9B")? as i32,
            slew_targets: [
                slew("Hash_B601DFAB3AF7DE66")?,
                slew("Hash_08441EA8E8019665")?,
                slew("Hash_36F8D12486A929D1")?,
            ],
            slew_down: vault.float(TUNING_CLASS, SLEW_KEY, "Hash_6B57BD44C0E0B267")?,
            slew_up: vault.float(TUNING_CLASS, SLEW_KEY, "Hash_57AED5FBC374D8F1")?,
        })
    }
}

/// Owner bytes and game-global inputs the Tricks component reads outside the audio state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TricksOwner {
    /// `[this+28]+72` (process/update gates) and `[this+16]+72` (w25, w26): the same local
    /// owner for the local player.
    pub local_72: bool,
    /// `[this+16]+64`, the player index (0 for the local player).
    pub index_64: i32,
    /// The SFX-pack game setting `[0x830CFDC4]+564` (Bool `1AC5BF7A05F82E62` of the pack
    /// record via `sub_824844B8`); off in the retail capture (w23 = 0), not modelled.
    pub sfx_pack_564: bool,
}

impl TricksOwner {
    pub(crate) const LOCAL: Self = Self {
        local_72: true,
        index_64: 0,
        sfx_pack_564: false,
    };
}

/// The game-global words `sub_824CD170` picks the w12 target from. Unidentified: the flags
/// word `*(0x83083C38)+0x2F0D0` (the audio frame record base `+0x2F070`, `+96`; bits 0x8000,
/// 0x4000, 0x2000) and the rest of `sub_824898C8` (`[0x830CFDC4]+932` with the byte at
/// `+476`, or game mode `+1060 == 8`). No writer was found and the engine has none, so both
/// stay clear and the target is 0. The retail capture shows targets 250/700/1000 during tricks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TrickEmphasis {
    pub flags_2f0d0: u32,
    pub mode_824898c8: bool,
}

/// `sub_824CD170`'s target: `sub_824898C8` (flag 0x2000, or the game-mode terms) first, then
/// flag 0x8000, then 0x4000.
pub(crate) fn emphasis_target(tuning: &TricksTuning, emphasis: TrickEmphasis) -> i32 {
    if emphasis.mode_824898c8 || emphasis.flags_2f0d0 & 0x2000 != 0 {
        tuning.slew_targets[2]
    } else if emphasis.flags_2f0d0 & 0x8000 != 0 {
        tuning.slew_targets[0]
    } else if emphasis.flags_2f0d0 & 0x4000 != 0 {
        tuning.slew_targets[1]
    } else {
        0
    }
}

/// `sub_824CD170`: slew `+72` toward the target, at most `fctiwz(up × dt)` up and
/// `fctiwz(down × dt)` down per call; `dt ≤ 0` (or NaN) zeroes it.
pub(crate) fn emphasis_slew(current: i32, target: i32, tuning: &TricksTuning, dt: f32) -> i32 {
    if !(dt > 0.0) {
        return 0;
    }
    let down = fctiwz(tuning.slew_down * dt);
    let up = fctiwz(tuning.slew_up * dt);
    if target < current {
        if current - target > down {
            current - down
        } else {
            target
        }
    } else if target > current {
        if target - current > up {
            current + up
        } else {
            target
        }
    } else {
        target
    }
}

/// The three dead-zoned rotation rates (x = +480 → w9, y = +484 → w8, z = +488 → w7).
fn rates(tuning: &TricksTuning, audio: &AudioState) -> [i32; 3] {
    std::array::from_fn(|lane| {
        flip_rate(
            audio.deck_angular_velocity_480[lane],
            tuning.rate_divisor[lane],
            tuning.rate_threshold[lane],
        ) as i32
    })
}

/// `(332 && 343) || (!332 && local && 310)` and whether the hold path (id 34) applies:
/// `sub_824CBFB8`'s post gate and `sub_824CC7D8`'s keep gate share the hold term.
fn hold_path(audio: &AudioState, owner: TricksOwner) -> bool {
    !audio.in_known_air_332 && owner.local_72 && audio.hold_expired_310
}

/// `sub_824CBFB8`'s gate `(332 && 343) || hold` and the id it stores at `+52` (34 on the hold
/// path, else `state+348`).
fn flip_gate(audio: &AudioState, owner: TricksOwner) -> Option<i32> {
    let hold = hold_path(audio, owner);
    (audio.in_known_air_332 && audio.trick_active_343 || hold).then_some(if hold {
        HOLD_TRICK
    } else {
        audio.audio_trick_348 as i32
    })
}

/// `sub_824CBFB8`: the id the poster posts this frame (`None` when it posts nothing).
pub(crate) fn flip_post_id(audio: &AudioState, owner: TricksOwner) -> Option<i32> {
    flip_gate(audio, owner).filter(|id| !UNPOSTED.contains(id))
}

/// `sub_824CBFB8` → `sub_824AFAD8`: the posted packet for trick `id`.
pub(crate) fn flips_constructor(
    tuning: &TricksTuning,
    audio: &AudioState,
    controls: &dyn Controls,
    owner: TricksOwner,
    id: i32,
) -> [u32; FLIPS_WORDS] {
    let [x, y, z] = rates(tuning, audio);
    let mut words = [0; FLIPS_WORDS];
    words[1] = 32_767;
    words[4] = 4_096;
    words[5] = 25_000;
    words[7] = clamp_word(z, 0, 1_000);
    words[8] = clamp_word(y, 0, 1_000);
    words[9] = clamp_word(x, 0, 1_000);
    words[10] = clamp_word(fctiwz(audio.time_scale_220 * TIME_SCALE), 0, 1_000);
    words[11] = clamp_word(id, 0, 40);
    words[16] = clamp_word(i32::from(!audio.paused_224), 0, 1);
    words[17] = clamp_word(tuning.w17, 0, 32_767);
    words[18] = clamp_word(tuning.w18, 0, 32_767);
    words[19] = clamp_word(tuning.w19, 0, 32_767);
    words[21] = clamp_word(tuning.w21, 0, 32_767);
    words[23] = clamp_word(i32::from(owner.sfx_pack_564), 0, 1);
    words[24] = clamp_word(i32::from(owner.local_72 && owner.index_64 == 0), 0, 1);
    words[25] = clamp_word(i32::from(owner.local_72), 0, 1);
    let level6 = if owner.local_72 {
        controls.level(6) as i32
    } else {
        0
    };
    words[26] = clamp_word(level6, 0, 32_767);
    words[27] = clamp_word(tuning.eq_flips, 0, 32_767);
    words
}

/// `sub_824CC7D8`: whether a held flip survives this update (else it is released).
pub(crate) fn flip_keeps(audio: &AudioState, owner: TricksOwner, posted_id: i32) -> bool {
    let hold = hold_path(audio, owner);
    let id = if hold {
        HOLD_TRICK
    } else {
        audio.audio_trick_348 as i32
    };
    (audio.in_known_air_332 || hold) && id == posted_id
}

/// `sub_824CC7D8`'s rewrite of a held flip.
pub(crate) fn flips_update(
    words: &mut [u32; FLIPS_WORDS],
    tuning: &TricksTuning,
    audio: &AudioState,
    controls: &dyn Controls,
    owner: TricksOwner,
    slew_72: i32,
) {
    let [x, y, z] = rates(tuning, audio);
    words[9] = clamp_word(x, 0, 1_000);
    words[8] = clamp_word(y, 0, 1_000);
    words[7] = clamp_word(z, 0, 1_000);
    words[10] = clamp_word(fctiwz(audio.time_scale_220 * TIME_SCALE), 0, 1_000);
    words[4] = clamp_word(controls.pitch(2), 0, 8_192);
    words[0] = clamp_word(controls.level(1) as i32, 0, 32_767);
    words[3] = clamp_word(controls.raw(0) as i32, 0, 0x1_0000);
    words[1] = 32_767;
    words[2] = 0;
    words[5] = clamp_word(controls.level(3) as i32, 0, 25_000);
    words[6] = 0;
    words[16] = clamp_word(i32::from(!audio.paused_224), 0, 1);
    let level6 = if owner.local_72 {
        controls.level(6) as i32
    } else {
        0
    };
    words[26] = clamp_word(level6, 0, 32_767);
    words[20] = clamp_word(controls.level(8) as i32, 0, 32_767);
    words[22] = clamp_word(controls.level(7) as i32, 0, 32_767);
    words[12] = clamp_word(slew_72, 0, 1_000);
}

/// `sub_824B71C0`: a cloth_trick packet for trick `id`.
pub(crate) fn cloth_constructor(tuning: &TricksTuning, id: i32) -> [u32; CLOTH_WORDS] {
    let mut words = [0; CLOTH_WORDS];
    words[2] = 4_096;
    words[4] = 25_000;
    words[8] = 1;
    words[9] = clamp_word(id, 0, 40);
    words[10] = clamp_word(tuning.eq_cloth, 0, 32_767);
    words
}

/// `sub_824CCE48` / `sub_824CCFE8`'s rewrite of a held cloth_trick (identical for A and B).
pub(crate) fn cloth_update(words: &mut [u32; CLOTH_WORDS], controls: &dyn Controls) {
    words[0] = 32_767;
    words[4] = 25_000;
    words[5] = 0;
    words[6] = 0;
    words[7] = clamp_word(controls.level(4) as i32, 0, 32_767);
    words[1] = clamp_word(controls.raw(0) as i32, 0, 0xFFFF);
    words[2] = clamp_word(controls.pitch(5), 0, 8_192);
}

pub(crate) struct Tricks {
    tuning: TricksTuning,
    owner: TricksOwner,
    /// See [`TrickEmphasis`]; stays clear.
    pub emphasis: TrickEmphasis,
    /// `+36`, `+40`, `+44`.
    flips: Option<(u32, [u32; FLIPS_WORDS])>,
    cloth_a: Option<(u32, [u32; CLOTH_WORDS])>,
    cloth_b: Option<(u32, [u32; CLOTH_WORDS])>,
    prev_id_48: i32,
    flip_id_52: i32,
    cloth_a_id_56: i32,
    latched_352_60: i32,
    duration_64: f32,
    countdown_68: f32,
    slew_72: i32,
}

impl Tricks {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self::with_tuning(TricksTuning::load(vault)?))
    }

    fn with_tuning(tuning: TricksTuning) -> Self {
        Self {
            tuning,
            owner: TricksOwner::LOCAL,
            emphasis: TrickEmphasis::default(),
            flips: None,
            cloth_a: None,
            cloth_b: None,
            prev_id_48: NO_TRICK,
            flip_id_52: NO_TRICK,
            cloth_a_id_56: NO_TRICK,
            latched_352_60: NO_TRICK,
            duration_64: 0.0,
            countdown_68: 0.0,
            slew_72: 0,
        }
    }
}

/// What one call did to the three messages, for the runtime glue and the capture replay.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TricksEvents {
    pub post_flips: Option<[u32; FLIPS_WORDS]>,
    pub post_cloth_a: Option<[u32; CLOTH_WORDS]>,
    pub post_cloth_b: Option<[u32; CLOTH_WORDS]>,
    pub release_flips: bool,
    pub release_cloth_a: bool,
    pub release_cloth_b: bool,
}

/// The runtime-free state machine; [`Component`] applies its events to the authored runtime.
impl Tricks {
    /// `sub_824CBEB0` (without the `sub_824CD390` event triggers).
    pub(crate) fn step_process(
        &mut self,
        audio: &AudioState,
        controls: &dyn Controls,
        dt: f32,
    ) -> TricksEvents {
        let mut events = TricksEvents::default();
        if !self.owner.local_72 {
            return events;
        }
        let id = audio.audio_trick_348 as i32;
        if id != NO_TRICK {
            self.latched_352_60 = audio.audio_trick_352 as i32;
        }
        // sub_824CBFB8: while nothing is held and the gate passes, +52 takes the id; ids −1, 35
        // and 36 are not posted.
        if self.flips.is_none() {
            if let Some(gate_id) = flip_gate(audio, self.owner) {
                self.flip_id_52 = gate_id;
                if !UNPOSTED.contains(&gate_id) {
                    events.post_flips = Some(flips_constructor(
                        &self.tuning,
                        audio,
                        controls,
                        self.owner,
                        gate_id,
                    ));
                }
            }
        }
        // sub_824CC590: cloth A.
        if id != NO_TRICK {
            if self.cloth_a.is_none() {
                events.post_cloth_a = Some(cloth_constructor(&self.tuning, id));
                self.cloth_a_id_56 = id;
            } else if id != self.cloth_a_id_56 {
                events.release_cloth_a = true;
            }
        }
        // sub_824CC680: cloth B.
        let mut trigger = false;
        if id == NO_TRICK {
            if self.prev_id_48 != NO_TRICK {
                self.countdown_68 = self.duration_64;
                self.duration_64 = 0.0;
                trigger = true;
            }
        } else {
            self.duration_64 += dt;
        }
        if self.countdown_68 > 0.0 {
            if self.latched_352_60 != NO_TRICK && trigger {
                if self.cloth_b.is_some() {
                    events.release_cloth_b = true;
                }
                events.post_cloth_b = Some(cloth_constructor(&self.tuning, self.latched_352_60));
            }
        } else if self.cloth_b.is_some() {
            events.release_cloth_b = true;
        }
        self.countdown_68 = if self.countdown_68 > 0.0 {
            self.countdown_68 - dt
        } else {
            0.0
        };
        self.prev_id_48 = id;
        self.emphasis.flags_2f0d0 = audio.multiplier_flags_2f0d0;
        let target = emphasis_target(&self.tuning, self.emphasis);
        self.slew_72 = emphasis_slew(self.slew_72, target, &self.tuning, dt);
        events
    }

    /// `sub_824CBF78`: releases, and rewrites of what stays held (applied to the held words).
    pub(crate) fn step_update(
        &mut self,
        audio: &AudioState,
        controls: &dyn Controls,
    ) -> TricksEvents {
        let mut events = TricksEvents::default();
        // sub_824CC7D8
        if self.flips.is_some() {
            if flip_keeps(audio, self.owner, self.flip_id_52) {
                if let Some((_, words)) = self.flips.as_mut() {
                    flips_update(
                        words,
                        &self.tuning,
                        audio,
                        controls,
                        self.owner,
                        self.slew_72,
                    );
                }
            } else {
                events.release_flips = true;
            }
        }
        // sub_824CCE48
        if self.cloth_a.is_some() {
            if audio.bail_676
                || (audio.audio_trick_348 as i32 == NO_TRICK && !audio.in_known_air_332)
            {
                events.release_cloth_a = true;
            } else if let Some((_, words)) = self.cloth_a.as_mut() {
                cloth_update(words, controls);
            }
        }
        // sub_824CCFE8
        if self.cloth_b.is_some() {
            if audio.bail_676 {
                events.release_cloth_b = true;
            } else if let Some((_, words)) = self.cloth_b.as_mut() {
                cloth_update(words, controls);
            }
        }
        events
    }

    /// Record a posted handle, or drop a released one, after the runtime applied `events`
    /// (a release is applied before a post of the same message).
    fn apply(
        &mut self,
        runtime: &mut skate_audio_core::authored::AuthoredRuntime,
        events: &TricksEvents,
    ) -> Result<(), String> {
        fn swap<const N: usize>(
            runtime: &mut skate_audio_core::authored::AuthoredRuntime,
            slot: &mut Option<(u32, [u32; N])>,
            released: bool,
            posted: Option<[u32; N]>,
            object: &str,
        ) -> Result<(), String> {
            if released {
                let mut handle = slot.take().map(|(handle, _)| handle);
                release(runtime, &mut handle)?;
            }
            if let Some(words) = posted {
                *slot = Some((post(runtime, object, &words)?, words));
            }
            Ok(())
        }
        swap(
            runtime,
            &mut self.flips,
            events.release_flips,
            events.post_flips,
            FLIPS_OBJECT,
        )?;
        swap(
            runtime,
            &mut self.cloth_a,
            events.release_cloth_a,
            events.post_cloth_a,
            CLOTH_OBJECT,
        )?;
        swap(
            runtime,
            &mut self.cloth_b,
            events.release_cloth_b,
            events.post_cloth_b,
            CLOTH_OBJECT,
        )
    }

    #[cfg(test)]
    fn apply_local(&mut self, events: &TricksEvents, next_handle: &mut u32) {
        fn swap<const N: usize>(
            slot: &mut Option<(u32, [u32; N])>,
            released: bool,
            posted: Option<[u32; N]>,
            next: &mut u32,
        ) {
            if released {
                *slot = None;
            }
            if let Some(words) = posted {
                *next += 1;
                *slot = Some((*next, words));
            }
        }
        swap(
            &mut self.flips,
            events.release_flips,
            events.post_flips,
            next_handle,
        );
        swap(
            &mut self.cloth_a,
            events.release_cloth_a,
            events.post_cloth_a,
            next_handle,
        );
        swap(
            &mut self.cloth_b,
            events.release_cloth_b,
            events.post_cloth_b,
            next_handle,
        );
    }
}

impl Component for Tricks {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let events = self.step_process(tick.audio, tick.controls, tick.dt);
        self.apply(tick.runtime, &events)
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        let events = self.step_update(tick.audio, tick.controls);
        self.apply(tick.runtime, &events)?;
        for (handle, words) in self.flips.iter() {
            redeliver(tick.runtime, *handle, words)?;
        }
        for (handle, words) in self.cloth_a.iter().chain(self.cloth_b.iter()) {
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::capture::{self, Captured, Matches};
    use super::*;

    pub(super) fn tuning() -> TricksTuning {
        TricksTuning {
            rate_divisor: [
                3.0,
                f32::from_bits(0x408C_CCCD),
                f32::from_bits(0x4123_3333),
            ],
            rate_threshold: [297, 703, 603],
            w17: 0x6BFE,
            w18: 0x3A00,
            w19: 0x1A00,
            w21: 10_000,
            eq_flips: 6,
            eq_cloth: 0,
            slew_targets: [250, 700, 1_000],
            slew_down: 1_000.0,
            slew_up: 10_000.0,
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

    fn air(id: i32, latched: i32, rates: [f32; 3]) -> AudioState {
        {
            let mut s = AudioState::default();
            s.in_known_air_332 = true;
            s.trick_active_343 = id != NO_TRICK;
            s.audio_trick_348 = id as u32;
            s.audio_trick_352 = latched as u32;
            s.deck_angular_velocity_480 = rates;
            s.time_scale_220 = 1.0;
            s
        }
    }

    #[test]
    fn flips_post_matches_the_retail_ollie() {
        // Retail post, frame 3848 (same-frame state): ollie (28), x rate −3.828 → 1000.
        let audio = air(28, NO_TRICK, [f32::from_bits(0xC074_FDF4), -0.085, -1.109]);
        assert_eq!(flip_post_id(&audio, TricksOwner::LOCAL), Some(28));
        let words = flips_constructor(
            &tuning(),
            &audio,
            &Fixed(&[(60, 6, 0)]),
            TricksOwner::LOCAL,
            28,
        );
        assert_eq!(
            words,
            [
                0, 32767, 0, 0, 4096, 25000, 0, 0, 0, 1000, 500, 28, 0, 0, 0, 0, 1, 0x6BFE, 0x3A00,
                0x1A00, 0, 10000, 0, 0, 1, 1, 0, 6
            ]
        );
    }

    #[test]
    fn flips_gate_skips_unposted_ids_and_forces_the_hold_id() {
        let owner = TricksOwner::LOCAL;
        assert_eq!(flip_post_id(&air(35, NO_TRICK, [0.0; 3]), owner), None);
        assert_eq!(flip_post_id(&air(36, NO_TRICK, [0.0; 3]), owner), None);
        let mut ground = {
            let mut s = AudioState::default();
            s.hold_expired_310 = true;
            s.audio_trick_348 = 5;
            s
        };
        assert_eq!(flip_post_id(&ground, owner), Some(HOLD_TRICK));
        assert!(flip_keeps(&ground, owner, HOLD_TRICK));
        ground.hold_expired_310 = false;
        assert!(!flip_keeps(&ground, owner, HOLD_TRICK));
        // Id change in the air releases.
        assert!(!flip_keeps(&air(38, 28, [0.0; 3]), owner, 3));
    }

    #[test]
    fn emphasis_slews_up_fast_and_down_slow() {
        let t = tuning();
        assert_eq!(emphasis_slew(0, 700, &t, 1.0 / 60.0), 166);
        assert_eq!(emphasis_slew(650, 700, &t, 1.0 / 60.0), 700);
        assert_eq!(emphasis_slew(700, 0, &t, 1.0 / 60.0), 684);
        assert_eq!(emphasis_slew(700, 0, &t, 0.0), 0);
        // 0x2000 (sub_824898C8) wins over 0x8000 and 0x4000.
        assert_eq!(
            emphasis_target(
                &t,
                TrickEmphasis {
                    flags_2f0d0: 0xE000,
                    mode_824898c8: false
                }
            ),
            1_000
        );
        assert_eq!(
            emphasis_target(
                &t,
                TrickEmphasis {
                    flags_2f0d0: 0xC000,
                    mode_824898c8: false
                }
            ),
            250
        );
        assert_eq!(
            emphasis_target(
                &t,
                TrickEmphasis {
                    flags_2f0d0: 0x4000,
                    mode_824898c8: false
                }
            ),
            700
        );
        assert_eq!(
            emphasis_target(
                &t,
                TrickEmphasis {
                    flags_2f0d0: 0x2000,
                    mode_824898c8: false
                }
            ),
            1_000
        );
        assert_eq!(
            emphasis_target(
                &t,
                TrickEmphasis {
                    flags_2f0d0: 0x8000,
                    mode_824898c8: true
                }
            ),
            1_000
        );
        assert_eq!(emphasis_target(&t, TrickEmphasis::default()), 0);
    }

    #[test]
    fn cloth_b_posts_the_latched_id_when_the_trick_ends_and_times_out() {
        let mut tricks = Tricks::with_tuning(tuning());
        let controls = Fixed(&[]);
        let mut handles = 0;
        let dt = 0.25;
        // Two frames of trick 3 (latched 352 = 28): cloth A posts on the first.
        let events = tricks.step_process(&air(3, 28, [0.0; 3]), &controls, dt);
        assert_eq!(events.post_cloth_a.map(|w| w[9]), Some(3));
        tricks.apply_local(&events, &mut handles);
        let events = tricks.step_process(&air(3, 28, [0.0; 3]), &controls, dt);
        tricks.apply_local(&events, &mut handles);
        // The id drops to −1: cloth B posts id 28, held for the 0.5 s duration.
        let events = tricks.step_process(&air(NO_TRICK, NO_TRICK, [0.0; 3]), &controls, dt);
        assert_eq!(events.post_cloth_b.map(|w| w[9]), Some(28));
        tricks.apply_local(&events, &mut handles);
        let events = tricks.step_process(&air(NO_TRICK, NO_TRICK, [0.0; 3]), &controls, dt);
        assert!(!events.release_cloth_b);
        tricks.apply_local(&events, &mut handles);
        let events = tricks.step_process(&air(NO_TRICK, NO_TRICK, [0.0; 3]), &controls, dt);
        assert!(events.release_cloth_b);
    }

    #[test]
    fn cloth_update_matches_a_retail_row() {
        let mut words = cloth_constructor(&tuning(), 28);
        cloth_update(
            &mut words,
            &Fixed(&[(56, 5, 0xFD0), (60, 4, 0x2D26), (52, 0, 0x1FF)]),
        );
        assert_eq!(
            words,
            [32767, 0x1FF, 0xFD0, 0, 25000, 0, 0, 0x2D26, 1, 28, 0]
        );
    }

    /// The bridge clock (+312) difference: the frame's dt.
    fn frame_dt(states: &std::collections::BTreeMap<u32, Vec<u32>>, frame: u32) -> f32 {
        match (states.get(&frame), states.get(&(frame - 1))) {
            (Some(now), Some(before)) => capture::float(now, 312) - capture::float(before, 312),
            _ => 0.0,
        }
    }

    /// Replays the retail capture: the component steps process and update on each frame's
    /// state (retail: update(F) reads state F−1, then the bridge, then process(F) reads state F,
    /// so process F and update F+1 see the same state). Compares posts, releases and every
    /// update's words; retail's w12 (the unidentified emphasis slew) is copied back in.
    #[test]
    #[ignore = "needs the retail capture in .local"]
    fn tricks_replays_the_retail_capture() {
        let Some(root) = capture::root() else { return };
        let states = capture::states(&root);
        let flips_rows = capture::rows(&root, "Class_Flips");
        let cloth_rows = capture::rows(&root, "cloth_trick");
        let by_frame = |rows: &[capture::Row],
                        kind: &str|
         -> std::collections::BTreeMap<u32, Vec<capture::Row>> {
            let mut map = std::collections::BTreeMap::<u32, Vec<capture::Row>>::new();
            for row in rows.iter().filter(|r| r.kind == kind) {
                map.entry(row.frame).or_default().push(row.clone());
            }
            map
        };
        let (flips_po, flips_up, flips_rl) = (
            by_frame(&flips_rows, "PO"),
            by_frame(&flips_rows, "UP"),
            by_frame(&flips_rows, "RL"),
        );
        let (cloth_po, cloth_up, cloth_rl) = (
            by_frame(&cloth_rows, "PO"),
            by_frame(&cloth_rows, "UP"),
            by_frame(&cloth_rows, "RL"),
        );
        let mut tricks = Tricks::with_tuning(tuning());
        let mut handles = 0;
        let mut flips_match = Matches::new("Class_Flips update", FLIPS_WORDS);
        let mut cloth_match = Matches::new("cloth_trick update", CLOTH_WORDS);
        let (mut flips_post_ok, mut flips_post_n, mut cloth_post_ok, mut cloth_post_n) =
            (0, 0, 0, 0);
        let (mut timing_ok, mut timing_bad) = (0usize, Vec::new());
        let first = *states.keys().next().unwrap();
        let last = *states.keys().last().unwrap();
        for frame in first..=last {
            // update(frame) with state frame−1
            if let Some(state) = states.get(&(frame - 1)) {
                let audio = AudioState::from_capture(state);
                let reads: Vec<_> = flips_up
                    .get(&frame)
                    .into_iter()
                    .flatten()
                    .chain(cloth_up.get(&frame).into_iter().flatten())
                    .flat_map(|r| r.reads.clone())
                    .collect();
                let controls = Captured::from_reads(&reads, 0x824C_C7D8..0x824C_D170);
                // Retail w12 (emphasis) cannot be computed; carry the retail value.
                if let (Some(rows), Some(_)) = (flips_up.get(&frame), tricks.flips.as_ref()) {
                    tricks.slew_72 = rows[0].words[12] as i32;
                }
                let events = tricks.step_update(&audio, &controls);
                let ours_rl = events.release_flips as usize
                    + events.release_cloth_a as usize
                    + events.release_cloth_b as usize;
                let retail_rl = flips_rl.get(&frame).map_or(0, Vec::len)
                    + cloth_rl.get(&frame).map_or(0, Vec::len);
                tricks.apply_local(&events, &mut handles);
                if let (Some(rows), Some((_, words))) =
                    (flips_up.get(&frame), tricks.flips.as_ref())
                {
                    flips_match.add(frame, &rows[0].words[..FLIPS_WORDS], words);
                }
                if let Some(rows) = cloth_up.get(&frame) {
                    for row in rows {
                        let a = Captured::from_reads(&row.reads, 0x824C_CE48..0x824C_CFE8);
                        let held = if a.0.is_empty() {
                            tricks.cloth_b.as_ref()
                        } else {
                            tricks.cloth_a.as_ref()
                        };
                        if let Some((_, words)) = held {
                            cloth_match.add(frame, &row.words[..CLOTH_WORDS], words);
                        }
                    }
                }
                // Releases in update(frame) vs retail releases at frame (process releases are
                // counted below).
                let process_state = states.get(&frame).map(|s| AudioState::from_capture(s));
                let dt = frame_dt(&states, frame);
                let pevents = process_state
                    .as_ref()
                    .map(|a| {
                        let reads: Vec<_> = flips_po
                            .get(&frame)
                            .into_iter()
                            .flatten()
                            .flat_map(|r| r.reads.clone())
                            .collect();
                        tricks.step_process(
                            a,
                            &Captured::from_reads(&reads, 0x824C_BFB8..0x824C_C590),
                            dt,
                        )
                    })
                    .unwrap_or_default();
                let ours_rl =
                    ours_rl + pevents.release_cloth_a as usize + pevents.release_cloth_b as usize;
                let ours_po = pevents.post_flips.is_some() as usize
                    + pevents.post_cloth_a.is_some() as usize
                    + pevents.post_cloth_b.is_some() as usize;
                let retail_po = flips_po.get(&frame).map_or(0, Vec::len)
                    + cloth_po.get(&frame).map_or(0, Vec::len);
                if ours_rl == retail_rl && ours_po == retail_po {
                    timing_ok += 1;
                } else if timing_bad.len() < 12 {
                    timing_bad.push((frame, ours_po, retail_po, ours_rl, retail_rl));
                }
                if let Some(words) = pevents.post_flips {
                    flips_post_n += 1;
                    if flips_po
                        .get(&frame)
                        .is_some_and(|rows| rows[0].words[..FLIPS_WORDS] == words)
                    {
                        flips_post_ok += 1;
                    }
                }
                for words in pevents
                    .post_cloth_a
                    .iter()
                    .chain(pevents.post_cloth_b.iter())
                {
                    cloth_post_n += 1;
                    if cloth_po.get(&frame).is_some_and(|rows| {
                        rows.iter().any(|r| r.words[..CLOTH_WORDS] == words[..])
                    }) {
                        cloth_post_ok += 1;
                    }
                }
                tricks.apply_local(&pevents, &mut handles);
            }
        }
        flips_match.print();
        cloth_match.print();
        println!(
            "flips posts exact {flips_post_ok}/{flips_post_n}, cloth posts exact {cloth_post_ok}/{cloth_post_n}"
        );
        println!(
            "frames with matching post/release counts {timing_ok}, first mismatches (frame, ours po, retail po, ours rl, retail rl) {timing_bad:?}"
        );
    }
}
