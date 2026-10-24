//! The clean tier's menus: the start screen, How to play, and the SK8 menu
//! (live tuning of the clean-room engine's own values, practice tools and
//! presets). One UI tree, shown on the desktop window or, in XR, on a panel
//! in front of the player (`xr::panel`).
//!
//! Opening: start screen = tap the left menu button (Esc on the desktop);
//! SK8 menu = hold the left menu button and squeeze both grips (Tab).
//! Inside: stick / arrows move, A / Enter selects, B / Backspace goes back.
//! In the SK8 menu: grips / Q E switch tabs, stick left/right / arrows adjust
//! (hold a trigger / Shift for fine, the other trigger / Ctrl for coarse),
//! A / F freezes a value (presets and "reset all" leave it alone), X / R
//! resets it.
use super::UiCapture;
use super::sim::CleanSkate;
use bevy::prelude::*;
use skate_lite_sys::EventKind;
use serde::{Deserialize, Serialize};
use skate_lite_sys::{Mode, Param, flags};
use std::path::PathBuf;

const VISIBLE_ROWS: usize = 15;
const MARKERS: usize = 5;
const SAVE_FILE: &str = "sk8-tuning.json";
/// Held-direction auto-repeat timing, seconds.
const REPEAT_DELAY: f32 = 0.35;
const REPEAT_EVERY: f32 = 0.09;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    Start,
    HowTo(usize),
    Sk8,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct Marker {
    pub position: [f32; 3],
    pub facing: f32,
}

#[derive(Clone, Debug)]
enum Tab {
    Params(&'static str, Vec<usize>),
    Practice,
    World,
    Presets,
}

impl Tab {
    fn title(&self) -> &str {
        match self {
            Tab::Params(group, _) => group,
            Tab::Practice => "Practice",
            Tab::World => "World",
            Tab::Presets => "Presets",
        }
    }
}

/// Requests the XR layer applies (the menu does not own the rig or the park).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XrRequests {
    pub park_scale: Option<f32>,
    pub world_scale: Option<f32>,
    pub build_mode: bool,
    pub recenter: bool,
}

#[derive(Resource)]
pub(crate) struct Menu {
    pub open: bool,
    pub page: Page,
    cursor: usize,
    tab: usize,
    tabs: Vec<Tab>,
    params: Vec<Param>,
    frozen: Vec<bool>,
    markers: [Option<Marker>; MARKERS],
    last_marker: Option<usize>,
    auto_return: bool,
    never_bail: bool,
    status: (String, f32),
    files: PathBuf,
    dirty: bool,
    pub xr: bool,
    /// Current values mirrored from the XR layer, and what to change there.
    pub park_scale: f32,
    pub world_scale: f32,
    pub xr_requests: XrRequests,
    retail_available: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    version: u32,
    #[serde(default)]
    values: std::collections::BTreeMap<String, f32>,
    #[serde(default)]
    frozen: Vec<String>,
    #[serde(default)]
    never_bail: bool,
    #[serde(default)]
    auto_return: bool,
    #[serde(default)]
    markers: Vec<Option<Marker>>,
}

impl Menu {
    pub(crate) fn new(files: PathBuf, xr: bool) -> Self {
        let params = skate_lite_sys::params();
        let mut tabs: Vec<Tab> = Vec::new();
        for p in &params {
            match tabs.iter_mut().find(|t| matches!(t, Tab::Params(g, _) if *g == p.group)) {
                Some(Tab::Params(_, list)) => list.push(p.index),
                _ => tabs.push(Tab::Params(p.group, vec![p.index])),
            }
        }
        tabs.push(Tab::Practice);
        tabs.push(Tab::World);
        tabs.push(Tab::Presets);
        let retail_available = files.join("assets/private/game.json").is_file();
        Self {
            // The scripted demo (SK8_DEMO) skips the start screen.
            open: std::env::var_os("SK8_DEMO").is_none(),
            page: Page::Start,
            cursor: 0,
            tab: 0,
            frozen: vec![false; params.len()],
            tabs,
            params,
            markers: [None; MARKERS],
            last_marker: None,
            auto_return: false,
            never_bail: false,
            status: (String::new(), 0.0),
            files,
            dirty: false,
            xr,
            park_scale: 1.0,
            world_scale: 12.0,
            xr_requests: XrRequests::default(),
            retail_available,
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.status = (text.into(), 3.0);
    }

    fn path(&self) -> PathBuf {
        self.files.join(SAVE_FILE)
    }

    fn load(&mut self, sim: &mut CleanSkate) {
        let Ok(bytes) = std::fs::read(self.path()) else { return };
        let saved: Saved = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                warn!("SK8 menu: ignoring {}: {e}", self.path().display());
                return;
            }
        };
        for p in &self.params {
            if let Some(&v) = saved.values.get(p.key) {
                sim.rider.set_param(p.index, v);
            }
            self.frozen[p.index] = saved.frozen.iter().any(|k| k == p.key);
        }
        self.never_bail = saved.never_bail;
        self.auto_return = saved.auto_return;
        for (slot, m) in self.markers.iter_mut().zip(saved.markers) {
            *slot = m;
        }
        info!("SK8 menu: loaded {} tuned values from {}", saved.values.len(), self.path().display());
    }

    fn save(&mut self, sim: &CleanSkate) {
        self.dirty = false;
        let saved = Saved {
            version: 1,
            values: self
                .params
                .iter()
                .filter(|p| sim.rider.param(p.index) != p.default)
                .map(|p| (p.key.to_string(), sim.rider.param(p.index)))
                .collect(),
            frozen: self.params.iter().filter(|p| self.frozen[p.index]).map(|p| p.key.to_string()).collect(),
            never_bail: self.never_bail,
            auto_return: self.auto_return,
            markers: self.markers.to_vec(),
        };
        let result = serde_json::to_vec_pretty(&saved)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(self.path(), json).map_err(|e| e.to_string()));
        if let Err(e) = result {
            warn!("SK8 menu: could not save {}: {e}", self.path().display());
        }
    }
}

