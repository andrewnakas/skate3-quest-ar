//! Native noise pass, composited before the original HUD.
use super::noise::Noise;
use bevy::{
    asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path},
    camera::visibility::RenderLayers,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat},
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};

#[derive(Clone, Debug, ShaderType)]
struct Params {
    scroll: Vec4,
    first: Vec4,
    second: Vec4,
    fade: Vec4,
}
impl Params {
    fn bits(&self) -> [[u32; 4]; 4] {
        [self.scroll, self.first, self.second, self.fade].map(|v| v.to_array().map(f32::to_bits))
    }
}
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
struct Effect {
    #[uniform(0)]
    params: Params,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
}
impl Material2d for Effect {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from(embedded_path!("effect.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}
#[derive(Resource)]
struct Runtime {
    noise: Noise,
    material: Handle<Effect>,
    time: f64,
}
#[cfg(test)]
mod shader_path_tests {
    use super::*;
    #[test]
    fn embedded_shader_paths_match_this_binary() {
        let ShaderRef::Path(path) = Effect::fragment_shader() else {
            panic!("Expected embedded shader path")
        };
        assert_eq!(
            path,
            AssetPath::from(embedded_path!("effect.wgsl")).with_source("embedded")
        );
    }

    #[test]
    fn hidden_noise_keeps_its_sequence_and_publishes_before_becoming_visible() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Effect>()
            .init_resource::<super::super::SessionMarker>();
        let initial = Params {
            scroll: Vec4::ZERO,
            first: Vec4::X,
            second: Vec4::X,
            fade: Vec4::ZERO,
        };
        let material = app
            .world_mut()
            .resource_mut::<Assets<Effect>>()
            .add(Effect {
                params: initial.clone(),
                texture: default(),
            });
        let mut noise = Noise::default();
        noise.texture();
        app.insert_resource(Runtime {
            noise,
            material: material.clone(),
            time: 0.,
        });
        app.world_mut().spawn(Window::default());
        let screen = app
            .world_mut()
            .spawn((Screen, Transform::default(), Visibility::Hidden))
            .id();
        app.update();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<Effect>>>()
            .clear();
        let mut reference = Noise::default();
        reference.texture();
        let mut reference_time = 0.;
        let mut published = initial;
        for (dt, progress) in [
            (0.016, 0.),
            (0.25, 0.),
            (0., 0.5),
            (0., 0.5),
            (0.02, 0.7),
            (0.05, 0.),
            (0., 0.4),
        ] {
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(std::time::Duration::from_secs_f64(dt));
            app.world_mut()
                .resource_mut::<super::super::SessionMarker>()
                .progress = progress;
            reference_time += dt;
            while reference_time >= 1. / 60. {
                reference_time -= 1. / 60.;
                reference.advance();
            }
            let expected = Params {
                scroll: Vec4::from_array(reference.scroll),
                first: Vec4::from_array(Noise::weights(reference.phases[0])),
                second: Vec4::from_array(Noise::weights(reference.phases[1])),
                fade: Vec4::new(progress, 0., 0., 0.),
            };
            let modified = progress > 0. && expected.bits() != published.bits();
            app.world_mut().run_system_once(present).unwrap();
            if progress > 0. {
                published = expected;
            }
            assert_eq!(
                app.world()
                    .resource::<Assets<Effect>>()
                    .get(&material)
                    .unwrap()
                    .params
                    .bits(),
                published.bits()
            );
            let runtime = app.world().resource::<Runtime>();
            assert_eq!(
                runtime.noise.scroll.map(f32::to_bits),
                reference.scroll.map(f32::to_bits)
            );
            assert_eq!(
                runtime.noise.phases.map(f32::to_bits),
                reference.phases.map(f32::to_bits)
            );
            assert_eq!(
                *app.world().get::<Visibility>(screen).unwrap(),
                if progress > 0. {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                }
            );
            app.update();
            let modifications = app
                .world_mut()
                .resource_mut::<Messages<AssetEvent<Effect>>>()
                .drain()
                .filter(|e| matches!(e, AssetEvent::Modified { id } if *id == material.id()))
                .count();
            assert_eq!(modifications, usize::from(modified));
        }
    }
}
#[derive(Component)]
struct Screen;

pub(super) fn install(app: &mut App) {
    embedded_asset!(app, "effect.wgsl");
    app.add_plugins(Material2dPlugin::<Effect>::default())
        .add_systems(Startup, load)
        .add_systems(Update, present);
}
fn load(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<Effect>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let mut noise = Noise::default();
    let mut image = Image::new(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        noise.texture(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    let material = materials.add(Effect {
        texture: images.add(image),
        params: Params {
            scroll: Vec4::ZERO,
            first: Vec4::X,
            second: Vec4::X,
            fade: Vec4::ZERO,
        },
    });
    commands.spawn((
        Screen,
        Mesh2d(meshes.add(Rectangle::new(1., 1.))),
        MeshMaterial2d(material.clone()),
        Transform::from_xyz(0., 0., -1.),
        Visibility::Hidden,
        RenderLayers::layer(29),
    ));
    commands.insert_resource(Runtime {
        noise,
        material,
        time: 0.,
    });
}
fn present(
    mut runtime: ResMut<Runtime>,
    session: Res<super::SessionMarker>,
    time: Res<Time<Real>>,
    window: Single<&Window>,
    mut materials: ResMut<Assets<Effect>>,
    mut screen: Query<(&mut Transform, &mut Visibility), With<Screen>>,
) {
    runtime.time += time.delta_secs_f64();
    while runtime.time >= 1. / 60. {
        runtime.time -= 1. / 60.;
        runtime.noise.advance();
    }
    // Advance the native noise stream even while hidden, then publish the current
    // values on the first visible frame. Hidden assets need no GPU preparation.
    if session.progress > 0. {
        let params = Params {
            scroll: Vec4::from_array(runtime.noise.scroll),
            first: Vec4::from_array(Noise::weights(runtime.noise.phases[0])),
            second: Vec4::from_array(Noise::weights(runtime.noise.phases[1])),
            fade: Vec4::new(session.progress, 0., 0., 0.),
        };
        if materials
            .get(&runtime.material)
            .is_some_and(|m| m.params.bits() != params.bits())
        {
            materials.get_mut(&runtime.material).unwrap().params = params;
        }
    }
    for (mut transform, mut visibility) in &mut screen {
        let scale = Vec3::new(window.width(), window.height(), 1.);
        if transform.scale != scale {
            transform.scale = scale;
        }
        visibility.set_if_neq(if session.progress > 0. {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}
