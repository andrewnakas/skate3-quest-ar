//! The menu laser: a ray from the right controller's aim pose onto the panel
//! quad, turned into a panel pixel and hit-tested against the laid-out rows
//! (and slider bars). The right trigger clicks; held on a slider it drags.
use super::{UiBar, UiRow, XrUi};
use crate::xr::actions::{Hand, LatestTouch};
use crate::xr::panel::{HEIGHT_M, HEIGHT_PX, WIDTH_M, WIDTH_PX, XrPanel};
use crate::xr::rig::Rig;
use bevy::prelude::*;

const PRESS: f32 = 0.6;
const RELEASE: f32 = 0.35;
/// A miss draws this much beam, real metres.
const MISS_M: f32 = 1.2;
const BEAM_RADIUS_M: f32 = 0.0018;
const DOT_RADIUS_M: f32 = 0.007;

#[derive(Resource, Default, Clone, Debug)]
pub(crate) struct UiPointer {
    /// Row slot under the laser.
    pub row: Option<usize>,
    /// Panel pixel under the laser.
    px: Option<Vec2>,
    /// Slider bars on screen: slot, centre and size in panel pixels.
    bars: Vec<(usize, Vec2, Vec2)>,
    /// The trigger was pulled this frame with the laser on the panel.
    pub click: bool,
    /// The trigger is held.
    pub held: bool,
    /// Tracking-space beam (from, to) and whether it ends on the panel.
    beam: Option<(Vec3, Vec3, bool)>,
}

/// A drag starts this close to a slider bar's ends, panel pixels.
const BAR_GRAB_PX: f32 = 24.0;

impl UiPointer {
    /// The laser is on (or just beside) widget `index`'s slider bar: a
    /// click there starts a drag; a click on the label doesn't move it.
    pub(crate) fn over_bar(&self, first: usize, index: usize) -> bool {
        let (Some(slot), Some(px)) = (index.checked_sub(first), self.px) else { return false };
        self.bars.iter().any(|&(s, centre, size)| {
            s == slot && size.x > 0.0 && (px.x - centre.x).abs() <= size.x / 2.0 + BAR_GRAB_PX
        })
    }

    /// Where along widget `index`'s slider bar the laser is (0..1), by its
    /// x alone so a drag keeps working off the row.
    pub(crate) fn bar_frac(&self, first: usize, index: usize) -> Option<f32> {
        let slot = index.checked_sub(first)?;
        let px = self.px?;
        let &(_, centre, size) = self.bars.iter().find(|b| b.0 == slot)?;
        (size.x > 0.0).then(|| ((px.x - (centre.x - size.x / 2.0)) / size.x).clamp(0.0, 1.0))
    }
}

