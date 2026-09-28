//! The Contacts component's one-shot bank voices: wheel pops and the landing impact.
//!
//! `ContactsOwner` decides *when* a contact sound plays (the retail edges and selector ladders) but
//! cannot reach the runtime from [`ContactVoices`], so it records its plays here and the worker
//! drains them: the request's selector and the audio state pick a Splice bank sample
//! (`skate_data::audio::splice`), the sample resolves to its layered members, and each member is
//! opened as a one-shot voice (`AuthoredRuntime::play_oneshot`). One retail voice is therefore a
//! set of handles, held and freed together the way `sub_824836B8` frees the container's children.
//!
//! `ContactSound::GrindOnset` takes a different route. Retail's `sub_824BB0E0` does not start a
//! bank voice at all: it posts a 48-byte message to the contact-sound manager at `[manager+668]`,
//! which is `CSTATEMGR_Collision`. That subsystem is [`super::collision_states`], and
//! [`ContactVoicePlayer::post_grind_onset`] drives it — the message resolves to up to two voices,
//! one per material, each chosen against the *other* material's class.
//!
//! Still not routed here: `ContactSound::PopRoll`. The rolling/scrape contacts are
//! `sub_824BC188`'s own path and the pop-roll layer's bank is not resolved by the ported
//! selection, so it is dropped with a label rather than played from an invented sample.

use skate_audio_core::authored::{
    AuthoredRuntime,
    oneshot::{OneshotBus, OneshotHandle, OneshotVoice, Rand},
};
use skate_data::audio::splice::{
    LandingTuning, PopsTuning, SpliceBanks, SpliceState, surface_category,
};
use skate_data::collections::Collections;

use super::audio_state::AudioState;
use super::collision_states::{
    CollisionMaterials, CollisionSample, CollisionStates, ContactMessage, NO_MATERIAL, NO_TIER,
};
use super::components::contacts::{ContactSound, ContactVoices, SurfaceMap, VoiceRequest};

/// Contacts controller output 15 at landing class 2, measured from the real MixMap under the
/// retail pre-roll (`skate-audio-core` example `contacts_input_probe`): classes 0/1/2 give
/// 2584/3103/3650, a 3.0 dB spread. Class 2 is the reference so the loudest landing keeps the
/// level `player-audio-retail-drivers.md` §8 measured against the recomp.
const LANDING_SEND_MAXIMUM: f32 = 3650.0;

/// Headroom for the landing stack, over and above `CONTACT_TRIM`.
///
/// `CONTACT_TRIM` was calibrated when a landing was **two** voices (the fixed impact and the class
/// voice) and was set to put that pair at full scale. A landing is now **five** — the `+496` ladder
/// voice and the two contact-sound-manager collision voices joined it — and the stack clips:
/// measured over a playtest, every landing peaked at exactly 1.0, 271 output blocks at full scale.
/// Clipping is a hard limiter, so it flattened class 0, 1 and 2 to the same loudness however well
/// the classes were resolved. That is what "every landing sounds equally loud" was.
///
/// Retail's own class-0 landings sit at **−4.4 dBFS** and do not clip (its class 2 reaches +2.0 and
/// just touches the ceiling), measured from `.local/captures/retail-levels-*.log` binned by
/// retail's class thresholds. This trim exists to put class 0 back below the ceiling so the
/// authored class step — +4.7 dB at class 1 and +5.8 dB at class 2, which is in the samples' own
/// layer counts and gains — survives to the output.
///
/// It is applied to the landing voices only. The pops and the grind onset keep `CONTACT_TRIM`
/// alone, because their voice count did not change.
///
/// **Calibrated, in two measured passes.** A clipped peak reads as exactly 1.0 whatever is under
/// it, so the value could not be computed in one step:
///
/// | pass | trim | class 0 peak | class 1 | class 2 |
/// |---|---|---|---|---|
/// | before | 0.625 | 0.0 (clipped) | 0.0 | 0.0 |
/// | headroom probe | 0.4 | −1.1 | −0.5 | −0.6 |
/// | this | **0.274** | −4.4 (target) | — | — |
///
/// The probe's −8 dB moved the peak only 1.1 dB, which says the stack had been roughly 7 dB *into*
/// the ceiling — not, as it first looked, that some other voice was responsible. `0.4 × 10^(−3.3
/// / 20) = 0.274` puts class 0 on retail's measured −4.4 dBFS. Class 2 then lands near +1.4 and
/// still touches the ceiling, which is retail's behaviour too (its class 2 measures +2.0).
///
/// This is a calibration against a measured retail target, not a trim by ear, and like
/// `CONTACT_TRIM` it is a deviation that should disappear once the Splice graph's own levels are
/// recovered.
const LANDING_TRIM: f32 = 0.274;

/// `sub_824D20E8`'s Collision controller output for a material category, as a 0..1 scale.
///
/// Every contact voice is scaled by this (`sub_824D2318`): the category picks a Collision
/// controller output (13,14,15,16,17,18,12,19,21,20 for categories 0..=9) and the voice's gain is
/// that output times the message level times the material level. This port applied the two level
/// terms and **not** this one, so a landing's collision voices played at full material level and
/// drowned the class voice — which is why they were switched off.
///
/// **Measured, not read.** The `SFXObj_Collision` controllers exist in `MixMapSK8.mxb` with
/// exactly these outputs, but their inputs are fed by the global mix controllers, which this
/// engine does not drive: every one of these outputs reads 0 both in `collision_output_probe`
/// and in the live runtime (`headless_collision_controller_outputs`). So these are the medians of
/// what retail's own controllers read, over every recomp capture in `.local/captures` (about
/// 19,000 reads at `824D21BC`), and they stand in until those inputs are driven:
///
/// | category | output | median | dB |
/// |---|---|---|---|
/// | 0 | 13 | 8869 | −11.4 |
/// | 1 | 14 | 22153 | −3.4 |
/// | 2 | 15 | 6079 | −14.6 |
/// | 4 | 17 | 14669 | −7.0 |
/// | 5 | 18 | 7923 | −12.3 |
/// | 6 | 12 | 8126 | −12.1 |
/// | 9 | 20 | 7913 | −12.3 |
///
/// Categories 3, 7 and 8 never came up in a capture; they take the median of the measured ones
/// (7923) rather than full scale, which is the closest defensible value and still far from the
/// 1.0 that was audibly wrong.
fn collision_controller_scale(category: u8) -> f32 {
    const MEASURED: [u32; 10] = [8869, 22153, 6079, 7923, 14669, 7923, 8126, 7923, 7923, 7913];
    f32::from(MEASURED[usize::from(category).min(9)] as u16) / 32_767.0
}

/// Contacts output 15 — the landing voice's owner send — at each landing class, measured off the
/// real MixMap under the retail pre-roll with controller input 2 held at retail's own words
/// (0 / 16000 / 32767). A 3.0 dB spread, class 0 to class 2.
const LANDING_SEND_BY_CLASS: [f32; 3] = [2584.0, 3103.0, 3650.0];

