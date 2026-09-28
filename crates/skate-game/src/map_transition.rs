//! Transactional world replacement: prepare without mutating the live session,
//! then commit at a schedule boundary with gameplay suspended.
use crate::{
    config::Config,
    map_library::Entry,
    map_render::PreparedScene,
    physics::{GamePhysics, PlayerControls, SkaterRuntime},
};
use bevy::prelude::*;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};

/// Persistent character customisation can reapply after this set, while the
/// transition still holds the loading overlay and gameplay remains suspended.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MapTransitionSet;
#[derive(Message)]
pub(crate) struct WorldChanged {
    pub generation: u64,
}

#[cfg(test)]
#[path = "tests/map_transition.rs"]
mod tests;

#[derive(Resource, Debug)]
pub(crate) struct CurrentMap {
    pub path: Option<PathBuf>,
    pub name: String,
    pub spawn: [f32; 3],
    pub heading: f32,
    pub generation: u64,
}
impl CurrentMap {
    fn from_package(path: Option<PathBuf>, map: Option<&skate_data::skate_map::SkateMap>) -> Self {
        Self {
            path,
            name: map.map_or_else(|| "Test world".into(), |m| m.name.clone()),
            spawn: map.map_or([0., crate::physics::ground::HEIGHT, 0.], |m| m.spawn),
            heading: map.map_or(0., |m| m.heading),
            generation: 0,
        }
    }
}

struct PreparedWorld {
    map_fingerprint: u64,
    scene: PreparedScene,
    physics: GamePhysics,
    skater: SkaterRuntime,
    controls: PlayerControls,
    camera: crate::camera::CameraRuntime,
    metadata: CurrentMap,
    retail: bool,
    difficulty: crate::difficulty::Difficulty,
}

enum Phase {
    Idle,
    Requested(Entry),
    Loading {
        entry: Entry,
        progress: Arc<AtomicU8>,
        job: JoinHandle<Result<PreparedWorld, String>>,
    },
    // Let extraction see the committed scene before releasing the pause menu.
    Publishing {
        frames: u8,
        notice: String,
    },
}
#[derive(Resource)]
pub(crate) struct MapTransition {
    phase: Phase,
}
impl Default for MapTransition {
    fn default() -> Self {
        Self { phase: Phase::Idle }
    }
}
impl MapTransition {
    pub fn busy(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }
    pub fn request(&mut self, entry: Entry) {
        if !self.busy() {
            self.phase = Phase::Requested(entry);
        }
    }
    pub fn label(&self) -> String {
        match &self.phase {
            Phase::Idle => String::new(),
            Phase::Requested(entry) => format!("Loading {} — preparing…", entry.label),
            Phase::Loading {
                entry, progress, ..
            } => format!(
                "Loading {} — {}…",
                entry.label,
                match progress.load(Ordering::Relaxed) {
                    0 => "reading and validating map",
                    1 => "building collision and rendering",
                    2 => "initializing skater, camera and rendering",
                    _ => "finishing meshes, textures and sky",
                }
            ),
            Phase::Publishing { .. } => "Loading — publishing the new world…".into(),
        }
    }
}

pub(crate) struct MapTransitionPlugin;
impl Plugin for MapTransitionPlugin {
    fn build(&self, app: &mut App) {
        let config = app.world().resource::<Config>();
        let current = CurrentMap::from_package(config.map_path.clone(), config.map.as_ref());
        app.insert_resource(current)
            .init_resource::<MapTransition>()
            .add_message::<WorldChanged>()
            .add_systems(
                PreUpdate,
                poll.in_set(MapTransitionSet)
                    .after(crate::graphics_menu::MenuInput)
                    .before(crate::input::poll_controllers),
            )
            .configure_sets(
                FixedUpdate,
                (
                    crate::app::SimulationSet::Input,
                    crate::app::SimulationSet::Controls,
                    crate::app::SimulationSet::Physics,
                )
                    .run_if(crate::graphics_menu::gameplay_active),
            );
    }
}

