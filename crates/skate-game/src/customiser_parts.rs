//! Resident retail parts: selection changes visibility/materials; morphs stay on GPU.
use crate::customiser_material::{SkaterMaterial, SkinStamp};
use bevy::{gltf::Gltf, mesh::morph::MorphWeights, prelude::*};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Deserialize)]
pub(crate) struct Part {
    pub slot: String,
    pub name: String,
    pub flags: HashMap<String, String>,
    pub materials: Vec<String>,
    pub scene: String,
}
impl Part {
    pub fn flag(&self, key: &str) -> &str {
        self.flags.get(key).map(String::as_str).unwrap_or("")
    }
}
#[derive(Clone, Deserialize)]
pub(crate) struct Material {
    pub name: String,
    pub flags: HashMap<String, String>,
    diffuse: String,
    normal: Option<String>,
    rough: Option<String>,
    alpha: bool,
    #[serde(default)]
    opacity: Option<String>,
    tint: [f32; 3],
    metallic: f32,
    roughness: f32,
    #[serde(default)]
    lighting: Option<crate::retail_character::MaterialData>,
}
impl Material {
    pub fn flag(&self, key: &str) -> &str {
        self.flags.get(key).map(String::as_str).unwrap_or("")
    }
}
#[derive(Clone, Deserialize)]
pub(crate) struct Colour {
    pub name: String,
    pub rgb: [f32; 3],
}
#[derive(Clone, Deserialize)]
pub(crate) struct Tattoo {
    pub name: String,
    pub texture: String,
    pub bounds: [f32; 4],
}
#[derive(Default, Deserialize)]
pub(crate) struct Library {
    pub models: HashMap<String, Part>,
    pub materials: HashMap<String, Material>,
    pub defaults: HashMap<String, Value>,
    pub morphs: Vec<String>,
    #[serde(default)]
    pub tattoos: HashMap<String, Tattoo>,
    #[serde(default)]
    pub colours: Vec<Colour>,
}
#[derive(Resource, Default)]
pub(crate) struct Parts {
    pub library: Library,
    geometry: HashMap<String, Handle<Gltf>>,
    materials: HashMap<String, (Handle<SkaterMaterial>, Vec<Handle<Image>>)>,
    instances: HashMap<String, Entity>,
    tattoos: HashMap<String, Handle<Image>>,
    pub applied: Value,
}
#[derive(Component)]
pub(crate) struct PartRoot(pub String);
pub(crate) fn asset_directory(assets: &std::path::Path) -> std::path::PathBuf {
    let base = assets.join("private/customisation");
    if std::fs::read(base.join("customiser-availability.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|v| v["version"] == 1 && v["status"] == "unavailable")
    {
        return base.join("unavailable");
    }
    if let Some(set) = std::fs::read(base.join("current.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["set"].as_str().map(str::to_owned))
        .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return base.join("sets").join(set);
    }
    base
}
impl Parts {
    #[cfg(test)]
    pub fn for_test(library: Library) -> Self {
        Self {
            library,
            ..default()
        }
    }
    pub fn resolve(&self, profile: &Value) -> Result<Value, String> {
        let lib = &self.library;
        if let Some(gender) = profile["gender"].as_str() {
            if !["male", "female"].contains(&gender) {
                return Err("Invalid body".into());
            }
        }
        for (name, max) in [
            ("truck", 1.),
            ("wheel", 1.),
            ("stance", 1.),
            ("style", 3.),
            ("posture", 3.),
        ] {
            if let Some(value) = profile.get(name) {
                if value.as_f64().is_none_or(|v| !(0.0..=max).contains(&v)) {
                    return Err(format!("Invalid {name}"));
                }
            }
        }
        if let Some(colours) = profile["colours"].as_object() {
            for value in colours.values() {
                // The menu uses null to clear a tint and restore the authored
                // material colour; profile_material already handles this reset.
                if value.is_null() {
                    continue;
                }
                let rgb = serde_json::from_value::<[f32; 3]>(value.clone())
                    .map_err(|_| "Invalid clothing colour")?;
                if rgb.iter().any(|v| !(0.0..=1.0).contains(v)) {
                    return Err("Invalid clothing colour".into());
                }
            }
        }
        for key in ["skin_tint", "hair_tint"] {
            if let Some(value) = profile.get(key) {
                let v = serde_json::from_value::<[f32; 3]>(value.clone())
                    .map_err(|_| "Invalid colour")?;
                if v.iter().any(|v| !(0.0..=1.0).contains(v)) {
                    return Err("Invalid colour".into());
                }
            }
        }
        if let Some(weights) = profile["morphs"].as_object() {
            if weights.iter().any(|(key, v)| {
                !lib.morphs.contains(key) || v.as_f64().is_none_or(|v| !(0.0..=0.5).contains(&v))
            }) {
                return Err("Invalid face or body value".into());
            }
        }
        if let Some(gestures) = profile["gestures"].as_object() {
            if gestures.iter().any(|(key, v)| {
                !["0", "1", "2", "3"].contains(&key.as_str()) || v.as_u64().is_none_or(|v| v >= 37)
            }) {
                return Err("Invalid gesture".into());
            }
        }
        if let Some(zones) = profile["tattoos"].as_object() {
            for (zone, v) in zones {
                if !["Arm", "Leg"].contains(&zone.as_str())
                    || v["id"]
                        .as_str()
                        .is_some_and(|id| !lib.tattoos.contains_key(id))
                    || v["side"]
                        .as_u64()
                        .is_some_and(|v| v > if zone == "Arm" { 3 } else { 1 })
                {
                    return Err("Invalid tattoo".into());
                }
            }
        }

        let mut result = profile.clone();
        let gender = profile["gender"].as_str().unwrap_or("male");
        if let Some(hair) = profile.get("hair_choice") {
            result["selections"]["Hair"] = hair.clone();
        }
        let selections = result["selections"]
            .as_object_mut()
            .ok_or("No outfit selected")?;
        selections.retain(|_, v| !v.is_null());
        for (slot, v) in selections.iter() {
            let id = v["asset_id"].as_str().ok_or("Invalid item")?;
            let part = lib.models.get(id).ok_or("Item unavailable")?;
            if part.slot != *slot || !["", gender, "unisex"].contains(&part.flag("Gender")) {
                return Err("Choose an item for this body".into());
            }
            let mid = v["material_id"].as_str().ok_or("Invalid colour")?;
            if !part.materials.iter().any(|s| s == mid) || !lib.materials.contains_key(mid) {
                return Err("Colour unavailable".into());
            }
        }
        let model = |s: &serde_json::Map<String, Value>, slot: &str| {
            s.get(slot)
                .and_then(|v| v["asset_id"].as_str())
                .and_then(|id| lib.models.get(id))
        };
        let choose = |s: &mut serde_json::Map<String, Value>,
                      slot: &str,
                      key: &str,
                      value: &str|
         -> Result<(), String> {
            if value.is_empty() || value == "none" {
                s.remove(slot);
                return Ok(());
            }
            if model(s, slot).is_some_and(|p| p.flag(key) == value) {
                return Ok(());
            }
            let mut candidates: Vec<_> = lib
                .models
                .iter()
                .filter(|(_, p)| {
                    p.slot == slot
                        && ["", gender, "unisex"].contains(&p.flag("Gender"))
                        && p.flag(key) == value
                })
                .collect();
            candidates.sort_by_key(|(id, _)| *id);
            let (id, p) = candidates
                .first()
                .ok_or_else(|| format!("No matching {}", slot))?;
            let mid = p
                .materials
                .iter()
                .find(|id| {
                    lib.materials
                        .get(*id)
                        .is_some_and(|m| m.flag("IsDefault") == "yes")
                })
                .or_else(|| {
                    p.materials
                        .iter()
                        .find(|id| lib.materials.contains_key(*id))
                })
                .ok_or("No matching material")?;
            s.insert(
                slot.into(),
                serde_json::json!({"asset_id":id,"material_id":mid}),
            );
            Ok(())
        };
        if let Some(outer) = model(selections, "OuterTorso") {
            choose(
                selections,
                "Arm",
                "ArmModelType",
                outer.flag("ArmModelRequired"),
            )?;
            choose(
                selections,
                "InnerTorso",
                "InnerCutType",
                outer.flag("InnerCutTypeRequired"),
            )?;
            if outer.flag("SupportsWristItem") == "no" {
                selections.remove("WristItem");
            }
            if outer.flag("IsNecklaceRemoved") == "yes" {
                selections.remove("Jewellery");
            }
        }
        if let Some(pants) = model(selections, "Pants") {
            choose(
                selections,
                "Leg",
                "LegModelType",
                pants.flag("LegModelRequired"),
            )?;
        }
        if let Some(leg) = model(selections, "Leg") {
            let sock_style = selections
                .get("Sock")
                .and_then(|v| v["material_id"].as_str())
                .and_then(|id| lib.materials.get(id))
                .map(|m| m.flag("SockStyle"))
                .unwrap_or("none");
            let desired = if sock_style == "none" { "" } else { sock_style };
            if leg.flag("RequiresSockStyle") != desired
                && !(desired.is_empty() && leg.flag("RequiresSockStyle") == "none")
            {
                let mut matches: Vec<_> = lib
                    .models
                    .iter()
                    .filter(|(_, p)| {
                        p.slot == "Leg"
                            && ["", gender, "unisex"].contains(&p.flag("Gender"))
                            && p.flag("LegModelType") == leg.flag("LegModelType")
                            && (p.flag("RequiresSockStyle") == desired
                                || (desired.is_empty() && p.flag("RequiresSockStyle") == "none"))
                    })
                    .collect();
                matches.sort_by_key(|(id, _)| *id);
                let (id, p) = matches.first().ok_or("Choose another sock length")?;
                selections.insert(
                    "Leg".into(),
                    serde_json::json!({"asset_id":id,"material_id":p.materials[0]}),
                );
            }
        }
        if !selections.contains_key("Hat") {
            if let Some(hair) = model(selections, "Hair") {
                if hair.flag("HairModelType") != "full" {
                    let candidate = lib
                        .models
                        .iter()
                        .filter(|(_, p)| {
                            p.slot == "Hair"
                                && p.flag("HairStyle") == hair.flag("HairStyle")
                                && p.flag("Gender") == hair.flag("Gender")
                                && p.flag("HairModelType") == "full"
                        })
                        .min_by_key(|(id, _)| *id);
                    if let Some((id, p)) = candidate {
                        selections.insert(
                            "Hair".into(),
                            serde_json::json!({"asset_id":id,"material_id":p.materials[0]}),
                        );
                    }
                }
            }
        }
        if let Some(hat) = model(selections, "Hat") {
            let required = hat.flag("RequiresHairModelType");
            if required == "none" {
                selections.remove("Hair");
            } else if let Some(hair) = model(selections, "Hair") {
                if !required.is_empty() && hair.flag("HairModelType") != required {
                    let matching = lib
                        .models
                        .iter()
                        .filter(|(_, p)| {
                            p.slot == "Hair"
                                && p.flag("HairStyle") == hair.flag("HairStyle")
                                && p.flag("HairModelType") == required
                                && ["", gender, "unisex"].contains(&p.flag("Gender"))
                        })
                        .min_by_key(|(id, _)| *id);
                    if let Some((id, p)) = matching {
                        selections.insert(
                            "Hair".into(),
                            serde_json::json!({"asset_id":id,"material_id":p.materials[0]}),
                        );
                    } else {
                        selections.remove("Hair");
                    }
                }
            }
        }
        let removals: Vec<String> = selections
            .keys()
            .flat_map(|s| {
                model(selections, s).into_iter().flat_map(|p| {
                    p.flag("RequiresRemovalOfComponents")
                        .split(',')
                        .map(|s| s.trim().to_owned())
                })
            })
            .filter(|s| !s.is_empty())
            .collect();
        for s in removals {
            selections.remove(&s);
        }
        let shadow = selections
            .get("Hair")
            .and_then(|v| v["asset_id"].as_str())
            .and_then(|id| lib.models.get(id))
            .map(|p| {
                if p.flag("RequiresHairlineShadowStyle") == "on" {
                    "on"
                } else {
                    ""
                }
            })
            .unwrap_or("");
        let head = selections
            .get("Rostral")
            .and_then(|v| v["material_id"].as_str())
            .and_then(|id| lib.materials.get(id))
            .ok_or("Head unavailable")?;
        let tone = profile["skin"].as_str().unwrap_or(head.flag("SkinTone"));
        let beard = if gender == "female" {
            ""
        } else {
            profile["beard"]
                .as_str()
                .unwrap_or(head.flag("FacialHairStyle"))
        };
        for (slot, value) in selections.iter_mut() {
            let old_id = value["material_id"].as_str().unwrap();
            let old = &lib.materials[old_id];
            if old.flag("SkinTone").is_empty() {
                continue;
            }
            let part = &lib.models[value["asset_id"].as_str().unwrap()];
            let best = part
                .materials
                .iter()
                .filter_map(|id| lib.materials.get(id).map(|m| (id, m)))
                .filter(|(_, m)| {
                    m.flag("SkinTone") == tone
                        && (slot != "Rostral" || m.flag("FacialHairStyle") == beard)
                })
                .min_by_key(|(id, m)| {
                    let shadow_penalty =
                        if slot == "Rostral" && m.flag("HairlineShadowStyle") != shadow {
                            100
                        } else {
                            0
                        };
                    shadow_penalty
                        + if *id == old_id {
                            0
                        } else {
                            1 + old
                                .flags
                                .iter()
                                .filter(|(k, v)| {
                                    k.as_str() != "SkinTone"
                                        && k.as_str() != "FacialHairStyle"
                                        && m.flags.get(*k) != Some(*v)
                                })
                                .count()
                        }
                });
            let (id, _) = best.ok_or("This skin and facial hair combination is unavailable")?;
            value["material_id"] = Value::String(id.clone());
        }
        Ok(result)
    }
    /// Clone per outfit: remote colours and tattoos must never mutate another player.
    pub(crate) fn profile_material(
        &self,
        id: &str,
        mid: &str,
        profile: &Value,
        materials: &Assets<SkaterMaterial>,
    ) -> Option<SkaterMaterial> {
        let part = self.library.models.get(id)?;
        let source = self.library.materials.get(mid)?;
        let mut material = materials.get(&self.materials.get(mid)?.0)?.clone();
        let tint = if !source.flag("SkinTone").is_empty() {
            profile.get("skin_tint")
        } else if part.slot == "Hair" {
            profile.get("hair_tint")
        } else {
            profile["colours"].get(&part.slot)
        };
        let rgb = tint
            .and_then(|v| serde_json::from_value::<[f32; 3]>(v.clone()).ok())
            .unwrap_or(source.tint);
        material.base.base_color = Color::srgb(rgb[0], rgb[1], rgb[2]);
        let slot = if part.slot == "OuterTorso" && part.flag("TopType").is_empty() {
            "Arm"
        } else {
            &part.slot
        };
        let stamp = &profile["tattoos"][slot];
        let mut extension = SkinStamp {
            hair_opacity: material.extension.hair_opacity.clone(),
            retail: material.extension.retail.clone(),
            retail_mask: material.extension.retail_mask.clone(),
            enabled: if source.opacity.is_some() {
                Vec4::Y
            } else {
                Vec4::ZERO
            },
            ..default()
        };
        if extension.retail.tint.w > 0. {
            extension.retail.tint =
                Vec4::from_array(material.base.base_color.to_linear().to_f32_array());
        }
        if let Some((tid, t)) = stamp["id"]
            .as_str()
            .and_then(|id| self.library.tattoos.get(id).map(|t| (id, t)))
        {
            let key = match stamp["side"].as_u64().unwrap_or(0) {
                1 => "StampUVConstraintQ4",
                2 => "StampUVConstraintQ1",
                3 => "StampUVConstraintQ2",
                _ => "StampUVConstraintQ3",
            };
            let bounds: Vec<f32> = part
                .flag(key)
                .split(',')
                .filter_map(|n| n.parse().ok())
                .collect();
            if let Ok(bounds) = <[f32; 4]>::try_from(bounds) {
                let (transform, rectangle) =
                    crate::customiser_material::placement(bounds, t.bounds);
                extension.texture = self.tattoos.get(tid).cloned();
                extension.transform = transform;
                extension.rectangle = rectangle;
                extension.enabled.x = 1.;
            }
        }
        material.extension = extension;
        Some(material)
    }
    pub(crate) fn material_ready(&self, mid: &str, server: &AssetServer) -> bool {
        self.materials.get(mid).is_some_and(|(_, images)| {
            images
                .iter()
                .all(|h| server.is_loaded_with_dependencies(h.id()))
        })
    }
    pub(crate) fn tattoos_ready(&self, profile: &Value, server: &AssetServer) -> bool {
        profile["tattoos"]
            .as_object()
            .into_iter()
            .flat_map(|v| v.values())
            .all(|v| {
                v["id"].as_str().is_none_or(|id| {
                    self.tattoos
                        .get(id)
                        .is_some_and(|h| server.is_loaded_with_dependencies(h.id()))
                })
            })
    }
    pub fn warm(&mut self, id: &str, server: &AssetServer, materials: &mut Assets<SkaterMaterial>) {
        if self.materials.contains_key(id) {
            return;
        }
        let Some(m) = self.library.materials.get(id) else {
            return;
        };
        let mut images = vec![];
        let mut load = |path: &String, linear: bool| {
            let handle = server.load_with_settings(
                path.clone(),
                move |s: &mut bevy::image::ImageLoaderSettings| {
                    s.is_srgb = !linear;
                },
            );
            images.push(handle.clone());
            handle
        };
        let diffuse = load(&m.diffuse, false);
        let normal = m.normal.as_ref().map(|p| load(p, true));
        let rough = m.rough.as_ref().map(|p| load(p, true));
        let opacity = m.opacity.as_ref().map(|p| load(p, true));
        let retail_mask = m
            .lighting
            .as_ref()
            .and_then(|l| l.specular.as_ref())
            .map(|p| load(p, true));
        let retail = m
            .lighting
            .as_ref()
            .filter(|l| l.params.len() == 9)
            .map(|l| crate::retail_character::CharacterParams {
                tint: Vec4::from_array(
                    Color::srgb(m.tint[0], m.tint[1], m.tint[2])
                        .to_linear()
                        .to_f32_array(),
                ),
                options: Vec4::new(
                    f32::from(normal.is_some()),
                    f32::from(retail_mask.is_some()),
                    -1.,
                    f32::from(l.is_hair()),
                ),
                rows: std::array::from_fn(|i| Vec4::from_array(l.params[i])),
                ..default()
            })
            .unwrap_or_default();
        let material = materials.add(SkaterMaterial {
            base: StandardMaterial {
                base_color: Color::srgb(m.tint[0], m.tint[1], m.tint[2]),
                base_color_texture: Some(diffuse.clone()),
                normal_map_texture: normal.clone(),
                metallic_roughness_texture: rough,
                double_sided: m.opacity.is_some(),
                cull_mode: if m.opacity.is_some() {
                    None
                } else {
                    Some(bevy::render::render_resource::Face::Back)
                },
                metallic: m.metallic,
                perceptual_roughness: if m.rough.is_some() { 1. } else { m.roughness },
                alpha_mode: if m.alpha {
                    AlphaMode::Mask(0.5)
                } else {
                    AlphaMode::Opaque
                },
                ..default()
            },
            extension: SkinStamp {
                retail,
                retail_mask,
                hair_opacity: opacity,
                enabled: if m.opacity.is_some() {
                    Vec4::Y
                } else {
                    Vec4::ZERO
                },
                ..default()
            },
        });
        self.materials.insert(id.into(), (material, images));
    }
}
pub(crate) fn setup(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    server: Res<AssetServer>,
) {
    let directory = asset_directory(&config.asset_root);
    let library = std::fs::read(directory.join("library-v3.json"))
        .or_else(|_| std::fs::read(directory.join("library.json")))
        .ok()
        .and_then(|b| serde_json::from_slice::<Library>(&b).ok())
        .unwrap_or_default();
    let geometry = library
        .models
        .iter()
        .map(|(id, p)| (id.clone(), server.load(p.scene.clone())))
        .collect();
    let tattoos = library
        .tattoos
        .iter()
        .map(|(id, t)| {
            (
                id.clone(),
                server.load_with_settings(
                    t.texture.clone(),
                    |s: &mut bevy::image::ImageLoaderSettings| {
                        s.is_srgb = false;
                    },
                ),
            )
        })
        .collect();
    commands.insert_resource(Parts {
        tattoos,
        library,
        geometry,
        ..default()
    });
}
pub(crate) fn update(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut parts: ResMut<Parts>,
    mut state: ResMut<crate::customiser::Customiser>,
    mut materials: ResMut<Assets<SkaterMaterial>>,
    root: Query<Entity, With<crate::world::PlayerRoot>>,
    parents: Query<&ChildOf>,
    mut scenes: Query<
        (Entity, Option<&PartRoot>, &mut Visibility),
        (
            With<SceneRoot>,
            Without<crate::custom_models::CustomModelRoot>,
        ),
    >,
    mut mesh_materials: Query<(Entity, Option<&mut MeshMaterial3d<SkaterMaterial>>), With<Mesh3d>>,
    mut morphs: Query<(Entity, &mut MorphWeights)>,
    mut animation: ResMut<crate::animation::AnimationStatus>,
    custom_models: Res<crate::custom_models::CustomModels>,
) {
    if !state.enabled || custom_models.active.is_some() {
        return;
    }
    let Ok(root) = root.single() else {
        return;
    };
    let preview = state.preview(&parts);
    let Some(selections) = preview["selections"].as_object() else {
        return;
    };
    let desired: Vec<(String, String)> = selections
        .values()
        .filter_map(|v| {
            Some((
                v["asset_id"].as_str()?.into(),
                v["material_id"].as_str()?.into(),
            ))
        })
        .collect();
    // A missing/failed library is not a valid empty outfit. Never hide the
    // fallback skater until a complete replacement can actually be published.
    if desired.is_empty() || parts.resolve(&preview).is_err() {
        let message = "Character assets are unavailable. Update this copy's character assets; your skater is unchanged.";
        if state.status != message {
            state.status = message.into();
            state.redraw = true;
        }
        return;
    }
    if parts.applied == preview && !state.open {
        return;
    }
    // Resolve the visible choices ahead of selection, including skin, head,
    // sleeves, and sock dependencies. No conversion process runs in the game.
    let precache: HashSet<(String, String)> = state
        .preload_outfits(&parts)
        .iter()
        .flat_map(|p| {
            p["selections"]
                .as_object()
                .into_iter()
                .flat_map(|s| s.values())
        })
        .filter_map(|v| {
            Some((
                v["asset_id"].as_str()?.to_owned(),
                v["material_id"].as_str()?.to_owned(),
            ))
        })
        .chain(desired.iter().cloned())
        .collect();
    for (_, mid) in &precache {
        parts.warm(mid, &server, &mut materials);
    }
    let precache: HashSet<String> = precache.into_iter().map(|(id, _)| id).collect();
    for id in precache {
        if parts.instances.contains_key(&id) {
            continue;
        }
        if let Some(handle) = parts.geometry.get(&id) {
            if server.is_loaded_with_dependencies(handle.id()) {
                let scene = parts.library.models[&id].scene.clone();
                let entity = commands
                    .spawn((
                        PartRoot(id.clone()),
                        Visibility::Hidden,
                        SceneRoot(server.load(GltfAssetLabel::Scene(0).from_asset(scene))),
                    ))
                    .id();
                commands.entity(root).add_child(entity);
                parts.instances.insert(id, entity);
            }
        }
    }
    if parts.applied == preview {
        return;
    }
    let mut ready = true;
    for (id, mid) in &desired {
        parts.warm(mid, &server, &mut materials);
        if !parts.instances.contains_key(id) {
            ready = false;
        }
        let part = &parts.library.models[id];
        let slot = if part.slot == "OuterTorso" && part.flag("TopType").is_empty() {
            "Arm"
        } else {
            &part.slot
        };
        if let Some(tattoo) = state.draft["tattoos"][slot]["id"].as_str() {
            if parts
                .tattoos
                .get(tattoo)
                .is_none_or(|h| !server.is_loaded_with_dependencies(h.id()))
            {
                ready = false;
            }
        }
        if !parts.materials.get(mid).is_some_and(|(_, images)| {
            images
                .iter()
                .all(|h| server.is_loaded_with_dependencies(h.id()))
        }) {
            ready = false;
        }
        if let Some(&entity) = parts.instances.get(id) {
            if !mesh_materials
                .iter()
                .any(|(e, _)| parents.iter_ancestors(e).any(|p| p == entity))
            {
                ready = false;
            }
        }
    }
    if !ready {
        return;
    }
    let wanted: HashSet<_> = desired.iter().map(|(id, _)| id.as_str()).collect();
    let mut rebind = false;
    for (entity, part, mut visible) in &mut scenes {
        if !parents.get(entity).is_ok_and(|p| p.parent() == root) {
            continue;
        }
        let target = if part.is_some_and(|p| wanted.contains(p.0.as_str())) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visible != target {
            *visible = target;
            rebind = true;
        }
    }
    for (id, mid) in &desired {
        let entity = parts.instances[id];
        let handle = parts.materials[mid].0.clone();
        if let Some(updated) = parts.profile_material(id, mid, &state.draft, &materials) {
            if let Some(material) = materials.get_mut(&handle) {
                *material = updated;
            }
        }
        for (e, material) in &mut mesh_materials {
            if !parents.iter_ancestors(e).any(|p| p == entity) {
                continue;
            }
            commands
                .entity(e)
                .insert(bevy::camera::visibility::RenderLayers::from_layers(&[
                    0, 28,
                ]));
            if let Some(mut material) = material {
                if material.0 != handle {
                    material.0 = handle.clone();
                }
            } else {
                commands
                    .entity(e)
                    .remove::<MeshMaterial3d<StandardMaterial>>()
                    .insert(MeshMaterial3d(handle.clone()));
            }
        }
    }
    let weights: Vec<f32> = parts
        .library
        .morphs
        .iter()
        .enumerate()
        .map(|(i, n)| {
            state.draft["morphs"][n]
                .as_f64()
                .unwrap_or(if (2..19).contains(&i) { 0.25 } else { 0. })
                .clamp(0., 0.5) as f32
        })
        .collect();
    for (entity, mut morph) in &mut morphs {
        if parents
            .iter_ancestors(entity)
            .any(|p| parts.instances.values().any(|e| *e == p))
            && morph.weights().len() == weights.len()
            && morph.weights() != weights
        {
            morph.weights_mut().copy_from_slice(&weights);
        }
    }
    if rebind {
        *animation = default();
    }
    if parts.applied != preview {
        parts.applied = preview;
        state.status.clear();
        state.redraw = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_library_choices_resolve_without_conversion() {
        let Ok(path) = std::env::var("SKATE_CAC_TEST_LIBRARY") else {
            return;
        };
        let library: Library = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let parts = Parts {
            library,
            ..default()
        };
        let mut failures = vec![];
        for gender in ["male", "female"] {
            let base = &parts.library.defaults[gender];
            assert!(
                parts.resolve(base).is_ok(),
                "{gender}: {:?}",
                parts.resolve(base)
            );
            for (id, part) in &parts.library.models {
                if !["", gender, "unisex"].contains(&part.flag("Gender")) {
                    continue;
                }
                if ["Arm", "Leg", "Organ", "Rostral", "InnerTorso"].contains(&part.slot.as_str()) {
                    continue;
                }
                for mid in &part.materials {
                    let mut p = base.clone();
                    p["selections"][&part.slot] =
                        serde_json::json!({"asset_id":id,"material_id":mid});
                    if let Err(e) = parts.resolve(&p) {
                        failures.push(format!("{gender} {} {}: {e}", part.slot, part.name));
                    }
                }
            }
            for tone in ["light", "dark"] {
                let mut p = base.clone();
                p["skin"] = serde_json::json!(tone);
                if let Err(e) = parts.resolve(&p) {
                    failures.push(format!("{gender} {tone}: {e}"));
                }
            }
        }
        failures.sort();
        failures.dedup();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
#[cfg(test)]
mod online_tests {
    use super::*;
    #[test]
    fn online_appearance_materials_keep_players_colours_and_tattoos_independent() {
        let mut parts = Parts::default();
        parts.library.models.insert(
            "body".into(),
            Part {
                slot: "Arm".into(),
                name: "body".into(),
                flags: HashMap::from([("StampUVConstraintQ3".into(), "0,0,1,1".into())]),
                materials: vec!["skin".into()],
                scene: String::new(),
            },
        );
        parts.library.materials.insert(
            "skin".into(),
            Material {
                name: "skin".into(),
                flags: HashMap::from([("SkinTone".into(), "fair".into())]),
                diffuse: String::new(),
                normal: None,
                rough: None,
                alpha: false,
                opacity: None,
                tint: [1.; 3],
                metallic: 0.,
                roughness: 0.5,
                lighting: None,
            },
        );
        parts.library.tattoos.insert(
            "ink".into(),
            Tattoo {
                name: "ink".into(),
                texture: String::new(),
                bounds: [0., 0., 1., 1.],
            },
        );
        let mut images = Assets::<Image>::default();
        let image = images.add(Image::default());
        parts.tattoos.insert("ink".into(), image.clone());
        let mut materials = Assets::<SkaterMaterial>::default();
        let handle = materials.add(SkaterMaterial {
            base: StandardMaterial {
                normal_map_texture: Some(image.clone()),
                base_color_texture: Some(image.clone()),
                ..default()
            },
            extension: SkinStamp {
                retail_mask: Some(image.clone()),
                retail: crate::retail_character::CharacterParams {
                    tint: Vec4::ONE,
                    options: Vec4::new(1., 1., -1., 0.),
                    ..default()
                },
                ..default()
            },
        });
        parts
            .materials
            .insert("skin".into(), (handle.clone(), vec![]));
        let red = parts
            .profile_material(
                "body",
                "skin",
                &serde_json::json!({"skin_tint":[1,0,0],"tattoos":{"Arm":{"id":"ink","side":0}}}),
                &materials,
            )
            .unwrap();
        let blue = parts
            .profile_material(
                "body",
                "skin",
                &serde_json::json!({"skin_tint":[0,0,1]}),
                &materials,
            )
            .unwrap();
        assert_eq!(red.base.base_color, Color::srgb(1., 0., 0.));
        assert_eq!(blue.base.base_color, Color::srgb(0., 0., 1.));
        for material in [&red, &blue] {
            assert_eq!(material.base.normal_map_texture, Some(image.clone()));
            assert_eq!(material.base.base_color_texture, Some(image.clone()));
            assert_eq!(material.extension.retail_mask, Some(image.clone()));
            assert_eq!(
                material.extension.retail.options,
                Vec4::new(1., 1., -1., 0.)
            );
            assert_eq!(
                material.extension.retail.tint,
                Vec4::from_array(material.base.base_color.to_linear().to_f32_array())
            );
        }
        assert_eq!(red.extension.texture, Some(image));
        assert!(blue.extension.texture.is_none());
        assert_eq!(
            materials.get(&handle).unwrap().base.base_color,
            Color::WHITE
        );
        assert!(materials.get(&handle).unwrap().extension.texture.is_none());
    }
}

#[cfg(test)]
mod stock_hair_audit {
    use super::*;
    #[test]
    #[ignore = "requires prepared owned SKATE_CAC_TEST_LIBRARY"]
    fn stock_hair_keeps_authored_shading_and_textures_after_colour_restore() {
        let path = std::env::var("SKATE_CAC_TEST_LIBRARY").unwrap();
        let library: Library = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let pairs: Vec<_> = library
            .models
            .iter()
            .filter(|(_, p)| p.slot == "Hair")
            .flat_map(|(id, p)| p.materials.iter().map(move |mid| (id.clone(), mid.clone())))
            .collect();
        assert!(!pairs.is_empty());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<Image>();
        let server = app.world().resource::<AssetServer>();
        let mut parts = Parts {
            library,
            ..default()
        };
        let mut materials = Assets::<SkaterMaterial>::default();
        let mut checked = HashSet::new();
        for (id, mid) in pairs {
            parts.warm(&mid, server, &mut materials);
            let handle = parts.materials[&mid].0.clone();
            let original = materials.get(&handle).unwrap().clone();
            assert_eq!(
                original.extension.retail.options.w, 1.,
                "{mid} must use hair lighting"
            );
            let dyed = parts
                .profile_material(
                    &id,
                    &mid,
                    &serde_json::json!({"hair_tint":[1,0,0]}),
                    &materials,
                )
                .unwrap();
            *materials.get_mut(&handle).unwrap() = dyed;
            let restored = parts
                .profile_material(&id, &mid, &serde_json::json!({}), &materials)
                .unwrap();
            assert_eq!(restored.base.base_color, original.base.base_color);
            assert_eq!(
                restored.extension.retail.tint,
                original.extension.retail.tint
            );
            assert_eq!(
                restored.extension.retail.options,
                original.extension.retail.options
            );
            assert_eq!(
                restored.base.base_color_texture,
                original.base.base_color_texture
            );
            assert_eq!(
                restored.base.normal_map_texture,
                original.base.normal_map_texture
            );
            assert_eq!(
                restored.extension.hair_opacity,
                original.extension.hair_opacity
            );
            assert_eq!(
                restored.extension.retail_mask,
                original.extension.retail_mask
            );
            assert_eq!(
                restored.base.perceptual_roughness,
                original.base.perceptual_roughness
            );
            *materials.get_mut(&handle).unwrap() = restored;
            checked.insert(mid);
        }
        eprintln!(
            "STOCK_HAIR_AUDIT verified {} authored materials",
            checked.len()
        );
    }
}
