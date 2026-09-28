//! 8289B928 actor eligibility and 82BFBC18 checkpoint geometry checks.
use bevy::prelude::*;
use skate_core::{math::Vector3, physics::board_world::BoardWorld};
use skate_data::collections::Collections;

#[derive(Resource)]
pub(super) struct Validation {
    slope: f32,
    max_drop: f32,
    clearance_length: f32,
    clearance_radius: f32,
}
impl Validation {
    pub fn load(root: &std::path::Path) -> Result<Self, String> {
        let c = Collections::load(root)?;
        let f = |name| c.float("Hash_12B64C0E804B0853", "default", name);
        Ok(Self {
            slope: f("Hash_ADD032CACF6A1C15")?,
            max_drop: f("Hash_8ABE098D3806D273")?,
            clearance_length: f("Hash_C0526C883AF0ECCA")?,
            clearance_radius: f("Hash_CEB092E418A5B001")?,
        })
    }
    pub fn check(&self, world: &BoardWorld, position: [f32; 4]) -> bool {
        if !position.iter().all(|v| v.is_finite()) {
            return false;
        }
        let start = Vector3::new(position[0], position[1] + 0.1, position[2]);
        let end = Vector3::new(start.x, start.y - 10., start.z);
        let Ok(Some(hit)) = world.query_thin_line(start, end) else {
            return false;
        };
        //82BFBE30: CTR jump chain, including the zero-case fallthrough.
        let surface = (hit.tag >> 7) & 31;
        if hit.geometry.normal.y < self.slope
            || start.y - hit.geometry.position.y > self.max_drop
            || matches!(surface, 5 | 6 | 9 | 12 | 13)
        {
            return false;
        }
        let lower = Vector3::new(start.x, start.y + self.clearance_radius, start.z);
        let upper = Vector3::new(lower.x, lower.y + self.clearance_length, lower.z);
        matches!(
            world.query_swept_line(lower, upper, self.clearance_radius),
            Ok(None)
        )
    }
}
