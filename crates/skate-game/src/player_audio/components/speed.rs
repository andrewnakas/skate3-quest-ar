//! The SenseOfSpeed component (vtable 0x822FCF08, SenseOfSpeed controller 40010080):
//! SenseOfSpeed_rattle (holder `+36`) and SenseOfSpeed_wind (holder `+40`).
//!
//! - Process `sub_824E7980` (slot 9), gated on the owner's local byte `[this+28]+72`:
//!   rattle posts (`sub_824B0520`, 48-byte object, 11 words) while the board ground speed
//!   `+208 × 3.6 ≥ low` and is released below it; wind posts (`sub_824B0388`, 56-byte object,
//!   13 words) while the COM speed `+212 × 3.6 ≥ low` and is released below it, with the bail
//!   pair of bounds while `+676`. Intensity = `fctiwz(clamp01((kmh − low) / (high − low)) × 1000)`.
//! - Update `sub_824E7CB0` (slot 10), same gate: rattle w0 (vault), w1 vf52(0), w2 vf56(2),
//!   w6 0, w7 vf60(1), w3; wind w0 (vault), w2 vf56(3), w6 0, w7 vf60(4), w3. The updater's
//!   intensity uses `fmsubs` (`v × 3.6 − low` fused, one rounding).
//!
//! The `x_jet_rolling.grain` "AudBoard Rocket" layer ([`Rocket`]), a grain player at `+100`:
//! - Loaded as a type-14 resource (`sub_824E7658`); the player is created by slot 7
//!   (`sub_824E78F0`).
//! - Process, between the rattle and the wind: above `+136` km/h (`7508154FF73DDCED`, 35) it binds
//!   the grain to the default output target `[[[0x830CFDBC]+44]]` (`sub_828EC040`).
//!   - It copies the GrainParams at `+104..+120` into the player: vault `D18D1174735E5CDE`, which
//!     `sub_824E80D0` copies over the constructor's constants (0.1, 0.4, 0.1, 2.4, 0.1).
//!   - It picks and starts a grain (`sub_828ECAB0`, `sub_828EC3F0`) and sets `+128`.
//! - At or below `+136` it stops the player twice (`sub_828EBF90`).
//! - Update, between the rattle and the wind, while `+128`: the player record is
//!   - gain `level(5)/32767 × +132 / 32767` (`9BC13FA19CC4DF00`, 22000);
//!   - pitch `pitch(3) / 4096`;
//!   - position `clamp01((v × 3.6 − +136) / (+140 − +136))`, the subtraction fused
//!     (`1185E9A69919B051`, 60).
//!
//! Tuning: `sub_824E80D0` reads holder `*(0x830CFDA4)+132` = class `6E878344774A7999`/`default`;
//! eEQChain from holder `+140` = `42AFE160E647167C`/`default`.

use skate_audio_core::fp::nmsub_single;
use skate_audio_core::grain::board::GrainRecord;
use skate_data::audio::grains::GrainMember;
use skate_data::collections::Collections;

use super::super::audio_state::AudioState;
use super::contacts::{clamp_word, vault_word};
use super::words::{KMH_PER_MS, THOUSAND, fctiwz};
use super::{Component, Controls, Tick, post, redeliver, release};

const TUNING_CLASS: &str = "Hash_6E878344774A7999";
const DEFAULT_KEY: &str = "Hash_D7EDBD362D7D2152";
const EQ_CLASS: &str = "Hash_42AFE160E647167C";

