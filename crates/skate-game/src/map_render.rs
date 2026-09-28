//! CPU scene construction can run off-thread using the live asset allocators.
//! Only publishing assets/entities and retiring the previous scene touch the ECS.
use crate::retail_render::{RetailSkyMaterial, RetailWorldMaterial};
use bevy::{
    asset::{AssetHandleProvider, AssetId},
    ecs::world::CommandQueue,
    prelude::*,
};

#[derive(Component)]
pub(crate) struct MapEntity;

pub(crate) trait AssetSink<A: Asset> {
    fn add(&mut self, asset: A) -> Handle<A>;
}
impl<A: Asset> AssetSink<A> for Assets<A> {
    fn add(&mut self, asset: A) -> Handle<A> {
        Assets::add(self, asset)
    }
}

pub(crate) struct StagedAssets<A: Asset> {
    provider: AssetHandleProvider,
    values: Vec<(Handle<A>, A)>,
}
impl<A: Asset> StagedAssets<A> {
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (AssetId<A>, &mut A)> {
        self.values.iter_mut().map(|(h, a)| (h.id(), a))
    }
    fn new(world: &World) -> Self {
        Self {
            provider: world.resource::<Assets<A>>().get_handle_provider(),
            values: Vec::new(),
        }
    }
    fn publish(self, world: &mut World) -> Vec<AssetId<A>> {
        let mut assets = world.resource_mut::<Assets<A>>();
        self.values
            .into_iter()
            .map(|(handle, asset)| {
                let id = handle.id();
                // Reserved from this Assets' allocator; the staged strong handle is
                // alive throughout preparation, so its generation cannot be reused.
                assets
                    .insert(id, asset)
                    .expect("reserved map asset generation");
                id
            })
            .collect()
    }
}
impl<A: Asset> AssetSink<A> for StagedAssets<A> {
    fn add(&mut self, asset: A) -> Handle<A> {
        let handle = self.provider.reserve_handle().typed::<A>();
        self.values.push((handle.clone(), asset));
        handle
    }
}

/// Deferred spawns allocate their ECS identities only when the scene commits.
#[derive(Default)]
pub(crate) struct SceneCommands(CommandQueue);
pub(crate) struct SceneEntity<'a> {
    commands: &'a mut CommandQueue,
    id: std::sync::Arc<std::sync::OnceLock<Entity>>,
}
impl SceneCommands {
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> SceneEntity<'_> {
        let id = std::sync::Arc::new(std::sync::OnceLock::new());
        let spawned = id.clone();
        self.0.push(move |world: &mut World| {
            spawned.set(world.spawn((MapEntity, bundle)).id()).unwrap();
        });
        SceneEntity {
            commands: &mut self.0,
            id,
        }
    }
    pub fn insert_resource<R: Resource>(&mut self, resource: R) {
        self.0.push(move |world: &mut World| {
            world.insert_resource(resource);
        });
    }
}
impl SceneEntity<'_> {
    pub fn insert<B: Bundle>(&mut self, bundle: B) {
        let id = self.id.clone();
        self.commands.push(move |world: &mut World| {
            world
                .entity_mut(*id.get().expect("scene spawn precedes insert"))
                .insert(bundle);
        });
    }
}