/// The landing one-shot's gain relative to the loudest class.
///
/// Class 2 is the reference so the heaviest landing keeps the absolute level
/// `player-audio-retail-drivers.md` §8 measured against the recomp — `CONTACT_TRIM` was calibrated
/// with this send absent, so applying output 15 raw would drop every landing about 19 dB.
fn landing_send_scale(class: Option<u32>) -> f32 {
    let Some(class) = class else { return 1.0 };
    let level = LANDING_SEND_BY_CLASS
        .get(class as usize)
        .copied()
        .unwrap_or(LANDING_SEND_MAXIMUM);
    (level / LANDING_SEND_MAXIMUM).clamp(LANDING_SEND_FLOOR, 1.0)
}

/// `fmuls` then `fctiwz`: retail truncates the scaled level back to an integer.
fn scale_level(level: u32, gain: f32) -> u32 {
    (level as f32 * gain) as u32
}

/// `sub_824BA630` @ 0x824BA7D0: `clamp(air / divisor, 0, 1)`.
///
/// This is the value the tier test and both level windows run on — **not** seconds. With the
/// owner's divisor of 0.4 the tier splits at 0.04 s of air and the curve is spent by 0.12 s.
fn landing_weight(air_time: f32, divisor: f32) -> f32 {
    if divisor <= 0.0 {
        return 0.0;
    }
    (air_time / divisor).clamp(0.0, 1.0)
}
/// The send is an environment level as well as a class level, so it can legitimately sit low.
/// Retail's own class-0 reading is 0.708 of the maximum; this floor keeps an unusual environment
/// from muting a landing outright while still letting the class spread through.
const LANDING_SEND_FLOOR: f32 = 0.5;

/// The Contacts tuning class (`sub_824BA630` reads its landing fields off it) and the three fields
/// the landing's contact-sound message needs. The owner's vault holds 95, 0.1 and 0.3.
const CONTACTS_TUNING_CLASS: &str = "Hash_C26949FCB638A2CA";
const LANDING_MATERIAL_FIELD: &str = "Hash_85FDC8BF696BCA5C";
const LANDING_THRESHOLD_FIELD: &str = "Hash_3462CBB16DCA696E";
const LANDING_CEILING_FIELD: &str = "Hash_590495E420B399E5";
/// `sub_824BA630` @ 0x824BA7C0: the air-time divisor, and the two per-level gains at 0x824BAAC0
/// and 0x824BAB20.
const LANDING_DIVISOR_FIELD: &str = "Hash_6D68BC2D1A23C29A";
const LANDING_GAIN_A_FIELD: &str = "Hash_31DEEF8FA219950F";
const LANDING_GAIN_B_FIELD: &str = "Hash_0EC6EEF5366FEA85";

/// The grind tuning class and its own two impact windows (0.25 and 0.5 in the owner's vault).
/// `sub_824BB0E0` uses these where the landing uses the Contacts pair.
const GRIND_TUNING_CLASS: &str = "Hash_049861E8F9A8D16B";
const GRIND_THRESHOLD_FIELD: &str = "Hash_086B66C3D4FFEE8F";
const GRIND_CEILING_FIELD: &str = "Hash_B2ACAFDBCD963C93";

/// One recorded play, in the order `ContactsOwner` made it.
struct Queued {
    id: u32,
    sound: ContactSound,
    request: VoiceRequest,
}

/// A handle both the component and the worker hold: the component records plays through its
/// [`ContactVoices`] impl, the worker drains them. The whole player-sound path lives on the audio
/// worker thread, so a plain `Rc<RefCell<..>>` is enough.
#[derive(Clone, Default)]
pub(crate) struct SharedContactVoices(std::rc::Rc<std::cell::RefCell<QueuedContactVoices>>);

impl ContactVoices for SharedContactVoices {
    fn play(&mut self, sound: ContactSound, request: &VoiceRequest) -> Option<u32> {
        self.0.borrow_mut().play(sound, request)
    }

    fn free(&mut self, handle: u32) {
        self.0.borrow_mut().free(handle);
    }
}

/// The recorded plays. Opaque ids; [`ContactVoicePlayer`] maps them to the voices they opened.
#[derive(Default)]
pub(crate) struct QueuedContactVoices {
    queued: Vec<Queued>,
    freed: Vec<u32>,
    next: u32,
}

impl ContactVoices for QueuedContactVoices {
    fn play(&mut self, sound: ContactSound, request: &VoiceRequest) -> Option<u32> {
        self.next += 1;
        let id = self.next;
        self.queued.push(Queued {
            id,
            sound,
            request: *request,
        });
        Some(id)
    }

    fn free(&mut self, handle: u32) {
        self.freed.push(handle);
    }
}

/// The voices one recorded play opened.
struct Live {
    id: u32,
    handles: Vec<OneshotHandle>,
}

/// Turns the component's recorded plays into real voices and ticks them.
pub(crate) struct ContactVoicePlayer {
    banks: SpliceBanks,
    pops: PopsTuning,
    landing: LandingTuning,
    surfaces: SurfaceMap,
    state: SpliceState,
    rand: Rand,
    live: Vec<Live>,
    reported: std::collections::HashSet<String>,
    /// The takeoff pops. Retail's metered takeoff is +17 dB over its rolling bed while this
    /// engine's was +5.5 dB without them, so they are on by default; `SKATE_AUDIO_CONTACT_POPS=0`
    /// turns them off. Their MixMap-driven environment send still uses `RETAIL_POPS_LEVEL`: the
    /// component now writes Contacts inputs 0, 1, 2 and 6, but the pops' own send is driven by
    /// inputs 3, 10, 17, 18 and 21, and those are still unwritten.
    pops_enabled: bool,
    /// Whether a landing also posts to the contact-sound manager (its two collision voices).
    ///
    /// **On**, as `sub_824BA630` does. They were off while their gain was wrong: retail's is
    /// `controllerOutput × messageLevel × materialLevel` (`sub_824D2318`) and this port applied
    /// only the two level terms, so both voices played at full material level and drowned the
    /// class voice — the owner A/B'd that and preferred them off ("low ollies still sound too
    /// loud and different than retail"). [`collision_controller_scale`] now supplies the missing
    /// term from retail's own captured controller reads, which is −11 dB on ordinary ground.
    ///
    /// `SKATE_AUDIO_LANDING_COLLISION=0` switches them off again for an A/B.
    landing_collision_enabled: bool,
    /// The pops' owner-local six-channel send bus (`sub_82488DD0`), built on first use.
    pops_send: Option<u32>,
    /// `sub_824BA630`'s landing window, read from the Contacts tuning class: the board material it
    /// pairs the surface with (95), and the two air-time thresholds the level interpolates over.
    landing_material: i32,
    landing_threshold: f32,
    landing_ceiling: f32,
    /// `Hash_6D68BC2D1A23C29A` (0.4): the divisor that turns the latched air time into the
    /// `[0, 1]` ratio the tier test and both windows actually run on.
    landing_divisor: f32,
    /// `Hash_31DEEF8FA219950F` (0.65) and `Hash_0EC6EEF5366FEA85` (1.0): the per-word gains
    /// applied to each level after `sub_82496C58` and before the message.
    landing_gain_a: f32,
    landing_gain_b: f32,
    grind_threshold: f32,
    grind_ceiling: f32,
    /// `CSTATEMGR_Collision` and its per-material table: the subsystem the grind onset posts to.
    collision: CollisionStates,
    collision_materials: CollisionMaterials,
    /// The collision voices in flight. Unlike the pops and the landing, `sub_824BB0E0` keeps no
    /// slot the component reads back ("this routine does not hold its voice"), so these are owned
    /// here and freed when they finish. They must be ticked: `tick_oneshot` is what *opens* a
    /// voice whose Splice member carries a delay, and what releases one that has run out.
    collision_live: Vec<OneshotHandle>,
    /// The session marker's sounds, and the voices they have in flight. These are held for the
    /// same reason the collision voices are: a Splice member can carry a delay, and dropping the
    /// handle before `tick_oneshot` opens it strands the voice.
    frontend: super::frontend::FrontEndSounds,
    frontend_live: Vec<OneshotHandle>,
    /// `sub_824BC188`'s per-region re-trigger cooldown, `obj+260+4i`. A region that has just
    /// sounded will not sound again until this runs back down, which is what stops a body held
    /// against a wall from firing an impact every frame.
    body_cooldown: [f32; BODY_REGIONS],
    /// `SKATE_AUDIO_BODY_IMPACT=0` silences the body impacts again.
    body_impact_enabled: bool,
}

