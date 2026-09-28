use crate::{
    animation, assets, camera,
    config::Config,
    graph_runtime::StockGraphs,
    input,
    physics::{GamePhysics, PhysicsPlugin, SkaterRuntime},
    verification, world,
};
use bevy::{
    prelude::*,
    render::{
        RenderPlugin,
        settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings},
    },
};
use skate_data::GameAssets;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum FrameSet {
    Assets,
    Physics,
    Animation,
    Verification,
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum SimulationSet {
    Input,
    Controls,
    Physics,
}

pub(crate) fn build(
    config: Config,
    manifest: GameAssets,
    graphs: StockGraphs,
    physics: GamePhysics,
    skater: SkaterRuntime,
) -> App {
    let retail_scene = config
        .map
        .as_ref()
        .is_some_and(|map| crate::retail_render::RetailScene::for_map(map));
    let mut app = App::new();
    crate::custom_models::register_source(&mut app);
    app.register_asset_source(
        "mods",
        bevy::asset::io::AssetSourceBuilder::platform_default(
            &crate::modding::package_root().to_string_lossy(),
            None,
        ),
    );
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: config.asset_root.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: config
                        .multiplayer
                        .title
                        .clone()
                        .unwrap_or_else(|| "Skate 3 Rust Engine".into()),
                    resolution: (1280, 800).into(),
                    ..default()
                }),
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(WgpuSettings {
                    backends: Some(Backends::VULKAN),
                    // Existing machine's validation layer rejects wgpu atomic shaders.
                    // This workaround belongs only to the rendering adapter.
                    instance_flags: InstanceFlags::empty(),
                    ..default()
                }),
                ..default()
            })
            .build()
            .disable::<bevy::log::LogPlugin>()
            // Gameplay and menu navigation both use raw XInput. No game system
            // consumes Bevy gamepad events/rumble; its second device backend can
            // stall PreUpdate (70.68 ms in the University capture).
            .disable::<bevy::gilrs::GilrsPlugin>(),
    )
    .insert_resource(bevy::winit::WinitSettings {
        focused_mode: bevy::winit::UpdateMode::Continuous,
        unfocused_mode: bevy::winit::UpdateMode::Continuous,
    })
    .insert_resource(crate::skate_audio::player::PlayerAudioAssets(
        config.asset_root.clone(),
    ))
    .insert_resource(config)
    .insert_resource(crate::retail_render::RetailScene(retail_scene))
    .insert_resource(assets::AssetManifest(manifest))
    .insert_resource(graphs)
    .insert_resource(physics)
    .insert_resource(skater)
    .configure_sets(
        FixedUpdate,
        (
            SimulationSet::Input,
            SimulationSet::Controls,
            SimulationSet::Physics,
        )
            .chain(),
    )
    .configure_sets(
        Update,
        (
            FrameSet::Assets,
            FrameSet::Physics,
            FrameSet::Animation,
            FrameSet::Verification,
        )
            .chain(),
    )
    .add_plugins(crate::fps_overlay::FpsOverlayPlugin)
    .add_plugins((
        crate::retail_render::RetailRenderPlugin,
        input::InputPlugin,
        PhysicsPlugin,
        crate::presentation::PresentationPlugin,
        crate::replay::ReplayPlugin,
        assets::GameAssetsPlugin,
        animation::AnimationPlugin,
        world::WorldPlugin,
        crate::grind_world::GrindGeometryPlugin,
        camera::CameraPlugin,
        crate::graphics_menu::GraphicsMenuPlugin,
        crate::map_transition::MapTransitionPlugin,
        crate::render_capacity::RenderCapacityPlugin,
        verification::VerificationPlugin,
        crate::performance::PerformancePlugin,
    ));
    app.add_plugins((
        crate::session_marker::SessionMarkerPlugin,
        crate::customiser::CustomiserPlugin,
    ));
    app.add_plugins(crate::custom_models::CustomModelsPlugin);
    app.add_plugins(crate::modding::ModdingPlugin);
    app.add_plugins(crate::skate_audio::SkateAudioPlugin);
    crate::teleport_menu::install(&mut app);
    app.add_plugins(crate::updater::UpdaterPlugin);
    app.add_plugins(crate::multiplayer::MultiplayerPlugin);
    app.add_plugins(crate::scoring_hud::ScoringHudPlugin);
    app.add_systems(Last, crate::crash_context::sample);
    crate::profiling::install(&mut app);
    app
}