/// Packet lengths: object sizes minus the 4-byte header.
pub(crate) const RATTLE_WORDS: usize = 11;
pub(crate) const WIND_WORDS: usize = 13;
const RATTLE_OBJECT: &str = "SenseOfSpeed_rattle";
const WIND_OBJECT: &str = "SenseOfSpeed_wind";
/// `0x8231A844` (1.0), `0x82165A10` (0.0).
const ONE: f32 = f32::from_bits(0x3F80_0000);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpeedTuning {
    /// `+44` `EB224272A924C135` (rattle w0), `+48` `1BBA9174BD1B2D88` (wind w0).
    pub rattle_level: i32,
    pub wind_level: i32,
    /// `+52`/`+56` rattle bounds km/h `F57A74AFD22AD030` / `12275AA8AC4A63FB`.
    pub rattle_kmh: [f32; 2],
    /// `+60`/`+64` wind bounds `8B4DECD646B13210` / `73E9A42D88C51169`; `+68`/`+72` while
    /// bailing `3BDFAC298128131F` / `7BCB09DF227EDDAC`.
    pub wind_kmh: [f32; 2],
    pub bail_kmh: [f32; 2],
    /// Wind constructor w9..w12: `+76` `4381FEC9776F5EC9`, `+80` `F7A8067CAE4C51D3`, `+84`
    /// `D2715B3AD1BBCDB5`, `+88` `C689DD3434BC4170`.
    pub wind_words: [i32; 4],
    /// Rattle constructor w9, w10: `+92` `75F14A57A506E686`, `+96` `5F0E3DDE232C2BF3`.
    pub rattle_words: [i32; 2],
    /// eEQChain `4D30A6CA8C9E1ABC` (rattle w8), `C1DC8556BA66CDCD` (wind w8).
    pub rattle_eq: i32,
    pub wind_eq: i32,
}

impl SpeedTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let int = |name: &str| {
            vault
                .integer(TUNING_CLASS, DEFAULT_KEY, name)
                .map(|v| v as i32)
        };
        let float = |name: &str| vault.float(TUNING_CLASS, DEFAULT_KEY, name);
        Ok(Self {
            rattle_level: int("Hash_EB224272A924C135")?,
            wind_level: int("Hash_1BBA9174BD1B2D88")?,
            rattle_kmh: [
                float("Hash_F57A74AFD22AD030")?,
                float("Hash_12275AA8AC4A63FB")?,
            ],
            wind_kmh: [
                float("Hash_8B4DECD646B13210")?,
                float("Hash_73E9A42D88C51169")?,
            ],
            bail_kmh: [
                float("Hash_3BDFAC298128131F")?,
                float("Hash_7BCB09DF227EDDAC")?,
            ],
            wind_words: [
                int("Hash_4381FEC9776F5EC9")?,
                int("Hash_F7A8067CAE4C51D3")?,
                int("Hash_D2715B3AD1BBCDB5")?,
                int("Hash_C689DD3434BC4170")?,
            ],
            rattle_words: [int("Hash_75F14A57A506E686")?, int("Hash_5F0E3DDE232C2BF3")?],
            rattle_eq: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_4D30A6CA8C9E1ABC")? as i32,
            wind_eq: vault_word(vault, EQ_CLASS, DEFAULT_KEY, "Hash_C1DC8556BA66CDCD")? as i32,
        })
    }

    /// The wind bounds: the bail pair while `+676`.
    fn wind_bounds(&self, audio: &AudioState) -> [f32; 2] {
        if audio.bail_676 {
            self.bail_kmh
        } else {
            self.wind_kmh
        }
    }
}

/// `fsel` clamp to 0..1 then `× 1000`, `fctiwz`.
fn intensity_word(offset: f32, span: f32) -> i32 {
    let x = offset / span;
    let low = if -x >= 0.0 { 0.0 } else { x };
    let unit = if ONE - low >= 0.0 { low } else { ONE };
    fctiwz(unit * THOUSAND)
}

/// The process's intensity: `kmh` already rounded (`fmuls`), then `fsubs`, `fsubs`, `fdivs`.
pub(crate) fn process_intensity(kmh: f32, bounds: [f32; 2]) -> i32 {
    intensity_word(kmh - bounds[0], bounds[1] - bounds[0])
}

/// The updater's intensity: `fmsubs(speed, 3.6, low)` (fused), `fsubs`, `fdivs`.
pub(crate) fn update_intensity(speed: f32, bounds: [f32; 2]) -> i32 {
    intensity_word(speed.mul_add(KMH_PER_MS, -bounds[0]), bounds[1] - bounds[0])
}

