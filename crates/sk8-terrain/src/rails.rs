//! Grindable edges found in the surface: the lip of a flat top above a real drop (ledges, box
//! edges, quarter-pipe copings, bank tops).
use crate::smooth::Surface;
use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct RailConfig {
    /// A top steeper than this (rise over run) is a slope, not a ledge top.
    pub max_slope: f32,
    /// The drop off the edge, metres (stair risers and curbs stay out under this).
    pub min_drop: f32,
    pub min_length: f32,
    /// Douglas-Peucker tolerance, metres.
    pub simplify: f32,
    /// A rail splits where it turns more than this.
    pub corner_deg: f32,
}

impl Default for RailConfig {
    fn default() -> Self {
        Self { max_slope: 0.2, min_drop: 0.2, min_length: 0.6, simplify: 0.03, corner_deg: 50.0 }
    }
}

const FOUR: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
const EIGHT: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

pub fn edge_rails(s: &Surface, cfg: &RailConfig) -> Vec<Vec<Vec3>> {
    let m = &s.map;
    let (w, d) = (m.width as i32, m.depth as i32);
    // Each edge cell's rail point: on the cell's edge toward the drop, at the top's height.
    let mut point: Vec<Option<Vec3>> = vec![None; m.z.len()];
    for j in 0..d {
        for i in 0..w {
            let at = (j * w + i) as usize;
            let h = m.z[at];
            if h.is_nan() || s.slope[at].length() > cfg.max_slope {
                continue;
            }
            let toward: Vec2 = FOUR
                .iter()
                .filter(|(dx, dz)| {
                    let n = m.get(i + dx, j + dz);
                    !n.is_nan() && n < h - cfg.min_drop
                })
                .map(|&(dx, dz)| Vec2::new(dx as f32, dz as f32))
                .sum();
            if toward == Vec2::ZERO {
                continue;
            }
            let c = m.centre(i as usize, j as usize) + toward.normalize() * 0.5 * m.cell;
            point[at] = Some(Vec3::new(c.x, h, c.y));
        }
    }
    // Trace chains of edge cells, starting from the ends (fewest unused neighbours).
    let mut used = vec![false; m.z.len()];
    let free = |used: &Vec<bool>, i: i32, j: i32| -> Vec<(i32, i32)> {
        EIGHT
            .iter()
            .map(|&(dx, dz)| (i + dx, j + dz))
            .filter(|&(x, z)| x >= 0 && z >= 0 && x < w && z < d)
            .filter(|&(x, z)| {
                let at = (z * w + x) as usize;
                point[at].is_some() && !used[at]
            })
            .collect()
    };
    let mut rails = Vec::new();
    loop {
        let start = (0..m.z.len())
            .filter(|&at| point[at].is_some() && !used[at])
            .min_by_key(|&at| free(&used, at as i32 % w, at as i32 / w).len());
        let Some(start) = start else { break };
        let (mut i, mut j) = (start as i32 % w, start as i32 / w);
        let mut chain = Vec::new();
        let mut dir = (0, 0);
        loop {
            let at = (j * w + i) as usize;
            used[at] = true;
            chain.push(point[at].unwrap());
            let next = free(&used, i, j);
            // Keep straight on where there's a choice; 4-neighbours before diagonals.
            let Some(&(x, z)) = next.iter().max_by_key(|&&(x, z)| {
                let step = (x - i, z - j);
                (step.0 * dir.0 + step.1 * dir.1) * 2 - (step.0.abs() + step.1.abs())
            }) else {
                break;
            };
            dir = (x - i, z - j);
            // Mark the other neighbours so a two-cell-thick edge doesn't fork.
            for &(ox, oz) in &next {
                if (ox, oz) != (x, z) && (ox - x).abs() <= 1 && (oz - z).abs() <= 1 {
                    used[(oz * w + ox) as usize] = true;
                }
            }
            (i, j) = (x, z);
        }
        for part in split_corners(&simplify(&smooth_heights(&chain), cfg.simplify), cfg.corner_deg) {
            let len: f32 = part.windows(2).map(|p| p[0].distance(p[1])).sum();
            if part.len() >= 2 && len >= cfg.min_length {
                rails.push(part);
            }
        }
    }
    rails
}

fn smooth_heights(chain: &[Vec3]) -> Vec<Vec3> {
    (0..chain.len())
        .map(|k| {
            let (a, b) = (k.saturating_sub(2), (k + 3).min(chain.len()));
            let y = chain[a..b].iter().map(|p| p.y).sum::<f32>() / (b - a) as f32;
            Vec3::new(chain[k].x, y, chain[k].z)
        })
        .collect()
}

fn simplify(p: &[Vec3], tol: f32) -> Vec<Vec3> {
    if p.len() < 3 {
        return p.to_vec();
    }
    let (a, b) = (p[0], p[p.len() - 1]);
    let ab = b - a;
    let far = (1..p.len() - 1)
        .map(|k| {
            let t = if ab.length_squared() > 1e-12 { ((p[k] - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            (k, p[k].distance(a + ab * t))
        })
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap();
    if far.1 <= tol {
        return vec![a, b];
    }
    let mut left = simplify(&p[..=far.0], tol);
    left.pop();
    left.extend(simplify(&p[far.0..], tol));
    left
}

/// Splits where the rail turns a corner. A corner on the grid is cut across a cell (two half
/// turns either side of a short diagonal), so a short segment is judged by the turn across it and
/// dropped.
fn split_corners(p: &[Vec3], corner_deg: f32) -> Vec<Vec<Vec3>> {
    const SHORT: f32 = 0.2;
    let dir = |k: usize| (p[k + 1] - p[k]).normalize_or_zero();
    let turn = |a: Vec3, b: Vec3| a.angle_between(b).to_degrees();
    let mut out = vec![vec![]];
    let segs = p.len().saturating_sub(1);
    let mut k = 0;
    while k < segs {
        let part = out.last_mut().unwrap();
        if part.is_empty() {
            part.push(p[k]);
        }
        let short = p[k].distance(p[k + 1]) < SHORT;
        if short && k > 0 && k + 1 < segs && turn(dir(k - 1), dir(k + 1)) > corner_deg {
            out.push(vec![]);
            k += 1;
            continue;
        }
        part.push(p[k + 1]);
        if k + 1 < segs && turn(dir(k), dir(k + 1)) > corner_deg {
            out.push(vec![]);
        }
        k += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{smooth, HeightMap, SmoothConfig};
    use glam::IVec2;

    #[test]
    fn a_box_ledge_gives_its_four_edges() {
        // A 2 m x 0.6 m box, 0.45 m tall, on flat ground.
        let (w, d, cell) = (40usize, 30usize, 0.1);
        let mut z = vec![0.0; w * d];
        for j in 12..18 {
            for i in 10..30 {
                z[j * w + i] = 0.45;
            }
        }
        let map = HeightMap { first: IVec2::ZERO, cell, width: w, depth: d, weight: vec![1.0; w * d], z };
        let s = smooth(&map, &SmoothConfig::default());
        let rails = edge_rails(&s, &RailConfig::default());
        let long: Vec<f32> = rails.iter().map(|r| r.windows(2).map(|p| p[0].distance(p[1])).sum()).filter(|&l| l > 1.5).collect();
        assert_eq!(long.len(), 2, "two long sides: {rails:?}");
        for l in long {
            assert!((l - 2.0).abs() < 0.25, "{l}");
        }
        assert!(rails.iter().flatten().all(|p| (p.y - 0.45).abs() < 0.02));
    }
}
