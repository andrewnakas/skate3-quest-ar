//! Persistent local visual characters. Import and ECS publication are separate transactions.
use bevy::{
    asset::{
        LoadState,
        io::{AssetSourceBuilder, file::FileAssetReader},
    },
    gltf::Gltf,
    input::mouse::{MouseScrollUnit, MouseWheel},
    mesh::skinning::SkinnedMesh,
    prelude::*,
    scene::SceneInstance,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Instant,
};

const COLUMNS: usize = 3;
const CARD_HEIGHT: f32 = 246.;
const ROW_HEIGHT: f32 = CARD_HEIGHT + 10.;
#[derive(Clone, Deserialize)]
pub(crate) struct Entry {
    version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    native: Option<NativeCharacter>,
    #[serde(skip)]
    asset_prefix: String,
}
impl Entry {
    fn asset_path(&self, file: &str) -> String {
        let prefix = if self.asset_prefix.is_empty() {
            "characters://"
        } else {
            &self.asset_prefix
        };
        format!("{prefix}entries/{}/{file}", self.id)
    }
}
#[derive(Clone, Deserialize)]
struct NativeCharacter {
    key: String,
    category: String,
}
pub(crate) fn native_animation_style(key: &str) -> &'static str {
    // TU3 GetCACSettings 82590BE0..82590DBC; other Marquees use Aggressive.
    match key {
        "danny_way" => "DannyWay",
        "mike_carroll" => "MikeCarroll",
        "pj_ladd" => "PJLadd",
        "jason_dill" => "JasonDill",
        "jerry_hsu" => "JerryHsu",
        "rob_dyrdek" => "RobDyrdek",
        _ => "Aggressive",
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Selection {
    version: u32,
    selected: Option<String>,
}
struct Pending {
    id: String,
    root: Entity,
    asset: Handle<Gltf>,
    started: Instant,
}
struct Import {
    child: Child,
    result: PathBuf,
    started: Instant,
}
#[derive(Component)]
pub(crate) struct CustomModelRoot;
#[derive(Component)]
pub(crate) struct NativeModelRoot(pub String);
#[derive(Component)]
struct Panel;
#[derive(Component)]
struct Action(usize);
#[derive(Component)]
struct ModelScroll;

#[derive(Resource, Default)]
pub(crate) struct CustomModels {
    pub open: bool,
    just_opened: bool,
    pub active: Option<String>,
    entries: Vec<Entry>,
    directory: PathBuf,
    native_directory: PathBuf,
    native_prefix: String,
    request: Option<Option<String>>,
    pending: Option<Pending>,
    active_root: Option<Entity>,
    stock_visibility: Vec<(Entity, Visibility)>,
    import: Option<Import>,
    status: String,
    selected: usize,
    scroll: f32,
    filter: usize,
    dirty: bool,
}
impl CustomModels {
    pub(crate) fn online_selection(&self) -> Option<(Option<String>, PathBuf)> {
        let e = self
            .entries
            .iter()
            .find(|e| Some(&e.id) == self.active.as_ref())?;
        let directory = if e.asset_prefix.is_empty() {
            &self.directory
        } else {
            &self.native_directory
        };
        Some((
            e.native.as_ref().map(|n| n.key.clone()),
            directory.join("entries").join(&e.id).join("character.glb"),
        ))
    }
    pub(crate) fn online_native_path(&self, key: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|e| e.native.as_ref().is_some_and(|n| n.key == key))
            .map(|e| e.asset_path("character.glb"))
    }
    pub(crate) fn native_style(&self) -> Option<&'static str> {
        self.active
            .as_ref()
            .and_then(|id| self.entries.iter().find(|e| &e.id == id))
            .and_then(|e| e.native.as_ref())
            .map(|n| native_animation_style(&n.key))
    }
    fn visible_entries(&self) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| match self.filter {
                1 => e.native.as_ref().is_some_and(|n| n.category == "Pro"),
                2 => e.native.as_ref().is_some_and(|n| n.category == "Special"),
                3 => e.native.is_none(),
                _ => true,
            })
            .collect()
    }
    pub fn begin(&mut self) {
        self.open = true;
        self.just_opened = true;
        self.dirty = true;
    }
    pub fn request_stock(&mut self) {
        self.request = Some(None);
    }
}
pub(crate) fn library_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Skate3RustEngine/custom-characters")
}
pub(crate) fn register_source(app: &mut App) {
    let cache = crate::multiplayer::appearance::cache_directory().to_owned();
    app.register_asset_source(
        "online-characters",
        AssetSourceBuilder::new(move || Box::new(FileAssetReader::new(cache.clone()))),
    );
    let directory = library_path();
    app.register_asset_source(
        "characters",
        AssetSourceBuilder::new(move || Box::new(FileAssetReader::new(directory.clone()))),
    );
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn discover(directory: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = std::fs::read_dir(directory.join("entries"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|folder| {
            let id = folder.file_name().to_string_lossy().into_owned();
            if !valid_id(&id) || !folder.file_type().ok()?.is_dir() {
                return None;
            }
            let path = folder.path();
            let data = std::fs::read(path.join("manifest.json")).ok()?;
            if data.len() > 65536 {
                return None;
            }
            let mut entry: Entry = serde_json::from_slice(&data).ok()?;
            if entry.version != 1
                || entry.id != id
                || !path.join("character.glb").is_file()
                || !path.join("preview.png").is_file()
            {
                return None;
            }
            entry.name = entry
                .name
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect();
            Some(entry)
        })
        .collect();
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    entries
}
fn discover_all(directory: &Path, native_directory: &Path, native_prefix: &str) -> Vec<Entry> {
    let mut entries = discover(directory);
    for mut native in discover(native_directory)
        .into_iter()
        .filter(|e| e.native.is_some())
    {
        entries.retain(|entry| entry.id != native.id);
        native.asset_prefix = native_prefix.to_owned();
        entries.push(native);
    }
    entries.sort_by_key(|e| (e.name.to_lowercase(), e.id.clone()));
    entries
}
fn save_selection(directory: &Path, selected: Option<String>) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let temporary = directory.join("selection.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec(&Selection {
            version: 1,
            selected,
        })
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, directory.join("selection.json")).map_err(|e| e.to_string())
}
pub(crate) struct CustomModelsPlugin;
impl Plugin for CustomModelsPlugin {
    fn build(&self, app: &mut App) {
        let directory = library_path();
        let assets = &app.world().resource::<crate::config::Config>().asset_root;
        let native_directory =
            crate::customiser_parts::asset_directory(assets).join("native-roster");
        let native_prefix = format!(
            "{}/",
            native_directory
                .strip_prefix(assets)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        );
        let entries = discover_all(&directory, &native_directory, &native_prefix);
        let saved = std::fs::read(directory.join("selection.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Selection>(&b).ok())
            .filter(|s| s.version == 1)
            .and_then(|s| s.selected)
            .filter(|id| entries.iter().any(|e| &e.id == id));
        app.insert_resource(CustomModels {
            open: false,
            just_opened: false,
            active: None,
            entries,
            directory,
            native_directory,
            native_prefix,
            request: saved.map(Some),
            pending: None,
            active_root: None,
            stock_visibility: vec![],
            import: None,
            status: String::new(),
            selected: 0,
            scroll: 0.,
            filter: 0,
            dirty: true,
        })
        .add_systems(PreUpdate, interact.after(crate::graphics_menu::MenuInput))
        .add_systems(
            Update,
            (poll_import, publish, draw)
                .chain()
                .before(crate::customiser_parts::update)
                .before(crate::app::FrameSet::Animation),
        )
        .add_systems(Last, stop_import);
    }
}
fn interact(
    mut state: ResMut<CustomModels>,
    keys: Res<ButtonInput<KeyCode>>,
    nav: Res<crate::customiser::Navigation>,
    buttons: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut wheel: MessageReader<MouseWheel>,
    mut scroll_nodes: Query<(&mut ScrollPosition, &ComputedNode), With<ModelScroll>>,
    config: Res<crate::config::Config>,
    manifest: Res<crate::assets::AssetManifest>,
) {
    if !state.open {
        wheel.clear();
        return;
    }
    for event in wheel.read() {
        let amount = if event.unit == MouseScrollUnit::Line {
            event.y * 48.
        } else {
            event.y
        };
        for (mut position, computed) in &mut scroll_nodes {
            let limit = ((computed.content_size().y - computed.size().y)
                * computed.inverse_scale_factor())
            .max(0.);
            state.scroll = (state.scroll - amount).clamp(0., limit);
            position.y = state.scroll;
        }
    }
    if state.just_opened {
        state.just_opened = false;
        return;
    }
    if keys.just_pressed(KeyCode::Escape) || nav.pressed & (0x2000 | 0x10) != 0 {
        state.open = false;
        state.dirty = true;
        return;
    }
    let count = state.visible_entries().len() + 4;
    let previous = state.selected;
    if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 {
        state.selected = if state.selected >= 4 + COLUMNS {
            state.selected - COLUMNS
        } else {
            state.selected.saturating_sub(1)
        };
    }
    if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 {
        state.selected =
            (state.selected + if state.selected >= 4 { COLUMNS } else { 1 }).min(count - 1);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) || nav.pressed & 4 != 0 {
        state.selected = state.selected.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::ArrowRight) || nav.pressed & 8 != 0 {
        state.selected = (state.selected + 1).min(count - 1);
    }
    if previous != state.selected {
        if state.selected >= 4 {
            let top = ((state.selected - 4) / COLUMNS) as f32 * ROW_HEIGHT;
            let height = scroll_nodes
                .iter()
                .next()
                .map(|(_, n)| n.size().y * n.inverse_scale_factor())
                .unwrap_or(400.);
            state.scroll = state
                .scroll
                .min(top)
                .max(top + CARD_HEIGHT - height)
                .max(0.);
        }
        state.dirty = true;
    }
    let mut action = if keys.just_pressed(KeyCode::Enter) || nav.pressed & 0x1000 != 0 {
        Some(state.selected)
    } else {
        None
    };
    for (interaction, button) in &buttons {
        if *interaction == Interaction::Pressed {
            action = Some(button.0);
        }
    }
    if let Some(action) = action {
        state.selected = action;
        match action {
            0 if state.import.is_none() => {
                state.status = match start_import(
                    &state.directory,
                    &config
                        .asset_root
                        .join(manifest.0.character_scene.split('#').next().unwrap()),
                ) {
                    Ok(job) => {
                        state.import = Some(job);
                        "Choose a Mixamo FBX in the file picker. Import may take a moment.".into()
                    }
                    Err(e) => e,
                };
            }
            1 => state.request_stock(),
            2 => {
                state.filter = (state.filter + 1) % 4;
                state.scroll = 0.;
            }
            3 => state.open = false,
            action if action >= 4 => {
                if let Some(entry) = state.visible_entries().get(action - 4) {
                    state.request = Some(Some(entry.id.clone()));
                }
            }
            _ => (),
        }
        state.dirty = true;
    }
}
fn start_import(directory: &Path, reference: &Path) -> Result<Import, String> {
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("Missing game directory")?
        .join("support/skate3setup.exe");
    if !executable.is_file() {
        return Err("Character importer is missing. Restore support/skate3setup.exe from the complete Windows package.".into());
    }
    let jobs = directory.join("jobs");
    std::fs::create_dir_all(&jobs).map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let result = jobs.join(format!("{}-{nonce}.json", std::process::id()));
    let mut command = Command::new(&executable);
    // Preserve optional calibration from earlier local Custom Models builds.
    let profile = executable
        .parent()
        .unwrap()
        .join("custom-models/calibration.json");
    command.arg("--character-import");
    if profile.is_file() {
        command.arg("--profile").arg(profile);
    }
    command
        .arg("--library-import")
        .arg(directory)
        .arg("--reference")
        .arg(reference)
        .arg("--result")
        .arg(&result)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("Could not open importer: {e}"))?;
    Ok(Import {
        child,
        result,
        started: Instant::now(),
    })
}
fn poll_import(mut state: ResMut<CustomModels>) {
    let Some(job) = state.import.as_mut() else {
        return;
    };
    if job.started.elapsed().as_secs() > 900 {
        let _ = job.child.kill();
    }
    let finished = match job.child.try_wait() {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => true,
    };
    if !finished {
        return;
    }
    let job = state.import.take().unwrap();
    let reply = std::fs::read(&job.result)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let _ = std::fs::remove_file(&job.result);
    state.entries = discover_all(
        &state.directory,
        &state.native_directory,
        &state.native_prefix,
    );
    state.status = match reply.as_ref().and_then(|r| r["status"].as_str()) {
        Some("cancelled") => "Import cancelled. Your character is unchanged.".into(),
        Some("ready") => {
            let id = reply.as_ref().unwrap()["id"].as_str().unwrap_or("");
            if let Some(index) = state.entries.iter().position(|e| e.id == id) {
                let id = state.entries[index].id.clone();
                state.filter = 0;
                state.selected = index + 4;
                state.scroll = (index / COLUMNS) as f32 * ROW_HEIGHT;
                state.request = Some(Some(id));
                "Imported. Loading character...".into()
            } else {
                "Import did not produce a valid library entry.".into()
            }
        }
        Some("error") => reply.as_ref().unwrap()["message"]
            .as_str()
            .unwrap_or("Import failed")
            .chars()
            .take(350)
            .collect(),
        _ => "The importer stopped before finishing. Your character is unchanged.".into(),
    };
    state.dirty = true;
}
fn restore_stock(
    state: &mut CustomModels,
    parts: &mut crate::customiser_parts::Parts,
    animation: &mut crate::animation::AnimationStatus,
    scenes: &mut Query<(Entity, &ChildOf, &mut Visibility), With<SceneRoot>>,
) {
    for (e, visibility) in state.stock_visibility.drain(..) {
        if let Ok((_, _, mut v)) = scenes.get_mut(e) {
            *v = visibility;
        }
    }
    state.active = None;
    parts.applied = serde_json::Value::Null;
    *animation = default();
}
fn publish(
    mut commands: Commands,
    mut state: ResMut<CustomModels>,
    server: Res<AssetServer>,
    skater: Res<crate::physics::SkaterRuntime>,
    mut animation: ResMut<crate::animation::AnimationStatus>,
    mut parts: ResMut<crate::customiser_parts::Parts>,
    roots: Query<Entity, With<crate::world::PlayerRoot>>,
    spawner: Res<SceneSpawner>,
    instances: Query<&SceneInstance>,
    skins: Query<(Entity, &SkinnedMesh)>,
    nodes: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
    mut scenes: Query<(Entity, &ChildOf, &mut Visibility), With<SceneRoot>>,
) {
    let Ok(player) = roots.single() else {
        return;
    };
    if let Some(request) = state.request.take() {
        if let Some(p) = state.pending.take() {
            commands.entity(p.root).despawn();
        }
        match request {
            None => {
                if let Some(root) = state.active_root.take() {
                    commands.entity(root).despawn();
                }
                // Visibility was owned by the alternate character while active.
                // Reapply the current outfit even if its saved JSON is unchanged.
                restore_stock(&mut state, &mut parts, &mut animation, &mut scenes);
                state.status = match save_selection(&state.directory, None) {
                    Ok(()) => "Stock skater restored.".into(),
                    Err(e) => format!("Restored; selection could not be saved: {e}"),
                };
            }
            Some(id)
                if state.active.as_ref() != Some(&id)
                    && state.entries.iter().any(|e| e.id == id) =>
            {
                let path = state
                    .entries
                    .iter()
                    .find(|e| e.id == id)
                    .unwrap()
                    .asset_path("character.glb");
                let asset = server.load(path.clone());
                let root = commands
                    .spawn((
                        CustomModelRoot,
                        Transform::default(),
                        Visibility::Hidden,
                        SceneRoot(server.load(GltfAssetLabel::Scene(0).from_asset(path))),
                    ))
                    .id();
                commands.entity(player).add_child(root);
                if let Some(native) = state
                    .entries
                    .iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.native.as_ref())
                {
                    commands
                        .entity(root)
                        .insert(NativeModelRoot(native.key.clone()));
                }
                state.pending = Some(Pending {
                    id,
                    root,
                    asset,
                    started: Instant::now(),
                });
                state.status = "Loading model; your current skater stays active.".into();
            }
            _ => (),
        }
        state.dirty = true;
    }
    let Some(p) = state.pending.as_ref() else {
        return;
    };
    let error = match server.get_load_state(p.asset.id()) {
        Some(LoadState::Failed(e)) => Some(format!("Could not load model: {e}")),
        _ if p.started.elapsed().as_secs() > 120 => Some("Character loading timed out.".into()),
        _ => None,
    };
    let prepared = if let Some(error) = error {
        Some(Err(error))
    } else if server.is_loaded_with_dependencies(p.asset.id())
        && instances
            .get(p.root)
            .is_ok_and(|i| spawner.instance_is_ready(**i))
    {
        Some(crate::animation::AnimationStatus::for_scene(
            p.root,
            &skater.animation.evaluator.frames.bone_names,
            &skins,
            &nodes,
            &parents,
        ))
    } else {
        None
    };
    let Some(prepared) = prepared else {
        return;
    };
    let pending = state.pending.take().unwrap();
    match prepared {
        Err(error) => {
            commands.entity(pending.root).despawn();
            state.status = format!("{error} Previous character retained.");
        }
        Ok(bindings) => {
            if state.active_root.is_none() {
                for (entity, parent, mut visibility) in &mut scenes {
                    if parent.parent() == player && entity != pending.root {
                        state.stock_visibility.push((entity, *visibility));
                        *visibility = Visibility::Hidden;
                    }
                }
            }
            if let Some(old) = state.active_root.replace(pending.root) {
                commands.entity(old).despawn();
            }
            if let Ok((_, _, mut visibility)) = scenes.get_mut(pending.root) {
                *visibility = Visibility::Inherited;
            }
            // Seed the new rig before publishing visibility, even when paused.
            let pose: Vec<_> = skater
                .render_pose
                .iter()
                .copied()
                .map(crate::animation::native_matrix)
                .collect();
            for (joint, transform) in bindings.pose_transforms(&pose) {
                commands.entity(joint).insert(transform);
            }
            *animation = bindings;
            state.active = Some(pending.id.clone());
            state.status = match save_selection(&state.directory, Some(pending.id)) {
                Ok(()) => "Character equipped. Resume whenever you're ready.".into(),
                Err(e) => format!("Equipped; could not save selection: {e}"),
            };
        }
    }
    state.dirty = true;
}
fn stop_import(mut state: ResMut<CustomModels>, mut exit: MessageReader<AppExit>) {
    if exit.read().next().is_some() {
        if let Some(mut job) = state.import.take() {
            let _ = job.child.kill();
            let _ = job.child.wait();
        }
    }
}
fn draw(
    mut commands: Commands,
    mut state: ResMut<CustomModels>,
    server: Res<AssetServer>,
    existing: Query<Entity, With<Panel>>,
) {
    if !state.dirty {
        return;
    }
    state.dirty = false;
    for e in &existing {
        commands.entity(e).despawn();
    }
    if !state.open {
        return;
    }
    let selected = state.selected;
    commands.spawn((Panel, GlobalZIndex(30), Node { position_type:PositionType::Absolute, width:percent(100), height:percent(100),
        justify_content:JustifyContent::Center, align_items:AlignItems::Center, ..default() }, BackgroundColor(Color::srgba(0.015,0.025,0.04,0.94))))
        .with_children(|root| { root.spawn((Node { width:px(800), height:percent(94), max_width:percent(96), padding:UiRect::all(px(20)),
            flex_direction:FlexDirection::Column, row_gap:px(14), ..default() }, BackgroundColor(Color::srgb(0.035,0.055,0.08))))
            .with_children(|panel| {
                label(panel, "CUSTOM MODELS", 30.);
                label(panel, "Choose a pro, special character, or your own imported model.", 17.);
                panel.spawn(Node { column_gap:px(10), ..default() }).with_children(|bar| {
                    button(bar,0, if state.import.is_some() { "Importing..." } else { "Import model..." },selected);
                    button(bar,2,["Show: All", "Show: Pros", "Show: Specials", "Show: Imported"][state.filter],selected);
                    button(bar,1,if state.active.is_none() { "Stock skater (active)" } else { "Use stock skater" },selected);
                });
                panel.spawn((ModelScroll, ScrollPosition(Vec2::new(0., state.scroll)), Node {
                    display:Display::Grid, grid_template_columns:RepeatedGridTrack::flex(COLUMNS as u16, 1.),
                    grid_auto_rows:vec![GridTrack::px(CARD_HEIGHT)], column_gap:px(10), row_gap:px(10),
                    flex_grow:1., min_height:px(0), overflow:Overflow::scroll_y(), align_content:AlignContent::Start,
                    ..default() })).with_children(|cards| {
                    for (i,entry) in state.visible_entries().into_iter().enumerate() {
                        let action=i+4;
                        cards.spawn((Button,Action(action),Node { min_width:px(0),height:px(CARD_HEIGHT),align_items:AlignItems::Center,padding:UiRect::all(px(6)),flex_direction:FlexDirection::Column,row_gap:px(6), ..default() },
                            BackgroundColor(if selected==action { Color::srgb(0.12,0.3,0.34) } else { Color::srgb(0.075,0.105,0.14) })))
                            .with_children(|card| {
                                card.spawn((ImageNode::new(server.load(entry.asset_path("preview.png"))),Node {width:px(128),height:px(160),..default()}));
                                label(card,&entry.name,15.);
                                if let Some(native) = &entry.native { label(card,&format!("{} · Native",native.category),12.); }
                                if state.active.as_ref()==Some(&entry.id) { label(card,"Equipped",14.); }
                            });
                    }
                    if state.visible_entries().is_empty() { label(cards,"Your imported characters will appear here.",18.); }
                });
                panel.spawn(Node { column_gap:px(10), ..default() }).with_children(|bar| {
                    label(bar,&format!("{} models / Scroll to browse",state.visible_entries().len()),16.);
                    button(bar,3,"Back",selected);
                });
                label(panel,&state.status,16.);
                label(panel,"Up/Down select · Enter/A equip · Escape/B back\nNative characters retain retail animation styles. Imports use your customiser style.",14.);
            }); });
}
fn label(parent: &mut ChildSpawnerCommands, text: &str, size: f32) {
    parent.spawn((
        Text::new(text),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.91, 0.95)),
    ));
}
fn button(parent: &mut ChildSpawnerCommands, action: usize, text: &str, selected: usize) {
    parent
        .spawn((
            Button,
            Action(action),
            Node {
                padding: UiRect::all(px(10)),
                ..default()
            },
            BackgroundColor(if action == selected {
                Color::srgb(0.12, 0.3, 0.34)
            } else {
                Color::srgb(0.08, 0.12, 0.17)
            }),
        ))
        .with_children(|p| label(p, text, 17.));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn customiser_fresh_roster_combines_with_personal_imports_without_duplicates() {
        let root =
            std::env::temp_dir().join(format!("skate-roster-sources-{}", std::process::id()));
        let personal = root.join("personal");
        let native = root.join("native");
        let pro_id = "a".repeat(64);
        let import_id = "b".repeat(64);
        for (library, id, name, pro) in [
            (&personal, &pro_id, "Legacy pro", true),
            (&personal, &import_id, "My import", false),
            (&native, &pro_id, "Current pro", true),
        ] {
            let entry = library.join("entries").join(id);
            std::fs::create_dir_all(&entry).unwrap();
            std::fs::write(entry.join("manifest.json"), serde_json::to_vec(&serde_json::json!({
                "version":1,"id":id,"name":name,"native":if pro {serde_json::json!({"key":"pro","category":"Pro"})} else {serde_json::Value::Null}
            })).unwrap()).unwrap();
            std::fs::write(entry.join("character.glb"), b"fixture").unwrap();
            std::fs::write(entry.join("preview.png"), b"fixture").unwrap();
        }
        let entries = discover_all(&personal, &native, "private/roster/");
        assert_eq!(entries.len(), 2);
        let pro = entries.iter().find(|e| e.id == pro_id).unwrap();
        assert_eq!(pro.name, "Current pro");
        assert_eq!(
            pro.asset_path("character.glb"),
            format!("private/roster/entries/{pro_id}/character.glb")
        );
        let imported = entries.iter().find(|e| e.id == import_id).unwrap();
        assert_eq!(
            imported.asset_path("preview.png"),
            format!("characters://entries/{import_id}/preview.png")
        );
        assert_eq!(discover(&personal).len(), 2); // No legacy entry was removed.
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn customiser_restore_stock_invalidates_cached_outfit_and_animation() {
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        let player = world.spawn_empty().id();
        let scene = world
            .spawn((SceneRoot(default()), Visibility::Hidden, ChildOf(player)))
            .id();
        let mut state = CustomModels {
            active: Some("native".into()),
            stock_visibility: vec![(scene, Visibility::Inherited)],
            ..default()
        };
        let mut parts = crate::customiser_parts::Parts::default();
        parts.applied = serde_json::json!({"selections":{"body":"previous"}});
        let mut animation = crate::animation::AnimationStatus::default();
        animation.ready = true;
        let mut queries: SystemState<Query<(Entity, &ChildOf, &mut Visibility), With<SceneRoot>>> =
            SystemState::new(&mut world);
        restore_stock(
            &mut state,
            &mut parts,
            &mut animation,
            &mut queries.get_mut(&mut world),
        );
        assert_eq!(
            *world.get::<Visibility>(scene).unwrap(),
            Visibility::Inherited
        );
        assert!(state.active.is_none() && state.stock_visibility.is_empty());
        assert!(parts.applied.is_null());
        assert!(!animation.ready);
    }
    #[test]
    #[ignore = "Requires private retail roster and animation banks; asset loading only"]
    fn native_roster_assets_bind_to_the_retail_animation_skeleton() {
        use bevy::{
            asset::AssetPlugin, ecs::system::SystemState, gltf::GltfPlugin, image::ImagePlugin,
            mesh::MeshPlugin, scene::ScenePlugin,
        };
        let directory = PathBuf::from(std::env::var("SKATE_TEST_LIBRARY").unwrap());
        let banks = skate_data::animation_banks::AnimationBanks::load(Path::new(
            &std::env::var("SKATE_TEST_ASSETS").unwrap(),
        ))
        .unwrap();
        let names = skate_data::animation_frames::AnimationFrames::from_banks(&banks)
            .unwrap()
            .bone_names;
        let metadata = banks.metadata().unwrap();
        let mut supported = std::collections::HashSet::new();
        for bank in &banks.banks {
            for record in bank.records().iter().filter(|r| r.header.type_id == 8) {
                if let Ok(skate_data::animation_metadata::TreeMetadata::Selector(selector)) =
                    metadata.tree(&record.header.name)
                {
                    if selector.parameter.eq_ignore_ascii_case("ProSkater") {
                        for (value, child) in selector.values.iter().zip(&selector.children) {
                            assert!(
                                metadata.tree(child).is_ok(),
                                "Missing native style child {child}"
                            );
                            supported.insert(value.to_ascii_uppercase());
                        }
                    }
                }
            }
        }
        let entries: Vec<_> = discover(&directory)
            .into_iter()
            .filter(|e| e.native.is_some())
            .collect();
        assert_eq!(
            entries.len(),
            41,
            "All registered retail pros and specials must be installed"
        );
        for entry in &entries {
            let style = native_animation_style(&entry.native.as_ref().unwrap().key);
            assert!(
                supported.contains(&style.to_ascii_uppercase()),
                "Native animation bank has no {style} selector"
            );
        }
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: directory.to_string_lossy().into_owned(),
                ..default()
            },
            ImagePlugin::default(),
            MeshPlugin,
            ScenePlugin,
            GltfPlugin::default(),
        ));
        app.init_asset::<StandardMaterial>()
            .init_asset::<AnimationClip>();
        app.finish();
        app.cleanup();
        for entry in entries {
            let handle: Handle<Gltf> = app
                .world()
                .resource::<AssetServer>()
                .load(format!("entries/{}/character.glb", entry.id));
            let started = Instant::now();
            loop {
                app.update();
                let server = app.world().resource::<AssetServer>();
                if server.is_loaded_with_dependencies(handle.id()) {
                    break;
                }
                assert!(
                    !matches!(
                        server.get_load_state(handle.id()),
                        Some(LoadState::Failed(_))
                    ),
                    "{} failed to load",
                    entry.name
                );
                assert!(started.elapsed().as_secs() < 45, "{} timed out", entry.name);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let scene = app
                .world()
                .resource::<Assets<Gltf>>()
                .get(&handle)
                .unwrap()
                .scenes[0]
                .clone();
            let mut scenes = app.world_mut().resource_mut::<Assets<Scene>>();
            let world = &mut scenes.get_mut(&scene).unwrap().world;
            let roots: Vec<Entity> = world
                .query_filtered::<Entity, Without<ChildOf>>()
                .iter(world)
                .collect();
            let root = world.spawn_empty().id();
            for e in roots {
                world.entity_mut(e).insert(ChildOf(root));
            }
            let mut queries: SystemState<(
                Query<(Entity, &SkinnedMesh)>,
                Query<(&Name, &Transform)>,
                Query<&ChildOf>,
            )> = SystemState::new(world);
            let (skins, nodes, parents) = queries.get(world);
            let binding = crate::animation::AnimationStatus::for_scene(
                root, &names, &skins, &nodes, &parents,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", entry.name));
            assert!(binding.ready);
            drop(scenes);
            drop(scene);
            drop(handle);
            app.update();
            app.update();
        }
    }
    #[test]
    fn imported_png_and_jpeg_textures_decode_in_the_game() {
        use bevy::{
            asset::RenderAssetUsages,
            image::{CompressedImageFormats, ImageSampler, ImageType},
        };
        for (data, mime) in [
            (
                include_bytes!("tests/fixtures/texture-codec.jpg").as_slice(),
                "image/jpeg",
            ),
            (
                include_bytes!("tests/fixtures/texture-codec.png").as_slice(),
                "image/png",
            ),
        ] {
            let image = Image::from_buffer(
                data,
                ImageType::MimeType(mime),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::default(),
                RenderAssetUsages::default(),
            )
            .unwrap();
            assert_eq!(image.size(), UVec2::new(2, 2));
            let pixels = image.data.unwrap();
            assert!(
                pixels[0] > 200 && pixels[1] < 60 && pixels[2] < 40,
                "Texture must keep its authored red colour"
            );
        }
    }
    #[test]
    #[ignore = "Requires a private model and its expected diffuse image; loads assets only, no window or renderer"]
    fn private_model_keeps_its_diffuse_texture() {
        use bevy::{
            asset::AssetPlugin, gltf::GltfPlugin, image::ImagePlugin, mesh::MeshPlugin,
            scene::ScenePlugin,
        };
        let source = PathBuf::from(std::env::var("SKATE_TEST_CHARACTER").unwrap());
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: source.parent().unwrap().to_string_lossy().into_owned(),
                ..default()
            },
            ImagePlugin::default(),
            MeshPlugin,
            ScenePlugin,
            GltfPlugin::default(),
        ));
        app.init_asset::<StandardMaterial>()
            .init_asset::<AnimationClip>();
        app.finish();
        app.cleanup();
        let asset: Handle<Gltf> = app
            .world()
            .resource::<AssetServer>()
            .load(source.file_name().unwrap().to_string_lossy().into_owned());
        let started = Instant::now();
        loop {
            app.update();
            let server = app.world().resource::<AssetServer>();
            if server.is_loaded_with_dependencies(asset.id()) {
                break;
            }
            assert!(
                !matches!(
                    server.get_load_state(asset.id()),
                    Some(LoadState::Failed(_))
                ),
                "Model load failed"
            );
            assert!(started.elapsed().as_secs() < 30, "Model load timed out");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let gltf = app.world().resource::<Assets<Gltf>>().get(&asset).unwrap();
        let material = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&gltf.materials[0])
            .unwrap();
        let diffuse = app
            .world()
            .resource::<Assets<Image>>()
            .get(material.base_color_texture.as_ref().unwrap())
            .unwrap();
        let bytes = std::fs::read(std::env::var("SKATE_TEST_DIFFUSE").unwrap()).unwrap();
        let expected = Image::from_buffer(
            &bytes,
            bevy::image::ImageType::MimeType("image/png"),
            bevy::image::CompressedImageFormats::NONE,
            true,
            default(),
            default(),
        )
        .unwrap();
        assert_eq!(diffuse.size(), expected.size());
        assert_eq!(
            diffuse.data, expected.data,
            "Game material must use the authored diffuse, not a shifted texture handle"
        );
    }
    #[test]
    fn library_ids_cannot_escape_the_asset_source() {
        assert!(valid_id(&"ab".repeat(32)));
        for bad in ["../character", "C:/model", "", &"F".repeat(64)] {
            assert!(!valid_id(bad));
        }
    }
    #[test]
    fn corrupt_or_incomplete_entries_are_not_published() {
        let root = std::env::temp_dir().join(format!("skate-model-library-{}", std::process::id()));
        let id = "a".repeat(64);
        let entry = root.join("entries").join(&id);
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(
            entry.join("manifest.json"),
            format!(r#"{{"version":1,"id":"{id}","name":"Model"}}"#),
        )
        .unwrap();
        assert!(discover(&root).is_empty());
        std::fs::write(entry.join("character.glb"), b"fixture").unwrap();
        std::fs::write(entry.join("preview.png"), b"fixture").unwrap();
        assert_eq!(discover(&root).len(), 1);
        save_selection(&root, Some(id.clone())).unwrap();
        save_selection(&root, None).unwrap();
        let saved: Selection =
            serde_json::from_slice(&std::fs::read(root.join("selection.json")).unwrap()).unwrap();
        assert!(saved.selected.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
