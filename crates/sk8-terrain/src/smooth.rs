//! Turning the raw fused heights into a surface the board rolls over without catching.
//!
//! 1. Specks (cells with almost no neighbours) go, and a 3x3 median takes out spikes.
//! 2. Walls are made walls again: a scan smears a vertical face into a steep ramp a few cells
//!    wide (the face's samples land in the cells either side), so a short run of in-between
//!    heights between two flat plateaus, steeper on average than [`SmoothConfig::wall_slope`],
//!    is snapped to the nearer plateau. Transitions are many cells wide and stay as they are.
//! 3. Small holes are filled from their edges.
//! 4. Each cell is refitted as a robust local quadratic (moving least squares with Tukey weights,
//!    iterated): lumps and depth noise smooth out, a transition's curve is kept (the quadratic
//!    follows it), and a ledge or a coping stays sharp (the far side of the step gets no weight).
//!
//! The fit gives the slope too, so every cell has a smooth normal.
use crate::HeightMap;
use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct SmoothConfig {
    /// Holes this many cells across (from each side) are filled.
    pub fill: usize,
    /// Fit window radius, cells.
    pub radius: i32,
    /// Gaussian falloff of the window, cells.
    pub sigma: f32,
    /// Refits after the first.
    pub iterations: usize,
    /// Residual (metres) past which a neighbour is not the same surface, first pass and later.
    pub tukey_first: f32,
    pub tukey: f32,
    /// Rise over run past which a short run between plateaus is a wall.
    pub wall_slope: f32,
    /// The longest smeared run (cells) a wall is rebuilt from.
    pub wall_run: usize,
}

impl Default for SmoothConfig {
    fn default() -> Self {
        Self { fill: 4, radius: 3, sigma: 1.8, iterations: 2, tukey_first: 0.12, tukey: 0.07, wall_slope: 2.1, wall_run: 3 }
    }
}

/// A smoothed height map: heights plus the slope (dy/dx, dy/dz) of every cell.
#[derive(Clone, Debug)]
pub struct Surface {
    pub map: HeightMap,
    pub slope: Vec<Vec2>,
}

impl Surface {
    /// The smooth normal at cell `i`.
    pub fn normal(&self, i: usize) -> Vec3 {
        let s = self.slope[i];
        Vec3::new(-s.x, 1.0, -s.y).normalize()
    }
}

pub fn smooth(raw: &HeightMap, cfg: &SmoothConfig) -> Surface {
    let mut map = raw.clone();
    despeckle(&mut map);
    median(&mut map);
    sharpen_walls(&mut map, cfg);
    fill(&mut map, cfg.fill);
    let (w, d) = (map.width, map.depth);
    let n = w * d;
    // Pass 0: a slope-limited prediction (the smaller one-sided slope, so a step gives no slope
    // on either side of it); then local quadratics, each predicted by the last.
    let mut coef: Vec<[f32; 6]> = vec![[f32::NAN; 6]; n];
    for j in 0..d {
        for i in 0..w {
            let at = j * w + i;
            let h = map.z[at];
            if h.is_nan() {
                continue;
            }
            let gx = minmod(map.get(i as i32 - 1, j as i32), h, map.get(i as i32 + 1, j as i32));
            let gz = minmod(map.get(i as i32, j as i32 - 1), h, map.get(i as i32, j as i32 + 1));
            coef[at] = [h, gx, gz, 0.0, 0.0, 0.0];
        }
    }
    for pass in 0..=cfg.iterations {
        let c = if pass == 0 { cfg.tukey_first } else { cfg.tukey };
        let quadratic = pass > 0;
        let next: Vec<[f32; 6]> = (0..n)
            .map(|at| {
                if map.z[at].is_nan() {
                    return [f32::NAN; 6];
                }
                fit(&map, &coef, at, cfg, c, quadratic).unwrap_or(coef[at])
            })
            .collect();
        coef = next;
    }
    let cell = map.cell;
    let mut slope = vec![Vec2::ZERO; n];
    for at in 0..n {
        if map.z[at].is_nan() {
            continue;
        }
        map.z[at] = coef[at][0];
        slope[at] = Vec2::new(coef[at][1], coef[at][2]) / cell;
    }
    Surface { map, slope }
}

