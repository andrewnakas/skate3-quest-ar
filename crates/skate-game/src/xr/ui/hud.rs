//! The trick readout while skating: a small panel low in the corner of the
//! view that follows the head lazily (it drifts after a look, it doesn't
//! stick to the face). The trick being done big, its points under it in
//! yellow, and the score / combo line. Each tier fills [`TrickHud`].
//!
//! Hidden while a menu is open, and off with the AR settings' "Trick and
//! points readout".
use crate::xr::ar_settings::ArSettings;
use crate::xr::panel::{XrPanel, XrPanelCamera};
use crate::xr::rig::Rig;
use bevy::camera::RenderTarget;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_mod_xr::camera::XrCamera;

const WIDTH_PX: u32 = 640;
const HEIGHT_PX: u32 = 200;
const WIDTH_M: f32 = 0.22;
/// Where it sits in head space, metres: right, down, ahead.
const OFFSET: Vec3 = Vec3::new(0.15, -0.14, -0.6);
/// How quickly it catches up with the head, per second.
const FOLLOW: f32 = 5.0;

/// What the readout shows; each tier writes it every frame.
#[derive(Resource, Default, Clone, PartialEq)]
pub(crate) struct TrickHud {
    /// The trick in progress or just landed.
    pub name: String,
    /// Points it scored, if any.
    pub points: String,
    /// Score, combo, a new best.
    pub line: String,
}

#[derive(Component)]
struct HudQuad;
#[derive(Component)]
struct HudCamera;
#[derive(Component)]
struct HudName;
#[derive(Component)]
struct HudPoints;
#[derive(Component)]
struct HudLine;

pub(crate) struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrickHud>()
            .add_systems(Startup, setup)
            .add_systems(Update, draw)
            .add_systems(PostUpdate, place.before(bevy::transform::TransformSystems::Propagate));
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
) {
    let mut image = Image::new_target_texture(WIDTH_PX, HEIGHT_PX, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    let image = images.add(image);
    let camera = commands
        .spawn((
            XrPanelCamera,
            HudCamera,
            Camera2d,
            Camera {
                order: -11,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                is_active: false,
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
        ))
        .id();
    let height_m = WIDTH_M * HEIGHT_PX as f32 / WIDTH_PX as f32;
    commands.spawn((
        HudQuad,
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
    let font = assets.load(super::UI_FONT);
    let heavy = |size: f32| TextFont { font: font.clone(), font_size: size, weight: FontWeight(800), ..default() };
    let shadow = TextShadow { offset: Vec2::new(2.0, 3.0), color: Color::srgba(0.0, 0.0, 0.0, 0.55) };
    commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(px(16), px(8)),
                ..default()
            },
            UiTargetCamera(camera),
        ))
        .with_children(|c| {
            c.spawn((HudName, Text::new(""), heavy(58.0), TextColor(Color::WHITE), shadow));
            c.spawn((HudPoints, Text::new(""), heavy(40.0), TextColor(Color::srgb(0.98, 0.82, 0.22)), shadow));
            c.spawn((HudLine, Text::new(""), heavy(28.0), TextColor(Color::WHITE), shadow));
        });
}

#[allow(clippy::type_complexity)]
fn draw(
    hud: Res<TrickHud>,
    mut drawn: Local<TrickHud>,
    mut texts: ParamSet<(
        Query<&mut Text, With<HudName>>,
        Query<&mut Text, With<HudPoints>>,
        Query<&mut Text, With<HudLine>>,
    )>,
) {
    if *drawn == *hud {
        return;
    }
    *drawn = hud.clone();
    for mut t in &mut texts.p0() {
        **t = hud.name.to_uppercase();
    }
    for mut t in &mut texts.p1() {
        **t = hud.points.clone();
    }
    for mut t in &mut texts.p2() {
        **t = hud.line.clone();
    }
}

#[allow(clippy::type_complexity)]
fn place(
    rig: Res<Rig>,
    settings: Res<ArSettings>,
    panel: Res<XrPanel>,
    time: Res<Time<Real>>,
    eyes: Query<&Transform, (With<XrCamera>, Without<HudQuad>)>,
    mut quad: Query<(&mut Transform, &mut Visibility), With<HudQuad>>,
    mut camera: Query<&mut Camera, With<HudCamera>>,
    mut pose: Local<Option<Transform>>,
) {
    let (Ok((mut tf, mut vis)), Ok(mut cam)) = (quad.single_mut(), camera.single_mut()) else { return };
    let mut n = 0.0;
    let mut head = Vec3::ZERO;
    let mut rotation = None;
    for eye in &eyes {
        head += eye.translation;
        rotation.get_or_insert(eye.rotation);
        n += 1.0;
    }
    let show = settings.hud && !panel.visible && rotation.is_some();
    if cam.is_active != show {
        cam.is_active = show;
    }
    vis.set_if_neq(if show { Visibility::Visible } else { Visibility::Hidden });
    let (Some(rotation), Some(root), true) = (rotation, rig.root_transform(), show) else {
        *pose = None;
        return;
    };
    let head = head / n;
    let at = head + rotation * OFFSET;
    // Facing the eyes (its front, +Z, toward the head).
    let target = Transform::from_translation(at).looking_to(at - head, rotation * Vec3::Y);
    let k = 1.0 - (-FOLLOW * time.delta_secs()).exp();
    let current = pose.get_or_insert(target);
    current.translation = current.translation.lerp(target.translation, k);
    current.rotation = current.rotation.slerp(target.rotation, k);
    *tf = root.mul_transform(*current);
}
