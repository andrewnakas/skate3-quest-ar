//! Independent, draggable enabled-mod windows. Positions survive closing Escape.
use super::{ModMenu, Mods};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use std::collections::BTreeMap;
mod scrollbar;

#[derive(Default)]
struct Layout {
    position: Vec2,
    size: Vec2,
    collapsed: bool,
    selected: usize,
    status: String,
    reveal_selected: bool,
}
#[derive(Resource, Default)]
pub(crate) struct EnabledPanel {
    pub focused: bool,
    active: Option<String>,
    layouts: BTreeMap<String, Layout>,
    drag: Option<(String, Vec2)>,
    resize: Option<(String, Vec2)>,
    scroll_drag: Option<(String, f32)>,
    signature: Vec<(String, Vec<String>)>,
}
impl EnabledPanel {
    pub fn dragging(&self) -> bool {
        self.drag.is_some() || self.resize.is_some() || self.scroll_drag.is_some()
    }
}
#[derive(Component)]
struct Root(String);
#[derive(Component)]
struct Body(String);
#[derive(Component)]
struct Viewport(String);
#[derive(Component)]
struct Header(String);
#[derive(Component)]
struct ResizeGrip(String);
#[derive(Component)]
struct Row(String, usize);
#[derive(Component)]
struct Label(String, usize);
#[derive(Component)]
struct ValueLabel(String, usize);
#[derive(Component)]
struct Hint(String);
#[derive(Component)]
struct CollapseLabel(String);
#[derive(Component, Clone)]
struct Action(String, Operation);
#[derive(Clone)]
enum Operation {
    Adjust(usize, i32),
    Collapse,
    Configure,
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<EnabledPanel>()
        .add_systems(
            PreUpdate,
            input
                .after(crate::graphics_menu::MenuInput)
                .before(crate::map_transition::MapTransitionSet),
        )
        .add_systems(Update, (sync, draw, scroll, scrollbar::update).chain());
}
fn button(parent: &mut ChildSpawnerCommands, text: &str, action: Action) {
    parent
        .spawn((
            Button,
            action,
            Node {
                min_width: px(28.),
                height: px(28.),
                padding: UiRect::horizontal(px(5.)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgb(0.12, 0.20, 0.26)),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(text),
                TextFont {
                    font_size: 16.,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}
fn sync(
    mut commands: Commands,
    mods: Res<Mods>,
    mut panel: ResMut<EnabledPanel>,
    roots: Query<Entity, With<Root>>,
) {
    let signature: Vec<_> = mods
        .manager
        .packages
        .iter()
        .filter(|(_, p)| p.running())
        .map(|(id, p)| {
            (
                id.clone(),
                p.manifest.settings.keys().cloned().collect::<Vec<_>>(),
            )
        })
        .collect();
    if signature == panel.signature {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    panel.signature = signature.clone();
    panel.drag = None;
    panel.resize = None;
    panel.scroll_drag = None;
    if !signature
        .iter()
        .any(|(id, _)| panel.active.as_ref() == Some(id))
    {
        panel.active = signature.first().map(|(id, _)| id.clone());
        panel.focused = false;
    }
    for (index, (id, settings)) in signature.iter().enumerate() {
        panel.layouts.entry(id.clone()).or_insert_with(|| Layout {
            size: Vec2::new(360., 760.),
            position: Vec2::new(16. + index as f32 * 22., 64. + index as f32 * 34.),
            ..default()
        });
        let name = &mods.manager.packages[id].manifest.name;
        commands
            .spawn((
                Root(id.clone()),
                GlobalZIndex(12),
                Node {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    width: px(360.),
                    height: px(760.),
                    max_width: percent(95),
                    padding: UiRect::all(px(10.)),
                    row_gap: px(8.),
                    flex_direction: FlexDirection::Column,
                    border_radius: BorderRadius::all(px(9.)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.035, 0.055, 0.08)),
            ))
            .with_children(|window| {
                window
                    .spawn(Node {
                        width: percent(100),
                        column_gap: px(5.),
                        ..default()
                    })
                    .with_children(|bar| {
                        bar.spawn((
                            Button,
                            Header(id.clone()),
                            Node {
                                flex_grow: 1.,
                                min_width: px(0.),
                                padding: UiRect::all(px(5.)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.08, 0.18, 0.20)),
                        ))
                        .with_children(|title| {
                            title.spawn((
                                Text::new(name),
                                TextFont {
                                    font_size: 17.,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.25, 1., 0.4)),
                            ));
                        });
                        bar.spawn((
                            Button,
                            Action(id.clone(), Operation::Collapse),
                            Node {
                                width: px(30.),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.12, 0.20, 0.26)),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                CollapseLabel(id.clone()),
                                Text::new("-"),
                                TextFont {
                                    font_size: 18.,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                            ));
                        });
                    });
                window
                    .spawn((
                        Body(id.clone()),
                        Node {
                            flex_grow: 1.,
                            min_height: px(0.),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(6.),
                            ..default()
                        },
                    ))
                    .with_children(|body| {
                        body.spawn(Node {
                            flex_grow: 1.,
                            flex_basis: px(0.),
                            min_height: px(0.),
                            column_gap: px(6.),
                            ..default()
                        })
                        .with_children(|area| {
                            area.spawn((
                                Viewport(id.clone()),
                                ScrollPosition::default(),
                                Node {
                                    flex_grow: 1.,
                                    flex_basis: px(0.),
                                    min_height: px(0.),
                                    min_width: px(0.),
                                    height: percent(100),
                                    overflow: Overflow::scroll_y(),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: px(6.),
                                    ..default()
                                },
                            ))
                            .with_children(|list| {
                                for i in 0..settings.len() {
                                    list.spawn((
                                        Row(id.clone(), i),
                                        Node {
                                            align_items: AlignItems::Center,
                                            column_gap: px(4.),
                                            min_height: px(34.),
                                            flex_shrink: 0.,
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgb(0.08, 0.11, 0.15)),
                                    ))
                                    .with_children(|row| {
                                        row.spawn((
                                            Label(id.clone(), i),
                                            Text::new(""),
                                            TextFont {
                                                font_size: 14.,
                                                ..default()
                                            },
                                            TextColor(Color::WHITE),
                                            Node {
                                                flex_grow: 1.,
                                                width: px(132.),
                                                ..default()
                                            },
                                        ));
                                        button(
                                            row,
                                            "-",
                                            Action(id.clone(), Operation::Adjust(i, -1)),
                                        );
                                        row.spawn((
                                            ValueLabel(id.clone(), i),
                                            Text::new(""),
                                            TextFont {
                                                font_size: 14.,
                                                ..default()
                                            },
                                            TextColor(Color::WHITE),
                                            Node {
                                                width: px(51.),
                                                ..default()
                                            },
                                        ));
                                        button(
                                            row,
                                            "+",
                                            Action(id.clone(), Operation::Adjust(i, 1)),
                                        );
                                    });
                                }
                            });
                            scrollbar::spawn(area, id);
                        });
                        body.spawn(Node {
                            column_gap: px(6.),
                            ..default()
                        })
                        .with_children(|p| {
                            button(p, "Manage mod", Action(id.clone(), Operation::Configure));
                        });
                        body.spawn((
                            Hint(id.clone()),
                            Text::new(""),
                            TextFont {
                                font_size: 12.,
                                ..default()
                            },
                            TextColor(Color::srgb(0.65, 0.85, 0.85)),
                        ));
                        body.spawn((
                            Button,
                            ResizeGrip(id.clone()),
                            Node {
                                align_self: AlignSelf::FlexEnd,
                                width: px(28.),
                                height: px(22.),
                                flex_shrink: 0.,
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.12, 0.20, 0.26)),
                        ))
                        .with_children(|grip| {
                            grip.spawn((
                                Text::new("//"),
                                TextFont {
                                    font_size: 18.,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                            ));
                        });
                    });
            });
    }
}
fn input(
    mut panel: ResMut<EnabledPanel>,
    pause: Res<crate::graphics_menu::Menu>,
    mut menu: ResMut<ModMenu>,
    custom: Res<crate::customiser::Customiser>,
    mut mods: ResMut<Mods>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    nav: Res<crate::customiser::Navigation>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    grips: Query<(&Interaction, &ResizeGrip), Changed<Interaction>>,
    headers: Query<(&Interaction, &Header), Changed<Interaction>>,
    buttons: Query<(&Interaction, &Action), Changed<Interaction>>,
) {
    if !pause.open || menu.open || custom.open {
        panel.focused = false;
        panel.drag = None;
        panel.resize = None;
        panel.scroll_drag = None;
        return;
    }
    let ids: Vec<_> = mods
        .manager
        .packages
        .iter()
        .filter(|(_, p)| p.running())
        .map(|(id, _)| id.clone())
        .collect();
    if ids.is_empty() {
        panel.focused = false;
        panel.drag = None;
        panel.resize = None;
        panel.scroll_drag = None;
        return;
    }
    if keys.just_pressed(KeyCode::Tab) || nav.pressed & 0x4000 != 0 {
        let next = if panel.focused {
            panel
                .active
                .as_ref()
                .and_then(|id| ids.iter().position(|s| s == id))
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };
        panel.focused = next < ids.len();
        if panel.focused {
            panel.active = Some(ids[next].clone());
        }
    }
    if !mouse.pressed(MouseButton::Left) {
        panel.drag = None;
        panel.resize = None;
        panel.scroll_drag = None;
    }
    for (interaction, header) in &headers {
        if *interaction == Interaction::Pressed && mouse.pressed(MouseButton::Left) {
            if let (Some(cursor), Some(layout)) =
                (window.cursor_position(), panel.layouts.get(&header.0))
            {
                panel.drag = Some((header.0.clone(), cursor - fitted_position(layout, &window)));
                panel.active = Some(header.0.clone());
                panel.focused = true;
            }
        }
    }
    for (interaction, grip) in &grips {
        if *interaction == Interaction::Pressed && mouse.pressed(MouseButton::Left) {
            if let (Some(cursor), Some(layout)) =
                (window.cursor_position(), panel.layouts.get(&grip.0))
            {
                let size = fit_size(layout.size, &window);
                let position = fitted_position(layout, &window);
                panel.layouts.get_mut(&grip.0).unwrap().position = position;
                panel.resize = Some((grip.0.clone(), size - cursor));
                panel.drag = None;
                panel.active = Some(grip.0.clone());
                panel.focused = true;
            }
        }
    }
    if let (Some((id, offset)), Some(cursor)) = (panel.resize.clone(), window.cursor_position()) {
        if let Some(layout) = panel.layouts.get_mut(&id) {
            layout.size = fit_size(cursor + offset, &window)
                .min(Vec2::new(window.width(), window.height()) - layout.position);
        }
    }
    if let (Some((id, offset)), Some(cursor)) = (panel.drag.clone(), window.cursor_position()) {
        if let Some(layout) = panel.layouts.get_mut(&id) {
            layout.position = (cursor - offset).clamp(
                Vec2::ZERO,
                Vec2::new(
                    (window.width() - fit_size(layout.size, &window).x).max(0.),
                    (window.height()
                        - if layout.collapsed {
                            48.
                        } else {
                            fit_size(layout.size, &window).y
                        })
                    .max(0.),
                ),
            );
        }
    }
    let mut action = None;
    if panel.focused {
        if let Some(id) = panel.active.clone() {
            if let (Some(p), Some(layout)) =
                (mods.manager.packages.get(&id), panel.layouts.get_mut(&id))
            {
                let count = p.manifest.settings.len();
                if count > 0 {
                    layout.selected = layout.selected.min(count - 1);
                    if keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0 {
                        layout.selected = (layout.selected + count - 1) % count;
                        layout.reveal_selected = true;
                    }
                    if keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0 {
                        layout.selected = (layout.selected + 1) % count;
                        layout.reveal_selected = true;
                    }
                    if keys.just_pressed(KeyCode::ArrowLeft) || nav.pressed & 4 != 0 {
                        action = Some(Action(id.clone(), Operation::Adjust(layout.selected, -1)));
                    }
                    if keys.just_pressed(KeyCode::ArrowRight)
                        || keys.just_pressed(KeyCode::Enter)
                        || nav.pressed & (8 | 0x1000) != 0
                    {
                        action = Some(Action(id.clone(), Operation::Adjust(layout.selected, 1)));
                    }
                }
            }
        }
    }
    for (interaction, button) in &buttons {
        if *interaction == Interaction::Pressed {
            action = Some(button.clone());
            panel.active = Some(button.0.clone());
            panel.focused = true;
        }
    }
    let Some(Action(id, operation)) = action else {
        return;
    };
    let Some(p) = mods.manager.packages.get(&id) else {
        return;
    };
    if !p.running() {
        return;
    }
    let Some(layout) = panel.layouts.get_mut(&id) else {
        return;
    };
    match operation {
        Operation::Collapse => {
            layout.collapsed = !layout.collapsed;
        }
        Operation::Configure => menu.configure(id),
        Operation::Adjust(row, direction) => {
            if layout.collapsed {
                return;
            }
            let selected = row;
            let Some((key, s)) = p.manifest.settings.iter().nth(selected) else {
                return;
            };
            let key = key.clone();
            layout.selected = selected;
            let v = &p.settings[&key];
            let next = match s.kind.as_str() {
                "number" => Some(serde_json::json!(
                    (v.as_f64().unwrap() + direction as f64 * s.step.unwrap())
                        .clamp(s.min.unwrap(), s.max.unwrap())
                )),
                "boolean" => Some(serde_json::json!(!v.as_bool().unwrap())),
                "choice" => {
                    let i = s
                        .choices
                        .iter()
                        .position(|c| Some(c.as_str()) == v.as_str())
                        .unwrap_or(0);
                    Some(serde_json::json!(
                        s.choices
                            [(i as i32 + direction).rem_euclid(s.choices.len() as i32) as usize]
                    ))
                }
                _ => {
                    menu.configure(id.clone());
                    None
                }
            };
            if let Some(v) = next {
                layout.status = mods.manager.setting(&id, &key, v).err().unwrap_or_default();
            }
        }
    }
}
fn draw(
    panel: Res<EnabledPanel>,
    pause: Res<crate::graphics_menu::Menu>,
    menu: Res<ModMenu>,
    custom: Res<crate::customiser::Customiser>,
    mods: Res<Mods>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut roots: Query<(&Root, &mut Node, &mut GlobalZIndex)>,
    mut bodies: Query<(&Body, &mut Node), Without<Root>>,
    mut rows: Query<(&Row, &mut Node, &mut BackgroundColor), (Without<Root>, Without<Body>)>,
    mut text: Query<(
        &mut Text,
        Option<&Label>,
        Option<&ValueLabel>,
        Option<&Hint>,
        Option<&CollapseLabel>,
    )>,
) {
    let visible = pause.open && !menu.open && !custom.open;
    for (root, mut node, mut z) in &mut roots {
        if let Some(layout) = panel.layouts.get(&root.0) {
            node.display = if visible
                && mods
                    .manager
                    .packages
                    .get(&root.0)
                    .is_some_and(|p| p.running())
            {
                Display::Flex
            } else {
                Display::None
            };
            let size = fit_size(layout.size, &window);
            node.width = px(size.x);
            node.height = if layout.collapsed {
                Val::Auto
            } else {
                px(size.y)
            };
            node.left = px(layout
                .position
                .x
                .clamp(0., (window.width() - size.x).max(0.)));
            node.top = px(layout.position.y.clamp(
                0.,
                (window.height() - if layout.collapsed { 48. } else { size.y }).max(0.),
            ));
            z.0 = if panel.active.as_ref() == Some(&root.0) {
                14
            } else {
                12
            };
        }
    }
    for (body, mut node) in &mut bodies {
        node.display = if panel.layouts.get(&body.0).is_some_and(|l| !l.collapsed) {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (row, mut node, mut color) in &mut rows {
        let selected = panel.layouts.get(&row.0).map_or(0, |l| l.selected);
        let index = row.1;
        node.display = if mods
            .manager
            .packages
            .get(&row.0)
            .is_some_and(|p| index < p.manifest.settings.len())
        {
            Display::Flex
        } else {
            Display::None
        };
        color.0 = if panel.focused && panel.active.as_ref() == Some(&row.0) && index == selected {
            Color::srgb(0.10, 0.30, 0.34)
        } else {
            Color::srgb(0.08, 0.11, 0.15)
        };
    }
    for (mut t, label, value, hint, collapse) in &mut text {
        if let Some(c) = collapse {
            **t = if panel.layouts.get(&c.0).is_some_and(|l| l.collapsed) {
                "+"
            } else {
                "-"
            }
            .into();
        }
        if let Some(h) = hint {
            if let Some(l) = panel.layouts.get(&h.0) {
                **t = format!(
                    "Drag title to move | Drag // to resize\nWheel or drag scrollbar | Tab / X: focus\n{}",
                    l.status
                );
            }
        }
        let field = label
            .map(|l| (&l.0, l.1, false))
            .or_else(|| value.map(|v| (&v.0, v.1, true)));
        if let Some((id, row, is_value)) = field {
            let setting = mods
                .manager
                .packages
                .get(id)
                .and_then(|p| p.manifest.settings.iter().nth(row).map(|(k, s)| (p, k, s)));
            **t = setting
                .map(|(p, key, s)| {
                    if !is_value {
                        s.label.clone()
                    } else {
                        let v = &p.settings[key];
                        v.as_f64().map(|n| format!("{n:.2}")).unwrap_or_else(|| {
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string())
                        })
                    }
                })
                .unwrap_or_default();
        }
    }
}

// Scroll only the uppermost mod window under the pointer. Use computed UI
// geometry so DPI scaling, dragging and wrapped setting labels remain correct.
fn scroll(
    mut wheel: MessageReader<MouseWheel>,
    mut panel: ResMut<EnabledPanel>,
    pause: Res<crate::graphics_menu::Menu>,
    menu: Res<ModMenu>,
    custom: Res<crate::customiser::Customiser>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    roots: Query<(&Root, &ComputedNode, &UiGlobalTransform)>,
    mut lists: Query<(
        &Viewport,
        &ComputedNode,
        &UiGlobalTransform,
        &mut ScrollPosition,
    )>,
    rows: Query<(&Row, &ComputedNode, &UiGlobalTransform)>,
) {
    let delta: f32 = wheel
        .read()
        .map(|event| match event.unit {
            MouseScrollUnit::Line => event.y * 40.,
            MouseScrollUnit::Pixel => event.y,
        })
        .sum();
    if !pause.open || menu.open || custom.open || panel.dragging() {
        return;
    }
    let hovered = window.physical_cursor_position().and_then(|cursor| {
        roots
            .iter()
            .filter(|(_, node, transform)| node.contains_point(**transform, cursor))
            .max_by_key(|(root, node, _)| {
                (panel.active.as_ref() == Some(&root.0), node.stack_index)
            })
            .map(|(root, _, _)| root.0.clone())
    });
    for (viewport, node, transform, mut position) in &mut lists {
        let Some(layout) = panel.layouts.get_mut(&viewport.0) else {
            continue;
        };
        if layout.collapsed {
            continue;
        }
        if hovered.as_ref() == Some(&viewport.0) && delta != 0. {
            position.0.y -= delta;
            layout.reveal_selected = false;
        }
        if layout.reveal_selected && node.size().y > 0. {
            if let Some((_, row_node, row_transform)) = rows
                .iter()
                .find(|(row, _, _)| row.0 == viewport.0 && row.1 == layout.selected)
            {
                let center = transform
                    .inverse()
                    .transform_point2(row_transform.translation);
                let top = center.y - row_node.size().y / 2. + node.size().y / 2.;
                let bottom = top + row_node.size().y;
                let shift = if top < 0. {
                    top
                } else {
                    (bottom - node.size().y).max(0.)
                };
                position.0.y += shift * node.inverse_scale_factor;
                layout.reveal_selected = false;
            }
        }
        let max = ((node.content_size().y - node.size().y) * node.inverse_scale_factor).max(0.);
        position.0.y = position.0.y.clamp(0., max);
    }
}

fn fit_size(size: Vec2, window: &Window) -> Vec2 {
    let maximum = Vec2::new(
        (window.width() - 16.).max(1.),
        (window.height() - 32.).max(1.),
    );
    size.clamp(Vec2::new(320., 260.).min(maximum), maximum)
}

fn fitted_position(layout: &Layout, window: &Window) -> Vec2 {
    let mut size = fit_size(layout.size, window);
    if layout.collapsed {
        size.y = 48.;
    }
    layout.position.clamp(
        Vec2::ZERO,
        (Vec2::new(window.width(), window.height()) - size).max(Vec2::ZERO),
    )
}