/// `sub_824BC188` walks six of the eight contact regions (`cmpwi cr6,r17,6`).
const BODY_REGIONS: usize = 6;

/// `sub_824BCBA0`: the three body-part materials a contact region carries.
///
/// The ids are confirmed twice: from this switch in the lifted code, and independently by hashing
/// `tools/asset_pipeline/names.txt` against `MATERIAL_VAULT_KEY`, which names 97 `head`,
/// 98 `torso`, 99 `leg`, 100 `arm`, 107 `skin`, 108 `denim`, 109 `cotton`. Region 0 is the head,
/// and `+593` is its face sub-flag; its second material is left at retail's "none".
fn body_region_materials(region: usize, face: bool) -> (i32, i32, i32) {
    match region {
        0 => (97, NO_MATERIAL, if face { 112 } else { 110 }),
        1 => (98, 109, 110),
        2 | 3 => (100, 107, 111),
        _ => (99, 108, 111),
    }
}

/// The cooldown a region takes after it sounds, in frames: Int32 `0x0F` from the Contacts tuning
/// class `6EBA5BCD3E38A98A`, field `6DD85F43C1B6E6AA`. Retail counts it down by the audio state's
/// time scale (`+220`), which is 1.0 whenever the game is not in slow motion.
const BODY_IMPACT_RETRIGGER: f32 = 15.0;

impl ContactVoicePlayer {
    pub(crate) fn new(
        assets: &std::path::Path,
        cache: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let vault = Collections::load(assets)?;
        // `sub_82496FD0` resolves a material's category through the vault one lookup at a time;
        // doing all 143 once here is the same table. Only material 94 — the silent slot, whose
        // key is all zeroes — is expected to be missing, so anything else is worth saying.
        let (collision_materials, unresolved) = CollisionMaterials::load(&vault);
        if unresolved > 1 {
            eprintln!(
                "SKATE_PLAYER_AUDIO contact_voice_unavailable {unresolved} collision materials \
                 have no vault record; their category falls back to retail's default record"
            );
        }
        // `sub_824BA630` reads all three off the Contacts tuning class; the defaults are the
        // values the owner's vault holds, used only if a field is missing.
        let tuning_float = |name: &str, fallback: f32| {
            vault
                .float(CONTACTS_TUNING_CLASS, "default", name)
                .unwrap_or(fallback)
        };
        let landing_material = vault
            .field(CONTACTS_TUNING_CLASS, "default", LANDING_MATERIAL_FIELD)
            .ok()
            .and_then(|f| u32::from_str_radix(f.data.trim(), 16).ok())
            .map_or(95, |v| v as i32);
        Ok(Self {
            landing_material,
            landing_threshold: tuning_float(LANDING_THRESHOLD_FIELD, 0.1),
            landing_ceiling: tuning_float(LANDING_CEILING_FIELD, 0.3),
            landing_divisor: tuning_float(LANDING_DIVISOR_FIELD, 0.4),
            landing_gain_a: tuning_float(LANDING_GAIN_A_FIELD, 0.65),
            landing_gain_b: tuning_float(LANDING_GAIN_B_FIELD, 1.0),
            grind_threshold: vault
                .float(GRIND_TUNING_CLASS, "default", GRIND_THRESHOLD_FIELD)
                .unwrap_or(0.25),
            grind_ceiling: vault
                .float(GRIND_TUNING_CLASS, "default", GRIND_CEILING_FIELD)
                .unwrap_or(0.5),
            collision: CollisionStates::new(),
            collision_materials,
            collision_live: Vec::new(),
            banks: SpliceBanks::load(
                &assets.join("private/stock/data/audio/audiofiles.big"),
                // The DLC bank is absent from a stock installation; only the stock one is required.
                // `Skate_Metal.bnk` and `HOM_Set_1.bnk` are the other two families the collision
                // materials' kind word selects between (`sub_824967F8`).
                &[
                    skate_data::audio::splice::COLLISIONS_BANK,
                    skate_data::audio::splice::DLC_COLLISIONS_BANK,
                    super::collision_states::METAL_BANK,
                    super::collision_states::HOM_BANK,
                    // The front-end one-shots: the session marker's three cellphone sounds.
                    skate_data::audio::catalog::MENU_BANK,
                ],
                &[skate_data::audio::splice::COLLISIONS_BANK],
            )
            .map_err(|error| error.to_string())?,
            pops: PopsTuning::load(&vault).map_err(|error| error.to_string())?,
            landing: LandingTuning::load(&vault).map_err(|error| error.to_string())?,
            surfaces: SurfaceMap::load(&vault)?,
            state: SpliceState::default(),
            rand: Rand::new(cache.map_or(1, |_| 1)),
            live: Vec::new(),
            reported: std::collections::HashSet::new(),
            pops_enabled: std::env::var("SKATE_AUDIO_CONTACT_POPS").map_or(true, |v| v != "0"),
            landing_collision_enabled: std::env::var("SKATE_AUDIO_LANDING_COLLISION")
                .map_or(true, |v| v != "0"),
            pops_send: None,
            frontend: super::frontend::FrontEndSounds::load(&vault),
            frontend_live: Vec::new(),
            body_cooldown: [0.0; BODY_REGIONS],
            body_impact_enabled: std::env::var("SKATE_AUDIO_BODY_IMPACT")
                .map_or(true, |v| v != "0"),
        })
    }

