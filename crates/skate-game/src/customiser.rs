//! Compact character menus over resident retail parts.
use crate::customiser_parts::{Library, Parts};
use bevy::{input::mouse::MouseWheel, prelude::*};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Default, Deserialize)]
struct Entry {
    label: String,
    #[serde(default)]
    children: Vec<Entry>,
    #[serde(default)]
    options: Vec<Entry>,
    patch: Option<Value>,
    scalar: Option<String>,
    note: Option<String>,
    gender: Option<String>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    initial: Option<f64>,
    step: Option<f64>,
}
#[derive(Resource, Default)]
pub(crate) struct Navigation {
    pub pressed: u16,
    preview_turn: f32,
    previous: u16,
    held_for: f32,
    repeat_at: f32,
}
#[derive(Resource)]
pub(crate) struct Customiser {
    pub open: bool,
    pub enabled: bool,
    just_opened: bool,
    preview_yaw: f32,
    index: Entry,
    path: Vec<usize>,
    selected: usize,
    page_size: usize,
    search: String,
    pub draft: Value,
    settings: PathBuf,
    pub status: String,
    pub redraw: bool,
}
impl Customiser {
    pub(crate) fn begin(&mut self) {
        self.open = true;
        self.enabled = true;
        self.just_opened = true;
        self.preview_yaw = 0.;
        self.path.clear();
        self.search.clear();
        self.selected = 0;
        self.status.clear();
        self.redraw = true;
    }
    fn page(&self) -> &Entry {
        let mut page = &self.index;
        for &i in &self.path {
            page = &page.children[i];
        }
        page
    }
    fn visible(&self) -> Vec<usize> {
        let gender = self.draft["gender"].as_str().unwrap_or("male");
        self.page()
            .children
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.gender
                    .as_deref()
                    .is_none_or(|g| g == gender || g == "unisex")
            })
            .filter(|(_, e)| {
                self.search.is_empty()
                    || e.label.to_lowercase().contains(&self.search.to_lowercase())
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub fn preview_camera(&self) -> (f32, f32, f32) {
        let (height, distance, yaw) = self.preview_framing();
        (height, distance, yaw + self.preview_yaw)
    }
    fn preview_framing(&self) -> (f32, f32, f32) {
        let mut page = &self.index;
        let mut labels = vec![];
        for &i in &self.path {
            page = &page.children[i];
            labels.push(page.label.as_str());
        }
        if labels.contains(&"Upper body") || labels.contains(&"Lower body") {
            let upper = labels.contains(&"Upper body");
            let side = self.draft["tattoos"][if upper { "Arm" } else { "Leg" }]["side"]
                .as_u64()
                .unwrap_or(0);
            let yaw = match side {
                0 => -0.65,
                1 => 0.65,
                3 => std::f32::consts::PI,
                _ => 0.,
            };
            return (if upper { 1.15 } else { 0.5 }, 2.2, yaw);
        }
        if labels.iter().any(|s| {
            [
                "Face",
                "Face presets",
                "Hair",
                "Hair colour",
                "Facial hair",
                "Skin tone",
            ]
            .contains(s)
        }) {
            return (1.48, 1.65, 0.);
        }
        if labels.contains(&"Board") {
            return (0.25, 2.0, 0.6);
        }
        (0.95, 3.2, 0.)
    }
    pub fn preview(&self, parts: &Parts) -> Value {
        let mut profile = self.draft.clone();
        if !self.open {
            return profile;
        }
        let mut page = &self.index;
        let mut zone = None;
        let mut tattoos = false;
        for &i in &self.path {
            page = &page.children[i];
            if page.label == "Tattoos" {
                tattoos = true;
            }
            if tattoos && page.label == "Upper body" {
                zone = Some("OuterTorso");
            }
            if tattoos && page.label == "Lower body" {
                zone = Some("Pants");
            }
        }
        if let Some(slot) = zone {
            let gender = profile["gender"].as_str().unwrap_or("male");
            let candidate = parts
                .library
                .models
                .iter()
                .filter(|(_, p)| {
                    p.slot == slot
                        && p.flag("Gender") == gender
                        && (if slot == "OuterTorso" {
                            p.flag("TopType").is_empty()
                        } else {
                            p.flag("IsTattooViewingChoice").eq_ignore_ascii_case("true")
                        })
                })
                .min_by_key(|(id, _)| *id);
            if let Some((id, p)) = candidate {
                profile["selections"][slot] = json!({"asset_id":id,"material_id":p.materials[0]});
                if let Ok(resolved) = parts.resolve(&profile) {
                    return resolved;
                }
            }
        }
        profile
    }
    fn preload_entries(&self) -> Vec<&Entry> {
        if !self.open {
            return vec![];
        }
        let visible = self.visible();
        let start = self.selected / self.page_size * self.page_size;
        visible
            .iter()
            .skip(start)
            .take(self.page_size)
            .flat_map(|&i| {
                let e = &self.page().children[i];
                let current = option_index(e, &self.draft).unwrap_or(0);
                e.options
                    .iter()
                    .enumerate()
                    .filter(move |(i, _)| *i < 4 || i.abs_diff(current) <= 2)
                    .map(|(_, v)| v)
                    .chain(std::iter::once(e))
            })
            .collect()
    }
    pub fn preload_outfits(&self, parts: &Parts) -> Vec<Value> {
        self.preload_entries()
            .into_iter()
            .filter_map(|e| e.patch.as_ref())
            .filter_map(|patch| {
                let mut profile = self.draft.clone();
                if let Some(gender) = patch["gender"].as_str() {
                    profile = parts.library.defaults.get(gender)?.clone();
                } else {
                    merge(&mut profile, patch);
                    if let Some(hair) = patch["selections"].get("Hair") {
                        profile["hair_choice"] = hair.clone();
                    }
                }
                parts.resolve(&profile).ok()
            })
            .collect()
    }
}
#[derive(Component)]
struct Root;
#[derive(Component)]
struct Row(usize);
#[derive(Component)]
struct Adjust(usize, i32);
const BACK: usize = usize::MAX;
const NEXT: usize = usize::MAX - 1;
const PREV: usize = usize::MAX - 2;
const RESET: usize = usize::MAX - 3;
pub(crate) struct CustomiserPlugin;
impl Plugin for CustomiserPlugin {
    fn build(&self, app: &mut App) {
        crate::customiser_material::register(app);
        app.init_resource::<Navigation>()
            .add_systems(Startup, crate::customiser_parts::setup)
            .add_systems(
                PreUpdate,
                navigation.before(crate::graphics_menu::MenuInput),
            )
            .add_systems(
                PreUpdate,
                preferences
                    .after(crate::map_transition::MapTransitionSet)
                    .before(crate::input::poll_controllers),
            )
            .add_systems(PostStartup, setup)
            .add_systems(
                Update,
                (
                    interact,
                    rotate_preview,
                    crate::customiser_parts::update,
                    draw,
                )
                    .chain()
                    .after(crate::graphics_menu::interact)
                    .before(crate::app::FrameSet::Animation),
            );
    }
}
pub(crate) fn navigation(
    mut nav: ResMut<Navigation>,
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let pad = (0..4).find_map(|i| crate::input::platform::poll(i).ok());
    // Remap outside the dead zone so a resting stick cannot drift the preview.
    let axis = pad
        .as_ref()
        .map_or(0., |p| (p.state.right[0] as f32 / 32767.).clamp(-1., 1.));
    nav.preview_turn = axis.signum() * ((axis.abs() - 0.24) / 0.76).max(0.);
    let mut current = pad.map_or(0, |p| {
        p.state.buttons
            | if p.state.left[1] > 16000 {
                1
            } else if p.state.left[1] < -16000 {
                2
            } else {
                0
            }
            | if p.state.left[0] > 16000 {
                8
            } else if p.state.left[0] < -16000 {
                4
            } else {
                0
            }
    });
    for (key, bit) in [
        (KeyCode::ArrowUp, 1),
        (KeyCode::ArrowDown, 2),
        (KeyCode::ArrowLeft, 4),
        (KeyCode::ArrowRight, 8),
    ] {
        if keys.pressed(key) {
            current |= bit;
        }
    }
    nav.pressed = current & !nav.previous;
    if current & 15 != 0 && current & 15 == nav.previous & 15 {
        nav.held_for += time.delta_secs();
        if nav.held_for >= nav.repeat_at {
            nav.pressed |= current & 15;
            nav.repeat_at += 0.09;
        }
    } else {
        nav.held_for = 0.;
        nav.repeat_at = 0.35;
    }
    nav.previous = current;
}
fn rotate_preview(mut state: ResMut<Customiser>, nav: Res<Navigation>, time: Res<Time<Real>>) {
    if state.open && nav.preview_turn != 0. {
        // Real time keeps inspection responsive while gameplay is paused.
        state.preview_yaw = (state.preview_yaw
            + nav.preview_turn * 2.0 * time.delta_secs().min(0.1))
        .rem_euclid(std::f32::consts::TAU);
    }
}

fn page(label: impl Into<String>, children: Vec<Entry>) -> Entry {
    Entry {
        label: label.into(),
        children,
        ..default()
    }
}
fn choice(label: impl Into<String>, patch: Value) -> Entry {
    Entry {
        label: label.into(),
        patch: Some(patch),
        ..default()
    }
}
fn tidy(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|x| x.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn menu(lib: &Library, extras: Vec<Entry>) -> Entry {
    let model_page = |title: &str, slot: &str, types: &[&str]| {
        let mut entries = vec![];
        if [
            "Hat",
            "Hair",
            "Sock",
            "Glasses",
            "Jewellery",
            "WristItem",
            "Accessory",
        ]
        .contains(&slot)
        {
            entries.push(choice("None", json!({"selections":{slot:Value::Null}})));
        }
        if title == "T-shirts" {
            for (id, p) in &lib.models {
                if p.slot == "OuterTorso" && p.flag("TopType").is_empty() {
                    entries.push(Entry { gender:Some(p.flag("Gender").into()),
                        ..choice("No top",json!({"selections":{"OuterTorso":{"asset_id":id,"material_id":p.materials[0]}}})) });
                }
            }
        }
        let mut models: Vec<_> = lib
            .models
            .iter()
            .filter(|(_, p)| {
                p.slot == slot
                    && (slot != "Hair" || p.flag("HairModelType") == "full")
                    && (types.is_empty() || types.contains(&p.flag("TopType")))
            })
            .collect();
        models.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(b.0)));
        for (id, p) in models {
            let options: Vec<_> = p
                .materials
                .iter()
                .flat_map(|mid| {
                    let Some(m)=lib.materials.get(mid) else {return vec![];};
                    let mut options=vec![choice(&m.name,json!({"selections":{slot:{"asset_id":id,"material_id":mid}},"colours":{slot:Value::Null}}))];
                    if m.flag("IsColourizable").eq_ignore_ascii_case("true") {
                        options.extend(lib.colours.iter().map(|c| choice(&c.name,
                            json!({"selections":{slot:{"asset_id":id,"material_id":mid}},"colours":{slot:c.rgb}}))));
                    }
                    options
                })
                .collect();
            if options.is_empty() {
                continue;
            }
            let gender = p.flag("Gender");
            entries.push(Entry {
                label: if slot == "Hair" {
                    tidy(p.flag("HairStyle").replace("moosed", "moussed").as_str())
                } else {
                    p.name.clone()
                },
                options,
                gender: if gender.is_empty() {
                    None
                } else {
                    Some(gender.into())
                },
                ..default()
            });
        }
        if ["SkateBoard", "SkateTruck", "SkateWheel"].contains(&slot) {
            let mut groups: std::collections::BTreeMap<String, Vec<Entry>> = default();
            for e in entries {
                for mut option in e.options {
                    let label = option.label.trim_end_matches(" (Worn)").to_owned();
                    option.label = if option.label.ends_with(" (Worn)") {
                        "Worn"
                    } else {
                        "New"
                    }
                    .into();
                    groups.entry(label).or_default().push(option);
                }
            }
            entries = groups
                .into_iter()
                .map(|(label, options)| Entry {
                    label,
                    options,
                    ..default()
                })
                .collect();
        }
        page(title, entries)
    };
    let morph = |title: &str, name: &str| Entry {
        label: title.into(),
        scalar: Some(format!("morphs.{name}")),
        minimum: Some(0.),
        maximum: Some(0.5),
        initial: Some(if name == "fat" || name == "thin" {
            0.
        } else {
            0.25
        }),
        step: Some(0.025),
        ..default()
    };
    let mut body = page(
        "Body",
        vec![
            page(
                "Gender",
                vec![
                    choice("Male", json!({"gender":"male"})),
                    choice("Female", json!({"gender":"female"})),
                ],
            ),
            page(
                "Skin tone",
                ["light", "dark"]
                    .map(|s| choice(tidy(s), json!({"skin":s})))
                    .to_vec(),
            ),
            model_page("Hair", "Hair", &[]),
            page(
                "Build",
                vec![morph("Weight", "fat"), morph("Definition", "thin")],
            ),
            page(
                "Face",
                vec![
                    page(
                        "Brows",
                        vec![
                            morph("Depth", "local_brows_depth"),
                            morph("Height", "local_brows_height"),
                            morph("Angle", "local_brows_rotation"),
                        ],
                    ),
                    page(
                        "Eyes",
                        vec![
                            morph("Height", "local_eye_height"),
                            morph("Angle", "local_eye_rotation"),
                            morph("Width", "local_eye_width"),
                        ],
                    ),
                    page(
                        "Nose",
                        vec![
                            morph("Curve", "local_nose_curve"),
                            morph("Height", "local_nose_height"),
                            morph("Length", "local_nose_length"),
                            morph("Width", "local_nose_width"),
                        ],
                    ),
                    page(
                        "Mouth",
                        vec![
                            morph("Corners", "local_mouth_corner"),
                            morph("Height", "local_mouth_height"),
                            morph("Fullness", "local_mouth_lipsize"),
                            morph("Width", "local_mouth_width"),
                        ],
                    ),
                    page(
                        "Jaw & chin",
                        vec![
                            morph("Definition", "local_jaw_chiseled"),
                            morph("Depth", "local_jaw_depth"),
                            morph("Chin length", "local_chin_length"),
                        ],
                    ),
                ],
            ),
            page("Facial hair", {
                let mut names: Vec<_> = lib
                    .materials
                    .values()
                    .map(|m| m.flag("FacialHairStyle"))
                    .filter(|s| !s.is_empty())
                    .collect();
                names.sort();
                names.dedup();
                names
                    .into_iter()
                    .map(|s| Entry {
                        gender: Some("male".into()),
                        ..choice(
                            match s {
                                "none" => "Clean shaven".into(),
                                "billygoat" => "Long goatee".into(),
                                "soulpatch" => "Soul patch".into(),
                                "magnum" => "Thick moustache".into(),
                                _ => tidy(s),
                            },
                            json!({"beard":s}),
                        )
                    })
                    .collect()
            }),
        ],
    );
    let mut tattoos: Vec<_> = lib.tattoos.iter().collect();
    tattoos.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(b.0)));
    body.children.push(page(
        "Tattoos",
        [("Upper body", "Arm"), ("Lower body", "Leg")]
            .into_iter()
            .map(|(name, slot)| {
                let mut designs =
                    vec![choice("None", json!({"tattoos":{slot:{"id":Value::Null}}}))];
                designs.extend(
                    tattoos
                        .iter()
                        .map(|(id, t)| choice(&t.name, json!({"tattoos":{slot:{"id":id}}}))),
                );
                page(
                    name,
                    vec![
                        page("Design", designs),
                        page("Placement", {
                            let mut positions = vec![
                                choice(
                                    if slot == "Arm" {
                                        "Left arm"
                                    } else {
                                        "Left leg"
                                    },
                                    json!({"tattoos":{slot:{"side":0}}}),
                                ),
                                choice(
                                    if slot == "Arm" {
                                        "Right arm"
                                    } else {
                                        "Right leg"
                                    },
                                    json!({"tattoos":{slot:{"side":1}}}),
                                ),
                            ];
                            if slot == "Arm" {
                                positions.extend([
                                    choice("Chest", json!({"tattoos":{slot:{"side":2}}})),
                                    choice("Back", json!({"tattoos":{slot:{"side":3}}})),
                                ]);
                            }
                            positions
                        }),
                    ],
                )
            })
            .collect(),
    ));
    let clothes = page(
        "Clothes",
        vec![
            model_page("Hats", "Hat", &[]),
            model_page(
                "T-shirts",
                "OuterTorso",
                &["tshirt", "ls_tshirt", "tanktop"],
            ),
            model_page("Shirts", "OuterTorso", &["buttonshirt"]),
            model_page("Hoodies", "OuterTorso", &["hoody"]),
            model_page("Jackets", "OuterTorso", &["jacket"]),
            model_page("Sweaters", "OuterTorso", &["sweater"]),
            model_page("Pants & shorts", "Pants", &[]),
            model_page("Shoes", "Feet", &[]),
            model_page("Socks", "Sock", &[]),
            page(
                "Accessories",
                vec![
                    model_page("Glasses", "Glasses", &[]),
                    model_page("Necklaces", "Jewellery", &[]),
                    model_page("Wristwear", "WristItem", &[]),
                    model_page("Other", "Accessory", &[]),
                ],
            ),
        ],
    );
    let board = page(
        "Board",
        vec![
            model_page("Deck", "SkateBoard", &[]),
            model_page("Trucks", "SkateTruck", &[]),
            model_page("Wheels", "SkateWheel", &[]),
            Entry {
                label: "Truck tightness".into(),
                scalar: Some("truck".into()),
                minimum: Some(0.),
                maximum: Some(1.),
                initial: Some(0.7),
                step: Some(0.1),
                ..default()
            },
            Entry {
                label: "Wheel hardness".into(),
                scalar: Some("wheel".into()),
                minimum: Some(0.),
                maximum: Some(1.),
                initial: Some(0.7),
                step: Some(0.1),
                ..default()
            },
        ],
    );
    let mut root = page(
        "Your skater",
        vec![
            body,
            clothes,
            board,
            page(
                "Style",
                vec![page(
                    "Posture",
                    ["Default", "Stiff", "Slouch", "Buff"]
                        .iter()
                        .enumerate()
                        .map(|(i, s)| choice(*s, json!({"posture":i})))
                        .collect(),
                )],
            ),
        ],
    );
    for e in extras {
        if e.label == "Style" {
            root.children[3].children.extend(e.children);
        } else if let Some(i) = root.children[0]
            .children
            .iter()
            .position(|p| p.label == e.label)
        {
            root.children[0].children[i] = e;
        } else {
            root.children[0].children.push(e);
        }
    }
    root
}
fn setup(mut commands: Commands, config: Res<crate::config::Config>, parts: Res<Parts>) {
    let settings = config
        .asset_root
        .parent()
        .unwrap_or(&config.asset_root)
        .join("settings/character.json");
    let mut draft = parts
        .library
        .defaults
        .get("male")
        .cloned()
        .unwrap_or(json!({"selections":{},"morphs":{}}));
    let mut enabled = false;
    let status = String::new();
    if let Ok(b) = std::fs::read(&settings) {
        if let Ok(saved) = serde_json::from_slice::<Value>(&b) {
            if let Ok(p) = parts.resolve(&saved) {
                draft = p;
                enabled = true;
            }
        }
    }
    let extras = std::fs::read(
        crate::customiser_parts::asset_directory(&config.asset_root).join("extra-menu.json"),
    )
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .unwrap_or_default();
    commands.insert_resource(Customiser {
        open: false,
        enabled,
        just_opened: false,
        preview_yaw: 0.,
        index: menu(&parts.library, extras),
        path: vec![],
        selected: 0,
        page_size: 7,
        search: String::new(),
        draft,
        settings,
        status,
        redraw: true,
    });
    commands.spawn((
        Root,
        GlobalZIndex(20),
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: px(24),
            top: percent(4),
            width: percent(38),
            min_width: px(370),
            max_width: px(520),
            height: percent(92),
            padding: UiRect::all(px(22)),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            border_radius: BorderRadius::all(px(18)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.025, 0.035, 0.048, 0.98)),
    ));
}
fn merge(target: &mut Value, patch: &Value) {
    if let (Some(a), Some(b)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in b {
            merge(a.entry(k.clone()).or_insert(Value::Null), v);
        }
    } else {
        *target = patch.clone();
    }
}
fn scalar(profile: &Value, key: &str, initial: f64) -> f64 {
    profile
        .pointer(&format!("/{}", key.replace('.', "/")))
        .and_then(Value::as_f64)
        .unwrap_or(initial)
}
fn option_index(entry: &Entry, profile: &Value) -> Option<usize> {
    entry.options.iter().position(|o| {
        o.patch.as_ref().is_some_and(|p| {
            p.as_object().is_some_and(|fields| {
                fields.iter().all(|(key, value)| {
                    if key == "selections" {
                        value.as_object().is_some_and(|selections| {
                            selections.iter().all(|(slot, v)| {
                                (if slot == "Hair" {
                                    profile
                                        .get("hair_choice")
                                        .unwrap_or(&profile["selections"][slot])
                                } else {
                                    &profile["selections"][slot]
                                }) == v
                            })
                        })
                    } else {
                        matches_patch(&profile[key], value)
                    }
                })
            })
        })
    })
}

