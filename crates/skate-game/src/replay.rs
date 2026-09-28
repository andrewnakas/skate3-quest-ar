//! A rolling presentation recording, never a second simulation or a rewind of
//! live state. Root, skin (including the board), and camera share one playhead.
use crate::{
    app::SimulationSet,
    presentation::{Snapshot, blend},
};
use bevy::prelude::*;
use std::collections::VecDeque;

const WINDOW: f64 = 30.0;
const SELECT: u16 = 0x0020;
const A: u16 = 0x1000;
const B: u16 = 0x2000;
const Y: u16 = 0x8000;

struct Frame {
    time: f64,
    pose: Snapshot,
    cut: bool,
}

#[derive(Resource, Default)]
pub(crate) struct Replay {
    frames: VecDeque<Frame>,
    pub active: bool,
    pub free_camera: Option<Transform>,
    cursor: f64,
    playing: bool,
    previous_buttons: u16,
    release_controls: bool,
}

impl Replay {
    pub fn record(&mut self, pose: Snapshot, dt: f64, cut: bool) {
        if self.active || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let time = self.frames.back().map_or(0.0, |f| f.time + dt);
        self.frames.push_back(Frame { time, pose, cut });
        // Keep one bracketing sample at the start for fractional-time scrubs.
        while self.frames.len() > 2 && self.frames[1].time <= time - WINDOW {
            self.frames.pop_front();
        }
    }

    fn bounds(&self) -> (f64, f64) {
        let end = self.frames.back().map_or(0.0, |f| f.time);
        (
            self.frames
                .front()
                .map_or(0.0, |f| f.time)
                .max(end - WINDOW),
            end,
        )
    }

    pub(crate) fn enter(&mut self) {
        if self.frames.is_empty() {
            return;
        }
        self.active = true;
        self.playing = false;
        self.free_camera = None;
        self.cursor = self.bounds().1;
    }

    fn exit(&mut self) {
        self.active = false;
        self.playing = false;
        self.free_camera = None;
        self.release_controls = true;
    }

    fn seek(&mut self, time: f64) {
        let (start, end) = self.bounds();
        self.cursor = time.clamp(start, end);
    }

    fn step(&mut self, direction: i32) {
        self.playing = false;
        let next = if direction > 0 {
            self.frames.iter().find(|f| f.time > self.cursor + 1e-6)
        } else {
            self.frames
                .iter()
                .rev()
                .find(|f| f.time < self.cursor - 1e-6)
        };
        if let Some(next) = next {
            self.seek(next.time);
        }
    }

    pub fn sample(&self) -> Option<(&Snapshot, &Snapshot, f32)> {
        if !self.active {
            return None;
        }
        let upper = self.frames.partition_point(|f| f.time <= self.cursor);
        let a = self.frames.get(upper.saturating_sub(1))?;
        let b = self.frames.get(upper).unwrap_or(a);
        // Teleports, camera cuts and cadence changes remain discrete.
        if b.cut || a.pose.bones.len() != b.pose.bones.len() {
            return Some((&a.pose, &a.pose, 0.0));
        }
        let alpha = if b.time > a.time {
            ((self.cursor - a.time) / (b.time - a.time)).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        Some((&a.pose, &b.pose, alpha))
    }

    fn toggle_camera(&mut self) {
        self.free_camera = if self.free_camera.is_some() {
            None
        } else {
            self.sample().map(|(a, b, t)| blend(a.camera, b.camera, t))
        };
    }
}

pub(crate) struct ReplayPlugin;
impl Plugin for ReplayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Replay>()
            .configure_sets(
                FixedUpdate,
                (
                    SimulationSet::Input,
                    SimulationSet::Controls,
                    SimulationSet::Physics,
                )
                    .run_if(live),
            )
            .add_systems(
                PreUpdate,
                controls
                    .after(crate::input::poll_controllers)
                    .after(bevy::input::InputSystems),
            )
            .add_systems(Startup, setup_ui)
            .add_systems(Update, update_ui);
    }
}

fn live(replay: Res<Replay>) -> bool {
    !replay.active
}

fn axis(value: f32) -> f32 {
    value.signum() * ((value.abs() - 0.18) / 0.82).clamp(0.0, 1.0)
}