    /// `sub_824BC188`: the body-contact region loop, the sound a body makes hitting something.
    ///
    /// This is the on-board path — it runs while riding, and `+676` (bailing) only *modifies* it.
    /// `sub_824E3BE0` is the ragdoll counterpart and is a separate, still unported routine, so a
    /// bail's own body impacts do not come through here.
    ///
    /// Ported: the six-region walk, the impact and cooldown gates, `sub_824BCBA0`'s region to
    /// body-part material map, the world material from `+560`, `sub_82497088`'s band and window,
    /// and the first of retail's five contact-sound calls.
    ///
    /// **Not ported**, and labelled as missing rather than approximated: retail's four further
    /// layers per region (a detail layer, then three body-only layers using the second and third
    /// materials against 143), `sub_824BCEB0`'s one-shot accent, and the `+592`/`+596`/`+600`
    /// latches, which only feed the bail path.
    fn body_impacts(
        &mut self,
        runtime: &mut AuthoredRuntime,
        audio: &AudioState,
    ) -> Result<(), String> {
        if !self.body_impact_enabled {
            return Ok(());
        }
        // `lbz r22,676` / `bne` → `lbz r11,677` / `cmplwi cr6,r11,0` / `bne`: riding runs the
        // loop, a bail in progress still runs it, and a bail that is **over** skips it entirely.
        // `cr6` really is recomputed before that second branch, so the skip is a genuine gate and
        // not a fall-through — without it a finished bail keeps thudding as the body settles.
        if audio.bail_676 && audio.bail_over_677 {
            return Ok(());
        }
        for region in 0..BODY_REGIONS {
            // `lfsx f31,r9,r10` then `fcmpu`/`ble`: a region with no impact this frame is skipped,
            // and so is one whose cooldown has not run down (`lfsx f0,r30,r31`, `bgt`).
            let impact = audio.body_impact_496[region];
            if !(impact > 0.0) || self.body_cooldown[region] > 0.0 {
                continue;
            }
            // `lwzx` at `+560`, then `addi r11,r11,-1` and the 0..=143 clamp; retail's "none".
            let raw = audio.body_material_560[region] as i32;
            let world = match raw {
                0 => NO_MATERIAL,
                raw => {
                    let value = raw - 1;
                    if (0..=143).contains(&value) {
                        value
                    } else {
                        NO_MATERIAL
                    }
                }
            };
            let (body, _second, _third) = body_region_materials(region, audio.face_contact_593);
            if self.post_body_impact(runtime, body, world, impact)? {
                self.body_cooldown[region] = BODY_IMPACT_RETRIGGER;
            }
        }
        // `lfs f0,220(r10)`: the countdown runs on the audio state's time scale.
        for cooldown in &mut self.body_cooldown {
            *cooldown = (*cooldown - audio.time_scale_220).max(0.0);
        }
        Ok(())
    }

    /// The first of `sub_824BC188`'s five `sub_82486EF0` calls: the body part against the surface
    /// it hit. Returns whether anything was posted, which is what arms the region's cooldown.
    fn post_body_impact(
        &mut self,
        runtime: &mut AuthoredRuntime,
        body: i32,
        world: i32,
        impact: f32,
    ) -> Result<bool, String> {
        // `sub_82497088` twice, once per side. Retail drops the contact only when *both* sides
        // come back category 3 (`cmpwi cr6,r20,3` / `cmpwi cr6,r26,3`).
        let band_a = self.collision_materials.impact_band(body, impact);
        let band_b = self.collision_materials.impact_band(world, impact);
        if band_a.is_none() && band_b.is_none() {
            return Ok(false);
        }
        let level = |materials: &CollisionMaterials,
                     surfaces: &SurfaceMap,
                     material: i32,
                     other: i32,
                     band: Option<(i32, f32, f32)>| {
            let Some((tier, lo, hi)) = band else {
                return (NO_TIER, 0);
            };
            (
                tier,
                materials.contact_level(material, other, tier, lo, hi, impact, surfaces),
            )
        };
        let (tier_a, level_a) = level(
            &self.collision_materials,
            &self.surfaces,
            body,
            world,
            band_a,
        );
        let (tier_b, level_b) = level(
            &self.collision_materials,
            &self.surfaces,
            world,
            body,
            band_b,
        );
        let message = ContactMessage {
            material_a: body,
            material_b: world,
            tier_a,
            tier_b,
            level_a,
            level_b,
            ..ContactMessage::default()
        };
        let slot = self.collision.post(message, &self.collision_materials);
        let started = self.collision.slots()[slot].started;
        self.report(&format!(
            "BodyImpact posted a={body} b={world} tier={tier_a}/{tier_b} \
             level={level_a}/{level_b} -> slot {slot} started={started:?}"
        ));
        let mut played = false;
        for record in 0..2 {
            if !started[record] {
                continue;
            }
            let other = self
                .collision_materials
                .other_class(message.material(1 - record), &self.surfaces);
            let Some(sample) = self.collision_materials.sample(
                message.material(record),
                other,
                message.tier(record),
            ) else {
                continue;
            };
            let record_level = if record == 0 { level_a } else { level_b };
            let category = self.collision_materials.category(message.material(record));
            self.play_collision(
                runtime,
                &sample,
                record_level,
                collision_controller_scale(category),
                1.0,
            );
            played = true;
        }
        Ok(played)
    }

