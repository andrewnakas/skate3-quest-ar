//! The faithful port's adapter (imported-disc tier only): the AR world into its physics, teleports,
//! and its deck pose and scoring out to the shared XR layer.
use super::{ArWorld, SkaterLink, TeleportSkater};
use crate::physics::ground::PlacedSurfaces;
use crate::physics::{GamePhysics, SkaterRuntime};
use bevy::prelude::*;
use skate_core::math::Vector3;
use skate_core::physics::board::BodyId;

/// Adapter for the faithful port (imported-disc tier).
pub(crate) struct RetailLinkPlugin;

impl Plugin for RetailLinkPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (apply_world, teleport, publish_pose, publish_hud).chain());
    }
}

fn v(p: Vec3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
}

fn apply_world(
    world: Res<ArWorld>,
    physics: Option<ResMut<GamePhysics>>,
    skater: Option<ResMut<SkaterRuntime>>,
    mut applied: Local<Option<u64>>,
) {
    let (Some(mut physics), Some(mut skater)) = (physics, skater) else {
        return;
    };
    if world.revision == 0 || *applied == Some(world.revision) || skater.is_grinding() {
        return;
    }
    *applied = Some(world.revision);
    let quads = world.room_quads.iter().map(|q| q.map(v)).collect();
    let triangles = world.room_triangles.iter().map(|t| t.map(v)).collect();
    let edges = world.room_edges.iter().map(|&(a, b)| (v(a), v(b))).collect();
    let pieces = world
        .pieces
        .iter()
        .map(|p| PlacedSurfaces {
            quads: p.quads.iter().map(|q| q.map(v)).collect(),
            rail_quads: p.rail_quads.iter().map(|q| q.map(v)).collect(),
            edges: p.edges.iter().map(|&(a, b)| (v(a), v(b))).collect(),
        })
        .collect();
    if let Err(e) = physics.set_real_surfaces(quads, triangles, edges, pieces) {
        error!("AR collision rebuild failed: {e}");
        return;
    }
    let rails: Vec<Vec<Vector3>> = world.rails.iter().map(|r| r.iter().map(|&p| v(p)).collect()).collect();
    if let Err(e) = physics.set_grind_rails(&rails, &mut skater) {
        error!("AR grind rails rebuild failed: {e}");
    }
}

fn teleport(mut requests: MessageReader<TeleportSkater>, skater: Option<ResMut<SkaterRuntime>>) {
    let Some(mut skater) = skater else {
        requests.clear();
        return;
    };
    for r in requests.read() {
        match skater.travel_to(crate::xr::spawn::spawn_matrix(r.point, r.forward)) {
            Ok(()) => info!("XR teleport to {:?}", r.point),
            Err(e) => warn!("XR teleport: {e}"),
        }
    }
}

fn publish_pose(
    physics: Option<Res<GamePhysics>>,
    skater: Option<Res<SkaterRuntime>>,
    mut link: ResMut<SkaterLink>,
) {
    let (Some(physics), Some(skater)) = (physics, skater) else {
        return;
    };
    let deck = physics.board.part_transforms()[BodyId::Deck.index()];
    let t = deck.translation;
    let f = deck.basis.columns[2];
    link.deck = Some((Vec3::new(t.x, t.y, t.z), Vec3::new(f[0], f[1], f[2])));
    link.grinding = skater.is_grinding();
}

/// The port's scoring for the corner readout: the trick being composed, the
/// sequence's points and the line.
fn publish_hud(skater: Option<Res<SkaterRuntime>>, mut hud: ResMut<crate::xr::ui::hud::TrickHud>) {
    let Some(skater) = skater else { return };
    let input = skater.scoring.hud_input();
    let points = if input.sequence_score > 0 {
        if input.multiplier > 1.0 {
            format!("{} x{:.1}", input.sequence_score, input.multiplier)
        } else {
            input.sequence_score.to_string()
        }
    } else {
        String::new()
    };
    let next = crate::xr::ui::hud::TrickHud {
        name: input.trick_name,
        points,
        line: if input.line_score > 0 { format!("line {}", input.line_score) } else { String::new() },
    };
    if *hud != next {
        *hud = next;
    }
}
