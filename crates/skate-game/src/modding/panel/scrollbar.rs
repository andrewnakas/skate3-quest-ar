//! A scrollbar tied to the actual settings viewport, including resize and wheel changes.
use super::*;

#[derive(Component)]
pub(super) struct Track(String);
#[derive(Component)]
pub(super) struct Thumb(String);

pub(super) fn spawn(area: &mut ChildSpawnerCommands, id: &str) {
    area.spawn((
        Button,
        Track(id.into()),
        Node {
            width: px(14.),
            height: percent(100),
            flex_shrink: 0.,
            border_radius: BorderRadius::all(px(6.)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.07, 0.11, 0.16)),
    ))
    .with_children(|track| {
        track.spawn((
            Thumb(id.into()),
            bevy::ui::FocusPolicy::Pass,
            Node {
                position_type: PositionType::Absolute,
                left: px(2.),
                right: px(2.),
                top: px(0.),
                height: percent(100),
                border_radius: BorderRadius::all(px(5.)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.25, 0.65, 0.68)),
        ));
    });
}

pub(super) fn update(
    mut panel: ResMut<EnabledPanel>,
    pause: Res<crate::graphics_menu::Menu>,
    menu: Res<ModMenu>,
    custom: Res<crate::customiser::Customiser>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    tracks: Query<(&Track, &Interaction, &ComputedNode, &UiGlobalTransform)>,
    mut thumbs: Query<(&Thumb, &mut Node, &mut BackgroundColor)>,
    mut lists: Query<(&Viewport, &ComputedNode, &mut ScrollPosition)>,
) {
    if !mouse.pressed(MouseButton::Left) || !pause.open || menu.open || custom.open {
        panel.scroll_drag = None;
    }
    if !pause.open || menu.open || custom.open || panel.drag.is_some() || panel.resize.is_some() {
        return;
    }
    for (track, interaction, geometry, transform) in &tracks {
        if !panel
            .layouts
            .get(&track.0)
            .is_some_and(|layout| !layout.collapsed)
        {
            continue;
        }
        let Some((_, viewport, mut scroll)) = lists.iter_mut().find(|(v, _, _)| v.0 == track.0)
        else {
            continue;
        };
        let height = geometry.size().y * geometry.inverse_scale_factor;
        if height <= 0. {
            continue;
        }
        let visible = viewport.size().y;
        let content = viewport.content_size().y.max(visible);
        let maximum = ((content - visible) * viewport.inverse_scale_factor).max(0.);
        let thumb_height = (height * visible / content.max(1.)).max(24.).min(height);
        let travel = height - thumb_height;
        scroll.0.y = scroll.0.y.clamp(0., maximum);
        let mut top = if maximum > 0. {
            travel * scroll.0.y / maximum
        } else {
            0.
        };
        if let Some(cursor) = window.physical_cursor_position() {
            let y = (transform.inverse().transform_point2(cursor).y + geometry.size().y / 2.)
                * geometry.inverse_scale_factor;
            if maximum > 0.
                && travel > 0.
                && *interaction == Interaction::Pressed
                && mouse.just_pressed(MouseButton::Left)
            {
                let offset = if (top..=top + thumb_height).contains(&y) {
                    (y - top) / thumb_height
                } else {
                    0.5
                };
                panel.scroll_drag = Some((track.0.clone(), offset));
                panel.active = Some(track.0.clone());
                panel.focused = true;
            }
            if let Some((id, offset)) = &panel.scroll_drag {
                if id == &track.0 && travel > 0. {
                    top = (y - offset * thumb_height).clamp(0., travel);
                    scroll.0.y = top / travel * maximum;
                    if let Some(layout) = panel.layouts.get_mut(&track.0) {
                        layout.reveal_selected = false;
                    }
                }
            }
        }
        if let Some((_, mut node, mut color)) =
            thumbs.iter_mut().find(|(thumb, _, _)| thumb.0 == track.0)
        {
            node.top = px(top);
            node.height = px(thumb_height);
            color.0 = if maximum <= 0. {
                Color::srgb(0.16, 0.24, 0.28)
            } else if *interaction != Interaction::None
                || panel
                    .scroll_drag
                    .as_ref()
                    .is_some_and(|(id, _)| id == &track.0)
            {
                Color::srgb(0.35, 0.9, 0.85)
            } else {
                Color::srgb(0.25, 0.65, 0.68)
            };
        }
    }
}