    /// Open everything the component recorded this frame, free what it released, and advance the
    /// voices already playing.
    pub(crate) fn drain(
        &mut self,
        runtime: &mut AuthoredRuntime,
        shared: &SharedContactVoices,
        audio: &AudioState,
        dt: f32,
    ) -> Result<(), String> {
        // `sub_824B8218` runs the body-contact loop in its own chain, not off a queued request:
        // the regions are read straight from the audio state every frame.
        self.body_impacts(runtime, audio)?;
        let (queued, freed) = {
            let mut voices = shared.0.borrow_mut();
            (
                std::mem::take(&mut voices.queued),
                std::mem::take(&mut voices.freed),
            )
        };
        for id in freed {
            if let Some(index) = self.live.iter().position(|live| live.id == id) {
                let mut live = self.live.swap_remove(index);
                for handle in &mut live.handles {
                    runtime.release_oneshot(handle).map_err(|e| e.to_string())?;
                }
            }
        }
        for play in queued {
            // `sub_824BB0E0`'s tail posts to the contact-sound manager rather than starting a
            // bank voice, so the grind onset leaves this path here.
            if matches!(play.sound, ContactSound::GrindOnset) {
                self.post_grind_onset(runtime, &play.request)?;
                continue;
            }
            // `sub_824BA630` starts its two bank voices *and* posts to the contact-sound manager.
            // The bank voices are handled below as before; the message is what makes a landing
            // sound like the surface rather than like a generic impact.
            if matches!(play.sound, ContactSound::LandingClass) && self.landing_collision_enabled {
                self.post_landing(runtime, &play.request, audio)?;
            }
            let Some((bank, sample, mut bus)) = self.selection(&play, audio) else {
                // The two unported subsystem paths dropped silently, which made a
                // rail landing look like nothing had even been requested. Say so
                // once per run; the other `None`s are ordinary (pops disabled, or
                // a landing with no wheel actually down).
                if matches!(play.sound, ContactSound::PopRoll) {
                    self.report(&format!(
                        "{:?} needs the contact-sound manager ([manager+668], `sub_82486EF0`); not ported, so this contact is silent",
                        play.sound
                    ));
                }
                continue;
            };
            // `sub_824B9CC8` routes the pops through the owner's own send bus, whose first send
            // carries the environment level the owner reads as level(14).
            if matches!(play.sound, ContactSound::Pop) {
                let send = match self.pops_send {
                    Some(send) => send,
                    None => {
                        match runtime.build_owner_send(
                            self.pops.bus,
                            skate_audio_core::authored::oneshot::RETAIL_POPS_LEVEL,
                        ) {
                            Ok(send) => {
                                self.pops_send = Some(send);
                                send
                            }
                            Err(error) => {
                                self.report(&format!("pops send bus: {error}"));
                                continue;
                            }
                        }
                    }
                };
                bus = OneshotBus::Module(send);
            }
            // A contact sound that cannot resolve or play must not take the whole player-sound
            // worker down with it: report it once and carry on with the rest of the mix.
            let members = match self
                .banks
                .resolve(bank, sample, &mut self.state, &mut self.rand)
            {
                Ok(members) => members,
                Err(error) => {
                    self.report(&format!("{bank} sample {sample:#x}: {error}"));
                    continue;
                }
            };
            let Some(base) = runtime.bank_base(bank) else {
                self.report(&format!("Splice bank {bank} is not installed"));
                continue;
            };
            // Retail's landing level rides the owner send, whose level is Contacts output 15,
            // and output 15 is driven by the class this component just wrote to input 2.
            //
            // The class is used directly rather than reading output 15 back, because the read
            // would be a frame stale *and* wrong: the component reads `level(15)` before its own
            // input-2 write lands, and the MixMap smooths over tens of frames, so a one-shot
            // opened on the landing frame would still see the previous landing's send.
            //
            // Retail does not have this problem — its class voice is routed through the send bus
            // and follows output 15 continuously as the graph settles. This engine gives the
            // one-shot a fixed gain at open, so it uses the *settled* output-15 value for the
            // class, measured off the real MixMap under the retail pre-roll
            // (`skate-audio-core` example `contacts_input_probe`). **That is the approximation
            // here: retail's send ramps across the sample, ours is flat for its whole length.**
            let send_scale = match play.sound {
                ContactSound::LandingClass => {
                    landing_send_scale(play.request.landing_class) * LANDING_TRIM
                }
                // The other two landing voices take the headroom but not the send: retail's
                // impact and ladder levels are fixed, it is only the class voice that rides
                // output 15.
                ContactSound::Landing | ContactSound::LandingLadder => LANDING_TRIM,
                _ => 1.0,
            };
            let mut handles = Vec::with_capacity(members.len());
            for member in members {
                match runtime.play_oneshot(&OneshotVoice {
                    sample: base + member.stream_offset,
                    gain: member.values.gain * send_scale,
                    pitch: member.values.pitch,
                    delay: member.values.delay,
                    pan: member.pan,
                    bus,
                }) {
                    Ok(handle) => handles.push(handle),
                    Err(error) => {
                        self.report(&format!("{bank} sample {sample:#x} member: {error}"))
                    }
                }
            }
            self.live.push(Live {
                id: play.id,
                handles,
            });
        }
        // `sub_824D1E00` runs on every `SFXObj_Collision` every frame, whether or not anything was
        // posted this one: it rewrites both inputs from zero and only then decides to raise them.
        for (key, id, value) in self.collision.drive() {
            runtime
                .mixmap_apply(key, &[(id, value)])
                .map_err(|e| e.to_string())?;
        }
        for live in &mut self.live {
            for handle in &mut live.handles {
                runtime
                    .tick_oneshot(handle, dt)
                    .map_err(|e| e.to_string())?;
            }
        }
        // The collision voices own themselves: tick opens the delayed ones, and a voice that has
        // finished releases itself and drops out of the list.
        let mut still_live = Vec::with_capacity(self.collision_live.len());
        for mut handle in std::mem::take(&mut self.collision_live) {
            if runtime
                .tick_oneshot(&mut handle, dt)
                .map_err(|e| e.to_string())?
            {
                still_live.push(handle);
            }
        }
        self.collision_live = still_live;
        // The front-end one-shots own themselves the same way.
        let mut still_live = Vec::with_capacity(self.frontend_live.len());
        for mut handle in std::mem::take(&mut self.frontend_live) {
            if runtime
                .tick_oneshot(&mut handle, dt)
                .map_err(|e| e.to_string())?
            {
                still_live.push(handle);
            }
        }
        self.frontend_live = still_live;
        // A play whose voices have all finished keeps no handles; retail's slot still holds its
        // container until the component frees it, so the entry stays until `free`.
        Ok(())
    }

