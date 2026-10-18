//! What the XR layer shares with whichever skate engine runs: the faithful
//! port (imported disc) or the clean-room engine (public tier, `crate::clean`).
//!
//! `real_world` publishes the room and the placed pieces as [`ArWorld`] (game
//! metres, world space); spawn, build mode and the menus move the skater with
//! [`TeleportSkater`] and read where it is from [`SkaterLink`]. Each engine has
//! an adapter that applies the world, honours teleports and fills the link;
//! the faithful port's is [`RetailLinkPlugin`] here, the clean one lives in
//! `crate::clean::sim`.
use crate::physics::ground::PlacedSurfaces;
use crate::physics::{GamePhysics, SkaterRuntime};
use bevy::prelude::*;
use skate_core::math::Vector3;
use skate_core::physics::board::BodyId;

/// One placed piece's collision, world space.
#[derive(Clone, Default)]
pub(crate) struct PieceSurfaces {
    pub quads: Vec<[Vec3; 4]>,
    pub rail_quads: Vec<[Vec3; 4]>,
    pub edges: Vec<(Vec3, Vec3)>,
}

/// The rideable AR world in game metres. `revision` changes on every rebuild.
#[derive(Resource, Default)]
pub(crate) struct ArWorld {
    pub revision: u64,
    /// Scene planes and box faces.
    pub room_quads: Vec<[Vec3; 4]>,
    /// Room-mesh triangles not covered by a plane or box.
    pub room_triangles: Vec<[Vec3; 3]>,
    /// Grindable furniture edges.
    pub room_edges: Vec<(Vec3, Vec3)>,
    pub pieces: Vec<PieceSurfaces>,
    /// Every grind rail: furniture edges and the pieces' rails and copings.
    pub rails: Vec<Vec<Vec3>>,
}

impl ArWorld {
    /// The real room as triangles (no pieces).
    pub(crate) fn room(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.room_quads
            .iter()
            .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
            .chain(self.room_triangles.iter().copied())
    }

    /// Room and pieces as triangles.
    pub(crate) fn all_triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.room().chain(self.pieces.iter().flat_map(|p| {
            p.quads
                .iter()
                .chain(&p.rail_quads)
                .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
        }))
    }
}

/// Where the skater is, filled each frame by the running engine's adapter.
#[derive(Resource, Default)]
pub(crate) struct SkaterLink {
    /// Deck position and nose direction, game metres.
    pub deck: Option<(Vec3, Vec3)>,
    /// A grind refers to the current rails: collision waits until it ends.
    pub grinding: bool,
}

/// Put the skater down at `point` (a ground point), facing along `forward`.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct TeleportSkater {
    pub point: Vec3,
    pub forward: Vec3,
}

pub(crate) struct LinkPlugin;

impl Plugin for LinkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArWorld>()
            .init_resource::<SkaterLink>()
            .add_message::<TeleportSkater>();
    }
}

/// Adapter for the faithful port (imported-disc tier).
pub(crate) struct RetailLinkPlugin;

impl Plugin for RetailLinkPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (apply_world, teleport, publish_pose).chain());
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
        match skater.travel_to(super::spawn::spawn_matrix(r.point, r.forward)) {
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
