//! The headset menus, shared by both tiers: pages of buttons, toggles and
//! sliders drawn on the XR panel ([`super::panel`]), worked with the right
//! controller's laser (trigger clicks, drags a slider) or the sticks
//! (stick moves, A selects, stick left/right adjusts, B goes back).
//!
//! A tap of the left menu button opens and closes it. The first launch opens
//! the intro ([`pages`]) instead of the start page.
//!
//! Pages come from [`UiProvider`]s: the shared one (start, intro, AR
//! settings, build, Skate 3 data, controls) and whatever a tier adds (the
//! clean tier's skater page). A provider builds a page from the world each
//! frame and carries out what is pressed on it, so a page always shows live
//! values and needs no state of its own.
//!
//! Another menu can borrow the panel ([`XrUi::external`]): the clean tier's
//! full SK8 sheet. This module then only listens for the menu tap that
//! closes everything.
pub(crate) mod hud;
pub(crate) mod import;
pub(crate) mod pages;
mod pointer;

use super::actions::LatestTouch;
use super::build::BuildMode;
use super::panel::XrPanel;
use bevy::prelude::*;
use std::sync::Arc;

/// Rows on the panel at once; longer pages scroll with the cursor.
const VISIBLE_ROWS: usize = 13;
/// A menu press shorter than this (with nothing else held) is a tap.
const TAP_SECONDS: f32 = 0.35;
const STICK: f32 = 0.6;
const REPEAT_DELAY: f32 = 0.35;
const REPEAT_EVERY: f32 = 0.09;
const UI_FONT: &str = "embedded://skate_game/clean/assets/fonts/Montserrat-Italic.ttf";

pub(crate) type PageId = &'static str;

