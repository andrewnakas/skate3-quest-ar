//! The headset menu's shared pages, the same in both tiers: the start page,
//! the first-run intro, AR settings, building a park, controls and the
//! Skate 3 data page. A tier adds its own (the clean tier's skater page).
use super::import::{Import, Status};
use super::{Act, ControlsSheet, Outcome, PageId, UiPage, UiProvider, UiProviders, Widget, XrUi};
use crate::xr::actions::Hand;
use crate::xr::ar_settings::{ArMode, ArSettings};
use crate::xr::build::{BuildMode, MAX_PARK_SCALE, MAX_PIECE_SCALE, MAX_SCALE, MIN_PARK_SCALE, MIN_PIECE_SCALE, MIN_SCALE, PlacedPieces};
use crate::xr::link::{ArWorld, SkaterLink, TeleportSkater};
use crate::xr::planes::{RoomMesh, SceneCapture, SceneShapes};
use crate::xr::rig::Rig;
use crate::xr::spawn::ray_triangle;
use bevy::prelude::*;
use bevy_mod_xr::camera::XrCamera;

pub(crate) const START: PageId = "start";
pub(crate) const INTRO: PageId = "intro";
const INTRO_MODE: PageId = "intro_mode";
const INTRO_FLOOR: PageId = "intro_floor";
const INTRO_CONTROLS: PageId = "intro_controls";
pub(crate) const AR: PageId = "ar";
pub(crate) const BUILD: PageId = "build";
pub(crate) const CONTROLS: PageId = "controls";
const BUILD_CONTROLS: PageId = "build_controls";
pub(crate) const DATA: PageId = "data";

/// Where "put the skater in front of me" aims, real metres ahead of the head.
const AHEAD_M: f32 = 0.6;
/// Import buttons, one per bundle found.
const IMPORT_IDS: [&str; 4] = ["import0", "import1", "import2", "import3"];

/// Which tier is running.
#[derive(Resource, Clone, Copy)]
pub(crate) struct UiTier {
    pub retail: bool,
}

pub(crate) struct Shared;

impl UiProvider for Shared {
    fn page(&self, id: PageId, world: &World) -> Option<UiPage> {
        let settings = world.resource::<ArSettings>();
        let retail = world.resource::<UiTier>().retail;
        Some(match id {
            START => {
                let mut w = vec![Widget::button("skate", "Skate")];
                for provider in world.resource::<UiProviders>().0.iter() {
                    w.extend(provider.start_items(world));
                }
                w.extend([
                    Widget::link("AR settings", AR),
                    Widget::link("Build a park", BUILD),
                    Widget::button("recenter", "Put the skater in front of me"),
                    Widget::link("Controls", CONTROLS),
                    Widget::link("Skate 3 data", DATA),
                    Widget::info(""),
                    Widget::info(tier_line(retail)),
                    Widget::info(mode_line(settings)),
                ]);
                UiPage::new("SK8 AR", w)
            }
            INTRO => UiPage::new(
                "Welcome to SK8 AR",
                vec![
                    Widget::info("Skate a little skater around your real room: floors, tables,"),
                    Widget::info("couches, and ramps and rails you build yourself."),
                    Widget::info(tier_line(retail)),
                    Widget::info(""),
                    Widget::info("Point with the right controller and pull the trigger,"),
                    Widget::info("or move with a stick and press A. B goes back."),
                    Widget::info("Tap the left menu button any time for this menu."),
                    Widget::info(""),
                    Widget::button("next", "Start"),
                    Widget::button("skip", "Skip the intro"),
                ],
            ),
            INTRO_MODE => UiPage::new(
                "Where are you skating?",
                vec![
                    Widget::button("room", "In this room (use my room scan)"),
                    Widget::button("room_scan", "In this room: scan it now"),
                    Widget::button("free", "Outside / free roam (no scan, an endless floor)"),
                    Widget::info(""),
                    Widget::info("A room scan lets the skater ride your floor, walls, tables"),
                    Widget::info("and furniture. Free roam is one flat floor at your feet."),
                    Widget::info("Change it any time in AR settings."),
                ],
            ),
            INTRO_FLOOR => {
                let mut w = Vec::new();
                if settings.free_roam() {
                    w.push(Widget::info("Free roam: one flat floor at your feet, as far as you go."));
                    w.extend(floor_widgets(settings));
                } else {
                    w.push(Widget::info("The skater rides your floor, tables and furniture."));
                    if world.resource::<SceneCapture>().busy() {
                        w.push(Widget::info("Room scan running: finish it, then come back here."));
                    }
                }
                w.extend([
                    Widget::button("recenter", "Put the skater in front of me"),
                    Widget::link("Skater size", BUILD),
                    Widget::button("next", "Next"),
                ]);
                UiPage::new("Put the skater down", w)
            }
            INTRO_CONTROLS => {
                let mut w: Vec<Widget> = controls_rows(world).into_iter().take(9).collect();
                w.push(Widget::link("All the controls", CONTROLS));
                w.push(Widget::button("done", "Skate!"));
                UiPage::new("Controls", w)
            }
            AR => UiPage::new("AR settings", ar_widgets(world)),
            BUILD => UiPage::new("Build a park", build_widgets(world)),
            CONTROLS => {
                let mut w = controls_rows(world);
                w.push(Widget::link("Build mode controls", BUILD_CONTROLS));
                UiPage::new("Controls", w)
            }
            BUILD_CONTROLS => UiPage::new("Build mode controls", BUILD_HELP.iter().map(|(a, b)| pair(a, b)).collect()),
            DATA => UiPage::new("Skate 3 data", data_widgets(world)),
            _ => return None,
        })
    }