/// `sub_824B0520`.
pub(crate) fn rattle_constructor(tuning: &SpeedTuning, intensity: i32) -> [u32; RATTLE_WORDS] {
    let mut words = [0; RATTLE_WORDS];
    words[2] = 4_096;
    words[3] = clamp_word(intensity, 0, 1_000);
    words[4] = 25_000;
    words[8] = clamp_word(tuning.rattle_eq, 0, 32_767);
    words[9] = clamp_word(tuning.rattle_words[0], 0, 32_767);
    words[10] = clamp_word(tuning.rattle_words[1], 0, 32_767);
    words
}

/// `sub_824B0388`.
pub(crate) fn wind_constructor(tuning: &SpeedTuning, intensity: i32) -> [u32; WIND_WORDS] {
    let mut words = [0; WIND_WORDS];
    words[2] = 4_096;
    words[3] = clamp_word(intensity, 0, 1_000);
    words[4] = 25_000;
    words[8] = clamp_word(tuning.wind_eq, 0, 32_767);
    for (i, level) in tuning.wind_words.iter().enumerate() {
        words[9 + i] = clamp_word(*level, 0, 32_767);
    }
    words
}

/// `sub_824E7CB0`'s rattle rewrite.
pub(crate) fn rattle_update(
    words: &mut [u32; RATTLE_WORDS],
    tuning: &SpeedTuning,
    audio: &AudioState,
    controls: &dyn Controls,
) {
    words[0] = clamp_word(tuning.rattle_level, 0, 32_767);
    words[1] = clamp_word(controls.raw(0) as i32, 0, 0xFFFF);
    words[2] = clamp_word(controls.pitch(2), 0, 8_192);
    words[6] = 0;
    words[7] = clamp_word(controls.level(1) as i32, 0, 32_767);
    words[3] = clamp_word(
        update_intensity(audio.ground_speed_208, tuning.rattle_kmh),
        0,
        1_000,
    );
}

/// `sub_824E7CB0`'s wind rewrite (w1 keeps the constructor's 0).
pub(crate) fn wind_update(
    words: &mut [u32; WIND_WORDS],
    tuning: &SpeedTuning,
    audio: &AudioState,
    controls: &dyn Controls,
) {
    words[0] = clamp_word(tuning.wind_level, 0, 32_767);
    words[2] = clamp_word(controls.pitch(3), 0, 8_192);
    words[6] = 0;
    words[7] = clamp_word(controls.level(4) as i32, 0, 32_767);
    words[3] = clamp_word(
        update_intensity(audio.com_speed_212, tuning.wind_bounds(audio)),
        0,
        1_000,
    );
}

/// The rocket layer's vault values (holder `+132`, class `6E878344774A7999`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RocketTuning {
    /// `+136` `7508154FF73DDCED`, `+140` `1185E9A69919B051`: km/h.
    pub start_kmh: f32,
    pub top_kmh: f32,
    /// `+132` `9BC13FA19CC4DF00` (an Int32, converted with `fcfid ; frsp`).
    pub gain: f32,
    /// `+104..+120` GrainParams `D18D1174735E5CDE`.
    pub params: [f32; 5],
}

impl RocketTuning {
    pub(crate) fn load(vault: &Collections) -> Result<Self, String> {
        let words = vault.words::<5>(TUNING_CLASS, DEFAULT_KEY, "Hash_D18D1174735E5CDE")?;
        Ok(Self {
            start_kmh: vault.float(TUNING_CLASS, DEFAULT_KEY, "Hash_7508154FF73DDCED")?,
            top_kmh: vault.float(TUNING_CLASS, DEFAULT_KEY, "Hash_1185E9A69919B051")?,
            gain: vault.integer(TUNING_CLASS, DEFAULT_KEY, "Hash_9BC13FA19CC4DF00")? as i32 as f32,
            params: words.map(f32::from_bits),
        })
    }
}

/// `sub_824E7980`'s rocket branch: `Some(true)` to start, `Some(false)` to stop.
pub(crate) fn rocket_step(kmh: f32, start_kmh: f32, running: bool) -> Option<bool> {
    if kmh > start_kmh {
        (!running).then_some(true)
    } else {
        running.then_some(false)
    }
}

