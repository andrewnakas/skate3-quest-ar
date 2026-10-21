//! Safe Rust wrapper for the clean-room `sk8` skate engine (C API in
//! `vendor/include/sk8.h`, vendored from skatecore-cpp `clean/` by
//! `scripts/cleanroom/sync-lite.sh`).
//!
//! The engine has no data files: the host supplies collision through
//! [`Collision`] and grind rails as polylines, steps the rider at a fixed
//! 1/60 s ([`DT`]) and reads back a [`State`] and [`Event`]s.
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};

/// The engine's fixed step, seconds.
pub const DT: f32 = 1.0 / 60.0;

pub mod buttons {
    pub const PUSH: u32 = 1 << 0;
    pub const BRAKE: u32 = 1 << 1;
    pub const GRAB_LEFT: u32 = 1 << 2;
    pub const GRAB_RIGHT: u32 = 1 << 3;
    pub const MANUAL: u32 = 1 << 4;
    pub const NOSE_MANUAL: u32 = 1 << 5;
    pub const SLIDE: u32 = 1 << 6;
    /// Push with the front foot (mongo).
    pub const PUSH_MONGO: u32 = 1 << 7;
    /// Held while flicking: old school (no comply, boneless, no comply 180).
    pub const FOOT_OFF: u32 = 1 << 8;
    /// Press: step off the board and walk; press again: back on it.
    pub const OFF_BOARD: u32 = 1 << 9;
}

pub mod flags {
    /// Land everything, never fall.
    pub const NEVER_BAIL: u32 = 1 << 0;
    /// The host puts the rider back after a bail (markers).
    pub const NO_RESPAWN: u32 = 1 << 1;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    /// Left stick, x right / y up: carve, air spin, manual balance.
    pub steer: [f32; 2],
    /// Right stick, x right / y up: crouch and flick tricks.
    pub flick: [f32; 2],
    pub buttons: c_uint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ride,
    Manual,
    Slide,
    Air,
    Grind,
    Bail,
    /// Rolling along a wall.
    Wallride,
    /// Off the board, on foot.
    Walk,
}

impl Mode {
    fn from_raw(v: c_int) -> Self {
        match v {
            1 => Mode::Manual,
            2 => Mode::Slide,
            3 => Mode::Air,
            4 => Mode::Grind,
            5 => Mode::Bail,
            6 => Mode::Wallride,
            7 => Mode::Walk,
            _ => Mode::Ride,
        }
    }