    fn act(&self, page: PageId, widget: &'static str, act: Act, world: &mut World) -> Option<Outcome> {
        let page_widget = (page, widget);
        Some(match page_widget {
            (_, "skate") => Outcome::Close,
            (INTRO_FLOOR, "recenter") => {
                put_skater_ahead(world);
                Outcome::Say("The skater is in front of you".into())
            }
            (_, "recenter") => {
                put_skater_ahead(world);
                Outcome::Close
            }
            (INTRO, "next") => Outcome::Replace(INTRO_MODE),
            (INTRO, "skip") | (INTRO_CONTROLS, "done") => {
                edit(world, |s| s.onboarding_done = true);
                Outcome::Close
            }
            (INTRO_MODE, "room" | "room_scan" | "free") => {
                let mode = if widget == "free" { ArMode::FreeRoam } else { ArMode::Room };
                edit(world, |s| s.mode = mode);
                if widget == "room_scan" {
                    world.resource_mut::<SceneCapture>().requested = true;
                }
                Outcome::Replace(INTRO_FLOOR)
            }
            (INTRO_FLOOR, "next") => Outcome::Replace(INTRO_CONTROLS),
            (_, "floor_offset") => {
                if let Act::Set(v) = act {
                    edit(world, |s| s.floor_offset = v);
                }
                Outcome::Stay
            }
            (_, "floor_here") => match right_controller(world) {
                Some(c) => {
                    let y = (c.translation.y * 100.0).round() / 100.0;
                    edit(world, |s| s.floor_offset = y.clamp(-1.0, 1.0));
                    Outcome::Say(format!("Floor set {y:+.2} m"))
                }
                None => Outcome::Say("No right controller".into()),
            },
            (AR, _) => ar_act(widget, world),
            (BUILD | INTRO_FLOOR, _) => build_act(widget, act, world)?,
            (DATA, _) => data_act(widget, world),
            _ => return None,
        })
    }
}

fn pair(a: &str, b: &str) -> Widget {
    Widget::info(format!("{a:<26} {b}"))
}

fn controls_rows(world: &World) -> Vec<Widget> {
    world.resource::<ControlsSheet>().0.iter().map(|(a, b)| pair(a, b)).collect()
}

