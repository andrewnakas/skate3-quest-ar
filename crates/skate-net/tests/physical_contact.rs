//! Synthetic solver test: no assets, renderer, game launch, or Steam initialization.
use skate_core::{
    math::Vector3,
    physics::{
        assembly::{BoardHook, BodySnapshot},
        board::{BODY_COUNT, BodyId},
        board_step::{AttachedStep, BoardCollision, BoardStep, BoardStepSettings, CollisionBody},
        collision::Sphere,
        contact::RetailContactInput,
        drive_frames::{
            default_live_body_orientations, default_live_body_transforms, default_truck_transforms,
        },
        drive_parameters::{RetailTruckDriveSettings, retail_truck_drive_dynamics},
        force_queue::BoardForceQueue,
        hook_drive::HookDriveState,
        mass::default_skateboard_mass_properties,
        rigid_body::{
            RetailBodyRates, RetailSimulationStep, basis_from_quaternion, world_inverse_inertia,
        },
        world_contact::{ContactPrimitive, PrimitivePairSettings, primitive_pair_contacts},
    },
};
#[test]
fn remote_proxy_after_skeleton_and_targets_receives_opposite_collision_response() {
    let poses = default_live_body_transforms();
    let rotations = default_live_body_orientations();
    let masses = default_skateboard_mass_properties();
    let mut board: [BodySnapshot; BODY_COUNT] = std::array::from_fn(|i| {
        let basis = basis_from_quaternion(rotations[i]);
        BodySnapshot {
            state_flags: 4,
            inertia: masses[i].dynamics,
            rates: RetailBodyRates {
                orientation: rotations[i],
                basis,
                world_inverse_inertia: world_inverse_inertia(
                    basis,
                    masses[i].dynamics.inverse_tensor,
                ),
                position: poses[i].translation,
                linear_velocity: Vector3::new(1., 0., 0.),
                angular_velocity: Vector3::ZERO,
                force_acceleration: Vector3::ZERO,
                torque_acceleration: Vector3::ZERO,
                kinetic_energy: 1.,
                cool_down: 0,
            },
        }
    });
    let deck = BodyId::Deck.index();
    let mut hook = BoardHook {
        body: board[deck],
        drive: HookDriveState::initial(),
    };
    hook.body.state_flags = 0;
    let mut proxy = board[deck];
    proxy.rates.position.x += 0.15;
    proxy.rates.linear_velocity.x = -1.;
    let last_remote = 30 + 9 * 33 - 1;
    let mut attached = vec![proxy; last_remote + 1];
    for b in &mut attached[..last_remote] {
        b.state_flags = 0;
    }
    let manifold = primitive_pair_contacts(
        ContactPrimitive::Sphere(Sphere {
            center: board[deck].rates.position,
            radius: 0.1,
        }),
        ContactPrimitive::Sphere(Sphere {
            center: proxy.rates.position,
            radius: 0.1,
        }),
        PrimitivePairSettings::skater_self_collision(),
    )
    .unwrap();
    let pair = manifold.points[0];
    let collision = BoardCollision {
        body_a: CollisionBody::Board(BodyId::Deck),
        body_b: CollisionBody::Attached(last_remote),
        contact: RetailContactInput {
            position_on_a: pair.a,
            position_on_b: pair.b,
            normal: manifold.normal,
            restitution: 0.05,
            static_friction: 0.5,
            dynamic_friction: 0.4,
            tag: 0,
        },
    };
    BoardStep::default().advance_attached(
        &mut board,
        &mut hook,
        &BoardForceQueue::default(),
        &[collision],
        [0.; 2],
        BoardStepSettings {
            simulation: RetailSimulationStep::fixed_60_hz(30, 0.001, Vector3::ZERO),
            iterations: 25,
            base_truck_transforms: default_truck_transforms(),
            truck_dynamics: retail_truck_drive_dynamics(RetailTruckDriveSettings::STOCK),
            force_point_y_offset: 0.,
        },
        AttachedStep {
            bodies: attached.iter_mut().collect(),
            contacts: &mut [],
            joints: &mut [],
            drives: &mut [],
        },
    );
    assert!(
        board[deck].rates.linear_velocity.x < 1.,
        "Local board must react to the other player"
    );
    assert!(
        attached[last_remote].rates.linear_velocity.x > -1.,
        "Other player's dynamic proxy must receive opposite response"
    );
    assert!(
        board
            .iter()
            .chain(&attached)
            .all(|b| b.rates.position.x.is_finite())
    );
}