/// One row of a page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Widget {
    /// Opens another page.
    Link { label: String, page: PageId },
    Button { id: &'static str, label: String },
    Toggle { id: &'static str, label: String, on: bool },
    /// `text` is the value as shown ("1.5x", "on the floor").
    Slider { id: &'static str, label: String, value: f32, min: f32, max: f32, step: f32, text: String },
    /// Not selectable.
    Info(String),
}

impl Widget {
    pub(crate) fn link(label: impl Into<String>, page: PageId) -> Self {
        Widget::Link { label: label.into(), page }
    }
    pub(crate) fn button(id: &'static str, label: impl Into<String>) -> Self {
        Widget::Button { id, label: label.into() }
    }
    pub(crate) fn toggle(id: &'static str, label: impl Into<String>, on: bool) -> Self {
        Widget::Toggle { id, label: label.into(), on }
    }
    pub(crate) fn slider(
        id: &'static str,
        label: impl Into<String>,
        value: f32,
        (min, max, step): (f32, f32, f32),
        text: impl Into<String>,
    ) -> Self {
        Widget::Slider { id, label: label.into(), value, min, max, step, text: text.into() }
    }
    pub(crate) fn info(text: impl Into<String>) -> Self {
        Widget::Info(text.into())
    }

    fn selectable(&self) -> bool {
        !matches!(self, Widget::Info(_))
    }

    fn id(&self) -> &'static str {
        match self {
            Widget::Button { id, .. } | Widget::Toggle { id, .. } | Widget::Slider { id, .. } => id,
            Widget::Link { page, .. } => page,
            Widget::Info(_) => "",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct UiPage {
    pub title: String,
    pub widgets: Vec<Widget>,
}

impl UiPage {
    pub(crate) fn new(title: impl Into<String>, widgets: Vec<Widget>) -> Self {
        Self { title: title.into(), widgets }
    }
}

/// What a row was asked to do: pressed (buttons, toggles) or set to a value
/// (sliders, already clamped and on the step grid).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Act {
    Press,
    Set(f32),
}

/// What the menu does after a row acted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Stay,
    Open(PageId),
    /// Replace the current page (intro steps).
    Replace(PageId),
    Back,
    Close,
    Say(String),
    /// Hand the panel to another menu (see [`XrUi::external`]).
    External(&'static str),
}

pub(crate) trait UiProvider: Send + Sync + 'static {
    /// The page `id`, if this provider makes it.
    fn page(&self, id: PageId, world: &World) -> Option<UiPage>;
    /// Carry out `act` on row `widget` of page `page`; `None` if it isn't this
    /// provider's.
    fn act(&self, page: PageId, widget: &'static str, act: Act, world: &mut World) -> Option<Outcome>;
    /// Rows this provider adds to the start page.
    fn start_items(&self, _world: &World) -> Vec<Widget> {
        Vec::new()
    }
}

#[derive(Resource, Default, Clone)]
pub(crate) struct UiProviders(pub Vec<Arc<dyn UiProvider>>);

/// The controls list shown under "Controls" (each tier sets its own).
#[derive(Resource, Clone, Default)]
pub(crate) struct ControlsSheet(pub Vec<(String, String)>);

/// Set while a headset menu owns the controllers (the skater gets neutral
/// input).
#[derive(Resource, Default)]
pub(crate) struct UiOwnsControllers(pub bool);

#[derive(Resource, Default)]
pub(crate) struct XrUi {
    pub open: bool,
    stack: Vec<PageId>,
    cursor: usize,
    first: usize,
    status: (String, f32),
    /// Another menu has the panel (the clean tier's SK8 sheet); it hands it
    /// back by clearing this.
    pub external: Option<&'static str>,
    /// Open on this page next frame (other systems' requests).
    pub request: Option<PageId>,
    /// Put the skater in front of the player next frame.
    pub request_recenter: bool,
    drag: Option<usize>,
    view: View,
    /// The menu opened itself at launch.
    started: bool,
}

impl XrUi {
    fn show(&mut self, page: PageId) {
        self.open = true;
        self.external = None;
        self.stack = vec![pages::START];
        if page != pages::START {
            self.stack.push(page);
        }
        self.cursor = 0;
        self.first = 0;
    }

    fn close(&mut self) {
        self.open = false;
        self.external = None;
        self.drag = None;
    }

    pub(crate) fn say(&mut self, text: impl Into<String>) {
        self.status = (text.into(), 3.0);
    }
}

/// What the panel shows this frame (written by `frame`, drawn by `draw`).
#[derive(Clone, Debug, Default, PartialEq)]
struct View {
    title: String,
    trail: String,
    rows: Vec<RowView>,
    footer: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct RowView {
    label: String,
    value: String,
    /// Slider position 0..1.
    bar: Option<f32>,
    on: Option<bool>,
    selected: bool,
    dim: bool,
}

/// One frame of stick / button navigation.
#[derive(Clone, Copy, Debug, Default)]
struct Nav {
    toggle: bool,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    select: bool,
    back: bool,
    fast: bool,
}

#[derive(Resource, Default)]
struct UiNav(Nav);

pub(crate) struct XrUiPlugin {
    /// The imported-disc tier (it has no clean tier to embed the font).
    pub retail: bool,
}

impl Plugin for XrUiPlugin {
    fn build(&self, app: &mut App) {
        if self.retail {
            crate::clean::embed_font(app);
        }
        let mut providers = UiProviders::default();
        providers.0.push(Arc::new(pages::Shared));
        app.insert_resource(providers)
            .init_resource::<ControlsSheet>()
            .init_resource::<XrUi>()
            .init_resource::<UiNav>()
            .init_resource::<UiOwnsControllers>()
            .init_resource::<pointer::UiPointer>()
            .add_plugins((hud::HudPlugin, import::ImportPlugin))
            .add_systems(Startup, (spawn_ui, pointer::spawn_beam))
            .add_systems(
                Update,
                (touch_nav, pointer::aim, frame, draw, pointer::draw_beam)
                    .chain()
                    .in_set(UiSet),
            );
    }
}

/// The headset menu's frame; tiers order their own menu glue around it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct UiSet;

// ---------------------------------------------------------------- input

#[derive(Default)]
struct NavState {
    held: Option<(f32, bool)>,
    previous: super::touch::TouchState,
    repeat: [f32; 4],
}

/// Held-direction auto-repeat: true on the press, then every
/// `REPEAT_EVERY` after `REPEAT_DELAY`.
fn repeat(held: &mut f32, down: bool, dt: f32) -> bool {
    if !down {
        *held = 0.0;
        return false;
    }
    let before = *held;
    *held += dt;
    if before == 0.0 {
        return true;
    }
    let a = ((before - REPEAT_DELAY) / REPEAT_EVERY).floor();
    let b = ((*held - REPEAT_DELAY) / REPEAT_EVERY).floor();
    *held > REPEAT_DELAY && b > a
}

fn touch_nav(
    touch: Res<LatestTouch>,
    time: Res<Time<Real>>,
    ui: Res<XrUi>,
    mut state: Local<NavState>,
    mut nav: ResMut<UiNav>,
) {
    let t = touch.0;
    let p = state.previous;
    let dt = time.delta_secs();
    let mut n = Nav::default();
    // Menu tap (nothing chorded with it: holds are build mode, menu + grips
    // the SK8 sheet, menu + stick the pad's D-pad).
    let chorded = |t: &super::touch::TouchState| {
        t.x || t.left_grip > 0.5 || t.right_grip > 0.5 || t.left_stick[0].abs() > 0.3 || t.left_stick[1].abs() > 0.3
    };
    state.held = match (state.held, t.menu) {
        (None, true) => Some((0.0, chorded(&t))),
        (Some((s, c)), true) => Some((s + dt, c || chorded(&t))),
        (Some((s, c)), false) => {
            n.toggle = !c && s < TAP_SECONDS;
            None
        }
        (None, false) => None,
    };
    if ui.open && !t.menu {
        let pick = |a: f32, b: f32| if a.abs() > b.abs() { a } else { b };
        let y = pick(t.right_stick[1], t.left_stick[1]);
        let x = pick(t.right_stick[0], t.left_stick[0]);
        let r = &mut state.repeat;
        n.up = repeat(&mut r[0], y > STICK, dt);
        n.down = repeat(&mut r[1], y < -STICK, dt);
        n.left = repeat(&mut r[2], x < -STICK, dt);
        n.right = repeat(&mut r[3], x > STICK, dt);
        n.select = t.a && !p.a;
        n.back = t.b && !p.b;
        n.fast = t.left_trigger > 0.5;
    }
    state.previous = t;
    nav.0 = n;
}

// ---------------------------------------------------------------- logic

fn find_page(providers: &[Arc<dyn UiProvider>], id: PageId, world: &World) -> UiPage {
    providers
        .iter()
        .find_map(|p| p.page(id, world))
        .unwrap_or_else(|| UiPage::new("Menu", vec![Widget::info(format!("No page \"{id}\""))]))
}

/// `value` moved `steps` steps, clamped, on the step grid.
pub(crate) fn step_value(value: f32, steps: f32, min: f32, max: f32, step: f32) -> f32 {
    snap((value + steps * step).clamp(min, max), min, max, step)
}

/// The slider value at `frac` (0..1) along its bar.
pub(crate) fn value_at(frac: f32, min: f32, max: f32, step: f32) -> f32 {
    snap(min + frac.clamp(0.0, 1.0) * (max - min), min, max, step)
}

fn snap(v: f32, min: f32, max: f32, step: f32) -> f32 {
    if step <= 0.0 {
        return v;
    }
    (min + ((v - min) / step).round() * step).clamp(min, max)
}

/// The next selectable row from `from` going `dir`, wrapping.
fn next_selectable(widgets: &[Widget], from: usize, dir: isize) -> usize {
    let n = widgets.len() as isize;
    if n == 0 {
        return 0;
    }
    let mut i = from as isize;
    for _ in 0..n {
        i = (i + dir).rem_euclid(n);
        if widgets[i as usize].selectable() {
            return i as usize;
        }
    }
    from
}

fn first_selectable(widgets: &[Widget]) -> usize {
    widgets.iter().position(Widget::selectable).unwrap_or(0)
}

/// Opens, closes and navigates the menu and carries out what was pressed.
fn frame(world: &mut World) {
    let nav = world.resource::<UiNav>().0;
    let point = world.resource::<pointer::UiPointer>().clone();
    let dt = world.resource::<Time<Real>>().delta_secs();
    let providers = world.resource::<UiProviders>().0.clone();
    let building = world.resource::<BuildMode>().active;
    let onboarded = world.resource::<super::ar_settings::ArSettings>().onboarding_done;
    {
        // Open at launch: the intro the first time, else the start page
        // (the scripted demo, SK8_DEMO, starts skating).
        let mut ui = world.resource_mut::<XrUi>();
        if !ui.started {
            ui.started = true;
            if std::env::var_os("SK8_DEMO").is_none() {
                ui.request = Some(if onboarded { pages::START } else { pages::INTRO });
            }
        }
    }
    if std::mem::take(&mut world.resource_mut::<XrUi>().request_recenter) {
        pages::put_skater_ahead(world);
    }
    {
        let mut ui = world.resource_mut::<XrUi>();
        if ui.status.1 > 0.0 {
            ui.status.1 -= dt;
        }
        if let Some(page) = ui.request.take() {
            ui.show(page);
        } else if nav.toggle {
            if ui.open || ui.external.is_some() {
                ui.close();
            } else {
                ui.show(if onboarded { pages::START } else { pages::INTRO });
            }
        } else if building && ui.open {
            // Holding the menu button for build mode closes the menu.
            ui.close();
        }
    }
    let (open, external) = {
        let ui = world.resource::<XrUi>();
        (ui.open, ui.external.is_some())
    };
    // The panel shows this menu or the borrowed one; menus and build mode
    // both own the controllers.
    {
        let showing = open || external;
        let mut panel = world.resource_mut::<XrPanel>();
        if showing && !panel.visible {
            panel.recenter = true;
        }
        panel.visible = showing;
        world.resource_mut::<UiOwnsControllers>().0 = showing;
        if showing {
            world.resource_mut::<BuildMode>().active = false;
        }
    }
    if !open || external {
        world.resource_mut::<XrUi>().view = View::default();
        return;
    }

    let id = *world.resource::<XrUi>().stack.last().unwrap_or(&pages::START);
    let page = find_page(&providers, id, world);
    let mut outcome = Outcome::Stay;
    let mut acted: Option<(usize, Act)> = None;
    {
        let mut ui = world.resource_mut::<XrUi>();
        let w = &page.widgets;
        if ui.cursor >= w.len() || !w.get(ui.cursor).is_some_and(Widget::selectable) {
            ui.cursor = first_selectable(w);
        }
        if nav.up {
            ui.cursor = next_selectable(w, ui.cursor, -1);
        }
        if nav.down {
            ui.cursor = next_selectable(w, ui.cursor, 1);
        }
        // The laser picks the row it points at.
        let hovered = point.row.map(|slot| ui.first + slot).filter(|&i| w.get(i).is_some_and(Widget::selectable));
        if let Some(i) = hovered.filter(|_| ui.drag.is_none()) {
            ui.cursor = i;
        }
        if !point.held {
            ui.drag = None;
        }
        let cursor = ui.cursor;
        match w.get(cursor) {
            Some(Widget::Slider { value, min, max, step, .. }) => {
                // Held left trigger: five steps at a time.
                let gain = if nav.fast { 5.0 } else { 1.0 };
                if nav.left || nav.right {
                    let dir = if nav.left { -gain } else { gain };
                    acted = Some((cursor, Act::Set(step_value(*value, dir, *min, *max, *step))));
                }
                if point.click && hovered == Some(cursor) && point.over_bar(ui.first, cursor) {
                    ui.drag = Some(cursor);
                }
                if ui.drag == Some(cursor) {
                    if let Some(frac) = point.bar_frac(ui.first, cursor) {
                        let v = value_at(frac, *min, *max, *step);
                        if v != *value {
                            acted = Some((cursor, Act::Set(v)));
                        }
                    }
                }
            }
            Some(widget) if widget.selectable() => {
                if nav.select || (point.click && hovered == Some(cursor)) {
                    match widget {
                        Widget::Link { page, .. } => outcome = Outcome::Open(page),
                        _ => acted = Some((cursor, Act::Press)),
                    }
                }
            }
            _ => {}
        }
        if nav.back && acted.is_none() && outcome == Outcome::Stay {
            outcome = Outcome::Back;
        }
    }
    if let Some((i, act)) = acted {
        let widget = page.widgets[i].id();
        outcome = providers.iter().find_map(|p| p.act(id, widget, act, world)).unwrap_or(Outcome::Stay);
    }
    {
        let mut ui = world.resource_mut::<XrUi>();
        match outcome {
            Outcome::Stay => {}
            Outcome::Open(p) => {
                ui.stack.push(p);
                ui.cursor = 0;
                ui.first = 0;
            }
            Outcome::Replace(p) => {
                ui.stack.pop();
                ui.stack.push(p);
                ui.cursor = 0;
                ui.first = 0;
            }
            Outcome::Back => {
                if ui.stack.len() > 1 {
                    ui.stack.pop();
                    ui.cursor = 0;
                    ui.first = 0;
                } else {
                    ui.close();
                }
            }
            Outcome::Close => ui.close(),
            Outcome::Say(text) => ui.say(text),
            Outcome::External(what) => {
                ui.open = false;
                ui.external = Some(what);
            }
        }
        if !ui.open {
            ui.view = View::default();
            return;
        }
    }
    // Draw the page as it is now (values changed by this frame's press).
    let id = *world.resource::<XrUi>().stack.last().unwrap_or(&pages::START);
    let page = find_page(&providers, id, world);
    let trail = {
        let ui = world.resource::<XrUi>();
        ui.stack
            .iter()
            .map(|&p| find_page(&providers, p, world).title)
            .collect::<Vec<_>>()
            .join("  >  ")
    };
    let mut ui = world.resource_mut::<XrUi>();
    if ui.cursor >= page.widgets.len() || !page.widgets[ui.cursor].selectable() {
        ui.cursor = first_selectable(&page.widgets);
    }
    let cursor = ui.cursor;
    // Scroll so the cursor is on screen.
    if cursor < ui.first {
        ui.first = cursor;
    } else if cursor >= ui.first + VISIBLE_ROWS {
        ui.first = cursor + 1 - VISIBLE_ROWS;
    }
    ui.first = ui.first.min(page.widgets.len().saturating_sub(VISIBLE_ROWS));
    let rows = page
        .widgets
        .iter()
        .enumerate()
        .skip(ui.first)
        .take(VISIBLE_ROWS)
        .map(|(i, w)| row_view(w, i == cursor))
        .collect();
    let hint = "Point + trigger, or stick + A.   B: back   Menu: close";
    let footer = if ui.status.1 > 0.0 { format!("{}\n{hint}", ui.status.0) } else { hint.to_string() };
    ui.view = View { title: page.title, trail, rows, footer };
}

fn row_view(w: &Widget, selected: bool) -> RowView {
    match w {
        Widget::Link { label, .. } => RowView { label: label.clone(), value: ">".into(), selected, ..default() },
        Widget::Button { label, .. } => RowView { label: label.clone(), selected, ..default() },
        Widget::Toggle { label, on, .. } => RowView {
            label: label.clone(),
            value: if *on { "ON".into() } else { "OFF".into() },
            on: Some(*on),
            selected,
            ..default()
        },
        Widget::Slider { label, value, min, max, text, .. } => RowView {
            label: label.clone(),
            value: text.clone(),
            bar: Some(if max > min { (value - min) / (max - min) } else { 0.0 }),
            selected,
            ..default()
        },
        Widget::Info(text) => RowView { label: text.clone(), dim: true, ..default() },
    }
}

// ---------------------------------------------------------------- view

#[derive(Component)]
struct UiRoot;
#[derive(Component)]
struct UiTitle;
#[derive(Component)]
struct UiTrail;
#[derive(Component)]
struct UiFooter;
/// Row slot `i` (the hit test reads its rectangle).
#[derive(Component)]
pub(crate) struct UiRow(pub usize);
#[derive(Component)]
struct UiLabel(usize);
#[derive(Component)]
struct UiValue(usize);
/// A slider's bar in row slot `i` (the hit test reads it too).
#[derive(Component)]
pub(crate) struct UiBar(pub usize);
#[derive(Component)]
struct UiBarFill(usize);

const TEAL: Color = Color::srgb(0.45, 0.9, 0.85);
const SELECTED: Color = Color::srgb(0.12, 0.42, 0.4);
const DIM: Color = Color::srgb(0.72, 0.78, 0.8);
const OFF: Color = Color::srgb(0.55, 0.58, 0.6);

fn spawn_ui(mut commands: Commands, panel: Res<XrPanel>, assets: Res<AssetServer>) {
    let font = assets.load(UI_FONT);
    let text = |s: &str, size: f32, color: Color| {
        (Text::new(s), TextFont { font_size: size, ..default() }, TextColor(color))
    };
    let heavy = TextFont { font: font.clone(), font_size: 44.0, weight: FontWeight(800), ..default() };
    commands
        .spawn((
            UiRoot,
            Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(20),
            UiTargetCamera(panel.camera),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: px(960),
                    max_width: percent(98),
                    padding: UiRect::all(px(22)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(5),
                    border_radius: BorderRadius::all(px(18)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.05, 0.07, 0.92)),
            ))
            .with_children(|panel| {
                panel.spawn((UiTitle, Text::new("SK8 AR"), heavy.clone(), TextColor(Color::WHITE)));
                panel.spawn((UiTrail, text("", 20.0, TEAL)));
                for i in 0..VISIBLE_ROWS {
                    panel
                        .spawn((
                            UiRow(i),
                            Node {
                                width: percent(100),
                                min_height: px(46),
                                padding: UiRect::axes(px(12), px(4)),
                                align_items: AlignItems::Center,
                                column_gap: px(14),
                                border_radius: BorderRadius::all(px(8)),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                        ))
                        .with_children(|r| {
                            r.spawn((UiLabel(i), text("", 26.0, Color::WHITE), Node { flex_grow: 1.0, ..default() }));
                            r.spawn((
                                UiBar(i),
                                Node {
                                    width: px(230),
                                    height: px(14),
                                    border_radius: BorderRadius::all(px(7)),
                                    display: Display::None,
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.15)),
                            ))
                            .with_children(|bar| {
                                bar.spawn((
                                    UiBarFill(i),
                                    Node {
                                        width: percent(0),
                                        height: percent(100),
                                        border_radius: BorderRadius::all(px(7)),
                                        ..default()
                                    },
                                    BackgroundColor(TEAL),
                                ));
                            });
                            r.spawn((
                                UiValue(i),
                                text("", 26.0, TEAL),
                                Node { min_width: px(110), justify_content: JustifyContent::FlexEnd, ..default() },
                            ));
                        });
                }
                panel.spawn((UiFooter, text("", 19.0, DIM)));
            });
        });
}

#[allow(clippy::type_complexity)]
fn draw(
    ui: Res<XrUi>,
    mut drawn: Local<View>,
    mut root: Query<&mut Node, (With<UiRoot>, Without<UiRow>, Without<UiBar>, Without<UiBarFill>)>,
    mut texts: ParamSet<(
        Query<&mut Text, With<UiTitle>>,
        Query<&mut Text, With<UiTrail>>,
        Query<&mut Text, With<UiFooter>>,
        Query<(&UiLabel, &mut Text, &mut TextColor)>,
        Query<(&UiValue, &mut Text, &mut TextColor)>,
    )>,
    mut rows: Query<(&UiRow, &mut BackgroundColor, &mut Node), (Without<UiRoot>, Without<UiBar>, Without<UiBarFill>)>,
    mut bars: Query<(&UiBar, &mut Node), (Without<UiRoot>, Without<UiRow>, Without<UiBarFill>)>,
    mut fills: Query<(&UiBarFill, &mut Node), (Without<UiRoot>, Without<UiRow>, Without<UiBar>)>,
) {
    let Ok(mut node) = root.single_mut() else { return };
    let display = if ui.open && ui.external.is_none() { Display::Flex } else { Display::None };
    if node.display != display {
        node.display = display;
    }
    if *drawn == ui.view {
        return;
    }
    *drawn = ui.view.clone();
    let v = &ui.view;
    for mut t in &mut texts.p0() {
        **t = v.title.clone();
    }
    for mut t in &mut texts.p1() {
        **t = v.trail.clone();
    }
    for mut t in &mut texts.p2() {
        **t = v.footer.clone();
    }
    for (UiLabel(i), mut t, mut color) in &mut texts.p3() {
        let r = v.rows.get(*i);
        **t = r.map(|r| r.label.clone()).unwrap_or_default();
        color.0 = if r.is_some_and(|r| r.dim) { DIM } else { Color::WHITE };
    }
    for (UiValue(i), mut t, mut color) in &mut texts.p4() {
        let r = v.rows.get(*i);
        **t = r.map(|r| r.value.clone()).unwrap_or_default();
        color.0 = match r.and_then(|r| r.on) {
            Some(false) => OFF,
            _ => TEAL,
        };
    }
    for (UiRow(i), mut bg, mut node) in &mut rows {
        let r = v.rows.get(*i);
        bg.0 = if r.is_some_and(|r| r.selected) { SELECTED } else { Color::NONE };
        let display = if r.is_some() { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
    for (UiBar(i), mut node) in &mut bars {
        let display = if v.rows.get(*i).is_some_and(|r| r.bar.is_some()) { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
    for (UiBarFill(i), mut node) in &mut fills {
        let frac = v.rows.get(*i).and_then(|r| r.bar).unwrap_or(0.0);
        node.width = percent(frac.clamp(0.0, 1.0) * 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_clamp_and_snap() {
        assert_eq!(step_value(1.0, 1.0, 0.5, 4.0, 0.25), 1.25);
        assert_eq!(step_value(3.9, 1.0, 0.5, 4.0, 0.25), 4.0);
        assert_eq!(step_value(0.5, -1.0, 0.5, 4.0, 0.25), 0.5);
        assert_eq!(value_at(0.0, 0.5, 4.0, 0.25), 0.5);
        assert_eq!(value_at(1.0, 0.5, 4.0, 0.25), 4.0);
        assert_eq!(value_at(0.5, 0.0, 2.0, 0.1), 1.0);
        assert_eq!(value_at(2.0, 0.0, 2.0, 0.1), 2.0);
    }

    #[test]
    fn cursor_skips_info_rows_and_wraps() {
        let w = vec![
            Widget::info("a"),
            Widget::button("b", "b"),
            Widget::info("c"),
            Widget::toggle("d", "d", true),
        ];
        assert_eq!(first_selectable(&w), 1);
        assert_eq!(next_selectable(&w, 1, 1), 3);
        assert_eq!(next_selectable(&w, 3, 1), 1);
        assert_eq!(next_selectable(&w, 1, -1), 3);
        assert_eq!(next_selectable(&[Widget::info("x")], 0, 1), 0);
    }

    #[test]
    fn taps_repeat_after_the_delay() {
        let mut h = 0.0;
        assert!(repeat(&mut h, true, 0.016));
        let fired: u32 = (0..60).map(|_| repeat(&mut h, true, 0.016) as u32).sum();
        assert!(fired >= 4, "{fired}");
        assert!(!repeat(&mut h, false, 0.016));
    }
}
