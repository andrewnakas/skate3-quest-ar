//! One-shot bank voices — the "Splice" sounds the game plays directly rather than through an
//! authored message: the wheel pops (`sub_824B9CC8`) and the four-wheel landing impact
//! (`sub_824BA630`) of `SFXObj_Contacts`.
//!
//! **What is ported here.** The parts of the retail path that are recoverable:
//!
//! | retail | here |
//! |---|---|
//! | `sub_82F4EAF0`, the generator every pick and range uses | [`Rand`] |
//! | `sub_82976DD8`, which alternative of a group or container to take | [`pick`] |
//! | `sub_82975CC8`, a voice's gain, pitch and delay | [`voice_values`] |
//! | `sub_829757D0`'s per-member probability | [`plays`] |
//! | `sub_82975A60`, the container's `+88` value | [`container_value`] |
//! | `sub_82975700`'s id → record/container resolution | `skate_audio_formats::splc::Splc::resolve` |
//! | `sub_824836B8`, free the children then the container | [`AuthoredRuntime::release_oneshot`], [`AuthoredRuntime::tick_oneshot`] |
//!
//! The bank side (the `SPLC` records, groups, members, containers and sample table) is decoded in
//! `skate_audio_formats::splc`, and `skate_data::audio::splice` resolves a sample id into the
//! voices to open.
//!
//! **What is substituted, and why.** The retail voice builds its own module graph in
//! `sub_82976020` from class pointers held in the bank runtime's globals at
//! `[0x83084248] + 4220..+4244`. That table is filled by `sub_82DE6678` and `sub_825F4B98` (two
//! boot functions this project does not port), and `[0x83084248]` is **0** in the guest image dump,
//! so the graph's module classes cannot be read out of the image: its shape is unrecoverable and is
//! **not** reproduced. Instead each resolved member is opened through the already-ported retail
//! device open (`sub_824A3140`, [`crate::device::open_voice_graph`]) — the same open the authored
//! patches use for a bank sample, which builds `SndPlayer1 → Rechannel → Resample → HighPassIir2 →
//! LowPassIir2 → [Send] → Gain → [Send] → Pan2D1 → Send`. The Splice gain reaches it as the open's
//! percentage byte, and the pitch is posted to the voice's `Resample`, which is what the retail
//! voice posts to its own `Rsp0` (`sub_824CEE00`'s counterpart, `sub_82975CC8`'s `+68`).
//!
//! **Where the level comes from.** A voice's `Gain` receives `[voice+64] × block[1]`
//! (`sub_82976360`): the member's own gain (`+8` plus a random `+48`, both linear and quantised to
//! half-decibel steps on the disc) times the caller's level word, which `sub_82975B08` rescales
//! every frame by the container's value. The wheel pops and the landing both pass 1.0, and the
//! collision samples are mastered at full scale, so a retail one-shot voice plays at unity.
//!
//! The pops do not send to a bus directly: `sub_824B9CC8` builds an owner-local six-channel send bus
//! with `sub_82488DD0` ([`AuthoredRuntime::build_owner_send`]) and the voice plays into its `Sub0`.
//! That bus's **first** send carries the owner's `level(14) / 32767` into the manager's intermediate
//! (environment) target, and its second send carries the dry signal to the eEQChain bus at the
//! class default. So `level(14)` is the pops' environment send, **not** an attenuation: the capture
//! measures it at 2590/32767 = 0.079 ([`RETAIL_POPS_LEVEL`]), 0 on many frames. The landing has no
//! such bus: `sub_824BA630` routes it straight to the default output bus.
//!
//! **Panning is what keeps a layered impact from stacking.** Each member carries a pan angle in
//! degrees at `+16` (0 and ±50 on these sounds, −127.0 meaning "unpanned", which `sub_82976020`
//! tests), and `sub_82976860` posts `member_pan × block[4] + block[2]` to the voice's panner every
//! frame. A landing plays three members at 0° and ±50°, so its layers spread rather than summing in
//! one channel. [`OneshotVoice::pan`] carries that angle into the open's panner.
//!
//! Also not ported, and not needed by a synchronous host: the bank runtime's 40-entry request
//! queue, its 60-slot voice table and the priority sort (`sub_82975668`, `sub_82975290`,
//! `sub_82975090`) that hand a queued voice to the audio thread. A member's start delay is kept
//! and counted down by [`AuthoredRuntime::tick_oneshot`] instead.

