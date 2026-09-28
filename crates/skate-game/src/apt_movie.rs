//! Original APT movie hierarchy and timeline control, independent of Bevy.
use crate::{
    apt_display::{Control, DisplayList, Placement},
    apt_vm::{Instruction, ObjectKind, Value, Vm},
};
use serde::Deserialize;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Deserialize)]
pub struct Frame {
    pub controls: Vec<Control>,
}
#[derive(Clone, Deserialize)]
pub struct Character {
    pub id: i32,
    pub type_name: String,
    #[serde(default)]
    pub frames: Vec<Frame>,
    pub text: Option<serde_json::Value>,
    pub bounds: Option<[f32; 4]>,
}
#[derive(Clone)]
pub struct Instance {
    pub character: i32,
    pub frame: usize,
    pub playing: bool,
    pub children: BTreeMap<i32, usize>,
    pub placement: Option<Placement>,
}
pub struct Movie {
    pub characters: BTreeMap<i32, Character>,
    pub instances: BTreeMap<usize, Instance>,
    pub actions: BTreeMap<String, Vec<Instruction>>,
    pub pending: VecDeque<(usize, u32)>,
    pub root: usize,
    pub text_assets: crate::apt_text::TextAssets,
    states: BTreeMap<i32, Vec<DisplayList>>,
}
impl Movie {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let characters: Vec<Character> =
            serde_json::from_value(json["characters"].clone()).map_err(|e| e.to_string())?;
        let mut states = BTreeMap::new();
        for c in &characters {
            let mut list = DisplayList::default();
            let mut frames = Vec::new();
            for frame in &c.frames {
                for control in &frame.controls {
                    list.apply(control)?;
                }
                frames.push(list.clone());
            }
            states.insert(c.id, frames);
        }
        Ok(Self {
            characters: characters.into_iter().map(|c| (c.id, c)).collect(),
            instances: BTreeMap::new(),
            actions: serde_json::from_value(json["actions"].clone()).map_err(|e| e.to_string())?,
            pending: VecDeque::new(),
            root: usize::MAX,
            text_assets: crate::apt_text::TextAssets::load(json)?,
            states,
        })
    }
    pub fn initialize(&mut self, vm: &mut Vm) -> Result<(), String> {
        self.root = self.create(vm, 0, None, 0)?;
        vm.set(self.root, "_root", Value::Object(self.root))?;
        // Every clip sees the same authored root, including unnamed children.
        for id in self.instances.keys() {
            vm.set(*id, "_root", Value::Object(self.root))?;
        }
        Ok(())
    }
    fn create(
        &mut self,
        vm: &mut Vm,
        character: i32,
        parent: Option<usize>,
        depth: usize,
    ) -> Result<usize, String> {
        if depth > 32 || self.instances.len() > 2048 {
            return Err("APT movie hierarchy limit".into());
        }
        let c = self
            .characters
            .get(&character)
            .ok_or("APT unknown character")?
            .clone();
        let id = vm.object(ObjectKind::Native(format!("movie:{character}")));
        if self.root != usize::MAX {
            vm.set(id, "_root", Value::Object(self.root))?;
        }
        if let Some(parent) = parent {
            vm.set(id, "_parent", Value::Object(parent))?;
        }
        vm.set(id, "_x", Value::Number(0.0))?;
        vm.set(id, "_y", Value::Number(0.0))?;
        vm.set(id, "_visible", Value::Bool(true))?;
        vm.set(id, "_alpha", Value::Number(100.0))?;
        if let Some(text) = &c.text {
            vm.set(
                id,
                "text",
                Value::Text(text["initial_text"].as_str().unwrap_or("").into()),
            )?;
        }
        self.instances.insert(
            id,
            Instance {
                character,
                frame: 0,
                playing: !c.frames.is_empty(),
                children: BTreeMap::new(),
                placement: None,
            },
        );
        self.text_changed(vm, id)?;
        if !c.frames.is_empty() {
            self.seek(vm, id, 0, depth + 1)?;
        }
        Ok(id)
    }
    pub fn text_changed(&self, vm: &mut Vm, id: usize) -> Result<(), String> {
        let Some(instance) = self.instances.get(&id) else {
            return Ok(());
        };
        let character = &self.characters[&instance.character];
        let Some(text) = &character.text else {
            return Ok(());
        };
        let value = self.text_assets.localize(&vm.get(id, "text").text());
        vm.set(id, "_displayText", Value::Text(value.clone()))?;
        let font = self
            .text_assets
            .fonts
            .get(&(text["font_id"].as_i64().ok_or("Invalid text font id")? as i32))
            .ok_or("Missing original text font")?;
        let height = text["font_height"].as_f64().ok_or("Invalid text size")? as f32;
        let width = font.width(&value, height);
        vm.set(id, "textWidth", Value::Number(width as f64))?;
        let bounds = character.bounds.ok_or("Missing text bounds")?;
        let autosize = vm.get(id, "autoSize").text();
        vm.set(
            id,
            "_width",
            Value::Number(
                if autosize == "left" || autosize == "right" || autosize == "center" {
                    width
                } else {
                    bounds[2] - bounds[0]
                } as f64,
            ),
        )?;
        Ok(())
    }
    fn remove(&mut self, vm: &mut Vm, id: usize) {
        if let Some(instance) = self.instances.remove(&id) {
            for child in instance.children.values() {
                self.remove(vm, *child);
            }
        }
        // Retired handles can remain referenced by ActionScript, but no longer
        // participate in timeline advancement or rendering.
        let _ = vm.set(id, "_visible", Value::Bool(false));
    }
    pub fn seek(
        &mut self,
        vm: &mut Vm,
        id: usize,
        frame: usize,
        nesting: usize,
    ) -> Result<(), String> {
        let instance = self
            .instances
            .get(&id)
            .ok_or("APT absent movie instance")?
            .clone();
        let character = self
            .characters
            .get(&instance.character)
            .ok_or("APT absent movie character")?;
        if frame >= character.frames.len() {
            return Err(format!(
                "APT frame {frame} outside character {}",
                character.id
            ));
        }
        let frame_count = character.frames.len();
        let actions: Vec<_> = character.frames[frame]
            .controls
            .iter()
            .filter(|c| c.type_name == "do_action" && c.actions_offset != 0)
            .map(|c| c.actions_offset)
            .collect();
        let list = self.states[&instance.character][frame].clone();
        let mut children = BTreeMap::new();
        for (depth, placement) in list.depths {
            let previous = instance.children.get(&depth).copied();
            if let Some(name) = previous
                .and_then(|old| self.instances.get(&old))
                .and_then(|i| i.placement.as_ref())
                .map(|p| p.name.clone())
            {
                if !name.is_empty() && name != placement.name {
                    vm.objects[id].fields.remove(&name);
                }
            }
            let child = if let Some(old) = previous.filter(|old| {
                self.instances
                    .get(old)
                    .is_some_and(|i| i.character == placement.character)
            }) {
                old
            } else {
                if let Some(old) = previous {
                    self.remove(vm, old);
                }
                self.create(vm, placement.character, Some(id), nesting + 1)?
            };
            let old = self.instances[&child].placement.as_ref();
            if old.is_none_or(|old| old.matrix != placement.matrix) {
                vm.set(child, "_x", Value::Number(placement.matrix[4] as f64))?;
                vm.set(child, "_y", Value::Number(placement.matrix[5] as f64))?;
            }
            if !placement.name.is_empty() {
                vm.set(id, &placement.name, Value::Object(child))?;
            }
            self.instances.get_mut(&child).unwrap().placement = Some(placement);
            children.insert(depth, child);
        }
        for (depth, child) in &instance.children {
            if !children.contains_key(depth) {
                if let Some(name) = self
                    .instances
                    .get(child)
                    .and_then(|i| i.placement.as_ref())
                    .map(|p| p.name.clone())
                {
                    if !name.is_empty() {
                        vm.objects[id].fields.remove(&name);
                    }
                }
                self.remove(vm, *child);
            }
        }
        let current = self.instances.get_mut(&id).unwrap();
        current.frame = frame;
        current.children = children;
        vm.set(id, "_currentframe", Value::Number((frame + 1) as f64))?;
        vm.set(id, "_totalframes", Value::Number(frame_count as f64))?;
        for offset in actions {
            self.pending.push_back((id, offset));
        }
        Ok(())
    }
    pub fn advance(&mut self, vm: &mut Vm) -> Result<(), String> {
        let playing: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, i)| i.playing)
            .map(|(id, i)| (*id, i.character, i.frame))
            .collect();
        for (id, character, frame) in playing {
            if !self.instances.contains_key(&id) {
                continue;
            }
            let count = self.characters[&character].frames.len();
            if count > 1 {
                self.seek(vm, id, (frame + 1) % count, 0)?;
            }
        }
        Ok(())
    }
    pub fn method(
        &mut self,
        vm: &mut Vm,
        id: usize,
        method: &str,
        args: &[Value],
    ) -> Result<bool, String> {
        if !self.instances.contains_key(&id) {
            return Ok(false);
        }
        match method {
            "stop" => self.instances.get_mut(&id).unwrap().playing = false,
            "play" => self.instances.get_mut(&id).unwrap().playing = true,
            "gotoAndPlay" | "gotoAndStop" => {
                let c = &self.characters[&self.instances[&id].character];
                let frame = match args.first().ok_or("APT goto requires frame")? {
                    Value::Text(label) => c
                        .frames
                        .iter()
                        .position(|f| {
                            f.controls.iter().any(|control| {
                                control.type_name == "frame_label"
                                    && control.label.as_deref() == Some(label)
                            })
                        })
                        .ok_or_else(|| format!("APT character {} lacks label {label}", c.id))?,
                    value => {
                        let frame = value.number();
                        if !frame.is_finite() || frame < 0.0 {
                            return Err(format!(
                                "Invalid APT frame {frame} in {} on character {}",
                                method, c.id
                            ));
                        }
                        // The shipped line timer deliberately requests zero at
                        // full capacity. Frame zero addresses the first frame.
                        (frame as usize).saturating_sub(1)
                    }
                };
                self.seek(vm, id, frame, 0)?;
                self.instances.get_mut(&id).unwrap().playing = method == "gotoAndPlay";
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
