//! Generic SDK replication. Only validated data from matching local packages is applied.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub(super) enum Payload {
    Cube {
        key: String,
        position: [f32; 3],
        size: [f32; 3],
        color: [f32; 3],
    },
    State {
        key: String,
        value: Value,
    },
    Tune {
        key: String,
        values: BTreeMap<String, Value>,
    },
    Car {
        key: String,
        state: vehicles::network::Car,
    },
}
use serde_json::Value;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    owner: String,
    fingerprint: u64,
    payload: Payload,
}
#[derive(Resource, Default)]
pub(crate) struct ModSync {
    local: BTreeMap<(String, String), Payload>,
    published: BTreeSet<String>,
    remote: BTreeMap<(u64, String), (u32, String, String)>,
    states: BTreeMap<String, BTreeMap<String, BTreeMap<String, Value>>>,
    identity: u64,
    elapsed: f32,
    pub status: String,
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<ModSync>()
        .add_systems(Update, synchronize.after(vehicles::present));
}
pub(super) fn snapshot(world: &World) -> Value {
    let (active, id, host) = world
        .get_resource::<crate::multiplayer::Multiplayer>()
        .map_or((false, 0, true), |n| n.mod_identity());
    let sync = world.resource::<ModSync>();
    json!({"active":active,"local_id":id.to_string(),"is_host":host,"states":sync.states,"status":sync.status})
}
fn wire_key(owner: &str, key: &str) -> String {
    format!(
        "{:016x}",
        skate_net::hash(format!("{owner}/{key}").as_bytes())
    )
}
pub(super) fn record(world: &mut World, owner: &str, command: &Command) -> Result<(), String> {
    // Remote descriptors are installed directly and never pass back into this path.
    let mut sync = world.resource_mut::<ModSync>();
    let entry = match command {
        Command::Cube {
            key,
            position,
            size,
            color,
        } => Some((
            format!("cube/{key}"),
            Payload::Cube {
                key: key.clone(),
                position: *position,
                size: *size,
                color: *color,
            },
        )),
        Command::Remove { key } | Command::Overlay { key, .. } => {
            sync.local.remove(&(owner.into(), format!("cube/{key}")));
            None
        }
        Command::VehicleRemove { key } => {
            sync.local.remove(&(owner.into(), format!("tune/{key}")));
            None
        }
        Command::NetworkState { key, value } => Some((
            format!("state/{key}"),
            Payload::State {
                key: key.clone(),
                value: value.clone(),
            },
        )),
        Command::VehicleTune { key, tuning } => {
            let slot = (owner.into(), format!("tune/{key}"));
            let mut values = match sync.local.get(&slot) {
                Some(Payload::Tune { values, .. }) => values.clone(),
                _ => BTreeMap::new(),
            };
            if let Value::Object(fields) =
                serde_json::to_value(tuning).map_err(|e| e.to_string())?
            {
                values.extend(fields.into_iter().filter(|(_, v)| !v.is_null()));
            }
            Some((
                slot.1,
                Payload::Tune {
                    key: key.clone(),
                    values,
                },
            ))
        }
        _ => None,
    };
    if let Some((key, value)) = entry {
        if sync.local.len() >= 128 && !sync.local.contains_key(&(owner.into(), key.clone())) {
            return Err("128 shared mod records maximum".into());
        }
        sync.local.insert((owner.into(), key), value);
    }
    Ok(())
}
fn synchronize(world: &mut World) {
    let dt = world.resource::<Time<Real>>().delta_secs();
    world.resource_scope(|world, mut sync: Mut<ModSync>| {
        sync.elapsed += dt;
        if sync.elapsed < 0.05 {
            return;
        }
        sync.elapsed = 0.;
        let (_, identity, _) = world
            .resource::<crate::multiplayer::Multiplayer>()
            .mod_identity();
        if identity != sync.identity {
            sync.published.clear();
            sync.identity = identity;
        }
        let packages: BTreeMap<_, _> = world
            .resource::<Mods>()
            .manager
            .packages
            .iter()
            .filter(|(_, p)| p.running())
            .map(|(id, p)| (id.clone(), (p.content_fingerprint(), p.root.clone())))
            .collect();
        sync.local
            .retain(|(owner, _), _| packages.contains_key(owner));
        let mut outgoing = sync.local.clone();
        for (owner, key, state) in vehicles::network::capture(world) {
            outgoing.insert((owner, format!("car/{key}")), Payload::Car { key, state });
        }
        let mut published = BTreeSet::new();
        let mut failures = 0;
        sync.states.clear();
        for ((owner, key), payload) in outgoing {
            let Some((fingerprint, _)) = packages.get(&owner) else {
                continue;
            };
            if let Payload::State { key, value } = &payload {
                if !value.is_null() {
                    sync.states
                        .entry(owner.clone())
                        .or_default()
                        .entry(identity.to_string())
                        .or_default()
                        .insert(key.clone(), value.clone());
                }
            }
            let key = wire_key(&owner, &key);
            let bytes = serde_json::to_vec(&Record {
                owner,
                fingerprint: *fingerprint,
                payload,
            })
            .unwrap();
            if identity != 0
                && !world
                    .resource_mut::<crate::multiplayer::Multiplayer>()
                    .publish_mod(&key, bytes)
            {
                failures += 1;
            }
            published.insert(key);
        }
        for key in sync.published.difference(&published) {
            world
                .resource_mut::<crate::multiplayer::Multiplayer>()
                .publish_mod(key, vec![]);
        }
        sync.published = published;
        let records = world
            .resource::<crate::multiplayer::Multiplayer>()
            .mod_records();
        let mut seen = BTreeSet::new();
        let mut mismatches = BTreeSet::new();
        for (peer, wire, seq, bytes) in records {
            let Ok(record) = serde_json::from_slice::<Record>(&bytes) else {
                continue;
            };
            let Some((fingerprint, root)) = packages
                .get(&record.owner)
                .filter(|(f, _)| *f == record.fingerprint)
            else {
                mismatches.insert(record.owner);
                continue;
            };
            let _ = fingerprint;
            let remote_owner = format!("@{peer}:{}", record.owner);
            let record_id = (peer, wire.clone());
            let changed = sync
                .remote
                .get(&record_id)
                .is_none_or(|(old, _, _)| *old != seq);
            let mut object_key = String::new();
            let result = match record.payload {
                Payload::State { key, value } => {
                    if key.len() > 64 || serde_json::to_vec(&value).unwrap().len() > 512 {
                        continue;
                    }
                    if !value.is_null() {
                        sync.states
                            .entry(record.owner.clone())
                            .or_default()
                            .entry(peer.to_string())
                            .or_default()
                            .insert(key, value);
                    }
                    Ok(())
                }
                Payload::Cube {
                    key,
                    position,
                    size,
                    color,
                } => {
                    object_key = format!("cube/{key}");
                    let c = Command::Cube {
                        key,
                        position,
                        size,
                        color,
                    };
                    if !c.validate() {
                        continue;
                    }
                    if changed {
                        world.resource_scope(|world, mut mods: Mut<Mods>| {
                            super::apply_one(world, &mut mods, &remote_owner, c)
                        })
                    } else {
                        Ok(())
                    }
                }
                Payload::Car { key, state } => {
                    object_key = format!("car/{key}");
                    if changed {
                        vehicles::network::receive(world, root, &remote_owner, &key, state)
                    } else {
                        Ok(())
                    }
                }
                Payload::Tune { key, values } => {
                    let Ok(tuning) = serde_json::from_value(serde_json::to_value(values).unwrap())
                    else {
                        continue;
                    };
                    let c = Command::VehicleTune { key, tuning };
                    if !c.validate() {
                        continue;
                    }
                    if changed {
                        vehicles::command(world, root, &remote_owner, c)
                    } else {
                        Ok(())
                    }
                }
            };
            if result.is_ok() {
                seen.insert(record_id.clone());
                sync.remote
                    .insert(record_id, (seq, remote_owner, object_key));
            }
        }
        let removed: Vec<_> = sync
            .remote
            .keys()
            .filter(|id| !seen.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            if let Some((_, owner, key)) = sync.remote.remove(&id) {
                if let Some(key) = key.strip_prefix("cube/") {
                    world.resource_scope(|world, mut mods: Mut<Mods>| {
                        super::retire(world, &mut mods, &(owner.clone(), key.into()))
                    });
                }
                if let Some(key) = key.strip_prefix("car/") {
                    vehicles::network::remove(world, &owner, key);
                }
            }
        }
        // Remote lifecycle messages are observations, not local script commands.
        world
            .resource_mut::<vehicles::Vehicles>()
            .events
            .retain(|e| !e["owner"].as_str().is_some_and(|o| o.starts_with('@')));
        sync.status = if failures > 0 {
            format!("Mod network limit reached: {failures} records")
        } else if !mismatches.is_empty() {
            format!(
                "Mods missing, disabled or different: {}",
                mismatches.into_iter().collect::<Vec<_>>().join(", ")
            )
        } else {
            format!(
                "Matching mods synchronized | {} remote records",
                sync.remote.len()
            )
        };
    });
}

pub(super) fn retire(world: &mut World, owner: &str) {
    world
        .resource_mut::<ModSync>()
        .local
        .retain(|(id, _), _| id != owner);
}
pub(super) fn clear(world: &mut World) {
    let mut s = world.resource_mut::<ModSync>();
    s.local.clear();
    s.remote.clear();
    s.states.clear();
}