use crate::authored::AuthoredRuntime;
use crate::voice::{OpenRequest, VoiceDevice};
use crate::{Error, Result};

/// `lfs f29,-17516(r7)` (`0x8231BB94`): 1/32768, what a `rand()` result is scaled by.
pub const RAND_SCALE: f32 = f32::from_bits(0x3800_0000);
/// `0x82060C50`: 2.0, the pitch spread's symmetric scale.
const TWO: f32 = 2.0;
/// `0x82257308`: 4.0, the pitch of a member with no spread.
const NO_SPREAD_PITCH: f32 = 4.0;
/// `sub_824A3140` scales its byte by `0x820D71E8` = 0.01, so a gain reaches it as a percentage.
const PERCENT_MAX: f32 = 100.0;
/// **A deviation, not retail.** The absolute level of a retail Splice voice cannot be derived from
/// the image: `sub_82976360` posts the member's gain to the voice's second module, and which class
/// that is — and what the other modules' default levels are — lives in the bank runtime's globals at
/// `[0x83084248]+4220..+4244`, which are 0 in the dump (see the module note). Every recoverable part
/// of the level is applied (the member's gain and its random range, the container's value, the pan,
/// the owner send), and the collision samples are mastered at full scale, so a voice comes out at
/// 0 dBFS — about 20 dB above this engine's rolling bed, where it clips.
///
/// The trim is now set from a measurement rather than by ear. With the recomp's output pass metered
/// (`OUT` lines) and its music off, retail's own player mix over a session of ollies is: rolling
/// peak p50 −22.9 dBFS, takeoff −5.9 (+17.0 dB over rolling), landing +0.4 (+23.3 dB) — retail's
/// landings reach full scale. The same session in this engine measured rolling −22.7 (i.e. the bed
/// already matches retail) with the landing 14 dB short at this trim's earlier 0.125.
///
/// 0.625 raises a landing by the measured 14 dB, putting it at retail's level. It remains a
/// deviation only because the Splice graph's own levels are unrecoverable; when they are recovered
/// this should become 1.0 (or disappear).
pub const CONTACT_TRIM: f32 = 0.625;

/// The owner send level the retail capture shows the wheel pops using: `level(14)` = 2590 of
/// 32767, i.e. −22.0 dB (28 local pops, 0x0A15..0x0A1E). The MixMap produces it from the Contacts
/// controller's inputs 3, 6, 10, 17, 18 and 21, and yields 0 on many frames; a component that does
/// not drive those inputs should use this rather than a full-scale read.
pub const RETAIL_POPS_LEVEL: f32 = 2590.0 / 32767.0;

/// `sub_82F4EAF0`: the title's `rand()` — `seed = seed·0x343FD + 0x269EC3`, returning bits 16..30.
/// The seed is per-thread in retail and shared by every caller, so a retail *sequence* cannot be
/// reproduced (as with the grain player's generator); the recurrence and range are the retail ones.
#[derive(Clone, Debug)]
pub struct Rand(u32);

impl Default for Rand {
    fn default() -> Self {
        Self(1)
    }
}

impl Rand {
    pub fn new(seed: u32) -> Self {
        Self(seed)
    }

    /// The `rand()` result, 0..=32767.
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x0003_43FD).wrapping_add(0x0026_9EC3);
        (self.0 >> 16) & 0x7FFF
    }

    /// `rand() × 1/32768`, the unit value every range is taken against.
    pub fn unit(&mut self) -> f32 {
        self.next() as f32 * RAND_SCALE
    }
}