fn interact(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    nav: Res<Navigation>,
    mut wheel: MessageReader<MouseWheel>,
    mut typing: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut state: ResMut<Customiser>,
    parts: Res<Parts>,
    mut pause: ResMut<crate::graphics_menu::Menu>,
    buttons: Query<(&Interaction, &Row)>,
    adjustments: Query<(&Interaction, &Adjust)>,
) {
    if !state.open {
        wheel.clear();
        typing.clear();
        return;
    }
    if state.just_opened {
        state.just_opened = false;
        wheel.clear();
        typing.clear();
        return;
    }
    if state.page().children.iter().all(|e| e.children.is_empty()) {
        for event in typing.read() {
            if event.state == bevy::input::ButtonState::Pressed
                && !keys.pressed(KeyCode::ControlLeft)
                && !keys.pressed(KeyCode::ControlRight)
            {
                if let Some(text) = &event.text {
                    for c in text
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
                    {
                        if state.search.chars().count() < 32 {
                            state.search.push(c);
                            state.selected = 0;
                            state.redraw = true;
                        }
                    }
                }
            }
        }
    } else {
        typing.clear();
    }
    if !state.search.is_empty() {
        if keys.just_pressed(KeyCode::Backspace) {
            state.search.pop();
            state.selected = 0;
            state.redraw = true;
            return;
        }
        if keys.just_pressed(KeyCode::Escape) || nav.pressed & 0x2000 != 0 {
            state.search.clear();
            state.selected = 0;
            state.redraw = true;
            return;
        }
    }
    let visible = state.visible();
    let count = visible.len();
    let mut action = None;
    let mut back = keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::Backspace)
        || nav.pressed & 0x2000 != 0;
    let mut movement = 0;
    if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 {
        movement -= 1;
    }
    if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 {
        movement += 1;
    }
    for scroll in wheel.read() {
        if scroll.y > 0. {
            movement -= 1;
        } else if scroll.y < 0. {
            movement += 1;
        }
    }
    if keys.just_pressed(KeyCode::PageDown) {
        movement += state.page_size as i32;
    }
    if keys.just_pressed(KeyCode::PageUp) {
        movement -= state.page_size as i32;
    }
    if count > 0 {
        if movement != 0 {
            state.selected = (state.selected as i32 + movement).rem_euclid(count as i32) as usize;
            state.redraw = true;
        }
        if keys.just_pressed(KeyCode::Enter) || nav.pressed & 0x1000 != 0 {
            action = Some((visible[state.selected], 0));
        }
        if keys.just_pressed(KeyCode::ArrowRight) || nav.pressed & 8 != 0 {
            action = Some((visible[state.selected], 1));
        }
        if keys.just_pressed(KeyCode::ArrowLeft) || nav.pressed & 4 != 0 {
            action = Some((visible[state.selected], -1));
        }
    }
    if mouse.just_pressed(MouseButton::Left) {
        for (interaction, row) in &buttons {
            if *interaction == Interaction::Pressed {
                match row.0 {
                    BACK => back = true,
                    NEXT => {
                        if count > 0 {
                            state.selected = (state.selected + state.page_size).min(count - 1);
                            state.redraw = true;
                        }
                    }
                    PREV => {
                        state.selected = state.selected.saturating_sub(state.page_size);
                        state.redraw = true;
                    }
                    RESET => {
                        let g = state.draft["gender"].as_str().unwrap_or("male");
                        if let Some(p) = parts.library.defaults.get(g) {
                            state.draft = p.clone();
                            state.redraw = true;
                        }
                    }
                    i => {
                        state.selected = visible.iter().position(|v| *v == i).unwrap_or(0);
                        action = Some((i, 0));
                    }
                }
            }
        }
    }
    if mouse.just_pressed(MouseButton::Left) {
        for (interaction, adjust) in &adjustments {
            if *interaction == Interaction::Pressed {
                state.selected = visible.iter().position(|i| *i == adjust.0).unwrap_or(0);
                action = Some((adjust.0, adjust.1));
            }
        }
    }
    if back {
        state.search.clear();
        if let Some(i) = state.path.pop() {
            state.selected = state.visible().iter().position(|v| *v == i).unwrap_or(0);
        } else {
            let save = (|| -> Result<(), String> {
                std::fs::create_dir_all(state.settings.parent().unwrap())
                    .map_err(|e| e.to_string())?;
                let tmp = state.settings.with_extension("pending.json");
                std::fs::write(
                    &tmp,
                    serde_json::to_vec_pretty(&state.draft).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                std::fs::rename(tmp, &state.settings).map_err(|e| e.to_string())
            })();
            match save {
                Ok(()) => {
                    state.open = false;
                    pause.open = false;
                }
                Err(e) => state.status = format!("Could not save: {e}"),
            }
        }
        state.redraw = true;
        return;
    }
    if let Some((i, direction)) = action {
        let Some(entry) = state.page().children.get(i).cloned() else {
            return;
        };
        if !entry.children.is_empty() {
            state.search.clear();
            state.path.push(i);
            state.selected = 0;
            state.status.clear();
        } else if let Some(note) = entry.note {
            state.status = note;
        } else {
            let mut profile = state.draft.clone();
            if let Some(key) = entry.scalar {
                let amount = if direction < 0 { -1. } else { 1. };
                let val = (scalar(&profile, &key, entry.initial.unwrap_or(0.))
                    + amount * entry.step.unwrap_or(0.1))
                .clamp(entry.minimum.unwrap_or(0.), entry.maximum.unwrap_or(1.));
                if let Some((p, c)) = key.split_once('.') {
                    profile[p][c] = json!(val);
                } else {
                    profile[&key] = json!(val);
                }
            } else {
                let patch = if entry.options.is_empty() {
                    entry.patch
                } else {
                    let current = option_index(&entry, &profile);
                    let index = match current {
                        Some(i) => (i as i32 + if direction == 0 { 1 } else { direction })
                            .rem_euclid(entry.options.len() as i32)
                            as usize,
                        None => 0,
                    };
                    entry.options[index].patch.clone()
                };
                if let Some(patch) = patch {
                    if let Some(gender) = patch["gender"].as_str() {
                        if let Some(default) = parts.library.defaults.get(gender) {
                            profile = default.clone();
                        }
                    } else {
                        merge(&mut profile, &patch);
                        if let Some(hair) = patch["selections"].get("Hair") {
                            profile["hair_choice"] = hair.clone();
                        }
                    }
                }
            }
            match parts.resolve(&profile) {
                Ok(p) => {
                    state.draft = p;
                    state.status.clear();
                }
                Err(e) => state.status = e,
            }
        }
        state.redraw = true;
    }
}
fn preferences(
    state: Res<Customiser>,
    models: Res<crate::custom_models::CustomModels>,
    mut physics: ResMut<crate::physics::GamePhysics>,
    mut skater: ResMut<crate::physics::SkaterRuntime>,
) {
    apply_preferences(&state.draft, &mut physics, &mut skater.animation);
    if let Some(style) = models.native_style() {
        skater.animation.motion.playback_context.pro_skater =
            skate_core::animation::skeleton_input::name::encode(style.as_bytes());
        skater.animation.motion.animation.posture.set_profile(0);
        physics.set_gesture_preferences(None);
    }
}
pub(crate) fn apply_preferences(
    profile: &Value,
    physics: &mut crate::physics::GamePhysics,
    animation: &mut crate::skater_animation::SkaterAnimation,
) {
    physics.set_equipment_preferences(
        scalar(profile, "truck", 0.7) as f32,
        scalar(profile, "wheel", 0.7) as f32,
    );
    physics.set_gesture_preferences(profile["gestures"].as_object().map(|g| {
        // ResetGestureSet824FA730 marks all 37 entries available and selects
        // the first four in table order for Up,Down,Left,Right.
        std::array::from_fn(|i| {
            g.get(&i.to_string())
                .and_then(Value::as_u64)
                .unwrap_or(i as u64) as u32
        })
    }));
    animation.set_customisation(
        scalar(profile, "stance", 1.) as u32,
        scalar(profile, "style", 0.) as u32,
    );
    animation
        .motion
        .animation
        .posture
        .set_profile(scalar(profile, "posture", 0.) as u32);
}
fn draw(
    mut commands: Commands,
    mut state: ResMut<Customiser>,
    window: Single<&Window>,
    mut root: Single<(Entity, &mut Node), With<Root>>,
) {
    root.1.display = if state.open {
        Display::Flex
    } else {
        Display::None
    };
    if !state.open {
        return;
    }
    let size = (((window.height() * 0.92 - 255.) / 90.).floor() as usize).clamp(1, 10);
    if size != state.page_size {
        state.page_size = size;
        state.redraw = true;
    }
    if !state.redraw {
        return;
    }
    state.redraw = false;
    commands.entity(root.0).despawn_children();
    let page = state.page();
    let visible = state.visible();
    let start = state.selected / state.page_size * state.page_size;
    let subtitle = if !state.search.is_empty() {
        format!("Search: {}", state.search)
    } else if state.path.is_empty() {
        "Make it yours".to_owned()
    } else {
        let mut p = &state.index;
        let mut crumbs = vec![];
        for &i in &state.path {
            crumbs.push(p.label.clone());
            p = &p.children[i];
        }
        crumbs.join(" / ")
    };
    commands.entity(root.0).with_children(|p| {
        p.spawn((
            Text::new(subtitle),
            TextFont {
                font_size: 13.,
                ..default()
            },
            TextColor(Color::srgb(0.48, 0.62, 0.68)),
        ));
        p.spawn((
            Text::new(&page.label),
            TextFont {
                font_size: 29.,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
        p.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            flex_grow: 1.,
            min_height: px(0),
            ..default()
        })
        .with_children(|list| {
            if visible.is_empty() {
                list.spawn((
                    Text::new("No matches. Esc clears your search."),
                    TextFont {
                        font_size: 16.,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
            }
            for (pos, &i) in visible.iter().enumerate().skip(start).take(state.page_size) {
                let e = &page.children[i];
                let active = option_index(e, &state.draft);
                let detail = if let Some(key) = &e.scalar {
                    let v = scalar(&state.draft, key, e.initial.unwrap_or(0.));
                    format!(
                        "{}%   − / +",
                        ((v - e.minimum.unwrap_or(0.))
                            / (e.maximum.unwrap_or(1.) - e.minimum.unwrap_or(0.))
                            * 100.)
                            .round() as i32
                    )
                } else if !e.options.is_empty() {
                    let index = active.unwrap_or(0);
                    format!(
                        "{}  ·  {}/{}",
                        e.options[index].label,
                        index + 1,
                        e.options.len()
                    )
                } else if !e.children.is_empty() {
                    String::new()
                } else if e.patch.is_some() {
                    if e.patch
                        .as_ref()
                        .is_some_and(|p| matches_patch(&state.draft, p))
                    {
                        "Selected".into()
                    } else {
                        "Select".into()
                    }
                } else {
                    String::new()
                };
                let adjustable = e.scalar.is_some() || e.options.len() > 1;
                let width = (window.width() * 0.38).clamp(370., 520.)
                    - 44.
                    - 24.
                    - if adjustable { 68. } else { 0. };
                let label = format!(
                    "{}{}",
                    e.label,
                    if !e.children.is_empty() { "  ›" } else { "" }
                );
                let label_size =
                    (width / (label.chars().count().max(1) as f32 * 0.64)).clamp(16., 18.);
                let detail_size =
                    (width / (detail.chars().count().max(1) as f32 * 0.64)).clamp(10., 12.);
                list.spawn((
                    Node {
                        width: percent(100),
                        height: px(84),
                        flex_shrink: 0.,
                        align_items: AlignItems::Stretch,
                        border_radius: BorderRadius::all(px(9)),
                        ..default()
                    },
                    BackgroundColor(if pos == state.selected {
                        Color::srgb(0.08, 0.27, 0.29)
                    } else {
                        Color::srgb(0.045, 0.065, 0.085)
                    }),
                ))
                .with_children(|r| {
                    if adjustable {
                        adjust_button(r, i, -1, "−");
                    }
                    r.spawn((
                        Button,
                        Row(i),
                        Node {
                            flex_grow: 1.,
                            min_width: px(0),
                            padding: UiRect::axes(px(12), px(7)),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ))
                    .with_children(|text| {
                        text.spawn((
                            Text::new(label),
                            TextFont {
                                font_size: label_size,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                        if !detail.is_empty() {
                            text.spawn((
                                Text::new(detail),
                                TextFont {
                                    font_size: detail_size,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.60, 0.76, 0.78)),
                            ));
                        }
                    });
                    if adjustable {
                        adjust_button(r, i, 1, "+");
                    }
                });
            }
        });
        if !state.status.is_empty() {
            p.spawn((
                Text::new(&state.status),
                TextFont {
                    font_size: 13.,
                    ..default()
                },
                TextColor(Color::srgb(1., 0.72, 0.4)),
            ));
        }
        p.spawn(Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|footer| {
            for (id, title) in [
                (
                    BACK,
                    if state.path.is_empty() {
                        "Done"
                    } else {
                        "‹ Back"
                    },
                ),
                (PREV, "‹"),
                (NEXT, "›"),
            ] {
                if id != BACK && visible.len() <= state.page_size {
                    continue;
                }
                footer
                    .spawn((
                        Button,
                        Row(id),
                        Node {
                            padding: UiRect::axes(px(14), px(8)),
                            border_radius: BorderRadius::all(px(8)),
                            ..default()
                        },
                        BackgroundColor(if id == BACK {
                            Color::srgb(0.12, 0.39, 0.38)
                        } else {
                            Color::srgb(0.07, 0.10, 0.13)
                        }),
                    ))
                    .with_children(|r| {
                        r.spawn((
                            Text::new(title),
                            TextFont {
                                font_size: 17.,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                    });
            }
            if visible.len() > state.page_size {
                footer.spawn((
                    Text::new(format!(
                        "{} / {}",
                        start / state.page_size + 1,
                        visible.len().div_ceil(state.page_size)
                    )),
                    TextFont {
                        font_size: 13.,
                        ..default()
                    },
                    TextColor(Color::srgb(0.6, 0.7, 0.75)),
                ));
            }
        });
        p.spawn((
            Text::new(if state.path.is_empty() {
                "Right stick: Rotate   •   Done saves and resumes."
            } else {
                "↑↓ Browse   ←→ Change   Right stick: Rotate
Type to search"
            }),
            TextFont {
                font_size: 12.,
                ..default()
            },
            TextColor(Color::srgb(0.48, 0.60, 0.66)),
        ));
    });
}

fn adjust_button(parent: &mut ChildSpawnerCommands, index: usize, direction: i32, label: &str) {
    parent
        .spawn((
            Button,
            Adjust(index, direction),
            Node {
                width: px(34),
                flex_shrink: 0.,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(label),
                TextFont {
                    font_size: 22.,
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.9, 0.9)),
            ));
        });
}
fn matches_patch(profile: &Value, patch: &Value) -> bool {
    if let Some(object) = patch.as_object() {
        object.iter().all(|(k, v)| matches_patch(&profile[k], v))
    } else {
        profile == patch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn customiser_missing_library_keeps_stock_scene_visible() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_resource::<crate::customiser_parts::Parts>()
            .init_resource::<crate::custom_models::CustomModels>()
            .init_resource::<crate::animation::AnimationStatus>()
            .init_resource::<Assets<crate::customiser_material::SkaterMaterial>>();
        let world = app.world_mut();
        world.insert_resource(Customiser {
            open: true,
            enabled: true,
            just_opened: false,
            preview_yaw: 0.,
            index: Entry::default(),
            path: vec![],
            selected: 0,
            page_size: 6,
            search: String::new(),
            draft: json!({"selections":{},"morphs":{}}),
            settings: PathBuf::new(),
            status: String::new(),
            redraw: false,
        });
        let player = world.spawn(crate::world::PlayerRoot).id();
        let stock = world
            .spawn((SceneRoot(default()), Visibility::Inherited, ChildOf(player)))
            .id();
        world
            .run_system_once(crate::customiser_parts::update)
            .unwrap();
        assert_eq!(
            *world.get::<Visibility>(stock).unwrap(),
            Visibility::Inherited
        );
        assert!(
            world
                .resource::<Customiser>()
                .status
                .contains("unavailable")
        );
        assert!(world.resource::<Parts>().applied.is_null());
    }
    #[test]
    fn customiser_colour_choice_is_distinct_from_the_original() {
        let entry = Entry {
            options: vec![
                choice(
                    "Original",
                    json!({"selections":{"Feet":{"asset_id":"shoe","material_id":"new"}},"colours":{"Feet":null}}),
                ),
                choice(
                    "Red",
                    json!({"selections":{"Feet":{"asset_id":"shoe","material_id":"new"}},"colours":{"Feet":[1.,0.,0.]}}),
                ),
            ],
            ..default()
        };
        let mut p = json!({"selections":{"Feet":{"asset_id":"shoe","material_id":"new"}}});
        assert_eq!(option_index(&entry, &p), Some(0));
        merge(&mut p, entry.options[1].patch.as_ref().unwrap());
        assert_eq!(option_index(&entry, &p), Some(1));
        let material = json!({"name":"original","flags":{},"diffuse":"",
            "alpha":false,"tint":[0.4,0.5,0.6],"metallic":0.,"roughness":0.7});
        let library: Library = serde_json::from_value(json!({
            "models":{
                "shoe":{"slot":"Feet","name":"shoe","flags":{},"materials":["new"],"scene":""},
                "head":{"slot":"Rostral","name":"head","flags":{},"materials":["head"],"scene":""}},
            "materials":{"new":material.clone(),"head":material},"defaults":{},"morphs":[]
        }))
        .unwrap();
        let parts = Parts::for_test(library);
        p["selections"]["Rostral"] = json!({"asset_id":"head","material_id":"head"});
        assert!(parts.resolve(&p).is_ok());
        merge(&mut p, entry.options[0].patch.as_ref().unwrap());
        assert_eq!(option_index(&entry, &p), Some(0));
        assert!(
            parts.resolve(&p).is_ok(),
            "original material tint must be selectable"
        );
        let restored: Value = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert!(
            parts.resolve(&restored).is_ok(),
            "saved/online null reset must remain valid"
        );
        for invalid in [
            json!([2., 0., 0.]),
            json!([-0.1, 0., 0.]),
            json!([0., 1.]),
            json!("red"),
        ] {
            p["colours"]["Feet"] = invalid;
            assert_eq!(parts.resolve(&p).unwrap_err(), "Invalid clothing colour");
        }
    }
    #[test]
    fn customiser_owned_menu_presets_tattoos_and_search() {
        let Ok(path) = std::env::var("SKATE_CAC_TEST_LIBRARY") else {
            return;
        };
        let lib: Library = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let extras: Vec<Entry> = serde_json::from_slice(
            &std::fs::read(std::path::Path::new(&path).with_file_name("extra-menu.json")).unwrap(),
        )
        .unwrap();
        let parts = Parts::for_test(lib);
        for gender in ["male", "female"] {
            let mut state = Customiser {
                open: true,
                enabled: true,
                just_opened: false,
                preview_yaw: 0.,
                index: menu(&parts.library, extras.clone()),
                path: vec![],
                selected: 0,
                page_size: 6,
                search: String::new(),
                draft: parts.library.defaults[gender].clone(),
                settings: PathBuf::new(),
                status: String::new(),
                redraw: true,
            };
            assert_eq!(state.visible().len(), 4);
            let body = 0;
            let tattoos = state.index.children[body]
                .children
                .iter()
                .position(|e| e.label == "Tattoos")
                .unwrap();
            let original = state.draft.clone();
            for (i, slot) in [(0, "OuterTorso"), (1, "Pants")] {
                state.path = vec![body, tattoos, i];
                let preview = state.preview(&parts);
                assert_ne!(preview["selections"][slot], original["selections"][slot]);
                assert_eq!(state.draft, original);
            }
            state.open = false;
            assert_eq!(state.preview(&parts), original);
            state.open = true;
            let faces = state.index.children[body]
                .children
                .iter()
                .position(|e| e.label == "Face presets")
                .unwrap();
            state.path = vec![body, faces];
            assert_eq!(state.visible().len(), 10);
            for i in state.visible() {
                let mut profile = state.draft.clone();
                merge(
                    &mut profile,
                    state.page().children[i].patch.as_ref().unwrap(),
                );
                assert!(parts.resolve(&profile).is_ok());
            }
            state.search = "Face 10".into();
            assert_eq!(state.visible().len(), 1);
            state.search = "No such item".into();
            assert_eq!(state.visible().len(), 0);
            // Selecting a new face/body profile must survive JSON save/reload.
            let encoded = serde_json::to_vec(&state.draft).unwrap();
            let restored: Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(
                parts.resolve(&restored).unwrap(),
                parts.resolve(&state.draft).unwrap()
            );
        }
    }
}
