//! Force-based kart handling. Rapier owns suspension queries and rigid-body collision;
//! this module owns steering, wheel angular momentum and combined tire traction.
use crate::{Vehicle, rapier3d::prelude::*};

#[derive(Default)]
pub(crate) struct Handling {
    steering: f32,
    omega: [f32; 8],
    spin: [f32; 8],
}

impl Handling {
    /// Mean angular speed of the driven wheels, rad/s. Signed with the drive
    /// direction, so a reversing wheel reads negative.
    pub(crate) fn driven_speed(&self, d: &crate::VehicleDefinition) -> f32 {
        let driven: Vec<_> = d
            .wheels
            .iter()
            .enumerate()
            .filter(|(_, w)| w.driven)
            .map(|(i, _)| self.omega[i])
            .collect();
        if driven.is_empty() {
            return 0.;
        }
        driven.iter().sum::<f32>() / driven.len() as f32
    }
}

pub(crate) fn prepare(v: &mut Vehicle, bodies: &mut RigidBodySet, dt: f32) {
    let body = &mut bodies[v.body];
    let speed = body.linvel().dot(body.rotation() * Vector::Z);
    let d = &v.definition;
    let c = v.controls;
    if c.throttle != 0. || c.brake != 0. || c.handbrake {
        body.wake_up(true);
    }
    // Full low-speed lock, progressively reduced at speed. Rate limiting models
    // steering travel, including keyboard input; it never rotates the chassis.
    // A bike wants far more lock parked and far less at speed than a car, so its
    // profile supplies the falloff; four-wheel definitions keep the 0.008 curve.
    let falloff = if d.bike.enabled {
        d.bike.steer_falloff
    } else {
        0.008
    };
    let target = c.steering * d.steering_angle / (1. + speed.abs().powi(2) * falloff);
    v.handling.steering += (target - v.handling.steering).clamp(-2.5 * dt, 2.5 * dt);
    let front = d
        .wheels
        .iter()
        .filter(|w| w.steering)
        .map(|w| w.position[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let rear = d
        .wheels
        .iter()
        .filter(|w| !w.steering)
        .map(|w| w.position[2])
        .fold(f32::INFINITY, f32::min);
    let wheelbase = front - rear;
    for (wheel, def) in v.controller.wheels_mut().iter_mut().zip(&d.wheels) {
        let angle = v.handling.steering;
        // Ackermann: inside front wheel turns more sharply around the rear axle.
        wheel.steering = if !def.steering {
            0.
        } else if wheelbase > 0.1 && angle.abs() > 0.001 {
            let radius = wheelbase / angle.tan();
            (wheelbase / (radius - def.position[0])).atan()
        } else {
            angle
        };
        // Disable Rapier's arcade tire impulses: there must be exactly one tire owner.
        wheel.engine_force = 0.;
        wheel.brake = 0.;
        wheel.friction_slip = 0.;
        wheel.side_friction_stiffness = 0.;
    }
    // Quadratic aerodynamic drag, applied at COM; no artificial speed cap.
    let velocity = body.linvel();
    let drag = -velocity * velocity.length() * (0.5 * 1.225 * 0.65) * dt;
    body.apply_impulse(drag, false);
}

pub(crate) fn tires(v: &mut Vehicle, bodies: &mut RigidBodySet, colliders: &ColliderSet, dt: f32) {
    let d = &v.definition;
    let c = v.controls;
    let engine_scale = v.engine_scale;
    let launch = std::mem::take(&mut v.launch);
    let speed = bodies[v.body]
        .linvel()
        .dot(bodies[v.body].rotation() * Vector::Z);
    // Opposing pedal first brakes, then engages reverse close to rest.
    let opposing = c.throttle * speed < -0.5;
    let brake = c.brake.max(if opposing { c.throttle.abs() } else { 0. });
    let throttle = if opposing || brake > 0.01 || (d.bike.enabled && c.clutch) {
        // A bike with the clutch in drives nothing: the engine is revving
        // against it, which `bike` tracks and turns into a launch on release.
        0.
    } else {
        c.throttle
    };
    let driven = d.wheels.iter().filter(|w| w.driven).count() as f32;
    let wheel_count = d.wheels.len() as f32;
    for (i, (wheel, def)) in v
        .controller
        .wheels_mut()
        .iter_mut()
        .zip(&d.wheels)
        .enumerate()
    {
        let r = def.radius;
        // Solid-cylinder rotational inertia, 2% of chassis mass per wheel.
        let inertia = 0.5 * (d.mass * 0.02) * r * r;
        let omega = &mut v.handling.omega[i];
        let limit = if throttle < 0. {
            d.max_speed.min(8.)
        } else {
            d.max_speed
        };
        let taper = (1. - (omega.abs() * r / limit).powi(2)).max(0.);
        let engine = if def.driven {
            throttle * d.engine_force * engine_scale / driven * taper
        } else {
            0.
        };
        *omega += engine * r * dt / inertia;
        if def.driven && launch > omega.abs() {
            // The clutch let go: engine and wheel speed equalise at once.
            *omega = launch;
        }
        // Preserve legacy brake setting scale, but interpret at the reference 120 Hz.
        // Brake torque is time-scaled and can lock a wheel without reversing its spin.
        let braking = if c.handbrake && !def.steering {
            // On a bike the throttle fights the rear brake, and that fight is
            // the pivot: brake alone stops you, brake *and* throttle spins the
            // rear up while it is still sliding, so the bike comes round
            // without losing its drive. Held at full it anchored the bike and
            // a 0.8 s tap left it stationary.
            if d.bike.enabled {
                (1. - c.throttle.max(0.)).max(0.12)
            } else {
                1.
            }
        } else if d.bike.enabled && !def.steering {
            // `brake` is the front lever on a bike (LT), which is most of the
            // stopping power; the rear gets a share, and `handbrake` above is
            // the rear pedal locked for a slide.
            brake * 0.3
        } else {
            brake
        };
        let brake_step = braking * d.brake_impulse * 120. * r * dt / inertia;
        *omega -= omega.clamp(-brake_step, brake_step);
        let contact = *wheel.raycast_info();
        if contact.is_in_contact {
            let p = contact.contact_point_ws;
            let normal = contact.contact_normal_ws;
            let side = (wheel.axle() - normal * wheel.axle().dot(normal)).normalize_or_zero();
            let forward = normal.cross(side).normalize_or_zero();
            let ground = contact.ground_object.and_then(|h| colliders[h].parent());
            let ground_velocity = ground
                .map(|h| bodies[h].velocity_at_point(p))
                .unwrap_or(Vector::ZERO);
            let relative = bodies[v.body].velocity_at_point(p) - ground_velocity;
            let longitudinal = relative.dot(forward);
            let lateral = relative.dot(side);
            let load = wheel.wheel_suspension_force.min(wheel.max_suspension_force);
            // Tire and surface friction coefficients multiply; force scales with load.
            let surface = contact
                .ground_object
                .map(|h| colliders[h].friction())
                .unwrap_or(1.);
            // A rear wheel held locked under the brake is sliding, and a
            // sliding tyre gives up grip in *both* directions. Cutting only
            // its cornering stiffness left it decelerating at nearly a g, so
            // a pivot rotated the bike 166 degrees and parked it. This is
            // what turns that into a slide that keeps its momentum.
            let locked_rear = d.bike.enabled && c.handbrake && !def.steering;
            // Deliberately mild. Anything lower and the rear lets go far
            // enough that the bike swaps ends: at 0.35 a 0.8 s tap swept it
            // 166 degrees and scrubbed 9 m/s down to 2, and raising it to 0.6
            // was *worse*, because more grip while sideways is more drag. A
            // held rear brake should step the back out a little, not lose the
            // corner. A real drift mechanic needs the heading kept on a leash
            // as well, which this does not yet do.
            let slide = if locked_rear { 0.75 } else { 1. };
            let capacity = load * d.tire_grip * surface * slide * dt;
            let inv_long = effective_inverse_mass(&bodies[v.body], p, forward)
                + ground
                    .map(|h| effective_inverse_mass(&bodies[h], p, forward))
                    .unwrap_or(0.);
            let inv_side = effective_inverse_mass(&bodies[v.body], p, side)
                + ground
                    .map(|h| effective_inverse_mass(&bodies[h], p, side))
                    .unwrap_or(0.);
            // Implicit longitudinal slip solve includes wheel inertia. Locked wheels
            // instead use the brake's available holding torque.
            let locked = braking > 0. && omega.abs() < 0.001;
            let mut jx = if locked {
                (-longitudinal / inv_long.max(1e-6)).clamp(
                    -braking * d.brake_impulse * 120. * slide * dt,
                    braking * d.brake_impulse * 120. * slide * dt,
                )
            } else {
                (*omega * r - longitudinal) / (inv_long + r * r / inertia)
            };
            let slip_angle = lateral.atan2(longitudinal.abs().max(1.));
            // The bare 8.0 is tuned for four wheels sharing the load; two wheels
            // each carry double, so a single-track profile scales it.
            let stiffness = if d.bike.enabled {
                8. * d.bike.cornering_scale
            } else {
                8.
            };
            // A locked rear wheel has almost no lateral grip -- that is what
            // makes a handbrake turn a pivot instead of a stop. Without this
            // the rear keeps full cornering stiffness while its rotation is
            // held at zero, so the bike simply anchors and halts.
            let stiffness = if locked_rear { stiffness * 0.2 } else { stiffness };
            let cornering = -load * stiffness * slip_angle * dt;
            let stopping = -lateral / inv_side.max(1e-6) / wheel_count;
            let mut jy = cornering.signum() * cornering.abs().min(stopping.abs());
            // Braking, acceleration and turning share one friction circle.
            let magnitude = jx.hypot(jy);
            if magnitude > capacity && magnitude > 0. {
                jx *= capacity / magnitude;
                jy *= capacity / magnitude;
            }
            if !locked {
                *omega -= jx * r / inertia;
            }
            // Rolling resistance acts as an axle torque, so it cannot accelerate a wheel.
            let rolling = 0.015 * load * r * dt / inertia;
            *omega -= omega.clamp(-rolling, rolling);
            let impulse = forward * jx + side * jy;
            if d.bike.enabled {
                // Single track. In steady cornering a leaned bike's contact
                // patch, mass centre and resultant force are collinear, so the
                // cornering force exerts no net righting moment — gravity's
                // toppling moment cancels it. This chassis is a box on centreline
                // raycasts and has no toppling moment to cancel, so applying the
                // side force at the contact point would invent a righting moment
                // of several hundred N·m and pin roll to zero for good. Apply it
                // at the mass centre instead and let lean be a free degree of
                // freedom that `bike` controls. Drive and braking stay at the
                // contact point, so wheelies and stoppies still work.
                bodies[v.body].apply_impulse_at_point(forward * jx, p, false);
                bodies[v.body].apply_impulse(side * jy, false);
            } else {
                bodies[v.body].apply_impulse_at_point(impulse, p, false);
            }
            if let Some(h) = ground {
                // Suspension was applied to the chassis by Rapier. Return both the
                // tire and suspension reactions to movable support bodies.
                if bodies[h].is_dynamic() {
                    bodies[h].apply_impulse_at_point(-impulse - normal * load * dt, p, true);
                }
            }
            wheel.forward_impulse = jx;
            wheel.side_impulse = jy;
        }
        v.handling.spin[i] += *omega * dt;
        wheel.rotation = v.handling.spin[i];
    }
}

fn effective_inverse_mass(body: &RigidBody, point: Vector, direction: Vector) -> f32 {
    if !body.is_dynamic() {
        return 0.;
    }
    let lever = (point - body.center_of_mass()).cross(direction);
    body.mass_properties().local_mprops.inv_mass
        + lever.dot(body.mass_properties().effective_world_inv_inertia * lever)
}
