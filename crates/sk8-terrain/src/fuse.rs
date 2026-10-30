//! Fusion: depth samples and scan triangles into a 2.5D grid of height clusters.
//!
//! Each cell keeps a few height clusters (running weighted means) instead of one average, so a
//! wall face seen side-on, a person walking through or a stray depth pixel doesn't drag the top
//! surface around: the surface is the highest cluster with real support. Depth rays also carve:
//! a ray that passes over a cell proves nothing stands there above the ray, so clusters above it
//! lose weight (people and bags that have moved away fade out).
use crate::HeightMap;
use glam::{IVec2, Vec2, Vec3};
use std::collections::HashMap;

/// Cells per chunk edge.
pub const CHUNK: i32 = 32;
const CLUSTERS: usize = 4;
/// Weight a cluster stops gaining at, so new evidence can still move it.
const WEIGHT_CAP: f32 = 64.0;

#[derive(Clone, Copy, Debug)]
pub struct FuseConfig {
    /// Cell edge, metres.
    pub cell: f32,
    /// Samples this close in height join one cluster.
    pub gap: f32,
    /// Weight a carving ray takes from each cluster above it.
    pub carve: f32,
    /// Carving stops this far above a cluster (depth noise).
    pub carve_margin: f32,
    /// Carving stops this far (metres, along the ground) before the hit.
    pub carve_stop: f32,
}

