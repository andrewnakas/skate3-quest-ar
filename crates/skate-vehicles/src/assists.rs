//! Optional arcade assists, not recovered Burnout constants. Apply angular
//! impulses through the rigid body; never overwrite its transform or add lift.
use crate::rapier3d::utils::AngularInertiaOps;
use crate::{Vehicle, rapier3d::prelude::*};

pub(crate) fn apply(v: &Vehicle, bodies: &mut RigidBodySet, dt: f32) {
    let mut contacts = 0;
    let mut normal = Vector::ZERO;
    for wheel in v.controller.wheels() {
        if wheel.raycast_info().is_in_contact {
            contacts += 1;
            normal += wheel.raycast_info().contact_normal_ws;
        }
    }
    let body = &mut bodies[v.body];
    let rotation = *body.rotation();
    let local_omega = rotation.inverse() * body.angvel();
    let mut acceleration = Vector::ZERO;
    if contacts >= 2 && v.definition.ground_stability > 0. {
        let normal = normal.normalize_or_zero();
        let up = rotation * Vector::Y;
        if up.dot(normal) > 0.25 {
            // Follow banked support, not global up. Stabilize roll without steering
            // the car or fighting the pitch needed to climb a ramp.
            let error = rotation.inverse() * up.cross(normal);
            acceleration.z = (error.z * 24. - local_omega.z * 8.) * v.definition.ground_stability;
        }
    } else if contacts == 0 && v.occupied && v.definition.air_control > 0. {
        // Rate target keeps repeated input controllable. Neutral stick damps only
        // pitch/roll; yaw and linear momentum remain with the physical simulation.
        // Driver-left is +X: a negative local-Z rotation leans the roof left.
        let target = Vector::new(v.controls.pitch * 2.5, 0., -v.controls.steering * 2.5);
        let strength = v.definition.air_control;
        acceleration.x = ((target.x - local_omega.x) * 3.).clamp(-strength, strength);
        acceleration.z = ((target.z - local_omega.z) * 3.).clamp(-strength, strength);
    }
    if acceleration.length_squared() > 0. {
        let angular_impulse = body.mass_properties().effective_world_inv_inertia.inverse()
            * (rotation * acceleration * dt);
        body.apply_torque_impulse(angular_impulse, true);
    }
}
