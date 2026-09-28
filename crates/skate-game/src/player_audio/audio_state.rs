//! The retail per-frame audio state: a port of the bridge `sub_824B0DA8`, which copies the
//! per-skater PhysOut record `sub_827A1B78` builds into the audio state every player-sound
//! component reads (the component's `+32`/`+36` pointer). Field names carry the audio-state
//! offset; `docs/player-audio-retail-drivers.md` §1 and §6 list each one's native source.
//!
//! One [`AudioState::update`] runs, in retail order, the PhysOut audio conditioner
//! (`sub_82772748`, the 800-byte object whose output is PhysOut slot B+40; [`Conditioner`]), the
//! record builder's packing (`sub_827A1B78`) and the bridge. The builder packs most flags into
//! bit fields of record words +148..+172; the bridge unpacks them. Those round trips are
//! lossless for the values involved, so the port carries the values directly and cites the
//! record word.
//!
//! Only fields whose native source the engine publishes are here. A family that needs a field
//! the engine does not compute yet stays off rather than reading an invented value. Not ported:
//! +324 (listener distance), +368 (B60+12312 spin bucket), +696..+712 / +717 (B60 fields),
//! +760 / +764 (Air451 / Air+228 handplant).

use skate_core::math::Vector3;
use skate_core::physics::board_motion_output::length as native_length_vector;

use super::tuning::{AudioTrick, AudioTuning};
use crate::skate_audio::RetailAudioInputs;

/// `0x8209975C`: air-time scale of the per-wheel landing factor (bridge, before loc_824B11BC).
const AIR_FACTOR_SCALE: f32 = f32::from_bits(0x3F00_0000);
/// `0x8208EA70`: hold timer speed scale (bridge loc_824B140C).
const HOLD_SPEED_SCALE: f32 = f32::from_bits(0x3DF5_C28F);
/// `0x82181B90`: hold timer time scale (bridge loc_824B140C).
const HOLD_TIME_SCALE: f32 = f32::from_bits(0x3ECC_CCCD);
/// `0x820C6D98`: the slip ring average (`sub_82772E18`).
const QUARTER: f32 = f32::from_bits(0x3E80_0000);
/// `0x821BCD60` / `0x822F8F44`: jump velocity cap and its reciprocal (`sub_82772748`).
const JUMP_VELOCITY_CAP: f32 = f32::from_bits(0x4029_999A);
const JUMP_VELOCITY_SCALE: f32 = f32::from_bits(0x3EC1_3521);
/// `0x8216DEE0`: the record's "no jump velocity this frame" marker (record +132).
const UNSET: f32 = f32::from_bits(0xBF80_0000);
/// `0x8209975C`: SkateboardMotion+200 soft-wheel threshold (builder, before loc_827A2820).
const SOFT_WHEEL_THRESHOLD: f32 = f32::from_bits(0x3F00_0000);
/// `0x822F8E94`, `0x820641A8`, `0x8207268C`: loose-board deck-up thresholds (builder loc_827A2AA8..2B14).
const UPSIDE_DOWN: f32 = f32::from_bits(0xBF66_6666);
const ON_SIDE_HIGH: f32 = f32::from_bits(0x3DCC_CCCD);
const ON_SIDE_LOW: f32 = f32::from_bits(0xBDCC_CCCD);
/// `0x821BCD64`: plant height difference that makes a step (`sub_827729B8`).
const STEP_HEIGHT: f32 = f32::from_bits(0x3D8F_5C29);
/// `0x82063A48`: floor of an active ragdoll contact impact (`sub_82BD60C8`).
const BODY_IMPACT_FLOOR: f32 = f32::from_bits(0x3A83_126F);
/// EScorableID 234 (`hippyjump`), record +152 bit 23 (builder, before loc_827A1FE0).
const HIPPY_JUMP: i32 = 234;

/// Player state byte `offset` (52..=87) from the published state flags.
fn state_flag(inputs: &RetailAudioInputs, offset: usize) -> bool {
    inputs.state_flags[offset - 52]
}

/// PowerPC `fsel`: `a >= 0 ? b : c`, NaN selecting `c`.
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// `vmsum3fp128`.
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The vector length idiom (`vmsum3fp128`, `vrsqrtefp` with two Newton steps, zero selection).
fn native_length(v: [f32; 3]) -> f32 {
    native_length_vector(Vector3::new(v[0], v[1], v[2]))
}

/// The builder's material clamp (loc_827A2370 and its copies): 0 means none (143), otherwise the
/// value minus one, and anything outside 0..=143 is also 143. Signed compares.
fn material(value: u32) -> u32 {
    let value = value as i32;
    if value == 0 {
        return 143;
    }
    let value = value - 1;
    if value > 143 || value < 0 {
        143
    } else {
        value as u32
    }
}

/// `sub_82D2D908`: the category of a physical state id.
fn category(state: u32) -> u32 {
    let state = state as i32;
    match state {
        700.. => 700,
        600.. => 600,
        500.. => 500,
        400.. => 400,
        300.. => 300,
        200.. => 200,
        // 82D2D968: 100 when state >= 100, else 0.
        100.. => 100,
        _ => 0,
    }
}

/// `max(|a|, |b|)` as the builder selects it (`fsubs` then `fsel`).
fn larger_magnitude(a: f32, b: f32) -> f32 {
    let (a, b) = (a.abs(), b.abs());
    fsel(a - b, a, b)
}

/// PhysOut slot B+40 (template `sub_82DE3358`), as the conditioner writes it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ConditionerOutput {
    /// B40+0 (`sub_82772748`): SkateboardMotion+64 projected on the rows of the deck part
    /// transform (B0+0), i.e. the deck-local angular velocity.
    pub deck_angular_velocity_0: [f32; 3],
    /// B40+16 (`sub_82772E18`): rise of the 4-sample average of |lateral deck speed|.
    pub slip_rise_16: f32,
    /// B40+20 (`sub_82772E18`): slip.
    pub slip_20: f32,
    /// B40+24 (`sub_82772748`): |Air+112| / 2.65, capped at 1.
    pub jump_velocity_24: f32,
    /// B40+28 / +32 / +40 / byte208 (`sub_827731C8`): latched grind family, latched last
    /// positive grind impact, latched grind material, grinding.
    pub grind_family_28: u32,
    pub grind_impact_32: f32,
    pub grind_material_40: u32,
    pub grinding_208: bool,
    /// B40+36 (`sub_82772748`): maximum of the last four Collision+24 deck scrapes.
    pub deck_scrape_36: f32,
    /// B40+44 (`sub_82772B88`): landing bucket 1..4.
    pub landing_bucket_44: u32,
    /// B40+48 (`sub_82772D30`): jump bucket 0..2.
    pub jump_bucket_48: u32,
    /// B40+52..+64 / bytes 68..71 (`sub_82772FD8`): per-wheel touchdown impact and landed latch.
    pub wheel_impact_52: [f32; 4],
    pub wheel_landed_68: [bool; 4],
    /// B40+92..+207 (`sub_82773298`): Collision+80..+195 with its first eight floats (the
    /// region impacts) replaced by their maximum over the last four frames.
    pub body_contacts_92: BodyContactBlock,
    /// B40 bytes 211..215 (`sub_827729B8`, reset to 0 each frame by the B+40 template
    /// `sub_82DE3358`): foot physical surface is 8; step up; step down; left / right plant
    /// edges.
    pub foot_surface_8_211: bool,
    pub step_212: bool,
    pub step_213: bool,
    pub left_plant_214: bool,
    pub right_plant_215: bool,
}

/// Collision+80..+195 (`sub_82BD60C8`), as the conditioner forwards it (audio state +496..+611).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BodyContactBlock {
    pub impact: [f32; 8],
    pub slide: [f32; 8],
    pub material: [u32; 8],
    pub specific_current: [bool; 2],
    pub group_8_force: f32,
    pub skater_force: f32,
    pub other_skater: i32,
    pub group_11_force: f32,
}

/// The 800-byte PhysOut audio conditioner (vtable `0x82310A74`, built in `sub_82DF2130`, tail
/// constructor `sub_827725E8`). Fields are named by their object offset.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Conditioner {
    /// +8..+11: previous wheel contact. The constructor tail stores 1.
    wheel_contact_8: [bool; 4],
    /// +12..+24: frames each wheel has been out of contact.
    wheel_air_frames_12: [i32; 4],
    /// +32..+80: previous wheel velocity (SkateboardMotion+208+16i).
    wheel_velocity_32: [[f32; 3]; 4],
    /// +644..+656 / +660: deck scrape ring and its index.
    scrape_ring_644: [f32; 4],
    scrape_index_660: i32,
    /// +664 / +672 / +676: grind latches (0.0, −1, 0 at construction).
    grind_impact_664: f32,
    grind_family_672: u32,
    grind_material_676: u32,
    /// +680..+692 / +696: COM vertical velocity ring and its index.
    landing_ring_680: [f32; 4],
    landing_index_696: i32,
    /// +748..+760 / +764 / +768: |lateral deck speed| ring, index, last average.
    slip_ring_748: [f32; 4],
    slip_index_764: i32,
    slip_average_768: f32,
    /// +772..+784 / +788..+791: wheel impacts and landed latches (zeroed by `sub_82DF2130`).
    wheel_impact_772: [f32; 4],
    wheel_landed_788: [bool; 4],
    /// +176..+639 / +640: four 116-byte copies of Collision+80..+195 and the ring index; only
    /// their first eight floats are read back.
    body_ring_176: [[f32; 8]; 4],
    body_index_640: i32,
    /// +704 / +720: Skeleton+144 / +160 at the last left / right plant edge.
    left_plant_704: [f32; 3],
    right_plant_720: [f32; 3],
    /// +736 / +738: last nonzero low halves of OffBoard+52 / +56.
    foot_surface_736: u16,
    foot_surface_738: u16,
    /// +740 / +741 / +742 / +743: OffBoard bytes 306 / 307 this and the previous frame.
    left_plant_740: bool,
    left_plant_741: bool,
    right_plant_742: bool,
    right_plant_743: bool,
    output: ConditionerOutput,
}

