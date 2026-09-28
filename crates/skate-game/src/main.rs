mod animation;
mod animation_pose;
mod app;
mod apt_display;
mod apt_movie;
mod apt_scene;
mod apt_text;
mod apt_vm;
mod assets;
mod camera;
mod config;
mod crash_context;
mod crash_report;
mod custom_models;
mod customiser;
mod customiser_material;
mod customiser_parts;
mod difficulty;
mod fps_overlay;
mod graph_host;
mod graph_runtime;
mod graphics_menu;
mod grind_world;
mod hud_runtime;
mod input;
mod map_library;
mod map_render;
mod map_transition;
mod modding;
mod multiplayer;
mod performance;
mod physics;
mod presentation;
mod profiling;
mod render_capacity;
mod replay;
mod retail_character;
mod retail_irradiance;
mod retail_render;
mod scoring_hud;
mod scoring_runtime;
mod session_marker;
mod setup;
mod skate_audio;
mod skate_world;
mod skater_animation;
mod teleport_menu;
mod updater;
mod verification;
mod world;

fn main() -> bevy::app::AppExit {
    match updater::recover() {
        Ok(true) => return bevy::app::AppExit::Success,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::Success;
        }
        Ok(false) => {}
    }
    if let Some(code) = crash_report::entry() {
        std::process::exit(code);
    }
    let _trace = match profiling::init() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    };
    let _startup = bevy::log::info_span!("startup").entered();
    eprintln!("REPORT_META stage=configuration_and_installation");
    let config = match bevy::log::info_span!("load_configuration_and_map")
        .in_scope(config::Config::from_env)
    {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    };
    profiling::map_metadata(&config);
    eprintln!(
        "REPORT_META startup=map_fingerprint:{:016x} difficulty:{} multiplayer_requested:{} custom_appearance:{} renderer:Vulkan",
        config.map_fingerprint,
        config.difficulty.key(),
        config.multiplayer.host.is_some() || config.multiplayer.direct.is_some(),
        config.multiplayer.appearance.is_some()
    );
    eprintln!("REPORT_META stage=gameplay_configuration");
    if let Err(error) = skate_data::input_config::StockGameplayConfig::load(&config.asset_root) {
        eprintln!("{error}");
        return bevy::app::AppExit::error();
    }
    eprintln!("REPORT_META stage=asset_manifest");
    let manifest = match bevy::log::info_span!("load_manifest")
        .in_scope(|| skate_data::GameAssets::load(&config.asset_root))
    {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    };
    eprintln!("REPORT_META stage=stock_graphs");
    let graphs = match bevy::log::info_span!("load_graphs")
        .in_scope(|| graph_runtime::StockGraphs::load(&config.asset_root, &manifest))
    {
        Ok(graphs) => graphs,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    };
    eprintln!("REPORT_META stage=map_validation");
    if let Some(map) = &config.map {
        if let Err(error) = skate_world::validate_runtime(map) {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    }
    eprintln!(
        "SKATE_DIFFICULTY mode={} native_index={}",
        config.difficulty.key(),
        config.difficulty as u32
    );
    eprintln!("REPORT_META stage=physics_initialization");
    let mut physics = match bevy::log::info_span!("load_physics").in_scope(|| {
        physics::GamePhysics::load_with_difficulty(
            &config.asset_root,
            config.map.as_ref(),
            config.difficulty,
        )
    }) {
        Ok(physics) => physics,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    };
    if config.multiplayer.spawn_offset != 0.0 {
        let mut spawn =
            physics.board.part_transforms()[skate_core::physics::board::BodyId::Deck.index()];
        spawn.translation.x += config.multiplayer.spawn_offset;
        physics.board.set_transform(spawn);
    }
    eprintln!("REPORT_META stage=skater_initialization");
    let skater = match bevy::log::info_span!("load_skater").in_scope(|| {
        physics::SkaterRuntime::load(
            &config.asset_root,
            &graphs,
            &physics,
            config.difficulty.key(),
        )
    }) {
        Ok(skater) => skater,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    };
    eprintln!("REPORT_META stage=controls_initialization");
    let controls = match physics::PlayerControls::load(&config.asset_root) {
        Ok(controls) => controls,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
    };
    if config.check_assets {
        if let Err(error) = camera::CameraRuntime::load(&config.asset_root) {
            eprintln!("{error}");
            return bevy::app::AppExit::error();
        }
        eprintln!("SKATE_ASSETS_READY");
        return bevy::app::AppExit::Success;
    }
    eprintln!("REPORT_META stage=renderer_and_app_initialization");
    let mut app = app::build(config, manifest, graphs, physics, skater);
    app.insert_resource(controls);
    eprintln!("REPORT_META stage=app_run");
    drop(_startup);
    app.run()
}

#[cfg(test)]
#[path = "tests/action_host.rs"]
mod action_host_tests;