/// `sub_824E7CB0`'s rocket record.
pub(crate) fn rocket_record(
    speed: f32,
    tuning: &RocketTuning,
    level_5: u32,
    pitch_3: i32,
) -> GrainRecord {
    const INV_32767: f32 = f32::from_bits(0x3800_0100);
    const INV_4096: f32 = f32::from_bits(0x3980_0000);
    // fmsubs f12 = v·3.6 − start, fused.
    let over = -nmsub_single(
        f64::from(speed),
        f64::from(KMH_PER_MS),
        f64::from(tuning.start_kmh),
    ) as f32;
    let ratio = over / (tuning.top_kmh - tuning.start_kmh);
    let clamped = if -ratio >= 0.0 { 0.0 } else { ratio };
    let position = if ONE - clamped >= 0.0 { clamped } else { ONE };
    let level = level_5 as i32 as f32 * INV_32767;
    GrainRecord {
        gain: tuning.gain * level * INV_32767,
        pitch: pitch_3 as f32 * INV_4096,
        position,
    }
}

/// The rocket grain player (`+100`) and its playing flag (`+128`).
pub(crate) struct Rocket {
    tuning: RocketTuning,
    player: u32,
    member: String,
    running: bool,
}

/// What one process call did.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SpeedEvents {
    pub post_rattle: Option<[u32; RATTLE_WORDS]>,
    pub post_wind: Option<[u32; WIND_WORDS]>,
    pub release_rattle: bool,
    pub release_wind: bool,
}

pub(crate) struct SenseOfSpeed {
    tuning: SpeedTuning,
    /// Owner byte `[this+28]+72`: the engine drives the local player only.
    local_72: bool,
    rattle: Option<(u32, [u32; RATTLE_WORDS])>,
    wind: Option<(u32, [u32; WIND_WORDS])>,
    rocket: Option<Rocket>,
}

impl SenseOfSpeed {
    pub(crate) fn new(vault: &Collections) -> Result<Self, String> {
        Ok(Self::with_tuning(SpeedTuning::load(vault)?))
    }

    fn with_tuning(tuning: SpeedTuning) -> Self {
        Self {
            tuning,
            local_72: true,
            rattle: None,
            wind: None,
            rocket: None,
        }
    }