impl Default for Conditioner {
    fn default() -> Self {
        Self {
            wheel_contact_8: [true; 4],
            wheel_air_frames_12: [0; 4],
            wheel_velocity_32: [[0.0; 3]; 4],
            scrape_ring_644: [0.0; 4],
            scrape_index_660: 0,
            grind_impact_664: 0.0,
            grind_family_672: u32::MAX,
            grind_material_676: 0,
            landing_ring_680: [0.0; 4],
            landing_index_696: 0,
            slip_ring_748: [0.0; 4],
            slip_index_764: 0,
            slip_average_768: 0.0,
            wheel_impact_772: [0.0; 4],
            wheel_landed_788: [false; 4],
            body_ring_176: [[0.0; 8]; 4],
            body_index_640: 0,
            left_plant_704: [0.0; 3],
            right_plant_720: [0.0; 3],
            foot_surface_736: 0,
            foot_surface_738: 0,
            left_plant_740: false,
            left_plant_741: false,
            right_plant_742: false,
            right_plant_743: false,
            output: ConditionerOutput {
                grind_family_28: u32::MAX,
                body_contacts_92: BodyContactBlock {
                    other_skater: -1,
                    ..BodyContactBlock::default()
                },
                ..ConditionerOutput::default()
            },
        }
    }
}

impl Conditioner {
    pub(crate) fn output(&self) -> &ConditionerOutput {
        &self.output
    }

    /// `sub_82772748`, in its call order. (`sub_82773298` body contacts, the Skeleton+336
    /// delta, bail bytes 209/210 and `sub_827729B8` have no audio-state reader ported here.)
    fn update(&mut self, inputs: &RetailAudioInputs, tuning: &AudioTuning) {
        self.slip(inputs, tuning);
        self.wheels(inputs, tuning);
        let velocity = native_length(inputs.jump_velocity_delta);
        self.output.jump_velocity_24 = if velocity < 0.0 {
            0.0
        } else if !(velocity <= JUMP_VELOCITY_CAP) {
            1.0
        } else {
            velocity * JUMP_VELOCITY_SCALE
        };
        // Rows x/y/z of the deck part transform, each dotted with Motion+64 by the
        // vmrghw/vmrglw transpose then vmulfp, vmaddfp, vmaddfp.
        let w = inputs.angular_velocity;
        let rows = inputs.deck_rows;
        self.output.deck_angular_velocity_0 = std::array::from_fn(|row| {
            rows[row][2].mul_add(w[2], rows[row][1].mul_add(w[1], rows[row][0] * w[0]))
        });
        self.grind(inputs);
        self.body_contacts(inputs, tuning);
        self.scrape_ring_644[self.scrape_index_660 as usize] = inputs.deck_scrape;
        self.scrape_index_660 = (self.scrape_index_660 + 1) % 4;
        let mut scrape = self.scrape_ring_644[0];
        for &sample in &self.scrape_ring_644[1..] {
            if !(sample <= scrape) {
                scrape = sample;
            }
        }
        self.output.deck_scrape_36 = scrape;
        self.landing(inputs, tuning);
        self.step(inputs);
        self.jump(inputs, tuning);
    }

    /// `sub_82773298` over Collision+80..+195. The impact floats are finished here from the
    /// transported ingredients exactly as `sub_82BD60C8` (loop 82BD68F0) writes Collision+80+4i:
    /// x = weighted change × K164; fsel(0.001 − x, 0.001, x); fsel(−x, 0, x); fsel(1 − x, x, 1).
    fn body_contacts(&mut self, inputs: &RetailAudioInputs, tuning: &AudioTuning) {
        let source = &inputs.body_contacts;
        let impact: [f32; 8] = std::array::from_fn(|i| {
            if !source.contact[i] {
                return 0.0;
            }
            let scaled = source.weighted_change[i] * tuning.body_impact_scale;
            let floored = fsel(BODY_IMPACT_FLOOR - scaled, BODY_IMPACT_FLOOR, scaled);
            let positive = fsel(-floored, 0.0, floored);
            fsel(1.0 - positive, positive, 1.0)
        });
        self.body_ring_176[self.body_index_640 as usize] = impact;
        self.body_index_640 = (self.body_index_640 + 1) % 4;
        let ring = &self.body_ring_176;
        self.output.body_contacts_92 = BodyContactBlock {
            impact: std::array::from_fn(|k| {
                let mut maximum = ring[0][k];
                for slot in &ring[1..] {
                    if !(slot[k] <= maximum) {
                        maximum = slot[k];
                    }
                }
                maximum
            }),
            slide: source.slide,
            material: source.material,
            specific_current: source.specific_current,
            group_8_force: source.group_8_force,
            skater_force: source.skater_force,
            other_skater: source.other_skater,
            group_11_force: source.group_11_force,
        };
    }

    /// `sub_827729B8`: the step code bytes. OffBoard+52 / +56 both carry Processed+2596.
    fn step(&mut self, inputs: &RetailAudioInputs) {
        let surface = inputs.offboard_surface as u16;
        if surface != 0 {
            self.foot_surface_738 = surface;
            self.foot_surface_736 = surface;
        }
        self.output.foot_surface_8_211 = (self.foot_surface_736 >> 7) & 31 == 8;
        self.left_plant_741 = self.left_plant_740;
        self.right_plant_743 = self.right_plant_742;
        self.left_plant_740 = inputs.offboard_feet[0];
        self.right_plant_742 = inputs.offboard_feet[1];
        let right_edge = self.right_plant_742 && !self.right_plant_743;
        let left_edge = self.left_plant_740 && !self.left_plant_741;
        if right_edge {
            self.right_plant_720 = inputs.toe_positions[1];
        }
        if left_edge {
            self.left_plant_704 = inputs.toe_positions[0];
        }
        self.output.right_plant_215 = right_edge;
        self.output.left_plant_214 = left_edge;
        self.output.step_212 = false;
        self.output.step_213 = false;
        let rise = self.right_plant_720[1] - self.left_plant_704[1];
        if !(rise.abs() <= STEP_HEIGHT) {
            if self.right_plant_742 {
                if rise <= 0.0 {
                    self.output.step_213 = true;
                } else {
                    self.output.step_212 = true;
                }
            } else if self.left_plant_740 {
                if rise <= 0.0 {
                    self.output.step_212 = true;
                } else {
                    self.output.step_213 = true;
                }
            }
        }
    }

    /// `sub_82772E18`: slip from the deck's lateral speed, |Motion+80 · deck X|.
    fn slip(&mut self, inputs: &RetailAudioInputs, tuning: &AudioTuning) {
        if (inputs.wheel_count as i32) <= 0 {
            self.output.slip_20 = 0.0;
            self.output.slip_rise_16 = 0.0;
            return;
        }
        let lateral = dot3(inputs.linear_velocity, inputs.deck_rows[0]);
        let speed = fsel(lateral, lateral, -lateral);
        let over = speed - tuning.slip_offset;
        let mut slip = fsel(over, over, 0.0) / tuning.slip_divisor;
        if !(slip <= 1.0) {
            slip = 1.0;
        }
        self.output.slip_20 = slip;
        let ring = &mut self.slip_ring_748;
        ring[self.slip_index_764 as usize] = speed;
        let average = (((ring[1] + ring[0]) + ring[2]) + ring[3]) * QUARTER;
        let rise = average - self.slip_average_768;
        self.output.slip_rise_16 = fsel(rise, rise, 0.0);
        self.slip_average_768 = average;
        self.slip_index_764 = (self.slip_index_764 + 1) % 4;
    }

    /// `sub_82772FD8`: a wheel landing after more than five frames without contact latches and
    /// records its closing speed from the previous frame's wheel velocity; the latch clears
    /// once the wheel has again been off for more than five frames.
    fn wheels(&mut self, inputs: &RetailAudioInputs, tuning: &AudioTuning) {
        for wheel in 0..4 {
            let contact = inputs.wheel_contacts[wheel];
            let previous = self.wheel_contact_8[wheel];
            let frames = self.wheel_air_frames_12[wheel];
            if !previous && frames > 5 {
                self.wheel_landed_788[wheel] = false;
            }
            if contact && !previous && frames > 5 {
                let closing = -dot3(
                    inputs.wheel_contact_normals[wheel],
                    self.wheel_velocity_32[wheel],
                );
                let scaled = fsel(-closing, 0.0, closing) / tuning.wheel_impact_divisor;
                self.wheel_impact_772[wheel] = fsel(1.0 - scaled, scaled, 1.0);
                self.wheel_landed_788[wheel] = true;
            }
            self.wheel_contact_8[wheel] = contact;
            self.wheel_velocity_32[wheel] = inputs.wheel_velocities[wheel];
            self.wheel_air_frames_12[wheel] = if contact { 0 } else { frames.wrapping_add(1) };
        }
        self.output.wheel_impact_52 = self.wheel_impact_772;
        self.output.wheel_landed_68 = self.wheel_landed_788;
    }

    /// `sub_827731C8`: grind family/material latched while Grinds byte316, last positive impact.
    fn grind(&mut self, inputs: &RetailAudioInputs) {
        if inputs.grinding {
            self.grind_family_672 = inputs.grind_family;
            self.grind_material_676 = inputs.grind_audio_surface;
        }
        self.output.grinding_208 =
            inputs.state_category == 400 || (inputs.state == 701 && inputs.grind_flag_323);
        self.output.grind_family_28 = self.grind_family_672;
        self.output.grind_material_40 = self.grind_material_676;
        if !(inputs.grind_impact_speed <= 0.0) {
            self.output.grind_impact_32 = inputs.grind_impact_speed;
            self.grind_impact_664 = inputs.grind_impact_speed;
        } else {
            self.output.grind_impact_32 = self.grind_impact_664;
        }
    }

    /// `sub_82772B88`: |min(0, last four Reckoning+20)| against the vault thresholds.
    fn landing(&mut self, inputs: &RetailAudioInputs, tuning: &AudioTuning) {
        let ring = &mut self.landing_ring_680;
        ring[self.landing_index_696 as usize] = inputs.com_velocity[1];
        let mut lowest = fsel(-ring[0], ring[0], 0.0);
        for &sample in &ring[1..] {
            lowest = fsel(lowest - sample, sample, lowest);
        }
        let speed = lowest.abs();
        let [low, middle, high] = tuning.landing_thresholds;
        self.output.landing_bucket_44 = if !(speed <= high) {
            4
        } else if !(speed <= middle) {
            3
        } else if speed > low {
            2
        } else {
            1
        };
        let next = self.landing_index_696 + 1;
        self.landing_index_696 = if next < 4 { next } else { 0 };
    }

    /// `sub_82772D30`: Ground+300 jump strength against the vault thresholds.
    fn jump(&mut self, inputs: &RetailAudioInputs, tuning: &AudioTuning) {
        let strength = inputs.jump_strength;
        let [low, high] = tuning.jump_thresholds;
        self.output.jump_bucket_48 = if !(strength <= high) {
            2
        } else if !(strength <= low) {
            1
        } else {
            0
        };
    }
}

