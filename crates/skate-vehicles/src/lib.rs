//! Bevy-independent, fixed-step Rapier vehicle simulation and validated mod definitions.
mod assists;
mod bike;
mod definition;
mod handling;
mod safety;
pub use definition::*;
pub use rapier3d;
use rapier3d::{
    control::{DynamicRayCastVehicleController, WheelTuning},
    prelude::*,
};
pub use safety::Ejection;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Controls {
    pub throttle: f32,
    pub steering: f32,
    /// Positive pitches the nose down in flight; ignored on the ground.
    pub pitch: f32,
    pub brake: f32,
    pub handbrake: bool,
    /// Rider weight left/right, -1..1. Positive leans toward driver-left (+X).
    /// Grounded it commands lean; airborne it commands roll.
    pub lean: f32,
    /// Rider weight fore/aft, -1..1. Positive is back: it builds suspension
    /// preload on the ground and is the seat-bounce/wheelie axis.
    pub weight: f32,
    /// Airborne yaw request, -1..1. Positive yaws toward driver-left. Whips.
    pub whip: f32,
    /// Selected trick, 0 for none. The host maps ids to authored rider poses.
    pub trick: u32,
    /// How far into the selected trick the rider is, 0..1.
    pub trick_extend: f32,
    pub clutch: bool,
}
impl Controls {
    pub fn valid(&self) -> bool {
        [
            self.throttle,
            self.steering,
            self.pitch,
            self.lean,
            self.weight,
            self.whip,
        ]
        .iter()
        .all(|v| v.is_finite() && (-1. ..=1.).contains(v))
            && self.brake.is_finite()
            && (0. ..=1.).contains(&self.brake)
            && self.trick_extend.is_finite()
            && (0. ..=1.).contains(&self.trick_extend)
    }
}
/// What the host needs to draw a single-track vehicle, sampled per frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct BikeState {
    /// Handling lean, radians. Negative leans toward driver-left.
    pub lean: f32,
    /// Chassis-local height of the contact line the model leans about.
    pub contact_line: f32,
    /// Suspension compression of the most loaded wheel, 0..1 of travel.
    pub compression: f32,
    /// Stored rider preload, 0..1.
    pub preload: f32,
    pub airborne: bool,
    pub air_time: f32,
    /// Seconds of post-touchdown planting assist left; the rider absorbs
    /// through it.
    pub landing: f32,
}
pub struct Vehicle {
    pub definition: VehicleDefinition,
    pub body: RigidBodyHandle,
    pub controller: DynamicRayCastVehicleController,
    pub controls: Controls,
    handling: handling::Handling,
    bike: bike::Bike,
    /// Drive multiplier for this tick; `bike` raises it after a clutch dump.
    pub(crate) engine_scale: f32,
    /// Driven-wheel speed a clutch dump is handing over, rad/s. Consumed once.
    pub(crate) launch: f32,
    rider: ColliderHandle,
    occupied: bool,
    pub remote: bool,
    inverted_time: f32,
    pub ejection: Option<Ejection>,
}
pub struct Simulation {
    pub world: PhysicsWorld,
    pub vehicles: BTreeMap<u64, Vehicle>,
    next: u64,
}
impl Default for Simulation {
    fn default() -> Self {
        Self {
            world: PhysicsWorld::default(),
            vehicles: BTreeMap::new(),
            next: 1,
        }
    }
}
impl Simulation {
    pub fn ground(&mut self, triangles: impl Iterator<Item = [[f32; 3]; 3]>) -> Result<(), String> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for triangle in triangles {
            let n = vertices.len() as u32;
            vertices.extend(triangle.map(Vector::from_array));
            indices.push([n, n + 1, n + 2]);
        }
        if vertices.is_empty() {
            return Err("Vehicle collision world has no triangles".into());
        }
        // Weld shared vertices and use neighboring face normals at internal edges.
        // Keep authored triangles: this does not simplify or delete map geometry.
        let collider = ColliderBuilder::trimesh_with_flags(
            vertices,
            indices,
            rapier3d::parry::shape::TriMeshFlags::FIX_INTERNAL_EDGES,
        )
        .map_err(|e| e.to_string())?
        .friction(1.);
        self.world.insert(RigidBodyBuilder::fixed(), collider);
        self.world.step();
        Ok(())
    }
    pub fn spawn(
        &mut self,
        definition: VehicleDefinition,
        position: [f32; 3],
        heading: f32,
    ) -> Result<u64, String> {
        definition.validate()?;
        if !point(&position, 100000.) || !heading.is_finite() {
            return Err("Invalid vehicle spawn pose".into());
        }
        let d = &definition;
        let r = d.collider_rounding;
        let shape = if r > 0. {
            ColliderBuilder::round_cuboid(
                d.half_extents[0] - r,
                d.half_extents[1] - r,
                d.half_extents[2] - r,
                r,
            )
        } else {
            ColliderBuilder::cuboid(d.half_extents[0], d.half_extents[1], d.half_extents[2])
        };
        let [x, y, z] = d.inertia_half_extents.unwrap_or(d.half_extents);
        let inertia = Vector::new(y * y + z * z, x * x + z * z, x * x + y * y) * (d.mass / 3.);
        let mass_properties = MassProperties::new(
            Vector::from_array(d.center_of_mass) - Vector::from_array(d.collider_offset),
            d.mass,
            inertia,
        );
        let (body, _) = self.world.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::from_array(position))
                .rotation(Vector::Y * heading)
                .ccd_enabled(true)
                .linear_damping(0.08)
                .angular_damping(0.5),
            shape
                .translation(Vector::from_array(d.collider_offset))
                .mass_properties(mass_properties)
                .friction(d.chassis_friction)
                .friction_combine_rule(CoefficientCombineRule::Min),
        );
        let mut controller = DynamicRayCastVehicleController::new(body);
        controller.index_up_axis = 1;
        controller.index_forward_axis = 2;
        let tuning = WheelTuning {
            suspension_stiffness: d.suspension_stiffness,
            suspension_compression: d.suspension_damping,
            suspension_damping: d.suspension_damping,
            max_suspension_travel: d.suspension_length,
            friction_slip: d.tire_grip,
            ..Default::default()
        };
        for w in &d.wheels {
            controller.add_wheel(
                Vector::from_array(w.position),
                -Vector::Y,
                -Vector::X,
                d.suspension_length,
                w.radius,
                &tuning,
            );
        }
        let safety = &d.rider_safety;
        let rider = self.world.colliders.insert_with_parent(
            ColliderBuilder::capsule_y(safety.half_height, safety.radius)
                .translation(Vector::from_array(d.seat) + Vector::from_array(safety.offset))
                .density(0.)
                .friction(0.2)
                .enabled(false)
                .build(),
            body,
            &mut self.world.bodies,
        );
        let id = self.next;
        self.next += 1;
        self.vehicles.insert(
            id,
            Vehicle {
                definition,
                body,
                controller,
                controls: Controls::default(),
                handling: handling::Handling::default(),
                bike: bike::Bike::default(),
                engine_scale: 1.,
                launch: 0.,
                rider,
                remote: false,
                occupied: false,
                inverted_time: 0.,
                ejection: None,
            },
        );
        Ok(id)
    }
    pub fn remove(&mut self, id: u64) {
        if let Some(v) = self.vehicles.remove(&id) {
            self.world.remove_body(v.body);
        }
    }
    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0. {
            return;
        }
        // Bound substeps even when a host uses a coarse fixed timestep.
        let steps = (dt / 0.008334).ceil().clamp(1., 16.) as u32;
        let h = dt.min(0.1) / steps as f32;
        for _ in 0..steps {
            self.world.integration_parameters.dt = h;
            for v in self.vehicles.values_mut() {
                if v.remote {
                    continue;
                }
                handling::prepare(v, &mut self.world.bodies, h);
                let queries = self.world.broad_phase.as_query_pipeline_mut(
                    self.world.narrow_phase.query_dispatcher(),
                    &mut self.world.bodies,
                    &mut self.world.colliders,
                    QueryFilter::default().exclude_rigid_body(v.body),
                );
                v.controller.update_vehicle(h, queries);
                handling::tires(v, &mut self.world.bodies, &self.world.colliders, h);
                if v.definition.bike.enabled {
                    bike::apply(v, &mut self.world.bodies, h);
                    // After the handling, and against the world rather than
                    // just the bodies: it has to probe the ground ahead.
                    bike::step_up(v, &mut self.world, h);
                } else {
                    assists::apply(v, &mut self.world.bodies, h);
                }
            }
            // Capture after suspension/tire impulses so crash delta-v measures the
            // collision solve, not the normal driving forces preceding it.
            let before = self.capture_riders();
            self.world.step();
            for v in self.vehicles.values_mut().filter(|v| !v.remote) {
                let body = &self.world.bodies[v.body];
                v.controller.current_vehicle_speed = body.linvel().dot(body.rotation() * Vector::Z);
            }
            self.check_riders(&before, h);
        }
    }
    /// Linear velocity, world angular velocity and how many wheels are touching.
    /// A freestyle scorer needs all three: rotation accumulates from the angular
    /// velocity, and the contact count is what separates an air from a landing.
    pub fn telemetry(&self, id: u64) -> Option<([f32; 3], [f32; 3], u32)> {
        let v = self.vehicles.get(&id)?;
        let body = &self.world.bodies[v.body];
        let contacts = v
            .controller
            .wheels()
            .iter()
            .filter(|w| w.raycast_info().is_in_contact)
            .count() as u32;
        Some((
            body.linvel().to_array(),
            body.angvel().to_array(),
            contacts,
        ))
    }
    /// Driven-wheel angular speed, rad/s. Engine audio needs this rather than
    /// chassis speed: a bike revs in the air and against the clutch, and neither
    /// shows up in how fast the chassis is moving.
    pub fn wheel_speed(&self, id: u64) -> f32 {
        self.vehicles
            .get(&id)
            .map(|v| {
                let driven = v.handling.driven_speed(&v.definition);
                // Against the clutch the engine is spinning and the wheel is
                // not. Report whichever is faster, so the motor is heard.
                if v.bike.revs > driven.abs() {
                    v.bike.revs
                } else {
                    driven
                }
            })
            .unwrap_or(0.)
    }
    /// Presentation state for a single-track vehicle. Lean is a handling
    /// state rather than body roll, so the host has to be told about it to
    /// draw it: the model and rider roll by `lean` about `contact_line`.
    pub fn bike_state(&self, id: u64) -> Option<BikeState> {
        let v = self.vehicles.get(&id)?;
        if !v.definition.bike.enabled {
            return None;
        }
        Some(BikeState {
            lean: v.bike.lean,
            contact_line: v.bike.contact_line.unwrap_or(0.),
            compression: v.bike.compression,
            preload: v.bike.preload(),
            airborne: v.bike.airborne,
            air_time: v.bike.air_time,
            landing: v.bike.landing,
        })
    }
    pub fn pose(&self, id: u64) -> Option<([f32; 3], [f32; 4])> {
        let body = &self.world.bodies[self.vehicles.get(&id)?.body];
        Some((body.translation().to_array(), body.rotation().to_array()))
    }
    pub fn reset(&mut self, id: u64, position: [f32; 3], heading: f32) -> Result<(), String> {
        if !point(&position, 100000.) || !heading.is_finite() {
            return Err("Invalid reset pose".into());
        }
        let v = self.vehicles.get_mut(&id).ok_or("Unknown vehicle")?;
        let b = &mut self.world.bodies[v.body];
        b.set_translation(Vector::from_array(position), true);
        b.set_rotation(Rotation::from_rotation_y(heading), true);
        b.set_linvel(Vector::ZERO, true);
        b.set_angvel(Vector::ZERO, true);
        v.controls = Controls::default();
        v.handling = handling::Handling::default();
        v.bike = bike::Bike::default();
        v.engine_scale = 1.;
        v.launch = 0.;
        v.controller.current_vehicle_speed = 0.;
        for wheel in v.controller.wheels_mut() {
            wheel.rotation = 0.;
            wheel.steering = 0.;
            wheel.forward_impulse = 0.;
            wheel.side_impulse = 0.;
        }
        v.ejection = None;
        v.inverted_time = 0.;
        Ok(())
    }
    pub fn floor(&self, position: [f32; 3]) -> Option<[f32; 3]> {
        let origin = Vector::from_array(position) + Vector::Y * 2.;
        let (_, hit) = self.world.cast_ray(
            &Ray::new(origin, -Vector::Y),
            12.,
            true,
            QueryFilter::only_fixed(),
        )?;
        let floor = origin - Vector::Y * hit;
        let shape = rapier3d::parry::shape::Capsule::new_y(0.55, 0.3);
        if self
            .world
            .intersect_shape(
                Pose::from_translation(floor + Vector::Y * 1.),
                &shape,
                QueryFilter::default(),
            )
            .next()
            .is_some()
        {
            return None;
        }
        Some(floor.to_array())
    }
}