pub(crate) struct PreparedScene {
    commands: SceneCommands,
    meshes: StagedAssets<Mesh>,
    materials: StagedAssets<StandardMaterial>,
    retail: StagedAssets<RetailWorldMaterial>,
    sky: StagedAssets<RetailSkyMaterial>,
    images: StagedAssets<Image>,
}
impl PreparedScene {
    pub fn new(world: &World) -> Self {
        Self {
            commands: Default::default(),
            meshes: StagedAssets::new(world),
            materials: StagedAssets::new(world),
            retail: StagedAssets::new(world),
            sky: StagedAssets::new(world),
            images: StagedAssets::new(world),
        }
    }
    pub fn prepare(
        &mut self,
        map: Option<&skate_data::skate_map::SkateMap>,
        root: &std::path::Path,
    ) {
        // Every scene starts with the same environment defaults. Map-specific
        // resources below overwrite these, including after a native scene.
        self.commands
            .insert_resource(ClearColor(Color::srgb(0.065, 0.08, 0.10)));
        self.commands.insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 350.,
            ..default()
        });
        if let Some(map) = map {
            crate::skate_world::spawn(
                map,
                &mut self.commands,
                &mut self.meshes,
                &mut self.materials,
                &mut self.retail,
                &mut self.images,
                &crate::retail_render::MaterialTuning::load(root),
            );
            if crate::retail_render::RetailScene::for_map(map) {
                crate::retail_render::spawn_backdrop(
                    &map.name,
                    root,
                    &mut self.commands,
                    &mut self.meshes,
                    &mut self.materials,
                    &mut self.retail,
                    &mut self.images,
                );
                crate::retail_render::spawn_sky(
                    &map.name,
                    root,
                    &mut self.commands,
                    &mut self.meshes,
                    &mut self.images,
                    &mut self.sky,
                    &mut self.retail,
                );
            } else {
                custom_lighting(map, &mut self.commands);
                celestial_bodies(
                    &mut self.commands,
                    &mut self.meshes,
                    &mut self.materials,
                    &mut self.images,
                );
            }
        } else {
            crate::world::spawn_test_world(
                &mut self.commands,
                &mut self.meshes,
                &mut self.materials,
            );
        }
    }
    pub fn publish(mut self, world: &mut World) {
        let _span = info_span!("publish_map_assets").entered();
        let owned = MapAssets {
            meshes: self.meshes.publish(world),
            materials: self.materials.publish(world),
            retail: self.retail.publish(world),
            sky: self.sky.publish(world),
            images: self.images.publish(world),
        };
        self.commands.0.apply(world);
        world.insert_resource(owned);
    }
}

#[derive(Resource, Default)]
pub(crate) struct MapAssets {
    meshes: Vec<AssetId<Mesh>>,
    materials: Vec<AssetId<StandardMaterial>>,
    retail: Vec<AssetId<RetailWorldMaterial>>,
    sky: Vec<AssetId<RetailSkyMaterial>>,
    images: Vec<AssetId<Image>>,
}

/// Scene-owned environment; retired with its sun when switching maps.
#[derive(Component)]
pub(crate) struct DayEnvironment {
    values: Vec<f32>,
    sky: Option<Color>,
    sun_direction: Vec3,
}
fn orbit(azimuth: f32, hour: f32) -> Vec3 {
    let angle = (hour - 6.) * std::f32::consts::PI / 12.;
    Vec3::new(
        azimuth.cos() * angle.cos(),
        angle.sin(),
        azimuth.sin() * angle.cos(),
    )
}