fn controls(
    vehicles: Res<crate::modding::vehicles::Vehicles>,
    mut replay: ResMut<Replay>,
    mut input: ResMut<crate::input::ControllerInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    time: Res<Time<Real>>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    net: Option<Res<crate::multiplayer::Multiplayer>>,
) {
    if net.is_some_and(|n| n.active()) {
        if replay.active {
            replay.exit();
        }
        return;
    }
    let pad = input.raw_input();
    let pressed = pad.buttons & !replay.previous_buttons;
    replay.previous_buttons = pad.buttons;
    let menu_open = !crate::graphics_menu::gameplay_active(menu);
    if !menu_open && !vehicles.occupied() {
        if pressed & SELECT != 0 || keys.just_pressed(KeyCode::F6) {
            if replay.active {
                replay.exit();
            } else {
                replay.enter();
            }
        } else if replay.active && pressed & B != 0 {
            replay.exit();
        }
    }
    // Continue reading the UI controller while fixed gameplay ticks are gated.
    // On exit, require release so a held scrub/flight control cannot become a
    // grab, push or flick in the preserved live simulation.
    if replay.active || replay.release_controls {
        input.discard_gameplay();
        if !replay.active
            && pad.buttons == 0
            && pad.triggers.iter().all(|v| *v < 0.05)
            && pad.left.iter().chain(&pad.right).all(|v| v.abs() < 0.18)
        {
            replay.release_controls = false;
        }
    }
    if !replay.active || menu_open {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    if pressed & Y != 0 || keys.just_pressed(KeyCode::KeyC) {
        replay.toggle_camera();
    }
    if pressed & A != 0 || keys.just_pressed(KeyCode::Space) {
        if replay.cursor >= replay.bounds().1 {
            let start = replay.bounds().0;
            replay.seek(start);
        }
        replay.playing = !replay.playing;
    }
    let trigger = |v: f32| ((v - 0.05) / 0.95).clamp(0.0, 1.0);
    let scrub = trigger(pad.triggers[1]) - trigger(pad.triggers[0])
        + f32::from(keys.pressed(KeyCode::ArrowRight))
        - f32::from(keys.pressed(KeyCode::ArrowLeft));
    if pad.triggers.iter().any(|v| *v > 0.05)
        || keys.pressed(KeyCode::ArrowLeft)
        || keys.pressed(KeyCode::ArrowRight)
    {
        replay.playing = false;
        let cursor = replay.cursor + f64::from(scrub * dt * 4.0);
        replay.seek(cursor);
    } else if replay.playing {
        let cursor = replay.cursor + f64::from(dt);
        replay.seek(cursor);
        if replay.cursor >= replay.bounds().1 {
            replay.playing = false;
        }
    }
    if pressed & 0x0004 != 0 || keys.just_pressed(KeyCode::Comma) {
        replay.step(-1);
    }
    if pressed & 0x0008 != 0 || keys.just_pressed(KeyCode::Period) {
        replay.step(1);
    }
    if let Some(camera) = replay.free_camera.as_mut() {
        let (mut yaw, mut pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
        yaw -= axis(pad.right[0]) * dt * 2.0;
        pitch += axis(pad.right[1]) * dt * 2.0;
        if mouse.pressed(MouseButton::Right) {
            yaw -= motion.delta.x * 0.003;
            pitch -= motion.delta.y * 0.003;
        }
        pitch = pitch.clamp(-1.5, 1.5);
        camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
        let x = axis(pad.left[0]) + f32::from(keys.pressed(KeyCode::KeyD))
            - f32::from(keys.pressed(KeyCode::KeyA));
        let z = axis(pad.left[1]) + f32::from(keys.pressed(KeyCode::KeyW))
            - f32::from(keys.pressed(KeyCode::KeyS));
        let y = f32::from(pad.buttons & 0x0200 != 0 || keys.pressed(KeyCode::KeyE))
            - f32::from(pad.buttons & 0x0100 != 0 || keys.pressed(KeyCode::KeyQ));
        let heading = Quat::from_rotation_y(yaw);
        let speed = if pad.buttons & 0x0040 != 0 || keys.pressed(KeyCode::ShiftLeft) {
            12.0
        } else {
            3.0
        };
        camera.translation += (heading * Vec3::new(x, 0.0, -z) + Vec3::Y * y) * dt * speed;
    }
}

#[derive(Component)]
struct ReplayUi;
#[derive(Component)]
struct ReplayLabel;
#[derive(Component)]
struct Playhead;

fn setup_ui(mut commands: Commands) {
    commands.spawn((ReplayUi, GlobalZIndex(5), Node {
        display: Display::None, position_type: PositionType::Absolute,
        bottom: px(24), left: percent(8), width: percent(84),
        padding: UiRect::all(px(16)), flex_direction: FlexDirection::Column,
        row_gap: px(10), ..default()
    }, BackgroundColor(Color::srgba(0.02, 0.035, 0.05, 0.92)))).with_children(|panel| {
        panel.spawn((ReplayLabel, Text::new("REPLAY"), TextFont { font_size: 20., ..default() }));
        panel.spawn((Node { width: percent(100), height: px(6), ..default() },
            BackgroundColor(Color::srgb(0.18, 0.22, 0.26)))).with_children(|bar| {
            bar.spawn((Playhead, Node { width: percent(100), height: percent(100), ..default() },
                BackgroundColor(Color::srgb(0.35, 0.9, 0.7))));
        });
        panel.spawn((Text::new("LT / RT  Scrub   |   D-pad left / right  Frame step   |   A  Play / pause   |   Y  Free camera   |   View / B  Exit\nFree camera: Left stick  Move   |   Right stick  Look   |   LB / RB  Down / up   |   L-stick click  Fast\nKeyboard: F6  Replay   |   Arrows  Scrub   |   , / .  Frame step   |   Space  Play   |   C  Camera   |   WASD / Q / E  Move   |   RMB drag  Look"),
            TextFont { font_size: 14., ..default() }, TextColor(Color::srgb(0.75, 0.82, 0.86))));
    });
}

fn update_ui(
    replay: Res<Replay>,
    mut roots: Query<&mut Node, With<ReplayUi>>,
    mut labels: Query<&mut Text, With<ReplayLabel>>,
    mut bars: Query<&mut Node, (With<Playhead>, Without<ReplayUi>)>,
) {
    for mut node in &mut roots {
        node.display = if replay.active {
            Display::Flex
        } else {
            Display::None
        };
    }
    if !replay.active {
        return;
    }
    let (start, end) = replay.bounds();
    for mut text in &mut labels {
        **text = format!(
            "REPLAY  {:.2} / {:.2} s   |   {}   |   {}",
            replay.cursor - start,
            end - start,
            if replay.playing { "PLAYING" } else { "PAUSED" },
            if replay.free_camera.is_some() {
                "FREE CAMERA"
            } else {
                "RECORDED CAMERA"
            }
        );
    }
    let fraction = if end > start {
        (replay.cursor - start) / (end - start)
    } else {
        1.0
    };
    for mut node in &mut bars {
        node.width = percent(fraction as f32 * 100.0);
    }
}

#[cfg(test)]
mod tests;
