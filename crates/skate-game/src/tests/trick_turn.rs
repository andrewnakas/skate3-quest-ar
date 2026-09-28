use super::*;

#[test]
#[ignore = "requires private stock graphs and animation assets"]
fn nollie_inward_heel_keeps_body_heading_at_pop() {
    for difficulty in crate::difficulty::Difficulty::ALL {
        for moving in [false, true] {
            replay(difficulty, moving);
        }
    }
}

fn replay(difficulty: crate::difficulty::Difficulty, moving: bool) {
    use skate_core::player::state::PhysicalStateId;
    let root = std::path::PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").unwrap());
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_difficulty(&root, None, difficulty).unwrap();
    let mut skater = SkaterRuntime::load(&root, &graphs, &physics, difficulty.key()).unwrap();
    let mut camera = crate::camera::CameraRuntime::load(&root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let mut controls = PlayerControls::load(&root).unwrap();
    let mut previous_heading = Vec3::Z;
    let mut launch_heading = None;
    let mut max_heading_error = 0.0_f32;
    let mut air_ticks = 0;
    let mut saw_trick = false;
    let mut first_board = None;
    let mut max_board_rotation = 0.0_f32;
    for tick in 0..280 {
        if moving && tick == 100 {
            // Seed a rolling approach without a concurrent steering/body-spin input.
            for body in physics.board.bodies_mut() {
                body.rates.linear_velocity = Vector3::new(0., 0., 5.);
            }
            for body in skater.skeleton.bodies_mut() {
                body.rates.linear_velocity = Vector3::new(0., 0., 5.);
            }
        }
        let right = if (120..150).contains(&tick) {
            [-20409, 26027]
        } else if tick == 150 {
            [10298, 30521]
        } else if (151..154).contains(&tick) {
            [-24902, -21158]
        } else {
            [0; 2]
        };
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: 0,
            triggers: [0; 2],
            left: [0; 2],
            right,
        });
        let mut actions = input.player_actions();
        controls.update(
            &mut actions,
            physics.settings.step.simulation.time_step,
            physics.settings.input_magnitude_threshold,
            skater.player_input.physical.scoring.capabilities_204,
        );
        controls.publish_gestures(
            physics.animation_profile.physics_mode,
            skater.player_input.physical.state.state_16,
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
        .unwrap();
        let root =
            crate::animation::native_matrix(skater.animated_skeleton.roots.animation_to_world);
        let forward = root.z_axis.truncate();
        let heading = Vec3::new(forward.x, 0., forward.z).normalize();
        let state = skater.player_state.current();
        saw_trick |= skater.animation.motion.animation.current_name.as_deref()
            == Some("B_N_INWARDHEELFLIP_A");
        if state == PhysicalStateId::GroundAnimation && air_ticks == 0 {
            launch_heading.get_or_insert(previous_heading);
        }
        if state == PhysicalStateId::KnownAir {
            air_ticks += 1;
        }
        if let Some(initial) = launch_heading {
            if air_ticks <= 10 {
                max_heading_error =
                    max_heading_error.max(initial.angle_between(heading).to_degrees());
                let board = (root * crate::animation::native_matrix(skater.render_pose[25]))
                    .to_scale_rotation_translation()
                    .1
                    .normalize();
                let first = *first_board.get_or_insert(board);
                max_board_rotation =
                    max_board_rotation.max(first.angle_between(board).to_degrees());
            }
        }
        previous_heading = heading;
    }
    assert!(
        saw_trick && air_ticks > 10,
        "{difficulty:?}, moving={moving}: intended nollie did not launch"
    );
    assert!(max_board_rotation > 30., "board spin must remain intact");
    assert!(
        max_heading_error < 0.5,
        "{difficulty:?}, moving={moving}: deck spin rotated rider {max_heading_error} degrees during pop"
    );
    eprintln!(
        "{difficulty:?}, moving={moving}: launch heading drift={max_heading_error}, board rotation={max_board_rotation}"
    );
}