fn start(world: &World, entry: Entry) -> Result<Phase, String> {
    let config = world.resource::<Config>();
    let root = config.asset_root.clone();
    let difficulty = config.difficulty;
    let graphs = world
        .resource::<crate::graph_runtime::StockGraphs>()
        .clone();
    let source = world.resource::<SkaterRuntime>().animation.source.clone();
    let preferences = world.resource::<PlayerControls>().preferences;
    let mut scene = PreparedScene::new(world);
    let selected = entry.clone();
    let progress = Arc::new(AtomicU8::new(0));
    let stage = progress.clone();
    let job = std::thread::Builder::new().name("map-loader".into()).spawn(move || {
        let _span = info_span!("load_map_transition").entered();
        let started = Instant::now();
        let map = info_span!("read_map").in_scope(|| selected.path.as_deref().map(skate_data::skate_map::SkateMap::load).transpose())?;
        let map_fingerprint = crate::config::map_fingerprint(selected.path.as_deref())?;
        let read_time = started.elapsed();
        let validation_started = Instant::now();
        if let Some(map) = &map { crate::skate_world::validate_runtime(map)?; }
        let validation_time = validation_started.elapsed();
        let metadata = CurrentMap::from_package(selected.path, map.as_ref());
        let retail = map.as_ref().is_some_and(|m| crate::retail_render::RetailScene::for_map(m));
        // Both builders only read the decoded package. Reserve render handles
        // on a second worker while the first constructs fresh simulation state;
        // neither publishes to the live world until both have succeeded.
        let (physics, skater, controls, camera, simulation_time, render_time) =
            std::thread::scope(|scope| -> Result<_, String> {
                let rendering = std::thread::Builder::new().name("map-render-loader".into())
                    .spawn_scoped(scope, || {
                        let render_started = Instant::now();
                        scene.prepare(map.as_ref(), &root);
                        render_started.elapsed()
                    }).map_err(|e| format!("Could not start render loader: {e}"))?;
                let simulation_started = Instant::now();
                let simulation = (|| -> Result<_, String> {
                    stage.store(1, Ordering::Relaxed);
                    let physics = info_span!("load_physics").in_scope(|| GamePhysics::load_with_difficulty(&root, map.as_ref(), difficulty))?;
                    stage.store(2, Ordering::Relaxed);
                    let skater = SkaterRuntime::load_for_world(&root, &graphs, &physics, difficulty.key(), Some(source))?;
                    let mut controls = PlayerControls::load(&root)?;
                    controls.preferences = preferences;
                    let camera = crate::camera::CameraRuntime::load(&root)?;
                    Ok((physics, skater, controls, camera))
                })();
                let simulation_time = simulation_started.elapsed();
                stage.store(3, Ordering::Relaxed);
                // Explicitly join even on a simulation error: no background
                // render work or reserved scene can outlive a failed request.
                let render_time = rendering.join()
                    .map_err(|_| "Map render preparation failed unexpectedly".to_string())?;
                let (physics, skater, controls, camera) = simulation?;
                Ok((physics, skater, controls, camera, simulation_time, render_time))
            })?;
        eprintln!("MAP_LOAD_TIMING name={:?} read_ms={} validation_ms={} simulation_ms={} render_ms={} prepare_ms={} parallel=true",
            metadata.name, read_time.as_millis(), validation_time.as_millis(),
            simulation_time.as_millis(), render_time.as_millis(), started.elapsed().as_millis());
        // Drop the decoded package on this worker. Physics and rendering now
        // own their data; retaining it would double large-city CPU memory.
        Ok(PreparedWorld { map_fingerprint, scene, physics, skater, controls, camera, metadata, retail, difficulty })
    }).map_err(|e| format!("Could not start map loader: {e}"))?;
    Ok(Phase::Loading {
        entry,
        progress,
        job,
    })
}

