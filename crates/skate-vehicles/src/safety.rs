use super::*;

#[derive(Clone, Copy, Debug)]
pub struct Ejection {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub reason: &'static str,
}

/// Build an ejection from a vehicle's current seat motion. Shared by the crash
/// detector and by `bike`'s landing judgement, so a bad landing hands the rider
/// off exactly the way a collision does.
pub(crate) fn ejection(
    definition: &VehicleDefinition,
    body: &RigidBody,
    angular: Vector,
    reason: &'static str,
) -> Ejection {
    let seat = body
        .position()
        .transform_point(Vector::from_array(definition.seat));
    let velocity =
        (body.velocity_at_point(seat) + Vector::Y * definition.rider_safety.eject_up_speed)
            .clamp_length_max(60.);
    Ejection {
        position: seat.to_array(),
        velocity: velocity.to_array(),
        angular_velocity: angular.clamp_length_max(15.).to_array(),
        reason,
    }
}

impl Simulation {
    pub fn set_occupied(&mut self, id: u64, occupied: bool) {
        if let Some(v) = self.vehicles.get_mut(&id) {
            if v.occupied != occupied {
                v.occupied = occupied;
                v.inverted_time = 0.;
                v.ejection = None;
                self.world.colliders[v.rider]
                    .set_enabled(occupied && v.definition.rider_safety.enabled);
                self.world.bodies[v.body].wake_up(true);
            }
        }
    }
    pub fn take_ejection(&mut self, id: u64) -> Option<Ejection> {
        self.vehicles.get_mut(&id)?.ejection.take()
    }
    pub(super) fn capture_riders(&self) -> BTreeMap<u64, (Vector, Vector)> {
        self.vehicles
            .iter()
            .filter(|(_, v)| !v.remote && v.occupied && v.definition.rider_safety.enabled)
            .map(|(&id, v)| {
                let body = &self.world.bodies[v.body];
                let point = body
                    .position()
                    .transform_point(Vector::from_array(v.definition.seat));
                (id, (body.linvel(), body.velocity_at_point(point)))
            })
            .collect()
    }
    pub(super) fn check_riders(&mut self, before: &BTreeMap<u64, (Vector, Vector)>, dt: f32) {
        for (&id, &(linear, point_velocity)) in before {
            let v = self.vehicles.get_mut(&id).unwrap();
            if v.ejection.is_some() {
                continue;
            }
            let body = &self.world.bodies[v.body];
            let safety = &v.definition.rider_safety;
            let up = body.rotation() * Vector::Y;
            let hit = self
                .world
                .narrow_phase
                .contact_pairs_with(v.rider)
                .map(|p| p.total_impulse_magnitude())
                .fold(0_f32, f32::max);
            let contact = body.colliders().iter().any(|&c| {
                self.world
                    .narrow_phase
                    .contact_pairs_with(c)
                    .any(|p| p.total_impulse_magnitude() > 0.)
            });
            // An aerial barrel roll is not a wreck. Require sustained physical
            // contact while inverted; airborne time never advances this timer.
            v.inverted_time = if contact && up.y < safety.inverted_up_y {
                v.inverted_time + dt
            } else {
                0.
            };
            let reason = if hit >= safety.hit_impulse {
                Some("rider_impact")
            } else if contact && (body.linvel() - linear).length() >= safety.crash_delta_v {
                Some("crash")
            } else if v.inverted_time >= safety.inverted_seconds {
                Some("inverted")
            } else {
                None
            };
            if let Some(reason) = reason {
                let mut e = ejection(&v.definition, body, body.angvel(), reason);
                // Carry the faster of the pre-solve seat velocity and the one
                // the helper measured: a collision that stops the chassis dead
                // should still throw the rider at the speed they were doing.
                let carried =
                    (point_velocity + Vector::Y * safety.eject_up_speed).clamp_length_max(60.);
                if carried.length_squared() > Vector::from_array(e.velocity).length_squared() {
                    e.velocity = carried.to_array();
                }
                v.ejection = Some(e);
            }
        }
    }
}