fn tier_line(retail: bool) -> &'static str {
    if retail {
        "Playing your own Skate 3 data (retail controls and skaters)."
    } else {
        "Clean-room game: no Skate 3 disc needed. Made for this app."
    }
}

fn mode_line(settings: &ArSettings) -> &'static str {
    match settings.mode {
        ArMode::Room => "AR: your room scan.",
        ArMode::FreeRoam => "AR: free roam (one endless floor).",
    }
}

/// Change the AR settings (saved, and collision / overlays follow).
pub(crate) fn edit(world: &mut World, f: impl FnOnce(&mut ArSettings)) {
    let mut settings = world.resource_mut::<ArSettings>();
    f(&mut settings);
    settings.revision += 1;
}

fn right_controller(world: &mut World) -> Option<Transform> {
    let mut hands = world.query::<(&Hand, &Transform)>();
    hands.iter(world).find(|(h, _)| matches!(h, Hand::Right)).map(|(_, t)| *t)
}

fn floor_widgets(settings: &ArSettings) -> [Widget; 2] {
    [
        Widget::slider(
            "floor_offset",
            "Floor height",
            settings.floor_offset,
            (-1.0, 1.0, 0.01),
            format!("{:+.2} m", settings.floor_offset),
        ),
        Widget::button("floor_here", "Set it from the right controller (hold it on the ground, press A)"),
    ]
}

// ---------------------------------------------------------------- AR

fn ar_widgets(world: &World) -> Vec<Widget> {
    let s = world.resource::<ArSettings>();
    let mut w = vec![
        Widget::info(mode_line(s)),
        Widget::toggle("free_roam", "Free roam (outside: no room scan, endless floor)", s.free_roam()),
    ];
    if s.free_roam() {
        w.extend(floor_widgets(s));
    } else {
        let busy = world.resource::<SceneCapture>().busy();
        w.push(Widget::button("scan", if busy { "Room scan running..." } else { "Set up a room scan (Space Setup)" }));
    }
    w.extend([
        Widget::toggle("floor_collide", "Floor: skate on it", s.floor.collide),
        Widget::toggle("floor_show", "Floor: show it", s.floor.show),
        Widget::toggle("planes_collide", "Walls, tables, furniture: skate on them", s.planes.collide),
        Widget::toggle("planes_show", "Walls, tables, furniture: show them", s.planes.show),
        Widget::toggle("mesh_collide", "3D room mesh: skate on it", s.mesh.collide),
        Widget::toggle("mesh_show", "3D room mesh: show it", s.mesh.show),
        Widget::button("show_all", "Show everything"),
        Widget::button("hide_all", "Hide everything (still skated)"),
    ]);
    if !s.floor.collide {
        w.push(Widget::info("Floor off: the skater falls through it where nothing else is."));
    }
    let shapes = world.resource::<SceneShapes>().0.len();
    let triangles = world.resource::<RoomMesh>().0.as_ref().map_or(0, |m| m.indices.len() / 3);
    w.extend([
        Widget::toggle("hud", "Trick and points readout", s.hud),
        Widget::button("intro", "Replay the intro"),
        Widget::info(format!("Room: {shapes} surfaces, {triangles} mesh triangles")),
    ]);
    w
}

fn ar_act(widget: &str, world: &mut World) -> Outcome {
    match widget {
        "scan" => {
            world.resource_mut::<SceneCapture>().requested = true;
            return Outcome::Say("Starting Space Setup".into());
        }
        "intro" => {
            edit(world, |s| s.onboarding_done = false);
            world.resource_mut::<XrUi>().request = Some(INTRO);
            return Outcome::Stay;
        }
        _ => {}
    }
    edit(world, |s| match widget {
        "free_roam" => s.mode = if s.free_roam() { ArMode::Room } else { ArMode::FreeRoam },
        "floor_collide" => s.floor.collide = !s.floor.collide,
        "floor_show" => s.floor.show = !s.floor.show,
        "planes_collide" => s.planes.collide = !s.planes.collide,
        "planes_show" => s.planes.show = !s.planes.show,
        "mesh_collide" => s.mesh.collide = !s.mesh.collide,
        "mesh_show" => s.mesh.show = !s.mesh.show,
        "show_all" => s.set_all_shown(true),
        "hide_all" => s.set_all_shown(false),
        "hud" => s.hud = !s.hud,
        _ => {}
    });
    Outcome::Stay
}

