#[cfg(feature = "retail")]
mod animation;
#[cfg(target_os = "android")]
mod android;
#[cfg(all(feature = "retail", any(target_os = "android", test)))]
mod clean_pack;
#[cfg(feature = "retail")]
mod animation_pose;
#[cfg(feature = "retail")]
mod app;
#[cfg(feature = "retail")]
mod apt_display;
#[cfg(feature = "retail")]
mod apt_movie;
#[cfg(feature = "retail")]
mod apt_scene;
#[cfg(feature = "retail")]
mod apt_text;
#[cfg(feature = "retail")]
mod apt_vm;
#[cfg(feature = "retail")]
mod assets;
#[cfg(feature = "retail")]
mod camera;
mod config;
#[cfg(feature = "retail")]
mod crash_context;
#[cfg(feature = "retail")]
mod crash_report;
#[cfg(feature = "retail")]
mod custom_models;
#[cfg(feature = "retail")]
mod customiser;
#[cfg(feature = "retail")]
mod customiser_material;
#[cfg(feature = "retail")]
mod customiser_parts;
#[cfg(feature = "retail")]
mod difficulty;
#[cfg(feature = "retail")]
mod fps_overlay;
#[cfg(feature = "retail")]
mod graph_host;
#[cfg(feature = "retail")]
mod graph_runtime;
#[cfg(feature = "retail")]
mod graphics_menu;
#[cfg(feature = "retail")]
mod grind_world;
#[cfg(feature = "retail")]
mod hud_runtime;
#[cfg(feature = "retail")]
mod input;
// The store build: the pad plumbing the imported-disc tier reads, as stand-ins.
#[cfg(not(feature = "retail"))]
#[path = "input_store.rs"]
mod input;
#[cfg(feature = "retail")]
mod map_library;
#[cfg(feature = "retail")]
mod map_render;
#[cfg(feature = "retail")]
mod map_transition;
#[cfg(feature = "retail")]
mod modding;
#[cfg(feature = "retail")]
mod multiplayer;
#[cfg(feature = "retail")]
mod performance;
#[cfg(feature = "retail")]
mod physics;
#[cfg(feature = "retail")]
mod presentation;
#[cfg(feature = "retail")]
mod profiling;
#[cfg(feature = "retail")]
mod render_capacity;
#[cfg(feature = "retail")]
mod replay;
#[cfg(feature = "retail")]
mod retail_character;
#[cfg(feature = "retail")]
mod retail_irradiance;
#[cfg(feature = "retail")]
mod retail_render;
#[cfg(feature = "retail")]
mod scoring_hud;
#[cfg(feature = "retail")]
mod scoring_runtime;
#[cfg(feature = "retail")]
mod session_marker;
#[cfg(feature = "retail")]
mod setup;
#[cfg(feature = "retail")]
mod skate_audio;
// The store build (no `retail`): only the clean tier's own sounds.
#[cfg(not(feature = "retail"))]
#[path = "skate_audio_store.rs"]
mod skate_audio;
mod stream_pcm;
#[cfg(feature = "retail")]
mod skate_world;
#[cfg(feature = "retail")]
mod skater_animation;
#[cfg(feature = "retail")]
mod teleport_menu;
#[cfg(feature = "retail")]
mod updater;
#[cfg(feature = "retail")]
mod verification;
#[cfg(feature = "retail")]
mod world;
#[cfg(feature = "xr")]
mod xr;
mod clean;

/// Platform hooks for shared storage (the headset's `Download` folder),
/// set by the Android entry point: whether the app may read it, and asking
/// the player for it ("All files access").
#[derive(Clone, Copy)]
pub struct StorageHooks {
    pub has_access: fn() -> bool,
    pub request_access: fn(),
}

static STORAGE_HOOKS: std::sync::OnceLock<StorageHooks> = std::sync::OnceLock::new();

pub fn set_storage_hooks(hooks: StorageHooks) {
    let _ = STORAGE_HOOKS.set(hooks);
}

/// Shared storage can be read (always, without hooks: the desktop).
#[cfg_attr(not(feature = "xr"), allow(dead_code))]
pub(crate) fn storage_access() -> bool {
    STORAGE_HOOKS.get().is_none_or(|h| (h.has_access)())
}

#[cfg_attr(not(feature = "xr"), allow(dead_code))]
pub(crate) fn request_storage_access() {
    if let Some(h) = STORAGE_HOOKS.get() {
        (h.request_access)();
    }
}

