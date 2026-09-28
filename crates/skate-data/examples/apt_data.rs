//! Bytecode-only validation. No Bevy, game, input, rendering or simulation.
#[path = "../../skate-game/src/apt_display.rs"]
mod apt_display;
#[path = "../../skate-game/src/apt_vm.rs"]
mod apt_vm;
use apt_vm::*;
struct Audit;

// Build the authored frame-zero hierarchy, so constructor property writes
// have actual targets. This is not a timeline playback or renderer test.
fn initial_tree(
    vm: &mut Vm,
    json: &serde_json::Value,
    character: i32,
    nesting: usize,
) -> Result<usize, String> {
    if nesting > 32 {
        return Err("APT hierarchy nesting limit".into());
    }
    let record = json["characters"]
        .as_array()
        .ok_or("No characters")?
        .iter()
        .find(|c| c["id"].as_i64() == Some(character as i64))
        .ok_or("Unknown APT character")?;
    let object = vm.object(ObjectKind::Native(format!("character:{character}")));
    vm.set(object, "_visible", Value::Bool(true))?;
    vm.set(object, "_x", Value::Number(0.0))?;
    vm.set(object, "_y", Value::Number(0.0))?;
    let mut list = apt_display::DisplayList::default();
    if let Some(controls) = record["frames"][0]["controls"].as_array() {
        for control in controls {
            list.apply(&serde_json::from_value(control.clone()).map_err(|e| e.to_string())?)?;
        }
    }
    for placement in list.depths.values() {
        let child = initial_tree(vm, json, placement.character, nesting + 1)?;
        vm.set(child, "_parent", Value::Object(object))?;
        vm.set(child, "_x", Value::Number(placement.matrix[4] as f64))?;
        vm.set(child, "_y", Value::Number(placement.matrix[5] as f64))?;
        if !placement.name.is_empty() {
            vm.set(object, &placement.name, Value::Object(child))?;
        }
    }
    Ok(object)
}
impl Host for Audit {
    fn call(
        &mut self,
        _vm: &mut Vm,
        object: usize,
        method: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        println!("native {object}.{method}({args:?})");
        Ok(Value::Undefined)
    }
}
fn main() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Expected private trickdisplay.json")?;
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut vm = Vm::new();
    let mut timeline_count = 0;
    for character in json["characters"].as_array().ok_or("No characters")? {
        let Some(frames) = character["frames"].as_array() else {
            continue;
        };
        let mut list = apt_display::DisplayList::default();
        for frame in frames {
            for control in frame["controls"].as_array().ok_or("No controls")? {
                list.apply(&serde_json::from_value(control.clone()).map_err(|e| e.to_string())?)?;
            }
        }
        timeline_count += 1;
    }
    println!("Validated placement controls in {timeline_count} original timelines");
    let clip = vm.object(ObjectKind::Native("MovieClip".into()));
    let proto = vm.object(ObjectKind::Plain);
    vm.set(clip, "prototype", Value::Object(proto))?;
    vm.set(vm.global, "MovieClip", Value::Object(clip))?;
    for c in json["characters"].as_array().ok_or("No characters")? {
        if let Some(frames) = c["frames"].as_array() {
            for frame in frames {
                for control in frame["controls"].as_array().ok_or("No controls")? {
                    if control["type_name"] == "do_init_action" {
                        let key = control["actions_offset"].as_u64().unwrap().to_string();
                        let code: Vec<Instruction> =
                            serde_json::from_value(json["actions"][&key].clone())
                                .map_err(|e| e.to_string())?;
                        vm.run(&code, &mut Audit)?;
                    }
                }
            }
        }
    }
    let Value::Object(class) = vm.get(vm.global, "trickdisplay2") else {
        return Err("APT class initialization failed".into());
    };
    println!("APT class initialized; {} objects", vm.objects.len());
    let screen = initial_tree(&mut vm, &json, 0, 0)?;
    vm.begin_update();
    let instance = vm.construct(class, vec![Value::Object(screen)], &mut Audit)?;
    for name in [
        "mcTrickAnim0",
        "mcTrickAnim1",
        "mcTrickAnim2",
        "mcTrickAnim3",
    ] {
        let Value::Object(child) = vm.get(screen, name) else {
            return Err(format!("Missing authored child {name}"));
        };
        if vm.get(child, "_visible") != Value::Bool(false) {
            return Err(format!("Constructor failed to hide {name}"));
        }
    }
    println!(
        "APT constructor completed; instance {instance}; {} objects",
        vm.objects.len()
    );
    Ok(())
}
