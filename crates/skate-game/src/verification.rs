//! Opt-in startup verification, with no in-game tool UI.
use crate::{
    animation::AnimationStatus,
    app::FrameSet,
    config::Config,
    input::ControllerInput,
    physics::{GamePhysics, PlayerControls, SkaterRuntime},
};
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};

#[derive(Resource, Default)]
struct Verification {
    elapsed: f32,
    requested: bool,
    captured: bool,
}

pub(crate) struct VerificationPlugin;
impl Plugin for VerificationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Verification>()
            .add_systems(Update, verify.in_set(FrameSet::Verification));
    }
}
fn verify(
    mut commands: Commands,
    time: Res<Time<Real>>,
    config: Res<Config>,
    animation: Res<AnimationStatus>,
    input: Res<ControllerInput>,
    physics: Res<GamePhysics>,
    controls: Res<PlayerControls>,
    skater: Res<SkaterRuntime>,
    camera: Res<crate::camera::CameraRuntime>,
    mut replay: ResMut<crate::replay::Replay>,
    mut state: ResMut<Verification>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &config.verification_capture else {
        return;
    };
    state.elapsed += time.delta_secs();
    // Opt-in visual smoke check of the replay HUD and presentation endpoints.
    if animation.ready
        && state.elapsed > 2.0
        && !replay.active
        && std::env::var("SKATE_VERIFY_REPLAY").as_deref() == Ok("1")
    {
        replay.enter();
    }
    if animation.ready && state.elapsed > 4. && !state.requested {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut state: ResMut<Verification>| {
                    state.captured = true;
                },
            );
        state.requested = true;
    }
    if state.captured && state.elapsed > 6. {
        let input_report = format!(
            "Game integration capture (not a Skate 3 parity verdict)\nExecutable: {}\nPolls: {}\nConsumed batches: {}\nStatus: {:?}\nPacket numbers: {:?}\nActions 64..81 by device: {:?}\nDerived ticks: {}\nIntents: {:?}\nDifficulty index: {}\nPhysics ticks: {}\nContacts: {}\nBody positions: {:?}\nPhysical pose publications: {}\nGraph animation ticks: {}\nTruck targets: {:?}\nGround speed: {}\nGameplay camera shot: {}\nCamera frame: {:?}\n",
            std::env::current_exe()
                .map_or_else(|_| "unavailable".into(), |p| p.display().to_string()),
            input.publications,
            input.consumed_batches,
            input.status,
            input.packet_numbers,
            input.mapped_actions,
            controls.ticks,
            controls.intents,
            physics.difficulty_index(),
            physics.ticks,
            physics.contact_count,
            physics.board.bodies().map(|body| body.rates.position),
            skater.pose_generation,
            skater.animation.ticks,
            skater.ground.steering.targets,
            physics.riding.motion.ground_speed,
            camera.selected_shot(),
            camera.frame,
        );
        let input_report = format!(
            "{input_report}Native OnBoard/OffBoard clips available: {}\n",
            skater.animation.evaluator.frames.clip_count()
        );
        if let Err(error) = std::fs::write(path.with_extension("input.txt"), input_report) {
            eprintln!("Cannot save controller verification: {error}");
            exit.write(AppExit::error());
            return;
        }
        info!("GAME_VERIFY_OK");
        exit.write(AppExit::Success);
    } else if state.elapsed > 45. {
        error!("Game startup/capture timed out");
        exit.write(AppExit::error());
    }
}