// ---------------------------------------------------------------- input

/// One frame of menu navigation, from Touch or the keyboard.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Nav {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub select: bool,
    pub back: bool,
    pub alt: bool,
    pub tab_prev: bool,
    pub tab_next: bool,
    pub fine: bool,
    pub coarse: bool,
    pub toggle_start: bool,
    pub toggle_sk8: bool,
}

/// Held-direction auto-repeat.
#[derive(Default)]
pub(crate) struct Repeat {
    held: [f32; 4],
}

impl Repeat {
    fn press(&mut self, i: usize, down: bool, dt: f32) -> bool {
        if !down {
            self.held[i] = 0.0;
            return false;
        }
        let before = self.held[i];
        self.held[i] += dt;
        if before == 0.0 {
            return true;
        }
        let a = ((before - REPEAT_DELAY) / REPEAT_EVERY).floor();
        let b = ((self.held[i] - REPEAT_DELAY) / REPEAT_EVERY).floor();
        self.held[i] > REPEAT_DELAY && b > a
    }

    pub(crate) fn directions(&mut self, up: bool, down: bool, left: bool, right: bool, dt: f32) -> [bool; 4] {
        [
            self.press(0, up, dt),
            self.press(1, down, dt),
            self.press(2, left, dt),
            self.press(3, right, dt),
        ]
    }
}

#[derive(Resource, Default)]
pub(crate) struct MenuNav(pub Nav);

pub(crate) fn keyboard_nav(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut repeat: Local<Repeat>,
    mut nav: ResMut<MenuNav>,
) {
    let [up, down, left, right] = repeat.directions(
        keys.pressed(KeyCode::ArrowUp),
        keys.pressed(KeyCode::ArrowDown),
        keys.pressed(KeyCode::ArrowLeft),
        keys.pressed(KeyCode::ArrowRight),
        time.delta_secs(),
    );
    nav.0 = Nav {
        up,
        down,
        left,
        right,
        select: keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::KeyF),
        back: keys.just_pressed(KeyCode::Backspace),
        alt: keys.just_pressed(KeyCode::KeyR),
        tab_prev: keys.just_pressed(KeyCode::KeyQ),
        tab_next: keys.just_pressed(KeyCode::KeyE),
        fine: keys.pressed(KeyCode::ShiftLeft),
        coarse: keys.pressed(KeyCode::ControlLeft),
        toggle_start: keys.just_pressed(KeyCode::Escape),
        toggle_sk8: keys.just_pressed(KeyCode::Tab),
    };
}

// ---------------------------------------------------------------- rows

struct Row {
    text: String,
    dim: bool,
}

/// A best's value as the engine gives it (points, hundredths of a second or metre, a count).
fn best_text(what: &str, v: u32) -> String {
    match what {
        "Air time" => format!("{:.2} s", v as f32 / 100.0),
        "Grind length" | "Manual length" => format!("{:.1} m", v as f32 / 100.0),
        "Line score" => format!("{v} points"),
        _ => v.to_string(),
    }
}

fn row(text: impl Into<String>) -> Row {
    Row { text: text.into(), dim: false }
}

fn info_row(text: impl Into<String>) -> Row {
    Row { text: text.into(), dim: true }
}

const START: [&str; 6] = ["Skate", "Build a park", "How to play", "SK8 menu", "Put the skater in front of me", "Close"];

const HOWTO_TITLES: [&str; 4] = ["Skating", "Tricks", "Building a park", "SK8 menu"];