    /// `sub_824BB0E0`'s tail: build the 48-byte message, hand it to `CSTATEMGR_Collision`, and
    /// start the voices the slot's two records resolve to.
    ///
    /// The argument order is the lifted one — `sub_82486EF0(this, family_base, material, tier,
    /// tier, &position, level_a, level_b, flags…)` — and note the grind onset passes **the same
    /// tier word twice** (`mr r6,r30` / `mr r7,r30`), so `contact_weight` sees a matched pair.
    ///
    /// `sub_824D1F68` then starts one voice per material, each against the *other* material's
    /// class: that is what makes a rail grind two sounds rather than one — the board's family base
    /// out of `Skate_Collisions.bnk` and the rail's own material out of `Skate_Metal.bnk`.
    ///
    /// Two message fields stay zero here, and neither is read by anything this resolves: the
    /// `+0x10` world position (audio state `+48`, consumed only by the spatialisation in
    /// `sub_82975A60`, which is not ported) and the two `+0x20`/`+0x24` levels from
    /// `sub_82496C58`. The per-voice level below is the material record's own `+52`, which is the
    /// one `sub_824965D0` writes into the voice record.
    fn post_grind_onset(
        &mut self,
        runtime: &mut AuthoredRuntime,
        request: &VoiceRequest,
    ) -> Result<(), String> {
        let tier = request.tier.unwrap_or(0);
        let impact = request.impact.unwrap_or(0.0);
        let (lo, hi) = if tier == 0 {
            (0.0, self.grind_threshold)
        } else {
            (self.grind_threshold, self.grind_ceiling)
        };
        let board = request.family_base.unwrap_or(0) as i32;
        let surface = request.material.unwrap_or(0) as i32;
        let message = ContactMessage {
            material_a: board,
            material_b: surface,
            tier_a: tier,
            tier_b: tier,
            level_a: self.collision_materials.contact_level(
                board,
                surface,
                tier,
                lo,
                hi,
                impact,
                &self.surfaces,
            ),
            level_b: self.collision_materials.contact_level(
                surface,
                board,
                tier,
                lo,
                hi,
                impact,
                &self.surfaces,
            ),
            ..ContactMessage::default()
        };
        let slot = self.collision.post(message, &self.collision_materials);
        let started = self.collision.slots()[slot].started;
        // Diagnostic: a grind onset that reaches here but makes no sound is the failure mode this
        // subsystem is most likely to have, and it is invisible otherwise. One line per distinct
        // message shape, through the same once-per-run channel as the failures.
        self.report(&format!(
            "GrindOnset posted a={} b={} tier={} -> slot {slot} started={started:?}",
            message.material_a, message.material_b, tier
        ));
        for record in 0..2 {
            if !started[record] {
                continue;
            }
            // `sub_824D1F68` passes the *other* record's material, which `sub_824965D0` runs
            // through `sub_82497910` to get its class.
            let other = self
                .collision_materials
                .other_class(message.material(1 - record), &self.surfaces);
            let Some(sample) = self.collision_materials.sample(
                message.material(record),
                other,
                message.tier(record),
            ) else {
                self.report(&format!(
                    "GrindOnset record {record} material {} against class {other} resolved to no sample",
                    message.material(record)
                ));
                continue;
            };
            let level = if record == 0 {
                message.level_a
            } else {
                message.level_b
            };
            let category = self.collision_materials.category(message.material(record));
            self.play_collision(
                runtime,
                &sample,
                level,
                collision_controller_scale(category),
                1.0,
            );
        }
        Ok(())
    }

    /// `sub_824BA630`'s message to `CSTATEMGR_Collision`.
    ///
    /// Retail pairs the surface the wheels are on (audio state `+620`) with a board material the
    /// Contacts tuning class supplies (95), gives both the same tier, and interpolates each one's
    /// level over the **latched air time** — `[this+340]`, i.e. how long the skater was actually
    /// falling. `tier` is 0 below the tuning threshold and 1 above it, and the window is `[0, t]`
    /// for tier 0 and `[t, ceiling]` for tier 1 (0.1 and 0.3 in the owner's vault).
    ///
    /// **If either level comes out 0 the message is not posted at all** — that is retail's own
    /// guard (`sub_824BA630` @ `0x824BABE0`), and it is why a landing on a material with no
    /// window stays silent instead of playing something generic.
    fn post_landing(
        &mut self,
        runtime: &mut AuthoredRuntime,
        request: &VoiceRequest,
        audio: &AudioState,
    ) -> Result<(), String> {
        let air_time = request.air_time.unwrap_or(0.0);
        let surface = audio.wheel_material_620[0] as i32;
        let board = self.landing_material;
        // `sub_824BA630` @ 0x824BA7D0 normalizes before it does anything else:
        //
        //     fdivs f13,f29,f0 ; fneg f12,f13 ; fsel f11,f12,f30,f13   (max with 0.0)
        //     fsubs f10,f31,f11 ; fsel f29,f10,f11,f31                 (min with 1.0)
        //
        // i.e. `clamp(air / D, 0, 1)` with `D` = `Hash_6D68BC2D1A23C29A` (0.4 in the owner's
        // vault). **The tier test and both windows are in these ratio units, not seconds** — so
        // retail's tier boundary is 0.04 s of air and its curve tops out at 0.12 s, not 0.1 and
        // 0.3. Passing raw seconds stretched the whole curve by 2.5×.
        let weight = landing_weight(air_time, self.landing_divisor);
        let tier = i32::from(weight >= self.landing_threshold);
        let (lo, hi) = if tier == 0 {
            (0.0, self.landing_threshold)
        } else {
            (self.landing_threshold, self.landing_ceiling)
        };
        // The argument order is retail's: the first call is `sub_82496C58(r4 = surface, r5 =
        // board)` and the second `(r4 = board, r5 = surface)`.
        let level_surface = self.collision_materials.contact_level(
            surface,
            board,
            tier,
            lo,
            hi,
            weight,
            &self.surfaces,
        );
        let level_board = self.collision_materials.contact_level(
            board,
            surface,
            tier,
            lo,
            hi,
            weight,
            &self.surfaces,
        );
        // Each level word then gets its own vault multiplier before it reaches the message
        // (`fmuls f10,f11,f0 ; fctiwz` at 0x824BAAC0 and 0x824BAB20): 0.65 for the first,
        // 1.0 for the second.
        let level_a = scale_level(level_surface, self.landing_gain_a);
        let level_b = scale_level(level_board, self.landing_gain_b);
        self.report(&format!(
            "Landing surface={surface} board={board} class={:?} tier={tier} air={air_time:.3} weight={weight:.3} levels={level_a}/{level_b}",
            request.landing_class
        ));
        // Retail's own guard: either level at 0 and there is no message.
        if level_a == 0 || level_b == 0 {
            return Ok(());
        }
        // `sub_82486EF0(this, r28 = board, r27 = surface, …, r9 = level_a, …)` — the message's
        // materials are (board, surface) but the level words are **crossed**: `+0x20` carries the
        // level computed for the surface and `+0x24` the one computed for the board. `sub_824D2318`
        // reads `msg[0x20 + 4 × record]` (`add r6,r28,r11` with `r28 = 32 - (r1+80)`), so record 0
        // — the board — really is leveled by the surface's table entry.
        let message = ContactMessage {
            material_a: board,
            material_b: surface,
            tier_a: tier,
            tier_b: tier,
            level_a,
            level_b,
            ..ContactMessage::default()
        };
        let slot = self.collision.post(message, &self.collision_materials);
        let started = self.collision.slots()[slot].started;
        for record in 0..2 {
            if !started[record] {
                continue;
            }
            let other = self
                .collision_materials
                .other_class(message.material(1 - record), &self.surfaces);
            let Some(sample) = self.collision_materials.sample(
                message.material(record),
                other,
                message.tier(record),
            ) else {
                continue;
            };
            // `material_a` is the board, `material_b` the surface, matching the two levels.
            let level = if record == 0 {
                message.level_a
            } else {
                message.level_b
            };
            let category = self.collision_materials.category(message.material(record));
            self.play_collision(
                runtime,
                &sample,
                level,
                collision_controller_scale(category),
                LANDING_TRIM,
            );
        }
        Ok(())
    }