/// `sub_82976DD8`: which of `count` alternatives to take. `state` is the group's or container's own
/// state word, which retail keeps in the bank's memory and this updates the same way.
///
/// - one alternative: always the first;
/// - mode 0: `trunc(rand × count)`;
/// - mode 2: a shuffle — the state's high half-word is a mask of the alternatives left in the
///   current half (bit 0 of the state picks the half), and when it empties the half flips and the
///   mask refills;
/// - any other mode (1 on the disc): sequential, `state = (state + 1) mod count`.
pub fn pick(mode: u8, count: u8, state: &mut u32, rand: &mut Rand) -> u8 {
    if count == 1 {
        return 0;
    }
    if count == 0 {
        return 0;
    }
    let n = u32::from(count);
    match mode {
        0 => {
            let unit = rand.unit();
            let scaled = unit * count as f32;
            (scaled as u32).min(n - 1) as u8
        }
        2 => {
            let phase = *state & 1;
            let mut mask = (*state >> 16) & 0xFFFF;
            let half = n >> 1;
            let size = half + (phase & n & 1);
            let start = (rand.unit() * (size + 1) as f32) as u32;
            if size == 0 {
                return 0;
            }
            let mut found = None;
            for k in 0..size {
                let slot = (k + start) % size;
                if mask & (1 << slot) != 0 {
                    found = Some(slot);
                    break;
                }
            }
            let Some(slot) = found else {
                return 0;
            };
            mask &= !(1 << slot);
            let index = slot + if phase != 0 { half } else { 0 };
            let mut phase = phase;
            if mask == 0 {
                // The half is exhausted: flip it and refill the mask with that half's items.
                phase = u32::from(phase == 0);
                let size = half + (phase & n & 1);
                mask = (1u32 << size) - 1;
            }
            *state = (mask << 16) | phase;
            index.min(n - 1) as u8
        }
        _ => {
            let next = (*state).wrapping_add(1) % n;
            *state = next;
            next as u8
        }
    }
}

/// The member fields the randomisation reads (`skate_audio_formats::splc::Member`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemberValues {
    /// `+8`, `+48`.
    pub gain: f32,
    pub gain_range: f32,
    /// `+20`, `+52`.
    pub delay: f32,
    pub delay_range: f32,
    /// `+44`.
    pub pitch_spread: f32,
    /// `+64`.
    pub probability: f32,
}

/// What `sub_82975CC8` stores on a voice: its gain (`+64`/`+72`), pitch (`+68`) and start delay
/// (`+80`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceValues {
    pub gain: f32,
    pub pitch: f32,
    pub delay: f32,
}

/// `sub_82975CC8`. The draws happen in the retail order: pitch, gain, then the delay range.
pub fn voice_values(member: &MemberValues, rand: &mut Rand) -> VoiceValues {
    let spread = member.pitch_spread;
    let below = 1.0 - spread;
    // fmsubs: rand·2 − 1, one rounding.
    let t = (f64::from(rand.unit()).mul_add(f64::from(TWO), -1.0)) as f32;
    let pitch = if t > 0.0 {
        if spread == 0.0 {
            NO_SPREAD_PITCH
        } else {
            (f64::from(1.0 / spread - 1.0).mul_add(f64::from(t), 1.0)) as f32
        }
    } else {
        // fnmsubs: 1 − (−t)·(1 − spread).
        (-(f64::from(-t).mul_add(f64::from(below), -1.0))) as f32
    };
    let gain = (f64::from(rand.unit())
        .mul_add(f64::from(member.gain_range), f64::from(member.gain))) as f32;
    let mut delay = if member.delay == 0.0 {
        0.0
    } else {
        member.delay
    };
    if member.delay_range != 0.0 {
        delay = (f64::from(rand.unit()).mul_add(f64::from(member.delay_range), f64::from(delay)))
            as f32;
    }
    VoiceValues { gain, pitch, delay }
}

