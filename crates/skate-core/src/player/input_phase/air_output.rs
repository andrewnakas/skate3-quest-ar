//! Completed Air publication shared by physical state and graph consumers.
use super::types::RawVector;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AirOutputFields {
    ///KnownAir Fill82D36880, retained template values outside states that write them.
    pub trajectory_apex_0: RawVector,
    pub collision_position_16: RawVector,
    pub landing_normal_32: RawVector,
    pub selector_vector_48: RawVector,
    pub trajectory_position_64: RawVector,
    pub landing_heading_80: RawVector,
    pub selector_com_position_96: RawVector,
    pub jump_velocity_delta_112: RawVector,
    pub launch_velocity_128: RawVector,
    pub time_in_state_176: f32,
    pub collision_time_180: f32,
    pub collision_normal_speed_188: f32,
    pub time_to_apex_196: f32,
    pub trajectory_index_220: i32,
    pub selected_trajectory_240: [RawVector; 4],
    pub trajectory_plane_samples_336: [f32; 25],
    pub known_air_valid_437: u8,
    pub launched_442: u8,
    /// Common ProcessOutput82DB703C: TrajectorySelector9653.
    pub flag_443: u8,
    pub flag_444: u8,
    /// Air324 high bits used by IsHandPlanting82BA55A8.
    pub handplant_flags_324: u32,
    pub handplant_position_304: RawVector,
    pub handplant_time_320: f32,
    pub vector_160: RawVector,
    pub scalar_184: f32,
    pub landing_normal_144: RawVector,
    pub jump_height_200: f32,
    /// FootPlantManager::Fill82D71040: predicted contact and ground-plant duration.
    pub footplant_contact_time_208: f32,
    pub footplant_duration_212: f32,
    pub footplant_surface_height_216: f32,
    pub footplant_surface_224: u32,
    pub footplant_left_449: u8,
    pub footplant_right_450: u8,
    pub reached_apex_436: u8,
    pub flag_441: u8,
    /// 445, the body flip's side, published beside 441 by 82DB6EC0 from the air
    /// reckoning's 1596/1597 pair. The score collector's flip metric 82DA8EB8 turns it
    /// into the sign of its +2348 -- `+1` when this is zero, `-1` otherwise -- which is
    /// what separates a front flip from a back flip.
    pub flag_445: u8,
    pub flag_446: u8,
    pub flag_447: u8,
    pub flag_448: u8,
    pub use_air_reckoning_452: u8,
}
