use super::*;

fn pose(x: f32) -> Snapshot {
    Snapshot {
        root: Transform::from_xyz(x, 0., 0.),
        bones: vec![Mat4::from_translation(Vec3::X * x)],
        camera: Transform::from_xyz(x, 2., 3.),
        fov: 1.0,
    }
}

#[test]
fn rolling_window_is_thirty_seconds_with_variable_cadence() {
    let mut replay = Replay::default();
    for i in 0..4000 {
        replay.record(
            pose(i as f32),
            if i % 2 == 0 { 1. / 60. } else { 1. / 30. },
            false,
        );
    }
    let (start, end) = replay.bounds();
    assert!((end - start - 30.).abs() < 1e-6);
    assert!(replay.frames.len() <= 1202);
    replay.enter();
    replay.seek(-100.);
    assert_eq!(replay.cursor, start);
    replay.seek(1000.);
    assert_eq!(replay.cursor, end);
}

#[test]
fn scrubbing_interpolates_shared_pose_and_camera_without_recording() {
    let mut replay = Replay::default();
    replay.enter();
    assert!(!replay.active);
    replay.record(pose(0.), 0.1, false);
    replay.record(pose(1.), 0.1, false);
    replay.enter();
    replay.record(pose(99.), 0.1, false);
    assert_eq!(replay.frames.len(), 2);
    replay.seek(0.05);
    let (a, b, alpha) = replay.sample().unwrap();
    assert_eq!(alpha, 0.5);
    assert_eq!(blend(a.root, b.root, alpha).translation.x, 0.5);
    assert_eq!(blend(a.camera, b.camera, alpha).translation.x, 0.5);
    replay.toggle_camera();
    let free = replay.free_camera.unwrap();
    replay.seek(0.1);
    assert_eq!(replay.free_camera.unwrap(), free);
    replay.toggle_camera();
    assert!(replay.free_camera.is_none());
    replay.exit();
    assert!(replay.sample().is_none());
    replay.record(pose(2.), 0.1, false);
    assert_eq!(replay.frames.back().unwrap().pose.root.translation.x, 2.);
}

#[test]
fn frame_steps_and_cuts_do_not_blend_through_teleports() {
    let mut replay = Replay::default();
    replay.record(pose(0.), 0.1, false);
    replay.record(pose(100.), 0.1, true);
    replay.record(pose(101.), 0.1, false);
    replay.enter();
    replay.seek(0.05);
    let (a, b, _) = replay.sample().unwrap();
    assert_eq!(a.root, b.root);
    assert_eq!(a.root.translation.x, 0.);
    replay.step(1);
    assert_eq!(replay.sample().unwrap().0.root.translation.x, 100.);
    replay.step(-1);
    assert_eq!(replay.cursor, 0.);
}

#[test]
fn replay_gates_all_fixed_gameplay_sets() {
    #[derive(Resource, Default)]
    struct Counts([usize; 3]);
    let mut app = App::new();
    app.init_resource::<Replay>()
        .init_resource::<Counts>()
        .configure_sets(
            FixedUpdate,
            (
                SimulationSet::Input,
                SimulationSet::Controls,
                SimulationSet::Physics,
            )
                .run_if(live),
        )
        .add_systems(
            FixedUpdate,
            (
                (|mut c: ResMut<Counts>| c.0[0] += 1).in_set(SimulationSet::Input),
                (|mut c: ResMut<Counts>| c.0[1] += 1).in_set(SimulationSet::Controls),
                (|mut c: ResMut<Counts>| c.0[2] += 1).in_set(SimulationSet::Physics),
            ),
        );
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().resource_mut::<Replay>().active = true;
    for _ in 0..120 {
        app.world_mut().run_schedule(FixedUpdate);
    }
    assert_eq!(app.world().resource::<Counts>().0, [1; 3]);
    app.world_mut().resource_mut::<Replay>().exit();
    app.world_mut().run_schedule(FixedUpdate);
    assert_eq!(app.world().resource::<Counts>().0, [2; 3]);
}

#[test]
fn controller_replay_scrub_camera_and_exit_do_not_leak_gameplay_input() {
    use crate::input::ControllerInput;
    use skate_core::input::xbox::XboxState;
    let mut app = App::new();
    app.init_resource::<Replay>()
        .init_resource::<ControllerInput>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<bevy::input::mouse::AccumulatedMouseMotion>()
        .init_resource::<Time<Real>>()
        // `controls` reads it to keep replay input out of a vehicle; the real
        // app always has it from `modding::vehicles::install`.
        .init_resource::<crate::modding::vehicles::Vehicles>()
        .add_systems(Update, controls);
    for i in 0..121 {
        app.world_mut()
            .resource_mut::<Replay>()
            .record(pose(i as f32), 1. / 60., false);
    }
    let tick = |app: &mut App, buttons, triggers, left| {
        app.world_mut()
            .resource_mut::<ControllerInput>()
            .sample_raw_for_test(XboxState {
                buttons,
                triggers,
                left,
                right: [0; 2],
            });
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(std::time::Duration::from_secs_f64(1. / 60.));
        app.world_mut().run_schedule(Update);
    };
    tick(&mut app, SELECT, [0; 2], [0; 2]);
    assert!(app.world().resource::<Replay>().active);
    tick(&mut app, SELECT, [0; 2], [0; 2]);
    assert!(
        app.world().resource::<Replay>().active,
        "held View must not toggle repeatedly"
    );
    let end = app.world().resource::<Replay>().cursor;
    tick(&mut app, 0, [255, 0], [0; 2]);
    let rewound = app.world().resource::<Replay>().cursor;
    assert!(rewound < end);
    tick(&mut app, Y, [0; 2], [0; 2]);
    let camera = app.world().resource::<Replay>().free_camera.unwrap();
    tick(&mut app, 0, [0, 128], [32767, 0]);
    let replay = app.world().resource::<Replay>();
    assert!(replay.cursor > rewound);
    assert_ne!(replay.free_camera.unwrap(), camera);
    tick(&mut app, B, [255, 0], [32767, 0]);
    assert!(!app.world().resource::<Replay>().active);
    assert_eq!(
        app.world().resource::<ControllerInput>().mapped_actions,
        [[0.; 18]; 4]
    );
    tick(&mut app, 0, [255, 0], [32767, 0]);
    assert!(app.world().resource::<Replay>().release_controls);
    assert_eq!(
        app.world().resource::<ControllerInput>().mapped_actions,
        [[0.; 18]; 4]
    );
    tick(&mut app, 0, [0; 2], [0; 2]);
    assert!(!app.world().resource::<Replay>().release_controls);
    tick(&mut app, A, [0; 2], [0; 2]);
    assert_ne!(
        app.world().resource::<ControllerInput>().mapped_actions,
        [[0.; 18]; 4]
    );
}
