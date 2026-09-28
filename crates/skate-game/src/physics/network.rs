//! Physics/transport boundary. Remote dynamic proxies join the existing solve;
//! only the locally owned assembly retains its solved reactions.
use super::*;
use skate_core::physics::{
    assembly::BodySnapshot,
    board_step::CollisionBody,
    board_world::BoardWorldVolume,
    collision::Sphere,
    rigid_body::{RetailQuaternion, world_inverse_inertia},
    world_contact::{ContactPrimitive, triangle_from_volume},
};
use skate_net::{
    Body, Bone, Pose,
    packed::{BodyState, PoseState},
};

pub(crate) fn pose(matrix: Mat4) -> Pose {
    let t = Transform::from_matrix(matrix);
    Pose {
        p: t.translation.to_array(),
        q: t.rotation.normalize().to_array(),
    }
}
pub(crate) fn matrix(p: Pose) -> Mat4 {
    Mat4::from_rotation_translation(Quat::from_array(p.q).normalize(), Vec3::from_array(p.p))
}
fn xyz(v: Vector3) -> [f32; 3] {
    [v.x, v.y, v.z]
}
fn vector(v: [f32; 3]) -> Vector3 {
    Vector3::new(v[0], v[1], v[2])
}
fn add(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}
fn body_pose(body: &BodySnapshot) -> Pose {
    let q = body.rates.orientation;
    Pose {
        p: xyz(body.rates.position),
        q: Quat::from_xyzw(q.x, q.y, q.z, q.w).normalize().to_array(),
    }
}
pub(crate) fn anchors(skater: &SkaterRuntime) -> Vec<usize> {
    let mut indices = skater.animated_skeleton.bone_indices.to_vec();
    let b = skater.skeleton_output.pose.board_bones;
    indices.extend([
        b.front_truck,
        b.back_truck,
        b.front_left_wheel,
        b.front_right_wheel,
        b.back_left_wheel,
        b.back_right_wheel,
    ]);
    // The physical head body is NECK1; HEAD is a separate visual joint.
    // Replicate it explicitly rather than replacing its animated neck offset.
    if let Some(head) = skater
        .animation
        .evaluator
        .frames
        .bone_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case("HEAD"))
    {
        indices.push(head);
    }
    indices.sort_unstable();
    indices.dedup();
    indices
}
pub(crate) fn capture_body(physics: &GamePhysics, skater: &SkaterRuntime) -> BodyState {
    let mut enabled = 0u64;
    {
        for (i, b) in physics.board.bodies().iter().enumerate() {
            if b.state_flags != 1
                && skater
                    .board_possession_live
                    .volume_enabled(CollisionBody::Board(
                        skate_core::physics::board::BodyId::ORDER[i],
                    ))
            {
                enabled |= 1 << i;
            }
        }
    }
    for (i, part) in skater.skeleton_collision.parts.iter().enumerate() {
        if part.enabled && part.volume_group == 0 {
            enabled |= 1 << (7 + i);
        }
    }
    if skater.skeleton_collision.is_ragdoll {
        enabled |= 1 << 63;
    }
    BodyState {
        root: pose(crate::animation::native_matrix(
            skater.animated_skeleton.roots.animation_to_world,
        )),
        enabled,
        bodies: physics
            .board
            .bodies()
            .iter()
            .chain(skater.skeleton.bodies())
            .map(|b| Body {
                pose: body_pose(b),
                velocity: xyz(b.rates.linear_velocity),
                angular: xyz(b.rates.angular_velocity),
            })
            .collect(),
    }
}
pub(crate) fn capture_pose(skater: &SkaterRuntime, anchors: &[usize]) -> PoseState {
    PoseState {
        root: pose(crate::animation::native_matrix(
            skater.animated_skeleton.roots.animation_to_world,
        )),
        bones: anchors
            .iter()
            .filter_map(|&index| {
                skater.render_pose.get(index).map(|m| Bone {
                    index: index as u16,
                    pose: pose(crate::animation::native_matrix(*m)),
                })
            })
            .collect(),
    }
}
/// Immutable stock geometry in each rigid body's COM frame; never transmitted.
pub(crate) struct Schema {
    volumes: Vec<(usize, BoardWorldVolume)>,
    pub fingerprint: u64,
}
impl Schema {
    pub fn new(physics: &GamePhysics, skater: &SkaterRuntime) -> Result<Self, String> {
        let mut mode = skate_core::physics::skeleton_body::SkeletonCollisionMode::new_normal(
            skater.skeleton_collision.settings,
            false,
        );
        for part in &mut mode.parts {
            part.enabled = true;
            part.volume_group = 0;
        }
        let mut volumes = colliders::world_volumes(&physics.board, &physics.settings);
        volumes.extend(skeleton_colliders::world_volumes(&skater.skeleton, &mode)?);
        let bodies = capture_body(physics, skater);
        let volumes = volumes
            .into_iter()
            .map(|mut v| {
                let i = match v.body {
                    CollisionBody::Board(id) => id.index(),
                    CollisionBody::Attached(i) => i + 7,
                    _ => unreachable!(),
                };
                v.primitive = transform(v.primitive, matrix(bodies.bodies[i].pose).inverse());
                (i, v)
            })
            .collect();
        let settings = &physics.settings;
        let fingerprint = skate_net::hash(
            format!(
                "v2{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
                settings.masses,
                settings.authored,
                settings.deck_geometry,
                settings.truck_shape,
                (settings.wheel_radius, settings.truck_collisions),
                skater.skeleton.definition.parts,
                skater.skeleton.definition.bones
            )
            .as_bytes(),
        );
        Ok(Self {
            volumes,
            fingerprint,
        })
    }
}
fn transform(shape: ContactPrimitive, m: Mat4) -> ContactPrimitive {
    let point = |p| vector(m.transform_point3(Vec3::from_array(xyz(p))).to_array());
    let direction = |p| vector(m.transform_vector3(Vec3::from_array(xyz(p))).to_array());
    match shape {
        ContactPrimitive::Sphere(s) => ContactPrimitive::Sphere(Sphere {
            center: point(s.center),
            ..s
        }),
        ContactPrimitive::Capsule {
            center,
            axis,
            half_length,
            radius,
        } => ContactPrimitive::Capsule {
            center: point(center),
            axis: direction(axis),
            half_length,
            radius,
        },
        ContactPrimitive::RoundedBox {
            center,
            basis,
            half_extents,
            radius,
        } => ContactPrimitive::RoundedBox {
            center: point(center),
            basis: skate_core::math::Basis3 {
                columns: (Mat3::from_mat4(m) * Mat3::from_cols_array_2d(&basis.columns))
                    .to_cols_array_2d(),
            },
            half_extents,
            radius,
        },
        ContactPrimitive::Triangle(t) => ContactPrimitive::Triangle(triangle_from_volume(
            t.vertices.map(point),
            t.fatness,
            [-1.; 3],
            0,
        )),
    }
}
/// Conservative enclosing spheres for the remote broad phase.
pub(crate) fn bounds(p: ContactPrimitive) -> (Vec3, f32) {
    match p {
        ContactPrimitive::Sphere(s) => (Vec3::from_array(xyz(s.center)), s.radius),
        ContactPrimitive::Capsule {
            center,
            half_length,
            radius,
            ..
        } => (Vec3::from_array(xyz(center)), half_length + radius),
        ContactPrimitive::RoundedBox {
            center,
            half_extents,
            radius,
            ..
        } => (
            Vec3::from_array(xyz(center)),
            Vec3::from_array(xyz(half_extents)).length() + radius,
        ),
        ContactPrimitive::Triangle(t) => {
            let vertices = t.vertices.map(|v| Vec3::from_array(xyz(v)));
            let center = (vertices[0] + vertices[1] + vertices[2]) / 3.;
            (
                center,
                vertices
                    .iter()
                    .map(|v| v.distance(center))
                    .fold(0., f32::max)
                    + t.fatness,
            )
        }
    }
}
#[derive(Default)]
pub(crate) struct Proxies {
    pub bodies: Vec<BodySnapshot>,
    pub volumes: Vec<BoardWorldVolume>,
}
impl Proxies {
    pub fn append(
        &mut self,
        frame: &BodyState,
        schema: &Schema,
        physics: &GamePhysics,
        skater: &SkaterRuntime,
        age: f32,
    ) {
        // Include detached boards as well as the rider. Distant bodies never enter the solver.
        let nearby = frame.bodies.iter().any(|b| {
            physics
                .board
                .bodies()
                .iter()
                .chain(skater.skeleton.bodies())
                .any(|local| {
                    Vec3::from_array(b.pose.p)
                        .distance_squared(Vec3::from_array(xyz(local.rates.position)))
                        < 64.
                })
        });
        if !nearby {
            return;
        }
        let base = skater.skeleton.bodies().len()
            + skater.skeleton_drives.targets.bodies.len()
            + self.bodies.len();
        let body_start = self.bodies.len();
        for (i, (template, wire)) in physics
            .board
            .bodies()
            .iter()
            .chain(skater.skeleton.bodies())
            .zip(&frame.bodies)
            .enumerate()
        {
            let mut b = *template;
            b.inertia = if i < 7 {
                physics.settings.masses[i].dynamics
            } else {
                let part = skater.skeleton.definition.parts[i - 7];
                let ragdoll = frame.enabled & (1 << 63) != 0 && (1..24).contains(&(i - 7));
                let mut inertia = if ragdoll {
                    part.ragdoll.dynamics
                } else {
                    part.animated.dynamics
                };
                inertia.inverse_mass = if ragdoll {
                    part.inverse_mass_ragdoll
                } else {
                    part.inverse_mass_animated
                };
                inertia
            };
            let q = Quat::from_array(wire.pose.q).normalize();
            b.rates.orientation = RetailQuaternion {
                x: q.x,
                y: q.y,
                z: q.z,
                w: q.w,
            };
            b.rates.basis = skate_core::math::Basis3 {
                columns: Mat3::from_quat(q).to_cols_array_2d(),
            };
            b.rates.world_inverse_inertia =
                world_inverse_inertia(b.rates.basis, b.inertia.inverse_tensor);
            b.rates.position = add(vector(wire.pose.p), vector(wire.velocity.map(|v| v * age)));
            b.rates.linear_velocity = vector(wire.velocity);
            b.rates.angular_velocity = vector(wire.angular);
            b.rates.force_acceleration = Vector3::ZERO;
            b.rates.torque_acceleration = Vector3::ZERO;
            b.state_flags = 4;
            self.bodies.push(b);
        }
        for &(i, ref template) in &schema.volumes {
            if frame.enabled & (1 << i) == 0 {
                continue;
            }
            let b = &self.bodies[body_start + i];
            self.volumes.push(BoardWorldVolume {
                body: CollisionBody::Attached(base + i),
                primitive: transform(template.primitive, matrix(body_pose(b))),
                linear_velocity: b.rates.linear_velocity,
                material: template.material,
            });
        }
    }
}

