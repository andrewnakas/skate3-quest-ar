//! Measurement-only production bail probe; no positional correction or fitted limits.
//! Include as a sibling of wipeout_playback in the parent-owned test module.
use super::*;
use skate_core::{
    input::xbox::XboxState,
    physics::{joint_records::default_joint_records, skeleton_animation_record::compose_affine},
    player::state::PhysicalStateId,
};

#[test]
#[ignore = "requires private stock animation banks and collections; diagnostic, not parity proof"]
fn raw_bail_reports_board_joint_anchors_and_render_mapping() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let joints = default_joint_records();
    let mut settled = 0;
    let mut requested = false;
    let mut entered = None;
    let mut maxima = [0.0_f32; 6];
    let mut maximum_ticks = [0; 6];
    for tick in 0..1200 {
        let request = settled >= 60 && !requested;
        requested |= request;
        input.sample_raw_for_test(XboxState {
            buttons: if request { 0x00C0 } else { 0 },
            triggers: if request { [255; 2] } else { [0; 2] },
            left: [0; 2],
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut actions,
            true,
            &mut camera,
        )
        .unwrap_or_else(|error| panic!("board assembly tick{tick}: {error}"));
        let state = skater.player_state.current();
        if !requested {
            if state == PhysicalStateId::PhysicsGround
                && physics.riding.ground.wheel_contact_count > 0
            {
                settled += 1;
            } else {
                settled = 0;
            }
        }
        if state == PhysicalStateId::WipeoutGround && entered.is_none() {
            entered = Some(tick);
        }
        // Same COM + rotated local anchors used in JointJacobian::Build82AE3BC8,
        // host joint_builder.rs127..128/229..232. Not geometric part centers.
        let anchor = |part: usize, words: &[u32]| {
            let body = &physics.board.bodies()[part];
            let local: [f32; 3] = std::array::from_fn(|i| f32::from_bits(words[i]));
            let basis = body.rates.basis.columns;
            let p = body.rates.position;
            let p = [p.x, p.y, p.z];
            std::array::from_fn::<_, 3, _>(|i| {
                basis[2][i].mul_add(
                    local[2],
                    basis[1][i].mul_add(local[1], basis[0][i] * local[0]),
                ) + p[i]
            })
        };
        let gaps = joints.map(|joint| {
            let a = anchor(joint.live_body_a().index(), &joint.frames.words[4..7]);
            let b = anchor(joint.live_body_b().index(), &joint.frames.words[12..15]);
            let gap = (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt();
            assert!(
                gap.is_finite(),
                "Nonfinite board anchors tick{tick}: {joint:?}; a={a:?} b={b:?}"
            );
            gap
        });
        // Track the ordinary post-bail interval separately from constructor settling.
        let mut new_maximum = false;
        if entered.is_some() {
            for i in 0..6 {
                if gaps[i] > maxima[i] {
                    maxima[i] = gaps[i];
                    maximum_ticks[i] = tick;
                    new_maximum = true;
                }
            }
        }
        if request
            || entered == Some(tick)
            || (entered.is_some() && (tick % 60 == 0 || new_maximum))
        {
            eprintln!(
                "board anchors tick{tick} state={state:?} gaps_m={gaps:?} max_m={maxima:?} max_ticks={maximum_ticks:?} hook={:?}",
                physics.board.hook().drive.dynamics
            );
            for (i, joint) in joints.iter().enumerate() {
                eprintln!(
                    "joint{i} A={:?} B={:?} anchorA={:?} anchorB={:?}",
                    joint.live_body_a(),
                    joint.live_body_b(),
                    anchor(joint.live_body_a().index(), &joint.frames.words[4..7]),
                    anchor(joint.live_body_b().index(), &joint.frames.words[12..15])
                );
            }
            // Render globals are animation-space. Convert to world before comparing
            // them with physical bodies; retain full bases to expose scale/shear.
            for bone in skater.skeleton_output.pose.board_bones.all() {
                let world = compose_affine(
                    &skater.animated_skeleton.roots.animation_to_world,
                    &skater.render_pose[bone],
                );
                eprintln!(
                    "board render tick{tick} bone={} parent={} authored_local={:?} world={world:?}",
                    skater.animation.evaluator.frames.bone_names[bone],
                    skater.animation.evaluator.frames.parents[bone],
                    skate_core::animation::output::sqt_to_matrix(skater.animation.pose[bone])
                );
            }
            eprintln!(
                "board physical tick{tick} parts={:?} skeleton_record_deck={:?}",
                physics.board.part_transforms(),
                skater.skeleton.record.pose[0]
            );
        }
        if entered.is_some_and(|start| tick - start >= 900) {
            eprintln!(
                "BOARD_BAIL_ASSEMBLY_RESULT post_bail_max_m={maxima:?} ticks={maximum_ticks:?}"
            );
            return;
        }
    }
    panic!("Did not complete900 post-bail ticks; entered={entered:?}");
}
