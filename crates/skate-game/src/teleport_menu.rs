//! Authored FE travel destinations; private content is produced by the extractor.
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Deserialize)]
pub(crate) struct Destination {
    pub id: String,
    pub name: String,
    pub map: String,
    pub matrix: Option<[[f32; 4]; 4]>,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}
#[derive(Deserialize)]
struct Catalog {
    version: u32,
    destinations: Vec<Destination>,
}

pub(crate) fn load(assets: &Path) -> Result<Vec<Destination>, String> {
    let path = assets.join("private/teleports.json");
    let bytes = std::fs::read(&path).map_err(|e| format!("Teleport locations: {e}"))?;
    let catalog: Catalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if catalog.version != 1 {
        return Err("Unsupported teleport catalog version".into());
    }
    let mut ids = std::collections::HashSet::new();
    for d in &catalog.destinations {
        if d.name.is_empty()
            || d.id.is_empty()
            || !ids.insert(&d.id)
            || d.map.is_empty()
            || d.map.contains(['/', '\\', ':'])
            || matches!(d.map.as_str(), "." | "..")
        {
            return Err("Invalid teleport destination identity".into());
        }
        if let Some(m) = d.matrix {
            if !valid_matrix(m) {
                return Err(format!("Invalid teleport transform: {}", d.name));
            }
        }
    }
    Ok(catalog.destinations)
}

fn valid_matrix(m: [[f32; 4]; 4]) -> bool {
    m.iter().flatten().all(|v| v.is_finite())
        && (0..3).all(|i| m[i][3].abs() < 1e-5)
        && (m[3][3] - 1.).abs() < 1e-5
        && m[2][0] * m[2][0] + m[2][2] * m[2][2] > 1e-6
}

pub(crate) fn same_map(path: &Path, name: &str) -> bool {
    path.file_stem()
        .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_destinations_and_preserves_authored_heading() {
        let mut m = skate_core::physics::skeleton_animation_record::IDENTITY;
        m[2] = [1., 0., 0., 0.];
        m[3] = [350.5, 140.2, -725.3, 1.];
        assert!(valid_matrix(m));
        m[3][1] = f32::NAN;
        assert!(!valid_matrix(m));
        assert!(same_map(Path::new("maps/DownTown.skate"), "Downtown"));
        assert!(!same_map(Path::new("maps/MegaPark.skate"), "University"));
    }
}

