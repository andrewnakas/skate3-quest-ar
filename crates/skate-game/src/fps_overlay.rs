//! Presentation-frame FPS, independent of fixed physics and HUD refresh cadence.
use bevy::prelude::*;

pub(crate) struct FpsOverlayPlugin;

#[derive(Component)]
struct FpsText;

#[derive(Default)]
struct FrameSample {
    frames: u32,
    seconds: f64,
}

impl Plugin for FpsOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

fn spawn(mut commands: Commands) {
    // Use the existing gameplay camera; no independent overlay render loop.
    commands
        .spawn((
            Name::new("FPS overlay"),
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                right: px(12),
                padding: UiRect::axes(px(10), px(6)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            GlobalZIndex(100),
        ))
        .with_children(|parent| {
            parent.spawn((
                FpsText,
                Text::new("FPS: --"),
                TextFont {
                    font_size: 20.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

fn update(
    time: Res<Time<Real>>,
    mut sample: Local<FrameSample>,
    mut labels: Query<&mut Text, With<FpsText>>,
) {
    let seconds = time.delta_secs_f64();
    if seconds <= 0.0 {
        return;
    }
    // One sample for EVERY presentation frame, including long/stalled frames.
    // Real time is unaffected by slow motion or the fixed simulation timestep.
    sample.frames += 1;
    sample.seconds += seconds;
    if sample.seconds >= 0.5 {
        let fps = f64::from(sample.frames) / sample.seconds;
        for mut text in &mut labels {
            text.0 = format!("FPS: {fps:.0}");
        }
        *sample = FrameSample::default();
    }
}
