//! Physical state602, TU3 vtable82327434. Stock no-comply/boneless/fastplant
//! animations select this state with AnimSkateboard and launch with FootJump.
use super::footplant::math::*;
use super::{GamePhysics, SkaterRuntime, air_phase, plant_skeleton};
use skate_core::point_graph::PointGraph;
use skate_data::collections::Collections;

pub(crate) struct Boneless {
    pub toe: usize,       //52
    pub anchor: [f32; 4], //64
    pub right: bool,      //80
    curves: [PointGraph<8>; 4],
}
impl Boneless {
    pub fn load(data: &Collections) -> Result<Self, String> {
        let graph = |name| -> Result<_, String> {
            let words = data
                .words::<20>("Hash_CCB95A83C78B4FF9", "default", name)?
                .map(f32::from_bits);
            Ok(PointGraph {
                x: words[4..12].try_into().unwrap(),
                y: words[12..20].try_into().unwrap(),
            })
        };
        Ok(Self {
            toe: 15,
            anchor: [0.0; 4],
            right: false,
            curves: [
                graph("Hash_6D781EFAF01E707D")?,
                graph("Hash_C9112CCD0BCB1850")?,
                graph("Hash_88D0CDEFA38A36D6")?,
                graph("Hash_9228B7F18C223F9C")?,
            ],
        })
    }
}

///82D4C900: anchor the last completed physical toe position.
pub(super) fn enter(physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    skater.ground_lifecycle.board_animated_290 = 1;
    let drive = &mut physics.board.hook_mut().drive;
    //Globals12/912 at822F860C/822F8990; linear HardDrive, angular SoftDrive.
    drive.dynamics[..4].copy_from_slice(&[0x426f_ffff, 0, 0x4560_fffe, 2]);
    drive.enable_angular_soft();
    let right = skater.player_input.processed.flags_2468 & (1 << 26) != 0;
    skater.boneless.toe = if right { 19 } else { 15 };
    skater.boneless.anchor = skater.skeleton.record.pose[skater.boneless.toe][3];
    skater.boneless.right = right;
    Ok(())
}

///82D4C9B8. Authored FootJump controls every launch request; no host timer.
pub(super) fn update(physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    plant_skeleton::advance(
        physics,
        skater,
        skater.boneless.anchor,
        Some(skater.boneless.toe),
    )?;
    plant_skeleton::hold_foot(skater, skater.boneless.right, skater.boneless.anchor, 3);
    if skater.player_input.processed.flags_2480 & (1 << 13) != 0 {
        launch(physics, skater)?;
    }
    Ok(())
}

///82D4CB18/82D4CC18, four PointNegGraphData8 values from the stock schema.
fn launch(physics: &mut GamePhysics, skater: &mut SkaterRuntime) -> Result<(), String> {
    let p = &skater.player_input.processed;
    let up = p.vectors_544_560_592_608[0].map(f32::from_bits);
    let current = p.vectors_544_560_592_608[3].map(f32::from_bits);
    let prepared = p.prepared_jump_704.map(f32::from_bits);
    let horizontal = sub(prepared, scale(up, dot(prepared, up)));
    let speed = length(horizontal);
    let g = &skater.boneless.curves;
    let vertical = g[0].evaluate(up[1]) * g[1].evaluate(speed);
    let scaled_speed = g[2].evaluate(speed) * speed;
    let a = madd(up, vertical - dot(current, up), current);
    let b = madd(normalize(horizontal), scaled_speed, scale(up, vertical));
    let blend = g[3].evaluate(up[1]);
    let mut velocity = madd(b, blend, scale(a, 1.0 - blend));
    if (length(velocity) - length(current)).abs() > 10.0 {
        velocity = current;
    }
    let forward = p.effective_anim_transform_192[2].map(f32::from_bits); //Processed224
    let component = dot(velocity, forward).max(0.0);
    if component < 1.92 {
        velocity = madd(forward, 1.92 - component, velocity);
    }
    let com = p.vectors_544_560_592_608[2].map(f32::from_bits);
    let position = madd(up, dot(sub(skater.boneless.anchor, com), up) + 0.25, com);
    let mut info = air_phase::launch_info(physics, skater)?;
    info.start_velocity = velocity;
    info.start_position_override = position;
    info.board_position_override = position;
    info.use_position_override = true;
    info.player_jumped = true;
    let input = air_phase::selector_input(physics, skater)?;
    skater.trajectory.launch(info, input, &physics.world)?;
    skater.trajectory.update(
        input,
        &physics.world,
        crate::physics::air_trajectory::GrindContext::from_processed(
            &skater.player_input.processed,
            crate::physics::solve::deck_frame(&physics.board)[3],
        ),
    )?;
    Ok(())
}