    /// [`SenseOfSpeed::new`] plus the rocket layer: place `x_jet_rolling.grain` (from the loaded
    /// `grains.big` members) and create its player (`sub_824E7658`, `sub_824E78F0`).
    pub(crate) fn with_rocket(
        tick: &mut Tick,
        vault: &Collections,
        grains: &[GrainMember],
    ) -> Result<Self, String> {
        const MEMBER: &str = "x_jet_rolling.grain";
        let member = grains
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(MEMBER))
            .ok_or_else(|| format!("{MEMBER} is not loaded"))?;
        let mut g = tick.runtime.grains();
        g.load(
            &member.name,
            &member.bytes,
            member.samples.clone(),
            member.channels,
            member.rate,
        )
        .map_err(|e| e.to_string())?;
        let player = g.create_player().map_err(|e| e.to_string())?;
        let mut this = Self::new(vault)?;
        this.rocket = Some(Rocket {
            tuning: RocketTuning::load(vault)?,
            player,
            member: member.name.clone(),
            running: false,
        });
        Ok(this)
    }

    /// `sub_824E7980`'s rocket branch.
    fn rocket_process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let local = self.local_72;
        let Some(rocket) = self.rocket.as_mut().filter(|_| local) else {
            return Ok(());
        };
        let kmh = tick.audio.ground_speed_208 * KMH_PER_MS;
        let mut g = tick.runtime.grains();
        match rocket_step(kmh, rocket.tuning.start_kmh, rocket.running) {
            Some(true) => {
                let bus = g.root_bus().map_err(|e| e.to_string())?;
                g.bind(rocket.player, &rocket.member, rocket.tuning.params, bus)
                    .map_err(|e| e.to_string())?;
                rocket.running = true;
            }
            Some(false) => {
                g.stop(rocket.player).map_err(|e| e.to_string())?;
                g.stop(rocket.player).map_err(|e| e.to_string())?;
                rocket.running = false;
            }
            None => {}
        }
        Ok(())
    }

    /// `sub_824E7CB0`'s rocket block.
    fn rocket_update(&mut self, tick: &mut Tick) -> Result<(), String> {
        let local = self.local_72;
        let Some(rocket) = self.rocket.as_ref().filter(|r| r.running && local) else {
            return Ok(());
        };
        let c = tick.controls;
        let record = rocket_record(
            tick.audio.ground_speed_208,
            &rocket.tuning,
            c.level(5),
            c.pitch(3),
        );
        tick.runtime
            .grains()
            .set_record(rocket.player, record)
            .map_err(|e| e.to_string())
    }

    /// `sub_824E7980` (without the rocket grain).
    pub(crate) fn step_process(&self, audio: &AudioState) -> SpeedEvents {
        let mut events = SpeedEvents::default();
        if !self.local_72 {
            return events;
        }
        let kmh = audio.ground_speed_208 * KMH_PER_MS;
        if kmh >= self.tuning.rattle_kmh[0] {
            if self.rattle.is_none() {
                let intensity = process_intensity(kmh, self.tuning.rattle_kmh);
                events.post_rattle = Some(rattle_constructor(&self.tuning, intensity));
            }
        } else {
            events.release_rattle = self.rattle.is_some();
        }
        let bounds = self.tuning.wind_bounds(audio);
        let kmh = audio.com_speed_212 * KMH_PER_MS;
        if kmh >= bounds[0] {
            if self.wind.is_none() {
                events.post_wind = Some(wind_constructor(
                    &self.tuning,
                    process_intensity(kmh, bounds),
                ));
            }
        } else {
            events.release_wind = self.wind.is_some();
        }
        events
    }

    /// `sub_824E7CB0` (without the rocket grain): rewrite what is held.
    pub(crate) fn rewrite(&mut self, audio: &AudioState, controls: &dyn Controls) {
        if !self.local_72 {
            return;
        }
        if let Some((_, words)) = self.rattle.as_mut() {
            rattle_update(words, &self.tuning, audio, controls);
        }
        if let Some((_, words)) = self.wind.as_mut() {
            wind_update(words, &self.tuning, audio, controls);
        }
    }

    #[cfg(test)]
    fn apply_local(&mut self, events: &SpeedEvents) {
        if events.release_rattle {
            self.rattle = None;
        }
        if events.release_wind {
            self.wind = None;
        }
        if let Some(words) = events.post_rattle {
            self.rattle = Some((0, words));
        }
        if let Some(words) = events.post_wind {
            self.wind = Some((0, words));
        }
    }
}

impl Component for SenseOfSpeed {
    fn process(&mut self, tick: &mut Tick) -> Result<(), String> {
        let events = self.step_process(tick.audio);
        if events.release_rattle {
            let mut handle = self.rattle.take().map(|(handle, _)| handle);
            release(tick.runtime, &mut handle)?;
        }
        if let Some(words) = events.post_rattle {
            self.rattle = Some((post(tick.runtime, RATTLE_OBJECT, &words)?, words));
        }
        self.rocket_process(tick)?;
        if events.release_wind {
            let mut handle = self.wind.take().map(|(handle, _)| handle);
            release(tick.runtime, &mut handle)?;
        }
        if let Some(words) = events.post_wind {
            self.wind = Some((post(tick.runtime, WIND_OBJECT, &words)?, words));
        }
        Ok(())
    }