#[derive(Component)]
pub(crate) struct CelestialBody {
    moon: bool,
}
fn celestial_bodies(
    commands: &mut SceneCommands,
    meshes: &mut StagedAssets<Mesh>,
    materials: &mut StagedAssets<StandardMaterial>,
    images: &mut StagedAssets<Image>,
) {
    let disc = meshes.add(Rectangle::new(2., 2.).into());
    for moon in [false, true] {
        let texture = images.add(celestial_texture(moon));
        commands.spawn((
            Name::new(if moon { "Custom moon" } else { "Custom sun" }),
            CelestialBody { moon },
            Mesh3d(disc.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(texture),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
    }
}
/// Generated once during off-thread map preparation, never during gameplay.
/// A transparent border contains the corona/halo without a hard square edge.
fn celestial_texture(moon: bool) -> Image {
    const SIZE: u32 = 256;
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let craters = [
        (-0.35f32, 0.2f32, 0.22f32),
        (0.28, -0.3, 0.16),
        (0.35, 0.4, 0.12),
        (-0.12, -0.58, 0.1),
        (-0.5, -0.28, 0.13),
        (0.05, 0.15, 0.08),
        (0.62, 0.02, 0.1),
        (-0.15, 0.62, 0.09),
    ];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = (x as f32 + 0.5) * 2. / SIZE as f32 - 1.;
            let v = (y as f32 + 0.5) * 2. / SIZE as f32 - 1.;
            let r = u.hypot(v);
            let q = r / 0.35;
            let edge = ((1. - q) * 60.).clamp(0., 1.);
            let (rgb, alpha) = if q <= 1. {
                if moon {
                    let px = u / 0.35;
                    let py = v / 0.35;
                    let mut tone = 0.73
                        + 0.055 * (px * 19. + (py * 13.).sin()).sin()
                        + 0.035 * (py * 37. + px * 29.).cos();
                    for (cx, cy, radius) in craters {
                        let d = (px - cx).hypot(py - cy) / radius;
                        tone -= 0.19 * (-d * d * 2.).exp();
                        tone += 0.12 * (-((d - 1.) * 7.).powi(2)).exp();
                    }
                    tone *= 0.65 + 0.35 * (1. - q * q).max(0.).sqrt();
                    ([tone * 0.94, tone * 0.98, tone * 1.07], edge)
                } else {
                    ([1., 0.96 - q * 0.13, 0.78 - q * 0.32], edge)
                }
            } else {
                let angle = v.atan2(u);
                let rays = if moon {
                    1.
                } else {
                    0.65 + 0.2 * (angle * 12. + r * 9.).sin().powi(2)
                        + 0.15 * (angle * 23. - r * 6.).cos().powi(2)
                };
                let glow = (-(q - 1.) * if moon { 4. } else { 2.4 }).exp()
                    * (1. - r).max(0.).powi(2)
                    * rays;
                (
                    if moon {
                        [0.55, 0.68, 1.]
                    } else {
                        [1., 0.63, 0.16]
                    },
                    glow * if moon { 0.36 } else { 0.85 },
                )
            };
            for c in rgb {
                pixels.push((c.clamp(0., 1.) * 255.) as u8);
            }
            pixels.push((alpha.clamp(0., 1.) * 255.) as u8);
        }
    }
    Image::new(
        bevy::render::render_resource::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        pixels,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
}
/// Follow the final gameplay camera before transform propagation, with no
/// parallax. Ordinary depth testing lets buildings and terrain obscure them.
pub(crate) fn position_celestial_bodies(
    time: Res<Time<Virtual>>,
    environment: Query<&DayEnvironment>,
    camera: Query<
        (&Transform, &Projection),
        (With<crate::camera::GameplayCamera>, Without<CelestialBody>),
    >,
    mut bodies: Query<
        (&CelestialBody, &mut Transform, &mut Visibility),
        Without<crate::camera::GameplayCamera>,
    >,
) {
    let (Ok(environment), Ok((camera, projection))) = (environment.single(), camera.single())
    else {
        return;
    };
    let far = match projection {
        Projection::Perspective(p) => p.far,
        Projection::Orthographic(p) => p.far,
        _ => 1000.,
    };
    let distance = if far.is_finite() { far * 0.98 } else { 10_000. };
    for (body, mut transform, mut visibility) in &mut bodies {
        let direction = environment.sun_direction * if body.moon { -1. } else { 1. };
        *visibility = if direction.y > 0. {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        *transform = Transform::from_translation(camera.translation + direction * distance)
            .looking_at(
                camera.translation,
                if direction.y.abs() > 0.99 {
                    Vec3::Z
                } else {
                    Vec3::Y
                },
            )
            .with_scale(Vec3::splat(distance * if body.moon { 0.13 } else { 0.115 }));
        if !body.moon {
            transform.rotate_local_z(time.elapsed_secs() * 0.025);
            transform.scale *= 1. + 0.012 * (time.elapsed_secs() * 0.8).sin();
        }
    }
}
fn daylight_values(e: &[f32], hour: f32) -> (Color, f32, f32, Transform, f32) {
    let angle = (hour - 6.) * std::f32::consts::PI / 12.;
    let elevation = angle.sin();
    let daylight = elevation.max(0.);
    let extended = e.len() == 45;
    let night = elevation < 0.;
    let rgb = |i: usize| Color::linear_rgb(e[i].max(0.), e[i + 1].max(0.), e[i + 2].max(0.));
    let color = if extended {
        rgb(if night { 35 } else { 32 })
    } else if night {
        Color::srgb(0.55, 0.65, 1.)
    } else {
        Color::WHITE
    };
    let strength = if extended {
        e[if night { 39 } else { 38 }].max(0.)
    } else if night {
        0.025
    } else {
        1.
    };
    let ambient = if extended {
        e[41] + (e[40] - e[41]) * daylight
    } else {
        0.025 + 0.295 * daylight
    };
    // Signed horizontal motion carries the sun east-to-west. The moon follows
    // the opposite hemisphere; both fade to zero at the horizon.
    let horizontal = angle.cos() * if night { -1. } else { 1. };
    let direction = Vec3::new(
        e[11].cos() * horizontal,
        elevation.abs().max(0.001),
        e[11].sin() * horizontal,
    )
    .normalize();
    (
        color,
        10_000. * strength * elevation.abs(),
        ambient.max(0.) * 1000.,
        Transform::default().looking_to(
            -direction,
            if direction.y > 0.99 { Vec3::Z } else { Vec3::Y },
        ),
        daylight,
    )
}
fn custom_lighting(map: &skate_data::skate_map::SkateMap, commands: &mut SceneCommands) {
    let (color, illuminance, brightness, transform, _) =
        daylight_values(&map.environment, map.environment[10].rem_euclid(24.));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness,
        ..default()
    });
    commands.spawn((
        Name::new("Custom map sun/moon"),
        DayEnvironment {
            values: map.environment.to_vec(),
            sky: None,
            sun_direction: orbit(map.environment[11], map.environment[10]),
        },
        DirectionalLight {
            color,
            illuminance,
            shadows_enabled: true,
            affects_lightmapped_mesh_diffuse: false,
            ..default()
        },
        transform,
        bevy::light::CascadeShadowConfigBuilder {
            maximum_distance: 100.,
            first_cascade_far_bound: 10.,
            ..default()
        }
        .build(),
    ));
}
pub(crate) fn advance_day(
    time: Res<Time<Virtual>>,
    mut menu: ResMut<crate::graphics_menu::Menu>,
    mut lights: Query<(&mut DayEnvironment, &mut DirectionalLight, &mut Transform)>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
) {
    if lights.is_empty() {
        return;
    }
    let hour = menu.advance_day(time.delta_secs());
    for (mut environment, mut light, mut transform) in &mut lights {
        let base = *environment.sky.get_or_insert(clear.0);
        let (color, illuminance, brightness, pose, daylight) =
            daylight_values(&environment.values, hour);
        environment.sun_direction = orbit(environment.values[11], hour);
        light.color = color;
        light.illuminance = illuminance;
        *transform = pose;
        ambient.brightness = menu.ambient_brightness(brightness);
        clear.0 = Color::srgb(0.003, 0.005, 0.015).mix(&base, daylight.sqrt());
    }
}
impl MapAssets {
    pub fn retire(world: &mut World) {
        let entities: Vec<_> = world
            .query_filtered::<Entity, With<MapEntity>>()
            .iter(world)
            .collect();
        for entity in entities {
            world.despawn(entity);
        }
        if let Some(owned) = world.remove_resource::<Self>() {
            fn remove<A: Asset>(world: &mut World, ids: Vec<AssetId<A>>) {
                let mut assets = world.resource_mut::<Assets<A>>();
                for id in ids {
                    assets.remove(id);
                }
            }
            remove(world, owned.meshes);
            remove(world, owned.materials);
            remove(world, owned.retail);
            remove(world, owned.sky);
            remove(world, owned.images);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daylight_orbit_and_authored_zero_intensity() {
        let mut e = vec![0.; 12];
        let (_, noon, ambient_day, pose, _) = daylight_values(&e, 12.);
        let (_, midnight, ambient_night, _, _) = daylight_values(&e, 0.);
        assert!(noon > midnight && ambient_day > ambient_night);
        assert!(pose.rotation.is_finite());
        let (_, _, _, morning, _) = daylight_values(&e, 9.);
        let (_, _, _, evening, _) = daylight_values(&e, 15.);
        assert!(morning.forward().x * evening.forward().x < 0.);
        e.resize(45, 0.);
        assert_eq!(daylight_values(&e, 12.).1, 0.);
        assert_eq!(daylight_values(&e, 0.).1, 0.);
    }
    #[test]
    #[ignore = "requires user-supplied custom map; CPU preparation only"]
    fn supplied_custom_map_prepares_and_retires() {
        let path = std::env::var("SKATE_TEST_CUSTOM_MAP").unwrap();
        let root = std::env::var("SKATE_TEST_CUSTOM_ASSETS").unwrap();
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&path)).unwrap();
        crate::skate_world::validate_runtime(&map).unwrap();
        assert!(!crate::retail_render::RetailScene::for_map(&map));
        let mut world = world();
        let mut scene = PreparedScene::new(&world);
        scene.prepare(Some(&map), std::path::Path::new(&root));
        scene.publish(&mut world);
        assert_eq!(world.query::<&DirectionalLight>().iter(&world).count(), 1);
        MapAssets::retire(&mut world);
        assert_eq!(
            world
                .query_filtered::<Entity, With<MapEntity>>()
                .iter(&world)
                .count(),
            0
        );
    }
    #[test]
    fn custom_lighting_switches_without_accumulation_and_keeps_pbr() {
        let mut world = world();
        let mut map = skate_data::skate_map::SkateMap::parse(include_bytes!(
            "../../../maps/format-demo.skate"
        ))
        .unwrap();
        map.name = "university".into(); // Names cannot select retail rendering.
        map.materials[0].retail_definition = None;
        map.extensions.clear();
        for retail in [false, true, false, true] {
            map.extensions.clear();
            if retail {
                // Legacy retail imports can retain provenance without definitions.
                map.extensions.push(skate_data::skate_map::Extension {
                    tag: *b"WMET",
                    schema: 1,
                    payload: b"{}".to_vec(),
                });
            }
            assert_eq!(crate::retail_render::RetailScene::for_map(&map), retail);
            let mut scene = PreparedScene::new(&world);
            scene.prepare(Some(&map), std::path::Path::new("unused"));
            scene.publish(&mut world);
            assert_eq!(
                world.query::<&DirectionalLight>().iter(&world).count(),
                usize::from(!retail)
            );
            assert_eq!(
                world.query::<&CelestialBody>().iter(&world).count(),
                if retail { 0 } else { 2 }
            );
            // Only sky discs are unlit; world surfaces retain the PBR path.
            let body_materials: Vec<_> = world
                .query_filtered::<&MeshMaterial3d<StandardMaterial>, With<CelestialBody>>()
                .iter(&world)
                .map(|m| m.0.id())
                .collect();
            assert!(
                world
                    .resource::<Assets<StandardMaterial>>()
                    .iter()
                    .all(|(id, m)| body_materials.contains(&id) || !m.unlit)
            );
            MapAssets::retire(&mut world);
            assert_eq!(world.query::<&DirectionalLight>().iter(&world).count(), 0);
        }
    }

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<RetailWorldMaterial>>();
        world.init_resource::<Assets<RetailSkyMaterial>>();
        world.init_resource::<Assets<Image>>();
        world
    }

    #[test]
    fn repeated_scene_retirement_releases_owned_assets_and_preserves_character() {
        let mut world = world();
        let character_mesh = world.resource_mut::<Assets<Mesh>>().add(Cuboid::default());
        let character = world
            .spawn((crate::world::PlayerRoot, Mesh3d(character_mesh.clone())))
            .id();
        let map = skate_data::skate_map::SkateMap::parse(include_bytes!(
            "../../../maps/format-demo.skate"
        ))
        .unwrap();
        for iteration in 0..6 {
            let mut scene = PreparedScene::new(&world);
            scene.prepare(
                (iteration % 2 == 0).then_some(&map),
                std::path::Path::new("unused"),
            );
            // Preparation has not published any live entities or assets.
            assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
            scene.publish(&mut world);
            assert!(
                world
                    .query_filtered::<Entity, With<MapEntity>>()
                    .iter(&world)
                    .count()
                    > 0
            );
            let old: Vec<_> = world
                .query_filtered::<Entity, With<MapEntity>>()
                .iter(&world)
                .collect();
            MapAssets::retire(&mut world);
            assert!(old.into_iter().all(|id| world.get_entity(id).is_err()));
            assert!(world.get_entity(character).is_ok());
            assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
            assert!(world.resource::<Assets<Mesh>>().contains(&character_mesh));
            assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
            assert_eq!(world.resource::<Assets<RetailWorldMaterial>>().len(), 0);
            assert_eq!(world.resource::<Assets<RetailSkyMaterial>>().len(), 0);
            assert_eq!(world.resource::<Assets<Image>>().len(), 0);
        }
    }

    #[test]
    fn abandoned_preparation_does_not_replace_live_scene() {
        let mut world = world();
        let mut active = PreparedScene::new(&world);
        active.prepare(None, std::path::Path::new("unused"));
        active.publish(&mut world);
        let entities: Vec<_> = world
            .query_filtered::<Entity, With<MapEntity>>()
            .iter(&world)
            .collect();
        let meshes = world.resource::<Assets<Mesh>>().len();
        let mut abandoned = PreparedScene::new(&world);
        abandoned.prepare(None, std::path::Path::new("unused"));
        drop(abandoned);
        assert_eq!(world.resource::<Assets<Mesh>>().len(), meshes);
        assert!(entities.into_iter().all(|id| world.get_entity(id).is_ok()));
    }
}
