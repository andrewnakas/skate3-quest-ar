//! Native HUD bindings and the original ActionScript-driven movie lifecycle.
use crate::{
    apt_movie::Movie,
    apt_vm::{Host, ObjectKind, Value, Vm},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub sequence_score: i32,
    pub line_score: i32,
    pub sequence_timer: i32,
    pub line_time: f32,
    pub line_capacity: f32,
    pub multiplier: f32,
    pub clean: bool,
    pub sketchy: bool,
    pub stance: [bool; 4],
    pub trick_name: String,
    pub trick_metrics: [Value; 5],
    pub context_tricks: Vec<Value>,
}
pub struct Bindings {
    pub movie: Movie,
    pub input: Input,
    random: u32,
}
pub struct Runtime {
    pub vm: Vm,
    pub bindings: Bindings,
    pub controller: usize,
}

fn array(vm: &mut Vm, values: impl IntoIterator<Item = Value>) -> Result<Value, String> {
    let object = vm.object(ObjectKind::Plain);
    let mut n = 0;
    for value in values {
        vm.set(object, n.to_string(), value)?;
        n += 1;
    }
    vm.set(object, "length", Value::Number(n as f64))?;
    Ok(Value::Object(object))
}
fn score(value: i32) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    if value < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
impl Host for Bindings {
    fn property_changed(&mut self, vm: &mut Vm, object: usize, key: &str) -> Result<(), String> {
        if key == "text" || key == "autoSize" {
            self.movie.text_changed(vm, object)?;
        }
        Ok(())
    }
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
        let native = match vm.objects.get(object).map(|o| &o.kind) {
            Some(ObjectKind::Native(name)) => name.as_str(),
            _ => "global",
        };
        match (native, method) {
            ("Math", "floor") => Ok(Value::Number(
                args.first().ok_or("Math.floor argument")?.number().floor(),
            )),
            ("Math", "random") => {
                // Presentation-only random source; never influences scoring.
                self.random ^= self.random << 13;
                self.random ^= self.random >> 17;
                self.random ^= self.random << 5;
                Ok(Value::Number(self.random as f64 / 4294967296.0))
            }
            // The HUD never enumerates prototype properties. Marking their
            // enumeration flags therefore leaves its observable fields intact.
            ("global", "ASSetPropFlags") => Ok(Value::Undefined),
            ("FELanguage", "GetLanguage") => Ok(Value::Text("english".into())),
            ("Tricks", "GetLineTimerMaxPoints") => {
                Ok(Value::Number(self.input.line_capacity as f64))
            }
            //825C2910 truncates manager+32 to integer;825C28F8 returns+192.
            ("Tricks", "GetLineTimeRemaining") => {
                Ok(Value::Number(self.input.line_time.trunc() as f64))
            }
            ("Tricks", "GetLineScore") => Ok(Value::Number(self.input.line_score as f64)),
            ("HUDComponents", "TrickDisplay_GetSequenceMultiplier") => {
                Ok(Value::Number(self.input.multiplier as f64))
            }
            ("Tricks", "GetGeneralInfo") => array(
                vm,
                [
                    Value::Number(self.input.sequence_score as f64),
                    Value::Text(score(self.input.sequence_score)),
                    // 825C2D90 exposes backend152/153; the shipped
                    // UpdateTrickScoring reads [2] as sketchy, [3] as clean.
                    Value::Bool(self.input.sketchy),
                    Value::Bool(self.input.clean),
                    Value::Number(self.input.sequence_timer as f64),
                    Value::Number(self.input.line_score as f64),
                    Value::Text(score(self.input.line_score)),
                ],
            ),
            ("Tricks", "GetCurrentTrickStance") => array(vm, self.input.stance.map(Value::Bool)),
            ("Tricks", "GetCurrentTrickName") => Ok(Value::Text(self.input.trick_name.clone())),
            ("Tricks", "GetCurrentTrickMetrics") => array(vm, self.input.trick_metrics.clone()),
            ("Tricks", "TrickDisplay_GetAllTrickData") => {
                array(vm, self.input.context_tricks.clone())
            }
            _ => Err(format!(
                "Unimplemented original HUD binding {native}.{method}"
            )),
        }
    }
}
impl Runtime {
    pub fn load(json: &serde_json::Value, input: Input) -> Result<Self, String> {
        let mut vm = Vm::new();
        for name in ["MovieClip", "Tricks", "FELanguage", "HUDComponents", "Math"] {
            let object = vm.object(ObjectKind::Native(name.into()));
            let prototype = vm.object(ObjectKind::Plain);
            vm.set(object, "prototype", Value::Object(prototype))?;
            vm.set(vm.global, name, Value::Object(object))?;
        }
        vm.set(vm.global, "Screen_EdgeOffset", Value::Number(0.0))?;
        let mut bindings = Bindings {
            movie: Movie::load(json)?,
            input,
            random: 0x9e3779b9,
        };
        let initial: Vec<_> = bindings
            .movie
            .characters
            .values()
            .flat_map(|c| &c.frames)
            .flat_map(|f| &f.controls)
            .filter(|c| c.type_name == "do_init_action")
            .map(|c| c.actions_offset)
            .collect();
        for offset in initial {
            let code = bindings.movie.actions[&offset.to_string()].clone();
            vm.run(&code, &mut bindings)?;
        }
        vm.begin_update();
        bindings.movie.initialize(&mut vm)?;
        let mut runtime = Self {
            vm,
            bindings,
            controller: usize::MAX,
        };
        runtime.drain()?;
        let Value::Object(controller) = runtime.vm.get(runtime.bindings.movie.root, "screen")
        else {
            return Err("Original HUD controller was not constructed".into());
        };
        runtime.controller = controller;
        runtime.vm.call_method(
            controller,
            "ScreenShow",
            vec![Value::Bool(true)],
            &mut runtime.bindings,
        )?;
        runtime.drain()?;
        Ok(runtime)
    }
    fn drain(&mut self) -> Result<(), String> {
        let mut calls = 0;
        while let Some((object, offset)) = self.bindings.movie.pending.pop_front() {
            calls += 1;
            if calls > 4096 {
                return Err("HUD frame action limit".into());
            }
            if !self.bindings.movie.instances.contains_key(&object) {
                continue;
            }
            let code = self
                .bindings
                .movie
                .actions
                .get(&offset.to_string())
                .ok_or("Missing HUD action block")?
                .clone();
            self.vm.run_on(object, &code, &mut self.bindings)?;
        }
        Ok(())
    }
    pub fn update(
        &mut self,
        input: Input,
        new_trick: bool,
        modified_trick: bool,
        close_tricks: bool,
    ) -> Result<(), String> {
        self.vm.begin_update();
        let previous = self.bindings.input.clone();
        self.bindings.input = input;
        let mut methods = Vec::new();
        if previous.multiplier != self.bindings.input.multiplier {
            methods.push("UpdateMultiplier");
        }
        if previous.sequence_timer != self.bindings.input.sequence_timer
            || previous.sequence_score != self.bindings.input.sequence_score
            || previous.clean != self.bindings.input.clean
            || previous.sketchy != self.bindings.input.sketchy
            || previous.line_score != self.bindings.input.line_score
        {
            methods.push("UpdateTrickScoring");
        }
        if previous.stance != self.bindings.input.stance {
            methods.push("UpdateStance");
        }
        if new_trick {
            methods.push("RefreshTrickName");
        } else if modified_trick {
            methods.push("CurrentTrickModified");
        }
        if close_tricks {
            methods.push("CloseTrickDisplay");
        }
        for method in methods {
            self.vm
                .call_method(self.controller, method, vec![], &mut self.bindings)?;
            self.drain()?;
        }
        // The movie handles this as event 11 (0196CC), not onEnterFrame.
        // Re-seeking the 501-frame multiTimer_mc on every tick pins it to
        // the same whole-second sample instead of letting gotoAndPlay run.
        if previous.line_time.trunc() != self.bindings.input.line_time.trunc()
            || previous.line_score != self.bindings.input.line_score
        {
            self.vm.call_method(
                self.controller,
                "UpdateLineDisplay",
                vec![],
                &mut self.bindings,
            )?;
            self.drain()?;
        }
        self.bindings.movie.advance(&mut self.vm)?;
        self.drain()?;
        self.vm
            .collect(self.bindings.movie.instances.keys().copied())?;
        Ok(())
    }
}
