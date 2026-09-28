//! TEMPORARY bottom-up audit observations; not native gameplay behavior.
//! Opt in with SKATE3_AUDIT_OFFBOARD_TICKS=start:end (inclusive simulation ticks).
//! Disabled by default. No gameplay state, guards, poses or forces are changed.
use super::{GamePhysics, PlayerControls, SkaterRuntime};
use std::sync::OnceLock;

static WINDOW: OnceLock<Option<(u64, u64)>> = OnceLock::new();

pub(super) fn stage(
    tick: u64,
    phase: &str,
    physics: &GamePhysics,
    s: &SkaterRuntime,
    controls: &PlayerControls,
) {
    let window = WINDOW.get_or_init(|| {
        std::env::var_os("SKATE3_AUDIT_OFFBOARD_TICKS").map(|value| {
            let value = value
                .to_str()
                .expect("Audit tick window must be UTF-8 start:end");
            let (start, end) = value
                .split_once(':')
                .expect("Audit tick window must be start:end");
            let start: u64 = start.parse().expect("Invalid audit start tick");
            let end: u64 = end.parse().expect("Invalid audit end tick");
            assert!(start <= end, "Audit tick window is reversed");
            (start, end)
        })
    });
    let Some((start, end)) = *window else {
        return;
    };
    if tick < start || tick > end {
        return;
    }
    let p = &s.player_input.processed;
    let roots = &s.animated_skeleton.roots;
    let owner = &s.biped_ground.controller.state;
    let deck = physics.board.bodies()[skate_core::physics::board::BodyId::Deck.index()];
    eprintln!(
        "OFFBOARD_AUDIT_BAIL tick={tick} phase={phase} state={:?} requests={:?} pose_error={:?} maximum_pose_error={:?} root_velocity={:?} part_errors={:?}",
        s.player_state.current(),
        s.wipeout.state,
        s.collision_pose_error,
        s.collision_maximum_error,
        s.skeleton_input.root_velocity,
        s.pose_errors.parts
    );
    eprintln!(
        "OFFBOARD_AUDIT tick={tick} phase={phase} state={:?} controls_tick={} controller_words={:08x?} ag={:?} mg={:?} flags={:08x?} root_bits={:08x?} inverse_root_bits={:08x?} motion={:?} output={:?} air_position={:?} air_velocity={:?} physical_com={:?} physical_velocity={:?} board_state={:?} hand={} hand_drives={:?} deck_drive={:?} deck_rates={:?} query_ready={} contact={:?}",
        s.player_state.current(),
        controls.ticks,
        controls.controller.words(),
        controls.action_intents,
        s.animation.motion.animation.motion_intents,
        [
            p.flags_2468,
            p.flags_2472,
            p.flags_2476,
            p.flags_2480,
            p.flags_2484
        ],
        roots.animation_to_world.map(|v| v.map(f32::to_bits)),
        roots.world_to_animation.map(|v| v.map(f32::to_bits)),
        owner.motion,
        owner.frame_output,
        s.biped_air.state.result.position_272,
        s.biped_air.state.result.velocity_288,
        s.skeleton.record.centre_of_mass,
        s.skeleton.record.centre_of_mass_velocity,
        s.skateboard_controller.fields,
        s.board_possession.state.selected_hand_424,
        s.board_possession
            .state
            .hands
            .iter()
            .map(|h| h.dynamics)
            .collect::<Vec<_>>(),
        physics.board.hook().drive.dynamics,
        deck.rates,
        s.offboard_contact.readiness(),
        s.biped_ground.contact
    );
    if phase == "state" || phase == "solve" {
        eprintln!(
            "OFFBOARD_AUDIT_POSE tick={tick} phase={phase} mapped={:?} targets={:?} physical={:?}",
            s.animated_skeleton.record.pose, s.skeleton_input.drive_frames, s.skeleton.record.pose
        );
    }
}
