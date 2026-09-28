//! Publish one immutable player/board audio observation for every completed physics tick.
//!
//! The observation has no sound-selection policy. In particular, the physics wheel-surface vote
//! is carried as raw evidence until it is matched to the retail audio-material path.

use bevy::prelude::*;
use skate_core::math::Vector3;
use skate_core::physics::{board::BodyId, board_ground::BoardGroundState, board_motion_output};

use super::{GamePhysics, SkaterRuntime};

#[derive(Resource)]
pub(crate) struct AudioObservationCursor {
    published_tick: Option<u64>,
    last_surface: Option<u32>,
    last_powersliding: bool,
    last_trick_id: Option<usize>,
    last_state: Option<u32>,
    last_board_vertical_speed: f32,
    last_rider_vertical_speed: f32,
    trace: bool,
}

impl Default for AudioObservationCursor {
    fn default() -> Self {
        Self {
            published_tick: None,
            last_surface: None,
            last_powersliding: false,
            last_trick_id: None,
            last_state: None,
            last_board_vertical_speed: 0.0,
            last_rider_vertical_speed: 0.0,
            trace: std::env::var_os("SKATE_AUDIO_OBSERVE").is_some(),
        }
    }
}

fn speed(value: Vector3) -> f32 {
    value
        .x
        .mul_add(value.x, value.y.mul_add(value.y, value.z * value.z))
        .sqrt()
}

fn raw_vector(raw: [u32; 4]) -> [f32; 3] {
    [
        f32::from_bits(raw[0]),
        f32::from_bits(raw[1]),
        f32::from_bits(raw[2]),
    ]
}

fn triple(raw: [u32; 4]) -> [f32; 3] {
    raw_vector(raw)
}

