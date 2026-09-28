use crate::{Manifest, read_bounded};
use mlua::{HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, VmState};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    NetworkState {
        key: String,
        #[serde(default)]
        value: Value,
    },
    VehicleTune {
        key: String,
        tuning: skate_vehicles::VehicleTuning,
    },
    VehicleSpawn {
        key: String,
        definition: String,
        position: [f32; 3],
        heading: f32,
    },
    VehicleRemove {
        key: String,
    },
    VehicleEnter {
        key: String,
    },
    VehicleExit {
        key: String,
    },
    VehicleReset {
        key: String,
        position: [f32; 3],
        heading: f32,
    },
    VehicleControl {
        key: String,
        controls: skate_vehicles::Controls,
    },
    Trainer {
        tuning: crate::TrainerTuning,
    },
    Log {
        text: String,
    },
    Animation {
        path: String,
    },
    Overlay {
        key: String,
        text: String,
    },
    Cube {
        key: String,
        position: [f32; 3],
        size: [f32; 3],
        color: [f32; 3],
    },
    Remove {
        key: String,
    },
    Teleport {
        position: [f32; 3],
        heading: f32,
        on_board: bool,
    },
}
impl Command {
    pub fn validate(&self) -> bool {
        let point = |p: &[f32; 3]| p.iter().all(|v| v.is_finite() && v.abs() <= 100_000.);
        match self {
            Self::NetworkState { key, value } => {
                crate::schema::valid_id(key)
                    && serde_json::to_vec(value).is_ok_and(|v| v.len() <= 512)
            }
            Self::VehicleTune { key, tuning } => crate::schema::valid_id(key) && tuning.valid(),
            Self::VehicleSpawn {
                key,
                definition,
                position,
                heading,
            } => {
                crate::schema::valid_id(key)
                    && skate_vehicles::package_path(definition)
                    && point(position)
                    && heading.is_finite()
            }
            Self::VehicleReset {
                key,
                position,
                heading,
            } => crate::schema::valid_id(key) && point(position) && heading.is_finite(),
            Self::VehicleControl { key, controls } => {
                crate::schema::valid_id(key) && controls.valid()
            }
            Self::VehicleRemove { key }
            | Self::VehicleEnter { key }
            | Self::VehicleExit { key } => crate::schema::valid_id(key),
            Self::Trainer { tuning } => tuning.valid(),
            Self::Animation { path } => !path.is_empty() && path.len() <= 256,
            Self::Log { text } => text.len() <= 2048,
            Self::Overlay { key, text } => crate::schema::valid_id(key) && text.len() <= 1024,
            Self::Cube {
                key,
                position,
                size,
                color,
            } => {
                crate::schema::valid_id(key)
                    && point(position)
                    && size.iter().all(|v| v.is_finite() && *v > 0. && *v <= 100.)
                    && color
                        .iter()
                        .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            }
            Self::Remove { key } => crate::schema::valid_id(key),
            Self::Teleport {
                position, heading, ..
            } => point(position) && heading.is_finite(),
        }
    }
}
pub(crate) struct Vm {
    lua: Lua,
    callbacks: Table,
    advance: mlua::Function,
    budget: Arc<AtomicUsize>,
    queue: Arc<Mutex<Vec<Command>>>,
}
impl Vm {
    pub fn new(
        root: &Path,
        manifest: &Manifest,
        settings: &BTreeMap<String, Value>,
        snapshot: &Value,
    ) -> Result<Self, String> {
        let build = || -> mlua::Result<Self> {
            let lua = Lua::new_with(
                StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
                LuaOptions::default(),
            )?;
            lua.set_memory_limit(8 * 1024 * 1024)?;
            // No catch/retry loops around quota errors, coroutine bypass, bytecode or filesystem loaders.
            for key in [
                "pcall",
                "xpcall",
                "load",
                "loadfile",
                "dofile",
                "collectgarbage",
                "print",
            ] {
                lua.globals().set(key, mlua::Value::Nil)?;
            }
            let budget = Arc::new(AtomicUsize::new(100));
            let counter = budget.clone();
            lua.set_hook(
                HookTriggers::new().every_nth_instruction(1000),
                move |_, _| {
                    if counter
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                        .is_err()
                    {
                        return Err(mlua::Error::RuntimeError(
                            "Lua instruction budget exhausted".into(),
                        ));
                    }
                    Ok(VmState::Continue)
                },
            )?;
            let queue = Arc::new(Mutex::new(Vec::<Command>::new()));
            let out = queue.clone();
            let sdk = lua.create_table()?;
            sdk.set("api_version", 1)?;
            sdk.set("mod_id", manifest.id.clone())?;
            sdk.set(
                "_submit",
                lua.create_function(move |lua, value: mlua::Value| {
                    let c: Command = lua.from_value(value)?;
                    if !c.validate() {
                        return Err(mlua::Error::RuntimeError(
                            "Invalid command arguments".into(),
                        ));
                    }
                    let mut q = out.lock().unwrap();
                    if q.len() >= 128 {
                        return Err(mlua::Error::RuntimeError(
                            "128 commands per callback maximum".into(),
                        ));
                    }
                    q.push(c);
                    Ok(())
                })?,
            )?;
            let asset_root = root.to_path_buf();
            sdk.set(
                "read_text",
                lua.create_function(move |_, path: String| {
                    let b = read_bounded(&asset_root, &path, 256 * 1024)
                        .map_err(mlua::Error::RuntimeError)?;
                    String::from_utf8(b).map_err(mlua::Error::external)
                })?,
            )?;
            sdk.set("settings", lua.to_value(settings)?)?;
            sdk.set("snapshot", lua.to_value(snapshot)?)?;
            lua.globals().set("sdk", sdk)?;
            lua.load(include_str!("api.lua"))
                .set_name("@skate-sdk-1")
                .exec()?;
            let sdk: Table = lua.globals().get("sdk")?;
            let advance = sdk.get("_advance")?;
            sdk.set("_advance", mlua::Value::Nil)?;
            let code = crate::read_bounded(root, &manifest.entry, 256 * 1024)
                .map_err(mlua::Error::RuntimeError)?;
            let source = std::str::from_utf8(&code).map_err(mlua::Error::external)?;
            let callbacks: Table = lua
                .load(source)
                .set_name(format!("@{}/{}", manifest.id, manifest.entry))
                .eval()?;
            for pair in callbacks.clone().pairs::<String, mlua::Value>() {
                let (key, value) = pair?;
                if ![
                    "on_load",
                    "on_unload",
                    "on_update",
                    "on_fixed_update",
                    "on_event",
                    "on_settings",
                ]
                .contains(&key.as_str())
                    || !matches!(value, mlua::Value::Function(_))
                {
                    return Err(mlua::Error::RuntimeError(format!(
                        "Unknown callback or non-function: {key}"
                    )));
                }
            }
            // Top-level commands are staged together with on_load, not committed yet.
            Ok(Self {
                lua,
                callbacks,
                advance,
                budget,
                queue,
            })
        };
        build().map_err(|e| e.to_string())
    }
    pub fn settings(&mut self, settings: &BTreeMap<String, Value>) -> mlua::Result<()> {
        self.lua
            .globals()
            .get::<Table>("sdk")?
            .set("settings", self.lua.to_value(settings)?)
    }
    pub fn call(
        &mut self,
        name: &str,
        payload: Value,
        snapshot: &Value,
    ) -> Result<Vec<Command>, String> {
        self.budget.store(100, Ordering::Relaxed);
        let invoke = || -> mlua::Result<()> {
            self.lua
                .globals()
                .get::<Table>("sdk")?
                .set("snapshot", self.lua.to_value(snapshot)?)?;
            if let Some(f) = self.callbacks.get::<Option<mlua::Function>>(name)? {
                f.call::<()>(self.lua.to_value(&payload)?)?;
            }
            if name == "on_update" {
                self.advance
                    .call::<()>(payload["dt"].as_f64().unwrap_or(0.))?;
            }
            Ok(())
        };
        let result = invoke();
        let commands = std::mem::take(&mut *self.queue.lock().unwrap());
        result.map(|_| commands).map_err(|e| e.to_string())
    }
}