fn howto_rows(page: usize, xr: bool) -> Vec<Row> {
    let pair = |a: &str, b: &str| info_row(format!("{a:<24} {b}"));
    match page {
        0 if xr => vec![
            pair("Left stick", "steer; in the air, spin"),
            pair("Right stick", "hold down to crouch, flick to pop"),
            pair("Right stick half down/up", "manual / nose manual (depth balances)"),
            pair("Left stick click + flick", "old school: no comply, boneless, 180s"),
            pair("Left stick click in the air", "+ left stick: body varial"),
            pair("Left stick click + half down/up", "primo / casper"),
            pair("A (hold)", "push"),
            pair("X (hold)", "mongo push"),
            pair("B", "brake"),
            pair("Y", "get off / back on the board (on foot: A jumps)"),
            pair("Left stick back + side", "powerslide (or right grip + steer)"),
            pair("Right grip as you land", "revert"),
            pair("Right grip up a ramp", "handplant on the lip (hold; A/X/B: feet off)"),
            pair("Trigger + A (on the ground)", "boneless (X: fastplant; left trigger FS)"),
            pair("Trigger (rolling / on a rail)", "a hand on the board / a grind grab"),
            pair("A / X (in the air)", "front / back foot off (one-foot ollie or grab)"),
            pair("B while grabbing", "let go of the board (dismount bail)"),
            pair("Flick back the other way", "just after a kick/heelflip: underflip"),
            pair("Right grip during a flip", "catch it upside down (flick again: dark to light)"),
            pair("Left stick hard over, slow", "kickturn"),
            pair("Grab + right stick swirl", "finger flip"),
            pair("Grab + R down + left stick", "shifty"),
            pair("B in a nose/tail / double grab", "airwalk / superman"),
            pair("Right stick (plain air)", "board adjust"),
            pair("Left grip", "nose manual"),
            pair("Triggers (in the air)", "grab; right stick held first picks it"),
            pair("Left grip + trigger", "aim, let go to put the skater there"),
            pair("Left menu (tap)", "this menu"),
            pair("Left menu (hold 1 s)", "build mode"),
            pair("Left menu + both grips", "SK8 menu"),
        ],
        0 => vec![
            pair("A / D", "steer; in the air, spin"),
            pair("Arrow keys", "right stick: hold down, flick to pop"),
            pair("Space", "push"),
            pair("Left Alt", "mongo push"),
            pair("Left Shift", "brake"),
            pair("G", "get off / back on the board"),
            pair("S + A / D", "powerslide (or C + steer)"),
            pair("C held as you land", "revert"),
            pair("C held up a ramp", "handplant on the lip (hold it)"),
            pair("Q / E + Space", "boneless (Left Alt: fastplant; Q frontside)"),
            pair("Z / X", "manual / nose manual (W/S balance)"),
            pair("Q / E (in the air)", "grab; arrow keys held first pick it"),
            pair("Esc / Tab", "this menu / SK8 menu"),
            info_row("A gamepad works too: A push, X mongo, B brake, Y off board, left stick back-left/right powerslide."),
            info_row("Manual: right stick part-way down (nose: part-way up), hold it there."),
            info_row("Hold the stick at the end of a flick for more turns: double, triple, quad."),
            info_row("Old school: hold F (pad: left stick click) and flick: no comply, boneless..."),
            info_row("Handplant: hold RB riding up a ramp; while planted A / X / B take feet off, right stick down / sideways."),
            info_row("Boneless: hold a trigger and press A on the ground (X: fastplant; left trigger frontside, right backside)."),
            info_row("Underflip: flick a kickflip / heelflip, then at once the mirror way (up-left / up-right)."),
            info_row("In the air A / X take a foot off (one-foot ollie, one-foot grab); a trigger on a rail grabs the board."),
            info_row("RB during a flip catches it dark (flick again: dark to light; landing dark runs it out); flick off a darkslide."),
            info_row("Late flips from up: back-foot flips. Slow + left stick hard over: kickturn. Left stick on a 5-0: salad / willy."),
            info_row("Grabbing: swirl the right stick (finger flip), R down + left stick (shifty), B (airwalk / superman)."),
            info_row("Grabs: point the right stick, then pull a trigger: up tail / seatbelt, down crail / nose,"),
            info_row("  sideways stalefish / mute / stiffy, both triggers double, centred frontside / backside grab."),
            info_row("  Trigger first, then push the stick: tweaks (nosebone, tailbone, japan, cross bone...)."),
        ],
        1 => {
            let mut rows = vec![info_row("Hold the right stick down to crouch, then:")];
            for (i, name) in skate_lite_sys::trick_names().into_iter().enumerate() {
                rows.push(pair(name, &describe_path(&skate_lite_sys::trick_demo_path(i))));
            }
            rows.push(info_row("Flick again in the air for a late flip. Land with the"));
            rows.push(info_row("board under you, rolling forward or fakie."));
            rows
        }
        2 => vec![
            pair("Left stick left/right", "choose a piece"),
            pair("Right controller", "point; trigger drops the piece"),
            pair("Right stick", "turn / resize (or the piece pointed at)"),
            pair("Right grip on a piece", "pick it up"),
            pair("B on a piece", "delete it"),
            pair("Right stick click", "copy the piece pointed at"),
            pair("Left trigger", "undo"),
            pair("Left stick click", "drop a starter park"),
            pair("Left stick up/down", "grow/shrink everything"),
            pair("Left grip + stick up/down", "grow/shrink only the park"),
            pair("A", "show the detected planes"),
            pair("Y", "rescan the room"),
            pair("Hold X", "clear every piece"),
            pair("Hold left menu 1 s", "leave build mode"),
        ],
        _ => vec![
            pair("Open", if xr { "hold left menu + squeeze both grips" } else { "Tab" }),
            pair("Tabs", if xr { "grips" } else { "Q / E" }),
            pair("Select / adjust", if xr { "stick" } else { "arrow keys" }),
            pair("Fine / coarse", if xr { "hold left / right trigger" } else { "Shift / Ctrl" }),
            pair("Freeze a value", if xr { "A" } else { "F" }),
            pair("Reset a value", if xr { "X" } else { "R" }),
            pair("Close", if xr { "B" } else { "Backspace" }),
            info_row("Practice: game speed, pause, frame step, five markers,"),
            info_row("return to a marker after a bail, never bail."),
            info_row("Every value is this engine's own; nothing is from a game disc."),
        ],
    }
}