/// `sub_82481E10` with n = 8: below the first x the first y, at or above the last x the last y,
/// else linear between the bracketing points (the upper y when they share an x).
fn point_graph(x: f32, xs: &[f32; 8], ys: &[f32; 8]) -> f32 {
    if x < xs[0] {
        return ys[0];
    }
    if !(x < xs[7]) {
        return ys[7];
    }
    for i in 1..8 {
        if x < xs[i] {
            let span = xs[i] - xs[i - 1];
            if !(span <= 0.0) {
                return ((ys[i] - ys[i - 1]) / span).mul_add(x - xs[i - 1], ys[i - 1]);
            }
            return ys[i];
        }
    }
    ys[0]
}

/// `sub_824B2268`: a stored air factor's landing bucket.
fn wheel_bucket(factor: f32, tuning: &AudioTuning) -> u32 {
    if !(factor < tuning.wheel_bucket_high) {
        2
    } else if factor >= tuning.wheel_bucket_low {
        1
    } else {
        0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AudioState {
    /// +96: COM velocity (SystemReckoning+16).
    pub com_velocity_96: [f32; 3],
    /// +112: the previous frame's +96.
    pub com_velocity_prev_112: [f32; 3],
    /// +128: +96 minus the previous frame's +96.
    pub com_velocity_delta_128: [f32; 3],
    /// +192: grind family, record +96 = B40+28 (`sub_827731C8`: Grinds+136 latched while
    /// Grinds byte316; −1 until the first grind).
    pub grind_family_192: u32,
    /// +200: wheel count, `(R152 >> 20) & 7`.
    pub wheel_count_200: u32,
    /// +204: turn input, record +100 = Ground+264 (Processed+2676, ProcessOutput).
    pub turn_204: f32,
    /// +208: board ground speed (SkateboardMotion+164), m/s. Rolling, rattle, grind, seams, skid,
    /// squeaks and the MixMap speed inputs all read this, not the 3-D deck speed.
    pub ground_speed_208: f32,
    /// +212: |COM velocity|, record +108 (builder: `vmsum3`, rsqrt with two refinements, zero
    /// selection, sign cleared).
    pub com_speed_212: f32,
    /// +216: the bridge reads it as the previous +212 (8-slot loop loc_824B18A8), then stores +212
    /// into it at the end of the pass; after an update it equals +212.
    pub com_speed_216: f32,
    /// +220: time scale. The engine has no slow-motion timer, so this is normal speed.
    pub time_scale_220: f32,
    /// +224: paused/replay flag.
    pub paused_224: bool,
    /// +228: last positive grind impact speed, record +112 = B40+32 (`sub_827731C8`).
    pub grind_impact_228: f32,
    /// +232: slip, record +116 = B40+20 (`sub_82772E18`: clamp((|Motion+80 · deck X| − K2) /
    /// K1, 0, 1), 0 with no wheel contact).
    pub slip_232: f32,
    /// +236 / +240 / +260: KnownAir time in state, time until landing, jump height.
    pub air_time_236: f32,
    pub air_until_landing_240: f32,
    /// +244..+256: per-wheel air factor stored by `sub_824B2350` (bridge) for the next landing
    /// bucket: clamp(+236 × 0.5, 0, 1) while +332, else 0.
    pub wheel_air_factor_244: [f32; 4],
    pub air_jump_height_260: f32,
    /// +264: signed deck tilt, record +136 = SkateboardMotion+184 (SkateboardBody+256, written
    /// by FillPhysOut `82C02A80`).
    pub deck_tilt_264: f32,
    /// +680: the listener-facing factor `sub_824B2088` writes alongside state controller input 3
    /// (camera/listener direction against the deck). Set by the worker, which owns the camera.
    pub listener_facing_680: f32,
    /// The frame record's multiplier tier flags (`*(0x83083C38)+0x2F0D0`, bits 0x2000 x3 /
    /// 0x4000 x2 / 0x8000 x1.5, `sub_827A2E88`), read by Flips w12 and the Music controller.
    pub multiplier_flags_2f0d0: u32,
    /// +268 / +272: |local toe velocity Y| of foot 1 / foot 0 (record +292 / +288 =
    /// |Skeleton+212| / |Skeleton+196|, `82BF22A0`).
    pub toe_local_speed_y_268: f32,
    pub toe_local_speed_y_272: f32,
    /// +276 / +280: max(|x|, |z|) of the local toe velocity of foot 1 / foot 0 (record +308 /
    /// +296 = Skeleton+208/+216, +192/+200).
    pub toe_local_speed_xz_276: f32,
    pub toe_local_speed_xz_280: f32,
    /// +284 / +288: max(|x|, |z|) of the world foot velocity of foot 0 / foot 1 (record +300 /
    /// +304 = Skeleton+224/+232, +240/+248).
    pub foot_world_speed_xz_284: f32,
    pub foot_world_speed_xz_288: f32,
    /// +292 / +296: |Y of Skeleton+320| / |Y of Skeleton+304| (record +316 / +312): the
    /// angular velocity (body+48) Y of ragdoll parts 16 / 20.
    pub ragdoll_spin_y_292: f32,
    pub ragdoll_spin_y_296: f32,
    /// +300: landing bucket 1..4, record `(+156 >> 2) & 7` = B40+44 (`sub_82772B88`); the bridge
    /// forces 1 when (+343 && +348 ≠ 31) or when the previous pass left +720 at 0.
    pub landing_bucket_300: u32,
    /// +304: jump bucket 0..2, record `+156 & 3` = B40+48 (`sub_82772D30`).
    pub jump_bucket_304: u32,
    /// +308: OffBoard byte311 (record +164 bit 1), the board possession owner's held flag.
    pub board_held_308: bool,
    /// +309: OffBoard byte309 (record +164 bit 0) = Processed2480 bit 18.
    pub hold_309: bool,
    /// +310: set when +309 has been held longer than (1 − min(max(+208 × 0.12, 0), 1)) × 0.4.
    pub hold_expired_310: bool,
    /// +311: +309 latched; +312 the bridge's free-running clock (`+= dt`); +316 the clock when
    /// +311 was set.
    pub hold_active_311: bool,
    pub hold_clock_312: f32,
    pub hold_start_316: f32,
    /// +320: OffBoard byte310 (record +160 bit 0) = Processed2480 bit 8.
    pub offboard_310_320: bool,
    /// +328: |Skeleton+288| (record +140): the angular speed of ragdoll part 23.
    pub ragdoll_spin_328: f32,
    /// +332: KnownAir, Air byte438 (record +148 bit 31).
    pub in_known_air_332: bool,
    /// +333: left push foot planted, `P && !338 && 337`.
    pub push_left_333: bool,
    /// +334: right push foot planted, `P && 338`.
    pub push_right_334: bool,
    /// +335: rising edge of a push-foot plant, `P && !prev334 && !prev333`. Retail's rattle trigger.
    pub push_plant_335: bool,
    /// +336: brake foot planted (State52).
    pub brake_336: bool,
    /// +337: push event present (State56).
    pub push_event_337: bool,
    /// +338: push event on the right toe (State57).
    pub push_right_toe_338: bool,
    /// +339: ManualBrake (State54).
    pub manual_brake_339: bool,
    /// +340: balance non-zero (State60).
    pub balance_340: bool,
    /// +341: grinding, record +148 bit 25 = B40 byte208 (`sub_827731C8`: State+12 == 400 ||
    /// (State+16 == 701 && Grinds byte323)); +342 the previous pass's +341.
    pub grinding_341: bool,
    pub grinding_prev_342: bool,
    /// +343: trick active, record +148 bit 24 (B60+152 ≠ −1).
    pub trick_active_343: bool,
    /// +344: audio trick flag, record +148 bit 23 (`eSk8AudioTricks` layout byte +176).
    pub trick_flag_344: bool,
    /// +348 / +352: audio trick ids, record +180 / +184 (layout +164 / +172; −1 without a
    /// trick or without a vault collection for its name).
    pub audio_trick_348: u32,
    pub audio_trick_352: u32,
    /// +372: record +152 bit 23, B60+152 == 234 (`hippyjump`).
    pub hippy_jump_372: bool,
    /// +384..+432: wheel body positions, record +208..+256 = B0+208+16i.
    pub wheel_position_384: [[f32; 3]; 4],
    /// +448..+460: per-wheel landing bucket 0..2 (`sub_824B2350` → `sub_824B2268` of the stored
    /// +244 factor), updated while the new factor is > 0, the wheel's +464 latch is set, or
    /// +341. The vault's `642CF9BFEC6BE988` selects this path.
    pub wheel_landing_bucket_448: [u32; 4],
    /// +464..+467: per-wheel landed latch, record +148 bits 22..19 = B40 bytes 68..71
    /// (`sub_82772FD8`).
    pub wheel_landed_464: [bool; 4],
    /// +468: jump velocity, record +132 = B40+24 while Air byte440, else −1.0 and the bridge
    /// keeps its previous value.
    pub jump_velocity_468: f32,
    /// +480: deck-local angular velocity, record +272 = B40+0.
    pub deck_angular_velocity_480: [f32; 3],
    /// +496..+524: ragdoll contact impact per region, the maximum of four conditioner frames
    /// (B40+92), then scaled by the bridge (loop loc_824B18A8) with the holder +36 graph of the
    /// previous pass's +212.
    pub body_impact_496: [f32; 8],
    /// +528..+556: ragdoll contact tangential (slide) speed per region (Collision+112).
    pub body_slide_528: [f32; 8],
    /// +560..+588: ragdoll contact material per region, `tag & 0x7F`, 0 = none (Collision+144).
    pub body_material_560: [u32; 8],
    /// +592 / +593: groin / face specific contact current (Collision bytes 176 / 177).
    pub groin_contact_592: bool,
    pub face_contact_593: bool,
    /// +596 / +600 / +604 / +608: maximum group-8 force, maximum skater force, other skater
    /// (−1 none), maximum group-11 force (Collision+180..+192).
    pub group_8_force_596: f32,
    pub skater_force_600: f32,
    pub other_skater_604: i32,
    pub group_11_force_608: f32,
    /// +612 / +613 / +614: front truck / back truck / deck contact (record +148 bits 10/9/8 =
    /// Collision bytes 3473/3474/3475).
    pub front_truck_contact_612: bool,
    pub back_truck_contact_613: bool,
    pub deck_contact_614: bool,
    /// +615 / +616: feet in the deck box (record +148 bits 7/6 = Skeleton bytes 600/601).
    pub foot_in_deck_box_615: bool,
    pub foot_in_deck_box_616: bool,
    /// +620..+632: per-wheel audio material (Collision+3440+4i clamped, 143 = none).
    pub wheel_material_620: [u32; 4],
    /// +636..+648: per-wheel seam pattern (Collision+3456+4i, `(tag >> 12) & 0xF`).
    pub wheel_seam_636: [u32; 4],
    /// +652 / +656 / +660: front truck / back truck / deck audio material (Collision+4/+8/+12
    /// clamped, 143 = none).
    pub front_truck_material_652: u32,
    pub back_truck_material_656: u32,
    pub deck_material_660: u32,
    /// +664: deck slide speed, record +480 = Collision+20.
    pub deck_slide_speed_664: f32,
    /// +668: deck scrape, record +484 = B40+36.
    pub deck_scrape_668: f32,
    /// +672: 0.25 × (((Skeleton+572 + +568) + +564) + +560) (record +144), the mean limb speed
    /// relative to the COM.
    pub limb_speed_672: f32,
    /// +676: bail (State59).
    pub bail_676: bool,
    /// +677: end of bail, record +148 bit 4 = Skeleton byte599.
    pub bail_over_677: bool,
    /// +684: soft wheels, record +148 bit 3 = SkateboardMotion+200 < 0.5 (Processed+2764).
    pub soft_wheels_684: u32,
    /// +690: revert (State66).
    pub revert_690: bool,
    /// +692: grind material, record +512 = B40+40 clamped (143 = none).
    pub grind_material_692: u32,
    /// +716: record +152 bit 30, `sub_82D2D908(State+16) == 500` (any 5xx state).
    pub walking_716: bool,
    /// +718: record +152 bit 24, Filtered+0 == 7 (OffboardAir); +720 = 20 while set, then
    /// counts down to 0.
    pub offboard_air_718: bool,
    pub offboard_air_countdown_720: i32,
    /// +712: record +516 = Skeleton+516, the pumping absorption (Pumping+56, −speed × angular
    /// speed), which the board component's slope inputs (`sub_824CA738`) read.
    pub pump_absorption_712: f32,
    /// +724 / +725: right and left foot down levels.
    pub foot_down_right_724: bool,
    pub foot_down_left_725: bool,
    /// +728 / +732: foot materials from +738 / +736 (`& 0x7F`, clamped, 143 = none).
    pub foot_material_728: u32,
    pub foot_material_732: u32,
    /// +736 / +738: foot surface words, record +200 / +202 = low halves of OffBoard+56 / +52
    /// (Air+224 while Air byte448), 1 when zero.
    pub foot_surface_736: u16,
    pub foot_surface_738: u16,
    /// +740: step code, record `(+152 >> 25) & 7` from B40 bytes 211..213: 212 ? (211 ? 2 : 4)
    /// : 213 ? (211 ? 3 : 5) : 1.
    pub step_code_740: u32,
    /// +768: Air byte448 (record +156 bit 7).
    pub footplant_768: bool,
    /// +780: loose board, record `(+156 >> 5) & 3`: with (+676 or +716), deck contact and deck
    /// material < 94, d = effective deck up (B0+80) · Ground+80: d < −0.9 → 1, −0.1 < d < 0.1 → 2.
    pub loose_board_780: u32,
    /// +796: `AudibleFootStepStrength`.
    pub footstep_strength_796: f32,
    conditioner: Conditioner,
}

impl AudioState {
    /// The conditioner output (PhysOut slot B+40) of the last update.
    pub(crate) fn conditioner(&self) -> &ConditionerOutput {
        self.conditioner.output()
    }

    /// One conditioner pass, record build and bridge pass. Stores follow `sub_824B0DA8`'s order
    /// where a later field reads an earlier one (+335 before +333/+334, +342 before +341, +300
    /// before +720, +216 last).
    pub(crate) fn update(&mut self, inputs: &RetailAudioInputs, tuning: &AudioTuning) {
        self.conditioner.update(inputs, tuning);
        let b40 = self.conditioner.output;

        self.com_velocity_prev_112 = self.com_velocity_96;
        self.com_velocity_96 = inputs.com_velocity;
        self.com_velocity_delta_128 =
            std::array::from_fn(|i| self.com_velocity_96[i] - self.com_velocity_prev_112[i]);
        self.wheel_count_200 = inputs.wheel_count & 7;
        self.ground_speed_208 = inputs.ground_speed;
        self.com_speed_212 = native_length(inputs.com_velocity).abs();
        self.time_scale_220 = 1.0;
        self.paused_224 = false;
        self.turn_204 = inputs.turn;
        self.in_known_air_332 = inputs.in_known_air;

        let planted = state_flag(inputs, 55);
        self.brake_336 = state_flag(inputs, 52);
        self.manual_brake_339 = state_flag(inputs, 54);
        self.push_event_337 = state_flag(inputs, 56);
        self.push_right_toe_338 = state_flag(inputs, 57);
        self.push_plant_335 = planted && !self.push_right_334 && !self.push_left_333;
        self.push_right_334 = planted && self.push_right_toe_338;
        self.push_left_333 = planted && !self.push_right_toe_338 && self.push_event_337;
        self.grinding_prev_342 = self.grinding_341;
        self.balance_340 = state_flag(inputs, 60);
        self.grinding_341 = b40.grinding_208;
        self.grind_family_192 = b40.grind_family_28;
        self.grind_impact_228 = b40.grind_impact_32;
        self.slip_232 = b40.slip_20;
        self.air_time_236 = inputs.air_time_in_state;
        self.air_until_landing_240 = inputs.air_time_until_landing;
        self.air_jump_height_260 = inputs.air_jump_height;
        self.deck_tilt_264 = inputs.deck_tilt;
        let record_132 = if inputs.air_440 {
            b40.jump_velocity_24
        } else {
            UNSET
        };
        if record_132 != UNSET {
            self.jump_velocity_468 = record_132;
        }

        let trick: Option<AudioTrick> = if inputs.scorable_id != -1 {
            tuning.trick(inputs.scorable_id)
        } else {
            None
        };
        self.trick_active_343 = inputs.scorable_id != -1;
        self.trick_flag_344 = trick.is_some_and(|trick| trick.flag_176);
        self.audio_trick_348 = trick.map_or(u32::MAX, |trick| trick.id_164);
        self.audio_trick_352 = trick.map_or(u32::MAX, |trick| trick.id_172);
        self.hippy_jump_372 = inputs.scorable_id == HIPPY_JUMP;
        self.wheel_position_384 = inputs.wheel_positions;
        self.wheel_landed_464 = b40.wheel_landed_68;

        self.bridge_wheel_landing(tuning);
        self.deck_angular_velocity_480 = b40.deck_angular_velocity_0;
        [self.front_truck_contact_612, self.back_truck_contact_613] = inputs.truck_contacts;
        self.deck_contact_614 = inputs.deck_contact;
        self.wheel_material_620 = inputs.wheel_audio_surfaces.map(material);
        self.wheel_seam_636 = inputs.wheel_seam_patterns;
        [
            self.front_truck_material_652,
            self.back_truck_material_656,
            self.deck_material_660,
        ] = inputs.part_audio_surfaces.map(material);
        self.deck_slide_speed_664 = inputs.deck_slide_speed;
        self.deck_scrape_668 = b40.deck_scrape_36;
        let [limb_560, limb_564, limb_568, limb_572] = inputs.limb_speeds;
        self.limb_speed_672 = (((limb_572 + limb_568) + limb_564) + limb_560) * QUARTER;
        let body = b40.body_contacts_92;
        self.body_impact_496 = body.impact;
        self.body_slide_528 = body.slide;
        self.body_material_560 = body.material;
        [self.groin_contact_592, self.face_contact_593] = body.specific_current;
        self.group_8_force_596 = body.group_8_force;
        self.skater_force_600 = body.skater_force;
        self.other_skater_604 = body.other_skater;
        self.group_11_force_608 = body.group_11_force;
        [self.foot_in_deck_box_615, self.foot_in_deck_box_616] = inputs.feet_in_deck_box;

        let [local_0, local_1] = inputs.foot_local_velocity;
        let [world_0, world_1] = inputs.foot_world_velocity;
        self.toe_local_speed_y_272 = local_0[1].abs();
        self.toe_local_speed_y_268 = local_1[1].abs();
        self.toe_local_speed_xz_280 = larger_magnitude(local_0[0], local_0[2]);
        self.toe_local_speed_xz_276 = larger_magnitude(local_1[0], local_1[2]);
        self.foot_world_speed_xz_284 = larger_magnitude(world_0[0], world_0[2]);
        self.foot_world_speed_xz_288 = larger_magnitude(world_1[0], world_1[2]);
        // Builder: +312/+316 are |Y| of Skeleton+304/+320 (vspltw 1, sign cleared), +140 the
        // length of Skeleton+288.
        let [spin_288, spin_304, spin_320] = inputs.ragdoll_spin;
        self.ragdoll_spin_y_292 = spin_320[1].abs();
        self.ragdoll_spin_y_296 = spin_304[1].abs();
        self.ragdoll_spin_328 = native_length(spin_288);

        self.bridge_landing_bucket(b40.landing_bucket_44);
        self.jump_bucket_304 = b40.jump_bucket_48 & 3;

        self.bridge_hold(inputs.offboard_311, inputs.offboard_309, inputs.dt);
        self.offboard_310_320 = inputs.offboard_310;

        self.bail_676 = state_flag(inputs, 59);
        self.bail_over_677 = inputs.skeleton_599;
        self.soft_wheels_684 = u32::from(!(inputs.motion_200 >= SOFT_WHEEL_THRESHOLD));
        self.revert_690 = state_flag(inputs, 66);
        self.grind_material_692 = material(b40.grind_material_40);
        self.pump_absorption_712 = inputs.pump_absorption;
        self.walking_716 = category(inputs.state) == 500;
        self.bridge_offboard_air(inputs.filtered_state == 7);
        self.foot_down_right_724 =
            inputs.offboard_feet[1] || inputs.footplant[1] || self.push_right_334 || self.brake_336;
        self.foot_down_left_725 =
            inputs.offboard_feet[0] || inputs.footplant[0] || self.push_left_333;

        // Builder loc_827A26B4..2714: record +200/+202 are the low halves of OffBoard+56/+52,
        // or of Air+224 while Air byte448.
        let surface = if inputs.footplant_448 {
            inputs.footplant_surface
        } else {
            inputs.offboard_surface
        } as u16;
        self.bridge_foot_surfaces(surface, inputs.footplant_448);

        // Builder loc_827A2A60..loc_827A2B14.
        self.loose_board_780 = if (self.bail_676 || self.walking_716)
            && self.deck_contact_614
            && (self.deck_material_660 as i32) < 94
        {
            let d = dot3(inputs.effective_deck_up, inputs.ground_normal);
            if !(d >= UPSIDE_DOWN) {
                1
            } else if !(d >= ON_SIDE_HIGH) && !(d <= ON_SIDE_LOW) {
                2
            } else {
                0
            }
        } else {
            0
        };
        // Builder loc_827A2714..278C (record +152 bits 4..6).
        self.step_code_740 = if b40.step_212 {
            if b40.foot_surface_8_211 { 2 } else { 4 }
        } else if b40.step_213 {
            if b40.foot_surface_8_211 { 3 } else { 5 }
        } else {
            1
        };
        self.footstep_strength_796 = inputs.footstep_strength;
        // Bridge loc_824B18A8: +496..+524 scaled by the holder +36 graph at +216, which still
        // holds the previous pass's +212 here.
        let scale = point_graph(
            self.com_speed_216,
            &tuning.body_impact_speed_x,
            &tuning.body_impact_speed_y,
        );
        for impact in &mut self.body_impact_496 {
            *impact = scale * *impact;
        }
        self.com_speed_216 = self.com_speed_212;
    }

    /// Bridge loc_824B11BC..loc_824B1280, the `642CF9BFEC6BE988` path: `sub_824B2350` per wheel
    /// with f29 = clamp(+236 × 0.5, 0, 1) while +332, else 0.
    fn bridge_wheel_landing(&mut self, tuning: &AudioTuning) {
        let factor = if self.in_known_air_332 {
            let scaled = self.air_time_236 * AIR_FACTOR_SCALE;
            let scaled = fsel(-scaled, 0.0, scaled);
            fsel(1.0 - scaled, scaled, 1.0)
        } else {
            0.0
        };
        for wheel in 0..4 {
            if factor > 0.0 || self.wheel_landed_464[wheel] || self.grinding_341 {
                self.wheel_landing_bucket_448[wheel] =
                    wheel_bucket(self.wheel_air_factor_244[wheel], tuning);
                self.wheel_air_factor_244[wheel] = factor;
            }
        }
    }

    /// Bridge, before loc_824B13B0: record `(+156 >> 2) & 7`, forced to 1 by an active trick
    /// other than audio trick 31 or by the previous pass's +720 being 0.
    fn bridge_landing_bucket(&mut self, conditioner_bucket: u32) {
        self.landing_bucket_300 = conditioner_bucket & 7;
        if self.trick_active_343 && self.audio_trick_348 != 31 {
            self.landing_bucket_300 = 1;
        }
        if self.offboard_air_countdown_720 == 0 {
            self.landing_bucket_300 = 1;
        }
    }

    /// Bridge 824B13C0..loc_824B1464 (+308..+316).
    fn bridge_hold(&mut self, board_held: bool, hold: bool, dt: f32) {
        self.board_held_308 = board_held;
        self.hold_expired_310 = false;
        self.hold_309 = hold;
        if self.hold_309 {
            if !self.hold_active_311 {
                self.hold_active_311 = true;
                self.hold_start_316 = self.hold_clock_312;
            }
            let scaled = self.ground_speed_208 * HOLD_SPEED_SCALE;
            let scaled = fsel(-scaled, 0.0, scaled);
            if !(self.hold_clock_312 <= self.hold_start_316) {
                let held = self.hold_clock_312 - self.hold_start_316;
                let capped = fsel(1.0 - scaled, scaled, 1.0);
                if !(held <= (1.0 - capped) * HOLD_TIME_SCALE) {
                    self.hold_expired_310 = true;
                }
            }
        } else {
            self.hold_active_311 = false;
        }
        self.hold_clock_312 += dt;
    }

    /// Bridge 824B1620..loc_824B1670: +718, and +720 = 20 while it is set, then down to 0.
    fn bridge_offboard_air(&mut self, offboard_air: bool) {
        self.offboard_air_718 = offboard_air;
        if self.offboard_air_718 {
            self.offboard_air_countdown_720 = 20;
        } else if self.offboard_air_countdown_720 > 0 {
            self.offboard_air_countdown_720 -= 1;
        }
    }

    /// Bridge 824B16C8..16F8 (+736/+738: the record half, 1 when zero), +768, and
    /// 824B1838..1898 (+728/+732: `& 0x7F` of +738/+736, clamped).
    fn bridge_foot_surfaces(&mut self, surface: u16, footplant: bool) {
        self.foot_surface_736 = if surface != 0 { surface } else { 1 };
        self.foot_surface_738 = if surface != 0 { surface } else { 1 };
        self.footplant_768 = footplant;
        self.foot_material_728 = material(u32::from(self.foot_surface_738 & 0x7f));
        self.foot_material_732 = material(u32::from(self.foot_surface_736 & 0x7f));
    }
}

/// Words of the retail capture's state dump (`state.tsv`): word i is audio-state +192+4i, big
/// endian, bytes big-endian within the word.
pub(crate) const CAPTURE_FIRST_OFFSET: usize = 192;
pub(crate) const CAPTURE_WORDS: usize = 160;

struct CaptureWords<'a>(&'a [u32]);

impl CaptureWords<'_> {
    fn word(&self, offset: usize) -> u32 {
        self.0[(offset - CAPTURE_FIRST_OFFSET) / 4]
    }
    fn float(&self, offset: usize) -> f32 {
        f32::from_bits(self.word(offset))
    }
    fn byte(&self, offset: usize) -> bool {
        (self.word(offset & !3) >> ((3 - (offset & 3)) * 8)) & 0xff != 0
    }
    fn half(&self, offset: usize) -> u16 {
        (self.word(offset & !3) >> ((2 - (offset & 2)) * 8)) as u16
    }
    fn vector(&self, offset: usize) -> [f32; 3] {
        std::array::from_fn(|lane| self.float(offset + 4 * lane))
    }
}