    pub fn grounded(self) -> bool {
        matches!(self, Mode::Ride | Mode::Manual | Mode::Slide)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct RawState {
    position: [f32; 3],
    velocity: [f32; 3],
    board_rot: [f32; 4],
    rider_rot: [f32; 4],
    up: [f32; 3],
    speed: f32,
    mode: c_int,
    fakie: c_int,
    crouch: f32,
    lean: f32,
    push_phase: f32,
    grab: c_int,
    flip_progress: f32,
    balance: f32,
    air_time: f32,
    bail_time: f32,
    grind_kind: c_int,
    combo_points: c_uint,
    combo_count: c_uint,
    score: u64,
    manual_dir: c_int,
    grab_style: c_int,
    push_start: f32,
    push_mongo: c_int,
    foot_off: c_int,
    revert: f32,
    manual_style: c_int,
    on_foot: c_int,
}

/// Where in a stroke the pushing foot plants and lifts (`SK8_PUSH_PLANT`, `SK8_PUSH_RELEASE`).
pub const PUSH_PLANT: f32 = 0.18;
pub const PUSH_RELEASE: f32 = 0.62;

/// One step's output. Rotations are quaternions `[x, y, z, w]`.
#[derive(Clone, Copy, Debug)]
pub struct State {
    /// The point under the wheels (or on the rail).
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    /// The deck, flips included.
    pub board_rot: [f32; 4],
    /// The rider's stance frame (no flip or shove).
    pub rider_rot: [f32; 4],
    pub up: [f32; 3],
    pub speed: f32,
    pub mode: Mode,
    pub fakie: bool,
    pub crouch: f32,
    pub lean: f32,
    /// 0..1 through a stroke: the foot is on the ground from [`PUSH_PLANT`] to [`PUSH_RELEASE`].
    pub push_phase: Option<f32>,
    /// 0..1 through the step off the board before the first stroke.
    pub push_start: Option<f32>,
    /// The front foot pushes (mongo).
    pub push_mongo: bool,
    /// The foot-off button is held on the ground (front foot stepped down for an old-school pop).
    pub foot_off: bool,
    /// 0..1 through a revert (the board spun round on landing).
    pub revert: Option<f32>,
    /// In a manual: 0 manual, 1 primo (board on its side), 2 casper (board upside down).
    pub manual_style: u32,
    /// Off the board: walking, or a jump on foot.
    pub on_foot: bool,
    /// Bit 1 left hand, bit 2 right hand.
    pub grab: u32,
    pub flip_progress: Option<f32>,
    pub balance: f32,
    pub air_time: f32,
    /// Seconds since the bail began.
    pub bail_time: Option<f32>,
    /// 0 none, 1 axle, 2 board slide, 3 nose, 4 tail.
    pub grind_kind: u32,
    pub combo_points: u32,
    pub combo_count: u32,
    pub score: u64,
    /// 1 manual (tail down), -1 nose manual, 0 none.
    pub manual_dir: i32,
    /// The grab being held (see [`grab_style`]), 0 when not grabbing.
    pub grab_style: u32,
}

impl From<RawState> for State {
    fn from(r: RawState) -> Self {
        Self {
            position: r.position,
            velocity: r.velocity,
            board_rot: r.board_rot,
            rider_rot: r.rider_rot,
            up: r.up,
            speed: r.speed,
            mode: Mode::from_raw(r.mode),
            fakie: r.fakie != 0,
            crouch: r.crouch,
            lean: r.lean,
            push_phase: (r.push_phase >= 0.0).then_some(r.push_phase),
            push_start: (r.push_start >= 0.0).then_some(r.push_start),
            push_mongo: r.push_mongo != 0,
            foot_off: r.foot_off != 0,
            revert: (r.revert >= 0.0).then_some(r.revert),
            manual_style: r.manual_style.max(0) as u32,
            on_foot: r.on_foot != 0,
            grab: r.grab as u32,
            flip_progress: (r.flip_progress >= 0.0).then_some(r.flip_progress),
            balance: r.balance,
            air_time: r.air_time,
            bail_time: (r.bail_time >= 0.0).then_some(r.bail_time),
            grind_kind: r.grind_kind as u32,
            combo_points: r.combo_points,
            combo_count: r.combo_count,
            score: r.score,
            manual_dir: r.manual_dir,
            grab_style: r.grab_style.max(0) as u32,
        }
    }
}

/// Grab styles (`sk8_grab_style`): the trigger(s) pick the hand, the left stick's direction when
/// the grab starts picks the style.
pub mod grab_style {
    pub const NONE: u32 = 0;
    pub const INDY: u32 = 1;
    pub const CRAIL: u32 = 2;
    pub const TAIL: u32 = 3;
    pub const STALEFISH: u32 = 4;
    pub const MELON: u32 = 5;
    pub const NOSE: u32 = 6;
    pub const TUCK_KNEE: u32 = 7;
    pub const JUDO: u32 = 8;
    pub const DOUBLE: u32 = 9;
    pub const SUPERMAN: u32 = 10;
    pub const NO_FOOT: u32 = 11;
    pub const STIFFY: u32 = 12;
    pub const CHRIST: u32 = 13;
    pub const METHOD: u32 = 14;
    pub const ROCKET: u32 = 15;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Pop,
    Trick,
    Land,
    Bail,
    Grind,
    GrindEnd,
    Bank,
    Push,
    Respawn,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawEvent {
    kind: c_int,
    name: [c_char; 40],
    points: c_uint,
    value: c_int,
    surface: c_int,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: EventKind,
    /// Trick, grind kind or bail reason.
    pub name: String,
    pub points: u32,
    /// Land: 2 clean, 1 sketchy. Grind: the grind kind.
    pub value: i32,
    /// Host surface id of the contact.
    pub surface: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawHit {
    point: [f32; 3],
    normal: [f32; 3],
    fraction: f32,
    surface: c_int,
}

#[repr(C)]
struct RawParam {
    key: *const c_char,
    label: *const c_char,
    group: *const c_char,
    min: f32,
    max: f32,
    def: f32,
    step: f32,
}

/// A tunable value, for menus.
#[derive(Clone, Debug)]
pub struct Param {
    pub index: usize,
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: f32,
}

#[repr(C)]
struct RawWorld {
    _p: [u8; 0],
}
#[repr(C)]
struct RawRider {
    _p: [u8; 0],
}

type RaycastFn = unsafe extern "C" fn(*mut c_void, *const f32, *const f32, *mut RawHit) -> c_int;

unsafe extern "C" {
    fn sk8_world_new(raycast: RaycastFn, user: *mut c_void) -> *mut RawWorld;
    fn sk8_world_delete(world: *mut RawWorld);
    fn sk8_world_add_rail(world: *mut RawWorld, points: *const f32, count: c_int) -> c_int;
    fn sk8_world_clear_rails(world: *mut RawWorld);
    fn sk8_world_rail_count(world: *const RawWorld) -> c_int;

    fn sk8_rider_new() -> *mut RawRider;
    fn sk8_rider_delete(rider: *mut RawRider);
    fn sk8_rider_place(rider: *mut RawRider, position: *const f32, facing: f32, speed: f32);
    fn sk8_rider_step(rider: *mut RawRider, world: *mut RawWorld, input: *const Input);
    fn sk8_rider_state(rider: *const RawRider, state: *mut RawState);
    fn sk8_rider_poll_event(rider: *mut RawRider, event: *mut RawEvent) -> c_int;
    fn sk8_rider_trick(rider: *const RawRider) -> *const c_char;
    fn sk8_rider_bail(rider: *mut RawRider);
    fn sk8_rider_set_flags(rider: *mut RawRider, flags: c_uint);
    fn sk8_rider_flags(rider: *const RawRider) -> c_uint;

    fn sk8_param_count() -> c_int;
    fn sk8_param_at(index: c_int) -> *const RawParam;
    fn sk8_rider_param(rider: *const RawRider, index: c_int) -> f32;
    fn sk8_rider_set_param(rider: *mut RawRider, index: c_int, value: f32);

    fn sk8_flick_classify(xy: *const f32, count: c_int) -> *const c_char;
    fn sk8_trick_count() -> c_int;
    fn sk8_trick_name(index: c_int) -> *const c_char;
    fn sk8_trick_demo_path(index: c_int, xy: *mut f32, max: c_int) -> c_int;
}

/// A collision hit.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub point: [f32; 3],
    /// Unit, facing the ray origin.
    pub normal: [f32; 3],
    /// 0..1 along the ray.
    pub fraction: f32,
    pub surface: i32,
}

/// Host collision: the nearest hit on the segment `origin .. origin + delta`.
pub trait Collision: Send + Sync {
    fn raycast(&self, origin: [f32; 3], delta: [f32; 3]) -> Option<Hit>;
}

struct NoCollision;
impl Collision for NoCollision {
    fn raycast(&self, _: [f32; 3], _: [f32; 3]) -> Option<Hit> {
        None
    }
}

unsafe extern "C" fn raycast_trampoline(
    user: *mut c_void,
    origin: *const f32,
    delta: *const f32,
    hit: *mut RawHit,
) -> c_int {
    // SAFETY: `user` is the `Box<dyn Collision>` owned by the `World` that
    // registered this callback; the engine only calls it during `step`, while
    // the world is mutably borrowed. The pointers are 3-float arrays.
    unsafe {
        let collision = &*(user as *const Box<dyn Collision>);
        let o = *(origin as *const [f32; 3]);
        let d = *(delta as *const [f32; 3]);
        match collision.raycast(o, d) {
            Some(h) => {
                *hit = RawHit {
                    point: h.point,
                    normal: h.normal,
                    fraction: h.fraction,
                    surface: h.surface,
                };
                1
            }
            None => 0,
        }
    }
}

/// Collision and grind rails.
pub struct World {
    raw: *mut RawWorld,
    /// Boxed twice so the pointer handed to C stays put when it is replaced.
    collision: Box<Box<dyn Collision>>,
}

// SAFETY: the engine world has no thread affinity; all access goes through
// `&mut self` or `&self` (read-only rail count).
unsafe impl Send for World {}
unsafe impl Sync for World {}

impl World {
    pub fn new(collision: Box<dyn Collision>) -> Self {
        let collision: Box<Box<dyn Collision>> = Box::new(collision);
        let user = &*collision as *const Box<dyn Collision> as *mut c_void;
        // SAFETY: `user` outlives the world (dropped after it in `Drop`).
        let raw = unsafe { sk8_world_new(raycast_trampoline, user) };
        Self { raw, collision }
    }

    pub fn empty() -> Self {
        Self::new(Box::new(NoCollision))
    }

    pub fn set_collision(&mut self, collision: Box<dyn Collision>) {
        *self.collision = collision;
    }

    pub fn add_rail(&mut self, points: &[[f32; 3]]) -> Option<usize> {
        if points.len() < 2 {
            return None;
        }
        // SAFETY: `points` is `len` contiguous 3-float points.
        let i = unsafe { sk8_world_add_rail(self.raw, points.as_ptr() as *const f32, points.len() as c_int) };
        (i >= 0).then_some(i as usize)
    }

    pub fn clear_rails(&mut self) {
        unsafe { sk8_world_clear_rails(self.raw) }
    }

    pub fn rail_count(&self) -> usize {
        unsafe { sk8_world_rail_count(self.raw) as usize }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        unsafe { sk8_world_delete(self.raw) }
    }
}

/// The skater (board and rider).
pub struct Rider {
    raw: *mut RawRider,
}

// SAFETY: as for `World`.
unsafe impl Send for Rider {}
unsafe impl Sync for Rider {}

impl Default for Rider {
    fn default() -> Self {
        Self::new()
    }
}

impl Rider {
    pub fn new() -> Self {
        Self {
            raw: unsafe { sk8_rider_new() },
        }
    }

    /// Stand on the board at `position`, nose along `facing` (radians about
    /// +Y, 0 = +Z), rolling at `speed`.
    pub fn place(&mut self, position: [f32; 3], facing: f32, speed: f32) {
        unsafe { sk8_rider_place(self.raw, position.as_ptr(), facing, speed) }
    }

    pub fn step(&mut self, world: &mut World, input: &Input) {
        unsafe { sk8_rider_step(self.raw, world.raw, input) }
    }

    pub fn state(&self) -> State {
        let mut s = RawState::default();
        unsafe { sk8_rider_state(self.raw, &mut s) };
        s.into()
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        let mut e = RawEvent {
            kind: 0,
            name: [0; 40],
            points: 0,
            value: 0,
            surface: 0,
        };
        if unsafe { sk8_rider_poll_event(self.raw, &mut e) } == 0 {
            return None;
        }
        let name = unsafe { CStr::from_ptr(e.name.as_ptr()) }.to_string_lossy().into_owned();
        let kind = match e.kind {
            0 => EventKind::Pop,
            1 => EventKind::Trick,
            2 => EventKind::Land,
            3 => EventKind::Bail,
            4 => EventKind::Grind,
            5 => EventKind::GrindEnd,
            6 => EventKind::Bank,
            7 => EventKind::Push,
            _ => EventKind::Respawn,
        };
        Some(Event {
            kind,
            name,
            points: e.points,
            value: e.value,
            surface: e.surface,
        })
    }

    /// The trick in progress, or "".
    pub fn trick(&self) -> String {
        unsafe { CStr::from_ptr(sk8_rider_trick(self.raw)) }.to_string_lossy().into_owned()
    }

    pub fn bail(&mut self) {
        unsafe { sk8_rider_bail(self.raw) }
    }

    pub fn flags(&self) -> u32 {
        unsafe { sk8_rider_flags(self.raw) }
    }

    pub fn set_flags(&mut self, flags: u32) {
        unsafe { sk8_rider_set_flags(self.raw, flags) }
    }

    pub fn param(&self, index: usize) -> f32 {
        unsafe { sk8_rider_param(self.raw, index as c_int) }
    }

    /// Clamped to the parameter's range.
    pub fn set_param(&mut self, index: usize, value: f32) {
        unsafe { sk8_rider_set_param(self.raw, index as c_int, value) }
    }
}

impl Drop for Rider {
    fn drop(&mut self) {
        unsafe { sk8_rider_delete(self.raw) }
    }
}

fn static_str(p: *const c_char) -> &'static str {
    if p.is_null() {
        return "";
    }
    // SAFETY: the engine's parameter and trick tables are static C strings.
    unsafe { CStr::from_ptr(p) }.to_str().unwrap_or("")
}

/// Every tunable value, in menu order.
pub fn params() -> Vec<Param> {
    let n = unsafe { sk8_param_count() };
    (0..n)
        .filter_map(|i| {
            let p = unsafe { sk8_param_at(i).as_ref() }?;
            Some(Param {
                index: i as usize,
                key: static_str(p.key),
                label: static_str(p.label),
                group: static_str(p.group),
                min: p.min,
                max: p.max,
                default: p.def,
                step: p.step,
            })
        })
        .collect()
}

pub fn trick_names() -> Vec<&'static str> {
    let n = unsafe { sk8_trick_count() };
    (0..n).map(|i| static_str(unsafe { sk8_trick_name(i) })).collect()
}

/// A sample right-stick path for a trick (one point per step), for tutorials.
pub fn trick_demo_path(index: usize) -> Vec<[f32; 2]> {
    let mut buf = vec![[0f32; 2]; 128];
    let n = unsafe { sk8_trick_demo_path(index as c_int, buf.as_mut_ptr() as *mut f32, 128) };
    buf.truncate(n.max(0) as usize);
    buf
}

/// Which trick a stick path makes, if any.
pub fn classify(path: &[[f32; 2]]) -> Option<&'static str> {
    let p = unsafe { sk8_flick_classify(path.as_ptr() as *const f32, path.len() as c_int) };
    (!p.is_null()).then(|| static_str(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Floor;
    impl Collision for Floor {
        fn raycast(&self, o: [f32; 3], d: [f32; 3]) -> Option<Hit> {
            if o[1] >= 0.0 && o[1] + d[1] < 0.0 {
                let t = o[1] / -d[1];
                Some(Hit {
                    point: [o[0] + d[0] * t, 0.0, o[2] + d[2] * t],
                    normal: [0.0, 1.0, 0.0],
                    fraction: t,
                    surface: 7,
                })
            } else {
                None
            }
        }
    }

    #[test]
    fn every_trick_classifies_from_its_demo_path() {
        for (i, name) in trick_names().into_iter().enumerate() {
            assert_eq!(classify(&trick_demo_path(i)), Some(name));
        }
    }

    #[test]
    fn pushing_and_an_ollie_on_a_flat_floor() {
        let mut world = World::new(Box::new(Floor));
        let mut rider = Rider::new();
        rider.place([0.0, 0.0, 0.0], 0.0, 0.0);
        let push = Input {
            buttons: buttons::PUSH,
            ..Default::default()
        };
        for _ in 0..120 {
            rider.step(&mut world, &push);
        }
        let s = rider.state();
        assert!(s.speed > 4.0, "speed {}", s.speed);
        assert!(s.position[2] > 3.0, "moved along +Z: {:?}", s.position);
        let ollie = trick_demo_path(0);
        let mut events = Vec::new();
        let mut top = 0f32;
        for p in ollie.iter().chain(std::iter::repeat_n(&[0.0, 0.0], 90)) {
            rider.step(&mut world, &Input { flick: *p, ..Default::default() });
            top = top.max(rider.state().position[1]);
            while let Some(e) = rider.poll_event() {
                events.push(e);
            }
        }
        assert!(top > 0.3, "ollie height {top}");
        assert!(events.iter().any(|e| e.kind == EventKind::Pop && e.name == "Ollie"));
        assert!(events.iter().any(|e| e.kind == EventKind::Land && e.surface == 7));
    }

    #[test]
    fn params_round_trip() {
        let ps = params();
        assert!(ps.len() > 20);
        let mut rider = Rider::new();
        let g = ps.iter().find(|p| p.key == "gravity").unwrap();
        assert_eq!(rider.param(g.index), g.default);
        rider.set_param(g.index, 1e9);
        assert_eq!(rider.param(g.index), g.max);
    }
}
