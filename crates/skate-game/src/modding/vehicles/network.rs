//! Owner-simulated vehicles, matching the native player's multiplayer ownership.
use super::*;
use serde::{Deserialize, Serialize};
use skate_vehicles::rapier3d::prelude::{RigidBodyType, Rotation, Vector};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Car {
    pub definition: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular: [f32; 3],
    pub controls: Controls,
    pub occupied: bool,
    pub rider: Option<([f32; 3], [f32; 4])>,
    pub wheels: Vec<[f32; 3]>,
    /// Handling lean of a remote bike. Defaulted, so a peer on an older build
    /// replicates an upright bike rather than failing to parse.
    #[serde(default)]
    pub lean: f32,
}
pub(super) struct Target {
    state: Car,
    age: f32,
}
pub(super) fn simulation_active(
    menu: Option<Res<crate::graphics_menu::Menu>>,
    net: Option<Res<crate::multiplayer::Multiplayer>>,
) -> bool {
    menu.is_none_or(|m| !m.open) || net.is_some_and(|n| n.active())
}
pub(crate) fn capture(world: &World) -> Vec<(String, String, Car)> {
    let v = world.resource::<Vehicles>();
    v.owned
        .iter()
        .filter(|(_, i)| !v.remote.contains_key(&i.id))
        .filter_map(|((owner, key), i)| {
            let car = v.simulation.vehicles.get(&i.id)?;
            let body = &v.simulation.world.bodies[car.body];
            Some((
                owner.clone(),
                key.clone(),
                Car {
                    definition: i.definition_path.clone(),
                    position: body.translation().to_array(),
                    rotation: body.rotation().to_array(),
                    velocity: body.linvel().to_array(),
                    angular: body.angvel().to_array(),
                    controls: car.controls,
                    rider: v
                        .network_pose
                        .as_ref()
                        .filter(|_| {
                            v.driver
                                .as_ref()
                                .is_some_and(|d| d.owner == *owner && d.key == *key)
                        })
                        .map(|pose| {
                            let body = Mat4::from_rotation_translation(
                                Quat::from_array(body.rotation().to_array()),
                                Vec3::from_array(body.translation().to_array()),
                            );
                            let t = Transform::from_matrix(
                                body.inverse() * crate::physics::network::matrix(pose.root),
                            );
                            (t.translation.to_array(), t.rotation.to_array())
                        }),
                    lean: v.simulation.bike_state(i.id).map_or(0., |b| b.lean),
                    occupied: v
                        .driver
                        .as_ref()
                        .is_some_and(|d| d.owner == *owner && d.key == *key),
                    wheels: car
                        .controller
                        .wheels()
                        .iter()
                        .map(|w| [w.steering, w.rotation, w.raycast_info().suspension_length])
                        .collect(),
                },
            ))
        })
        .collect()
}
pub(crate) fn receive(
    world: &mut World,
    root: &std::path::Path,
    owner: &str,
    key: &str,
    state: Car,
) -> Result<(), String> {
    if let Some((p, q)) = state.rider {
        let q = Quat::from_array(q);
        if p.iter().any(|x| !x.is_finite() || x.abs() > 20.)
            || !q.is_finite()
            || (q.length() - 1.).abs() > 0.02
        {
            return Err("Invalid rider pose".into());
        }
    }
    let spawn = Command::VehicleSpawn {
        key: key.into(),
        definition: state.definition.clone(),
        position: state.position,
        heading: 0.,
    };
    let q = Quat::from_array(state.rotation);
    if !spawn.validate()
        || !state.controls.valid()
        || !q.is_finite()
        || (q.length() - 1.).abs() > 0.02
        || state
            .velocity
            .iter()
            .chain(&state.angular)
            .any(|x| !x.is_finite() || x.abs() > 200.)
        || !state.lean.is_finite()
        || state.lean.abs() > 2.
        || state.wheels.len() > 16
        || state
            .wheels
            .iter()
            .flatten()
            .any(|x| !x.is_finite() || x.abs() > 100000.)
    {
        return Err("Invalid remote vehicle".into());
    }
    let owned = (owner.into(), key.into());
    let replace = world
        .resource::<Vehicles>()
        .owned
        .get(&owned)
        .is_some_and(|i| i.definition_path != state.definition);
    if replace {
        remove(world, owner, key);
    }
    if !world.resource::<Vehicles>().owned.contains_key(&owned) {
        super::command(world, root, owner, spawn)?;
    }
    let mut v = world.resource_mut::<Vehicles>();
    let id = v.owned[&owned].id;
    let car = v.simulation.vehicles.get_mut(&id).unwrap();
    car.remote = true;
    car.controls = state.controls;
    let handle = car.body;
    let body = &mut v.simulation.world.bodies[handle];
    body.set_body_type(RigidBodyType::KinematicVelocityBased, true);
    if !v.remote.contains_key(&id) {
        let body = &mut v.simulation.world.bodies[handle];
        body.set_translation(Vector::from_array(state.position), true);
        body.set_rotation(
            Rotation::from_xyzw(
                state.rotation[0],
                state.rotation[1],
                state.rotation[2],
                state.rotation[3],
            ),
            true,
        );
    }
    v.simulation.set_occupied(id, state.occupied);
    v.remote.insert(id, Target { state, age: 0. });
    Ok(())
}
pub(crate) fn remove(world: &mut World, owner: &str, key: &str) {
    // Removal has no asset access or effects on the locally occupied vehicle.
    let _ = super::command(
        world,
        std::path::Path::new(""),
        owner,
        Command::VehicleRemove { key: key.into() },
    );
}
pub(super) fn advance(v: &mut Vehicles, dt: f32) {
    for (&id, target) in &mut v.remote {
        target.age += dt;
        let Some(car) = v.simulation.vehicles.get_mut(&id) else {
            continue;
        };
        let body = &mut v.simulation.world.bodies[car.body];
        let source = Vec3::from_array(target.state.position)
            + Vec3::from_array(target.state.velocity) * target.age.min(0.15);
        let mut error = source - Vec3::from_array(body.translation().to_array());
        if error.length() > 5. {
            body.set_translation(Vector::from_array(source.to_array()), true);
            error = Vec3::ZERO;
        }
        let velocity = if target.age < 0.15 {
            Vec3::from_array(target.state.velocity)
        } else {
            Vec3::ZERO
        };
        body.set_linvel(
            Vector::from_array(
                (velocity + (error * 10.).clamp_length_max(5.))
                    .clamp_length_max(100.)
                    .to_array(),
            ),
            true,
        );
        let current = Quat::from_array(body.rotation().to_array());
        let desired = Quat::from_array(target.state.rotation);
        let delta = (desired * current.inverse()).normalize();
        let delta = if delta.w < 0. { -delta } else { delta };
        let angular = if target.age < 0.15 {
            Vec3::from_array(target.state.angular)
        } else {
            Vec3::ZERO
        };
        body.set_angvel(
            Vector::from_array(
                (angular + delta.to_scaled_axis() * 10.)
                    .clamp_length_max(20.)
                    .to_array(),
            ),
            true,
        );
        car.controls = target.state.controls;
        car.controller.current_vehicle_speed = velocity.dot(current * Vec3::Z);
        for (wheel, values) in car
            .controller
            .wheels_mut()
            .iter_mut()
            .zip(&target.state.wheels)
        {
            wheel.steering = values[0];
            wheel.rotation = values[1];
        }
    }
}