/// "down, centre, up-left" for a stick path.
fn describe_path(path: &[[f32; 2]]) -> String {
    const NAMES: [&str; 8] = ["up", "up-right", "right", "down-right", "down", "down-left", "left", "up-left"];
    let mut out: Vec<&str> = Vec::new();
    for &[x, y] in path {
        let r = (x * x + y * y).sqrt();
        let token = if r >= 0.55 {
            let s = (x.atan2(y) / (std::f32::consts::PI / 4.0)).round() as i32;
            NAMES[s.rem_euclid(8) as usize]
        } else if r < 0.3 && !out.is_empty() {
            "centre"
        } else {
            continue;
        };
        if out.last() != Some(&token) {
            out.push(token);
        }
    }
    if out.last() == Some(&"centre") {
        out.pop();
        out.push("let go");
    }
    out.join(", ")
}

fn fmt_value(p: &Param, v: f32) -> String {
    if p.step >= 1.0 {
        format!("{v:.0}")
    } else if p.step >= 0.05 {
        format!("{v:.2}")
    } else {
        format!("{v:.4}")
    }
}

fn sk8_rows(menu: &Menu, sim: &CleanSkate) -> Vec<Row> {
    match &menu.tabs[menu.tab] {
        Tab::Params(_, list) => list
            .iter()
            .map(|&i| {
                let p = &menu.params[i];
                let v = sim.rider.param(i);
                let mark = if menu.frozen[i] { " [frozen]" } else if v != p.default { " *" } else { "" };
                row(format!("{:<34} {:>9}{mark}", p.label, fmt_value(p, v)))
            })
            .collect(),
        Tab::Practice => {
            let s = &sim.current;
            let mut rows = vec![
                row(format!("Game speed {:>27.2}x", sim.time_scale)),
                row(format!("Pause {:>32}", if sim.paused { "on" } else { "off" })),
                row("Step one frame (while paused)"),
            ];
            for (i, m) in menu.markers.iter().enumerate() {
                rows.push(row(match m {
                    Some(m) => format!(
                        "Marker {}  ({:.1}, {:.1}, {:.1})   A save, X go",
                        i + 1,
                        m.position[0],
                        m.position[1],
                        m.position[2]
                    ),
                    None => format!("Marker {}  empty                   A save", i + 1),
                }));
            }
            rows.push(row(format!("Back to the last marker after a bail {:>3}", if menu.auto_return { "on" } else { "off" })));
            rows.push(row(format!("Never bail {:>27}", if menu.never_bail { "on" } else { "off" })));
            rows.push(row("Bail now"));
            rows.push(info_row(format!(
                "At ({:.1}, {:.1}, {:.1})  {:.1} m/s  {:?}  score {}",
                s.position[0], s.position[1], s.position[2], s.speed, s.mode, s.score
            )));
            rows
        }
        Tab::World => {
            if menu.xr {
                vec![
                    row(format!("Park size (pieces only) {:>12.2}x", menu.park_scale)),
                    row(format!("World scale (game m per real m) {:>6.1}", menu.world_scale)),
                    row("Put the skater in front of me"),
                ]
            } else {
                vec![info_row("World scale and park size apply in the headset.")]
            }
        }
        Tab::Presets => PRESETS.iter().map(|(name, _)| row(*name)).collect(),
    }
}

type Preset = fn(&mut Menu, &mut CleanSkate);

const PRESETS: [(&str, Preset); 8] = [
    ("Defaults (reset every value that isn't frozen)", |m, s| reset_all(m, s)),
    ("Mega pop", |m, s| scale_params(m, s, &[("pop_low", 1.9), ("pop_high", 1.9)])),
    ("Moon gravity", |m, s| set_params(m, s, &[("gravity", 4.5), ("spin_rate", 3.2)])),
    ("Spin cycle", |m, s| set_params(m, s, &[("spin_rate", 14.0), ("spin_ease", 30.0)])),
    ("Fast", |m, s| set_params(m, s, &[("push_top", 14.0), ("push_kick", 0.8), ("carve", 3.0)])),
    ("Never bail", |m, _| m.never_bail = true),
    ("Slow motion", |_, s| s.time_scale = 0.35),
    ("Real time", |_, s| s.time_scale = 1.0),
];

fn param_index(menu: &Menu, key: &str) -> Option<usize> {
    menu.params.iter().find(|p| p.key == key).map(|p| p.index)
}