fn lanes(value: Vector3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

/// `0x822F8C8C`, the deck scrape scale in `82C02A80`.
const DECK_SCRAPE_SCALE: f32 = f32::from_bits(0x3AA3_D70A);

/// PowerPC `fsel`: `a >= 0 ? b : c`, NaN selecting `c`.
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// Skateboard::FillPhysOut `82C02A80` Collision+20 (loc_82C034D8..loc_82C0356C): with deck
/// contact (CollisionInfo+850), the length of the deck's contact relative velocity
/// (CollisionInfo+640) after removing its component along the deck contact normal
/// (CollisionInfo+192); 0 without deck contact.
fn deck_slide_speed(ground: &BoardGroundState) -> f32 {
    let deck = ground.parts[BodyId::Deck.index()];
    if !deck.in_contact {
        return 0.0;
    }
    let normal = deck.normal;
    let velocity = deck.relative_velocity;
    let along = board_motion_output::dot(normal, velocity);
    board_motion_output::length(Vector3::new(
        velocity.x - normal.x * along,
        velocity.y - normal.y * along,
        velocity.z - normal.z * along,
    ))
}

/// Skateboard::FillPhysOut `82C02A80` Collision+24 (loc_82C03498..loc_82C034D8): with deck
/// contact, clamp(|deck acceleration (CollisionInfo+528) · deck normal| × 0.00125, 0, 1).
fn deck_scrape(ground: &BoardGroundState) -> f32 {
    let deck = ground.parts[BodyId::Deck.index()];
    if !deck.in_contact {
        return 0.0;
    }
    let scaled = board_motion_output::dot(ground.accelerations[BodyId::Deck.index()], deck.normal)
        .abs()
        * DECK_SCRAPE_SCALE;
    let positive = fsel(-scaled, 0.0, scaled);
    fsel(1.0 - positive, positive, 1.0)
}

/// Air byte438, ProcessOutput `sub_82DB6EC0` (loc_82DB7620..764C): State+16 in 200..300
/// (signed) and no wheel contact (Collision+0, already written by `82C02A80` earlier in the same
/// ProcessOutput).
fn in_known_air(state: u32, wheel_count: u32) -> bool {
    (200..300).contains(&(state as i32)) && wheel_count == 0
}

/// PhysOutScoring2+152, `sub_82DAC498`: the EScorableID of the score packet's trick name
/// (`sub_82DA5AC8`, a lookup the engine performs with `ScoringData::by_name`), only while the
/// packet flags carry bit 24 or 25; otherwise, or when the name is not scorable, -1.
/// ScoringTrick (`8258FA20`) is the writer that sets bit 24 together with this name slot.
fn scorable_id(flags: u32, trick: Option<i32>) -> i32 {
    if flags & 0x0300_0000 == 0 {
        return -1;
    }
    trick.unwrap_or(-1)
}

/// Skeleton Fill `sub_82BE1AE8` Skeleton+560/+564/+568/+572: |physical-record velocity of
/// parts 17, 21, 4 and 8 − the COM velocity| (vsubfp, vmsum3fp, rsqrt with two refinements,
/// zero selection).
fn limb_speeds(velocities: &[[f32; 4]], com_velocity: [f32; 3]) -> [f32; 4] {
    [17, 21, 4, 8].map(|part| {
        let v = velocities[part];
        board_motion_output::length(Vector3::new(
            v[0] - com_velocity[0],
            v[1] - com_velocity[1],
            v[2] - com_velocity[2],
        ))
    })
}

/// The per-region part of the ragdoll contact publisher `sub_82BD60C8` (loop 82BD68F0) and its
/// direct copies (Collision+176..+192), from the engine's SkeletonCollision owner.
fn body_contacts(
    feedback: &skate_core::physics::skeleton_body::SkeletonCollisionFeedback,
    velocity_changes: &[[f32; 4]],
    part_weights: &[f32; 24],
) -> crate::skate_audio::BodyContacts {
    let mut contacts = crate::skate_audio::BodyContacts {
        specific_current: [feedback.specific[0].current, feedback.specific[1].current],
        group_8_force: feedback.maximum_group_8_force,
        skater_force: feedback.maximum_skater_force,
        other_skater: feedback.other_skater,
        group_11_force: feedback.maximum_group_11_force,
        ..Default::default()
    };
    for (i, region) in feedback.regions.iter().enumerate() {
        let Some(part) = region.part else {
            continue;
        };
        let change = velocity_changes[part];
        let normal = region.normal;
        let along = (change[0] * normal[0] + change[1] * normal[1] + change[2] * normal[2]).abs();
        contacts.contact[i] = true;
        contacts.weighted_change[i] = along * part_weights[part];
        contacts.slide[i] = region.tangent_speed;
        contacts.material[i] = region.material_flags;
    }
    contacts
}

/// What each airborne state owns for Air+176 / Air+184.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AirTimingInputs {
    /// State+16.
    state: u32,
    /// Air+176 and Air+184 as the PhysOut record carries them.
    published_176: f32,
    published_184: f32,
    /// PhysicsAir+160, its own time in state.
    physics_air_elapsed: f32,
    /// The trajectory selector's current prediction time (`QueryResult+48`), `None` when there is
    /// no selection and negative when that trajectory hits nothing.
    predicted_contact_time: Option<f32>,
    /// BipedAir+448 / +444.
    biped_duration: f32,
    biped_remaining: f32,
}

/// Air+176 (time in the air state) and Air+184 (time until the predicted landing) for the state
/// the skater is actually in.
///
/// Only KnownAir Fill `sub_82D36880` writes both natively: +176 = KnownAir+180, its own timer,
/// and +184 = KnownAir+196 (the selected prediction's collision time, `QueryResult+48`) minus
/// that timer. `PhysState_PhysicsAir::FillPhysOut` `sub_82D34E90` writes +184 = 4.0 while the
/// selector latch (PhysicsAir+174) is set and never writes +176, and the BipedAir fill
/// `sub_82D30808` writes neither. Retail reaches KnownAir whenever a trajectory is valid, but the
/// engine publishes that flag (Processed2468 bit 10) only briefly, so ordinary ollies run in
/// PhysicsAir: the audio state then saw +236 = 0 and +240 = 4.0 and the Treatment packet's air
/// words stopped moving. Each branch below reads the same quantity from the native fields of the
/// state that owns it.
fn air_timing(inputs: AirTimingInputs) -> (f32, f32) {
    match inputs.state {
        201 => (inputs.published_176, inputs.published_184),
        // PhysicsAir+160 is the state timer; PhysicsAir keeps no landing time, so the KnownAir
        // formula runs over the trajectory selector's current prediction. A trajectory that hits
        // nothing leaves `contact_time` negative, where the state's own +184 write stands.
        200 | 202 => {
            let elapsed = inputs.physics_air_elapsed;
            let remaining = inputs
                .predicted_contact_time
                .filter(|time| *time >= 0.0)
                .map_or(inputs.published_184, |time| time - elapsed);
            (elapsed, remaining)
        }
        // BipedAir+448 is the sampled trajectory's duration and +444 its remaining time.
        501 => (
            inputs.biped_duration - inputs.biped_remaining,
            inputs.biped_remaining,
        ),
        _ => (inputs.published_176, inputs.published_184),
    }
}

