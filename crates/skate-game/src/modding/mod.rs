//! Main-thread SDK adapter. Lua never receives World, entity IDs, or asset handles.
mod menu;
pub(crate) mod network;
mod panel;
pub(crate) mod vehicles;
use bevy::prelude::*;
pub(crate) use menu::ModMenu;
pub(crate) use panel::EnabledPanel;
use serde_json::json;
use skate_mods::{Command, Manager};
use std::collections::BTreeMap;

#[derive(Resource)]
pub(crate) struct Mods {
    pub manager: Manager,
    animation_info: serde_json::Value,
    trainer: Option<(String, skate_mods::TrainerTuning)>,
    owned: BTreeMap<(String, String), Owned>,
    generation: u64,
    last_bail: bool,
    last_state: u32,
    last_grind: bool,
}
struct Owned {
    entity: Entity,
    mesh: Option<AssetId<Mesh>>,
    material: Option<AssetId<StandardMaterial>>,
}
pub(crate) struct ModdingPlugin;
impl Plugin for ModdingPlugin {
    fn build(&self, app: &mut App) {
        let root = package_root();
        if let Err(e) = std::fs::create_dir_all(&root) {
            warn!("Cannot create mods folder {}: {e}", root.display());
        }
        info!("Mod packages folder: {}", root.display());
        let settings = std::env::var_os("SKATE3_MOD_SETTINGS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                app.world()
                    .resource::<crate::config::Config>()
                    .asset_root
                    .parent()
                    .unwrap()
                    .join("settings/mods")
            });
        let frames = &app
            .world()
            .resource::<crate::physics::SkaterRuntime>()
            .animation
            .evaluator
            .frames;
        let slots: BTreeMap<_, _> = [
            "R_ANTIC_OLLIE_N_0_INTO",
            "R_ANTIC_OLLIE_N_0_CYC",
            "R_ANTIC_360SHUVIT_N_0_CYC",
            "360FLIP_D_HIGH_G",
            "360FLIP_D_HIGH_A",
            "360FLIP_D_LOW_G",
            "360FLIP_D_LOW_A",
        ]
        .into_iter()
        .filter_map(|name| {
            frames.clip(name).ok().map(|c| {
                (
                    name,
                    json!({"fps":f32::from_bits(c.fps_bits),"frame_count":c.frames.len()}),
                )
            })
        })
        .collect();
        let animation_info = json!({"bone_names":frames.bone_names,"slots":slots});
        app.insert_resource(Mods {
            animation_info,
            trainer: None,
            manager: Manager::new(root, settings),
            owned: BTreeMap::new(),
            generation: u64::MAX,
            last_bail: false,
            last_state: 0,
            last_grind: false,
        })
        .init_resource::<ModMenu>()
        .add_systems(
            PreUpdate,
            maintenance.after(crate::map_transition::MapTransitionSet),
        )
        .add_systems(
            FixedUpdate,
            fixed
                .after(crate::app::SimulationSet::Physics)
                .run_if(crate::graphics_menu::gameplay_active),
        )
        .add_systems(Update, update.after(crate::app::FrameSet::Animation));
        vehicles::install(app);
        network::install(app);
        menu::install(app);
        panel::install(app);
    }
}
fn snapshot(world: &World) -> serde_json::Value {
    let s = world.resource::<crate::physics::SkaterRuntime>();
    let p = &s.player_input.physical;
    let map = world.resource::<crate::map_transition::CurrentMap>();
    let physics = world.resource::<crate::physics::GamePhysics>();
    let keys: BTreeMap<_, _> = world
        .resource::<ButtonInput<KeyCode>>()
        .get_pressed()
        .map(|k| (format!("{k:?}"), true))
        .collect();
    let (grinding, grind_name, grind_kind, grind_distance) = s.mod_grind();
    let actions = *world
        .resource::<crate::input::PublishedTickInput>()
        .0
        .actions()
        .values();
    let vehicle_pose = world.resource::<vehicles::Vehicles>().player_pose();
    let player_position = vehicle_pose.map(|p| p.0).unwrap_or_else(|| {
        s.animated_skeleton.roots.animation_to_world[3][..3]
            .try_into()
            .unwrap()
    });
    json!({"player":{"position":player_position,"velocity": &p.skateboard.vector_80.map(f32::from_bits)[..3],"heading":vehicle_pose.map(|p|p.1).unwrap_or_else(||s.animated_skeleton.roots.animation_to_world[2][0].atan2(s.animated_skeleton.roots.animation_to_world[2][2])),"on_board":p.state.category_12!=500,"state":p.state.state_16,"category":p.state.category_12,"bailing":physics.board_wiping_out,"grind":{"active":grinding,"name":grind_name,"kind":grind_kind,"distance":grind_distance}},
        "network":network::snapshot(world),"vehicles":vehicles::snapshot(world),"vehicle_input":vehicles::input(world),
        "animation":world.resource::<Mods>().animation_info,
        "map":{"name":map.name,"generation":map.generation},"tick":physics.ticks,"keys":keys,"actions":actions,
        "paused":world.resource::<crate::graphics_menu::Menu>().open,"replay":world.resource::<crate::replay::Replay>().active})
}
fn maintenance(world: &mut World) {
    let snap = snapshot(world);
    world.resource_scope(|world, mut mods: Mut<Mods>| {
        mods.manager.snapshot = snap;
        let generation = world
            .resource::<crate::map_transition::CurrentMap>()
            .generation;
        if mods.generation != generation {
            vehicles::clear(world);
            network::clear(world);
            let ids: Vec<_> = mods.owned.keys().cloned().collect();
            for key in ids {
                retire(world, &mut mods, &key);
            }
            world
                .resource::<crate::physics::SkaterRuntime>()
                .animation
                .evaluator
                .clear_mod_clips();
            mods.generation = generation;
            mods.manager.commands.clear();
            let map = mods.manager.snapshot["map"].clone();
            mods.manager
                .dispatch("on_event", json!({"name":"world_changed","map":map}));
        }
        let events = std::mem::take(&mut world.resource_mut::<vehicles::Vehicles>().events);
        for event in events {
            mods.manager.dispatch("on_event", event);
        }
        mods.manager.scan(false);
        apply(world, &mut mods);
    });
}
fn fixed(world: &mut World) {
    if world.resource::<crate::replay::Replay>().active {
        return;
    }
    let snap = snapshot(world);
    let dt = world.resource::<Time<Fixed>>().delta_secs_f64();
    world.resource_scope(|world, mut mods: Mut<Mods>| {
        mods.manager.snapshot = snap;
        let bail = world
            .resource::<crate::physics::GamePhysics>()
            .board_wiping_out;
        let state = world
            .resource::<crate::physics::SkaterRuntime>()
            .player_input
            .physical
            .state
            .state_16;
        if bail != mods.last_bail {
            mods.last_bail = bail;
            mods.manager
                .dispatch("on_event", json!({"name":"bail_changed","bailing":bail}));
        }
        if state != mods.last_state {
            let previous = mods.last_state;
            mods.last_state = state;
            mods.manager.dispatch(
                "on_event",
                json!({"name":"player_state_changed","previous":previous,"state":state}),
            );
        }
        let grinding = world
            .resource::<crate::physics::SkaterRuntime>()
            .mod_grind()
            .0;
        if grinding != mods.last_grind {
            mods.last_grind = grinding;
            let grind = mods.manager.snapshot["player"]["grind"].clone();
            mods.manager
                .dispatch("on_event", json!({"name":"grind_changed","grind":grind}));
        }
        mods.manager.dispatch("on_fixed_update", json!({"dt":dt}));
        apply(world, &mut mods);
    });
}
fn update(world: &mut World) {
    let snap = snapshot(world);
    let paused =
        snap["paused"].as_bool().unwrap_or(true) || snap["replay"].as_bool().unwrap_or(false);
    let dt = world.resource::<Time<Real>>().delta_secs_f64().min(0.25);
    world.resource_scope(|world, mut mods: Mut<Mods>| {
        mods.manager.snapshot = snap;
        if !paused {
            mods.manager.dispatch("on_update", json!({"dt":dt}));
        }
        apply(world, &mut mods);
    });
}
fn retire(world: &mut World, mods: &mut Mods, key: &(String, String)) {
    if let Some(owned) = mods.owned.remove(key) {
        world.despawn(owned.entity);
        if let Some(id) = owned.mesh {
            world.resource_mut::<Assets<Mesh>>().remove(id);
        }
        if let Some(id) = owned.material {
            world.resource_mut::<Assets<StandardMaterial>>().remove(id);
        }
    }
}
fn apply(world: &mut World, mods: &mut Mods) {
    let retired = std::mem::take(&mut mods.manager.retired);
    if mods
        .trainer
        .as_ref()
        .is_some_and(|(id, _)| retired.contains(id))
    {
        mods.trainer = None;
    }
    world.resource_mut::<crate::physics::GamePhysics>().trainer =
        mods.trainer.as_ref().map(|(_, t)| *t).unwrap_or_default();
    for id in &retired {
        network::retire(world, id);
        vehicles::retire(world, id);
        world
            .resource::<crate::physics::SkaterRuntime>()
            .animation
            .evaluator
            .remove_mod_clips(id);
    }
    let keys: Vec<_> = mods
        .owned
        .keys()
        .filter(|(id, _)| retired.contains(id))
        .cloned()
        .collect();
    for key in keys {
        retire(world, mods, &key);
    }
    let mut batches = BTreeMap::<String, Vec<Command>>::new();
    for (id, command) in std::mem::take(&mut mods.manager.commands) {
        batches.entry(id).or_default().push(command);
    }
    for (id, commands) in batches {
        if !mods.manager.packages.get(&id).is_some_and(|p| p.running()) {
            continue;
        }
        let result = (|| {
            let mut keys: std::collections::BTreeSet<_> = mods
                .owned
                .keys()
                .filter(|(owner, _)| owner == &id)
                .map(|(_, key)| key.clone())
                .collect();
            let mut teleport = None;
            for c in &commands {
                match c {
                    Command::Remove { key } => {
                        keys.remove(key);
                    }
                    Command::Overlay { key, .. } | Command::Cube { key, .. } => {
                        keys.insert(key.clone());
                        if keys.len() > 64 {
                            return Err("64 owned objects per mod maximum".into());
                        }
                    }
                    Command::Teleport { .. } => {
                        if teleport.is_some() {
                            return Err("Only one teleport per mod command batch".into());
                        }
                        teleport = Some(c.clone());
                        teleport_ready(world)?;
                    }
                    _ => {}
                }
            }
            // Teleport is an irreversible native request. Submit only after all
            // reversible installations succeed, so a later bad asset cannot leave it pending.
            for command in commands {
                if !matches!(command, Command::Teleport { .. }) {
                    apply_one(world, mods, &id, command)?;
                }
            }
            if let Some(command) = teleport {
                apply_one(world, mods, &id, command)?;
            }
            Ok::<(), String>(())
        })();
        if let Err(e) = result {
            warn!("Lua mod {id}: {e}");
            mods.manager.fail(&id, e);
            vehicles::retire(world, &id);
            if mods.trainer.as_ref().is_some_and(|(owner, _)| owner == &id) {
                mods.trainer = None;
                world.resource_mut::<crate::physics::GamePhysics>().trainer = Default::default();
            }
            world
                .resource::<crate::physics::SkaterRuntime>()
                .animation
                .evaluator
                .remove_mod_clips(&id);
            let keys: Vec<_> = mods
                .owned
                .keys()
                .filter(|(owner, _)| owner == &id)
                .cloned()
                .collect();
            for key in keys {
                retire(world, mods, &key);
            }
        }
    }
    // Stable text order independent of Bevy entity allocation or previous reloads.
    let mut row = 0;
    for owned in mods.owned.values() {
        if let Some(mut node) = world.get_mut::<Node>(owned.entity) {
            node.top = px(16. + row as f32 * 28.);
            row += 1;
        }
    }
}
fn teleport_ready(world: &World) -> Result<(), String> {
    if world.resource::<vehicles::Vehicles>().occupied() {
        return Err("Exit the vehicle before teleporting the skater".into());
    }
    if world
        .resource::<crate::map_transition::MapTransition>()
        .busy()
        || world.resource::<crate::replay::Replay>().active
    {
        return Err("Teleport unavailable during map loading or replay".into());
    }
    let s = world.resource::<crate::physics::SkaterRuntime>();
    if s.player_input.physical.state.flag_69 != 0
        || s.player_input.physical.state.state_16 == 702
        || s.player_input.pending_teleport().is_some()
    {
        return Err("Teleport unavailable during reset".into());
    }
    Ok(())
}
fn apply_one(world: &mut World, mods: &mut Mods, id: &str, command: Command) -> Result<(), String> {
    let key = match &command {
        Command::Cube { key, .. } | Command::Overlay { key, .. } | Command::Remove { key } => {
            Some((id.to_owned(), key.clone()))
        }
        _ => None,
    };
    if let Some(key) = &key {
        if !matches!(command, Command::Remove { .. })
            && !mods.owned.contains_key(key)
            && mods.owned.keys().filter(|(owner, _)| owner == id).count() >= 64
        {
            return Err("64 owned objects per mod maximum".into());
        }
    }
    if !id.starts_with('@') {
        network::record(world, id, &command)?;
    }
    match command {
        Command::NetworkState { .. } => {}
        command @ (Command::VehicleTune { .. }
        | Command::VehicleSpawn { .. }
        | Command::VehicleRemove { .. }
        | Command::VehicleEnter { .. }
        | Command::VehicleExit { .. }
        | Command::VehicleReset { .. }
        | Command::VehicleControl { .. }) => {
            vehicles::command(world, &mods.manager.packages[id].root, id, command)?;
        }
        Command::Trainer { tuning } => {
            if mods.trainer.as_ref().is_some_and(|(owner, _)| owner != id) {
                return Err("Native trainer controls are already owned by another mod".into());
            }
            mods.trainer = Some((id.to_owned(), tuning));
            world.resource_mut::<crate::physics::GamePhysics>().trainer = tuning;
        }
        Command::Animation { path } => {
            let root = &mods.manager.packages[id].root;
            let bytes = skate_mods::read_bounded(root, &path, 4 * 1024 * 1024)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
            world
                .resource::<crate::physics::SkaterRuntime>()
                .animation
                .evaluator
                .install_mod_clips(id, text)?;
        }
        Command::Log { text } => info!("Lua [{id}]: {text}"),
        Command::Remove { .. } => retire(world, mods, &key.unwrap()),
        Command::Overlay { text, .. } => {
            let key = key.unwrap();
            if let Some(owned) = mods.owned.get(&key) {
                if let Some(mut t) = world.get_mut::<Text>(owned.entity) {
                    **t = text;
                    return Ok(());
                }
            }
            retire(world, mods, &key);
            let entity = world
                .spawn((
                    Text::new(text),
                    TextFont {
                        font_size: 19.,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    GlobalZIndex(3),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(16.),
                        ..default()
                    },
                ))
                .id();
            mods.owned.insert(
                key,
                Owned {
                    entity,
                    mesh: None,
                    material: None,
                },
            );
        }
        Command::Cube {
            position,
            size,
            color,
            ..
        } => {
            let key = key.unwrap();
            if let Some(owned) = mods.owned.get(&key) {
                if owned.mesh.is_some() {
                    *world.get_mut::<Transform>(owned.entity).unwrap() =
                        Transform::from_translation(Vec3::from_array(position))
                            .with_scale(Vec3::from_array(size));
                    if let Some(material) = owned.material {
                        if let Some(m) = world
                            .resource_mut::<Assets<StandardMaterial>>()
                            .get_mut(material)
                        {
                            m.base_color = Color::srgb(color[0], color[1], color[2]);
                        }
                    }
                    return Ok(());
                }
            }
            retire(world, mods, &key);
            let mesh = world
                .resource_mut::<Assets<Mesh>>()
                .add(Cuboid::new(1., 1., 1.));
            let material = world
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    base_color: Color::srgb(color[0], color[1], color[2]),
                    ..default()
                });
            let owned = Owned {
                entity: world
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(Vec3::from_array(position))
                            .with_scale(Vec3::from_array(size)),
                    ))
                    .id(),
                mesh: Some(mesh.id()),
                material: Some(material.id()),
            };
            mods.owned.insert(key, owned);
        }
        Command::Teleport {
            position,
            heading,
            on_board,
        } => {
            if world
                .resource::<crate::map_transition::MapTransition>()
                .busy()
                || world.resource::<crate::replay::Replay>().active
            {
                return Err("Teleport unavailable during map loading or replay".into());
            }
            let mut s = world.resource_mut::<crate::physics::SkaterRuntime>();
            if s.player_input.physical.state.flag_69 != 0
                || s.player_input.physical.state.state_16 == 702
                || s.player_input.pending_teleport().is_some()
            {
                return Err("Teleport unavailable during reset".into());
            }
            let (sin, cos) = heading.sin_cos();
            let transform = [
                [cos, 0., -sin, 0.],
                [0., 1., 0., 0.],
                [sin, 0., cos, 0.],
                [position[0], position[1], position[2], 0.],
            ];
            s.player_input
                .request_teleport(transform)
                .map_err(|e| format!("Teleport rejected: {e}"))?;
            s.teleport_state.request_manual(transform, on_board);
        }
    }
    Ok(())
}

pub(crate) fn package_root() -> std::path::PathBuf {
    std::env::var_os("SKATE3_MODS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("mods")))
                .unwrap_or_else(|| "mods".into())
        })
}
