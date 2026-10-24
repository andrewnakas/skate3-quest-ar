//! What the XR layer shares with whichever skate engine runs: the faithful
//! port (imported disc) or the clean-room engine (public tier, `crate::clean`).
//!
//! `real_world` publishes the room and the placed pieces as [`ArWorld`] (game
//! metres, world space); spawn, build mode and the menus move the skater with
//! [`TeleportSkater`] and read where it is from [`SkaterLink`]. Each engine has
//! an adapter that applies the world, honours teleports and fills the link;
//! the faithful port's is [`RetailLinkPlugin`] here, the clean one lives in
//! `crate::clean::sim`.
use bevy::prelude::*;

#[cfg(feature = "retail")]
#[path = "link_retail.rs"]
mod retail;
#[cfg(feature = "retail")]
pub(crate) use retail::RetailLinkPlugin;

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

