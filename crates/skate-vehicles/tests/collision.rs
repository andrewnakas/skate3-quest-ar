use skate_vehicles::{rapier3d::prelude::*, *};

fn setup() -> (Simulation, u64) {
    let mut s = Simulation::default();
    s.ground(
        [
            [[-100., 0., -100.], [100., 0., 100.], [100., 0., -100.]],
            [[-100., 0., -100.], [-100., 0., 100.], [100., 0., 100.]],
        ]
        .into_iter(),
    )
    .unwrap();
    let d = serde_json::from_str(include_str!(
        "../../../sdk/examples/mario-kart/vehicle.json"
    ))
    .unwrap();
    let id = s.spawn(d, [0., 1., 0.], 0.).unwrap();
    for _ in 0..240 {
        s.step(1. / 120.);
    }
    (s, id)
}

#[test]
fn map_faces_share_vertices_and_internal_edge_normals() {
    let (s, _) = setup();
    let mesh = s
        .world
        .colliders
        .iter()
        .find_map(|(_, c)| c.shape().as_trimesh())
        .unwrap();
    assert_eq!(mesh.vertices().len(), 4);
    assert_eq!(mesh.indices().len(), 2);
    assert!(
        mesh.flags()
            .contains(rapier3d::parry::shape::TriMeshFlags::FIX_INTERNAL_EDGES)
    );
}

#[test]
fn glancing_chassis_contact_preserves_tangential_motion() {
    let fixed = scrape(false);
    let previous = scrape(true);
    println!("Tangential speed after scrape: corrected={fixed}, previous average={previous}");
    assert!(
        fixed > 9.5,
        "wall scrape consumed tangential motion: {fixed}"
    );
    assert!(
        fixed > previous + 0.5,
        "friction fix did not improve scrape: {fixed}, {previous}"
    );
}

fn scrape(previous_average: bool) -> f32 {
    let (mut s, id) = setup();
    s.world.insert(
        RigidBodyBuilder::fixed().translation(Vector::new(1., 1., 0.)),
        ColliderBuilder::cuboid(0.1, 2., 100.).friction(1.),
    );
    s.world.step();
    let h = s.vehicles[&id].body;
    if previous_average {
        let colliders = s.world.bodies[h].colliders().to_vec();
        for c in colliders {
            s.world.colliders[c].set_friction_combine_rule(CoefficientCombineRule::Average);
        }
    }
    s.world.bodies[h].set_linvel(Vector::new(6., 0., 12.), true);
    let mut touched = false;
    for _ in 0..30 {
        s.step(1. / 120.);
        touched |= s
            .world
            .narrow_phase
            .contact_pairs()
            .any(|p| p.total_impulse_magnitude() > 1.);
    }
    assert!(touched);
    let velocity = s.world.bodies[h].linvel();
    assert!(s.pose(id).unwrap().0[0] < 1., "passed through wall");
    velocity.z
}

#[test]
fn head_on_collision_still_stops_and_ejects_at_speed() {
    let (mut s, id) = setup();
    s.world.insert(
        RigidBodyBuilder::fixed().translation(Vector::new(0., 1., 3.)),
        ColliderBuilder::cuboid(10., 2., 0.05),
    );
    s.world.step();
    s.set_occupied(id, true);
    let h = s.vehicles[&id].body;
    s.world.bodies[h].set_linvel(Vector::Z * 25., true);
    let mut ejection = false;
    for _ in 0..60 {
        s.step(1. / 120.);
        ejection |= s.take_ejection(id).is_some();
    }
    assert!(ejection);
    assert!(s.pose(id).unwrap().0[2] < 3.);
    assert!(s.world.bodies[h].linvel().z < 1.);
}