/// The native PhysOut fields `sub_827A1B78` packs for the audio-state bridge `sub_824B0DA8`
/// and the PhysOut audio conditioner `sub_82772748`, read from the engine's published native
/// records.
fn retail_inputs(
    physics: &GamePhysics,
    skater: &SkaterRuntime,
    camera: &crate::camera::CameraRuntime,
) -> crate::skate_audio::RetailAudioInputs {
    let physical = &skater.player_input.physical;
    let processed = &skater.player_input.processed;
    let riding = &physics.riding;
    let ground = &riding.ground;
    let bodies = physics.board.bodies();
    let deck = physics.board.part_transforms()[BodyId::Deck.index()];
    let feet = &skater.foot_physical.output;
    let skeleton = &skater.skeleton;
    let air_time = air_timing(AirTimingInputs {
        state: physical.state.state_16,
        published_176: physical.air.time_in_state_176,
        published_184: physical.air.scalar_184,
        physics_air_elapsed: skater.air_state.time_in_state,
        predicted_contact_time: skater
            .trajectory
            .selector
            .selection()
            .map(|selection| selection.prediction.result.contact_time),
        biped_duration: skater.biped_air.state.duration_448,
        biped_remaining: skater.biped_air.state.time_remaining_444,
    });
    let score = &skater.animation.motion.score_packet;
    let trick = score
        .trick_names
        .first
        .and_then(|name| skater.scoring.data.by_name(name))
        .map(|definition| definition.metadata.id as i32);
    crate::skate_audio::RetailAudioInputs {
        dt: processed.timestep_2604,
        ground_speed: physical.skateboard.scalar_164,
        com_velocity: raw_vector(physical.reckoning.vector_16),
        position: raw_vector(physical.reckoning.vector_64),
        wheel_count: physical.collision.wheel_count_0 & 7,
        air_time_in_state: air_time.0,
        air_time_until_landing: air_time.1,
        air_jump_height: physical.air.jump_height_200,
        state_flags: skater.player_state.state_flags,
        offboard_feet: physical.off_board.flags_306_307.map(|flag| flag != 0),
        footplant: [
            physical.air.footplant_left_449 != 0,
            physical.air.footplant_right_450 != 0,
        ],
        footstep_strength: skater.animation_input.extra.footstep_strength,
        state_category: physical.state.category_12,
        state: physical.state.state_16,
        filtered_state: physical.filtered_state_0,
        in_known_air: in_known_air(physical.state.state_16, physical.collision.wheel_count_0),
        grinding: physical.grinds.grinding_316 != 0,
        grind_family: physical.grinds.words_136_140[0],
        grind_audio_surface: physical.grinds.audio_surface_216,
        grind_impact_speed: physical.grinds.impact_speed_128,
        grind_flag_323: physical.grinds.flag_323 != 0,
        air_440: processed.flags_2468 & (1 << 22) != 0,
        jump_velocity_delta: triple(physical.air.jump_velocity_delta_112),
        footplant_448: physical.air.flag_448 != 0,
        footplant_surface: physical.air.footplant_surface_224,
        offboard_surface: processed.left_surface_2596,
        offboard_309: processed.flags_2480 & (1 << 18) != 0,
        offboard_310: processed.flags_2480 & (1 << 8) != 0,
        offboard_311: physical.off_board.flag_311 != 0,
        skeleton_599: physical.skeleton.over_599 != 0,
        feet_in_deck_box: [
            physical.skeleton.flag_600 != 0,
            physical.skeleton.flag_601 != 0,
        ],
        foot_local_velocity: feet.local_velocity.map(|v| [v[0], v[1], v[2]]),
        foot_world_velocity: feet.world_velocity.map(|v| [v[0], v[1], v[2]]),
        wheel_contacts: physical
            .collision
            .wheel_contact_3296_3299
            .map(|flag| flag != 0),
        truck_contacts: [
            ground.parts[BodyId::FrontTruck.index()].in_contact,
            ground.parts[BodyId::BackTruck.index()].in_contact,
        ],
        deck_contact: physical.collision.flag_3475 != 0,
        wheel_contact_normals: std::array::from_fn(|i| {
            if ground.parts[i].in_contact {
                lanes(ground.parts[i].normal)
            } else {
                [0.0; 3]
            }
        }),
        wheel_audio_surfaces: riding.wheel_lines.audio_surfaces,
        wheel_seam_patterns: riding.wheel_lines.seam_patterns,
        part_audio_surfaces: ground.part_audio_surfaces,
        deck_slide_speed: deck_slide_speed(ground),
        deck_scrape: deck_scrape(ground),
        angular_velocity: triple(physical.skateboard.vector_64),
        linear_velocity: triple(physical.skateboard.vector_80),
        deck_tilt: skater.ground.steering.deck_tilt,
        motion_200: processed.scalar_2764,
        wheel_velocities: std::array::from_fn(|i| lanes(bodies[i].rates.linear_velocity)),
        deck_rows: deck.basis.columns,
        effective_deck_up: riding.motion.effective_basis.columns[1],
        wheel_positions: std::array::from_fn(|i| lanes(bodies[i].rates.position)),
        // Native camera basis columns are right, up, at: the world matrix's row 2 is At.
        camera: camera.frame.as_ref().map(|frame| {
            (
                frame.basis.columns[2],
                [frame.position[0], frame.position[1], frame.position[2]],
            )
        }),
        deck_position: lanes(bodies[BodyId::Deck.index()].rates.position),
        deck_forward: riding.motion.effective_basis.columns[2],
        combo_multiplier: skater.scoring.session.combo.multiplier,
        // The simulation does not run while paused, so the worker's own forwarder repeats the
        // last observation with this set (see `player_audio::forward`).
        paused: false,
        ground_normal: triple(physical.ground.vector_80),
        turn: skater.animation_input.fields.turn,
        jump_strength: skater.animation_input.extra.jump_strength,
        scorable_id: scorable_id(score.flags, trick),
        ragdoll_spin: [23, 20, 16]
            .map(|part| lanes(skeleton.bodies()[part].rates.angular_velocity)),
        limb_speeds: limb_speeds(
            &skeleton.record.velocities,
            triple(physical.reckoning.vector_16),
        ),
        toe_positions: [15, 19].map(|part| {
            let position = skeleton.record.pose[part][3];
            [position[0], position[1], position[2]]
        }),
        body_contacts: body_contacts(
            &skater.collision_feedback,
            &skeleton.record.velocity_changes,
            &skeleton.definition.animation_masses.part_weights,
        ),
        pump_absorption: skater.ground.pumping.absorption,
    }
}