    fn update(&mut self, tick: &mut Tick) -> Result<(), String> {
        self.rewrite(tick.audio, tick.controls);
        if let Some((handle, words)) = self.rattle.as_ref() {
            redeliver(tick.runtime, *handle, words)?;
        }
        self.rocket_update(tick)?;
        if let Some((handle, words)) = self.wind.as_ref() {
            redeliver(tick.runtime, *handle, words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::contacts::capture::{self, Captured, Matches};
    use super::*;

    fn tuning() -> SpeedTuning {
        SpeedTuning {
            rattle_level: 32_767,
            wind_level: 27_000,
            rattle_kmh: [30.0, 80.0],
            wind_kmh: [15.0, 55.0],
            bail_kmh: [1.0, 10.0],
            wind_words: [32_767, 17_000, 17_000, 6_750],
            rattle_words: [32_767, 23_000],
            rattle_eq: 7,
            wind_eq: 7,
        }
    }

    #[test]
    fn constructors_match_retail_posts() {
        // Wind post, frame 2708 (w3 13); rattle post, frame 3754 (w3 8).
        assert_eq!(
            wind_constructor(&tuning(), 13),
            [0, 0, 4096, 13, 25000, 0, 0, 0, 7, 32767, 17000, 17000, 6750]
        );
        assert_eq!(
            rattle_constructor(&tuning(), 8),
            [0, 0, 4096, 8, 25000, 0, 0, 0, 7, 32767, 23000]
        );
    }

    #[test]
    fn posts_and_releases_at_the_low_bound_and_bail_switches_the_wind_pair() {
        let mut sos = SenseOfSpeed::with_tuning(tuning());
        // 20 km/h COM, 10 km/h ground: wind only.
        let audio = {
            let mut s = AudioState::default();
            s.com_speed_212 = 20.0 / 3.6;
            s.ground_speed_208 = 10.0 / 3.6;
            s
        };
        let events = sos.step_process(&audio);
        assert!(events.post_wind.is_some() && events.post_rattle.is_none());
        sos.apply_local(&events);
        // Slow COM releases wind, unless bailing (1..10 km/h).
        let slow = {
            let mut s = AudioState::default();
            s.com_speed_212 = 5.0 / 3.6;
            s
        };
        assert!(sos.step_process(&slow).release_wind);
        let bail = {
            let mut s = slow.clone();
            s.bail_676 = true;
            s
        };
        assert!(!sos.step_process(&bail).release_wind);
        let mut words = sos.wind.unwrap().1;
        wind_update(&mut words, &tuning(), &bail, &Captured::default());
        // (5 − 1) / (10 − 1) → 444.
        assert_eq!(words[3], 444);
    }

    #[test]
    fn update_intensity_is_fused() {
        // Both roundings agree away from ties; the fused form never differs by more than a unit.
        let v = f32::from_bits(0x4105_5555);
        assert!(
            (update_intensity(v, [30.0, 80.0]) - process_intensity(v * KMH_PER_MS, [30.0, 80.0]))
                .abs()
                <= 1
        );
    }

    /// Replays the retail capture: posts/releases at process F (state F) and the updates of
    /// F+1 (state F).
    #[test]
    #[ignore = "needs the retail capture in .local"]
    fn sense_of_speed_replays_the_retail_capture() {
        let Some(root) = capture::root() else { return };
        let states = capture::states(&root);
        let group = |object: &str| {
            let mut map = std::collections::BTreeMap::<(u32, String), Vec<capture::Row>>::new();
            for row in capture::rows(&root, object) {
                map.entry((row.frame, row.kind.clone()))
                    .or_default()
                    .push(row);
            }
            map
        };
        let (rattle, wind) = (group("SenseOfSpeed_rattle"), group("SenseOfSpeed_wind"));
        let get = |map: &std::collections::BTreeMap<(u32, String), Vec<capture::Row>>,
                   frame: u32,
                   kind: &str| {
            map.get(&(frame, kind.to_string()))
                .cloned()
                .unwrap_or_default()
        };
        let mut sos = SenseOfSpeed::with_tuning(tuning());
        let mut rattle_up = Matches::new("SenseOfSpeed_rattle update", RATTLE_WORDS);
        let mut wind_up = Matches::new("SenseOfSpeed_wind update", WIND_WORDS);
        let mut rattle_po = Matches::new("SenseOfSpeed_rattle post", RATTLE_WORDS);
        let mut wind_po = Matches::new("SenseOfSpeed_wind post", WIND_WORDS);
        let (mut timing_ok, mut timing_bad) = (0, Vec::new());
        let (mut fused_diff, mut plain_diff) = (0, 0);
        let first = *states.keys().next().unwrap();
        let last = *states.keys().last().unwrap();
        for frame in first + 1..=last {
            if let Some(state) = states.get(&(frame - 1)) {
                let audio = AudioState::from_capture(state);
                let reads: Vec<_> = get(&rattle, frame, "UP")
                    .iter()
                    .chain(get(&wind, frame, "UP").iter())
                    .flat_map(|r| r.reads.clone())
                    .collect();
                sos.rewrite(
                    &audio,
                    &Captured::from_reads(&reads, 0x824E_7CB0..0x824E_80D0),
                );
                for row in get(&rattle, frame, "UP") {
                    if let Some((_, words)) = sos.rattle.as_ref() {
                        rattle_up.add(frame, &row.words[..RATTLE_WORDS], words);
                        let plain = process_intensity(
                            audio.ground_speed_208 * KMH_PER_MS,
                            tuning().rattle_kmh,
                        )
                        .clamp(0, 1_000) as u32;
                        fused_diff += usize::from(words[3] != row.words[3]);
                        plain_diff += usize::from(plain != row.words[3]);
                    }
                }
                for row in get(&wind, frame, "UP") {
                    if let Some((_, words)) = sos.wind.as_ref() {
                        wind_up.add(frame, &row.words[..WIND_WORDS], words);
                    }
                }
            }
            if let Some(state) = states.get(&frame) {
                let audio = AudioState::from_capture(state);
                let events = sos.step_process(&audio);
                let (r_po, w_po) = (get(&rattle, frame, "PO"), get(&wind, frame, "PO"));
                let (r_rl, w_rl) = (get(&rattle, frame, "RL"), get(&wind, frame, "RL"));
                if let (Some(words), Some(row)) = (events.post_rattle, r_po.first()) {
                    rattle_po.add(frame, &row.words[..RATTLE_WORDS], &words);
                }
                if let (Some(words), Some(row)) = (events.post_wind, w_po.first()) {
                    wind_po.add(frame, &row.words[..WIND_WORDS], &words);
                }
                let ours = (
                    events.post_rattle.is_some(),
                    events.post_wind.is_some(),
                    events.release_rattle,
                    events.release_wind,
                );
                let retail = (
                    !r_po.is_empty(),
                    !w_po.is_empty(),
                    !r_rl.is_empty(),
                    !w_rl.is_empty(),
                );
                if ours == retail {
                    timing_ok += 1;
                } else if timing_bad.len() < 12 {
                    timing_bad.push((frame, ours, retail));
                }
                sos.apply_local(&events);
            }
        }
        rattle_po.print();
        wind_po.print();
        rattle_up.print();
        wind_up.print();
        println!("rattle w3 mismatches: fused {fused_diff}, unfused {plain_diff}");
        println!(
            "frames with matching post/release {timing_ok}; mismatches (frame, ours, retail) {timing_bad:?}"
        );
    }

    fn rocket_tuning() -> RocketTuning {
        // Vault class 6E878344774A7999/default.
        RocketTuning {
            start_kmh: 35.0,
            top_kmh: 60.0,
            gain: 22_000.0,
            params: [
                0x3DCC_CCCD,
                0x3ECC_CCCD,
                0x3DCC_CCCD,
                0x4019_999A,
                0x3DCC_CCCD,
            ]
            .map(f32::from_bits),
        }
    }

    #[test]
    fn rocket_starts_above_35_kmh_and_stops_at_or_below() {
        assert_eq!(rocket_step(35.1, 35.0, false), Some(true));
        assert_eq!(rocket_step(35.1, 35.0, true), None);
        assert_eq!(rocket_step(35.0, 35.0, true), Some(false));
        assert_eq!(rocket_step(20.0, 35.0, false), None);
    }

    /// Player records at the rocket's retail grain picks (`picks.tsv`, player `40C98FC0`): the
    /// state one frame before the update, the update's `level(5)` and `pitch(3)`.
    #[test]
    fn rocket_records_match_the_retail_picks() {
        let t = rocket_tuning();
        for (speed, level, pitch, words) in [
            (
                0x412D_AA75u32,
                1165u32,
                4086i32,
                [0x3CC3_8DA6u32, 0x3F7F_6000, 0x3E26_E788],
            ),
            (
                0x413A_68E6,
                1270,
                4086,
                [0x3CD5_2DA5, 0x3F7F_6000, 0x3E8E_2D18],
            ),
            (
                0x412B_062C,
                1295,
                4086,
                [0x3CD9_5FEE, 0x3F7F_6000, 0x3E0E_8EE3],
            ),
            (
                0x413D_A31D,
                1428,
                4086,
                [0x3CEF_B31E, 0x3F7F_6000, 0x3E9D_0C4B],
            ),
            (
                0x411E_BB0D,
                1332,
                4086,
                [0x3CDF_95DE, 0x3F7F_6000, 0x3CEA_1825],
            ),
        ] {
            let r = rocket_record(f32::from_bits(speed), &t, level, pitch);
            assert_eq!(
                [r.gain.to_bits(), r.pitch.to_bits(), r.position.to_bits()],
                words
            );
        }
    }

    /// Plays the rocket layer headlessly: above 35 km/h the grain binds and sounds, at 20 km/h it
    /// stops and the output falls silent.
    #[test]
    #[ignore = "needs the installed assets"]
    fn rocket_plays_headless() {
        use super::super::wheels::tests::Fixed;
        use skate_audio_core::authored::AuthoredRuntime;
        use skate_data::audio::catalog::PlayerAudioCatalog;
        let assets = std::path::PathBuf::from(
            r"C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets",
        );
        let cache = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("Skate3RustEngine/audio-pcm-cache"));
        let catalog = PlayerAudioCatalog::from_assets(&assets, cache.as_deref()).unwrap();
        let mut runtime =
            AuthoredRuntime::new(catalog.guest, catalog.projects, catalog.banks).unwrap();
        let vault = Collections::load(&assets).unwrap();
        let grains = skate_data::audio::grains::load_grains(
            &assets.join("private/stock/data/audio/grains.big"),
            &["x_jet_rolling.grain"],
            cache.as_deref(),
        )
        .unwrap();
        let controls = Fixed(&[(60, 5, 20_000), (56, 3, 4096)]);
        let mut audio = AudioState::default();
        let mut speed = {
            let mut tick = Tick {
                runtime: &mut runtime,
                audio: &audio,
                controls: &controls,
                dt: 1.0 / 60.0,
                tick: 0,
            };
            SenseOfSpeed::with_rocket(&mut tick, &vault, &grains).unwrap()
        };
        let mut rms = Vec::new();
        let mut blocks = 0.0f64;
        for frame in 0..180u64 {
            audio.ground_speed_208 = if (10..120).contains(&frame) {
                14.0
            } else {
                5.0
            };
            {
                let mut tick = Tick {
                    runtime: &mut runtime,
                    audio: &audio,
                    controls: &controls,
                    dt: 1.0 / 60.0,
                    tick: frame,
                };
                speed.rocket_process(&mut tick).unwrap();
                speed.rocket_update(&mut tick).unwrap();
            }
            blocks += 48_000.0 / 256.0 / 60.0;
            let (mut sum, mut n) = (0.0f64, 0usize);
            while blocks >= 1.0 {
                blocks -= 1.0;
                let pcm = runtime.pump_once().unwrap();
                sum += pcm
                    .iter()
                    .map(|s| f64::from(*s) * f64::from(*s))
                    .sum::<f64>();
                n += pcm.len();
            }
            rms.push((sum / n.max(1) as f64).sqrt());
        }
        let loud = rms[20..115].iter().filter(|r| **r > 1e-4).count();
        let after = rms[150..].iter().fold(0.0f64, |m, r| m.max(*r));
        println!(
            "frames with sound while above 35 km/h {loud}/95, max rms after the stop {after:e}"
        );
        assert!(loud > 85, "the rocket layer is silent");
        assert!(after < 1e-6, "the rocket layer keeps sounding");
    }
}
