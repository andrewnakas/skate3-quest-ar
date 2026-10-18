//! A world-space UI panel for XR: a UI camera renders into an image shown on
//! a quad that opens in front of the player. There is no flat HUD in XR, so
//! menus target [`XrPanel::camera`] (with `UiTargetCamera`) instead.
//!
//! The quad is kept in tracking space (real metres), so it stays put in the
//! room while the world is rescaled.
use super::rig::Rig;
use bevy::camera::RenderTarget;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_mod_xr::camera::XrCamera;

/// Panel image size, pixels, and its real size, metres.
const WIDTH_PX: u32 = 1000;
const HEIGHT_PX: u32 = 900;
const WIDTH_M: f32 = 0.62;
/// How far in front of the eyes it opens, and how far below them, metres.
const DISTANCE_M: f32 = 0.7;
const DROP_M: f32 = 0.12;

/// Marks the panel's UI camera, which `headset_only_rendering` leaves on.
#[derive(Component)]
pub(crate) struct XrPanelCamera;

#[derive(Component)]
struct PanelQuad;

#[derive(Resource)]
pub(crate) struct XrPanel {
    pub camera: Entity,
    /// Show the quad (and render its camera).
    pub visible: bool,
    /// Place it in front of the head on the next frame.
    pub recenter: bool,
    /// Tracking-space pose of the quad.
    pose: Transform,
}

pub(crate) struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, setup)
            .add_systems(PostUpdate, place.before(bevy::transform::TransformSystems::Propagate));
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut image = Image::new_target_texture(WIDTH_PX, HEIGHT_PX, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    let image = images.add(image);
    let camera = commands
        .spawn((
            XrPanelCamera,
            Camera2d,
            Camera {
                order: -10,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
        ))
        .id();
    let height_m = WIDTH_M * HEIGHT_PX as f32 / WIDTH_PX as f32;
    commands.spawn((
        PanelQuad,
        Mesh3d(meshes.add(Rectangle::new(WIDTH_M, height_m))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(image),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.insert_resource(XrPanel {
        camera,
        visible: true,
        recenter: true,
        pose: Transform::from_xyz(0.0, 1.2, -DISTANCE_M),
    });
}

fn place(
    rig: Res<Rig>,
    mut panel: ResMut<XrPanel>,
    eyes: Query<&Transform, (With<XrCamera>, Without<PanelQuad>)>,
    mut quad: Query<(&mut Transform, &mut Visibility), With<PanelQuad>>,
    mut cameras: Query<&mut Camera, With<XrPanelCamera>>,
) {
    if let Ok(mut camera) = cameras.single_mut() {
        if camera.is_active != panel.visible {
            camera.is_active = panel.visible;
        }
    }
    let Ok((mut tf, mut vis)) = quad.single_mut() else { return };
    let want = if panel.visible { Visibility::Visible } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if panel.recenter {
        let mut n = 0.0;
        let mut head = Vec3::ZERO;
        let mut rotation = None;
        for eye in &eyes {
            head += eye.translation;
            rotation.get_or_insert(eye.rotation);
            n += 1.0;
        }
        if let Some(rotation) = rotation {
            panel.recenter = false;
            let head = head / n;
            let forward = (rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
            let at = head + forward * DISTANCE_M - Vec3::Y * DROP_M;
            // Face the player: -Z looks away, so the quad's front (+Z) faces the head.
            panel.pose = Transform::from_translation(at).looking_to(forward, Vec3::Y);
        }
    }
    let Some(root) = rig.root_transform() else { return };
    let target = root.mul_transform(panel.pose);
    if *tf != target {
        *tf = target;
    }
}
