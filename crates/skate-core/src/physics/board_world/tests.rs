use super::*;
use crate::math::Basis3;
use crate::physics::drive_frames::RetailAffineTransform;
use query_metadata::{QueryMesh, QueryPool};

fn material() -> RetailContactMaterial {
    RetailContactMaterial {
        static_friction: 0.7,
        dynamic_friction: 0.4,
        restitution: 0.2,
    }
}
fn face(x: f32, tag: u32, fatness: f32) -> WorldTriangle {
    WorldTriangle::from_vertices(
        [
            Vector3::new(x - 2., 0., -2.),
            Vector3::new(x - 2., 0., 2.),
            Vector3::new(x + 2., 0., -2.),
        ],
        material(),
        tag,
        0xe0,
        [1.; 3],
        fatness,
    )
    .unwrap()
}
fn face_at(x: f32, y: f32, tag: u32, fatness: f32) -> WorldTriangle {
    WorldTriangle::from_vertices(
        [
            Vector3::new(x - 2., y, -2.),
            Vector3::new(x - 2., y, 2.),
            Vector3::new(x + 2., y, -2.),
        ],
        material(),
        tag,
        0xe0,
        [1.; 3],
        fatness,
    )
    .unwrap()
}
fn annotated(triangles: Vec<WorldTriangle>) -> BoardWorld {
    let meshes = triangles
        .iter()
        .enumerate()
        .map(|(i, t)| QueryMesh {
            triangle_range: i..i + 1,
            local_to_world: RetailAffineTransform::IDENTITY,
            world_to_local: RetailAffineTransform::IDENTITY,
            local_bounds: Bounds::from_points(t.triangle.vertices).unwrap(),
            matching_group: -1,
            rejection_flags: 0x2000,
            geometry: 5000 + i as u32,
            pool: QueryPool::Ground,
        })
        .collect();
    let packed_surfaces = vec![17; triangles.len()];
    BoardWorld::with_query_metadata(
        triangles,
        QueryMetadata {
            packed_surfaces,
            meshes,
            static_edges: vec![],
            island_flags: 0,
        },
    )
    .unwrap()
}
fn tiled() -> Vec<WorldTriangle> {
    (0..1024)
        .map(|i| face(((i * 37) % 1024) as f32 * 10., i, 0.))
        .collect()
}

#[test]
fn zip_hierarchy_matches_inclusive_linear_mesh_scan_and_source_order() {
    let world = annotated(tiled());
    let meshes = &world.query_metadata().unwrap().meshes;
    let index = query_index::QueryIndex::new(meshes);
    for x in [-100., -2., 0., 2., 8., 10., 155., 10230., 10300.] {
        for radius in [0., 2., 31., 20000.] {
            let bounds = Bounds::from_points([Vector3::new(x, 0., 0.)])
                .unwrap()
                .expanded(radius);
            let expected: Vec<_> = meshes
                .iter()
                .enumerate()
                .filter(|(_, m)| m.local_bounds.overlaps(bounds))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(index.query(bounds, meshes), expected);
        }
    }
}

#[test]
fn line_candidates_prune_distant_clusters_and_keep_canonical_indices() {
    let world = annotated(tiled());
    let start = Vector3::new(-1., 1., -1.);
    let end = Vector3::new(-1., -1., -1.);
    let indices: Vec<_> = world
        .line_candidates(start, end, 0.)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(indices, vec![0]);
    assert_eq!(world.query_metadata().unwrap().meshes[0].geometry, 5000);
    assert_eq!(
        world.query_metadata().unwrap().meshes[0].rejection_flags,
        0x2000
    );
    assert_eq!(world.query_metadata().unwrap().packed_surfaces[0], 17);
    assert_eq!(world.candidate_ranges(None), vec![0..1024]);
    let invalid = Bounds {
        min: Vector3::new(f32::NAN, 0., 0.),
        max: Vector3::ZERO,
    };
    assert_eq!(world.candidate_ranges(Some(invalid)), vec![0..1024]);
    let empty = annotated(vec![]);
    assert_eq!(empty.line_candidates(start, end, 0.).count(), 0);
}

#[test]
fn accelerated_lines_match_full_scan_with_fatness_and_equal_hits() {
    let mut triangles = tiled();
    triangles.insert(0, face(0., 9000, 0.1));
    triangles.insert(0, face(0., 9001, 0.1));
    let linear = BoardWorld::new(triangles.clone());
    let world = annotated(triangles);
    for x in [-100., -2.1, -1., 0., 2., 9., 509., 10229.] {
        for radius in [0., 0.03, 0.5] {
            let start = Vector3::new(x, 2., -1.);
            let end = Vector3::new(x, -2., -1.);
            assert_eq!(
                world.query_swept_line(start, end, radius),
                linear.query_swept_line(start, end, radius)
            );
        }
    }
    assert_eq!(
        world
            .query_thin_line(Vector3::new(-1., 2., -1.), Vector3::new(-1., -2., -1.))
            .unwrap()
            .unwrap()
            .tag,
        9001
    );
}

