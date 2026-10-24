//! The clean tier's pages in the headset menu: "Skater" (size, pop height,
//! flip and spin speed, grind lock-on, no bail, the controls) and the way
//! into the full SK8 sheet.
//!
//! Each slider is a multiple of the engine's own default, read back from the
//! live values (so the SK8 sheet and this page always agree, and the values
//! save with the SK8 sheet's `sk8-tuning.json`):
//!
//! - pop height: `pop_low` and `pop_high` (take-off speeds) times the square
//!   root of it, since height goes with speed squared. The nollie, grind and
//!   boneless pops are shares of these and follow.
//! - flip speed: `flip_base` and `flip_per_rev` (seconds) divided by it.
//! - spin speed: `spin_rate` times it.
//! - grind lock-on: `grind_assist` (the pull onto a rail) and `grind_catch`
//!   (how far a rail is caught from) times it.
use super::menu::Menu;
use super::sim::CleanSkate;
use crate::xr::ui::pages::{CONTROLS, set_skater_size, skater_size_widget};
use crate::xr::ui::{Act, Outcome, PageId, UiPage, UiProvider, Widget};
use bevy::prelude::*;

pub(crate) const SKATER: PageId = "skater";
/// The SK8 sheet's name when it borrows the panel.
pub(crate) const SK8_SHEET: &str = "sk8";

pub(crate) struct CleanPages;

/// Engine value `key`'s index, current value and default.
fn param(sim: &CleanSkate, key: &str) -> Option<(usize, f32, f32)> {
    skate_lite_sys::params()
        .into_iter()
        .find(|p| p.key == key)
        .map(|p| (p.index, sim.rider.param(p.index), p.default))
}

/// The current multiple of the default for a slider.
fn multiple(sim: &CleanSkate, slider: &str) -> f32 {
    let ratio = |key: &str| param(sim, key).map_or(1.0, |(_, v, d)| if d != 0.0 { v / d } else { 1.0 });
    match slider {
        "pop" => ratio("pop_high").powi(2),
        "flip" => 1.0 / ratio("flip_per_rev").max(1e-3),
        "spin" => ratio("spin_rate"),
        "grind" => ratio("grind_assist"),
        _ => 1.0,
    }
}

/// Set a slider's multiple on the engine values behind it.
fn set_multiple(sim: &mut CleanSkate, slider: &str, k: f32) {
    let scaled: &[(&str, f32)] = match slider {
        "pop" => &[("pop_low", k.sqrt()), ("pop_high", k.sqrt())],
        "flip" => &[("flip_base", 1.0 / k.max(1e-3)), ("flip_per_rev", 1.0 / k.max(1e-3))],
        "spin" => &[("spin_rate", k)],
        "grind" => &[("grind_assist", k), ("grind_catch", k)],
        _ => &[],
    };
    for &(key, f) in scaled {
        if let Some((i, _, d)) = param(sim, key) {
            sim.rider.set_param(i, d * f);
        }
    }
}

const SLIDERS: [(&str, &str, (f32, f32, f32)); 4] = [
    ("pop", "Pop height", (0.5, 4.0, 0.05)),
    ("flip", "Flip speed", (0.25, 4.0, 0.05)),
    ("spin", "Spin speed", (0.0, 3.0, 0.05)),
    ("grind", "Grind lock-on", (0.0, 2.5, 0.05)),
];

impl UiProvider for CleanPages {
    fn page(&self, id: PageId, world: &World) -> Option<UiPage> {
        if id != SKATER {
            return None;
        }
        let sim = world.resource::<CleanSkate>();
        let menu = world.resource::<Menu>();
        let mut w = vec![skater_size_widget(world)];
        for (id, label, range) in SLIDERS {
            let k = multiple(sim, id);
            w.push(Widget::slider(id, label, k, range, format!("{k:.2}x")));
        }
        w.extend([
            Widget::toggle("no_bail", "No bail", menu.never_bail()),
            Widget::link("Controls", CONTROLS),
            Widget::button("reset", "Back to normal (1x everything, bails on)"),
            Widget::info(""),
            Widget::info("Every engine value, practice tools and presets:"),
            Widget::button("sk8", "Advanced tuning (SK8 menu)"),
        ]);
        Some(UiPage::new("Skater", w))
    }

    fn act(&self, page: PageId, widget: &'static str, act: Act, world: &mut World) -> Option<Outcome> {
        if widget == "sk8" {
            return Some(Outcome::External(SK8_SHEET));
        }
        if page != SKATER {
            return None;
        }
        match (widget, act) {
            ("skater_size", Act::Set(v)) => set_skater_size(world, v),
            ("no_bail", _) => {
                let mut menu = world.resource_mut::<Menu>();
                let on = !menu.never_bail();
                menu.set_never_bail(on);
            }
            ("reset", _) => {
                let mut sim = world.resource_mut::<CleanSkate>();
                for (id, ..) in SLIDERS {
                    set_multiple(&mut sim, id, 1.0);
                }
                let mut menu = world.resource_mut::<Menu>();
                menu.set_never_bail(false);
                return Some(Outcome::Say("Skater back to normal".into()));
            }
            (id, Act::Set(k)) if SLIDERS.iter().any(|s| s.0 == id) => {
                set_multiple(&mut world.resource_mut::<CleanSkate>(), id, k);
                world.resource_mut::<Menu>().mark_dirty();
            }
            _ => {}
        }
        Some(Outcome::Stay)
    }

    fn start_items(&self, _world: &World) -> Vec<Widget> {
        vec![Widget::link("Skater", SKATER), Widget::button("sk8", "Advanced tuning (SK8 menu)")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiples_round_trip_through_the_engine() {
        let mut sim = CleanSkate::new();
        for (id, _, (min, max, _)) in SLIDERS {
            assert!((multiple(&sim, id) - 1.0).abs() < 1e-4, "{id} starts at 1x");
            for k in [min.max(0.25), 1.5, max] {
                set_multiple(&mut sim, id, k);
                assert!((multiple(&sim, id) - k).abs() < 0.02, "{id} {k} read back {}", multiple(&sim, id));
            }
            set_multiple(&mut sim, id, 1.0);
        }
        // Pop height 4x doubles the take-off speed.
        set_multiple(&mut sim, "pop", 4.0);
        let (_, v, d) = param(&sim, "pop_low").unwrap();
        assert!((v / d - 2.0).abs() < 1e-4);
    }
}
