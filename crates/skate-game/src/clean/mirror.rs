//! Switch: the rider's own stance, mirrored. After the clips are sampled (and before the procedural
//! layer pins the feet), every bone takes its partner's pose (left for right, `.L` / `_l`) reflected
//! through the body's middle, so one set of clips rides both stances.
//!
//! The reflection works on the skeleton's own space (the glTF scene, where the body's left-right is
//! X): a bone's mirrored pose is its partner's, reflected, then corrected by what turns the
//! reflected partner's rest pose into this bone's rest pose. That needs no bone-roll convention
//! and leaves the rest pose exactly where it was. Mirroring on and off cross-fades (a shuffle into
//! switch swaps over at a pose that is the same both ways, so there it barely shows).
use super::sim::CleanSkate;
use super::skater;
use bevy::math::Affine3A;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use skate_lite_sys::EventKind;

/// Seconds for the mirror to fade in or out.
const FADE: f32 = 0.06;

struct Joint {
    entity: Entity,
    /// Index of the mirror-image bone (itself for the middle ones).
    partner: usize,
    /// Index of the parent bone, or `None` for the top of the skeleton.
    parent: Option<usize>,
    /// (reflection · partner's rest)⁻¹ · own rest, in the scene's space.
    correction: Affine3A,
}

#[derive(Default)]
pub(crate) struct Mirror {
    /// Bones, parents before children; empty until the skater is in.
    joints: Vec<Joint>,
    /// The scene root (the space the skeleton is reflected in).
    root: Option<Entity>,
    /// 0 the rider's own stance, 1 mirrored.
    amount: f32,
}

/// The bone that mirrors `name` (`leg.L` <-> `leg.R`, `sk_index_01_l` <-> `sk_index_01_r`).
fn partner_name(name: &str) -> String {
    for (a, b) in [(".L", ".R"), (".R", ".L"), ("_l", "_r"), ("_r", "_l")] {
        if let Some(stem) = name.strip_suffix(a) {
            return format!("{stem}{b}");
        }
    }
    name.to_string()
}

const REFLECT: Affine3A = Affine3A::from_cols(Vec3A::NEG_X, Vec3A::Y, Vec3A::Z, Vec3A::ZERO);

/// `e`'s transform in the root's space (the root's own transform left out).
fn in_root(e: Entity, root: Entity, q: &Query<(&mut Transform, Option<&ChildOf>)>) -> Option<Affine3A> {
    let mut m = Affine3A::IDENTITY;
    let mut cur = e;
    while cur != root {
        let (t, parent) = q.get(cur).ok()?;
        m = t.compute_affine() * m;
        cur = parent?.parent();
    }
    Some(m)
}

fn find(
    skins: &Query<&SkinnedMesh>,
    roots: &Query<Entity, With<SceneRoot>>,
    names: &Query<&Name>,
    q: &Query<(&mut Transform, Option<&ChildOf>)>,
) -> Option<(Entity, Vec<Joint>)> {
    let skin = skins.iter().next()?;
    let root = roots.iter().find(|r| {
        let mut cur = skin.joints.first().copied();
        while let Some(c) = cur {
            if c == *r {
                return true;
            }
            cur = q.get(c).ok().and_then(|(_, p)| p).map(|p| p.parent());
        }
        false
    })?;
    // parents first: by depth below the root
    let depth = |e: Entity| {
        let (mut d, mut cur) = (0, e);
        while let Some(p) = q.get(cur).ok().and_then(|(_, p)| p) {
            d += 1;
            cur = p.parent();
        }
        d
    };
    let mut order: Vec<Entity> = skin.joints.clone();
    order.sort_by_key(|e| depth(*e));
    let name_of = |e: Entity| names.get(e).map(|n| n.as_str().to_string()).unwrap_or_default();
    let index_of = |n: &str| order.iter().position(|e| name_of(*e) == n);
    let rest: Vec<Affine3A> = order.iter().map(|e| in_root(*e, root, q)).collect::<Option<_>>()?;
    let mut joints = Vec::with_capacity(order.len());
    for (i, e) in order.iter().enumerate() {
        let partner = index_of(&partner_name(&name_of(*e))).unwrap_or(i);
        let parent = q.get(*e).ok().and_then(|(_, p)| p).and_then(|p| order.iter().position(|o| *o == p.parent()));
        let correction = (REFLECT * rest[partner]).inverse() * rest[i];
        joints.push(Joint { entity: *e, partner, parent, correction });
    }
    let pairs = joints.iter().enumerate().filter(|(i, j)| j.partner != *i).count() / 2;
    info!("SK8 mirror: {} bones, {} left/right pairs", joints.len(), pairs);
    Some((root, joints))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    time: Res<Time<Real>>,
    sim: Res<CleanSkate>,
    mut mirror: Local<Mirror>,
    skins: Query<&SkinnedMesh>,
    roots: Query<Entity, With<SceneRoot>>,
    names: Query<&Name>,
    mut q: Query<(&mut Transform, Option<&ChildOf>)>,
) {
    let target = if skater::mirrored(&sim.current) { 1.0 } else { 0.0 };
    if sim.events.iter().any(|e| e.kind == EventKind::Respawn) {
        mirror.amount = target;
    } else {
        let dt = if sim.paused { 0.0 } else { time.delta_secs().min(0.1) };
        mirror.amount += (target - mirror.amount) * (1.0 - (-dt / FADE).exp());
        if (target - mirror.amount).abs() < 1e-3 {
            mirror.amount = target;
        }
    }
    let w = mirror.amount;
    if w < 1e-4 {
        return;
    }
    if mirror.root.is_none() {
        let Some((root, joints)) = find(&skins, &roots, &names, &q) else { return };
        mirror.root = Some(root);
        mirror.joints = joints;
    }
    let (Some(root), joints) = (mirror.root, &mirror.joints) else { return };

    // This frame's pose, in the root's space.
    let mut local = Vec::with_capacity(joints.len());
    let mut base = Vec::with_capacity(joints.len());
    let mut global: Vec<Affine3A> = Vec::with_capacity(joints.len());
    for j in joints {
        let Ok((t, parent)) = q.get(j.entity) else { return };
        let l = t.compute_affine();
        let above = match j.parent {
            Some(p) => global[p],
            None => match parent.map(|p| p.parent()) {
                Some(p) if p != root => in_root(p, root, &q).unwrap_or(Affine3A::IDENTITY),
                _ => Affine3A::IDENTITY,
            },
        };
        local.push(*t);
        base.push(above);
        global.push(above * l);
    }
    // Mirrored: each bone where its partner is, reflected.
    let mirrored: Vec<Affine3A> = joints.iter().map(|j| REFLECT * global[j.partner] * j.correction).collect();
    for (i, j) in joints.iter().enumerate() {
        let above = j.parent.map_or(base[i], |p| mirrored[p]);
        let m = Transform::from_matrix(Mat4::from(above.inverse() * mirrored[i]));
        let own = local[i];
        let blended = Transform {
            translation: own.translation.lerp(m.translation, w),
            rotation: own.rotation.slerp(m.rotation, w),
            scale: own.scale.lerp(m.scale, w),
        };
        if let Ok((mut t, _)) = q.get_mut(j.entity) {
            *t = blended;
        }
    }
}