// ---------------------------------------------------------------- build

/// The skater's size against the real room as a 0..1 slider: 1 is life
/// size, 0 the smallest (the world scale is game metres per real metre).
pub(crate) fn skater_size_widget(world: &World) -> Widget {
    let scale = world.resource::<Rig>().scale;
    let v = 1.0 - scale.ln() / MAX_SCALE.ln();
    let text = if scale <= 1.01 { "life size".to_string() } else { format!("1/{scale:.0} life size") };
    Widget::slider("skater_size", "Skater size", v, (0.0, 1.0, 0.01), text)
}

/// Set the skater's size from its slider. Only the skater changes: the world
/// scale moves and the park scale moves with it, so every placed piece keeps
/// its real size (and its own scale). The skater stays on the same real spot.
pub(crate) fn set_skater_size(world: &mut World, v: f32) {
    let old = world.resource::<Rig>().scale;
    let scale = MAX_SCALE.powf(1.0 - v).clamp(MIN_SCALE, MAX_SCALE);
    {
        let mut placed = world.resource_mut::<PlacedPieces>();
        placed.park_scale = (placed.park_scale * scale / old).clamp(MIN_PARK_SCALE, MAX_PARK_SCALE);
    }
    set_world_scale(world, scale);
}

/// The park-size slider is logarithmic over the wide park range.
fn park_slider(world: &World) -> Widget {
    let park = world.resource::<PlacedPieces>().park_scale;
    let span = (MAX_PARK_SCALE / MIN_PARK_SCALE).ln();
    let v = (park / MIN_PARK_SCALE).ln() / span;
    Widget::slider("park_scale", "Park size (every piece)", v, (0.0, 1.0, 0.005), format!("{park:.2}x"))
}

fn park_from_slider(v: f32) -> f32 {
    (MIN_PARK_SCALE * ((MAX_PARK_SCALE / MIN_PARK_SCALE).ln() * v).exp()).clamp(MIN_PARK_SCALE, MAX_PARK_SCALE)
}

pub(crate) fn set_world_scale(world: &mut World, scale: f32) {
    let before = world.resource::<Rig>().root_transform();
    let deck = world.resource::<SkaterLink>().deck;
    world.resource_mut::<Rig>().scale = scale;
    world.resource_mut::<PlacedPieces>().revision += 1; // saves the layout (it carries the scale)
    let after = world.resource::<Rig>().root_transform();
    if let (Some(before), Some(after), Some((deck, forward))) = (before, after, deck) {
        let inverse = before.compute_affine().inverse();
        let (spot, dir) = (inverse.transform_point3(deck), inverse.transform_vector3(forward));
        world.write_message(TeleportSkater {
            point: after.transform_point(spot),
            forward: after.rotation * dir,
        });
    }
}

fn build_widgets(world: &World) -> Vec<Widget> {
    let placed = world.resource::<PlacedPieces>();
    let mut w = vec![
        Widget::button("build", "Start building (hold the menu button 1 s to stop)"),
        skater_size_widget(world),
        park_slider(world),
    ];
    match placed.selected.and_then(|id| placed.items.iter().find(|p| p.id == id)) {
        Some(item) => w.extend([
            Widget::info(format!("Selected: {} (the last piece pointed at)", item.name())),
            Widget::slider(
                "piece_scale",
                "This piece's size",
                item.scale,
                (MIN_PIECE_SCALE, MAX_PIECE_SCALE, 0.05),
                format!("{:.2}x", item.scale),
            ),
            Widget::toggle("piece_hidden", "This piece invisible (still skated)", item.hidden),
            Widget::button("piece_delete", "Delete this piece"),
        ]),
        None => w.push(Widget::info("Point at a piece in build mode to edit it here.")),
    }
    w.extend([
        Widget::toggle("show_hidden", "Show invisible pieces while skating", placed.show_hidden),
        Widget::button("undo", "Undo"),
        Widget::button("clear", "Clear every piece"),
        Widget::info(format!("{} pieces", placed.items.len())),
        Widget::link("Build mode controls", BUILD_CONTROLS),
    ]);
    w
}