fn poll(world: &mut World) {
    let ready = match &world.resource::<MapTransition>().phase {
        Phase::Idle => false,
        Phase::Loading { job, .. } => job.is_finished(),
        _ => true,
    };
    if !ready {
        return;
    }
    let phase = std::mem::replace(
        &mut world.resource_mut::<MapTransition>().phase,
        Phase::Idle,
    );
    let next = match phase {
        Phase::Requested(entry) => match start(world, entry) {
            Ok(phase) => phase,
            Err(error) => {
                failed(world, error);
                Phase::Idle
            }
        },
        Phase::Loading { job, .. } => match job
            .join()
            .unwrap_or_else(|_| Err("Map loader failed unexpectedly".into()))
        {
            Ok(prepared) => {
                let mut notice = commit(world, prepared);
                let config = world.resource::<Config>();
                match crate::map_library::save_default(
                    &config.asset_root,
                    config.map_path.as_deref(),
                ) {
                    Ok(()) => notice.push_str(" Default map saved."),
                    Err(error) => notice.push_str(&format!(" Could not save default: {error}")),
                }
                Phase::Publishing { frames: 3, notice }
            }
            Err(error) => {
                failed(world, error);
                Phase::Idle
            }
        },
        Phase::Publishing { frames, notice } if frames > 0 => Phase::Publishing {
            frames: frames - 1,
            notice,
        },
        Phase::Publishing { notice, .. } => {
            world
                .resource_mut::<crate::graphics_menu::Menu>()
                .transition_finished(notice, true);
            world.resource_mut::<Time<Virtual>>().unpause();
            Phase::Idle
        }
        Phase::Idle => Phase::Idle,
    };
    world.resource_mut::<MapTransition>().phase = next;
}

fn failed(world: &mut World, error: String) {
    warn!("MAP_TRANSITION_FAILED {error}");
    world.resource_mut::<crate::graphics_menu::Menu>()
        .transition_finished(format!("Could not load map: {error}\nPrevious world retained. Choose another map or Resume."), false);
}

fn commit(world: &mut World, mut prepared: PreparedWorld) -> String {
    let publication_started = Instant::now();
    prepared.metadata.generation = world.resource::<CurrentMap>().generation + 1;
    // All fallible decoding/validation/construction finished before this point.
    // No simulation system can observe a mixture of the two worlds.
    crate::map_render::MapAssets::retire(world);
    prepared.scene.publish(world);
    crate::camera::set_world_environment(world, prepared.retail);
    world.insert_resource(crate::retail_render::RetailScene(prepared.retail));
    world.insert_resource(crate::grind_world::GrindGeometry::for_world(
        prepared.metadata.path.is_none(),
    ));
    world.insert_resource(Time::<Fixed>::from_duration(prepared.physics.period()));
    let root_transform = Transform::from_matrix(crate::animation::native_matrix(
        prepared.skater.animated_skeleton.roots.animation_to_world,
    ));
    for mut transform in world
        .query_filtered::<&mut Transform, With<crate::world::PlayerRoot>>()
        .iter_mut(world)
    {
        *transform = root_transform;
    }
    world.insert_resource(prepared.physics);
    world.insert_resource(prepared.skater);
    world.insert_resource(prepared.controls);
    world.insert_resource(prepared.camera);
    world.insert_resource(crate::presentation::Presentation::default());
    world.insert_resource(crate::replay::Replay::default());
    world.insert_resource(crate::input::ControllerInput::default());
    world.insert_resource(crate::input::PublishedTickInput::default());
    world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
    world.resource_mut::<ButtonInput<MouseButton>>().reset_all();
    let mut config = world.resource_mut::<Config>();
    config.map_path = prepared.metadata.path.clone();
    config.map_fingerprint = prepared.map_fingerprint;
    config.difficulty = prepared.difficulty;
    let notice = format!("Loaded {}.", prepared.metadata.name);
    info!(
        "MAP_TRANSITION_COMMITTED generation={} name={:?} spawn={:?} heading={} pid={}",
        prepared.metadata.generation,
        prepared.metadata.name,
        prepared.metadata.spawn,
        prepared.metadata.heading,
        std::process::id()
    );
    if let Some(mut messages) = world.get_resource_mut::<Messages<WorldChanged>>() {
        messages.write(WorldChanged {
            generation: prepared.metadata.generation,
        });
    }
    world.insert_resource(prepared.metadata);
    eprintln!(
        "MAP_PUBLISH_TIMING cpu_ms={}",
        publication_started.elapsed().as_millis()
    );
    notice
}
