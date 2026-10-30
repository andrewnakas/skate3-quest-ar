//! Scan a park in simulation and ride it: how well does a noisy headset scan, fused and smoothed,
//! skate compared with the park itself?
//!
//! `cargo run -p sk8-terrain --release --example scan_ride -- <park.txt> [out-dir]`
//!
//! `park.txt` is a triangle dump ("t ax ay az bx by bz cx cy cz" lines, as skatecore's `mapdump`
//! or `SK8_MESH` use). A headset walks the park in lanes at eye height, looking about, and casts
//! depth rays with the Quest's kind of noise (growing with the square of the range), outliers and
//! tracking wobble. The rays are fused, and the result is built two ways: raw (fused heights,
//! flat-shaded: like riding the headset's own lumpy mesh) and smoothed (the game's terrain). Then
//! the same push-and-roll lines are ridden on the park, the raw scan and the smoothed scan, and
//! the bumps are counted. With an out dir, both scans are written as `SK8_MESH` files.
use glam::Vec3;
use sk8_terrain::{build_from, BuildConfig, Collider, FuseConfig, Fusion, TerrainMesh};
use skate_lite_sys::{buttons, Collision, Hit, Input, Mode, Rider, World};
use std::sync::Arc;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn normal(&mut self) -> f32 {
        let (u, v) = (self.next().max(1e-7), self.next());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

fn load(path: &str) -> Vec<[Vec3; 3]> {
    let text = std::fs::read_to_string(path).expect("park file");
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == "t").then_some(())?;
            let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
            (v.len() >= 9).then(|| [Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]), Vec3::new(v[6], v[7], v[8])])
        })
        .collect()
}

/// A long cast in short steps (each step only looks in the buckets it crosses).
fn cast_far(c: &Collider, origin: Vec3, delta: Vec3) -> Option<(f32, Vec3)> {
    let steps = (delta.length() / 0.4).ceil().max(1.0) as usize;
    for k in 0..steps {
        let (a, b) = (k as f32 / steps as f32, (k + 1) as f32 / steps as f32);
        if let Some((f, n)) = c.cast(origin + delta * a, delta * (b - a)) {
            return Some((a + f * (b - a), n));
        }
    }
    None
}

fn flat(tris: &[[Vec3; 3]]) -> TerrainMesh {
    let normals = tris
        .iter()
        .map(|t| {
            let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
            [n; 3]
        })
        .collect();
    TerrainMesh { triangles: tris.to_vec(), normals }
}

/// A collider plus a floor under everything (the tracking floor, under unscanned ground).
struct Ride {
    c: Arc<Collider>,
    floor: f32,
}

impl Collision for Ride {
    fn raycast(&self, origin: [f32; 3], delta: [f32; 3]) -> Option<Hit> {
        let (o, d) = (Vec3::from_array(origin), Vec3::from_array(delta));
        let mut best = self.c.cast(o, d);
        if d.y != 0.0 {
            let f = (self.floor - o.y) / d.y;
            if (0.0..=1.0).contains(&f) && best.is_none_or(|(b, _)| f < b) {
                best = Some((f, if d.y < 0.0 { Vec3::Y } else { -Vec3::Y }));
            }
        }
        best.map(|(f, n)| Hit { point: (o + d * f).to_array(), normal: n.to_array(), fraction: f, surface: 0 })
    }
}

#[derive(Default, Debug, Clone, Copy)]
struct Score {
    /// Grounded ticks.
    ticks: u32,
    /// Times the board left the ground with no pop (a catch or a bump throw).
    hops: u32,
    bails: u32,
    /// RMS change in vertical speed per tick while grounded (m/s): rattle.
    rattle: f32,
    /// RMS turn of the board's up vector per tick while grounded (degrees).
    wobble: f32,
}