/// `sub_829757D0`'s probability test: the member is skipped when the draw exceeds `+64`.
pub fn plays(member: &MemberValues, rand: &mut Rand) -> bool {
    !(rand.unit() > member.probability)
}

/// `sub_82975A60`: the value the container keeps at `+88`, `base + rand × range`.
pub fn container_value(base: f32, range: f32, rand: &mut Rand) -> f32 {
    (f64::from(rand.unit()).mul_add(f64::from(range), f64::from(base))) as f32
}

/// Where a one-shot's output goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneshotBus {
    /// The default output (`[0x830775EC]`), what the landing impact plays on
    /// (`sub_824BA630` passes `[[[0x830CFDBC]+44]]`).
    Default,
    /// An `eEQChain` bus index, resolved by `sub_82491108` as the open's routing record does
    /// (the pops' vault bus `0xE34B48082B5BF185`).
    EqChain(u32),
    /// Into a module built beforehand: the owner send bus of [`AuthoredRuntime::build_owner_send`],
    /// which is how the wheel pops reach their bus (`sub_829757D0` stores its `Sub0` as the voice's
    /// `+92`, and `sub_82976360` points the voice's output send at it).
    Module(u32),
}

/// One voice to open: a member's sample and the values [`voice_values`] produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OneshotVoice {
    /// The EAAC stream's guest address: `bank_base + Splc::stream_offset(member.sample)`.
    pub sample: u32,
    /// 0..1; reaches the open as its percentage byte.
    pub gain: f32,
    /// The `Resample` ratio.
    pub pitch: f32,
    /// Seconds to wait before opening (`sub_82975CC8`'s `+80`).
    pub delay: f32,
    /// The member's pan angle in degrees (`+16`), 0 when it is unpanned.
    pub pan: f32,
    pub bus: OneshotBus,
}

/// A one-shot the runtime holds: retail keeps these at the owner's `+56`/`+60` and frees them with
/// `sub_824836B8`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OneshotHandle {
    /// The voice object, or 0 while the voice is still waiting out its delay.
    pub voice: u32,
    pub pending: Option<OneshotVoice>,
    pub remaining: f32,
}

impl AuthoredRuntime {
    /// Open one voice now (or hold it until its delay elapses), and return its handle.
    ///
    /// The sample must already have decoded PCM installed under its address
    /// (`AuthoredRuntime::insert_pcm(bank_base + header_offset, ..)`), which
    /// `PlayerAudioCatalog::load_splice_banks` provides for the `SPLC` banks.
    pub fn play_oneshot(&mut self, voice: &OneshotVoice) -> Result<OneshotHandle> {
        if voice.delay > 0.0 {
            return Ok(OneshotHandle {
                voice: 0,
                pending: Some(*voice),
                remaining: voice.delay,
            });
        }
        let opened = self.open_oneshot_voice(voice)?;
        Ok(OneshotHandle {
            voice: opened,
            pending: None,
            remaining: 0.0,
        })
    }

