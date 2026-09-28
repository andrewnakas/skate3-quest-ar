//! Character SH/key/rim shader, independent of the world's additive lights.
use crate::retail_irradiance::Irradiance;
use bevy::{
    asset::embedded_asset,
    camera::visibility::RenderLayers,
    gltf::GltfMaterialName,
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use serde::Deserialize;
use std::collections::HashMap;

pub(crate) struct CharacterLightingPlugin;
impl Plugin for CharacterLightingPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "retail_character.wgsl");
        embedded_asset!(app, "retail_character_depth.wgsl");
        embedded_asset!(app, "retail_character_common.wgsl");
        let common: Handle<Shader> = bevy::asset::load_embedded_asset!(
            app.world().resource::<AssetServer>(),
            "retail_character_common.wgsl"
        );
        app.insert_resource(CharacterShader(common));
        app.add_plugins(MaterialPlugin::<CharacterMaterial>::default())
            .add_systems(Startup, load)
            .add_systems(
                PreUpdate,
                load.after(crate::map_transition::MapTransitionSet)
                    .run_if(crate::retail_render::world_changed),
            )
            .add_systems(Update, (bind, shadow_views, update).chain());
    }
}
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct MaterialData {
    pub shader: String,
    pub params: Vec<[f32; 4]>,
    pub specular: Option<String>,
}
impl MaterialData {
    pub(crate) fn is_hair(&self) -> bool {
        matches!(
            self.shader.as_str(),
            "character.hair"
                | "character.hair_ropa"
                | "character.default_hair"
                | "character.default_hair_ropa"
        )
    }
}