fn minmod(a: f32, h: f32, b: f32) -> f32 {
    let (l, r) = (h - a, b - h);
    match (l.is_nan(), r.is_nan()) {
        (true, true) => 0.0,
        (true, false) => 0.0,
        (false, true) => 0.0,
        _ if l * r <= 0.0 => 0.0,
        _ => if l.abs() < r.abs() { l } else { r },
    }
}

/// The weighted local fit around cell `at`, in cell units: h = c0 + c1 x + c2 z (+ c3 x^2 + c4 xz
/// + c5 z^2). Neighbours are weighted by distance and by how well the cell's last fit predicts them.
fn fit(map: &HeightMap, coef: &[[f32; 6]], at: usize, cfg: &SmoothConfig, tukey: f32, quadratic: bool) -> Option<[f32; 6]> {
    let (i0, j0) = ((at % map.width) as i32, (at / map.width) as i32);
    let p = coef[at];
    let k = if quadratic { 6 } else { 3 };
    let mut ata = [[0f64; 6]; 6];
    let mut atb = [0f64; 6];
    let mut count = 0;
    let r = cfg.radius;
    // Steep parts get a narrower window: a quadratic can't follow the top of a transition as it
    // goes vertical, and smoothing there would round the lip off.
    let steep = (1.0 + p[1] * p[1] + p[2] * p[2]).sqrt();
    let sigma = cfg.sigma / steep;
    let inv2s2 = 1.0 / (2.0 * sigma * sigma);
    for dz in -r..=r {
        for dx in -r..=r {
            let (x, z) = (dx as f32, dz as f32);
            let d2 = x * x + z * z;
            if d2 > (r * r) as f32 + 0.5 {
                continue;
            }
            let h = map.get(i0 + dx, j0 + dz);
            if h.is_nan() {
                continue;
            }
            let predicted = p[0] + p[1] * x + p[2] * z + p[3] * x * x + p[4] * x * z + p[5] * z * z;
            let res = (h - predicted) / tukey;
            if res.abs() >= 1.0 {
                continue;
            }
            let t = 1.0 - res * res;
            let wgt = ((-d2 * inv2s2).exp() * t * t) as f64;
            let basis = [1.0, x, z, x * x, x * z, z * z].map(|v| v as f64);
            for a in 0..k {
                atb[a] += wgt * basis[a] * h as f64;
                for b in 0..k {
                    ata[a][b] += wgt * basis[a] * basis[b];
                }
            }
            count += 1;
        }
    }
    if count < if quadratic { 9 } else { 4 } {
        return None;
    }
    // A little ridge on the slope and curvature terms keeps one-sided windows (scan edges, the
    // top of a ledge) from extrapolating wildly.
    let ridge = 1e-3 * ata[0][0];
    for a in 1..k {
        ata[a][a] += ridge;
    }
    let sol = solve(&mut ata, &mut atb, k)?;
    let mut out = [0f32; 6];
    for a in 0..k {
        out[a] = sol[a] as f32;
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

fn solve(a: &mut [[f64; 6]; 6], b: &mut [f64; 6], k: usize) -> Option<[f64; 6]> {
    for col in 0..k {
        let piv = (col..k).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for row in col + 1..k {
            let f = a[row][col] / a[col][col];
            for c in col..k {
                a[row][c] -= f * a[col][c];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0f64; 6];
    for row in (0..k).rev() {
        let mut s = b[row];
        for c in row + 1..k {
            s -= a[row][c] * x[c];
        }
        x[row] = s / a[row][row];
    }
    Some(x)
}

/// Rows, then columns: each smeared wall run is snapped to the plateaus either side of it.
fn sharpen_walls(map: &mut HeightMap, cfg: &SmoothConfig) {
    let (w, d) = (map.width, map.depth);
    for line in 0..d {
        let idx: Vec<usize> = (0..w).map(|i| line * w + i).collect();
        sharpen_line(map, &idx, cfg);
    }
    for line in 0..w {
        let idx: Vec<usize> = (0..d).map(|j| j * w + line).collect();
        sharpen_line(map, &idx, cfg);
    }
}

fn sharpen_line(map: &mut HeightMap, idx: &[usize], cfg: &SmoothConfig) {
    const FLAT: f32 = 0.05;
    const MIN_RISE: f32 = 0.2;
    let mut h: Vec<f32> = idx.iter().map(|&i| map.z[i]).collect();
    let len = h.len() as i32;
    let get = |h: &[f32], k: i32| if k < 0 || k >= len { f32::NAN } else { h[k as usize] };
    // The plateau ending at k (k and the cell beyond it, in direction dir, level).
    let plateau = |h: &[f32], k: i32, dir: i32| {
        let (a, b) = (get(h, k), get(h, k + dir));
        (!a.is_nan() && !b.is_nan() && (a - b).abs() < FLAT).then_some(a)
    };
    let mut k = 1;
    while k < len - 1 {
        let Some(lo) = plateau(&h, k - 1, -1) else {
            k += 1;
            continue;
        };
        let mut next = k + 1;
        for run in 1..=cfg.wall_run as i32 {
            let end = k + run - 1;
            let Some(hi) = plateau(&h, end + 1, 1) else { continue };
            let rise = hi - lo;
            if rise.abs() < MIN_RISE || rise.abs() / ((run + 1) as f32 * map.cell) < cfg.wall_slope {
                continue;
            }
            let (min, max) = (lo.min(hi) - FLAT, lo.max(hi) + FLAT);
            let cells = || (k..=end).map(|c| get(&h, c));
            if cells().all(|v| (min..=max).contains(&v)) && cells().any(|v| (v - lo).abs() > FLAT && (v - hi).abs() > FLAT) {
                for c in k..=end {
                    let v = h[c as usize];
                    h[c as usize] = if (v - lo).abs() <= (v - hi).abs() { lo } else { hi };
                }
                next = end + 1;
                break;
            }
        }
        k = next;
    }
    for (&i, v) in idx.iter().zip(h) {
        map.z[i] = v;
    }
}

const NEIGHBOURS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

fn despeckle(map: &mut HeightMap) {
    let keep: Vec<bool> = (0..map.z.len())
        .map(|at| {
            let (i, j) = ((at % map.width) as i32, (at / map.width) as i32);
            !map.z[at].is_nan() && NEIGHBOURS.iter().filter(|(dx, dz)| !map.get(i + dx, j + dz).is_nan()).count() >= 3
        })
        .collect();
    for (z, k) in map.z.iter_mut().zip(keep) {
        if !k {
            *z = f32::NAN;
        }
    }
}

fn median(map: &mut HeightMap) {
    let out: Vec<f32> = (0..map.z.len())
        .map(|at| {
            let h = map.z[at];
            if h.is_nan() {
                return h;
            }
            let (i, j) = ((at % map.width) as i32, (at / map.width) as i32);
            let mut v: Vec<f32> = std::iter::once(h)
                .chain(NEIGHBOURS.iter().map(|(dx, dz)| map.get(i + dx, j + dz)))
                .filter(|z| !z.is_nan())
                .collect();
            if v.len() < 5 {
                return h;
            }
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        })
        .collect();
    map.z = out;
}

/// Grows the surface into holes `rounds` cells deep: each empty cell with at least three filled
/// neighbours takes the lowest of their median and the mean, so a hole beside a ledge fills with
/// the ground, not the ledge top.
fn fill(map: &mut HeightMap, rounds: usize) {
    for _ in 0..rounds {
        let mut changed = Vec::new();
        for at in 0..map.z.len() {
            if !map.z[at].is_nan() {
                continue;
            }
            let (i, j) = ((at % map.width) as i32, (at / map.width) as i32);
            let mut v: Vec<f32> = NEIGHBOURS.iter().map(|(dx, dz)| map.get(i + dx, j + dz)).filter(|z| !z.is_nan()).collect();
            if v.len() < 3 {
                continue;
            }
            v.sort_by(f32::total_cmp);
            let mean = v.iter().sum::<f32>() / v.len() as f32;
            changed.push((at, v[(v.len() - 1) / 2].min(mean)));
        }
        if changed.is_empty() {
            break;
        }
        for (at, h) in changed {
            map.z[at] = h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::IVec2;

    fn map_of(w: usize, d: usize, f: impl Fn(f32, f32) -> f32) -> HeightMap {
        let cell = 0.1;
        let mut z = Vec::with_capacity(w * d);
        for j in 0..d {
            for i in 0..w {
                z.push(f((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell));
            }
        }
        HeightMap { first: IVec2::ZERO, cell, width: w, depth: d, weight: vec![1.0; z.len()], z }
    }

    /// Deterministic noise, -1..1.
    fn noise(x: f32, z: f32) -> f32 {
        let s = ((x * 127.1 + z * 311.7).sin() * 43758.547).rem_euclid(1.0);
        s * 2.0 - 1.0
    }

    /// A 1.5 m radius quarter pipe rising along x from x = 1, 2.5 cm of lumps on it.
    fn qp(x: f32) -> f32 {
        let r = 1.5;
        let t = (x - 1.0).clamp(0.0, r * 0.95);
        r - (r * r - t * t).sqrt()
    }

    #[test]
    fn lumps_smooth_out_and_the_transition_keeps_its_curve() {
        let raw = map_of(40, 30, |x, z| qp(x) + 0.025 * noise(x, z));
        let s = smooth(&raw, &SmoothConfig::default());
        let mut worst: f32 = 0.0;
        for j in 6..24 {
            for i in 4..32 {
                let x = (i as f32 + 0.5) * 0.1;
                // (the steep cells up against the coping, 0.3 m of rise per cell, get more slack)
                let slack = if (22..=24).contains(&i) { 0.5 } else { 1.0 };
                worst = worst.max((s.map.z[j * 40 + i] - qp(x)).abs() * slack);
            }
        }
        assert!(worst < 0.016, "height error {worst}");
        // The normal follows the ramp's true curve up to the coping (x 2.43): no facet kinks.
        let row = 15 * 40;
        for i in 4..23 {
            let x = (i as f32 + 0.5) * 0.1;
            let t = (x - 1.0).clamp(0.0, 1.425);
            let truth = Vec3::new(-t / (1.5f32 * 1.5 - t * t).sqrt(), 1.0, 0.0).normalize();
            let err = s.normal(row + i).angle_between(truth).to_degrees();
            assert!(err < 3.0, "normal {err} deg off at x {x}");
        }
    }

    #[test]
    fn a_ledge_stays_sharp() {
        let ledge = |x: f32| if x > 1.5 { 0.4 } else { 0.0 };
        let raw = map_of(30, 20, |x, z| ledge(x) + 0.015 * noise(x, z));
        let s = smooth(&raw, &SmoothConfig::default());
        for i in 0..30 {
            let x = (i as f32 + 0.5) * 0.1;
            let h = s.map.z[10 * 30 + i];
            assert!((h - ledge(x)).abs() < 0.02, "x {x}: {h}");
            assert!(s.normal(10 * 30 + i).y > 0.98, "flat on both sides of the edge, x {x}");
        }
    }

    #[test]
    fn a_smeared_wall_is_a_wall_again() {
        // A 0.9 m block face at x 1.5, scanned as 0.3 and 0.6 m in the two cells across it.
        let raw = map_of(30, 20, |x, _| match x {
            x if x < 1.4 => 0.0,
            x if x < 1.5 => 0.3,
            x if x < 1.6 => 0.6,
            _ => 0.9,
        });
        let s = smooth(&raw, &SmoothConfig::default());
        for i in 0..30 {
            let h = s.map.z[10 * 30 + i];
            assert!(h.abs() < 0.01 || (h - 0.9).abs() < 0.01, "cell {i}: {h}");
        }
    }

    #[test]
    fn spikes_and_small_holes_go() {
        let mut raw = map_of(20, 20, |_, _| 0.0);
        raw.z[5 * 20 + 5] = 0.8;
        for j in 9..12 {
            for i in 9..12 {
                raw.z[j * 20 + i] = f32::NAN;
            }
        }
        let s = smooth(&raw, &SmoothConfig::default());
        assert!(s.map.z.iter().all(|z| z.abs() < 0.01), "{:?}", s.map.z.iter().fold(0f32, |a, b| a.max(b.abs())));
    }
}