    /// `sub_82488DD0`: the owner-local send bus the wheel pops play into —
    /// `Sub0 → Sen0(level, → [[manager+52]]) → Sen0(→ the eEQChain bus)`, six channels, scheduler
    /// order 4. `level` is the owner's `level(14) / 32767` ([`RETAIL_POPS_LEVEL`]). Returns the
    /// `Sub0` to pass as [`OneshotBus::Module`]; retail keeps it at the owner's `+72`/`+76` and
    /// rebuilds it on every pop.
    pub fn build_owner_send(&mut self, bus_index: u32, level: f32) -> Result<u32> {
        use crate::classes::{self, TAG_POINTER};
        use crate::modules;
        let frame = super::SCRATCH + 0x200;
        let submix = crate::grain::chain::find(&self.guest, crate::grain::chain::SUBMIX_ID)?;
        let send = crate::grain::chain::find(&self.guest, crate::grain::chain::SEND_ID)?;
        if submix == 0 || send == 0 {
            return Err(Error::new(0, "the owner send classes are not registered"));
        }
        for (index, class) in [(0u32, submix), (1, send), (2, send)] {
            crate::grain::chain::descriptor(&mut self.guest, frame + 12 * index, 0, class, 6)?;
        }
        let root = self.guest.u32(classes::BUS_ROOT)?;
        let system = self.guest.u32(root + 8)?;
        let graph = {
            let Self { guest, owner, .. } = self;
            modules::build_graph(
                guest,
                &mut owner.device.heap,
                &mut crate::mathlib::Image,
                system,
                4,
                3,
                frame,
            )?
        };
        if graph == 0 {
            return Err(Error::new(0, "the owner send graph was not built"));
        }
        let modules_at = graph + 80;
        let block = frame + 64;
        // The first send carries the level into the manager's intermediate target.
        let manager = self.guest.u32(crate::device::BUS_MANAGER)?;
        let intermediate = self.guest.u32(self.guest.u32(manager + 52)?)?;
        classes::class_defaults(&mut self.guest, send, 0, block)?;
        let first = self.guest.u32(modules_at + 4)?;
        self.guest.set_u32(block, TAG_POINTER)?;
        self.guest.set_u32(block + 4, intermediate)?;
        crate::device::configure(&mut self.guest, first, 0, block)?;
        crate::device::post_property(&mut self.guest, first, 0, f64::from(level))?;
        // The second send carries the dry signal to the bus.
        let bus = {
            let Self { guest, owner, .. } = self;
            let _ = owner;
            crate::device::bus_for(guest, &mut crate::device::NoBuses, manager, bus_index, 0)?
        };
        let second = self.guest.u32(modules_at + 8)?;
        classes::class_defaults(&mut self.guest, send, 0, block)?;
        self.guest.set_u32(block, TAG_POINTER)?;
        self.guest.set_u32(block + 4, bus)?;
        crate::device::configure(&mut self.guest, second, 0, block)?;
        self.guest.u32(modules_at)
    }

    fn open_oneshot_voice(&mut self, voice: &OneshotVoice) -> Result<u32> {
        // The routing record the open reads: one 12-byte `{id, _, value}` whose id is at least 9.
        // A value of 0..7 is an eEQChain bus, which the open resolves through `sub_82491108`.
        let records = super::SCRATCH + 64;
        let count = match voice.bus {
            OneshotBus::Default | OneshotBus::Module(_) => 0,
            OneshotBus::EqChain(index) => {
                self.guest.set_u8(records, 9)?;
                self.guest.set_u32(records + 4, 0)?;
                self.guest.set_u32(records + 8, index)?;
                1
            }
        };
        // The open's byte scales the player's published float (`player+56`), which this mixer does
        // not read as a level, so the gain goes where retail puts it: the voice's `Gain` module
        // (`sub_82976360` posts it to the voice's own gain module, not to the player).
        let gain = PERCENT_MAX;
        // The open turns descriptor word 0 into the panner's angle with `0x8231BC08`'s 360/65536,
        // so the member's degrees go back through that scale.
        let mut shifted = [0u32; 6];
        shifted[0] = ((voice.pan * (65536.0 / 360.0)).round() as i32) as u32;
        let request = OpenRequest {
            sample: u64::from(voice.sample),
            index: 0,
            byte2: gain.round() as u8,
            shifted,
            bank_72: 0,
            arg8: 0,
            record_count: count,
            records,
        };
        let opened = {
            let Self { guest, owner, .. } = self;
            owner.device.open(guest, &request)?
        };
        if opened == 0 {
            return Err(Error::new(
                voice.sample,
                "the one-shot voice was not opened",
            ));
        }
        // Retail points the voice's output send at `[voice+92]`, the owner send bus's `Sub0`
        // (`sub_82976360`). The open has already pointed it at a bus, so it is re-pointed here.
        if let OneshotBus::Module(target) = voice.bus {
            let player = self.guest.u32(opened + 4)?;
            let count = u32::from(self.guest.u8(player + 68)?);
            if count == 0 {
                return Err(Error::new(opened, "the one-shot voice has no modules"));
            }
            let output = self.guest.u32(player + 80 + 4 * (count - 1))?;
            let send = crate::grain::chain::find(&self.guest, crate::grain::chain::SEND_ID)?;
            let block = super::SCRATCH + 96;
            crate::classes::class_defaults(&mut self.guest, send, 0, block)?;
            self.guest.set_u32(block, crate::classes::TAG_POINTER)?;
            self.guest.set_u32(block + 4, target)?;
            crate::device::configure(&mut self.guest, output, 0, block)?;
        }
        // `sub_82976360` posts `[voice+64] × block[1]` to the voice's gain module; the open records
        // that module at `+28`.
        let gain_module = self.guest.u32(opened + 28)?;
        if gain_module != 0 {
            crate::device::post_property(
                &mut self.guest,
                gain_module,
                0,
                f64::from(voice.gain.clamp(0.0, 1.0)),
            )?;
        }
        // `sub_82975CC8`'s `+68` reaches the retail voice's `Rsp0`; here it is the open's Resample.
        let resample = self.guest.u32(opened + 12)?;
        if resample != 0 {
            crate::device::post_property(&mut self.guest, resample, 0, f64::from(voice.pitch))?;
        }
        Ok(opened)
    }

