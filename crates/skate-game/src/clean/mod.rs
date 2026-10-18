//! The public tier: the clean-room engine (`skate-lite-sys`, written from
//! scratch, no retail data), a skater built in code ([`skater`]), CC0
//! sounds, and the shared XR layer (room collision, build mode, spawn).
//!
//! Nothing here reads a file from an imported disc. The imported-disc tier
//! (the faithful port) runs instead when `files/use-retail-data` exists on
//! the Quest, or without `--clean` on the desktop.
mod collide;
mod input;
mod menu;
pub(crate) mod sim;
mod skater;
mod view;
#[cfg(feature = "xr")]
mod xr_menu;

use bevy::prelude::*;
use sim::CleanSkate;
use skate_lite_sys::{EventKind, Mode};

/// Set while a menu owns the controllers (the skater gets neutral input).
#[derive(Resource, Default)]
pub(crate) struct UiCapture(pub bool);

#[derive(Clone, Debug, Default)]
pub(crate) struct Options {
    pub xr: Option<crate::config::XrOptions>,
    pub check: bool,
    pub files: std::path::PathBuf,
}

/// Whether this launch runs the clean tier.
pub(crate) fn wanted() -> bool {
    #[cfg(target_os = "android")]
    {
        match crate::android::files_dir() {
            Ok(files) => !files.join("use-retail-data").exists(),
            Err(_) => true,
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        std::env::args().any(|a| a == "--clean")
    }
}

fn options() -> Result<Options, String> {
    let mut o = Options::default();
    #[cfg(target_os = "android")]
    {
        o.xr = Some(crate::config::XrOptions::default());
        o.files = crate::android::files_dir()?;
    }
    #[cfg(not(target_os = "android"))]
    {
        o.files = std::env::current_dir().map_err(|e| e.to_string())?;
    }
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--clean" | "--test-world" => {}
            "--check-assets" => o.check = true,
            "--xr" => {
                if !cfg!(feature = "xr") {
                    return Err("--xr needs a build with the `xr` feature".into());
                }
                o.xr.get_or_insert_default();
            }
            "--xr-scale" => {
                let s: f32 = args
                    .next()
                    .ok_or("--xr-scale requires a number")?
                    .parse()
                    .map_err(|_| "--xr-scale requires a number")?;
                o.xr.get_or_insert_default().scale = s;
            }
            "--files" => o.files = args.next().ok_or("--files requires a directory")?.into(),
            other => warn!("clean tier ignores {other}"),
        }
    }
    Ok(o)
}

pub(crate) fn run() -> AppExit {
    let options = match options() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return AppExit::error();
        }
    };
    if options.check {
        // Everything the clean tier needs is compiled in.
        let params = skate_lite_sys::params().len();
        let tricks = skate_lite_sys::trick_names().len();
        eprintln!("SKATE_CLEAN_TIER engine params={params} tricks={tricks}");
        eprintln!("SKATE_ASSETS_READY");
        return AppExit::Success;
    }
    eprintln!("SKATE_CLEAN_TIER starting (xr={})", options.xr.is_some());
    build(options).run()
}

fn build(options: Options) -> App {
    use bevy::render::{
        RenderPlugin,
        settings::{Backends, InstanceFlags, RenderCreation, WgpuSettings},
    };
    let mut app = App::new();
    let plugins = DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: "SK8 AR (clean-room engine)".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        })
        .set(RenderPlugin {
            render_creation: RenderCreation::Automatic(WgpuSettings {
                // Metal on Apple machines (no Vulkan driver there), Vulkan elsewhere.
                backends: Some(if cfg!(target_vendor = "apple") { Backends::METAL } else { Backends::VULKAN }),
                instance_flags: InstanceFlags::empty(),
                ..default()
            }),
            ..default()
        })
        .build();
    #[cfg(target_os = "android")]
    let plugins = plugins.disable::<bevy::gilrs::GilrsPlugin>();
    #[cfg(feature = "xr")]
    let plugins = if options.xr.is_some() { crate::xr::plugins(plugins) } else { plugins };
    app.add_plugins(plugins)
        .insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        })
        .insert_resource(CleanSkate::new())
        .insert_resource(menu::Menu::new(options.files.clone(), options.xr.is_some()))
        .init_resource::<menu::MenuNav>()
        .init_resource::<UiCapture>()
        .add_systems(Startup, (view::lights, skater::spawn))
        .add_systems(PostStartup, menu::spawn_ui);
    crate::skate_audio::clean::install_sk8(&mut app);
    bevy::asset::embedded_asset!(app, "assets/skater.glb");

    let xr = options.xr.is_some();
    #[cfg(feature = "xr")]
    if let Some(xr_options) = options.xr {
        app.add_plugins(crate::xr::XrPlugin {
            options: xr_options,
            files: options.files.clone(),
            retail: false,
        })
        .add_systems(Startup, |panel: Res<crate::xr::panel::XrPanel>, mut commands: Commands| {
            commands.insert_resource(menu::MenuTarget(panel.camera));
        })
        .add_systems(
            Update,
            (
                xr_menu::touch_nav,
                menu::update,
                xr_menu::apply,
                input::read_touch,
                sim::apply_ar_world,
                sim::teleport,
                sim::step,
                sim::publish_link,
            )
                .chain()
                .before(feed_sounds),
        );
    }
    if !xr {
        app.add_systems(Startup, view::desktop_world).add_systems(
            Update,
            (menu::keyboard_nav, menu::update, input::read_desktop, sim::step, view::chase)
                .chain()
                .before(feed_sounds),
        );
    }
    app.add_systems(Update, (feed_sounds, skater::bind, skater::animate, menu::draw).chain());
    app
}

/// Turn this frame's engine output into sound cues.
fn feed_sounds(sim: Res<CleanSkate>, mut sounds: ResMut<crate::skate_audio::clean::Sk8Sounds>) {
    use crate::skate_audio::clean::Sk8Cue;
    let s = &sim.current;
    sounds.position = Vec3::from_array(s.position);
    sounds.speed = s.speed;
    sounds.grounded = s.mode.grounded();
    sounds.grinding = s.mode == Mode::Grind;
    sounds.powersliding = s.mode == Mode::Slide;
    sounds.paused = sim.paused;
    sounds.cues.clear();
    for e in &sim.events {
        let cue = match e.kind {
            EventKind::Pop => Some(Sk8Cue::Pop),
            EventKind::Land => Some(Sk8Cue::Land(Vec3::from_array(s.velocity).length())),
            EventKind::Push => Some(Sk8Cue::Push),
            EventKind::Bail => Some(Sk8Cue::Bail),
            EventKind::Bank => Some(Sk8Cue::Chime),
            _ => None,
        };
        sounds.cues.extend(cue);
    }
    // A flip starts when the trick name appears in the air.
    if s.flip_progress.is_some() && sim.previous.flip_progress.is_none() {
        sounds.cues.push(Sk8Cue::Flip);
    }
}