    /// Start one collision voice. Retail opens these through `sub_82975700` and keeps them live
    /// under `sub_824D2318`; this uses the same Splice one-shot path the pops and the landing
    /// already use, with the material record's level as the voice's gain.
    fn play_collision(
        &mut self,
        runtime: &mut AuthoredRuntime,
        sample: &CollisionSample,
        contact_level: u32,
        // `sub_824D20E8`'s output for this material's category, as a 0..1 scale; see
        // [`collision_controller_scale`].
        controller_scale: f32,
        // `LANDING_TRIM` when these two voices are part of a landing stack, 1.0 for a grind
        // onset — the grind's voice count did not change, so it keeps `CONTACT_TRIM` alone.
        trim: f32,
    ) {
        let members =
            match self
                .banks
                .resolve(sample.bank, sample.sample, &mut self.state, &mut self.rand)
            {
                Ok(members) => members,
                Err(error) => {
                    self.report(&format!(
                        "{} sample {:#x}: {error}",
                        sample.bank, sample.sample
                    ));
                    return;
                }
            };
        let Some(base) = runtime.bank_base(sample.bank) else {
            self.report(&format!("Splice bank {} is not installed", sample.bank));
            return;
        };
        // The material's `+52` is a *static* per-material level; on its own it made every landing
        // arrive at the same near-full gain, which is what "all the landings sound the same" was.
        // What varies is `sub_82496C58`'s per-contact level — air time for a landing, impact for a
        // grind — so it modulates the voice here. Retail carries it through the Collision
        // controller's outputs instead; these voices do not read that controller, so applying it
        // directly is the closest the one-shot path gets.
        // `sub_824D2318`: the voice's gain is the material's `+52` level **times the Collision
        // controller output `sub_824D20E8` picks for the material's category** times the
        // message's contact level. The controller term was missing, so both of a landing's
        // collision voices played at full material level and drowned the class voice that is
        // supposed to make one landing differ from another.
        let gain = (sample.level as f32 / 32_767.0)
            * (contact_level as f32 / 32_767.0)
            * controller_scale
            * trim;
        self.report(&format!(
            "collision voice {} #{:#x} material level {} x contact {} -> {} member(s) at gain x{gain:.3}",
            sample.bank,
            sample.sample,
            sample.level,
            contact_level,
            members.len(),
        ));
        for member in members {
            match runtime.play_oneshot(&OneshotVoice {
                sample: base + member.stream_offset,
                gain: member.values.gain * gain,
                pitch: member.values.pitch,
                delay: member.values.delay,
                pan: member.pan,
                bus: OneshotBus::Default,
            }) {
                // Hold the handle: dropping it left a delayed member permanently unopened and a
                // playing one never released.
                Ok(handle) => self.collision_live.push(handle),
                Err(error) => {
                    self.report(&format!(
                        "{} sample {:#x} member: {error}",
                        sample.bank, sample.sample
                    ));
                }
            }
        }
    }

    /// Play one front-end one-shot: retail's `GlobalFEPlaySound` (`sub_825DFAF0`), as far as this
    /// engine has a front end.
    ///
    /// The session marker's three sounds live in `sk8_menu.bnk` and resolve exactly like a
    /// collision sample, so they take the same Splice path, including each member's **authored
    /// pan**. These containers are panned deliberately — the "go" layers sit at −15, −15, −15 and
    /// −5 degrees — and centring them collapsed that spread, which is what "the go-to marker
    /// didn't have the static" was. A front-end sound having no world position does not make it
    /// mono: the pan is part of the authored mix here exactly as it is for a collision voice.
    pub(crate) fn play_frontend(
        &mut self,
        runtime: &mut AuthoredRuntime,
        sound: super::frontend::FrontEndSound,
    ) {
        let Some(voice) = self.frontend.voice(sound) else {
            self.report(&format!(
                "front-end sound {sound:?} has no record in this installation"
            ));
            return;
        };
        let bank = skate_data::audio::catalog::MENU_BANK;
        let members = match self
            .banks
            .resolve(bank, voice.sample, &mut self.state, &mut self.rand)
        {
            Ok(members) => members,
            Err(error) => {
                self.report(&format!("{bank} sample {:#x}: {error}", voice.sample));
                return;
            }
        };
        let Some(base) = runtime.bank_base(bank) else {
            self.report(&format!("Splice bank {bank} is not installed"));
            return;
        };
        self.report(&format!(
            "front-end voice {sound:?} {bank} #{:#x} -> {} member(s) at gain x{:.3} [{}]",
            voice.sample,
            members.len(),
            voice.gain,
            members
                .iter()
                .map(|m| format!(
                    "s{:#x}@{:.3}s pan{:.0} g{:.2}",
                    m.sample, m.values.delay, m.pan, m.values.gain
                ))
                .collect::<Vec<_>>()
                .join(", "),
        ));
        for member in members {
            match runtime.play_oneshot(&OneshotVoice {
                sample: base + member.stream_offset,
                gain: member.values.gain * voice.gain,
                pitch: member.values.pitch,
                delay: member.values.delay,
                pan: member.pan,
                bus: OneshotBus::Default,
            }) {
                // Held for the same reason the collision voices are: a delayed member that loses
                // its handle never opens.
                Ok(handle) => self.frontend_live.push(handle),
                Err(error) => {
                    self.report(&format!("{bank} sample {:#x} member: {error}", voice.sample));
                }
            }
        }
    }

    /// Report a contact-sound failure once per distinct message (they would otherwise repeat every
    /// landing).
    fn report(&mut self, message: &str) {
        if self.reported.insert(message.to_owned()) {
            eprintln!("SKATE_PLAYER_AUDIO contact_voice_unavailable {message}");
        }
    }

