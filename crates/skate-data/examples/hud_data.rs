//! Original action-stream validation with supplied data, without game systems.
#[path = "../../skate-game/src/apt_display.rs"]
mod apt_display;
#[path = "../../skate-game/src/apt_movie.rs"]
mod apt_movie;
#[path = "../../skate-game/src/apt_scene.rs"]
mod apt_scene;
#[path = "../../skate-game/src/apt_text.rs"]
mod apt_text;
#[path = "../../skate-game/src/apt_vm.rs"]
mod apt_vm;
#[path = "../../skate-game/src/hud_runtime.rs"]
mod hud_runtime;
use apt_vm::Value;
fn main() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Expected owned trickdisplay.json")?;
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut input = hud_runtime::Input {
        sequence_score: 0,
        line_score: 0,
        sequence_timer: 0,
        line_time: 0.0,
        line_capacity: 400.0,
        multiplier: 1.0,
        clean: false,
        sketchy: false,
        stance: [false; 4],
        trick_name: String::new(),
        trick_metrics: std::array::from_fn(|_| Value::Undefined),
        context_tricks: Vec::new(),
    };
    let mut runtime = hud_runtime::Runtime::load(&json, input.clone())?;
    let shapes: apt_scene::Shapes =
        serde_json::from_value(json["shapes"].clone()).map_err(|e| e.to_string())?;
    let mut max_draws = 0;
    let mut saw_native_shadow_pair = false;
    let mut glow_alphas = std::collections::BTreeSet::new();
    let mut flicker_alphas = std::collections::BTreeSet::new();
    let mut previous_timer_frame = None;
    for tick in 0..1800 {
        let new_trick = tick % 180 == 10;
        if new_trick {
            input.trick_name = "Kickflip".into();
            input.trick_metrics = [
                Value::Text(input.trick_name.clone()),
                Value::Number(0.0),
                Value::Number(0.0),
                Value::Bool(false),
                Value::Bool(false),
            ];
            input.sequence_score += 100;
            input.line_score += 100;
            input.clean = tick % 360 == 10;
            input.sketchy = !input.clean;
            input.line_time = 8.0;
            input.multiplier = match (tick / 180) % 3 {
                0 => 1.5,
                1 => 2.0,
                _ => 3.0,
            };
        }
        input.line_time = (input.line_time - 1.0 / 60.0).max(0.0);
        input.sequence_timer = input.line_time as i32;
        runtime
            .update(input.clone(), new_trick, false, tick % 180 == 120)
            .map_err(|e| format!("Data frame {tick}: {e}"))?;
        if tick == 10 {
            if !runtime.vm.get(runtime.controller, "mLastClean").truth()
                || runtime.vm.get(runtime.controller, "mLastSketchy").truth()
            {
                return Err("Clean landing selected the sketchy color timeline".into());
            }
        }
        let draws = apt_scene::draw(&runtime.bindings.movie, &runtime.vm, &shapes)?;
        for (&id, instance) in &runtime.bindings.movie.instances {
            let name = instance.placement.as_ref().map(|p| p.name.as_str());
            if name == Some("multiTimer_mc") && (20..40).contains(&tick) {
                if let Some(previous) = previous_timer_frame {
                    if instance.frame != previous + 1 {
                        return Err(
                            "Line timer was re-seeked instead of playing consecutive frames".into(),
                        );
                    }
                }
                previous_timer_frame = Some(instance.frame);
            }
            if input.multiplier == 3.0 {
                let alpha = runtime.vm.get(id, "_alpha").number();
                if name == Some("mBlueGlow") {
                    if !(80.0..100.0).contains(&alpha) {
                        return Err("Blue glow outside authored flicker range".into());
                    }
                    glow_alphas.insert(alpha.to_bits());
                }
                if name == Some("mFlicker") {
                    if !(90.0..100.0).contains(&alpha) {
                        return Err("Multiplier flicker outside authored range".into());
                    }
                    flicker_alphas.insert(alpha.to_bits());
                }
            }
        }
        max_draws = max_draws.max(draws.len());
        saw_native_shadow_pair |= draws.windows(2).any(|pair| {
            pair[0].texture.contains("futurashadow")
                && pair[0].multiply[..3] == [0., 0., 0.]
                && pair[1].texture.contains("futuraheavy")
                && pair[1].multiply[0] > 0.
        });

        if draws
            .iter()
            .flat_map(|d| &d.vertices)
            .any(|v| v.position.iter().chain(v.uv.iter()).any(|x| !x.is_finite()))
        {
            return Err("Nonfinite authored draw geometry".into());
        }
    }
    if !saw_native_shadow_pair {
        return Err("Missing native black shadow / heavy foreground pair".into());
    }
    if glow_alphas.len() < 4 || flicker_alphas.len() < 4 {
        return Err("Original x3 blue glow/flicker clips did not animate".into());
    }
    println!(
        "Original HUD action audit passed 1800 authored frames; {} object slots, {} display instances",
        runtime.vm.objects.len(),
        runtime.bindings.movie.instances.len()
    );
    println!("Validated glyph/shape traversal: maximum {max_draws} draw batches");
    Ok(())
}
