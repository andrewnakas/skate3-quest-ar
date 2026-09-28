//! Executes owned movie bytecode against a data-only host. No game or renderer.
#[path = "../../skate-game/src/apt_display.rs"]
mod apt_display;
#[path = "../../skate-game/src/apt_movie.rs"]
mod apt_movie;
#[path = "../../skate-game/src/apt_text.rs"]
mod apt_text;
#[path = "../../skate-game/src/apt_vm.rs"]
mod apt_vm;
use apt_vm::*;
struct Audit {
    movie: apt_movie::Movie,
}
impl Host for Audit {
    fn call(
        &mut self,
        vm: &mut Vm,
        object: usize,
        method: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        if self.movie.method(vm, object, method, &args)? {
            return Ok(Value::Undefined);
        }
        match method {
            "ASSetPropFlags" => Ok(Value::Undefined),
            "GetLineTimerMaxPoints" => Ok(Value::Number(400.0)),
            "GetLineTimeRemaining" | "GetLineScore" => Ok(Value::Number(0.0)),
            "GetLanguage" => Ok(Value::Text("english".into())),
            _ => Err(format!(
                "Unimplemented audit binding {object}.{method}({args:?})"
            )),
        }
    }
}
fn drain(vm: &mut Vm, host: &mut Audit) -> Result<(), String> {
    let mut count = 0;
    while let Some((object, offset)) = host.movie.pending.pop_front() {
        count += 1;
        if count > 4096 {
            return Err("Frame action limit".into());
        }
        if !host.movie.instances.contains_key(&object) {
            continue;
        }
        let code = host
            .movie
            .actions
            .get(&offset.to_string())
            .ok_or("Missing movie action block")?
            .clone();
        vm.run_on(object, &code, host)?;
    }
    Ok(())
}
fn main() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("Expected owned trickdisplay.json")?;
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut host = Audit {
        movie: apt_movie::Movie::load(&json)?,
    };
    let mut vm = Vm::new();
    for name in ["MovieClip", "Tricks", "FELanguage"] {
        let object = vm.object(ObjectKind::Native(name.into()));
        let proto = vm.object(ObjectKind::Plain);
        vm.set(object, "prototype", Value::Object(proto))?;
        vm.set(vm.global, name, Value::Object(object))?;
    }
    vm.set(vm.global, "Screen_EdgeOffset", Value::Number(0.0))?;
    for c in host.movie.characters.values().cloned().collect::<Vec<_>>() {
        for frame in c.frames {
            for control in frame.controls {
                if control.type_name == "do_init_action" {
                    let code = host.movie.actions[&control.actions_offset.to_string()].clone();
                    vm.run(&code, &mut host)?;
                }
            }
        }
    }
    vm.begin_update();
    host.movie.initialize(&mut vm)?;
    drain(&mut vm, &mut host)?;
    let Value::Object(controller) = vm.get(host.movie.root, "screen") else {
        return Err("Root action did not construct controller".into());
    };
    vm.call_method(controller, "ScreenShow", vec![Value::Bool(true)], &mut host)?;
    drain(&mut vm, &mut host)?;
    for _ in 0..90 {
        vm.begin_update();
        host.movie.advance(&mut vm)?;
        drain(&mut vm, &mut host)?;
    }
    println!(
        "Original movie initialized and advanced 90 authored frames; {} retained display instances",
        host.movie.instances.len()
    );
    Ok(())
}