impl Proxies {
    /// Vehicle chassis join the existing native contact solver; reactions remain local.
    pub fn append_vehicle(
        &mut self,
        shape: &crate::modding::vehicles::network::CollisionShape,
        physics: &GamePhysics,
        skater: &SkaterRuntime,
    ) {
        let p = Vec3::from_array(shape.position);
        if !physics
            .board
            .bodies()
            .iter()
            .chain(skater.skeleton.bodies())
            .any(|b| Vec3::from_array(xyz(b.rates.position)).distance_squared(p) < 100.)
        {
            return;
        }
        let base = skater.skeleton.bodies().len()
            + skater.skeleton_drives.targets.bodies.len()
            + self.bodies.len();
        let mut body = physics.board.bodies()[0];
        let q = Quat::from_array(shape.rotation).normalize();
        body.rates.position = vector(shape.position);
        body.rates.orientation = RetailQuaternion {
            x: q.x,
            y: q.y,
            z: q.z,
            w: q.w,
        };
        body.rates.basis = skate_core::math::Basis3 {
            columns: Mat3::from_quat(q).to_cols_array_2d(),
        };
        body.inertia.inverse_mass = 1. / shape.mass;
        let [x, y, z] = shape.half;
        body.inertia.inverse_tensor = vector([
            3. / (shape.mass * (y * y + z * z)),
            3. / (shape.mass * (x * x + z * z)),
            3. / (shape.mass * (x * x + y * y)),
        ]);
        body.rates.world_inverse_inertia =
            world_inverse_inertia(body.rates.basis, body.inertia.inverse_tensor);
        body.rates.linear_velocity = vector(shape.velocity);
        body.rates.angular_velocity = vector(shape.angular);
        body.rates.force_acceleration = Vector3::ZERO;
        body.rates.torque_acceleration = Vector3::ZERO;
        body.state_flags = 4;
        self.bodies.push(body);
        self.volumes.push(BoardWorldVolume {
            body: CollisionBody::Attached(base),
            primitive: ContactPrimitive::RoundedBox {
                center: vector((p + q * Vec3::from_array(shape.offset)).to_array()),
                basis: body.rates.basis,
                half_extents: vector(shape.half.map(|x| x - shape.rounding)),
                radius: shape.rounding,
            },
            linear_velocity: body.rates.linear_velocity,
            material: skate_core::physics::contact::RetailContactMaterial {
                static_friction: 0.4,
                dynamic_friction: 0.3,
                restitution: 0.1,
            },
        });
    }
}