/// Where a ray hits the panel: the tracking-space point and the panel pixel
/// (0,0 top left). `None` if it misses the quad or points away.
pub(crate) fn ray_panel(pose: Transform, origin: Vec3, dir: Vec3) -> Option<(Vec3, Vec2)> {
    let normal = pose.rotation * Vec3::Z;
    let denom = dir.dot(normal);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = (pose.translation - origin).dot(normal) / denom;
    if t <= 0.0 {
        return None;
    }
    let hit = origin + dir * t;
    let local = pose.rotation.inverse() * (hit - pose.translation);
    let u = local.x / WIDTH_M + 0.5;
    let v = 0.5 - local.y / HEIGHT_M;
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v))
        .then(|| (hit, Vec2::new(u * WIDTH_PX as f32, v * HEIGHT_PX as f32)))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn aim(
    touch: Res<LatestTouch>,
    panel: Res<XrPanel>,
    ui: Res<XrUi>,
    hands: Query<(&Hand, &Transform)>,
    rows: Query<(&UiRow, &ComputedNode, &UiGlobalTransform)>,
    bars: Query<(&UiBar, &ComputedNode, &UiGlobalTransform)>,
    mut pointer: ResMut<UiPointer>,
    mut pressed: Local<bool>,
) {
    let trigger = touch.0.right_trigger;
    let was = *pressed;
    *pressed = if was { trigger > RELEASE } else { trigger > PRESS };
    let active = ui.open && ui.external.is_none() && panel.visible;
    let controller = hands.iter().find(|(h, _)| matches!(h, Hand::Right)).map(|(_, t)| *t);
    let (Some(controller), true) = (controller, active) else {
        *pointer = UiPointer { held: *pressed, ..default() };
        return;
    };
    let origin = controller.translation;
    let dir = (controller.rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
    let hit = ray_panel(panel.pose(), origin, dir);
    let px = hit.map(|(_, px)| px);
    let row = px.and_then(|p| {
        rows.iter()
            .find(|(_, node, tf)| node.size().x > 0.0 && node.contains_point(**tf, p))
            .map(|(r, ..)| r.0)
    });
    *pointer = UiPointer {
        row,
        px,
        bars: bars
            .iter()
            .filter(|(_, node, _)| node.size().x > 0.0)
            .map(|(b, node, tf)| (b.0, tf.translation, node.size()))
            .collect(),
        click: *pressed && !was && hit.is_some(),
        held: *pressed,
        beam: Some(match hit {
            Some((point, _)) => (origin, point, true),
            None => (origin, origin + dir * MISS_M, false),
        }),
    };
}

#[derive(Component)]
pub(super) struct Beam;
#[derive(Component)]
pub(super) struct BeamDot;

pub(super) fn spawn_beam(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.45, 0.95, 0.88, 0.75),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.spawn((
        Beam,
        Mesh3d(meshes.add(Cylinder::new(1.0, 1.0))),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.spawn((
        BeamDot,
        Mesh3d(meshes.add(Sphere::new(1.0))),
        MeshMaterial3d(material),
        Transform::default(),
        Visibility::Hidden,
    ));
}

#[allow(clippy::type_complexity)]
pub(super) fn draw_beam(
    rig: Res<Rig>,
    pointer: Res<UiPointer>,
    mut beam: Query<(&mut Transform, &mut Visibility), (With<Beam>, Without<BeamDot>)>,
    mut dot: Query<(&mut Transform, &mut Visibility), (With<BeamDot>, Without<Beam>)>,
) {
    let (Ok((mut beam_tf, mut beam_vis)), Ok((mut dot_tf, mut dot_vis))) = (beam.single_mut(), dot.single_mut()) else {
        return;
    };
    let (Some((from, to, on_panel)), Some(root)) = (pointer.beam, rig.root_transform()) else {
        beam_vis.set_if_neq(Visibility::Hidden);
        dot_vis.set_if_neq(Visibility::Hidden);
        return;
    };
    let length = from.distance(to).max(1e-3);
    let along = (to - from) / length;
    let local = Transform {
        translation: (from + to) / 2.0,
        rotation: Quat::from_rotation_arc(Vec3::Y, along),
        scale: Vec3::new(BEAM_RADIUS_M, length, BEAM_RADIUS_M),
    };
    *beam_tf = root.mul_transform(local);
    beam_vis.set_if_neq(Visibility::Visible);
    *dot_tf = root.mul_transform(Transform::from_translation(to).with_scale(Vec3::splat(DOT_RADIUS_M)));
    dot_vis.set_if_neq(if on_panel { Visibility::Visible } else { Visibility::Hidden });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_the_panel_centre_and_corners() {
        // The panel 0.7 m ahead (-Z), facing the player at the origin.
        let pose = Transform::from_xyz(0.0, 0.0, -0.7).looking_to(Vec3::NEG_Z, Vec3::Y);
        let (hit, px) = ray_panel(pose, Vec3::ZERO, Vec3::NEG_Z).unwrap();
        assert!(hit.distance(Vec3::new(0.0, 0.0, -0.7)) < 1e-5);
        assert!(px.distance(Vec2::new(WIDTH_PX as f32 / 2.0, HEIGHT_PX as f32 / 2.0)) < 0.5);
        // Up and to the right of the centre is the top-right of the image.
        let target = Vec3::new(WIDTH_M * 0.4, HEIGHT_M * 0.4, -0.7);
        let (_, px) = ray_panel(pose, Vec3::ZERO, target.normalize()).unwrap();
        assert!(px.x > WIDTH_PX as f32 * 0.85 && px.y < HEIGHT_PX as f32 * 0.15, "{px}");
        // Away from it, or past its edge: nothing.
        assert!(ray_panel(pose, Vec3::ZERO, Vec3::Z).is_none());
        assert!(ray_panel(pose, Vec3::ZERO, Vec3::new(1.0, 0.0, -0.2).normalize()).is_none());
    }

    #[test]
    fn bar_fraction_reads_x_only() {
        let p = UiPointer {
            px: Some(Vec2::new(150.0, 999.0)),
            bars: vec![(2, Vec2::new(200.0, 300.0), Vec2::new(200.0, 14.0))],
            ..default()
        };
        assert_eq!(p.bar_frac(3, 5), Some(0.25));
        assert_eq!(p.bar_frac(3, 4), None);
        assert_eq!(p.bar_frac(6, 5), None);
        assert!(p.over_bar(3, 5));
        let p = UiPointer { px: Some(Vec2::new(40.0, 0.0)), ..p };
        assert!(!p.over_bar(3, 5), "the label is not the bar");
    }
}