impl AudioState {
    /// The audio state as retail's bridge left it, from one capture row (160 words from +192).
    /// Every field at +192 and above is filled with the same meaning [`AudioState::update`]
    /// gives it; +96/+112/+128 lie below the dump and stay zero, and the conditioner (a PhysOut
    /// object, not audio state) stays at its constructed state.
    pub(crate) fn from_capture(words: &[u32]) -> Self {
        assert!(
            words.len() >= CAPTURE_WORDS,
            "a capture row holds {CAPTURE_WORDS} words"
        );
        let w = CaptureWords(words);
        Self {
            grind_family_192: w.word(192),
            wheel_count_200: w.word(200),
            turn_204: w.float(204),
            ground_speed_208: w.float(208),
            com_speed_212: w.float(212),
            com_speed_216: w.float(216),
            time_scale_220: w.float(220),
            paused_224: w.byte(224),
            grind_impact_228: w.float(228),
            slip_232: w.float(232),
            air_time_236: w.float(236),
            air_until_landing_240: w.float(240),
            wheel_air_factor_244: std::array::from_fn(|i| w.float(244 + 4 * i)),
            air_jump_height_260: w.float(260),
            deck_tilt_264: w.float(264),
            listener_facing_680: w.float(680),
            multiplier_flags_2f0d0: 0,
            toe_local_speed_y_268: w.float(268),
            toe_local_speed_y_272: w.float(272),
            toe_local_speed_xz_276: w.float(276),
            toe_local_speed_xz_280: w.float(280),
            foot_world_speed_xz_284: w.float(284),
            foot_world_speed_xz_288: w.float(288),
            ragdoll_spin_y_292: w.float(292),
            ragdoll_spin_y_296: w.float(296),
            ragdoll_spin_328: w.float(328),
            landing_bucket_300: w.word(300),
            jump_bucket_304: w.word(304),
            board_held_308: w.byte(308),
            hold_309: w.byte(309),
            hold_expired_310: w.byte(310),
            hold_active_311: w.byte(311),
            hold_clock_312: w.float(312),
            hold_start_316: w.float(316),
            offboard_310_320: w.byte(320),
            in_known_air_332: w.byte(332),
            push_left_333: w.byte(333),
            push_right_334: w.byte(334),
            push_plant_335: w.byte(335),
            brake_336: w.byte(336),
            push_event_337: w.byte(337),
            push_right_toe_338: w.byte(338),
            manual_brake_339: w.byte(339),
            balance_340: w.byte(340),
            grinding_341: w.byte(341),
            grinding_prev_342: w.byte(342),
            trick_active_343: w.byte(343),
            trick_flag_344: w.byte(344),
            audio_trick_348: w.word(348),
            audio_trick_352: w.word(352),
            hippy_jump_372: w.byte(372),
            wheel_position_384: std::array::from_fn(|i| w.vector(384 + 16 * i)),
            wheel_landing_bucket_448: std::array::from_fn(|i| w.word(448 + 4 * i)),
            wheel_landed_464: std::array::from_fn(|i| w.byte(464 + i)),
            jump_velocity_468: w.float(468),
            deck_angular_velocity_480: w.vector(480),
            front_truck_contact_612: w.byte(612),
            back_truck_contact_613: w.byte(613),
            deck_contact_614: w.byte(614),
            foot_in_deck_box_615: w.byte(615),
            foot_in_deck_box_616: w.byte(616),
            wheel_material_620: std::array::from_fn(|i| w.word(620 + 4 * i)),
            wheel_seam_636: std::array::from_fn(|i| w.word(636 + 4 * i)),
            front_truck_material_652: w.word(652),
            back_truck_material_656: w.word(656),
            deck_material_660: w.word(660),
            deck_slide_speed_664: w.float(664),
            deck_scrape_668: w.float(668),
            limb_speed_672: w.float(672),
            body_impact_496: std::array::from_fn(|i| w.float(496 + 4 * i)),
            body_slide_528: std::array::from_fn(|i| w.float(528 + 4 * i)),
            body_material_560: std::array::from_fn(|i| w.word(560 + 4 * i)),
            groin_contact_592: w.byte(592),
            face_contact_593: w.byte(593),
            group_8_force_596: w.float(596),
            skater_force_600: w.float(600),
            other_skater_604: w.word(604) as i32,
            group_11_force_608: w.float(608),
            bail_676: w.byte(676),
            bail_over_677: w.byte(677),
            soft_wheels_684: w.word(684),
            revert_690: w.byte(690),
            grind_material_692: w.word(692),
            pump_absorption_712: w.float(712),
            walking_716: w.byte(716),
            offboard_air_718: w.byte(718),
            offboard_air_countdown_720: w.word(720) as i32,
            foot_down_right_724: w.byte(724),
            foot_down_left_725: w.byte(725),
            foot_material_728: w.word(728),
            foot_material_732: w.word(732),
            foot_surface_736: w.half(736),
            foot_surface_738: w.half(738),
            step_code_740: w.word(740),
            footplant_768: w.byte(768),
            loose_board_780: w.word(780),
            footstep_strength_796: w.float(796),
            ..Self::default()
        }
    }
}