fn build_act(widget: &str, act: Act, world: &mut World) -> Option<Outcome> {
    let value = match act {
        Act::Set(v) => Some(v),
        Act::Press => None,
    };
    Some(match widget {
        "build" => {
            world.resource_mut::<BuildMode>().active = true;
            Outcome::Close
        }
        "skater_size" => {
            set_skater_size(world, value?);
            Outcome::Stay
        }
        "park_scale" => {
            let mut p = world.resource_mut::<PlacedPieces>();
            p.park_scale = park_from_slider(value?);
            p.revision += 1;
            Outcome::Stay
        }
        "piece_scale" | "piece_hidden" | "piece_delete" => {
            let mut p = world.resource_mut::<PlacedPieces>();
            let id = p.selected?;
            if widget == "piece_delete" {
                p.remove(id);
                p.selected = None;
                return Some(Outcome::Say("Piece deleted".into()));
            }
            p.checkpoint();
            let item = p.get_mut(id)?;
            if widget == "piece_scale" {
                item.scale = crate::xr::build::snap_scale(value?);
            } else {
                item.hidden = !item.hidden;
            }
            p.revision += 1;
            Outcome::Stay
        }
        "show_hidden" => {
            let mut p = world.resource_mut::<PlacedPieces>();
            p.show_hidden = !p.show_hidden;
            Outcome::Stay
        }
        "undo" => {
            let undone = world.resource_mut::<PlacedPieces>().undo();
            Outcome::Say(if undone { "Undone".into() } else { "Nothing to undo".into() })
        }
        "clear" => {
            world.resource_mut::<PlacedPieces>().clear();
            Outcome::Say("Cleared (Undo brings them back)".into())
        }
        _ => return None,
    })
}

const BUILD_HELP: [(&str, &str); 17] = [
    ("Left menu (hold 1 s)", "enter / leave build mode"),
    ("Left stick left/right", "choose a piece"),
    ("Right controller", "point; trigger drops the piece"),
    ("Right stick left/right", "turn (the piece pointed at)"),
    ("Right stick up/down", "resize its length / height"),
    ("Left grip + right stick up/down", "scale the whole piece"),
    ("A on a piece", "make it invisible (still skated)"),
    ("A on nothing", "show / hide the detected planes"),
    ("Right grip on a piece", "pick it up"),
    ("B on a piece", "delete it"),
    ("Right stick click", "copy the piece pointed at"),
    ("Left trigger", "undo"),
    ("Left stick click", "drop a starter park"),
    ("Left stick up/down", "grow / shrink everything"),
    ("Left grip + left stick", "grow / shrink only the park"),
    ("Y", "rescan the room"),
    ("Hold X", "clear every piece"),
];

// ---------------------------------------------------------------- data