    /// Whether a held one-shot has finished playing.
    ///
    /// The voice's own query (`vtable+20`, what the patch runtime asks) answers it: its first word
    /// is zero once the voice has ended, and its second is the milliseconds left. Retail's Splice
    /// manager instead watches `[graph+71] == 2`, which its audio thread sets when the player
    /// retires; the ported device only sets that on release, so the query is the equivalent test
    /// here.
    pub fn oneshot_finished(&mut self, handle: &OneshotHandle) -> Result<bool> {
        if handle.voice == 0 {
            return Ok(false);
        }
        let out = self.oneshot_query(handle)?;
        // The remaining milliseconds are `fctiwz((duration − elapsed) × 1000)`, so a sample that has
        // run out saturates negative rather than reaching zero exactly.
        Ok(out[0] == 0 || (out[1] as i32) <= 0)
    }

    /// The voice's query words: `[0]` is zero once it has ended, `[1]` the milliseconds left and
    /// `[2]` those elapsed (voice `vtable+20`).
    pub fn oneshot_query(&mut self, handle: &OneshotHandle) -> Result<[u32; 11]> {
        let mut out = [0u32; 11];
        if handle.voice != 0 {
            let Self { guest, owner, .. } = self;
            owner.device.query(guest, handle.voice, &mut out)?;
        }
        Ok(out)
    }

    /// `sub_824836B8`: free a held one-shot.
    pub fn release_oneshot(&mut self, handle: &mut OneshotHandle) -> Result<()> {
        handle.pending = None;
        handle.remaining = 0.0;
        if handle.voice == 0 {
            return Ok(());
        }
        let voice = std::mem::replace(&mut handle.voice, 0);
        let Self { guest, owner, .. } = self;
        owner.device.release(guest, voice)
    }

