use super::Mods;
use bevy::prelude::*;
#[derive(Resource, Default)]
pub(crate) struct ModMenu {
    pub open: bool,
    just_opened: bool,
    id: Option<String>,
    selected: usize,
    editing: Option<(String, String)>,
    status: String,
}
impl ModMenu {
    pub fn configure(&mut self, id: String) {
        self.begin();
        self.id = Some(id);
    }
    pub fn begin(&mut self) {
        self.open = true;
        self.just_opened = true;
        self.id = None;
        self.selected = 0;
    }
}
#[derive(Component)]
struct Root;
#[derive(Component)]
struct Row(usize);
#[derive(Component)]
struct Label(usize);
#[derive(Component)]
struct Badge(usize);
#[derive(Component)]
struct Detail;
#[derive(Clone)]
enum Action {
    Select(String),
    Enable(String),
    Reload(String),
    Reset(String),
    Setting(String, String),
    Scan,
    OpenFolder,
    Back,
}
fn rows(menu: &ModMenu, mods: &Mods) -> Vec<(String, Action)> {
    let manager = &mods.manager;
    if let Some(id) = &menu.id {
        if let Some(p) = manager.packages.get(id) {
            let mut rows = vec![
                (
                    if p.running() {
                        "DISABLE MOD".into()
                    } else {
                        "ENABLE MOD".into()
                    },
                    Action::Enable(id.clone()),
                ),
                ("Reload from disk".into(), Action::Reload(id.clone())),
                (
                    "Reset settings to defaults".into(),
                    Action::Reset(id.clone()),
                ),
            ];
            for (key, s) in &p.manifest.settings {
                rows.push((
                    format!("{}: {}", s.label, p.settings[key]),
                    Action::Setting(id.clone(), key.clone()),
                ));
            }
            rows.push(("Back".into(), Action::Back));
            return rows;
        }
    }
    let mut rows: Vec<_> = manager
        .packages
        .iter()
        .map(|(id, p)| (p.manifest.name.clone(), Action::Select(id.clone())))
        .collect();
    rows.push(("Open mods folder".into(), Action::OpenFolder));
    rows.push(("Rescan packages".into(), Action::Scan));
    rows.push(("Back to pause menu".into(), Action::Back));
    rows
}
pub(super) fn install(app: &mut App) {
    app.add_systems(PostStartup, setup)
        .add_systems(
            PreUpdate,
            input
                .after(crate::graphics_menu::MenuInput)
                .before(crate::map_transition::MapTransitionSet),
        )
        .add_systems(Update, draw);
}
fn setup(mut commands: Commands) {
    commands.spawn((Root,GlobalZIndex(20),Node{display:Display::None,width:percent(100),height:percent(100),position_type:PositionType::Absolute,align_items:AlignItems::Center,justify_content:JustifyContent::Center,..default()},BackgroundColor(Color::srgba(0.015,0.025,0.04,0.98)))).with_children(|root| {
        root.spawn((Node{width:px(760),max_width:percent(95),padding:UiRect::all(px(18)),flex_direction:FlexDirection::Column,row_gap:px(5),..default()},BackgroundColor(Color::srgb(0.035,0.055,0.08)))).with_children(|panel| {
            panel.spawn((Text::new("MODS — Lua SDK 1"),TextFont{font_size:28.,..default()},TextColor(Color::WHITE)));
            for i in 0..8 { panel.spawn((Button,Row(i),Node{min_height:px(32),padding:UiRect::all(px(6)),justify_content:JustifyContent::SpaceBetween,..default()},BackgroundColor(Color::srgb(0.08,0.11,0.15)))).with_children(|row| {row.spawn((Label(i),Text::new(""),TextFont{font_size:17.,..default()},TextColor(Color::WHITE)));row.spawn((Badge(i),Text::new(""),TextFont{font_size:16.,..default()},TextColor(Color::WHITE)));}); }
            panel.spawn((Detail,Text::new(""),TextFont{font_size:15.,..default()},TextColor(Color::srgb(0.65,0.85,0.85))));
            panel.spawn((Text::new("Arrows / D-pad: select & adjust  |  Enter / A: choose  |  Esc / B: back\nStrings: Enter then type; Enter saves, Esc cancels. More rows scroll automatically."),TextFont{font_size:14.,..default()},TextColor(Color::WHITE)));
        });
    });
}
fn input(
    mut menu: ResMut<ModMenu>,
    mut mods: ResMut<Mods>,
    keys: Res<ButtonInput<KeyCode>>,
    nav: Res<crate::customiser::Navigation>,
    buttons: Query<(&Interaction, &Row), Changed<Interaction>>,
    mut typing: MessageReader<bevy::input::keyboard::KeyboardInput>,
) {
    if !menu.open {
        typing.clear();
        return;
    }
    if menu.just_opened {
        menu.just_opened = false;
        typing.clear();
        return;
    }
    if let Some((key, mut value)) = menu.editing.take() {
        if keys.just_pressed(KeyCode::Escape) || nav.pressed & 0x2000 != 0 {
            typing.clear();
            return;
        }
        if keys.just_pressed(KeyCode::Enter) || nav.pressed & 0x1000 != 0 {
            if let Some(id) = &menu.id {
                menu.status = mods
                    .manager
                    .setting(id, &key, value.into())
                    .err()
                    .unwrap_or_else(|| "Saved".into());
            }
            typing.clear();
            return;
        }
        if keys.just_pressed(KeyCode::Backspace) {
            value.pop();
        }
        for e in typing.read() {
            if e.state == bevy::input::ButtonState::Pressed {
                if let Some(t) = &e.text {
                    for c in t.chars().filter(|c| !c.is_control()) {
                        if value.chars().count() < 128 {
                            value.push(c);
                        }
                    }
                }
            }
        }
        menu.editing = Some((key, value));
        return;
    }
    typing.clear();
    if keys.just_pressed(KeyCode::Escape) || nav.pressed & (0x2000 | 0x10) != 0 {
        if menu.id.take().is_none() {
            menu.open = false;
        }
        menu.selected = 0;
        return;
    }
    let entries = rows(&menu, &mods);
    let count = entries.len();
    menu.selected = menu.selected.min(count - 1);
    if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 {
        menu.selected = (menu.selected + count - 1) % count;
    }
    if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 {
        menu.selected = (menu.selected + 1) % count;
    }
    let mut direction = 0;
    if keys.just_pressed(KeyCode::ArrowLeft) || nav.pressed & 4 != 0 {
        direction = -1;
    }
    if keys.just_pressed(KeyCode::ArrowRight)
        || keys.just_pressed(KeyCode::Enter)
        || nav.pressed & (8 | 0x1000) != 0
    {
        direction = 1;
    }
    let offset = menu.selected / 8 * 8;
    for (interaction, row) in &buttons {
        if *interaction == Interaction::Pressed && row.0 + offset < count {
            menu.selected = row.0 + offset;
            direction = 1;
        }
    }
    if direction == 0 {
        return;
    }
    let action = entries[menu.selected].1.clone();
    let result = match action {
        Action::Select(id) => {
            menu.id = Some(id);
            menu.selected = 0;
            Ok(())
        }
        Action::Back => {
            if menu.id.take().is_none() {
                menu.open = false;
            }
            menu.selected = 0;
            Ok(())
        }
        Action::OpenFolder => {
            #[cfg(target_os = "windows")]
            let program = "explorer.exe";
            #[cfg(target_os = "macos")]
            let program = "open";
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let program = "xdg-open";
            std::process::Command::new(program)
                .arg(mods.manager.root())
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Could not open mods folder: {e}"))
        }
        Action::Scan => {
            mods.manager.scan(true);
            Ok(())
        }
        Action::Enable(id) => {
            let enabled = !mods.manager.packages[&id].running();
            mods.manager.enable(&id, enabled)
        }
        Action::Reload(id) => {
            mods.manager.scan(true);
            mods.manager.reload(&id);
            Ok(())
        }
        Action::Reset(id) => mods.manager.reset(&id),
        Action::Setting(id, key) => {
            let p = &mods.manager.packages[&id];
            let s = &p.manifest.settings[&key];
            let value = &p.settings[&key];
            let next = match s.kind.as_str() {
                "boolean" => Some(serde_json::json!(!value.as_bool().unwrap())),
                "number" => Some(serde_json::json!(
                    (value.as_f64().unwrap() + direction as f64 * s.step.unwrap())
                        .clamp(s.min.unwrap(), s.max.unwrap())
                )),
                "choice" => {
                    let i = s
                        .choices
                        .iter()
                        .position(|v| Some(v.as_str()) == value.as_str())
                        .unwrap_or(0);
                    Some(serde_json::json!(
                        s.choices
                            [(i as i32 + direction).rem_euclid(s.choices.len() as i32) as usize]
                    ))
                }
                "string" => {
                    menu.editing = Some((key.clone(), value.as_str().unwrap().into()));
                    None
                }
                _ => None,
            };
            if let Some(value) = next {
                mods.manager.setting(&id, &key, value)
            } else {
                Ok(())
            }
        }
    };
    menu.status = result.err().unwrap_or_default();
}
fn draw(
    menu: Res<ModMenu>,
    mods: Res<Mods>,
    mut root: Single<&mut Node, With<Root>>,
    mut labels: Query<(&Label, &mut Text), Without<Badge>>,
    mut badges: Query<(&Badge, &mut Text, &mut TextColor), Without<Label>>,
    mut buttons: Query<(&Row, &mut Node, &mut BackgroundColor), Without<Root>>,
    mut detail: Single<&mut Text, (With<Detail>, Without<Label>, Without<Badge>)>,
) {
    root.display = if menu.open {
        Display::Flex
    } else {
        Display::None
    };
    if !menu.open {
        return;
    }
    let entries = rows(&menu, &mods);
    let selected = menu.selected.min(entries.len() - 1);
    let offset = selected / 8 * 8;
    for (label, mut text) in &mut labels {
        **text = entries
            .get(offset + label.0)
            .map(|e| e.0.clone())
            .unwrap_or_default();
    }
    for (row, mut node, mut color) in &mut buttons {
        node.display = if offset + row.0 < entries.len() {
            Display::Flex
        } else {
            Display::None
        };
        color.0 = if offset + row.0 == selected {
            Color::srgb(0.10, 0.30, 0.34)
        } else {
            Color::srgb(0.08, 0.11, 0.15)
        };
    }
    for (badge, mut text, mut color) in &mut badges {
        let package = entries.get(offset + badge.0).and_then(|(_, a)| match a {
            Action::Select(id) | Action::Enable(id) => mods.manager.packages.get(id),
            _ => None,
        });
        **text = package
            .map(|p| if p.running() { "ENABLED" } else { "DISABLED" })
            .unwrap_or("")
            .into();
        color.0 = if package.is_some_and(|p| p.running()) {
            Color::srgb(0.25, 1., 0.4)
        } else {
            Color::srgb(1., 0.25, 0.25)
        };
    }
    let mut description = format!("{} / {}\n{}", selected + 1, entries.len(), menu.status);
    if let Some(p) = menu
        .id
        .as_ref()
        .and_then(|id| mods.manager.packages.get(id))
    {
        description += &format!(
            "\n{} v{} by {}\n{}\n{}",
            p.manifest.id,
            p.manifest.version,
            p.manifest.author,
            p.manifest.description,
            p.error.as_deref().unwrap_or("")
        );
        if let Action::Setting(_, key) = &entries[selected].1 {
            description += &format!("\n{}", p.manifest.settings[key].description);
        }
    } else {
        description += &mods
            .manager
            .diagnostics
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some((key, value)) = &menu.editing {
        description += &format!("\nEditing {key}: {value}_");
    }
    if description.chars().count() > 650 {
        description = description.chars().take(650).collect::<String>()
            + "...\nFull errors are in the session log.";
    }
    ***detail = description;
}