fn data_widgets(world: &World) -> Vec<Widget> {
    let retail = world.resource::<UiTier>().retail;
    let import = world.resource::<Import>();
    let installed = import.installed();
    let next = import.retail_next();
    let mut w = vec![
        Widget::info(tier_line(retail)),
        Widget::info(match (installed, next) {
            (true, true) => "Next launch: your Skate 3 data.",
            (true, false) => "Next launch: the clean-room game (your Skate 3 data is kept).",
            (false, _) => "No Skate 3 data imported.",
        }),
    ];
    if installed && !next {
        w.push(Widget::button("use_retail", "Play my Skate 3 from the next launch"));
    }
    if next || retail {
        w.push(Widget::button("use_clean", "Play the clean-room game from the next launch"));
    }
    match import.status() {
        Status::Working { what, done, total } => {
            let pct = if total > 0 { done as f32 / total as f32 * 100.0 } else { 0.0 };
            w.push(Widget::info(format!("Importing: {pct:.0}%  {what}")));
        }
        Status::Done(Ok(msg)) => {
            w.push(Widget::info(msg));
            w.push(Widget::button("restart", "Close the app now (then open it again)"));
        }
        Status::Done(Err(e)) => w.push(Widget::info(format!("Import failed: {e}"))),
        Status::Scanned(found) if found.is_empty() => {
            w.push(Widget::info("Nothing found in Download/Skate3."));
            w.push(Widget::button("scan", "Look again"));
        }
        Status::Scanned(found) => {
            for (id, source) in IMPORT_IDS.iter().zip(found.iter()) {
                w.push(Widget::button(id, format!("Import {}", source.describe())));
            }
            w.push(Widget::button("scan", "Look again"));
        }
        Status::Idle => w.push(Widget::button("scan", "Import Skate 3 data from Download/Skate3")),
    }
    if retail || next {
        w.push(Widget::button("restart", "Close the app now (then open it again)"));
    }
    w.extend([
        Widget::info(""),
        Widget::info("The headset can't read a disc. On a PC, run the Skate 3 setup"),
        Widget::info("on your own ISO, then scripts/quest/make-bundle.ps1. Copy the"),
        Widget::info("quest-data folder (or a zip of it) to Download/Skate3 on the"),
        Widget::info("headset, or push it with scripts/quest/push-assets.sh."),
    ]);
    w.dedup();
    w
}

fn data_act(widget: &str, world: &mut World) -> Outcome {
    let import = world.resource::<Import>().clone();
    match widget {
        "use_retail" | "use_clean" => match import.set_retail_next(widget == "use_retail") {
            Ok(()) => Outcome::Say("Saved. Close and reopen the app to switch.".into()),
            Err(e) => Outcome::Say(format!("Could not switch: {e}")),
        },
        "scan" => {
            if !crate::storage_access() {
                crate::request_storage_access();
                return Outcome::Say("Allow \"All files access\" for SK8 AR, then press it again.".into());
            }
            import.scan();
            Outcome::Stay
        }
        "restart" => {
            info!("UI closing the app to switch tiers");
            std::process::exit(0);
        }
        id => {
            let Some(i) = IMPORT_IDS.iter().position(|&x| x == id) else { return Outcome::Stay };
            if let Status::Scanned(found) = import.status() {
                if let Some(source) = found.get(i) {
                    import.start(source.clone());
                }
            }
            Outcome::Stay
        }
    }
}

// ---------------------------------------------------------------- skater

/// Put the skater on the ground just ahead of the player (the nearest
/// ground-like face below that spot, else the floor height).
pub(crate) fn put_skater_ahead(world: &mut World) {
    let Some(root) = world.resource::<Rig>().root_transform() else { return };
    let mut eyes = world.query_filtered::<&Transform, With<XrCamera>>();
    let Some(eye) = eyes.iter(world).next().copied() else { return };
    let forward = (eye.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
    let ahead_tracking = eye.translation + forward * AHEAD_M;
    let ahead = root.transform_point(ahead_tracking);
    let from = ahead + Vec3::Y * root.scale.y * 0.5;
    let ground = world
        .resource::<ArWorld>()
        .all_triangles()
        .filter(|[a, b, c]| (*b - *a).cross(*c - *a).normalize_or_zero().y.abs() > 0.6)
        .filter_map(|[a, b, c]| ray_triangle(from, Vec3::NEG_Y, a, b, c))
        .min_by(f32::total_cmp)
        .map(|t| from - Vec3::Y * t);
    let floor_y = world.resource::<ArSettings>().floor_offset;
    let point = ground.unwrap_or_else(|| root.transform_point(ahead_tracking.with_y(floor_y)));
    world.write_message(TeleportSkater {
        point,
        forward: root.rotation * forward,
    });
}