    /// Advance held one-shots by `dt`: start those whose delay has elapsed and free those that have
    /// finished. Returns whether the handle is still live.
    pub fn tick_oneshot(&mut self, handle: &mut OneshotHandle, dt: f32) -> Result<bool> {
        if let Some(pending) = handle.pending {
            handle.remaining -= dt;
            if handle.remaining <= 0.0 {
                handle.pending = None;
                handle.voice = self.open_oneshot_voice(&pending)?;
            }
            return Ok(true);
        }
        if handle.voice == 0 {
            return Ok(false);
        }
        if self.oneshot_finished(handle)? {
            self.release_oneshot(handle)?;
            return Ok(false);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rand_is_the_titles_generator() {
        // seed·0x343FD + 0x269EC3, bits 16..30 — the C runtime's rand() with seed 1.
        let mut rand = Rand::new(1);
        assert_eq!(rand.next(), 41);
        assert_eq!(rand.next(), 18467);
        assert_eq!(rand.next(), 6334);
        assert_eq!(Rand::new(1).unit(), 41.0 / 32768.0);
    }

    #[test]
    fn sequential_and_uniform_picks() {
        let mut rand = Rand::new(1);
        // mode 1: state + 1 modulo the count.
        let mut state = 0;
        let picks: Vec<u8> = (0..5).map(|_| pick(1, 3, &mut state, &mut rand)).collect();
        assert_eq!(picks, [1, 2, 0, 1, 2]);
        // one alternative never draws.
        let mut state = 0;
        assert_eq!(pick(0, 1, &mut state, &mut rand), 0);
        // mode 0: trunc(rand × count), always in range.
        let mut state = 0;
        for _ in 0..200 {
            assert!(pick(0, 6, &mut state, &mut rand) < 6);
        }
    }

    #[test]
    fn the_shuffle_visits_every_alternative_before_repeating() {
        let mut rand = Rand::new(7);
        let mut state = 0xFFFF_0000; // a full mask, first half
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..3 {
            seen.insert(pick(2, 6, &mut state, &mut rand));
        }
        assert_eq!(seen.len(), 3, "a half repeated an alternative: {seen:?}");
        for _ in 0..3 {
            let index = pick(2, 6, &mut state, &mut rand);
            assert!(index < 6);
            seen.insert(index);
        }
        assert!(seen.len() >= 3);
    }

    #[test]
    fn voice_values_follow_the_member() {
        // A member with no spread and no ranges: gain and pitch are the member's own.
        let member = MemberValues {
            gain: 0.75,
            gain_range: 0.0,
            delay: 0.0,
            delay_range: 0.0,
            pitch_spread: 1.0,
            probability: 1.0,
        };
        let mut rand = Rand::new(3);
        let values = voice_values(&member, &mut rand);
        assert_eq!(values.gain, 0.75);
        assert_eq!(values.pitch, 1.0, "spread 1.0 must not vary the pitch");
        assert_eq!(values.delay, 0.0);
        // A spread of 0 takes the fixed 4.0 branch when the draw lands above zero.
        let mut wide = member;
        wide.pitch_spread = 0.0;
        let mut rand = Rand::new(1);
        let mut pitches = Vec::new();
        for _ in 0..8 {
            pitches.push(voice_values(&wide, &mut rand).pitch);
        }
        assert!(pitches.iter().any(|p| *p == NO_SPREAD_PITCH));
        assert!(pitches.iter().all(|p| *p > 0.0));
        // Ranges add a positive amount at most as large as the range.
        let mut ranged = member;
        ranged.gain_range = 0.25;
        ranged.delay = 0.1;
        ranged.delay_range = 0.2;
        let mut rand = Rand::new(11);
        for _ in 0..32 {
            let v = voice_values(&ranged, &mut rand);
            assert!((0.75..=1.0).contains(&v.gain), "gain {}", v.gain);
            assert!((0.1..=0.3).contains(&v.delay), "delay {}", v.delay);
        }
    }

    #[test]
    fn probability_gates_a_member() {
        let mut member = MemberValues {
            gain: 1.0,
            gain_range: 0.0,
            delay: 0.0,
            delay_range: 0.0,
            pitch_spread: 1.0,
            probability: 1.0,
        };
        let mut rand = Rand::new(5);
        for _ in 0..64 {
            assert!(plays(&member, &mut rand), "probability 1.0 always plays");
        }
        member.probability = 0.0;
        let mut rand = Rand::new(5);
        let played = (0..64).filter(|_| plays(&member, &mut rand)).count();
        assert!(played <= 1, "probability 0.0 played {played} times");
    }

    #[test]
    fn container_value_stays_inside_its_range() {
        let mut rand = Rand::new(9);
        for _ in 0..32 {
            let value = container_value(0.84, 0.12, &mut rand);
            assert!((0.84..=0.96).contains(&value), "{value}");
        }
    }
}