fn reset_all(menu: &mut Menu, sim: &mut CleanSkate) {
    for p in &menu.params {
        if !menu.frozen[p.index] {
            sim.rider.set_param(p.index, p.default);
        }
    }
    menu.never_bail = false;
    sim.time_scale = 1.0;
}

fn set_params(menu: &mut Menu, sim: &mut CleanSkate, values: &[(&str, f32)]) {
    for &(key, v) in values {
        if let Some(i) = param_index(menu, key).filter(|&i| !menu.frozen[i]) {
            sim.rider.set_param(i, v);
        }
    }
}

fn scale_params(menu: &mut Menu, sim: &mut CleanSkate, values: &[(&str, f32)]) {
    for &(key, k) in values {
        if let Some(i) = param_index(menu, key).filter(|&i| !menu.frozen[i]) {
            sim.rider.set_param(i, menu.params[i].default * k);
        }
    }
}

fn page_rows(menu: &Menu, sim: &CleanSkate) -> Vec<Row> {
    match menu.page {
        Page::Start => {
            let mut rows: Vec<Row> = START
                .iter()
                .filter(|item| menu.xr || !matches!(**item, "Build a park" | "Put the skater in front of me"))
                .map(|s| row(*s))
                .collect();
            if menu.retail_available {
                rows.push(row("Use my imported disc from the next launch"));
            }
            rows.push(info_row(""));
            rows.push(info_row("Clean-room engine: every animation, value and sound"));
            rows.push(info_row("in this mode was made for this app."));
            rows
        }
        Page::HowTo(i) => howto_rows(i, menu.xr),
        Page::Sk8 => sk8_rows(menu, sim),
    }
}

// ---------------------------------------------------------------- actions

fn adjust(menu: &mut Menu, sim: &mut CleanSkate, dir: f32, nav: &Nav) {
    let gain = if nav.fine { 0.2 } else if nav.coarse { 5.0 } else { 1.0 };
    match &menu.tabs[menu.tab] {
        Tab::Params(_, list) => {
            let Some(&i) = list.get(menu.cursor) else { return };
            let p = &menu.params[i];
            let v = sim.rider.param(i) + dir * p.step * gain;
            let v = (v / (p.step * 0.2)).round() * p.step * 0.2;
            sim.rider.set_param(i, v);
            menu.dirty = true;
        }
        Tab::Practice if menu.cursor == 0 => {
            sim.time_scale = (sim.time_scale + dir * 0.05 * gain).clamp(0.05, 2.0);
        }
        Tab::World if menu.xr => match menu.cursor {
            0 => {
                let v = (menu.park_scale * (1.0 + 0.05 * dir * gain)).clamp(0.25, 4.0);
                menu.xr_requests.park_scale = Some(v);
            }
            1 => {
                let v = (menu.world_scale * (1.0 + 0.05 * dir * gain)).clamp(1.0, 60.0);
                menu.xr_requests.world_scale = Some(v);
            }
            _ => {}
        },
        _ => {}
    }
}

fn select(menu: &mut Menu, sim: &mut CleanSkate, alt: bool) {
    match menu.page {
        Page::Start => {
            if alt {
                return;
            }
            let rows = page_rows(menu, sim);
            let Some(choice) = rows.get(menu.cursor).map(|r| r.text.clone()) else { return };
            match choice.as_str() {
                "Skate" | "Close" => menu.open = false,
                "Build a park" => {
                    menu.xr_requests.build_mode = true;
                    menu.open = false;
                }
                "How to play" => {
                    menu.page = Page::HowTo(0);
                    menu.cursor = 0;
                }
                "SK8 menu" => {
                    menu.page = Page::Sk8;
                    menu.cursor = 0;
                }
                "Put the skater in front of me" => {
                    menu.xr_requests.recenter = true;
                    menu.open = false;
                }
                "Use my imported disc from the next launch" => {
                    match std::fs::write(menu.files.join("use-retail-data"), b"") {
                        Ok(()) => menu.say("Imported disc next launch. Quit and reopen the app."),
                        Err(e) => menu.say(format!("Could not switch: {e}")),
                    }
                }
                _ => {}
            }
        }
        Page::HowTo(_) => {}
        Page::Sk8 => match menu.tabs[menu.tab].clone() {
            Tab::Params(_, list) => {
                let Some(&i) = list.get(menu.cursor) else { return };
                if alt {
                    sim.rider.set_param(i, menu.params[i].default);
                    menu.say(format!("{} reset", menu.params[i].label));
                } else {
                    menu.frozen[i] = !menu.frozen[i];
                    let what = if menu.frozen[i] { "frozen" } else { "unfrozen" };
                    menu.say(format!("{} {what}", menu.params[i].label));
                }
                menu.dirty = true;
            }
            Tab::Practice => {
                match menu.cursor {
                    0 if alt => sim.time_scale = 1.0,
                    1 => sim.paused = !sim.paused,
                    2 => sim.step_once += 1,
                    c @ 3..=7 => {
                        let slot = c - 3;
                        if alt {
                            if let Some(m) = menu.markers[slot] {
                                sim.place(Vec3::from_array(m.position), m.facing);
                                menu.last_marker = Some(slot);
                                menu.say(format!("Back to marker {}", slot + 1));
                            }
                        } else if sim.current.mode.grounded() {
                            let s = &sim.current;
                            let facing = super::sim::facing_of(Quat::from_array(s.rider_rot) * Vec3::Z);
                            menu.markers[slot] = Some(Marker {
                                position: s.position,
                                facing,
                            });
                            menu.last_marker = Some(slot);
                            menu.say(format!("Marker {} saved", slot + 1));
                        } else {
                            menu.say("Markers save on the ground");
                        }
                    }
                    8 => menu.auto_return = !menu.auto_return,
                    9 => menu.never_bail = !menu.never_bail,
                    10 => {
                        sim.rider.bail();
                        menu.open = false;
                    }
                    _ => {}
                }
                menu.dirty = true;
            }
            Tab::World => {
                if menu.cursor == 2 {
                    menu.xr_requests.recenter = true;
                    menu.open = false;
                }
            }
            Tab::Presets => {
                if let Some((name, apply)) = PRESETS.get(menu.cursor) {
                    apply(menu, sim);
                    menu.dirty = true;
                    menu.say(format!("Preset: {name}"));
                }
            }
        },
    }
}