#[derive(Resource)]
struct CharacterShader(#[allow(dead_code)] Handle<Shader>);
#[derive(Deserialize)]
struct LightingData {
    materials: HashMap<String, MaterialData>,
    default_sh: [[f32; 3]; 9],
    #[serde(default)]
    native: HashMap<String, HashMap<String, MaterialData>>,
}
#[derive(Resource)]
struct Lighting {
    data: LightingData,
    probes: Irradiance,
    light: Vec4,
    display_sh: Option<[Vec4; 9]>,
}
#[derive(Clone, Debug, Default, ShaderType)]
pub(crate) struct CharacterParams {
    pub light: Vec4,
    pub tint: Vec4,
    // normal map present, dedicated specular mask, alpha cutoff, hair family
    pub options: Vec4,
    pub rows: [Vec4; 9],
    pub sh: [Vec4; 9],
}
#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct CharacterMaterial {
    #[uniform(0)]
    params: CharacterParams,
    #[texture(1)]
    #[sampler(2)]
    diffuse: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    normal: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    mask: Option<Handle<Image>>,
    alpha: AlphaMode,
}
impl Material for CharacterMaterial {
    fn fragment_shader() -> ShaderRef {
        bevy::asset::AssetPath::from(bevy::asset::embedded_path!("retail_character.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        bevy::asset::AssetPath::from(bevy::asset::embedded_path!("retail_character_depth.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
}
#[derive(Component)]
struct ShadowSource;

#[derive(Component)]
struct OriginalCharacterMaterial {
    material: Handle<StandardMaterial>,
    layers: Option<RenderLayers>,
}
fn load(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    sources: Query<Entity, With<ShadowSource>>,
    mut shadow: ResMut<crate::retail_render::ShadowState>,
    retail: Res<crate::retail_render::RetailScene>,
    originals: Query<(Entity, &OriginalCharacterMaterial)>,
    cameras: Query<(Entity, &RenderLayers), With<crate::camera::GameplayCamera>>,
    mut customiser: ResMut<Assets<crate::customiser_material::SkaterMaterial>>,
) {
    for (_, material) in customiser.iter_mut() {
        material.extension.retail.light = Vec4::ZERO;
    }
    commands.remove_resource::<Lighting>();
    for e in &sources {
        commands.entity(e).despawn();
    }
    *shadow = Default::default();
    for (entity, original) in &originals {
        let mut entity = commands.entity(entity);
        entity
            .remove::<(MeshMaterial3d<CharacterMaterial>, OriginalCharacterMaterial)>()
            .insert(MeshMaterial3d(original.material.clone()));
        if let Some(layers) = &original.layers {
            entity.insert(layers.clone());
        } else {
            entity.remove::<RenderLayers>();
        }
    }
    for (entity, layers) in &cameras {
        commands.entity(entity).insert(layers.clone().without(28));
    }
    if !retail.0 {
        return;
    }
    let Some(map_path) = &config.map_path else {
        return;
    };
    let path = config.asset_root.join("private/character-lighting.json");
    let mut data = match std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice::<LightingData>(&b).map_err(|e| e.to_string()))
    {
        Ok(data) => data,
        Err(e) => {
            warn!("SKATE_CHARACTER_LIGHTING: {e}");
            return;
        }
    };
    data.native = std::fs::read(
        crate::customiser_parts::asset_directory(&config.asset_root).join("native-lighting.json"),
    )
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .unwrap_or_default();
    let name = map_path.file_stem().unwrap_or_default().to_string_lossy();
    let overlay = config
        .asset_root
        .join("private/native-lighting")
        .join(format!("{name}.irradiance"));
    let probe_path = if overlay.is_file() {
        overlay
    } else {
        map_path.with_extension("irradiance")
    };
    let probes = match std::fs::read(probe_path)
        .map_err(|e| e.to_string())
        .and_then(|b| Irradiance::parse(&b))
    {
        Ok(probes) => probes,
        Err(e) => {
            warn!("SKATE_CHARACTER_LIGHTING: spatial data unavailable: {e}");
            return;
        }
    };
    let sky_path = config
        .asset_root
        .join("private/native-skies")
        .join(format!("{name}.json"));
    let sky: serde_json::Value = match std::fs::read(sky_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(sky) => sky,
        None => {
            warn!("SKATE_CHARACTER_LIGHTING: missing authored light direction");
            return;
        }
    };
    let Some(sun) = sky["environment"]["sun_direction"].as_array() else {
        return;
    };
    if sun.len() != 3 {
        return;
    }
    let mut light = Vec4::new(0., 0., 0., 2.5); // Existing retail scene exposure.
    for i in 0..3 {
        let Some(v) = sun[i].as_f64() else {
            return;
        };
        light[i] = v as f32;
    }
    info!("SKATE_CHARACTER_LIGHTING: loaded authored {name} irradiance and character parameters");
    spawn_shadow_sources(&mut commands, light.truncate());
    commands.insert_resource(Lighting {
        data,
        probes,
        light,
        display_sh: None,
    });
}
fn spawn_shadow_sources(commands: &mut Commands, light: Vec3) {
    // World + skater casters, sampled only by the character shader.
    commands.spawn((
        ShadowSource,
        Name::new("Character shadow visibility"),
        DirectionalLight {
            illuminance: 0.,
            shadows_enabled: true,
            affects_lightmapped_mesh_diffuse: false,
            ..default()
        },
        Transform::default().looking_to(-light, Vec3::Y),
        bevy::light::CascadeShadowConfigBuilder {
            maximum_distance: 100.,
            first_cascade_far_bound: 10.,
            ..default()
        }
        .build(),
    ));
    // Only skinned player pieces inhabit layer 28. The world receiver samples
    // this separate map so baked building/terrain shadows are not re-applied.
    commands.spawn((
        ShadowSource,
        Name::new("Player shadow onto baked world"),
        DirectionalLight {
            illuminance: 0.,
            shadows_enabled: true,
            affects_lightmapped_mesh_diffuse: true,
            // Receiver-only map: no self-shadow acne to hide with large bias.
            shadow_depth_bias: 0.002,
            shadow_normal_bias: 0.0,
            ..default()
        },
        RenderLayers::layer(28),
        Transform::default().looking_to(-light, Vec3::Y),
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 1,
            maximum_distance: 24.,
            ..default()
        }
        .build(),
    ));
}

fn bind(
    mut commands: Commands,
    lighting: Option<Res<Lighting>>,
    source: Res<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    entities: Query<(
        Entity,
        &GltfMaterialName,
        &MeshMaterial3d<StandardMaterial>,
        Option<&RenderLayers>,
    )>,
    parents: Query<&ChildOf>,
    players: Query<
        (),
        Or<(
            With<crate::world::PlayerRoot>,
            With<crate::multiplayer::appearance::RemoteCharacter>,
        )>,
    >,
    parts: Query<(), With<crate::customiser_parts::PartRoot>>,
    native: Query<&crate::custom_models::NativeModelRoot>,
    imports: Query<(), With<crate::custom_models::CustomModelRoot>>,
) {
    let Some(lighting) = lighting else {
        return;
    };
    for (entity, name, handle, layers) in &entities {
        if !parents.iter_ancestors(entity).any(|e| players.contains(e)) {
            continue;
        }
        // Modular CAC pieces own an extended material with tattoos/hair coverage.
        // Giving them a second material races publication and can render twice.
        if parents.iter_ancestors(entity).any(|e| parts.contains(e)) {
            continue;
        }
        let native_key = parents
            .iter_ancestors(entity)
            .find_map(|e| native.get(e).ok().map(|n| &n.0));
        if native_key.is_none() && parents.iter_ancestors(entity).any(|e| imports.contains(e)) {
            continue;
        }
        let table = if let Some(key) = native_key {
            lighting.data.native.get(key)
        } else {
            Some(&lighting.data.materials)
        };
        let Some(data) = table.and_then(|m| m.get(&name.0)) else {
            continue;
        };
        if data.params.len() != 9 {
            continue;
        }
        let Some(m) = source.get(&handle.0) else {
            continue;
        };
        let alpha_cutoff = if let AlphaMode::Mask(cutoff) = m.alpha_mode {
            cutoff
        } else {
            -1.
        };
        let material = materials.add(CharacterMaterial {
            params: CharacterParams {
                light: lighting.light,
                tint: Vec4::from_array(m.base_color.to_linear().to_f32_array()),
                options: Vec4::new(
                    f32::from(m.normal_map_texture.is_some()),
                    f32::from(data.specular.is_some()),
                    alpha_cutoff,
                    f32::from(data.is_hair()),
                ),
                rows: std::array::from_fn(|i| Vec4::from_array(data.params[i])),
                sh: lighting
                    .data
                    .default_sh
                    .map(|v| Vec3::from_array(v).extend(0.)),
            },
            diffuse: m.base_color_texture.clone(),
            normal: m.normal_map_texture.clone(),
            mask: data.specular.as_ref().map(|path| {
                server.load_with_settings(
                    path.clone(),
                    |settings: &mut bevy::image::ImageLoaderSettings| {
                        settings.is_srgb = false;
                    },
                )
            }),
            alpha: m.alpha_mode,
        });
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((
                OriginalCharacterMaterial {
                    material: handle.0.clone(),
                    layers: layers.cloned(),
                },
                MeshMaterial3d(material),
                RenderLayers::from_layers(&[0, 28]),
            ));
    }
}
fn shadow_views(
    mut commands: Commands,
    lighting: Option<Res<Lighting>>,
    cameras: Query<(Entity, Option<&RenderLayers>), With<crate::camera::GameplayCamera>>,
) {
    if lighting.is_none() {
        return;
    }
    for (entity, layers) in &cameras {
        let layers = layers.cloned().unwrap_or_default();
        if !layers.intersects(&RenderLayers::layer(28)) {
            commands.entity(entity).insert(layers.with(28));
        }
    }
}
fn update(
    lighting: Option<ResMut<Lighting>>,
    root: Query<&Transform, With<crate::world::PlayerRoot>>,
    mut materials: ResMut<Assets<CharacterMaterial>>,
    mut customiser: ResMut<Assets<crate::customiser_material::SkaterMaterial>>,
    mut shadow: ResMut<crate::retail_render::ShadowState>,
    time: Res<Time>,
    mut changed: Local<Vec<AssetId<CharacterMaterial>>>,
) {
    let (Some(mut lighting), Ok(root)) = (lighting, root.single()) else {
        return;
    };
    let fallback = lighting
        .data
        .default_sh
        .map(|v| Vec3::from_array(v).extend(0.));
    let sh = lighting.probes.sample(root.translation, fallback);
    let displayed = lighting.display_sh.map_or(sh, |old| {
        let weight = 1. - (-time.delta_secs().clamp(0., 0.05) / 0.35).exp();
        std::array::from_fn(|i| old[i].lerp(sh[i], weight))
    });
    publish_sh(&mut materials, displayed, &mut changed);
    for (_, material) in customiser.iter_mut() {
        if material.extension.retail.tint.w == 0. {
            continue;
        }
        material.extension.retail.light = lighting.light;
        material.extension.retail.sh = displayed;
    }
    lighting.display_sh = Some(displayed);
    // Adapter floor: the local probe's direction-independent ambient term.
    // The native per-frame c8 shadow-colour controller remains unrecovered.
    shadow.approach(sh[0].truncate(), time.delta_secs());
}

fn publish_sh(
    materials: &mut Assets<CharacterMaterial>,
    displayed: [Vec4; 9],
    changed: &mut Vec<AssetId<CharacterMaterial>>,
) {
    // iter_mut emits Modified even for identical assignments, rebuilding GPU
    // material bindings. Read first and mutate only bitwise-different SH payloads.
    // Check every material so newly bound pieces still receive the current SH.
    changed.clear();
    changed.extend(materials.iter().filter_map(|(id, material)| {
        (!material
            .params
            .sh
            .iter()
            .zip(&displayed)
            .all(|(a, b)| a.to_array().map(f32::to_bits) == b.to_array().map(f32::to_bits)))
        .then_some(id)
    }));
    for &id in changed.iter() {
        materials
            .get_mut(id)
            .expect("material enumerated in this call")
            .params
            .sh = displayed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn customiser_hair_families_include_pro_skater_defaults() {
        for shader in [
            "character.hair",
            "character.hair_ropa",
            "character.default_hair",
            "character.default_hair_ropa",
        ] {
            assert!(
                MaterialData {
                    shader: shader.into(),
                    params: vec![],
                    specular: None
                }
                .is_hair(),
                "{shader}"
            );
        }
        for shader in [
            "character.default_skin",
            "character.default_cloth",
            "character.default_cloth_ropa",
        ] {
            assert!(
                !MaterialData {
                    shader: shader.into(),
                    params: vec![],
                    specular: None
                }
                .is_hair(),
                "{shader}"
            );
        }
    }
    #[test]
    fn customiser_material_binding_uses_native_rows_and_leaves_cac_material_owned() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<CharacterMaterial>>();
        let world = app.world_mut();
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let authored = MaterialData {
            shader: "character.cloth".into(),
            params: vec![[2., 3., 4., 5.]; 9],
            specular: None,
        };
        world.insert_resource(Lighting {
            data: LightingData {
                materials: HashMap::new(),
                default_sh: [[0.; 3]; 9],
                native: HashMap::from([(
                    "pro".into(),
                    HashMap::from([("Retail_Torso".into(), authored)]),
                )]),
            },
            probes: default(),
            light: Vec4::ONE,
            display_sh: None,
        });
        let player = world.spawn(crate::world::PlayerRoot).id();
        let pro = world
            .spawn((
                crate::custom_models::NativeModelRoot("pro".into()),
                ChildOf(player),
            ))
            .id();
        let torso = world
            .spawn((
                ChildOf(pro),
                GltfMaterialName("Retail_Torso".into()),
                MeshMaterial3d(material.clone()),
            ))
            .id();
        let part = world
            .spawn((
                crate::customiser_parts::PartRoot("shirt".into()),
                ChildOf(player),
            ))
            .id();
        let cloth = world
            .spawn((
                ChildOf(part),
                GltfMaterialName("Retail_Torso".into()),
                MeshMaterial3d(material),
            ))
            .id();
        let hair_data = MaterialData {
            shader: "character.default_hair".into(),
            params: vec![[1.; 4]; 9],
            specular: None,
        };
        world
            .resource_mut::<Lighting>()
            .data
            .native
            .get_mut("pro")
            .unwrap()
            .insert("Retail_Hair".into(), hair_data);
        let hair_material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let hair = world
            .spawn((
                ChildOf(pro),
                GltfMaterialName("Retail_Hair".into()),
                MeshMaterial3d(hair_material),
            ))
            .id();
        world.run_system_once(bind).unwrap();
        let hair_handle = &world
            .get::<MeshMaterial3d<CharacterMaterial>>(hair)
            .unwrap()
            .0;
        assert_eq!(
            world
                .resource::<Assets<CharacterMaterial>>()
                .get(hair_handle)
                .unwrap()
                .params
                .options
                .w,
            1.
        );

        let h = &world
            .get::<MeshMaterial3d<CharacterMaterial>>(torso)
            .unwrap()
            .0;
        assert_eq!(
            world
                .resource::<Assets<CharacterMaterial>>()
                .get(h)
                .unwrap()
                .params
                .rows[0],
            Vec4::new(2., 3., 4., 5.)
        );
        assert!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(torso)
                .is_none()
        );
        assert!(
            world
                .get::<MeshMaterial3d<CharacterMaterial>>(cloth)
                .is_none()
        );
        assert!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(cloth)
                .is_some()
        );
    }

    #[test]
    fn custom_scene_restores_character_and_removes_only_gameplay_shadow_layer() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(crate::config::Config {
            asset_root: "unused".into(),
            verification_capture: None,
            map: None,
            map_path: None,
            difficulty: crate::difficulty::Difficulty::Hardcore,
            check_assets: false,
            start_paused: false,
            teleport: None,
            multiplayer: Default::default(),
            map_fingerprint: 0,
        });
        world.insert_resource(crate::retail_render::RetailScene(false));
        world.init_resource::<Assets<crate::customiser_material::SkaterMaterial>>();
        world.insert_resource(crate::retail_render::ShadowState(
            Vec4::ONE,
            Vec4::ONE,
            [Vec4::ONE; 7],
        ));
        let material = Handle::<StandardMaterial>::default();
        let player = world
            .spawn((
                OriginalCharacterMaterial {
                    material: material.clone(),
                    layers: None,
                },
                MeshMaterial3d(Handle::<CharacterMaterial>::default()),
                RenderLayers::from_layers(&[0, 28]),
            ))
            .id();
        let light = world.spawn(ShadowSource).id();
        let camera = world
            .spawn((
                crate::camera::GameplayCamera,
                RenderLayers::from_layers(&[0, 28]),
            ))
            .id();
        let overlay = world.spawn(RenderLayers::layer(31)).id();
        world.run_system_once(load).unwrap();
        assert!(world.get_entity(light).is_err());
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(player)
                .unwrap()
                .0,
            material
        );
        assert!(
            world
                .get::<MeshMaterial3d<CharacterMaterial>>(player)
                .is_none()
        );
        assert!(world.get::<RenderLayers>(player).is_none());
        assert_eq!(
            world.get::<RenderLayers>(camera).unwrap(),
            &RenderLayers::default()
        );
        assert_eq!(
            world.get::<RenderLayers>(overlay).unwrap(),
            &RenderLayers::layer(31)
        );
        assert_eq!(
            world.resource::<crate::retail_render::ShadowState>().0,
            Vec4::ZERO
        );
    }

    #[test]
    fn unchanged_sh_emits_no_asset_modification_but_new_values_and_pieces_do() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<CharacterMaterial>();
        fn material() -> CharacterMaterial {
            CharacterMaterial {
                params: CharacterParams {
                    light: Vec4::ZERO,
                    tint: Vec4::ONE,
                    options: Vec4::ZERO,
                    rows: [Vec4::ZERO; 9],
                    sh: [Vec4::ZERO; 9],
                },
                diffuse: None,
                normal: None,
                mask: None,
                alpha: AlphaMode::Opaque,
            }
        }
        let first = app
            .world_mut()
            .resource_mut::<Assets<CharacterMaterial>>()
            .add(material());
        app.update();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<CharacterMaterial>>>()
            .clear();
        let mut scratch = Vec::new();
        let mut displayed = [Vec4::ZERO; 9];
        publish_sh(
            &mut app.world_mut().resource_mut::<Assets<CharacterMaterial>>(),
            displayed,
            &mut scratch,
        );
        assert!(scratch.is_empty());
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<AssetEvent<CharacterMaterial>>>()
                .drain()
                .count(),
            0
        );
        // Signed zero and NaN payloads must retain their bits, without epsilon tests.
        displayed[0] = Vec4::new(-0.0, f32::from_bits(0x7fc01234), 1.0, 0.0);
        publish_sh(
            &mut app.world_mut().resource_mut::<Assets<CharacterMaterial>>(),
            displayed,
            &mut scratch,
        );
        assert_eq!(scratch, vec![first.id()]);
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<Messages<AssetEvent<CharacterMaterial>>>()
                .drain()
                .any(|e| matches!(e,AssetEvent::Modified{id} if id==first.id()))
        );
        publish_sh(
            &mut app.world_mut().resource_mut::<Assets<CharacterMaterial>>(),
            displayed,
            &mut scratch,
        );
        assert!(scratch.is_empty());
        let second = app
            .world_mut()
            .resource_mut::<Assets<CharacterMaterial>>()
            .add(material());
        publish_sh(
            &mut app.world_mut().resource_mut::<Assets<CharacterMaterial>>(),
            displayed,
            &mut scratch,
        );
        assert_eq!(scratch, vec![second.id()]);
        let assets = app.world().resource::<Assets<CharacterMaterial>>();
        for handle in [&first, &second] {
            assert_eq!(
                assets.get(handle).unwrap().params.sh[0]
                    .to_array()
                    .map(f32::to_bits),
                displayed[0].to_array().map(f32::to_bits)
            );
        }
    }

    #[test]
    fn world_receiver_source_excludes_world_casters_and_emits_no_light() {
        let mut world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        spawn_shadow_sources(&mut Commands::new(&mut queue, &world), Vec3::Y);
        queue.apply(&mut world);
        let player = RenderLayers::from_layers(&[0, 28]);
        let terrain = RenderLayers::default();
        let mut receivers = 0;
        let mut character_sources = 0;
        for (light, layers, cascades) in world
            .query::<(
                &DirectionalLight,
                Option<&RenderLayers>,
                &bevy::light::CascadeShadowConfig,
            )>()
            .iter(&world)
        {
            let layers = layers.cloned().unwrap_or_default();
            assert_eq!(light.illuminance, 0.);
            assert!(light.shadows_enabled);
            assert!(layers.intersects(&player));
            if light.affects_lightmapped_mesh_diffuse {
                assert!(!layers.intersects(&terrain));
                assert_eq!(cascades.bounds, vec![24.]);
                assert_eq!(light.shadow_normal_bias, 0.);
                assert!(light.shadow_depth_bias < 0.005);
                receivers += 1;
            } else {
                assert!(layers.intersects(&terrain));
                character_sources += 1;
            }
        }
        assert_eq!((receivers, character_sources), (1, 1));
    }
}