#[test]
fn thin_endpoint_and_barycentric_tolerances_survive_broadphase() {
    let triangles = vec![face(0., 7, 0.)];
    let linear = BoardWorld::new(triangles.clone());
    let world = annotated(triangles);
    for (start, end) in [
        (Vector3::new(-1., 1., -1.), Vector3::new(-1., 0.000005, -1.)),
        (
            Vector3::new(-1., -0.000005, -1.),
            Vector3::new(-1., -1., -1.),
        ),
        (
            Vector3::new(-2.00002, 1., -1.),
            Vector3::new(-2.00002, -1., -1.),
        ),
    ] {
        let expected = linear.query_thin_line(start, end).unwrap();
        assert!(
            expected.is_some(),
            "fixture must exercise the thin-leaf tolerance"
        );
        assert_eq!(world.query_thin_line(start, end).unwrap(), expected);
    }
}

#[test]
fn predictive_contacts_and_retention_match_full_scan_for_every_primitive() {
    // The distant tiles are staggered in Y on purpose. `tiled()` puts all 1024
    // faces in the plane y=0, and the recovered candidate producer82ACEA30
    // offers only the triangle's own face normal for point/segment/triangle
    // volumes, so a coplanar tile 10km away still projects to a near-zero
    // separation and survives82ACE968's `separation > fat + limit` gate. That
    // is native behaviour: the native narrow phase is only ever reached through
    // the broadphase, which rejects laterally. Feeding it an unculled full scan
    // therefore measures the narrow phase outside its contract, not the
    // hierarchy. Separating the tiles along the one axis the SAT does test
    // makes the linear scan a valid oracle for the accelerated traversal.
    let mut triangles: Vec<_> = tiled()
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let x = t.triangle.vertices[0].x + 2.;
            let y = if x == 0. { 0. } else { 5. + i as f32 };
            face_at(x, y, t.tag, 0.)
        })
        .collect();
    triangles.insert(0, face(0., 9000, 0.02));
    triangles.insert(0, face(0., 9001, 0.02));
    let mut linear = BoardWorld::new(triangles.clone());
    let mut world = annotated(triangles);
    let center = Vector3::new(-0.7, 0.25, -0.7);
    let primitives = [
        ContactPrimitive::Sphere(Sphere {
            center,
            radius: 0.2,
        }),
        ContactPrimitive::Capsule {
            center,
            axis: Vector3::new(1., 0., 0.),
            half_length: 0.3,
            radius: 0.2,
        },
        ContactPrimitive::RoundedBox {
            center,
            basis: Basis3 {
                columns: [[0.6, 0., 0.8], [0., 1., 0.], [-0.8, 0., 0.6]],
            },
            half_extents: Vector3::new(0.3, 0.2, 0.1),
            radius: 0.02,
        },
        ContactPrimitive::Triangle(triangle_from_volume(
            [
                Vector3::new(-1., 0.1, -1.),
                Vector3::new(-1., 0.1, 0.),
                Vector3::new(0., 0.1, -1.),
            ],
            0.03,
            [1.; 3],
            0xe0,
        )),
    ];
    let mut observed = false;
    for velocity in [-60., -1., 0., 1.] {
        let volumes: Vec<_> = primitives
            .iter()
            .enumerate()
            .map(|(i, &primitive)| BoardWorldVolume {
                body: CollisionBody::Attached(i),
                primitive,
                linear_velocity: Vector3::new(0., velocity, 0.),
                material: material(),
            })
            .collect();
        for (padding, maximum) in [(0., 0.), (0., 0.5), (0.2, 0.), (0.02, 0.5)] {
            let query = WorldContactSettings {
                volume_padding: padding,
                maximum_separating_distance: maximum,
                edge_cos_bend_normal_threshold: -1.,
                convexity_epsilon: 0.,
                is_object: false,
            };
            for capacity in [1, 3, 100] {
                for deferred_reduction in [false, true] {
                    let retention = ContactRetentionSettings {
                        capacity,
                        duplicate_distance_squared: 0.000001,
                        deferred_reduction,
                    };
                    let snapshot = |hits: &[BoardCollision]| {
                        hits.iter()
                            .map(|h| retention_record(h.body_a, h.contact))
                            .collect::<Vec<_>>()
                    };
                    let expected = snapshot(linear.query_primitives(&volumes, query, retention));
                    observed |= !expected.is_empty();
                    let actual = snapshot(world.query_primitives(&volumes, query, retention));
                    assert_eq!(
                        actual, expected,
                        "vel={velocity} pad={padding} max={maximum} cap={capacity} defer={deferred_reduction}"
                    );
                    assert_eq!(world.dropped_contacts(), linear.dropped_contacts());
                }
            }
        }
    }
    assert!(observed);
}