pub(crate) fn attached_root(v: &Vehicles, peer: u64) -> Option<Transform> {
    let prefix = format!("@{peer}:");
    v.owned
        .iter()
        .filter(|((owner, _), _)| owner.starts_with(&prefix))
        .find_map(|(_, i)| {
            let target = v.remote.get(&i.id)?;
            if !target.state.occupied {
                return None;
            }
            let (p, q) = target.state.rider?;
            // The leaned frame, like the local rider: a remote bike's lean is
            // handling state its body never took, so the rider has to ride the
            // same frame the model is drawn in or they come apart in a corner.
            let body = v.rendered_motion.get(&i.id)?.leaned();
            Some(Transform::from_matrix(
                body.to_matrix()
                    * Mat4::from_rotation_translation(Quat::from_array(q), Vec3::from_array(p)),
            ))
        })
}

pub(crate) struct CollisionShape {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular: [f32; 3],
    pub half: [f32; 3],
    pub offset: [f32; 3],
    pub rounding: f32,
    pub mass: f32,
}
pub(crate) fn collision_shapes(v: &Vehicles) -> Vec<CollisionShape> {
    v.simulation
        .vehicles
        .values()
        .map(|car| {
            let b = &v.simulation.world.bodies[car.body];
            let d = &car.definition;
            CollisionShape {
                position: b.translation().to_array(),
                rotation: b.rotation().to_array(),
                velocity: b.linvel().to_array(),
                angular: b.angvel().to_array(),
                half: d.half_extents,
                offset: d.collider_offset,
                rounding: d.collider_rounding,
                mass: d.mass,
            }
        })
        .collect()
}
pub(super) fn skater_proxies(world: &World, v: &mut Vehicles) {
    use skate_vehicles::rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
    let mut frames = world
        .resource::<crate::multiplayer::Multiplayer>()
        .collision_players();
    if !v.occupied() {
        frames.push((
            0,
            crate::physics::network::capture_body(
                world.resource::<crate::physics::GamePhysics>(),
                world.resource::<crate::physics::SkaterRuntime>(),
            ),
        ));
    }
    let mut live = std::collections::BTreeSet::new();
    for (peer, frame) in frames {
        for (index, body) in frame.bodies.iter().enumerate() {
            if frame.enabled & (1u64 << index) == 0 {
                continue;
            }
            // Only near a car: bounded spheres approximate the native articulated collision parts.
            if !v.simulation.vehicles.values().any(|c| {
                Vec3::from_array(v.simulation.world.bodies[c.body].translation().to_array())
                    .distance_squared(Vec3::from_array(body.pose.p))
                    < 100.
            }) {
                continue;
            }
            let key = (peer, index);
            live.insert(key);
            let handle = *v.skaters.entry(key).or_insert_with(|| {
                v.simulation
                    .world
                    .insert(
                        RigidBodyBuilder::kinematic_velocity_based(),
                        ColliderBuilder::ball(if index < 7 { 0.08 } else { 0.14 }).friction(0.3),
                    )
                    .0
            });
            let proxy = &mut v.simulation.world.bodies[handle];
            proxy.set_translation(Vector::from_array(body.pose.p), true);
            proxy.set_linvel(Vector::from_array(body.velocity), true);
        }
    }
    let removed: Vec<_> = v
        .skaters
        .keys()
        .filter(|key| !live.contains(key))
        .copied()
        .collect();
    for key in removed {
        if let Some(body) = v.skaters.remove(&key) {
            v.simulation.world.remove_body(body);
        }
    }
}

pub(super) fn occupied(v: &Vehicles, id: u64) -> bool {
    v.remote.get(&id).map_or_else(
        || {
            v.driver
                .as_ref()
                .and_then(|d| v.owned.get(&(d.owner.clone(), d.key.clone())))
                .is_some_and(|i| i.id == id)
        },
        |t| t.state.occupied,
    )
}

pub(super) fn motion(v: &Vehicles, id: u64) -> Option<interpolation::Motion> {
    let mut m = interpolation::Motion::capture(&v.simulation, id)?;
    if let Some(target) = v.remote.get(&id) {
        let d = &v.simulation.vehicles[&id].definition;
        for (wheel, values) in m.wheels.iter_mut().zip(&target.state.wheels) {
            wheel.translation.y = (d.suspension_length - values[2]) / d.model_scale;
        }
        // A remote bike is a kinematic body, so its own `bike` state never
        // runs. Its lean comes over the wire with the rest of its pose.
        m.lean = target.state.lean;
    }
    Some(m)
}
