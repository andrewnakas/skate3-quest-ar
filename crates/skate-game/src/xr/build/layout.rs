//! The placed pieces, saved next to the assets folder (on Quest, the app's
//! external files directory) whenever the layout changes and loaded at
//! startup. Poses are in STAGE space, which Quest keeps for the same
//! Guardian, so pieces come back on the same tables.
use super::{Placed, PlacedPieces};
use crate::config::Config;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Layout {
    version: u32,
    pieces: Vec<Placed>,
}

fn path(config: &Config) -> PathBuf {
    config
        .asset_root
        .parent()
        .unwrap_or(&config.asset_root)
        .join("ar-layout.json")
}

pub(super) fn load(config: Res<Config>, mut placed: ResMut<PlacedPieces>) {
    let path = path(&config);
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    match serde_json::from_slice::<Layout>(&bytes) {
        Ok(layout) if layout.version == VERSION => {
            info!("BUILD loaded {} pieces from {}", layout.pieces.len(), path.display());
            placed.items = layout.pieces;
            placed.revision += 1;
        }
        Ok(layout) => warn!("BUILD ignoring {} (version {})", path.display(), layout.version),
        Err(e) => warn!("BUILD ignoring {}: {e}", path.display()),
    }
}

/// Write on every committed edit (not while a piece is being dragged). The
/// first run only records the loaded revision.
pub(super) fn save(config: Res<Config>, placed: Res<PlacedPieces>, mut saved: Local<Option<u64>>) {
    if placed.dragging.is_some() || *saved == Some(placed.revision) {
        return;
    }
    if saved.replace(placed.revision).is_none() {
        return;
    }
    let path = path(&config);
    let layout = Layout {
        version: VERSION,
        pieces: placed.items.clone(),
    };
    let result = serde_json::to_vec_pretty(&layout)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
    if let Err(e) = result {
        warn!("BUILD could not save {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xr::build::pieces::{Piece, PieceKind};

    #[test]
    fn layout_round_trips() {
        let layout = Layout {
            version: VERSION,
            pieces: vec![Placed {
                id: 3,
                piece: Piece { kind: PieceKind::Rail, size: 1.5 },
                position: [0.4, 0.75, -1.2],
                yaw: 0.5,
            }],
        };
        let json = serde_json::to_vec(&layout).unwrap();
        let back: Layout = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.pieces, layout.pieces);
    }
}