pub(crate) fn update(
    time: Res<Time<Real>>,
    nav: Res<MenuNav>,
    mut menu: ResMut<Menu>,
    mut sim: ResMut<CleanSkate>,
    mut capture: ResMut<UiCapture>,
    mut loaded: Local<bool>,
    mut save_timer: Local<f32>,
) {
    if !*loaded {
        *loaded = true;
        let sim = &mut *sim;
        menu.load(sim);
    }
    let nav = nav.0;
    let menu = &mut *menu;
    if nav.toggle_sk8 {
        if menu.open && menu.page == Page::Sk8 {
            menu.open = false;
        } else {
            menu.open = true;
            menu.page = Page::Sk8;
            menu.cursor = 0;
        }
    } else if nav.toggle_start {
        menu.open = !menu.open;
        menu.page = Page::Start;
        menu.cursor = 0;
    } else if menu.open {
        let count = page_rows(menu, &sim).len().max(1);
        if nav.up {
            menu.cursor = (menu.cursor + count - 1) % count;
        }
        if nav.down {
            menu.cursor = (menu.cursor + 1) % count;
        }
        match menu.page {
            Page::HowTo(i) => {
                let pages = HOWTO_TITLES.len();
                if nav.left || nav.tab_prev {
                    menu.page = Page::HowTo((i + pages - 1) % pages);
                    menu.cursor = 0;
                }
                if nav.right || nav.tab_next || nav.select {
                    menu.page = Page::HowTo((i + 1) % pages);
                    menu.cursor = 0;
                }
            }
            Page::Sk8 => {
                let n = menu.tabs.len();
                if nav.tab_prev {
                    menu.tab = (menu.tab + n - 1) % n;
                    menu.cursor = 0;
                }
                if nav.tab_next {
                    menu.tab = (menu.tab + 1) % n;
                    menu.cursor = 0;
                }
                if nav.left {
                    adjust(menu, &mut sim, -1.0, &nav);
                }
                if nav.right {
                    adjust(menu, &mut sim, 1.0, &nav);
                }
            }
            Page::Start => {}
        }
        if nav.select {
            select(menu, &mut sim, false);
        }
        if nav.alt {
            select(menu, &mut sim, true);
        }
        if nav.back {
            match menu.page {
                Page::Start => menu.open = false,
                _ => {
                    menu.page = Page::Start;
                    menu.cursor = 0;
                }
            }
        }
        let count = page_rows(menu, &sim).len().max(1);
        menu.cursor = menu.cursor.min(count - 1);
    }
    capture.0 = menu.open;

    // Practice flags.
    let mut f = 0;
    if menu.never_bail {
        f |= flags::NEVER_BAIL;
    }
    let marker = menu.last_marker.and_then(|i| menu.markers[i]);
    if menu.auto_return && marker.is_some() {
        f |= flags::NO_RESPAWN;
    }
    if sim.rider.flags() != f {
        sim.rider.set_flags(f);
    }
    if let (Some(m), Some(t)) = (marker.filter(|_| menu.auto_return), sim.current.bail_time) {
        if sim.current.mode == Mode::Bail && t >= 1.0 {
            sim.place(Vec3::from_array(m.position), m.facing);
        }
    }

    if menu.status.1 > 0.0 {
        menu.status.1 -= time.delta_secs();
    }
    if menu.dirty {
        *save_timer += time.delta_secs();
        if *save_timer > 1.0 || !menu.open {
            *save_timer = 0.0;
            menu.save(&sim);
        }
    }
}

// ---------------------------------------------------------------- view

