//! Original APT/GEO triangles, Xbox prompts and bitmap glyphs; no system fonts.
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat},
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Hud {
    version: u32,
    canvas: [f32; 2],
    textures: Vec<Texture>,
    meshes: Vec<Primitive>,
}
#[derive(Deserialize)]
struct Texture {
    file: String,
    width: u32,
    height: u32,
}
#[derive(Deserialize)]
struct Primitive {
    texture: Option<usize>,
    color: [f32; 4],
    vertices: Vec<Vertex>,
}
#[derive(Deserialize)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
}
#[derive(Component)]
struct MarkerHudRoot;

pub(super) fn install(app: &mut App) {
    app.add_systems(Startup, load).add_systems(Update, present);
}

pub(super) fn overlay_path(root: &Path) -> PathBuf {
    std::env::var_os("SKATE3_SESSION_MARKER_OVERLAY")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("private/session-marker"))
}

fn load(
    mut commands: Commands,
    config: Res<crate::config::Config>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let layer = RenderLayers::layer(29);
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Msaa::Off,
        layer.clone(),
    ));
    let folder = overlay_path(&config.asset_root);
    let read = || -> Result<Hud, String> {
        let bytes = std::fs::read(folder.join("hud.json")).map_err(|e| e.to_string())?;
        let hud: Hud = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if hud.version != 1 || hud.canvas != [1280., 720.] {
            return Err("Unsupported original HUD manifest".into());
        }
        Ok(hud)
    };
    let hud = match read() {
        Ok(h) => h,
        Err(e) => {
            warn!(
                "Original session-marker HUD unavailable at {}: {e}. See docs/hud-installation.md",
                folder.display()
            );
            return;
        }
    };
    let mut textures = Vec::new();
    for t in hud.textures {
        let path = Path::new(&t.file);
        if path.components().count() != 1 || t.width == 0 || t.height == 0 {
            error!("Invalid marker texture descriptor");
            return;
        }
        let Ok(bytes) = std::fs::read(folder.join(path)) else {
            error!("Missing marker texture {}", t.file);
            return;
        };
        if bytes.len() != t.width as usize * t.height as usize * 4 {
            error!("Invalid marker texture length");
            return;
        }
        let mut image = Image::new(
            Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            bytes,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::linear();
        textures.push(images.add(image));
    }
    if hud.meshes.iter().any(|m| {
        m.vertices.len() % 3 != 0
            || m.texture.is_some_and(|t| t >= textures.len())
            || !m
                .vertices
                .iter()
                .all(|v| v.position.iter().chain(&v.uv).all(|x| x.is_finite()))
    }) {
        error!("Invalid marker HUD geometry");
        return;
    }
    info!(
        "Original session-marker HUD loaded from {}",
        folder.display()
    );
    commands
        .spawn((MarkerHudRoot, Transform::default(), Visibility::Hidden))
        .with_children(|parent| {
            for (order, primitive) in hud.meshes.into_iter().enumerate() {
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    primitive
                        .vertices
                        .iter()
                        .map(|v| [v.position[0], -v.position[1], 0.])
                        .collect::<Vec<_>>(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_UV_0,
                    primitive.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
                );
                let [r, g, b, a] = primitive.color;
                let material = ColorMaterial {
                    color: Color::srgba(r, g, b, a),
                    texture: primitive.texture.map(|i| textures[i].clone()),
                    ..default()
                };
                parent.spawn((
                    Mesh2d(meshes.add(mesh)),
                    MeshMaterial2d(materials.add(material)),
                    layer.clone(),
                    Transform::from_xyz(0., 0., order as f32 * 0.01),
                ));
            }
        });
}

fn present(
    session: Res<super::SessionMarker>,
    window: Single<&Window>,
    mut roots: Query<(&mut Transform, &mut Visibility), With<MarkerHudRoot>>,
) {
    let scale = (window.width() / 1280.).min(window.height() / 720.);
    for (mut transform, mut visibility) in &mut roots {
        *visibility = if session.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.translation = Vec3::new(-window.width() / 2., window.height() / 2., 0.);
        transform.scale = Vec3::splat(scale);
    }
}