fn landing_state_edge(previous: Option<u32>, current: u32) -> bool {
    matches!(previous, Some(103 | 200 | 201 | 202)) && current == 100
}

/// Copy the completed snapshot only once, even if presentation runs more than once between fixed
/// simulation ticks.
pub(crate) fn publish(
    physics: Res<GamePhysics>,
    skater: Res<SkaterRuntime>,
    camera: Res<crate::camera::CameraRuntime>,
    mut cursor: ResMut<AudioObservationCursor>,
    mut observations: MessageWriter<crate::skate_audio::PlayerAudioObservation>,
) {
    let Some(output) = physics.exchange.output() else {
        return;
    };
    if cursor.published_tick == Some(output.tick) {
        return;
    }
    cursor.published_tick = Some(output.tick);
    let wheel_surface = super::ground_runtime::active_audio_surface(&physics.riding);
    let surface_changed = cursor.last_surface != Some(wheel_surface);
    cursor.last_surface = Some(wheel_surface);
    let trick = skater
        .animation
        .motion
        .score_packet
        .trick_names
        .first
        .and_then(|name| skater.scoring.data.by_name(name));
    let grind = &skater.player_input.physical.grinds;
    let state = output.state as u32;
    let landed = output.landed || landing_state_edge(cursor.last_state, state);
    cursor.last_state = Some(state);
    let landing_impact_speed = if landed {
        (-cursor.last_board_vertical_speed)
            .max(-cursor.last_rider_vertical_speed)
            .max(-output.board_linear_velocity.y)
            .max(-output.rider_linear_velocity.y)
            .max(0.0)
    } else {
        0.0
    };
    cursor.last_board_vertical_speed = output.board_linear_velocity.y;
    cursor.last_rider_vertical_speed = output.rider_linear_velocity.y;
    let foot_surface = if output.state as u32 / 100 == 5 {
        let left = skater.player_input.processed.left_surface_2596 & 0x7f;
        let right = skater.player_input.processed.right_surface_2600 & 0x7f;
        if left != 0 { left } else { right }
    } else {
        wheel_surface
    };
    let landing_quality = skater.landing_quality;
    let (landing_clean, landing_sketchy) = skater.scoring.classify_landing_audio(landing_quality);
    let observation = crate::skate_audio::PlayerAudioObservation {
        tick: output.tick,
        state,
        board_speed: speed(output.board_linear_velocity),
        rider_speed: speed(output.rider_linear_velocity),
        grounded: output.grounded,
        grinding: output.state.is_grind(),
        grind_family: grind.words_136_140[0],
        grind_substate: grind.words_136_140[1],
        grind_audio_surface: grind.audio_surface_216,
        grind_impact_speed: grind.impact_speed_128,
        wiping_out: output.wiping_out,
        landed,
        landing_impact_speed,
        landing_clean,
        landing_sketchy,
        landing_type: landing_quality.landing_type_96,
        landing_spin: landing_quality.spin_92,
        landing_sideways_speed: landing_quality.sideways_speed_84,
        footstep_strength: output.footstep_strength,
        footstep_bone: output.footstep_bone,
        foot_push_speed: output.foot_push_speed,
        feet_supported: output.feet_supported,
        foot_surface,
        contact_count: output.contact_count,
        powersliding: skater.animation.motion.is_power_sliding,
        trick_id: trick.map(|definition| definition.metadata.id),
        trick_identifier: trick.map(|definition| definition.identifier.to_owned()),
        animation_name: skater.animation.motion.animation.current_name.clone(),
        riding_switch: skater.animation.packet.riding_switch,
        riding_fakie: skater.animation.packet.riding_fakie,
        nollie: skater.animation.packet.weight_forwards,
        wheel_surface,
        events: output.events.clone(),
        // The forwarder attaches these; physics does not produce front-end sounds.
        frontend_sounds: Vec::new(),
        retail: retail_inputs(&physics, &skater, &camera),
    };
    let powerslide_changed = cursor.last_powersliding != observation.powersliding;
    let trick_changed = cursor.last_trick_id != observation.trick_id;
    cursor.last_powersliding = observation.powersliding;
    cursor.last_trick_id = observation.trick_id;
    if cursor.trace
        && (surface_changed
            || powerslide_changed
            || trick_changed
            || observation.footstep_strength != 0.0
            || observation.foot_push_speed != 0.0
            || observation.landed
            || !observation.events.is_empty())
    {
        eprintln!(
            "skate-audio-observe tick={} state={} board_speed={:.3} rider_speed={:.3} grounded={} wiping_out={} powersliding={} contacts={} wheel_surface={} foot_surface={} grind={}/{}/{} grind_impact={:.3} landing_impact={:.3} landing_quality={}/{}/{} spin={:.3} side={:.3} trick={:?}/{:?} animation={:?} switch={} fakie={} nollie={} footstep={} foot_bone={} push={} events={:?}",
            observation.tick,
            observation.state,
            observation.board_speed,
            observation.rider_speed,
            observation.grounded,
            observation.wiping_out,
            observation.powersliding,
            observation.contact_count,
            observation.wheel_surface,
            observation.foot_surface,
            observation.grind_family,
            observation.grind_substate,
            observation.grind_audio_surface,
            observation.grind_impact_speed,
            observation.landing_impact_speed,
            observation.landing_type,
            observation.landing_clean,
            observation.landing_sketchy,
            observation.landing_spin,
            observation.landing_sideways_speed,
            observation.trick_id,
            observation.trick_identifier,
            observation.animation_name,
            observation.riding_switch,
            observation.riding_fakie,
            observation.nollie,
            observation.footstep_strength,
            observation.footstep_bone,
            observation.foot_push_speed,
            observation.events,
        );
    }
    observations.write(observation);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_uses_all_three_physics_axes() {
        assert_eq!(speed(Vector3::new(2.0, 3.0, 6.0)), 7.0);
    }

    #[test]
    fn air_to_ground_edge_is_an_audio_landing_even_without_a_physics_event() {
        assert!(landing_state_edge(Some(201), 100));
        assert!(landing_state_edge(Some(200), 100));
        assert!(!landing_state_edge(Some(101), 100));
        assert!(!landing_state_edge(Some(201), 403));
    }

    #[test]
    fn deck_slide_and_scrape_need_deck_contact() {
        let mut ground = BoardGroundState::default();
        let deck = BodyId::Deck.index();
        ground.parts[deck].normal = Vector3::new(0.0, 1.0, 0.0);
        ground.parts[deck].relative_velocity = Vector3::new(3.0, -7.0, 4.0);
        ground.accelerations[deck] = Vector3::new(1.0, -400.0, 2.0);
        assert_eq!(deck_slide_speed(&ground), 0.0);
        assert_eq!(deck_scrape(&ground), 0.0);
        ground.parts[deck].in_contact = true;
        // Only the tangential part (3, 0, 4) counts.
        assert!((deck_slide_speed(&ground) - 5.0).abs() < 1e-5);
        assert_eq!(deck_scrape(&ground), 400.0 * DECK_SCRAPE_SCALE);
        ground.accelerations[deck] = Vector3::new(0.0, 1000.0, 0.0);
        assert_eq!(deck_scrape(&ground), 1.0);
    }

    #[test]
    fn known_air_is_an_air_state_without_wheel_contact() {
        assert!(in_known_air(200, 0));
        assert!(in_known_air(299, 0));
        assert!(!in_known_air(201, 1));
        assert!(!in_known_air(300, 0));
        assert!(!in_known_air(103, 0));
    }

    #[test]
    fn scorable_id_needs_packet_flag_bit_24_or_25() {
        assert_eq!(scorable_id(0, Some(96)), -1);
        assert_eq!(scorable_id(0x0100_0000, Some(96)), 96);
        assert_eq!(scorable_id(0x0200_0000, Some(96)), 96);
        assert_eq!(scorable_id(0x0100_0000, None), -1);
    }

    #[test]
    fn limb_speeds_are_relative_to_the_com_in_native_part_order() {
        let mut velocities = [[0.0f32; 4]; 26];
        velocities[17] = [4.0, 1.0, 3.0, 0.0];
        velocities[21] = [1.0, 1.0, 1.0, 0.0];
        velocities[4] = [1.0, -3.0, 1.0, 0.0];
        velocities[8] = [1.0, 1.0, 1.0, 9.0];
        let speeds = limb_speeds(&velocities, [1.0, 1.0, 1.0]);
        assert!((speeds[0] - 3.605_551).abs() < 1e-5);
        assert_eq!(speeds[1], 0.0);
        assert!((speeds[2] - 4.0).abs() < 1e-5);
        assert_eq!(speeds[3], 0.0);
    }

    #[test]
    fn air_timing_uses_the_owning_state_and_leaves_known_air_alone() {
        let inputs = AirTimingInputs {
            published_176: 0.25,
            published_184: 4.0,
            physics_air_elapsed: 0.5,
            biped_duration: 0.8,
            biped_remaining: 0.3,
            ..Default::default()
        };
        // KnownAir keeps the pair it published itself.
        assert_eq!(
            air_timing(AirTimingInputs {
                state: 201,
                ..inputs
            }),
            (0.25, 4.0)
        );
        // PhysicsAir with no prediction: only the timer changes.
        assert_eq!(
            air_timing(AirTimingInputs {
                state: 200,
                ..inputs
            }),
            (0.5, 4.0)
        );
        // A landing predicted 1.1 s after launch leaves 0.6 s from 0.5 s in.
        let (elapsed, remaining) = air_timing(AirTimingInputs {
            state: 202,
            predicted_contact_time: Some(1.1),
            ..inputs
        });
        assert_eq!(elapsed, 0.5);
        assert!((remaining - 0.6).abs() < 1e-6, "{remaining}");
        // A trajectory that hits nothing keeps the state's own +184.
        assert_eq!(
            air_timing(AirTimingInputs {
                state: 200,
                predicted_contact_time: Some(-1.0),
                ..inputs
            }),
            (0.5, 4.0)
        );
        // BipedAir splits its own duration and remaining time; the sum stays the duration.
        let (elapsed, remaining) = air_timing(AirTimingInputs {
            state: 501,
            ..inputs
        });
        assert!((elapsed - 0.5).abs() < 1e-6 && remaining == 0.3);
        assert!((elapsed + remaining - 0.8).abs() < 1e-6);
        // On the ground the published pair stands.
        assert_eq!(
            air_timing(AirTimingInputs {
                state: 100,
                ..inputs
            }),
            (0.25, 4.0)
        );
    }
}
