//! Runs the clean-room engine: fixed 1/60 s steps from frame time (scaled for
//! slow motion, held for pause, one at a time for frame step), collision from
//! the AR world or the desktop test park, teleports, and the pose everything
//! else reads.
use super::collide::Grid;
#[cfg(feature = "xr")]
use super::collide::{SURFACE_GROUND, SURFACE_METAL, SURFACE_WOOD};
use bevy::prelude::*;
#[cfg(feature = "xr")]
use skate_lite_sys::Mode;
use skate_lite_sys::{DT, Event, EventKind, Input, Rider, State, World};

/// Never run more than this many steps in one frame (a hitch would otherwise
/// spiral).
const MAX_STEPS_PER_FRAME: u32 = 6;

#[derive(Resource)]
pub(crate) struct CleanSkate {
    pub world: World,
    pub rider: Rider,
    /// Input for the next steps (written by `input`).
    pub input: Input,
    /// 1 = real time; the mod menu's game speed.
    pub time_scale: f32,
    pub paused: bool,
    /// Steps to run while paused (frame step).
    pub step_once: u32,
    accum: f32,
    pub previous: State,
    pub current: State,
    /// The scripted demo's own rail (drawn as a line; it has no mesh).
    pub demo_rail: Option<[Vec3; 2]>,
    /// Every event of the steps run this frame.
    pub events: Vec<Event>,
    /// Where the rider was last put down (for "reset").
    pub spawn: ([f32; 3], f32),
    pub collision_triangles: usize,
    /// The world's surfaces, kept for the camera (it pulls in rather than look through a wall).
    pub camera_grid: Option<super::collide::Grid>,
    /// Placed by the host since the last step (sent on as a Respawn event).
    placed: bool,
}

impl CleanSkate {
    pub(crate) fn new() -> Self {
        let mut rider = Rider::new();
        // SK8_PARAMS=key=value,key=value: engine values for this run (tuning, A/B traces)
        if let Ok(list) = std::env::var("SK8_PARAMS") {
            let params = skate_lite_sys::params();
            for kv in list.split(',').filter(|kv| !kv.is_empty()) {
                let found = kv.split_once('=').and_then(|(k, v)| {
                    let p = params.iter().find(|p| p.key == k)?;
                    Some((p.index, v.parse::<f32>().ok()?))
                });
                match found {
                    Some((i, v)) => rider.set_param(i, v),
                    None => warn!("SK8_PARAMS: unknown or bad {kv}"),
                }
            }
        }
        rider.place([0.0, 0.0, 0.0], 0.0, 0.0);
        let state = rider.state();
        Self {
            world: World::empty(),
            rider,
            input: Input::default(),
            time_scale: 1.0,
            paused: false,
            step_once: 0,
            accum: 0.0,
            previous: state,
            current: state,
            demo_rail: None,
            events: Vec::new(),
            spawn: ([0.0; 3], 0.0),
            collision_triangles: 0,
            camera_grid: None,
            placed: false,
        }
    }

    /// 0..1 between `previous` and `current`, for smooth rendering.
    pub(crate) fn alpha(&self) -> f32 {
        (self.accum / DT).clamp(0.0, 1.0)
    }

    pub(crate) fn place(&mut self, position: Vec3, facing: f32) {
        self.place_rolling(position, facing, 0.0);
    }

    pub(crate) fn place_rolling(&mut self, position: Vec3, facing: f32, speed: f32) {
        self.rider.place(position.to_array(), facing, speed);
        self.spawn = (position.to_array(), facing);
        self.current = self.rider.state();
        self.previous = self.current;
        self.placed = true;
    }

    pub(crate) fn set_collision(&mut self, grid: Grid, rails: &[Vec<Vec3>]) {
        self.collision_triangles = grid.len();
        self.camera_grid = Some(grid.clone());
        self.world.set_collision(Box::new(grid));
        self.world.clear_rails();
        for rail in rails {
            let points: Vec<[f32; 3]> = rail.iter().map(|p| p.to_array()).collect();
            self.world.add_rail(&points);
        }
    }

    fn run_steps(&mut self, n: u32) {
        for _ in 0..n {
            self.previous = self.current;
            self.rider.step(&mut self.world, &self.input);
            self.current = self.rider.state();
            while let Some(e) = self.rider.poll_event() {
                self.events.push(e);
            }
        }
    }
}

/// Facing angle (radians about +Y, 0 = +Z) of a direction.
pub(crate) fn facing_of(dir: Vec3) -> f32 {
    let d = dir.with_y(0.0);
    if d.length_squared() < 1e-8 { 0.0 } else { d.x.atan2(d.z) }
}

pub(crate) fn step(time: Res<Time<Real>>, mut sim: ResMut<CleanSkate>) {
    sim.events.clear();
    // put somewhere by the host (spawn, a demo slot, the teleport): a respawn for the animation, so
    // blends cut instead of sweeping the body across from where it was
    if std::mem::take(&mut sim.placed) {
        sim.events.push(Event { kind: EventKind::Respawn, name: String::new(), points: 0, value: 0, surface: 0 });
    }
    let dt = time.delta_secs().min(0.1);
    if sim.paused {
        sim.accum = 0.0;
        let n = std::mem::take(&mut sim.step_once).min(MAX_STEPS_PER_FRAME);
        sim.run_steps(n);
        return;
    }
    sim.accum += dt * sim.time_scale;
    let mut n = 0;
    while sim.accum >= DT && n < MAX_STEPS_PER_FRAME {
        sim.accum -= DT;
        n += 1;
    }
    if n == MAX_STEPS_PER_FRAME {
        sim.accum = sim.accum.min(DT);
    }
    sim.run_steps(n);
    for e in &sim.events {
        match e.kind {
            EventKind::Trick | EventKind::Bank | EventKind::Bail => {
                info!("SK8 {:?} {} {}", e.kind, e.name, e.points)
            }
            _ => {}
        }
    }
}

/// Rebuild collision whenever the AR world changes (not mid-grind).
#[cfg(feature = "xr")]
pub(crate) fn apply_ar_world(
    world: Res<crate::xr::link::ArWorld>,
    mut sim: ResMut<CleanSkate>,
    mut applied: Local<u64>,
) {
    if world.revision == *applied || sim.current.mode == Mode::Grind {
        return;
    }
    *applied = world.revision;
    let mut grid = Grid::default();
    for t in world.room() {
        grid.add(t, SURFACE_GROUND);
    }
    for p in &world.pieces {
        for &q in &p.quads {
            grid.add_quad(q, SURFACE_WOOD);
        }
        for &q in &p.rail_quads {
            grid.add_quad(q, SURFACE_METAL);
        }
    }
    sim.set_collision(grid, &world.rails);
    info!(
        "SK8 collision: {} triangles, {} rails (AR revision {})",
        sim.collision_triangles,
        world.rails.len(),
        world.revision
    );
}

#[cfg(feature = "xr")]
pub(crate) fn teleport(
    mut requests: MessageReader<crate::xr::link::TeleportSkater>,
    mut sim: ResMut<CleanSkate>,
) {
    for r in requests.read() {
        sim.place(r.point + Vec3::Y * 0.05, facing_of(r.forward));
        info!("SK8 placed at {:?}", r.point);
    }
}

#[cfg(feature = "xr")]
pub(crate) fn publish_link(sim: Res<CleanSkate>, mut link: ResMut<crate::xr::link::SkaterLink>) {
    let s = &sim.current;
    let rot = Quat::from_array(s.rider_rot);
    link.deck = Some((Vec3::from_array(s.position), rot * Vec3::Z));
    link.grinding = s.mode == Mode::Grind;
}
