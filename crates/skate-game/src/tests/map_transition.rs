//! Construction/lifetime checks only. Never advance physics or drive gameplay.
use super::*;

#[test]
fn a_pending_request_cannot_be_replaced_by_another_menu_action() {
    let mut transition = MapTransition::default();
    transition.request(Entry {
        label: "first".into(),
        path: None,
    });
    transition.request(Entry {
        label: "second".into(),
        path: Some("other.skate".into()),
    });
    assert!(transition.busy());
    assert!(matches!(&transition.phase, Phase::Requested(entry) if entry.label == "first"));
}

#[test]
#[ignore = "requires private installed assets; CPU lifecycle only, no gameplay"]
fn installed_worlds_prepare_commit_and_retire_without_simulation() {
    let root = PathBuf::from(std::env::var("SKATE_TRANSITION_TEST_ASSETS").unwrap())
        .canonicalize()
        .unwrap();
    let manifest = skate_data::GameAssets::load(&root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(&root, &manifest).unwrap();
    let difficulty = crate::difficulty::Difficulty::Hardcore;
    let physics = GamePhysics::load_with_difficulty(&root, None, difficulty).unwrap();
    let skater = SkaterRuntime::load(&root, &graphs, &physics, difficulty.key()).unwrap();
    let source = skater.animation.source.clone();
    let evaluator = skater.animation.evaluator.clone();
    let mut controls = PlayerControls::load(&root).unwrap();
    controls.preferences = skate_core::input::riding_intentions::PushPreferences {
        automatic_push_enabled: true,
        automatic_push_right: true,
    };
    let mut world = World::new();
    world.insert_resource(Config {
        asset_root: root.clone(),
        verification_capture: None,
        map: None,
        map_path: None,
        difficulty,
        check_assets: false,
        start_paused: false,
        teleport: None,
        multiplayer: Default::default(),
        map_fingerprint: crate::config::map_fingerprint(None).unwrap(),
    });
    world.insert_resource(CurrentMap::from_package(None, None));
    world.insert_resource(graphs);
    world.insert_resource(physics);
    world.insert_resource(skater);
    world.insert_resource(controls);
    world.init_resource::<ButtonInput<KeyCode>>();
    world.init_resource::<ButtonInput<MouseButton>>();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<Image>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<Assets<crate::retail_render::RetailWorldMaterial>>();
    world.init_resource::<Assets<crate::retail_render::RetailSkyMaterial>>();
    let character = world
        .spawn((crate::world::PlayerRoot, Transform::default()))
        .id();
    let mut initial = PreparedScene::new(&world);
    initial.prepare(None, &root);
    initial.publish(&mut world);
    let procedural_meshes = world.resource::<Assets<Mesh>>().len();
    let procedural_entities = world
        .query_filtered::<Entity, With<crate::map_render::MapEntity>>()
        .iter(&world)
        .count();

    for (iteration, file) in [
        Some("BlackBoxPark.skate"),
        None,
        Some("University.skate"),
        None,
        Some("BlackBoxPark.skate"),
        None,
    ]
    .into_iter()
    .enumerate()
    {
        let entry = Entry {
            label: file.unwrap_or("Test world").into(),
            path: file.map(|name| root.parent().unwrap().join("maps").join(name)),
        };
        let old: Vec<_> = world
            .query_filtered::<Entity, With<crate::map_render::MapEntity>>()
            .iter(&world)
            .collect();
        world.resource_mut::<GamePhysics>().contact_count = 123;
        world.resource_mut::<SkaterRuntime>().pose_generation = 999;
        let Phase::Loading { job, .. } = start(&world, entry).unwrap() else {
            panic!("expected worker");
        };
        let prepared = job.join().unwrap().unwrap();
        assert!(
            old.iter().all(|&id| world.get_entity(id).is_ok()),
            "preparation changed live scene"
        );
        assert_eq!(world.resource::<GamePhysics>().contact_count, 123);
        let triangles = prepared.physics.world_triangles().len();
        let spawn = prepared.metadata.spawn;
        assert!(Arc::ptr_eq(&prepared.skater.animation.source, &source));
        assert!(Arc::ptr_eq(
            &prepared.skater.animation.evaluator,
            &evaluator
        ));
        commit(&mut world, prepared);
        assert!(old.into_iter().all(|id| world.get_entity(id).is_err()));
        assert!(world.get_entity(character).is_ok());
        assert_eq!(
            world.resource::<CurrentMap>().generation,
            iteration as u64 + 1
        );
        assert_eq!(world.resource::<CurrentMap>().spawn, spawn);
        let physics = world.resource::<GamePhysics>();
        assert_eq!(physics.world_triangles().len(), triangles);
        assert_eq!(physics.contact_count, 0);
        assert_eq!(physics.ticks, 0);
        assert!(!physics.failed);
        assert_eq!(physics.difficulty_index(), difficulty as u32);
        assert_eq!(world.resource::<SkaterRuntime>().pose_generation, 0);
        assert_eq!(world.resource::<PlayerControls>().ticks, 0);
        assert!(
            world
                .resource::<PlayerControls>()
                .preferences
                .automatic_push_enabled
        );
        assert!(
            world
                .resource::<PlayerControls>()
                .preferences
                .automatic_push_right
        );
        assert!(
            world
                .resource::<crate::presentation::Presentation>()
                .pair()
                .is_none()
        );
        assert!(!world.resource::<crate::replay::Replay>().active);
        assert!(
            world
                .resource::<crate::camera::CameraRuntime>()
                .frame
                .is_none()
        );
        if file.is_none() {
            assert_eq!(world.resource::<Assets<Mesh>>().len(), procedural_meshes);
            assert_eq!(world.resource::<Assets<Image>>().len(), 0);
            assert_eq!(
                world
                    .resource::<Assets<crate::retail_render::RetailSkyMaterial>>()
                    .len(),
                0
            );
            assert_eq!(
                world
                    .resource::<Assets<crate::retail_render::RetailWorldMaterial>>()
                    .len(),
                0
            );
            assert_eq!(
                world
                    .query_filtered::<Entity, With<crate::map_render::MapEntity>>()
                    .iter(&world)
                    .count(),
                procedural_entities
            );
        }
        eprintln!(
            "LIFECYCLE_CHECK generation={} map={file:?} triangles={triangles} entities={} meshes={} images={}",
            iteration + 1,
            world
                .query_filtered::<Entity, With<crate::map_render::MapEntity>>()
                .iter(&world)
                .count(),
            world.resource::<Assets<Mesh>>().len(),
            world.resource::<Assets<Image>>().len()
        );
    }
    let missing = Entry {
        label: "missing".into(),
        path: Some(root.join("nonexistent-transition-check.skate")),
    };
    let Phase::Loading { job, .. } = start(&world, missing).unwrap() else {
        panic!("expected worker");
    };
    assert!(job.join().unwrap().is_err());
    assert_eq!(world.resource::<CurrentMap>().generation, 6);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), procedural_meshes);
    assert!(world.get_entity(character).is_ok());
}
