//! Production controller/graph/solver regression across an authored walking edge.
use super::*;
use skate_core::player::state::PhysicalStateId;

pub(super) fn edge_world(
    material: skate_core::physics::contact::RetailContactMaterial,
    extent: f32,
    floor_y: f32,
) -> BoardWorld {
    // Test-authored collision geometry, not imported render triangles or a
    // support override: four triangles queried by the real world/analyzer.
    let mut triangles = Vec::new();
    for (extent, height, tag) in [(extent, ground::HEIGHT, 1), (50.0, floor_y, 0)] {
        let corners = [
            Vector3::new(-extent, height, -extent),
            Vector3::new(extent, height, -extent),
            Vector3::new(extent, height, extent),
            Vector3::new(-extent, height, extent),
        ];
        for indices in [[0, 2, 1], [0, 3, 2]] {
            triangles.push(
                skate_core::physics::board_world::WorldTriangle::from_vertices(
                    indices.map(|i| corners[i]),
                    material,
                    tag,
                    skate_core::physics::collision::TriangleFeature::ONE_SIDED,
                    [1.0; 3],
                    0.0,
                )
                .unwrap(),
            );
        }
    }
    use skate_core::physics::board_world::query_metadata::{
        Bounds, QueryMesh, QueryMetadata, QueryPool,
    };
    use skate_core::physics::drive_frames::RetailAffineTransform;
    // Explicit test-level collision metadata, using the same authoring contract
    // as Terrain::Flat. No synthetic support/contact publication.
    let metadata = QueryMetadata {
        packed_surfaces: vec![0; triangles.len()],
        meshes: vec![QueryMesh {
            triangle_range: 0..triangles.len(),
            local_to_world: RetailAffineTransform::IDENTITY,
            world_to_local: RetailAffineTransform::IDENTITY,
            local_bounds: Bounds::from_points(triangles.iter().flat_map(|t| t.triangle.vertices))
                .unwrap(),
            matching_group: -1,
            rejection_flags: 0,
            geometry: 1,
            pool: QueryPool::Ground,
        }],
        static_edges: Vec::new(),
        island_flags: 0,
    };
    BoardWorld::with_query_metadata(triangles, metadata).unwrap()
}

#[test]
#[ignore = "requires private stock animation banks and collections"]
fn offboard_walk_off_edge_runs_ground_air_and_landing() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load(root).unwrap();
    physics.world = edge_world(physics.settings.floor_material, 2.0, ground::FLOOR_HEIGHT);
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "normal").unwrap();
    let mut controls = PlayerControls::default();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut grounded = false;
    let mut air_tick = None;
    let mut landed_tick = None;
    for tick in 0..1200 {
        input.sample_raw_for_test(skate_core::input::xbox::XboxState {
            buttons: if tick == 20 { 0x8000 } else { 0 },
            triggers: [0; 2],
            left: if (120..480).contains(&tick) {
                [0, 24000]
            } else {
                [0; 2]
            },
            right: [0; 2],
        });
        let mut actions = input.player_actions();
        controls
            .update_for_physics(&mut actions, &physics, &skater, &camera)
            .unwrap();
        frame::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut actions,
            true,
            &mut camera,
        )
        .unwrap_or_else(|e| panic!("Offboard edge tick{tick}: {e}"));
        let state = skater.player_state.current();
        if state == PhysicalStateId::BipedGround {
            grounded = true;
            if air_tick.is_some() {
                landed_tick.get_or_insert(tick);
            }
        }
        if state == PhysicalStateId::BipedAir {
            assert!(grounded, "Air entered before walking Ground at tick{tick}");
            air_tick.get_or_insert(tick);
            assert_eq!(
                skater.player_input.physical.off_board.trajectory_valid_331, 1,
                "BipedAir Fill did not publish its trajectory at tick{tick}"
            );
            assert_eq!(
                skater.player_input.physical.off_board.flag_304, 0,
                "BipedAir invented an object-grab gate at tick{tick}"
            );
        }
        assert!(
            skater
                .render_pose
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite()),
            "Invalid offboard pose at tick{tick}"
        );
        assert!(
            camera
                .frame
                .as_ref()
                .expect("completed camera")
                .position
                .iter()
                .all(|v| v.is_finite()),
            "Invalid offboard camera at tick{tick}"
        );
    }
    assert!(
        air_tick.is_some(),
        "Walking off the authored platform never entered BipedAir"
    );
    assert!(
        landed_tick.is_some(),
        "BipedAir never returned to BipedGround on the lower floor"
    );
}
