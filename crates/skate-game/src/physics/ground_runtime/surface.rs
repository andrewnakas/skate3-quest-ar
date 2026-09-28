//! Native wheel surface vote82C08818 on live wheel lines and contact reports.
use super::super::riding_outputs::RidingOutputs;
use skate_core::physics::{board_runtime::BoardRuntime, contact_feedback::choose_surface};
/// Wheel surface IDs come from line queries. The report loop82C08198 updates
/// only truck/deck material slots, so wheel contacts contribute vote weights
/// without replacing the query-owned material ID.
pub(crate) fn active_surface(riding: &RidingOutputs, board: &BoardRuntime) -> u32 {
    let contacts = std::array::from_fn(|i| riding.ground.parts[i].in_contact);
    let forced = board
        .contact_reports()
        .iter()
        .any(|r| r.other_surface & 0x0f80 == 0x0600);
    choose_surface(riding.wheel_lines.physics_surfaces, contacts, forced)
}

/// Select the authored audio material from the same four retained wheel queries. Skate 3 packs
/// this independently from the physics surface: audio occupies bits 0..6, while friction uses
/// bits 7..11. Contacting wheels receive the retail four-vote weight and ties prefer the lower ID.
pub(crate) fn active_audio_surface(riding: &RidingOutputs) -> u32 {
    let contacts = std::array::from_fn(|i| riding.ground.parts[i].in_contact);
    choose_audio_surface(riding.wheel_lines.audio_surfaces, contacts)
}

fn choose_audio_surface(surfaces: [u32; 4], contacts: [bool; 4]) -> u32 {
    let mut votes = [0u32; 128];
    for (surface, contact) in surfaces.into_iter().zip(contacts) {
        if surface != 0 {
            votes[surface as usize] += if contact { 4 } else { 1 };
        }
    }
    votes
        .iter()
        .enumerate()
        .skip(1)
        .max_by_key(|(surface, count)| (**count, std::cmp::Reverse(*surface)))
        .filter(|(_, count)| **count != 0)
        .map_or(1, |(surface, _)| surface as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_vote_prefers_contact_weight_then_lower_id() {
        assert_eq!(
            choose_audio_surface([7, 7, 3, 3], [true, false, true, false]),
            3
        );
        assert_eq!(
            choose_audio_surface([7, 7, 3, 3], [true, true, true, false]),
            7
        );
        assert_eq!(choose_audio_surface([0, 0, 0, 0], [true; 4]), 1);
    }
}

/// SurfacePhysics constructor82D85D08 binds these exact five collection keys
/// at24/40/56/72/88. Lookup8 hashes independently match the native immediates.
/// The selector's mode normalization lives in the processed-input producer.
pub(crate) fn surface_key(mode: u32) -> Result<&'static str, String> {
    match mode {
        1 => Ok("smooth"),
        2 => Ok("rough"),
        3 => Ok("slow"),
        4 => Ok("slippery"),
        5 => Ok("veryslow"),
        _ => Err(format!(
            "Processed surface mode {mode} was not normalized by SurfacePhysics"
        )),
    }
}