#[derive(Component)]
pub(crate) struct MenuRoot;
#[derive(Component)]
pub(crate) struct TitleText;
#[derive(Component)]
pub(crate) struct TabsText;
#[derive(Component)]
pub(crate) struct RowNode(usize);
#[derive(Component)]
pub(crate) struct RowText(usize);
#[derive(Component)]
pub(crate) struct FooterText;
#[derive(Component)]
pub(crate) struct HudText;
/// The trick being done, big at the top middle, and under it the points the last one scored.
#[derive(Component)]
pub(crate) struct TrickName;
#[derive(Component)]
pub(crate) struct TrickPoints;
/// The HUD's face: a heavy italic sans (Montserrat, OFL - assets/fonts/OFL.txt).
const HUD_FONT: &str = "embedded://skate_game/clean/assets/fonts/Montserrat-Italic.ttf";

/// The UI camera the menu draws to, when not the window's (the XR panel).
#[derive(Resource, Clone, Copy)]
pub(crate) struct MenuTarget(pub Entity);

pub(crate) fn spawn_ui(mut commands: Commands, target: Option<Res<MenuTarget>>, assets: Res<AssetServer>) {
    let text = |s: &str, size: f32, color: Color| {
        (
            Text::new(s),
            TextFont {
                font_size: size,
                ..default()
            },
            TextColor(color),
        )
    };
    let mut root = commands.spawn((
        MenuRoot,
        Node {
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        GlobalZIndex(10),
    ));
    if let Some(t) = &target {
        root.insert(UiTargetCamera(t.0));
    }
    root.with_children(|root| {
        root.spawn((
            Node {
                width: px(900),
                max_width: percent(98),
                padding: UiRect::all(px(22)),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.05, 0.07, 0.92)),
        ))
        .with_children(|panel| {
            panel.spawn((TitleText, text("SK8", 44.0, Color::WHITE)));
            panel.spawn((TabsText, text("", 22.0, Color::srgb(0.45, 0.9, 0.85))));
            for i in 0..VISIBLE_ROWS {
                panel
                    .spawn((
                        RowNode(i),
                        Node {
                            width: percent(100),
                            min_height: px(38),
                            padding: UiRect::axes(px(10), px(4)),
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(7)),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ))
                    .with_children(|r| {
                        r.spawn((RowText(i), text("", 25.0, Color::WHITE)));
                    });
            }
            panel.spawn((FooterText, text("", 20.0, Color::srgb(0.7, 0.78, 0.82))));
        });
    });
    // Score and the trick in progress (desktop only; XR shows them on the panel).
    if target.is_none() {
        let font = assets.load(HUD_FONT);
        let heavy = |size: f32| TextFont { font: font.clone(), font_size: size, weight: FontWeight(800), ..default() };
        commands
            .spawn(Node {
                position_type: PositionType::Absolute,
                top: percent(9),
                width: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|c| {
                c.spawn((
                    TrickName,
                    Text::new(""),
                    heavy(46.0),
                    TextColor(Color::WHITE),
                    TextShadow { offset: Vec2::new(2.0, 3.0), color: Color::srgba(0.0, 0.0, 0.0, 0.45) },
                ));
                c.spawn((
                    TrickPoints,
                    Text::new(""),
                    heavy(30.0),
                    TextColor(Color::srgb(0.98, 0.82, 0.22)),
                    TextShadow { offset: Vec2::new(1.5, 2.0), color: Color::srgba(0.0, 0.0, 0.0, 0.45) },
                ));
            });
        commands.spawn((
            HudText,
            Text::new(""),
            heavy(24.0),
            TextColor(Color::WHITE),
            TextShadow { offset: Vec2::new(1.5, 2.0), color: Color::srgba(0.0, 0.0, 0.0, 0.4) },
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                top: px(12),
                ..default()
            },
        ));
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn draw(
    menu: Res<Menu>,
    sim: Res<CleanSkate>,
    mut root: Query<&mut Node, With<MenuRoot>>,
    mut title: Query<&mut Text, (With<TitleText>, Without<TabsText>, Without<RowText>, Without<FooterText>, Without<HudText>)>,
    mut tabs: Query<&mut Text, (With<TabsText>, Without<TitleText>, Without<RowText>, Without<FooterText>, Without<HudText>)>,
    mut rows: Query<(&RowText, &mut Text, &mut TextColor), (Without<TitleText>, Without<TabsText>, Without<FooterText>, Without<HudText>)>,
    mut row_nodes: Query<(&RowNode, &mut BackgroundColor, &mut Node), Without<MenuRoot>>,
    mut footer: Query<&mut Text, (With<FooterText>, Without<TitleText>, Without<TabsText>, Without<RowText>, Without<HudText>)>,
    mut hud: Query<&mut Text, (With<HudText>, Without<TitleText>, Without<TabsText>, Without<RowText>, Without<FooterText>)>,
    time: Res<Time<Real>>,
    mut news: Local<(String, f32)>,
    mut trick_name: Query<&mut Text, (With<TrickName>, Without<HudText>, Without<TitleText>, Without<TabsText>, Without<RowText>, Without<FooterText>, Without<TrickPoints>)>,
    mut trick_points: Query<&mut Text, (With<TrickPoints>, Without<HudText>, Without<TitleText>, Without<TabsText>, Without<RowText>, Without<FooterText>, Without<TrickName>)>,
    mut shown: Local<(String, u32, f32)>,
) {
    // the trick being done, big; once it scores, its points under it a moment
    for e in sim.events.iter().filter(|e| e.kind == EventKind::Trick) {
        *shown = (e.name.clone(), e.points, 1.6);
    }
    shown.2 -= time.delta_secs();
    let doing = sim.rider.trick();
    let (name, points) = if !doing.is_empty() {
        (doing.to_string(), String::new())
    } else if shown.2 > 0.0 {
        (shown.0.clone(), shown.1.to_string())
    } else {
        (String::new(), String::new())
    };
    if let Ok(mut t) = trick_name.single_mut() {
        **t = name.to_uppercase();
    }
    if let Ok(mut t) = trick_points.single_mut() {
        **t = points;
    }
    // a personal best beaten: said on the HUD for a few seconds
    for e in sim.events.iter().filter(|e| e.kind == EventKind::Best) {
        news.0 = format!("New best {}: {}", e.name.to_lowercase(), best_text(&e.name, e.points));
        news.1 = 3.0;
    }
    news.1 -= time.delta_secs();
    if let Ok(mut hud) = hud.single_mut() {
        let s = &sim.current;
        let combo = if s.combo_count > 0 { format!("   combo {} x{}", s.combo_points, s.combo_count) } else { String::new() };
        let best = if news.1 > 0.0 { format!("\n{}", news.0) } else { String::new() };
        **hud = format!("{}{combo}{best}", s.score);
    }
    let Ok(mut node) = root.single_mut() else { return };
    let display = if menu.open { Display::Flex } else { Display::None };
    if node.display != display {
        node.display = display;
    }
    if !menu.open {
        return;
    }
    let (title_text, tabs_text) = match menu.page {
        Page::Start => ("SK8 AR".to_string(), format!("Score {}", sim.current.score)),
        Page::HowTo(i) => (
            format!("How to play: {}", HOWTO_TITLES[i]),
            HOWTO_TITLES
                .iter()
                .enumerate()
                .map(|(j, t)| if j == i { format!("[{t}]") } else { t.to_string() })
                .collect::<Vec<_>>()
                .join("   "),
        ),
        Page::Sk8 => (
            "SK8 menu".to_string(),
            menu.tabs
                .iter()
                .enumerate()
                .map(|(j, t)| if j == menu.tab { format!("[{}]", t.title()) } else { t.title().to_string() })
                .collect::<Vec<_>>()
                .join("  "),
        ),
    };
    if let Ok(mut t) = title.single_mut() {
        **t = title_text;
    }
    if let Ok(mut t) = tabs.single_mut() {
        **t = tabs_text;
    }
    let list = page_rows(&menu, &sim);
    let first = menu.cursor.saturating_sub(VISIBLE_ROWS - 1);
    let interactive = !matches!(menu.page, Page::HowTo(_));
    for (RowText(i), mut text, mut color) in &mut rows {
        let r = list.get(first + i);
        **text = r.map(|r| r.text.clone()).unwrap_or_default();
        color.0 = if r.is_some_and(|r| r.dim) { Color::srgb(0.75, 0.8, 0.82) } else { Color::WHITE };
    }
    for (RowNode(i), mut bg, mut node) in &mut row_nodes {
        let selected = interactive && first + i == menu.cursor && list.get(first + i).is_some_and(|r| !r.dim);
        bg.0 = if selected { Color::srgb(0.12, 0.42, 0.4) } else { Color::NONE };
        let display = if first + i < list.len() { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
    if let Ok(mut t) = footer.single_mut() {
        let hint = match (menu.page, menu.xr) {
            (Page::Start, true) => "Stick: move   A: select   B: close",
            (Page::Start, false) => "Arrows: move   Enter: select   Esc: close",
            (Page::HowTo(_), true) => "Stick left/right or A: next page   B: back",
            (Page::HowTo(_), false) => "Left/Right: page   Backspace: back",
            (Page::Sk8, true) => "Grips: tab   Stick: select/adjust   A: freeze   X: reset   B: back",
            (Page::Sk8, false) => "Q/E: tab   Arrows: select/adjust   F: freeze   R: reset   Backspace: back",
        };
        **t = if menu.status.1 > 0.0 { format!("{}\n{hint}", menu.status.0) } else { hint.to_string() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trick_paths_read_as_directions() {
        let ollie = skate_lite_sys::trick_demo_path(0);
        assert_eq!(describe_path(&ollie), "down, centre, up");
        let names = skate_lite_sys::trick_names();
        let shove = names.iter().position(|n| *n == "Pop Shove-it").unwrap();
        assert_eq!(describe_path(&skate_lite_sys::trick_demo_path(shove)), "down, down-right, right, let go");
    }

    #[test]
    fn repeat_fires_on_press_then_after_the_delay() {
        let mut r = Repeat::default();
        assert!(r.press(0, true, 0.016));
        let mut fired = 0;
        for _ in 0..60 {
            fired += r.press(0, true, 0.016) as u32;
        }
        assert!(fired >= 4, "{fired}");
        assert!(!r.press(0, false, 0.016));
    }
}
