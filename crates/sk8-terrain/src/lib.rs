//! Scanned skatepark terrain.
//!
//! The headset's scans (its own room mesh, and depth frames fused while walking a park) go into a
//! [`Fusion`]: a 2.5D grid of height clusters, tracking-space metres, y up. [`build`] turns that
//! into something to skate: the raw heights are cleaned and smoothed ([`smooth`]) so lumps and
//! depth noise don't catch the wheels while transitions keep their curve and ledges stay sharp,
//! then meshed with smooth normals ([`mesh`], [`Collider`]) and searched for grindable edges
//! ([`edge_rails`]).
mod fuse;
mod mesh;
mod rails;
mod smooth;

pub use fuse::{FuseConfig, Fusion};
pub use mesh::{mesh, Collider, TerrainMesh};
pub use rails::{edge_rails, RailConfig};
pub use smooth::{smooth, SmoothConfig, Surface};

use glam::{IVec2, Vec2};

/// Heights on a grid of cells: `z[j * width + i]` is the cell at `first + (i, j)`; NaN is no data.
#[derive(Clone, Debug)]
pub struct HeightMap {
    pub first: IVec2,
    pub cell: f32,
    pub width: usize,
    pub depth: usize,
    pub z: Vec<f32>,
    /// Scan support behind each height.
    pub weight: Vec<f32>,
}

impl HeightMap {
    pub fn empty(cell: f32) -> Self {
        Self { first: IVec2::ZERO, cell, width: 0, depth: 0, z: Vec::new(), weight: Vec::new() }
    }

    /// The height at grid (i, j), NaN outside or where there is no data.
    pub fn get(&self, i: i32, j: i32) -> f32 {
        if i < 0 || j < 0 || i as usize >= self.width || j as usize >= self.depth {
            return f32::NAN;
        }
        self.z[j as usize * self.width + i as usize]
    }

    /// Ground (x, z) of a cell's centre.
    pub fn centre(&self, i: usize, j: usize) -> Vec2 {
        ((self.first + IVec2::new(i as i32, j as i32)).as_vec2() + 0.5) * self.cell
    }

    pub fn filled(&self) -> usize {
        self.z.iter().filter(|z| !z.is_nan()).count()
    }

    /// `top` laid over `self` (same cell size): `top` wins wherever it has data.
    pub fn overlay(&self, top: &HeightMap) -> HeightMap {
        if self.z.is_empty() {
            return top.clone();
        }
        if top.z.is_empty() {
            return self.clone();
        }
        let size = |m: &HeightMap| IVec2::new(m.width as i32, m.depth as i32);
        let first = self.first.min(top.first);
        let end = (self.first + size(self)).max(top.first + size(top));
        let (w, d) = ((end.x - first.x) as usize, (end.y - first.y) as usize);
        let mut out = HeightMap { first, cell: self.cell, width: w, depth: d, z: vec![f32::NAN; w * d], weight: vec![0.0; w * d] };
        for m in [self, top] {
            for j in 0..m.depth {
                for i in 0..m.width {
                    let at = j * m.width + i;
                    if m.z[at].is_nan() {
                        continue;
                    }
                    let g = m.first + IVec2::new(i as i32, j as i32) - first;
                    let o = g.y as usize * w + g.x as usize;
                    out.z[o] = m.z[at];
                    out.weight[o] = m.weight[at];
                }
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BuildConfig {
    /// Support a cell needs to count.
    pub min_weight: f32,
    pub smooth: SmoothConfig,
    pub crease_deg: f32,
    pub rails: RailConfig,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self { min_weight: 2.0, smooth: SmoothConfig::default(), crease_deg: 35.0, rails: RailConfig::default() }
    }
}

/// Everything the game needs from a scan.
pub struct Terrain {
    pub surface: Surface,
    pub mesh: TerrainMesh,
    pub rails: Vec<Vec<glam::Vec3>>,
}

pub fn build(fusion: &Fusion, cfg: &BuildConfig) -> Terrain {
    build_from(&fusion.heightmap(cfg.min_weight), cfg)
}

pub fn build_from(raw: &HeightMap, cfg: &BuildConfig) -> Terrain {
    let surface = smooth(raw, &cfg.smooth);
    let mesh = mesh(&surface, cfg.crease_deg);
    let rails = edge_rails(&surface, &cfg.rails);
    Terrain { surface, mesh, rails }
}