fn ride(collider: Arc<Collider>, floor: f32, start: Vec3, facing: f32, trace: bool) -> (Score, Vec<Vec3>) {
    let mut world = World::new(Box::new(Ride { c: collider, floor }));
    let mut rider = Rider::new();
    rider.place(start.to_array(), facing, 1.0);
    let mut s = Score::default();
    let (mut sum_dv, mut sum_up) = (0.0f64, 0.0f64);
    let mut last: Option<(Mode, f32, Vec3)> = None;
    let mut path = Vec::new();
    for t in 0..60 * 9 {
        // Push for two seconds (taps), then roll.
        let push = t < 120 && (t % 30) < 10;
        let input = Input { buttons: if push { buttons::PUSH } else { 0 }, ..Default::default() };
        rider.step(&mut world, &input);
        let st = rider.state();
        let up = Vec3::from_array(st.up);
        path.push(Vec3::from_array(st.position));
        if let Some((m, vy, lup)) = last {
            if m.grounded() && st.mode.grounded() {
                s.ticks += 1;
                sum_dv += ((st.velocity[1] - vy) as f64).powi(2);
                sum_up += (lup.angle_between(up).to_degrees() as f64).powi(2);
            }
            if m.grounded() && st.mode == Mode::Air {
                s.hops += 1;
            }
            if m != Mode::Bail && st.mode == Mode::Bail {
                s.bails += 1;
                if std::env::var("SCAN_DEBUG").is_ok() {
                    eprintln!("  bail at t {t} {:?} from {m:?}, speed {:.2}, up {up:.2}", st.position, st.speed);
                }
            }
        }
        if trace && t % 4 == 0 && t <= 140 {
            eprintln!("  t {t} {:?} p {:.2?} v {:.2?} up {up:.2}", st.mode, st.position, st.velocity);
        }
        last = Some((st.mode, st.velocity[1], up));
    }
    if s.ticks > 0 {
        s.rattle = (sum_dv / s.ticks as f64).sqrt() as f32;
        s.wobble = (sum_up / s.ticks as f64).sqrt() as f32;
    }
    (s, path)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let park = load(args.get(1).expect("usage: scan_ride <park.txt> [out-dir]"));
    let out = args.get(2);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in park.iter().flatten() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let floor = lo.y;
    println!("park: {} triangles, {lo:.2} .. {hi:.2}", park.len());
    let truth = Arc::new(Collider::new(&flat(&park)));

    // Walk it in lanes 3 m apart, eye 1.6 m above the floor, a depth frame every 0.2 s at 1 m/s.
    let mut rng = Rng(0x5eed_1234_abcd);
    let mut fusion = Fusion::new(FuseConfig::default());
    let started = std::time::Instant::now();
    let (mut rays, mut frames) = (0u64, 0u32);
    let mut drift = Vec3::ZERO;
    let mut lane = 0;
    let mut z = lo.z + 1.0;
    while z < hi.z {
        let dir = if lane % 2 == 0 { 1.0 } else { -1.0 };
        let mut x = if dir > 0.0 { lo.x + 0.5 } else { hi.x - 0.5 };
        while (dir > 0.0 && x < hi.x) || (dir < 0.0 && x > lo.x) {
            // Standing on whatever is under foot.
            let ground = truth.cast(Vec3::new(x, hi.y + 5.0, z), Vec3::new(0.0, -(hi.y - lo.y) - 10.0, 0.0));
            let feet = ground.map_or(floor, |(f, _)| hi.y + 5.0 - f * (hi.y - lo.y + 10.0));
            drift = drift * 0.98 + Vec3::new(rng.normal(), rng.normal() * 0.5, rng.normal()) * 0.004;
            let eye = Vec3::new(x, feet + 1.6, z);
            // Looking along the lane, the head swinging side to side.
            let yaw = if dir > 0.0 { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 }
                + 0.9 * (frames as f32 * 0.35).sin();
            let pitch = -0.6 + 0.3 * (frames as f32 * 0.23).sin();
            for v in 0..24 {
                for u in 0..32 {
                    let (ay, ap) = (yaw + (u as f32 / 31.0 - 0.5) * 1.4, pitch + (v as f32 / 23.0 - 0.5) * 1.1);
                    let d = Vec3::new(ay.sin() * ap.cos(), ap.sin(), ay.cos() * ap.cos());
                    let Some((f, _)) = cast_far(&truth, eye, d * 5.0) else { continue };
                    let range = f * 5.0;
                    let mut r = range + rng.normal() * (0.004 + 0.005 * range * range);
                    if rng.next() < 0.006 {
                        r = range * (0.5 + 0.7 * rng.next());
                    }
                    let seen_from = eye + drift;
                    fusion.add_ray(seen_from, seen_from + d * r, 1.0);
                    rays += 1;
                }
            }
            frames += 1;
            x += dir * 0.2;
        }
        z += 3.0;
        lane += 1;
    }
    println!(
        "scan: {frames} frames, {rays} rays, {} chunks ({:.1} s)",
        fusion.chunk_count(),
        started.elapsed().as_secs_f32()
    );

    let cfg = BuildConfig::default();
    let t = std::time::Instant::now();
    let raw_map = fusion.heightmap(cfg.min_weight);
    let terrain = build_from(&raw_map, &cfg);
    println!(
        "terrain: {} of {} cells, {} triangles, {} rails ({:.2} s)",
        terrain.surface.map.filled(),
        raw_map.z.len(),
        terrain.mesh.triangles.len(),
        terrain.rails.len(),
        t.elapsed().as_secs_f32()
    );
    // The raw scan: the fused heights as they are, flat-shaded.
    let raw_surface = sk8_terrain::Surface { slope: vec![glam::Vec2::ZERO; raw_map.z.len()], map: raw_map.clone() };
    let raw_mesh = flat(&sk8_terrain::mesh(&raw_surface, 0.0).triangles);
    // Height error against the park, over cells the park has ground under.
    let mut errs = Vec::new();
    for j in 0..terrain.surface.map.depth {
        for i in 0..terrain.surface.map.width {
            let h = terrain.surface.map.z[j * terrain.surface.map.width + i];
            if h.is_nan() {
                continue;
            }
            let c = terrain.surface.map.centre(i, j);
            let top = hi.y + 5.0;
            if let Some((f, n)) = truth.cast(Vec3::new(c.x, top, c.y), Vec3::new(0.0, -(top - floor) - 1.0, 0.0)) {
                if n.y > 0.7 {
                    errs.push((h - (top - f * (top - floor + 1.0))).abs());
                }
            }
        }
    }
    if let Ok(row) = std::env::var("SCAN_ROW") {
        // "z x0 x1": truth, raw fused and smoothed heights along a row of cells.
        let v: Vec<f32> = row.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        let m = &terrain.surface.map;
        let j = ((v[0] / m.cell).floor() as i32 - m.first.y) as usize;
        for i in 0..m.width {
            let c = m.centre(i, j);
            if c.x < v[1] || c.x > v[2] {
                continue;
            }
            let top = hi.y + 5.0;
            let t = truth.cast(Vec3::new(c.x, top, c.y), Vec3::new(0.0, -(top - floor) - 1.0, 0.0)).map(|(f, _)| top - f * (top - floor + 1.0));
            let at = j * m.width + i;
            eprintln!("x {:.2} truth {:.3} raw {:.3} smooth {:.3} normal {:.2}", c.x, t.unwrap_or(f32::NAN), raw_map.z[at], m.z[at], terrain.surface.normal(at));
        }
    }
    errs.sort_by(f32::total_cmp);
    if !errs.is_empty() {
        let q = |p: f32| errs[((errs.len() - 1) as f32 * p) as usize];
        println!("smoothed height error (m, ground under 45 deg): median {:.3} p90 {:.3} p99 {:.3}", q(0.5), q(0.9), q(0.99));
    }

    if let Some(dir) = out {
        let write = |name: &str, m: &TerrainMesh, rails: &[Vec<Vec3>]| {
            use std::fmt::Write;
            let mut s = String::new();
            for t in &m.triangles {
                let _ = writeln!(s, "t {} {} {} {} {} {} {} {} {}", t[0].x, t[0].y, t[0].z, t[1].x, t[1].y, t[1].z, t[2].x, t[2].y, t[2].z);
            }
            for r in rails {
                let _ = write!(s, "r {}", r.len());
                for p in r {
                    let _ = write!(s, " {} {} {}", p.x, p.y, p.z);
                }
                s.push('\n');
            }
            let path = std::path::Path::new(dir).join(name);
            std::fs::write(&path, s).unwrap();
            println!("wrote {}", path.display());
        };
        write("scan_raw.txt", &raw_mesh, &[]);
        write("scan_smooth.txt", &terrain.mesh, &terrain.rails);
    }

    let worlds = [
        ("park", truth.clone()),
        ("raw scan", Arc::new(Collider::new(&raw_mesh))),
        ("smoothed", Arc::new(Collider::new(&terrain.mesh))),
    ];
    // Lines: from points on open floor, every 20 degrees round.
    let mut starts = Vec::new();
    let mut rng = Rng(77);
    while starts.len() < 24 {
        let p = Vec3::new(lo.x + rng.next() * (hi.x - lo.x), 0.0, lo.z + rng.next() * (hi.z - lo.z));
        let top = hi.y + 5.0;
        if let Some((f, n)) = truth.cast(Vec3::new(p.x, top, p.z), Vec3::new(0.0, -(top - floor) - 1.0, 0.0)) {
            let y = top - f * (top - floor + 1.0);
            if n.y > 0.99 && y < floor + 0.3 {
                starts.push((Vec3::new(p.x, y + 0.05, p.z), rng.next() * std::f32::consts::TAU));
            }
        }
    }
    println!("\n{:>9} | {:>6} {:>5} {:>5} {:>11} {:>11} {:>8}", "world", "ticks", "hops", "bails", "rattle m/s", "wobble deg", "vs park");
    let mut park_paths: Vec<Vec<Vec3>> = Vec::new();
    for (name, c) in &worlds {
        let mut total = Score::default();
        let (mut r2, mut w2, mut dev) = (0.0f64, 0.0f64, 0.0f32);
        for (k, &(p, f)) in starts.iter().enumerate() {
            let trace = std::env::var("SCAN_TRACE").ok().and_then(|v| v.parse::<usize>().ok()) == Some(k);
            if trace { eprintln!("{name}:"); }
            let (s, path) = ride(c.clone(), floor, p, f, trace);
            if std::env::var("SCAN_DEBUG").is_ok() && (s.bails > 0 || s.hops > 0) {
                eprintln!("{name} line {k} from {p:.2} facing {f:.2}: {s:?}");
            }
            total.ticks += s.ticks;
            total.hops += s.hops;
            total.bails += s.bails;
            r2 += (s.rattle as f64).powi(2) * s.ticks as f64;
            w2 += (s.wobble as f64).powi(2) * s.ticks as f64;
            if *name == "park" {
                park_paths.push(path);
            } else {
                // How far the line ends from the same line on the park.
                dev += path.last().unwrap().distance(*park_paths[k].last().unwrap());
            }
        }
        let n = total.ticks.max(1) as f64;
        println!(
            "{:>9} | {:>6} {:>5} {:>5} {:>11.4} {:>11.3} {:>7.2}m",
            name,
            total.ticks,
            total.hops,
            total.bails,
            (r2 / n).sqrt(),
            (w2 / n).sqrt(),
            dev / starts.len() as f32
        );
    }
}