/// Startup loading. `Err` is the exit to return, including the asset
/// check's success.
#[cfg(feature = "retail")]
fn load() -> Result<Loaded, bevy::app::AppExit> {
    eprintln!("REPORT_META stage=configuration_and_installation");
    let config = match bevy::log::info_span!("load_configuration_and_map")
        .in_scope(config::Config::from_env)
    {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
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
        return Err(bevy::app::AppExit::error());
    }
    eprintln!("REPORT_META stage=asset_manifest");
    let manifest = match bevy::log::info_span!("load_manifest")
        .in_scope(|| skate_data::GameAssets::load(&config.asset_root))
    {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
    };
    eprintln!("REPORT_META stage=stock_graphs");
    let graphs = match bevy::log::info_span!("load_graphs")
        .in_scope(|| graph_runtime::StockGraphs::load(&config.asset_root, &manifest))
    {
        Ok(graphs) => graphs,
        Err(error) => {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
    };
    eprintln!("REPORT_META stage=map_validation");
    if let Some(map) = &config.map {
        if let Err(error) = skate_world::validate_runtime(map) {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
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
            return Err(bevy::app::AppExit::error());
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
            return Err(bevy::app::AppExit::error());
        }
    };
    eprintln!("REPORT_META stage=controls_initialization");
    let controls = match physics::PlayerControls::load(&config.asset_root) {
        Ok(controls) => controls,
        Err(error) => {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
    };
    if config.check_assets {
        if let Err(error) = camera::CameraRuntime::load(&config.asset_root) {
            eprintln!("{error}");
            return Err(bevy::app::AppExit::error());
        }
        eprintln!("SKATE_ASSETS_READY");
        return Err(bevy::app::AppExit::Success);
    }
    Ok(Loaded {
        config,
        manifest,
        graphs,
        physics,
        skater,
        controls,
    })
}

/// The whole game; the `skate3rust` binary and the Quest library both start here.
pub fn run() -> bevy::app::AppExit {
    run_with(None)
}

/// Everything startup loads before the app is built.
#[cfg(feature = "retail")]
struct Loaded {
    config: config::Config,
    manifest: skate_data::GameAssets,
    graphs: graph_runtime::StockGraphs,
    physics: physics::GamePhysics,
    skater: physics::SkaterRuntime,
    controls: physics::PlayerControls,
}

/// [`run`], optionally loading on a worker thread while `pump` is called
/// repeatedly on this one. Android needs that: until this thread services
/// the Activity's looper, the Java UI thread is blocked in onStart and the
/// window is never shown (an ANR on the first key press), and loading takes
/// seconds.
pub fn run_with(pump: Option<&mut dyn FnMut()>) -> bevy::app::AppExit {
    #[cfg(target_os = "android")]
    if let Err(error) = android::prepare() {
        eprintln!("{error}");
        return bevy::app::AppExit::error();
    }
    // The public tier: the clean-room engine, no game data at all. (The store build has nothing
    // else.)
    #[cfg(feature = "retail")]
    if !clean::wanted() {
        return run_retail(pump);
    }
    #[cfg(not(feature = "retail"))]
    let _ = pump;
    clean::run()
}

/// The imported-disc tier: the faithful port on the player's own converted data.
#[cfg(feature = "retail")]
fn run_retail(pump: Option<&mut dyn FnMut()>) -> bevy::app::AppExit {
    // The updater and the crash supervisor relaunch the current executable,
    // which on Android is the system app process.
    #[cfg(not(target_os = "android"))]
    match updater::recover() {
        Ok(true) => return bevy::app::AppExit::Success,
        Err(error) => {
            eprintln!("{error}");
            return bevy::app::AppExit::Success;
        }
        Ok(false) => {}
    }
    #[cfg(not(target_os = "android"))]
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
    let loaded = match pump {
        None => load(),
        Some(pump) => std::thread::scope(|scope| {
            let loader = std::thread::Builder::new()
                .name("startup-loader".into())
                .stack_size(64 << 20)
                .spawn_scoped(scope, load)
                .expect("could not start the startup loader thread");
            while !loader.is_finished() {
                pump();
            }
            loader
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        }),
    };
    let Loaded {
        config,
        manifest,
        graphs,
        physics,
        skater,
        controls,
    } = match loaded {
        Ok(loaded) => loaded,
        Err(exit) => return exit,
    };
    eprintln!("REPORT_META stage=renderer_and_app_initialization");
    let mut app = app::build(config, manifest, graphs, physics, skater);
    app.insert_resource(controls);
    // The winit runner would do this first thing; doing it here puts its
    // cost (render/OpenXR plugin setup, before the Android looper is ever
    // serviced) in the startup log.
    if app.plugins_state() == bevy::app::PluginsState::Ready {
        eprintln!("REPORT_META stage=plugins_finish");
        app.finish();
        app.cleanup();
    }
    eprintln!("REPORT_META stage=app_run");
    drop(_startup);
    app.run()
}

#[cfg(all(test, feature = "retail"))]
#[path = "tests/action_host.rs"]
mod action_host_tests;
