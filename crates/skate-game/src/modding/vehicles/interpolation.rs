//! One render-time sample for every visible part of a fixed-step vehicle.
use bevy::prelude::*;

#[derive(Clone)]
pub(super) struct Motion {
    pub body: Transform,
    pub wheels: Vec<Transform>,
    /// Handling lean of a single-track vehicle, radians, and the chassis-local
    /// height of the contact line it leans about. The physics body never
    /// rolls, so this is the only thing that says the bike is leaned over.
    pub lean: f32,
    pub contact_line: f32,
}

fn blend(a: Transform, b: Transform, alpha: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, alpha),
        rotation: a.rotation.normalize().slerp(b.rotation.normalize(), alpha),
        scale: a.scale.lerp(b.scale, alpha),
    }
}

impl Motion {
    /// The body frame with the handling lean applied, rotating about the
    /// contact line rather than the chassis origin so the bike hangs its mass
    /// over the inside of the turn the way a real one does.
    pub fn leaned(&self) -> Transform {
        if self.lean == 0. {
            return self.body;
        }
        let pivot = Vec3::Y * self.contact_line;
        Transform::from_matrix(
            self.body.to_matrix()
                * Mat4::from_translation(pivot)
                * Mat4::from_rotation_z(self.lean)
                * Mat4::from_translation(-pivot),
        )
    }

    pub fn capture(sim: &skate_vehicles::Simulation, id: u64) -> Option<Self> {
        let (p, q) = sim.pose(id)?;
        let car = sim.vehicles.get(&id)?;
        let bike = sim.bike_state(id);
        Some(Self {
            body: Transform::from_translation(Vec3::from_array(p))
                .with_rotation(Quat::from_array(q)),
            lean: bike.map_or(0., |b| b.lean),
            contact_line: bike.map_or(0., |b| b.contact_line),
            wheels: car
                .controller
                .wheels()
                .iter()
                .map(|wheel| {
                    Transform::from_xyz(
                        0.,
                        (car.definition.suspension_length - wheel.raycast_info().suspension_length)
                            / car.definition.model_scale,
                        0.,
                    )
                    .with_rotation(
                        Quat::from_rotation_y(wheel.steering)
                            * Quat::from_rotation_x(-wheel.rotation),
                    )
                })
                .collect(),
        })
    }

    pub fn sample(&self, current: &Self, alpha: f32) -> Self {
        let alpha = alpha.clamp(0., 1.);
        Self {
            body: blend(self.body, current.body, alpha),
            lean: self.lean + (current.lean - self.lean) * alpha,
            contact_line: self.contact_line
                + (current.contact_line - self.contact_line) * alpha,
            wheels: current
                .wheels
                .iter()
                .enumerate()
                .map(|(i, &now)| {
                    self.wheels
                        .get(i)
                        .map_or(now, |&before| blend(before, now, alpha))
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subframes_move_continuously_and_keep_seat_attached() {
        let before = Motion {
            body: Transform::IDENTITY,
            wheels: vec![Transform::IDENTITY],
            lean: 0.,
            contact_line: -0.4,
        };
        let after = Motion {
            body: Transform::from_xyz(1., 0., 0.).with_rotation(Quat::from_rotation_y(0.2)),
            wheels: vec![Transform::from_xyz(0., 0.1, 0.)],
            lean: 0.4,
            contact_line: -0.4,
        };
        let seat = Vec3::new(0., 0.18, -0.2);
        for index in 0..=4 {
            let alpha = index as f32 / 4.;
            let sample = before.sample(&after, alpha);
            assert!((sample.body.translation.x - alpha).abs() < 1e-6);
            assert!((sample.wheels[0].translation.y - alpha * 0.1).abs() < 1e-6);
            assert!((sample.lean - alpha * 0.4).abs() < 1e-6);
            // Leaning is a rigid rotation about a line below the bike, so the
            // seat swings out to the inside instead of staying over the
            // chassis, and its distance from the pivot is unchanged.
            let leaned = sample.leaned();
            let rider = leaned.transform_point(seat);
            let pivot = sample.body.transform_point(Vec3::Y * sample.contact_line);
            let upright = sample.body.transform_point(seat);
            assert!(
                (rider.distance(pivot) - upright.distance(pivot)).abs() < 1e-5,
                "lean must be rigid about the contact line"
            );
            // In the body's own frame, because it is also yawed and moving.
            let swing = sample.body.rotation.inverse() * (rider - upright);
            if alpha > 0. {
                assert!(
                    swing.x < -0.001,
                    "a positive lean should swing the seat toward local -X, got {swing}"
                );
            } else {
                assert!(swing.length() < 1e-6, "no lean should not move the seat");
            }
        }
    }
}