    /// `sub_824B9CC8`'s and `sub_824BA630`'s sample choice.
    fn selection(
        &self,
        play: &Queued,
        audio: &AudioState,
    ) -> Option<(&'static str, u16, OneshotBus)> {
        match play.sound {
            ContactSound::Pop if self.pops_enabled => {
                let class = usize::try_from(play.request.selector.unwrap_or(0)).unwrap_or(0);
                // `sub_824BA310`: the wheel material's AudioSurfaceMap lane +8 plus the soft-wheel
                // test (`sub_82494D78`).
                let material = audio.wheel_material_620[0];
                let lane = if material >= 143 {
                    0
                } else {
                    self.surfaces.lookup(material as i32, 8) as u32
                };
                let category = surface_category(lane, audio.soft_wheels_684 != 0);
                let (bank, sample) = self.pops.sample(class, category, false)?;
                // Replaced with the owner send bus in `drain`.
                Some((bank, sample, OneshotBus::EqChain(self.pops.bus)))
            }
            ContactSound::Landing => Some((
                self.landing.bank(),
                self.landing.sample,
                OneshotBus::Default,
            )),
            // `sub_824BA630` @ 0x824BAC50: the ladder voice's column is `sub_82494D78` of the
            // deck material — which is `AudioSurfaceMap` lane +8, the same lookup the pops use,
            // with retail's own clamp to element 94 for anything out of range. Retail takes
            // column 1 when that lane is non-zero, and the row is `air >= 0.75 s`.
            ContactSound::LandingLadder => {
                let test = play
                    .request
                    .deck_material
                    .is_some_and(|material| self.surfaces.lookup(material as i32, 8) != 0);
                let air = play.request.air_time.unwrap_or(0.0);
                let sample = self.landing.ladder_sample(test, air);
                // Retail opens this one through its own eEQChain (class `42AFE160E647167C`,
                // value 1) rather than the default output bus. This engine does not model those
                // chains for contact one-shots, so it takes the same default bus the impact
                // voice does — an approximation, and the only one in this path.
                (sample != 0).then_some((self.landing.bank(), sample, OneshotBus::Default))
            }
            // `sub_824B8D48` via `sub_824BA3F0`: index `3 × kind + class` with `kind = 0` for a
            // landing, and the mode is the surface category *except* on a class-2 landing, where
            // `and r3,r7,r3` clears it back to 0. See [`LandingTuning::class_sample`], which
            // carries the lifted code: this comment had the polarity backwards until 2026-09-23,
            // and the heaviest landings were silent on three categories out of four as a result.
            //
            // The class is the largest `+448` bucket over the wheels that are in contact
            // (`+464`), exactly as `sub_824BA630`'s tail computes it for MixMap input 2. Retail
            // derives those buckets from time in air, thresholded at 0.62 s and 1.00 s.
            ContactSound::LandingClass => {
                // `sub_824BA630` resolves the class once and uses it for the sample *and* the
                // controller-input 2 write, so the component owns it and passes it here rather
                // than each side recomputing it from the audio state.
                let class = play.request.landing_class?;
                let material = audio.wheel_material_620[0];
                let lane = if material >= 143 {
                    0
                } else {
                    self.surfaces.lookup(material as i32, 8) as u32
                };
                let category = surface_category(lane, audio.soft_wheels_684 != 0);
                let sample = self.landing.class_sample(0, class, category)?;
                // Retail routes this voice through the owner's send bus, whose level is the
                // Contacts controller's output 15 (`sub_824B8D48` @ 0x824B8E88 →
                // `sub_82488DD0`). Output 15 is driven by controller input 2, which
                // `ContactsInputs::process` already writes from the landing class — so retail's
                // landing level really does vary with the drop, and this engine was throwing that
                // away by sending the dry voice to the default bus.
                //
                // Probing the real MixMap under the retail pre-roll
                // (`skate-audio-core` example `contacts_input_probe`) gives output 15 =
                // 2584 / 3103 / 3650 for classes 0 / 1 / 2: a 3.0 dB spread, class 0 to class 2.
                //
                // The absolute level is left where §8 measured it. `CONTACT_TRIM` was calibrated
                // with the send absent, so re-applying the raw send level would drop every landing
                // ~19 dB. Instead the level is taken relative to its class-2 maximum, which keeps
                // the loudest landing exactly where it measures today and lets the lighter classes
                // fall back by retail's own ratios. That is the part the owner could hear missing:
                // every landing arriving at the same level.
                Some((self.landing.bank(), sample, OneshotBus::Default))
            }
            // Pops off (see `pops_enabled`), and the two paths that are a different subsystem.
            ContactSound::Pop | ContactSound::PopRoll | ContactSound::GrindOnset => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{landing_send_scale, landing_weight, scale_level};
    use skate_data::audio::splice::LandingTuning;

    /// The port fed `sub_82496C58` raw seconds, which stretched retail's curve by 2.5× and was
    /// the reason every landing past 0.3 s measured identically. Retail normalizes first.
    #[test]
    fn the_landing_weight_is_a_ratio_not_seconds() {
        let d = 0.4;
        assert_eq!(landing_weight(0.0, d), 0.0);
        // The tier split: retail's 0.1 in ratio units is 0.04 s of air.
        assert!(landing_weight(0.039, d) < 0.1);
        assert!(landing_weight(0.041, d) >= 0.1);
        // The ceiling: 0.3 in ratio units is 0.12 s of air.
        assert!(landing_weight(0.119, d) < 0.3);
        assert!(landing_weight(0.121, d) > 0.3);
        // Clamped at 1.0 — an ollie and a rooftop drop arrive at the same weight, which is
        // retail's own behaviour and not something to "fix" by widening the window.
        assert_eq!(landing_weight(0.4, d), 1.0);
        assert_eq!(landing_weight(3.5, d), 1.0);
        // A missing divisor must not produce inf/NaN and reach the voice gain.
        assert_eq!(landing_weight(0.5, 0.0), 0.0);
    }

    /// `fmuls` + `fctiwz` truncates toward zero.
    #[test]
    fn the_level_gains_truncate() {
        assert_eq!(scale_level(26_000, 0.65), 16_900);
        assert_eq!(scale_level(24_000, 1.0), 24_000);
        assert_eq!(scale_level(1, 0.65), 0);
        assert_eq!(scale_level(0, 0.65), 0);
    }

    /// The class must actually reach the gain. Before the controller-input 2 write was ported
    /// this was flat at every class, which is what made every landing the same loudness.
    #[test]
    fn the_landing_send_follows_the_class() {
        let db = |x: f32| 20.0 * x.log10();
        let (c0, c1, c2) = (
            landing_send_scale(Some(0)),
            landing_send_scale(Some(1)),
            landing_send_scale(Some(2)),
        );
        assert!(c0 < c1 && c1 < c2, "{c0} {c1} {c2}");
        assert_eq!(
            c2, 1.0,
            "class 2 is the reference and keeps the measured level"
        );
        // The real MixMap's spread, class 0 to class 2, is 3.0 dB.
        assert!(
            (db(c2) - db(c0) - 3.0).abs() < 0.1,
            "spread {} dB",
            db(c2) - db(c0)
        );
        // An unknown class must not attenuate, and no class must ever mute the landing.
        assert_eq!(landing_send_scale(None), 1.0);
        assert!(landing_send_scale(Some(99)) >= super::LANDING_SEND_FLOOR);
    }

    /// The 2×2 ladder is the voice this port never played: a landing's sample must change with
    /// time in air, which is how retail makes a hop sound unlike a drop.
    #[test]
    fn the_ladder_picks_a_different_sample_either_side_of_the_split() {
        let tuning = LandingTuning {
            sample: 0x447,
            ladder: [[0x35C, 0x35D], [0x35E, 0x35F]],
            ladder_seconds: 0.75,
            class_modes: [vec![], vec![], vec![], vec![]],
        };
        assert_eq!(tuning.ladder_sample(false, 0.30), 0x35C);
        assert_eq!(tuning.ladder_sample(false, 0.80), 0x35D);
        assert_eq!(tuning.ladder_sample(true, 0.30), 0x35E);
        assert_eq!(tuning.ladder_sample(true, 0.80), 0x35F);
        // The split is inclusive on the hard side (`air_seconds >= ladder_seconds`).
        assert_eq!(tuning.ladder_sample(false, 0.75), 0x35D);
    }
}
