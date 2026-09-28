//! Opt-in diagnostics for the offboard root producer chain. Test builds only.
use super::SkaterRuntime;

pub(crate) fn trace(tick: u64, phase: &str, s: &SkaterRuntime) {
    if phase == "finish" && std::env::var_os("SKATE3_TRACE_OFFBOARD_ANIMATION").is_some() {
        let render = crate::physics::offboard::skeleton_ground::audit_render_parts(s);
        let legs = [11, 12, 13, 15, 17, 19, 23].map(|part| {
            (
                part,
                s.animated_skeleton.record.pose[part],
                s.skeleton_input.drive_frames[part],
                s.skeleton.record.pose[part],
            )
        });
        eprintln!("POSECHAIN tick={tick} render_errors={render:?} mapped_ik_physical={legs:?}");
    }
    if std::env::var_os("SKATE3_TRACE_OFFBOARD_ROOT").is_none() {
        return;
    }
    let p = &s.player_input.processed;
    let a = &s.animated_skeleton;
    let b = &s.biped_ground.controller.state;
    if phase == "selected" || phase == "state" {
        let selector = &s.offboard_air_selector.core;
        eprintln!(
            "HANDOFF tick={tick} phase={phase} current={} previous={} air_active={} air_frame={} selector={:?} chosen={:?} correction={:?} ground_support={} support_frame={:?} support_velocity={:?} predicted_support={:?} ground_velocity={:?} output_velocity={:?} correction_target={:?} entry_correction={:?}",
            p.state_2508,
            p.state_2504,
            s.biped_air.state.active,
            selector.sampling.frame_8484,
            selector.sampling,
            selector.selected_candidate,
            selector.correction_8304,
            b.motion.support_id_352,
            b.motion.previous_support_frame_192,
            b.motion.support_velocity_256,
            b.motion.predicted_support_velocity_272,
            b.motion.velocity_480,
            b.frame_output.velocity,
            b.correction_target_592,
            b.motion.correction_576
        );
    }
    if phase == "state" && s.biped_air.state.active {
        eprintln!(
            "AIRATTR tick={tick} translation={:?} endCOM={:?} flags={:?} adjustment={:?} remaining={} blend={} target={:?}",
            s.animation_input.fields.animation_translation,
            s.animation_input.fields.animation_end_com,
            s.biped_air.state.flags_544_550,
            s.biped_air.state.adjustment_496,
            s.biped_air.state.time_remaining_444,
            s.biped_air.state.blend_440,
            s.biped_air.state.frame_144
        );
    }
    eprintln!(
        "ROOT tick={tick} phase={phase} state={:?} root={:?} localCOM={:?} physicalCOM={:?} comFrame={:?} motion={:?} output={:?} bipedPosition={:?} position592={:?} velocity608={:?} velocity912={:?} airPosition={:?} airVelocity={:?}",
        s.player_state.current(),
        a.roots.animation_to_world,
        a.record.centre_of_mass,
        s.skeleton.record.centre_of_mass,
        a.board_frames.com_frame[3],
        b.motion.frame_0,
        b.frame_output.frame,
        b.position_368,
        p.vectors_544_560_592_608[2].map(f32::from_bits),
        p.vectors_544_560_592_608[3].map(f32::from_bits),
        p.vectors_880_896_912_928_944[2].map(f32::from_bits),
        s.biped_air.state.result.position_272,
        s.biped_air.state.result.velocity_288
    );
}