impl Default for FuseConfig {
    fn default() -> Self {
        Self { cell: 0.1, gap: 0.08, carve: 0.35, carve_margin: 0.1, carve_stop: 0.3 }
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Cluster {
    h: f32,
    w: f32,
}

#[derive(Clone, Copy, Default, Debug)]
struct Cell {
    c: [Cluster; CLUSTERS],
    n: u8,
}

impl Cell {
    fn add(&mut self, h: f32, w: f32, gap: f32) {
        let n = self.n as usize;
        let near = (0..n)
            .map(|i| (i, (self.c[i].h - h).abs()))
            .filter(|&(_, d)| d <= gap)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = near {
            let c = &mut self.c[i];
            let total = c.w + w;
            c.h += (h - c.h) * w / total;
            c.w = total.min(WEIGHT_CAP);
            self.merge(gap);
        } else if n < CLUSTERS {
            self.c[n] = Cluster { h, w };
            self.n += 1;
        } else {
            // A new height while full: it takes the weakest cluster's place if that one is no
            // better supported than a couple of samples.
            let weakest = (0..n).min_by(|&a, &b| self.c[a].w.total_cmp(&self.c[b].w)).unwrap();
            if self.c[weakest].w < 2.0 * w {
                self.c[weakest] = Cluster { h, w };
            }
        }
    }

    fn merge(&mut self, gap: f32) {
        let mut i = 0;
        while i < self.n as usize {
            let mut j = i + 1;
            while j < self.n as usize {
                if (self.c[i].h - self.c[j].h).abs() <= gap {
                    let (a, b) = (self.c[i], self.c[j]);
                    let w = a.w + b.w;
                    self.c[i] = Cluster { h: (a.h * a.w + b.h * b.w) / w, w: w.min(WEIGHT_CAP) };
                    self.remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }

    fn remove(&mut self, i: usize) {
        let n = self.n as usize;
        self.c.copy_within(i + 1..n, i);
        self.n -= 1;
    }

    fn carve(&mut self, ray_h: f32, margin: f32, amount: f32) {
        let mut i = 0;
        while i < self.n as usize {
            if self.c[i].h > ray_h + margin {
                self.c[i].w -= amount;
                if self.c[i].w <= 0.0 {
                    self.remove(i);
                    continue;
                }
            }
            i += 1;
        }
    }

    /// The surface: the highest cluster with at least `min_w` weight and a quarter of the best
    /// cluster's. Where the clusters below it run on without a gap (a slope crossing the cell,
    /// seen over its whole rise) the surface is that run's weighted median: on a slope the height
    /// at the cell's centre, not its top edge; on a ledge's edge (a well-seen top over a face seen
    /// side-on) still the top.
    fn height(&self, min_w: f32, gap: f32) -> Option<(f32, f32)> {
        let n = self.n as usize;
        let best = self.c[..n].iter().map(|c| c.w).fold(0.0, f32::max);
        let need = min_w.max(0.25 * best);
        let top = *self.c[..n].iter().filter(|c| c.w >= need).max_by(|a, b| a.h.total_cmp(&b.h))?;
        let mut run: Vec<Cluster> = self.c[..n].iter().copied().filter(|c| c.h <= top.h && c.w >= 0.5 * min_w).collect();
        run.sort_by(|a, b| b.h.total_cmp(&a.h));
        let mut end = 1;
        while end < run.len() && run[end - 1].h - run[end].h <= 2.0 * gap {
            end += 1;
        }
        let run = &run[..end];
        let half = 0.5 * run.iter().map(|c| c.w).sum::<f32>();
        let mut acc = 0.0;
        for c in run {
            acc += c.w;
            if acc >= half {
                return Some((c.h, top.w));
            }
        }
        Some((top.h, top.w))
    }
}

/// The fused scan.
pub struct Fusion {
    pub config: FuseConfig,
    chunks: HashMap<IVec2, Box<[Cell]>>,
    /// Bumped by every change, so consumers know to rebuild.
    pub revision: u64,
    pub samples: u64,
}

impl Fusion {
    pub fn new(config: FuseConfig) -> Self {
        Self { config, chunks: HashMap::new(), revision: 0, samples: 0 }
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.revision += 1;
        self.samples = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The cell holding ground point (x, z).
    pub fn cell_of(&self, x: f32, z: f32) -> IVec2 {
        IVec2::new((x / self.config.cell).floor() as i32, (z / self.config.cell).floor() as i32)
    }

    /// The centre of a cell, ground coordinates.
    pub fn centre(&self, c: IVec2) -> Vec2 {
        (c.as_vec2() + 0.5) * self.config.cell
    }

    fn split(c: IVec2) -> (IVec2, usize) {
        let k = IVec2::new(c.x.div_euclid(CHUNK), c.y.div_euclid(CHUNK));
        let l = IVec2::new(c.x.rem_euclid(CHUNK), c.y.rem_euclid(CHUNK));
        (k, (l.y * CHUNK + l.x) as usize)
    }

    fn cell_mut(&mut self, c: IVec2) -> &mut Cell {
        let (k, i) = Self::split(c);
        &mut self.chunks.entry(k).or_insert_with(|| vec![Cell::default(); (CHUNK * CHUNK) as usize].into())[i]
    }

    fn cell_ref(&self, c: IVec2) -> Option<&Cell> {
        let (k, i) = Self::split(c);
        self.chunks.get(&k).map(|ch| &ch[i])
    }

    /// One surface point.
    pub fn add_point(&mut self, p: Vec3, w: f32) {
        let gap = self.config.gap;
        let c = self.cell_of(p.x, p.z);
        self.cell_mut(c).add(p.y, w, gap);
        self.samples += 1;
        self.revision += 1;
    }

    /// One depth ray from `origin` (the eye) to the surface point `hit`: carves the cells it passes
    /// over, then adds the hit.
    pub fn add_ray(&mut self, origin: Vec3, hit: Vec3, w: f32) {
        let ground = Vec2::new(hit.x - origin.x, hit.z - origin.z);
        let len = ground.length();
        let stop = len - self.config.carve_stop;
        if stop > 0.0 && self.config.carve > 0.0 {
            let dir = ground / len;
            let cell = self.config.cell;
            let (margin, amount) = (self.config.carve_margin, self.config.carve * w);
            // March half a cell at a time (cells the ray only clips a corner of aren't worth it).
            let steps = (stop / (0.5 * cell)) as usize;
            let mut last = IVec2::new(i32::MIN, i32::MIN);
            for s in 0..=steps.min(2000) {
                let d = s as f32 * 0.5 * cell;
                let p = Vec2::new(origin.x, origin.z) + dir * d;
                let c = self.cell_of(p.x, p.y);
                if c == last {
                    continue;
                }
                last = c;
                // The ray's lowest height over this cell.
                let h = origin.y + (hit.y - origin.y) * ((d + 0.5 * cell) / len).min(1.0);
                if let Some(cell) = self.cell_mut_existing(c) {
                    cell.carve(h, margin, amount);
                }
            }
        }
        self.add_point(hit, w);
    }

    fn cell_mut_existing(&mut self, c: IVec2) -> Option<&mut Cell> {
        let (k, i) = Self::split(c);
        self.chunks.get_mut(&k).map(|ch| &mut ch[i])
    }

    /// A scanned triangle (a room mesh from the headset's own scan), sampled at the cell spacing.
    /// Ceilings and undersides are skipped.
    pub fn add_triangle(&mut self, [a, b, c]: [Vec3; 3], w: f32) {
        let n = (b - a).cross(c - a);
        let len = n.length();
        if len < 1e-9 || n.y / len < -0.2 {
            return;
        }
        let step = 0.5 * self.config.cell;
        let (ab, ac) = ((b - a).length(), (c - a).length());
        let (nu, nv) = ((ab / step).ceil().max(1.0) as usize, (ac / step).ceil().max(1.0) as usize);
        for i in 0..=nu {
            for j in 0..=nv {
                let (u, v) = (i as f32 / nu as f32, j as f32 / nv as f32);
                if u + v > 1.0 + 1e-6 {
                    continue;
                }
                self.add_point(a + (b - a) * u + (c - a) * v, w);
            }
        }
    }

    /// The surface height of every cell with at least `min_w` support (NaN elsewhere).
    pub fn heightmap(&self, min_w: f32) -> HeightMap {
        let cell = self.config.cell;
        if self.chunks.is_empty() {
            return HeightMap::empty(cell);
        }
        let (mut lo, mut hi) = (IVec2::MAX, IVec2::MIN);
        for k in self.chunks.keys() {
            lo = lo.min(*k);
            hi = hi.max(*k);
        }
        let first = lo * CHUNK;
        let size = (hi - lo + 1) * CHUNK;
        let mut map = HeightMap {
            first,
            cell,
            width: size.x as usize,
            depth: size.y as usize,
            z: vec![f32::NAN; (size.x * size.y) as usize],
            weight: vec![0.0; (size.x * size.y) as usize],
        };
        for (k, ch) in &self.chunks {
            for (i, c) in ch.iter().enumerate() {
                let Some((h, w)) = c.height(min_w, self.config.gap) else { continue };
                let g = *k * CHUNK + IVec2::new(i as i32 % CHUNK, i as i32 / CHUNK) - first;
                let at = g.y as usize * map.width + g.x as usize;
                map.z[at] = h;
                map.weight[at] = w;
            }
        }
        map
    }

    /// The surface at one cell (for live feedback).
    pub fn height_at(&self, c: IVec2, min_w: f32) -> Option<f32> {
        self.cell_ref(c).and_then(|c| c.height(min_w, self.config.gap)).map(|(h, _)| h)
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stray_sample_does_not_move_a_supported_surface() {
        let mut c = Cell::default();
        for i in 0..20 {
            c.add(0.5 + 0.01 * ((i % 3) as f32 - 1.0), 1.0, 0.08);
        }
        c.add(1.7, 1.0, 0.08);
        let (h, _) = c.height(2.0, 0.08).unwrap();
        assert!((h - 0.5).abs() < 0.01, "{h}");
    }

    #[test]
    fn the_top_of_a_wall_face_wins_over_its_side() {
        // A ledge 0.4 m up: its face is seen at many heights, its top often.
        let mut c = Cell::default();
        for i in 0..10 {
            c.add(i as f32 * 0.04, 1.0, 0.08);
        }
        for _ in 0..15 {
            c.add(0.4, 1.0, 0.08);
        }
        let (h, _) = c.height(2.0, 0.08).unwrap();
        assert!((h - 0.4).abs() < 0.04, "{h}");
    }

    #[test]
    fn rays_over_a_cell_carve_away_what_moved() {
        let mut f = Fusion::new(FuseConfig::default());
        // Someone stood at (2, 0) (1.7 m tall), then left: the ground there is seen past them.
        for _ in 0..6 {
            f.add_point(Vec3::new(2.05, 1.7, 0.05), 1.0);
        }
        for _ in 0..6 {
            f.add_point(Vec3::new(2.05, 0.0, 0.05), 1.0);
        }
        let c = f.cell_of(2.05, 0.05);
        assert!(f.height_at(c, 1.0).unwrap() > 1.0);
        for _ in 0..30 {
            f.add_ray(Vec3::new(0.0, 1.6, 0.05), Vec3::new(4.0, 0.0, 0.05), 1.0);
        }
        assert!(f.height_at(c, 1.0).unwrap().abs() < 0.05);
    }

    #[test]
    fn triangles_cover_their_cells() {
        let mut f = Fusion::new(FuseConfig::default());
        f.add_triangle([Vec3::new(0.0, 0.3, 0.0), Vec3::new(0.0, 0.3, 1.0), Vec3::new(1.0, 0.3, 0.0)], 1.0);
        let m = f.heightmap(1.0);
        let filled = m.z.iter().filter(|z| !z.is_nan()).count();
        assert!((45..=70).contains(&filled), "{filled}");
        assert!(m.z.iter().filter(|z| !z.is_nan()).all(|z| (*z - 0.3).abs() < 1e-5));
    }
}