use bevy::prelude::*;
#[derive(Resource, Default)]
pub(crate) struct Travel {
    pub open: bool,
    pub closed_this_frame: bool,
    shown: bool,
    rows: Vec<Destination>,
    selected: usize,
    generation: Option<u64>,
}
#[derive(Component)]
struct TravelRoot;
#[derive(Component)]
struct TravelRow(usize);
#[derive(Component)]
struct TravelScroll;
pub(crate) fn install(app: &mut App) {
    app.init_resource::<Travel>().add_systems(
        PreUpdate,
        interact
            .after(crate::customiser::navigation)
            .before(crate::graphics_menu::MenuInput),
    );
}
fn interact(
    mut commands: Commands,
    mut travel: ResMut<Travel>,
    config: Res<crate::config::Config>,
    map: Res<crate::map_transition::CurrentMap>,
    keys: Res<ButtonInput<KeyCode>>,
    nav: Res<crate::customiser::Navigation>,
    mut menu: ResMut<crate::graphics_menu::Menu>,
    mut skater: ResMut<crate::physics::SkaterRuntime>,
    roots: Query<Entity, With<TravelRoot>>,
    mut colors: Query<(&TravelRow, &mut BackgroundColor)>,
    buttons: Query<(&Interaction, &TravelRow), Changed<Interaction>>,
    mut scroll: Query<&mut ScrollPosition, With<TravelScroll>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
) {
    travel.closed_this_frame = false;
    let wheel_delta: f32 = wheel
        .read()
        .map(|e| {
            e.y * if e.unit == bevy::input::mouse::MouseScrollUnit::Line {
                36.
            } else {
                1.
            }
        })
        .sum();
    if travel.generation != Some(map.generation) {
        travel.generation = Some(map.generation);
        travel.open = false;
        travel.shown = false;
        for e in &roots {
            commands.entity(e).despawn();
        }
        travel.rows = load(&config.asset_root).unwrap_or_else(|e| {
            warn!("Travel destinations: {e}");
            vec![]
        });
        travel.rows.retain(|d| {
            d.matrix.is_some() && map.path.as_ref().is_some_and(|p| same_map(p, &d.map))
        });
        if let Some(id) = &config.teleport {
            if map.generation == 0 {
                if let Some(m) = travel
                    .rows
                    .iter()
                    .find(|d| &d.id == id)
                    .and_then(|d| d.matrix)
                {
                    let _ = skater.travel_to(m);
                }
            }
        }
    }
    if travel.open && !travel.shown {
        travel.selected = 0;
        travel.shown = true;
        commands
            .spawn((
                TravelRoot,
                GlobalZIndex(11),
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.015, 0.025, 0.04, 0.95)),
            ))
            .with_children(|root| {
                root.spawn(Node {
                    width: px(680),
                    max_width: percent(95),
                    padding: UiRect::all(px(18)),
                    flex_direction: FlexDirection::Column,
                    ..default()
                })
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("TELEPORT"),
                        TextFont {
                            font_size: 28.,
                            ..default()
                        },
                    ));
                    panel
                        .spawn((
                            TravelScroll,
                            ScrollPosition::default(),
                            Node {
                                height: px(360),
                                overflow: Overflow::scroll_y(),
                                flex_direction: FlexDirection::Column,
                                ..default()
                            },
                        ))
                        .with_children(|rows| {
                            for (i, d) in travel.rows.iter().enumerate() {
                                rows.spawn((
                                    Button,
                                    TravelRow(i),
                                    Node {
                                        height: px(36),
                                        flex_shrink: 0.,
                                        padding: UiRect::all(px(6)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.08, 0.11, 0.15)),
                                ))
                                .with_child((
                                    Text::new(&d.name),
                                    TextFont {
                                        font_size: 18.,
                                        ..default()
                                    },
                                ));
                            }
                            if travel.rows.is_empty() {
                                rows.spawn(Text::new("No destinations available on this map."));
                            }
                        });
                    panel
                        .spawn((
                            Button,
                            TravelRow(travel.rows.len()),
                            Node {
                                height: px(36),
                                ..default()
                            },
                        ))
                        .with_child(Text::new("Back to pause menu"));
                });
            });
        return;
    }
    if travel.open {
        let count = travel.rows.len() + 1;
        let up = keys.just_pressed(KeyCode::ArrowUp) || nav.pressed & 1 != 0;
        let down = keys.just_pressed(KeyCode::ArrowDown) || nav.pressed & 2 != 0;
        if up {
            travel.selected = (travel.selected + count - 1) % count;
        }
        if down {
            travel.selected = (travel.selected + 1) % count;
        }
        for mut p in &mut scroll {
            p.y = (p.y - wheel_delta).clamp(0., (travel.rows.len() as f32 * 36. - 360.).max(0.));
            if (up || down) && travel.selected < travel.rows.len() {
                let y = travel.selected as f32 * 36.;
                p.y = p.y.min(y).max(y + 36. - 360.).max(0.);
            }
        }
        let mut chosen = (keys.just_pressed(KeyCode::Enter) || nav.pressed & 0x1000 != 0)
            .then_some(travel.selected);
        for (interaction, row) in &buttons {
            if *interaction == Interaction::Pressed {
                chosen = Some(row.0);
            }
        }
        if keys.just_pressed(KeyCode::Escape) || nav.pressed & (0x2000 | 0x10) != 0 {
            chosen = Some(travel.rows.len());
        }
        if let Some(i) = chosen {
            if let Some(m) = travel.rows.get(i).and_then(|d| d.matrix) {
                match skater.travel_to(m) {
                    Ok(()) => menu.open = false,
                    Err(e) => warn!("Travel: {e}"),
                }
            }
            travel.closed_this_frame = menu.open;
            travel.open = false;
        }
    }
    for (row, mut color) in &mut colors {
        color.0 = if row.0 == travel.selected {
            Color::srgb(0.10, 0.30, 0.34)
        } else {
            Color::srgb(0.08, 0.11, 0.15)
        };
    }
    if !travel.open && travel.shown {
        for e in &roots {
            commands.entity(e).despawn();
        }
        travel.shown = false;
    }
}