/// Every row of a retail capture state dump (`state.tsv`: frame, milliseconds, then
/// [`CAPTURE_WORDS`] hex words from +192), as `(frame, state)` in file order.
pub(crate) fn load_capture_states(
    path: &std::path::Path,
) -> Result<Vec<(u32, AudioState)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(row, line)| {
            let mut columns = line.split('\t');
            let frame = columns
                .next()
                .and_then(|frame| frame.trim().parse::<u32>().ok())
                .ok_or_else(|| format!("{}:{}: no frame", path.display(), row + 1))?;
            let words = columns
                .skip(1)
                .map(|word| u32::from_str_radix(word.trim(), 16))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("{}:{}: {e}", path.display(), row + 1))?;
            if words.len() < CAPTURE_WORDS {
                return Err(format!(
                    "{}:{}: {} words, expected {CAPTURE_WORDS}",
                    path.display(),
                    row + 1,
                    words.len()
                ));
            }
            Ok((frame, AudioState::from_capture(&words)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stock vault values (`tuning.rs` test checks them against the vault).
    fn tuning() -> AudioTuning {
        AudioTuning {
            slip_divisor: 45.0,
            slip_offset: -0.75,
            landing_thresholds: [
                1.5,
                f32::from_bits(0x4026_6666),
                f32::from_bits(0x405C_CCCD),
            ],
            jump_thresholds: [f32::from_bits(0x3EE6_6666), 0.75],
            wheel_impact_divisor: 9.0,
            wheel_bucket_high: 0.5,
            wheel_bucket_low: f32::from_bits(0x3E9E_B852),
            body_impact_scale: 10.0,
            body_impact_speed_x: [
                0.0,
                f32::from_bits(0x3EBB_9F41),
                f32::from_bits(0x3EE5_50DE),
                f32::from_bits(0x3F05_6B91),
                f32::from_bits(0x3F17_C3F8),
                f32::from_bits(0x3F3D_B4F8),
                f32::from_bits(0x3F5C_FA27),
                f32::from_bits(0x3F72_3DB4),
            ],
            body_impact_speed_y: [
                1.0,
                1.2,
                f32::from_bits(0x3FBE_2BE0),
                f32::from_bits(0x3FF5_0753),
                f32::from_bits(0x401B_6DB5),
                3.6,
                f32::from_bits(0x4092_4921),
                5.0,
            ],
            tricks: {
                let mut tricks = vec![None; 332];
                tricks[128] = Some(AudioTrick {
                    id_164: 28,
                    id_172: 28,
                    flag_176: false,
                });
                tricks[96] = Some(AudioTrick {
                    id_164: 0,
                    id_172: u32::MAX,
                    flag_176: true,
                });
                tricks
            },
        }
    }

    fn inputs_with(flags: &[usize]) -> RetailAudioInputs {
        let mut inputs = RetailAudioInputs::default();
        for &offset in flags {
            inputs.state_flags[offset - 52] = true;
        }
        inputs
    }

    #[test]
    fn push_plant_edge_fires_once_per_plant_and_selects_the_foot() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut observe = |flags: &[usize]| {
            state.update(&inputs_with(flags), &tuning);
            (
                state.push_plant_335,
                state.push_left_333,
                state.push_right_334,
            )
        };
        // Left-toe push: event present, planted.
        assert_eq!(observe(&[56, 55]), (true, true, false));
        // Still planted: no second edge.
        assert_eq!(observe(&[56, 55]), (false, true, false));
        // Lifted, then a right-toe plant.
        assert_eq!(observe(&[]), (false, false, false));
        assert_eq!(observe(&[56, 57, 55]), (true, false, true));
    }

    #[test]
    fn material_clamp_maps_zero_and_out_of_range_to_none() {
        assert_eq!(material(0), 143);
        assert_eq!(material(1), 0);
        assert_eq!(material(144), 143);
        assert_eq!(material(145), 143);
        assert_eq!(material(u32::MAX), 143);
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.wheel_audio_surfaces = [4, 0, 16, 200];
        inputs.wheel_seam_patterns = [11, 0, 4, 8];
        inputs.part_audio_surfaces = [0, 3, 16];
        state.update(&inputs, &tuning);
        assert_eq!(state.wheel_material_620, [3, 143, 15, 143]);
        assert_eq!(state.wheel_seam_636, [11, 0, 4, 8]);
        assert_eq!(
            (
                state.front_truck_material_652,
                state.back_truck_material_656,
                state.deck_material_660
            ),
            (143, 2, 15)
        );
    }

    #[test]
    fn grind_latches_family_material_and_positive_impact() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        state.update(&inputs, &tuning);
        assert_eq!(state.grind_family_192, u32::MAX);
        assert_eq!(state.grind_material_692, 143);
        assert!(!state.grinding_341);

        inputs.grinding = true;
        inputs.grind_family = 3;
        inputs.grind_audio_surface = 16;
        inputs.grind_impact_speed = 2.5;
        inputs.state_category = 400;
        state.update(&inputs, &tuning);
        assert_eq!(state.grind_family_192, 3);
        assert_eq!(state.grind_material_692, 15);
        assert_eq!(state.grind_impact_228, 2.5);
        assert!(state.grinding_341 && !state.grinding_prev_342);

        // Grind ends: family/material/impact hold; 342 is the previous 341.
        inputs.grinding = false;
        inputs.grind_family = u32::MAX;
        inputs.grind_audio_surface = 0;
        inputs.grind_impact_speed = 0.0;
        inputs.state_category = 100;
        state.update(&inputs, &tuning);
        assert_eq!(state.grind_family_192, 3);
        assert_eq!(state.grind_material_692, 15);
        assert_eq!(state.grind_impact_228, 2.5);
        assert!(!state.grinding_341 && state.grinding_prev_342);

        // State 701 counts only with Grinds byte323.
        inputs.state = 701;
        state.update(&inputs, &tuning);
        assert!(!state.grinding_341);
        inputs.grind_flag_323 = true;
        state.update(&inputs, &tuning);
        assert!(state.grinding_341);
    }

    #[test]
    fn slip_uses_lateral_deck_speed_and_is_zero_without_wheels() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.linear_velocity = [-9.0, 0.0, 30.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.slip_232, 0.0);
        assert_eq!(state.conditioner().slip_rise_16, 0.0);

        inputs.wheel_count = 4;
        state.update(&inputs, &tuning);
        assert_eq!(state.slip_232, (9.0 + 0.75) / 45.0);
        assert_eq!(state.conditioner().slip_rise_16, 9.0 * 0.25);
        // Steady speed: the 4-sample average rises until the ring is full, then holds.
        for _ in 0..3 {
            state.update(&inputs, &tuning);
        }
        state.update(&inputs, &tuning);
        assert_eq!(state.conditioner().slip_rise_16, 0.0);
        inputs.linear_velocity = [100.0, 0.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.slip_232, 1.0);
    }

    #[test]
    fn deck_angular_velocity_projects_on_the_deck_part_rows() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        // Deck yawed 90 degrees: its X row is world -Z, its Z row is world X.
        inputs.deck_rows = [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
        inputs.angular_velocity = [2.0, 3.0, 5.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.deck_angular_velocity_480, [-5.0, 3.0, 2.0]);
    }

    #[test]
    fn wheel_landing_needs_six_air_frames_and_uses_the_previous_velocity() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.wheel_contact_normals[0] = [0.0, 1.0, 0.0];
        // Constructed as "in contact": the first frames count air time.
        for _ in 0..6 {
            inputs.wheel_velocities[0] = [0.0, -4.5, 0.0];
            state.update(&inputs, &tuning);
            assert!(!state.wheel_landed_464[0]);
        }
        // Touchdown after six air frames; the impact reads last frame's -4.5.
        inputs.wheel_contacts[0] = true;
        inputs.wheel_velocities[0] = [0.0, 0.0, 0.0];
        state.update(&inputs, &tuning);
        assert!(state.wheel_landed_464[0]);
        assert_eq!(state.conditioner().wheel_impact_52[0], 0.5);
        // The latch holds on the ground and through five air frames, clearing on the sixth
        // frame's check (air frame count > 5).
        state.update(&inputs, &tuning);
        assert!(state.wheel_landed_464[0]);
        inputs.wheel_contacts[0] = false;
        for _ in 0..6 {
            state.update(&inputs, &tuning);
            assert!(state.wheel_landed_464[0]);
        }
        state.update(&inputs, &tuning);
        assert!(!state.wheel_landed_464[0]);
        // A bounce after only a few air frames does not latch.
        let mut short = AudioState::default();
        inputs.wheel_contacts[0] = true;
        short.update(&inputs, &tuning);
        inputs.wheel_contacts[0] = false;
        short.update(&inputs, &tuning);
        inputs.wheel_contacts[0] = true;
        short.update(&inputs, &tuning);
        assert!(!short.wheel_landed_464[0]);
    }

    #[test]
    fn wheel_landing_bucket_uses_the_stored_air_factor() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.in_known_air = true;
        inputs.air_time_in_state = 0.8;
        state.update(&inputs, &tuning);
        // The bucket reads the factor stored before this pass (0), then stores 0.4.
        assert_eq!(state.wheel_landing_bucket_448, [0; 4]);
        assert_eq!(state.wheel_air_factor_244, [0.4; 4]);
        inputs.air_time_in_state = 1.4;
        state.update(&inputs, &tuning);
        assert_eq!(state.wheel_landing_bucket_448, [1; 4]);
        assert_eq!(state.wheel_air_factor_244[0], 0.7);
        // Landed: factor 0; without a latch or grind the buckets and factors hold.
        inputs.in_known_air = false;
        state.update(&inputs, &tuning);
        assert_eq!(state.wheel_landing_bucket_448, [1; 4]);
        assert_eq!(state.wheel_air_factor_244[0], 0.7);
        // Grinding releases them: bucket of 0.7 = 2, factor 0.
        inputs.state_category = 400;
        state.update(&inputs, &tuning);
        assert_eq!(state.wheel_landing_bucket_448, [2; 4]);
        assert_eq!(state.wheel_air_factor_244, [0.0; 4]);
    }

    #[test]
    fn landing_bucket_is_forced_to_one_outside_the_offboard_air_window() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.com_velocity = [0.0, -3.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.conditioner().landing_bucket_44, 3);
        assert_eq!(state.landing_bucket_300, 1);
        // Filtered state 7 sets +720 = 20, but +300 reads +720 before that store.
        inputs.filtered_state = 7;
        state.update(&inputs, &tuning);
        assert_eq!(state.offboard_air_countdown_720, 20);
        assert_eq!(state.landing_bucket_300, 1);
        inputs.filtered_state = 0;
        inputs.com_velocity = [0.0, -4.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.landing_bucket_300, 4);
        assert_eq!(state.offboard_air_countdown_720, 19);
        // A trick other than audio trick 31 forces 1.
        inputs.scorable_id = 128;
        state.update(&inputs, &tuning);
        assert_eq!(state.landing_bucket_300, 1);
        // The ring keeps the most negative of the last four samples.
        inputs.scorable_id = -1;
        inputs.com_velocity = [0.0, 1.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.conditioner().landing_bucket_44, 4);
        for _ in 0..3 {
            state.update(&inputs, &tuning);
        }
        assert_eq!(state.conditioner().landing_bucket_44, 1);
        // The countdown stops at 0.
        for _ in 0..30 {
            state.update(&inputs, &tuning);
        }
        assert_eq!(state.offboard_air_countdown_720, 0);
    }

    #[test]
    fn jump_bucket_uses_strictly_greater_thresholds() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        for (strength, bucket) in [
            (0.2, 0),
            (tuning.jump_thresholds[0], 0),
            (0.5, 1),
            (0.75, 1),
            (0.8, 2),
        ] {
            inputs.jump_strength = strength;
            state.update(&inputs, &tuning);
            assert_eq!(state.jump_bucket_304, bucket, "strength {strength}");
        }
    }

    #[test]
    fn hold_timer_expires_sooner_at_speed() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.dt = 0.125;
        state.update(&inputs, &tuning);
        inputs.offboard_309 = true;
        // Stationary: expires once more than 0.4 s has passed since the latch.
        let mut expired_at = None;
        for frame in 0..8 {
            state.update(&inputs, &tuning);
            assert!(state.hold_309 && state.hold_active_311);
            if state.hold_expired_310 && expired_at.is_none() {
                expired_at = Some(frame);
            }
        }
        assert_eq!(state.hold_start_316, 0.125);
        assert_eq!(expired_at, Some(4));
        // Release resets the latch; at 5 m/s the limit is (1 - 0.6) × 0.4.
        inputs.offboard_309 = false;
        state.update(&inputs, &tuning);
        assert!(!state.hold_active_311 && !state.hold_expired_310);
        inputs.offboard_309 = true;
        inputs.ground_speed = 5.0;
        let start = state.hold_clock_312;
        let mut frames = 0;
        loop {
            state.update(&inputs, &tuning);
            if state.hold_expired_310 {
                break;
            }
            frames += 1;
        }
        assert_eq!(state.hold_start_316, start);
        assert_eq!(frames, 2);
    }

    #[test]
    fn jump_velocity_updates_only_with_air_440() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.jump_velocity_delta = [0.0, 1.325, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.jump_velocity_468, 0.0);
        inputs.air_440 = true;
        state.update(&inputs, &tuning);
        assert!((state.jump_velocity_468 - 0.5).abs() < 1e-6);
        inputs.air_440 = false;
        inputs.jump_velocity_delta = [0.0; 3];
        state.update(&inputs, &tuning);
        assert!((state.jump_velocity_468 - 0.5).abs() < 1e-6);
        inputs.air_440 = true;
        inputs.jump_velocity_delta = [0.0, 10.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.jump_velocity_468, 1.0);
    }

    #[test]
    fn foot_surfaces_fall_back_to_one_and_prefer_the_footplant_surface() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        state.update(&inputs, &tuning);
        assert_eq!((state.foot_surface_736, state.foot_surface_738), (1, 1));
        assert_eq!((state.foot_material_728, state.foot_material_732), (0, 0));
        inputs.offboard_surface = 0x0001_B084;
        state.update(&inputs, &tuning);
        assert_eq!(state.foot_surface_736, 0xB084);
        assert_eq!((state.foot_material_728, state.foot_material_732), (3, 3));
        inputs.footplant_448 = true;
        inputs.footplant_surface = 0x83;
        state.update(&inputs, &tuning);
        assert!(state.footplant_768);
        assert_eq!(state.foot_surface_738, 0x83);
        assert_eq!(state.foot_material_728, 2);
        inputs.footplant_surface = 0x80;
        state.update(&inputs, &tuning);
        assert_eq!(state.foot_material_732, 143);
    }

    #[test]
    fn trick_ids_come_from_the_vault_record_of_the_scorable() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        state.update(&inputs, &tuning);
        assert!(!state.trick_active_343);
        assert_eq!(
            (state.audio_trick_348, state.audio_trick_352),
            (u32::MAX, u32::MAX)
        );
        inputs.scorable_id = 96;
        state.update(&inputs, &tuning);
        assert!(state.trick_active_343 && state.trick_flag_344);
        assert_eq!(
            (state.audio_trick_348, state.audio_trick_352),
            (0, u32::MAX)
        );
        // A scorable without an audio record is still an active trick.
        inputs.scorable_id = HIPPY_JUMP;
        state.update(&inputs, &tuning);
        assert!(state.trick_active_343 && !state.trick_flag_344 && state.hippy_jump_372);
        assert_eq!(state.audio_trick_348, u32::MAX);
    }

    #[test]
    fn category_and_known_air_derivations() {
        assert_eq!(category(503), 500);
        assert_eq!(category(599), 500);
        assert_eq!(category(600), 600);
        assert_eq!(category(99), 0);
        assert_eq!(category(100), 100);
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.state = 503;
        inputs.motion_200 = 0.25;
        inputs.com_velocity = [3.0, 0.0, 4.0];
        state.update(&inputs, &tuning);
        assert!(state.walking_716);
        assert_eq!(state.soft_wheels_684, 1);
        assert!((state.com_speed_212 - 5.0).abs() < 1e-5);
        assert_eq!(state.com_speed_216, state.com_speed_212);
        inputs.motion_200 = 0.5;
        state.update(&inputs, &tuning);
        assert_eq!(state.soft_wheels_684, 0);
    }

    #[test]
    fn foot_speeds_select_the_larger_magnitude() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.foot_local_velocity = [[-3.0, -1.0, 2.0], [1.0, 2.0, -4.0]];
        inputs.foot_world_velocity = [[0.5, 9.0, -0.25], [-6.0, 0.0, 6.5]];
        state.update(&inputs, &tuning);
        assert_eq!(state.toe_local_speed_y_272, 1.0);
        assert_eq!(state.toe_local_speed_y_268, 2.0);
        assert_eq!(state.toe_local_speed_xz_280, 3.0);
        assert_eq!(state.toe_local_speed_xz_276, 4.0);
        assert_eq!(state.foot_world_speed_xz_284, 0.5);
        assert_eq!(state.foot_world_speed_xz_288, 6.5);
    }

    #[test]
    fn deck_scrape_is_the_maximum_of_the_last_four_frames() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        for (scrape, expected) in [
            (0.5, 0.5),
            (0.25, 0.5),
            (0.0, 0.5),
            (0.0, 0.5),
            (0.0, 0.25),
            (0.0, 0.0),
        ] {
            inputs.deck_scrape = scrape;
            state.update(&inputs, &tuning);
            assert_eq!(state.deck_scrape_668, expected);
        }
    }

    #[test]
    fn loose_board_needs_bail_or_walking_deck_contact_and_a_low_deck_material() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = inputs_with(&[59]);
        inputs.deck_contact = true;
        inputs.part_audio_surfaces = [0, 0, 16];
        inputs.effective_deck_up = [0.0, -1.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.loose_board_780, 1);
        inputs.effective_deck_up = [1.0, 0.05, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.loose_board_780, 2);
        inputs.effective_deck_up = [0.0, 1.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.loose_board_780, 0);
        inputs.effective_deck_up = [0.0, -1.0, 0.0];
        inputs.part_audio_surfaces = [0, 0, 95];
        state.update(&inputs, &tuning);
        assert_eq!(state.loose_board_780, 0);
        inputs.part_audio_surfaces = [0, 0, 16];
        inputs.state_flags[59 - 52] = false;
        state.update(&inputs, &tuning);
        assert_eq!(state.loose_board_780, 0);
        inputs.state = 500;
        state.update(&inputs, &tuning);
        assert_eq!(state.loose_board_780, 1);
    }

    /// `SKATE_AUDIO_CAPTURE_STATE`, else the worktree's `.local/captures/extract/state.tsv`.
    fn capture_path() -> Option<std::path::PathBuf> {
        let path = std::env::var_os("SKATE_AUDIO_CAPTURE_STATE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/captures/extract/state.tsv")
            });
        path.exists().then_some(path)
    }

    #[test]
    fn capture_rows_decode_bytes_halves_and_words_big_endian() {
        let mut words = [0u32; CAPTURE_WORDS];
        words[(332 - 192) / 4] = 0x0100_0001; // +332 and +335
        words[(736 - 192) / 4] = 0xB084_0083; // +736 / +738
        words[(720 - 192) / 4] = 20;
        words[(208 - 192) / 4] = 2.5f32.to_bits();
        let state = AudioState::from_capture(&words);
        assert!(state.in_known_air_332 && state.push_plant_335);
        assert!(!state.push_left_333 && !state.push_right_334);
        assert_eq!(
            (state.foot_surface_736, state.foot_surface_738),
            (0xB084, 0x83)
        );
        assert_eq!(state.offboard_air_countdown_720, 20);
        assert_eq!(state.ground_speed_208, 2.5);
        let mut words = [0u32; CAPTURE_WORDS];
        words[(740 - 192) / 4] = 4;
        words[(592 - 192) / 4] = 0x0001_0000; // +593 only
        words[(604 - 192) / 4] = u32::MAX;
        words[(560 - 192) / 4 + 7] = 16;
        let state = AudioState::from_capture(&words);
        assert_eq!(state.step_code_740, 4);
        assert!(!state.groin_contact_592 && state.face_contact_593);
        assert_eq!(state.other_skater_604, -1);
        assert_eq!(state.body_material_560[7], 16);
    }

    /// Replays the bridge steps whose inputs are all in the audio state against consecutive
    /// retail capture rows (skipped without the capture).
    #[test]
    fn bridge_steps_reproduce_the_retail_capture() {
        let Some(path) = capture_path() else {
            return;
        };
        let rows = load_capture_states(&path).unwrap();
        assert!(rows.len() > 1000);
        let tuning = tuning();
        let bits = |values: [f32; 4]| values.map(f32::to_bits);
        for pair in rows.windows(2) {
            let (previous, current) = (&pair[0].1, &pair[1].1);
            assert_eq!(current.grinding_prev_342, previous.grinding_341);
            assert_eq!(
                current.com_speed_216.to_bits(),
                current.com_speed_212.to_bits()
            );
            assert!(current.wheel_material_620.iter().all(|&m| m <= 143));
            assert!((1..=5).contains(&current.step_code_740));
            // +496..+524 are clamped conditioner impacts (0, or 0.001..1) times the holder +36
            // graph at the previous +212.
            let scale = point_graph(
                previous.com_speed_212,
                &tuning.body_impact_speed_x,
                &tuning.body_impact_speed_y,
            );
            for impact in current.body_impact_496 {
                let raw = impact / scale;
                assert!(
                    impact == 0.0 || (raw >= 0.000_999 && raw <= 1.000_001),
                    "{raw}"
                );
            }

            let mut state = previous.clone();
            state.in_known_air_332 = current.in_known_air_332;
            state.air_time_236 = current.air_time_236;
            state.wheel_landed_464 = current.wheel_landed_464;
            state.grinding_341 = current.grinding_341;
            state.bridge_wheel_landing(&tuning);
            assert_eq!(
                bits(state.wheel_air_factor_244),
                bits(current.wheel_air_factor_244)
            );
            assert_eq!(
                state.wheel_landing_bucket_448,
                current.wheel_landing_bucket_448
            );

            // When the forcing rule applies, the capture must hold 1 whatever the ring said.
            let mut state = previous.clone();
            state.trick_active_343 = current.trick_active_343;
            state.audio_trick_348 = current.audio_trick_348;
            state.bridge_landing_bucket(4);
            if state.landing_bucket_300 == 1 {
                assert_eq!(current.landing_bucket_300, 1);
            }

            let mut state = previous.clone();
            state.ground_speed_208 = current.ground_speed_208;
            state.bridge_hold(
                current.board_held_308,
                current.hold_309,
                current.hold_clock_312 - previous.hold_clock_312,
            );
            assert_eq!(
                (
                    state.hold_expired_310,
                    state.hold_active_311,
                    state.hold_start_316.to_bits()
                ),
                (
                    current.hold_expired_310,
                    current.hold_active_311,
                    current.hold_start_316.to_bits()
                )
            );

            let mut state = previous.clone();
            state.bridge_offboard_air(current.offboard_air_718);
            assert_eq!(
                state.offboard_air_countdown_720,
                current.offboard_air_countdown_720
            );

            let mut state = previous.clone();
            state.bridge_foot_surfaces(current.foot_surface_736, current.footplant_768);
            assert_eq!(
                (state.foot_material_728, state.foot_material_732),
                (current.foot_material_728, current.foot_material_732)
            );
        }
    }

    #[test]
    fn point_graph_clamps_and_interpolates_like_sub_82481e10() {
        let xs = [0.0, 1.0, 2.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let ys = [1.0, 2.0, 4.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        assert_eq!(point_graph(-1.0, &xs, &ys), 1.0);
        assert_eq!(point_graph(0.5, &xs, &ys), 1.5);
        assert_eq!(point_graph(6.0, &xs, &ys), 12.0);
        assert_eq!(point_graph(9.0, &xs, &ys), 12.0);
        // x = 2 is at or past the shared pair; the next bracket is (2, 3).
        assert_eq!(point_graph(2.5, &xs, &ys), 8.5);
    }

    #[test]
    fn ragdoll_spins_and_limb_speed_follow_the_builder() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.ragdoll_spin = [[3.0, 0.0, 4.0], [1.0, -2.5, 0.0], [0.0, -7.0, 9.0]];
        inputs.limb_speeds = [1.0, 2.0, 3.0, 6.0];
        state.update(&inputs, &tuning);
        assert!((state.ragdoll_spin_328 - 5.0).abs() < 1e-5);
        assert_eq!(state.ragdoll_spin_y_296, 2.5);
        assert_eq!(state.ragdoll_spin_y_292, 7.0);
        assert_eq!(state.limb_speed_672, 3.0);
    }

    #[test]
    fn step_code_compares_the_plant_heights_of_the_planting_foot() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        state.update(&inputs, &tuning);
        assert_eq!(state.step_code_740, 1);
        // Left plant at y 0, then a right plant 0.1 higher: step up (212), surface not 8 → 4.
        inputs.toe_positions = [[0.0, 0.0, 0.0], [0.0, 0.1, 0.0]];
        inputs.offboard_feet = [true, false];
        state.update(&inputs, &tuning);
        assert!(state.conditioner().left_plant_214);
        // Only the left foot is down and the right plant (0.0 at construction) is not above
        // the threshold: nothing.
        assert_eq!(state.step_code_740, 1);
        inputs.offboard_feet = [true, true];
        state.update(&inputs, &tuning);
        assert!(state.conditioner().right_plant_215 && !state.conditioner().left_plant_214);
        assert_eq!(state.step_code_740, 4);
        // Physical surface category 8 (bits 7..11) turns 4 into 2.
        inputs.offboard_surface = 8 << 7;
        state.update(&inputs, &tuning);
        assert_eq!(state.step_code_740, 2);
        // A zero surface keeps the last nonzero one.
        inputs.offboard_surface = 0;
        state.update(&inputs, &tuning);
        assert_eq!(state.step_code_740, 2);
        // Right foot lifted: the left foot decides, rise > 0 → 213 → 3 on surface 8.
        inputs.offboard_feet = [true, false];
        state.update(&inputs, &tuning);
        assert_eq!(state.step_code_740, 3);
        // Neither foot down: no step this frame.
        inputs.offboard_feet = [false, false];
        state.update(&inputs, &tuning);
        assert_eq!(state.step_code_740, 1);
        // Within 0.07: no step.
        inputs.toe_positions = [[0.0, 0.0, 0.0], [0.0, 0.05, 0.0]];
        inputs.offboard_feet = [false, true];
        state.update(&inputs, &tuning);
        assert!(state.conditioner().right_plant_215);
        assert_eq!(state.step_code_740, 1);
    }

    #[test]
    fn body_contacts_clamp_hold_four_frames_and_scale_by_the_previous_speed() {
        let tuning = tuning();
        let mut state = AudioState::default();
        let mut inputs = RetailAudioInputs::default();
        inputs.body_contacts.contact[0] = true;
        inputs.body_contacts.weighted_change[0] = 0.05;
        inputs.body_contacts.contact[1] = true;
        inputs.body_contacts.weighted_change[1] = 0.0;
        inputs.body_contacts.contact[2] = true;
        inputs.body_contacts.weighted_change[2] = 7.0;
        inputs.body_contacts.slide[0] = 2.5;
        inputs.body_contacts.material[0] = 4;
        inputs.body_contacts.specific_current = [false, true];
        state.update(&inputs, &tuning);
        // Previous +212 is 0: graph y0 = 1.
        assert_eq!(
            state.body_impact_496[..4],
            [0.5, BODY_IMPACT_FLOOR, 1.0, 0.0]
        );
        assert_eq!(
            (state.body_slide_528[0], state.body_material_560[0]),
            (2.5, 4)
        );
        assert!(!state.groin_contact_592 && state.face_contact_593);
        assert_eq!(state.other_skater_604, -1);
        // Contact ends and the COM speed rises past the last graph x: held three more
        // conditioner frames, scaled by 5 from the next pass on.
        inputs.body_contacts = crate::skate_audio::BodyContacts {
            other_skater: -1,
            ..Default::default()
        };
        inputs.com_velocity = [2.0, 0.0, 0.0];
        state.update(&inputs, &tuning);
        assert_eq!(state.body_impact_496[0], 0.5);
        assert_eq!(state.body_slide_528[0], 0.0);
        assert_eq!(state.body_material_560[0], 0);
        state.update(&inputs, &tuning);
        assert_eq!(state.body_impact_496[0], 2.5);
        state.update(&inputs, &tuning);
        assert_eq!(state.body_impact_496[2], 5.0);
        state.update(&inputs, &tuning);
        assert_eq!(state.body_impact_496, [0.0; 8]);
    }

    /// The retail air words the Treatment packet streams: within one hop +236 rises by the fixed
    /// step and +236 + +240 (the predicted total air time) stays put, +260 holds the jump height,
    /// and +240 is never the PhysicsAir constant 4.0. This is the behaviour the engine-side
    /// `air_timing` publication reproduces for ordinary airs.
    #[test]
    fn capture_air_words_hold_a_constant_total_and_a_plateau() {
        let Some(path) = capture_path() else {
            return;
        };
        let rows = load_capture_states(&path).unwrap();
        let mut air_rows = 0;
        let mut rising = 0;
        let mut sum_changed = 0;
        let mut height_dropped = 0;
        for pair in rows.windows(2) {
            let (previous, current) = (&pair[0].1, &pair[1].1);
            if !current.in_known_air_332 {
                continue;
            }
            air_rows += 1;
            assert_ne!(current.air_until_landing_240, 4.0);
            let step = current.air_time_236 - previous.air_time_236;
            if !previous.in_known_air_332 || !(step > 0.0 && step <= 0.04) {
                continue;
            }
            rising += 1;
            let total = current.air_time_236 + current.air_until_landing_240;
            let previous_total = previous.air_time_236 + previous.air_until_landing_240;
            if (total - previous_total).abs() > 1e-4 {
                sum_changed += 1;
            }
            if current.air_jump_height_260 < previous.air_jump_height_260 - 1e-6 {
                height_dropped += 1;
            }
        }
        assert!(air_rows > 1000, "{air_rows}");
        assert!(rising > 1000, "{rising}");
        // Both only move when the predictor re-plans inside a hop.
        assert!(sum_changed * 50 < rising, "{sum_changed} of {rising}");
        assert!(height_dropped * 50 < rising, "{height_dropped} of {rising}");
    }
}
