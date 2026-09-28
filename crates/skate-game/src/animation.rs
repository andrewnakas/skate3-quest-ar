//! Render the same stock skeleton that animation and physics update.
//! The GLB supplies the mesh, skin weights and hierarchy; it does not play a
//! separate idle clip or own the skater's pose.
use crate::{app::FrameSet, assets::AssetStatus, physics::SkaterRuntime, world::PlayerRoot};
use bevy::{camera::visibility::NoFrustumCulling, mesh::skinning::SkinnedMesh, prelude::*};
use skate_core::animation::output::NativeMatrix;

#[derive(Resource, Default)]
pub(crate) struct AnimationStatus {
    pub ready: bool,
    bindings: Vec<BoneBinding>,
}
struct BoneBinding {
    entity: Entity,
    bone: usize,
    parent_bone: Option<usize>,
}
impl AnimationStatus {
    pub(crate) fn pose_transforms(&self, pose: &[Mat4]) -> Vec<(Entity, Transform)> {
        self.bindings
            .iter()
            .filter_map(|b| {
                let global = *pose.get(b.bone)? * render_basis();
                let local = if let Some(parent) = b.parent_bone {
                    (*pose.get(parent)? * render_basis()).inverse() * global
                } else {
                    global
                };
                Some((b.entity, Transform::from_matrix(local)))
            })
            .collect()
    }

    pub(crate) fn online_bindings(&self) -> Vec<(Entity, usize, Option<usize>)> {
        self.bindings
            .iter()
            .map(|b| (b.entity, b.bone, b.parent_bone))
            .collect()
    }

    /// Prepare a hidden imported scene without disturbing the live bindings.
    pub(crate) fn for_scene(
        root: Entity,
        names: &[String],
        skins: &Query<(Entity, &SkinnedMesh)>,
        nodes: &Query<(&Name, &Transform)>,
        parents: &Query<&ChildOf>,
    ) -> Result<Self, String> {
        let mut bindings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (entity, skin) in skins.iter() {
            if !parents.iter_ancestors(entity).any(|p| p == root) {
                continue;
            }
            for &joint in &skin.joints {
                if !seen.insert(joint) {
                    continue;
                }
                if !parents.iter_ancestors(joint).any(|p| p == root) {
                    return Err("Imported skin refers to a joint outside its scene".into());
                }
                let (name, _) = nodes
                    .get(joint)
                    .map_err(|_| "Imported joint has no name/transform")?;
                let bone = names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                    .ok_or_else(|| format!("Unsupported imported bone: {name}"))?;
                let mut parent_bone = None;
                for p in parents.iter_ancestors(joint) {
                    if p == root {
                        break;
                    }
                    if let Ok((name, transform)) = nodes.get(p) {
                        if let Some(i) = names
                            .iter()
                            .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                        {
                            parent_bone = Some(i);
                            break;
                        }
                        if !transform.to_matrix().abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                            return Err(
                                "Imported armature has an unsupported ancestor transform".into()
                            );
                        }
                    }
                }
                bindings.push(BoneBinding {
                    entity: joint,
                    bone,
                    parent_bone,
                });
            }
        }
        if bindings.is_empty() {
            return Err("Imported scene has no skinned character".into());
        }
        Ok(Self {
            ready: true,
            bindings,
        })
    }
}
#[cfg(test)]
mod custom_model_binding_tests {
    use super::*;
    use bevy::ecs::system::SystemState;
    #[test]
    fn custom_models_bind_only_the_candidate_and_reject_foreign_joints() {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let hips = world
            .spawn((Name::new("HIPS"), Transform::default(), ChildOf(root)))
            .id();
        let head = world
            .spawn((Name::new("HEAD"), Transform::default(), ChildOf(hips)))
            .id();
        for _ in 0..2 {
            world.spawn((
                ChildOf(root),
                SkinnedMesh {
                    inverse_bindposes: default(),
                    joints: vec![hips, head],
                },
            ));
        }
        let foreign = world
            .spawn((Name::new("unrelated"), Transform::default()))
            .id();
        world.spawn(SkinnedMesh {
            inverse_bindposes: default(),
            joints: vec![foreign],
        });
        let names = vec!["HIPS".into(), "HEAD".into()];
        let mut queries: SystemState<(
            Query<(Entity, &SkinnedMesh)>,
            Query<(&Name, &Transform)>,
            Query<&ChildOf>,
        )> = SystemState::new(&mut world);
        let (skins, nodes, parents) = queries.get(&world);
        let prepared = AnimationStatus::for_scene(root, &names, &skins, &nodes, &parents).unwrap();
        assert!(prepared.ready);
        assert_eq!(prepared.bindings.len(), 2);
        assert_eq!(
            prepared
                .bindings
                .iter()
                .find(|b| b.entity == head)
                .unwrap()
                .parent_bone,
            Some(0)
        );
        world.spawn((
            ChildOf(root),
            SkinnedMesh {
                inverse_bindposes: default(),
                joints: vec![foreign],
            },
        ));
        let (skins, nodes, parents) = queries.get(&world);
        assert!(AnimationStatus::for_scene(root, &names, &skins, &nodes, &parents).is_err());
        assert_eq!(prepared.bindings.len(), 2);
    }
}
pub(crate) struct AnimationPlugin;
impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnimationStatus>().add_systems(
            Update,
            (disable_skater_culling, bind, present)
                .chain()
                .in_set(FrameSet::Animation),
        );
    }
}
fn disable_skater_culling(
    mut commands: Commands,
    skins: Query<Entity, (With<SkinnedMesh>, Without<NoFrustumCulling>)>,
    parents: Query<&ChildOf>,
    roots: Query<Entity, With<PlayerRoot>>,
) {
    // GLB primitive bounds describe the bind pose, not the animated skin.
    // Apply to every skater mesh (not just the first skin used to bind bones).
    // World meshes retain their normal culling behavior.
    for entity in &skins {
        if parents.iter_ancestors(entity).any(|e| roots.contains(e)) {
            commands.entity(entity).insert(NoFrustumCulling);
        }
    }
}

fn bind(
    status: Res<AssetStatus>,
    skater: Res<SkaterRuntime>,
    mut animation: ResMut<AnimationStatus>,
    skins: Query<(Entity, &SkinnedMesh)>,
    nodes: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
    visibility: Query<&Visibility>,
    roots: Query<Entity, With<PlayerRoot>>,
    mut exit: MessageWriter<AppExit>,
) {
    if *status != AssetStatus::Ready || animation.ready {
        return;
    }
    let names = &skater.animation.evaluator.frames.bone_names;
    let result = (|| -> Result<Option<Vec<BoneBinding>>, String> {
        let mut bindings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (entity, skin) in &skins {
            if !parents.iter_ancestors(entity).any(|e| roots.contains(e)) {
                continue;
            }
            if parents
                .iter_ancestors(entity)
                .any(|e| visibility.get(e).is_ok_and(|v| *v == Visibility::Hidden))
            {
                continue;
            }
            let mut binding = Vec::with_capacity(skin.joints.len());
            for &joint in &skin.joints {
                if !seen.insert(joint) {
                    continue;
                }
                let (name, _) = nodes
                    .get(joint)
                    .map_err(|_| "Skater skin joint is missing its name or transform")?;
                let bone = names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                    .ok_or_else(|| {
                        format!("Skater skin bone {name} is absent from the stock rig")
                    })?;
                let mut parent_bone = None;
                for parent in parents.iter_ancestors(joint) {
                    if roots.contains(parent) {
                        break;
                    }
                    if let Ok((name, transform)) = nodes.get(parent) {
                        if let Some(index) = names
                            .iter()
                            .position(|n| n.eq_ignore_ascii_case(name.as_str()))
                        {
                            parent_bone = Some(index);
                            break;
                        }
                        // The supplied GLB's armature node is identity. Reject
                        // another import convention rather than double a pose.
                        if !transform.to_matrix().abs_diff_eq(Mat4::IDENTITY, 0.00001) {
                            return Err(format!(
                                "Skater armature ancestor {name} has an unsupported transform"
                            ));
                        }
                    }
                }
                binding.push(BoneBinding {
                    entity: joint,
                    bone,
                    parent_bone,
                });
            }
            // Bind each visible modular rig, retaining its authored inverse binds.
            bindings.extend(binding);
        }
        Ok(if bindings.is_empty() {
            None
        } else {
            Some(bindings)
        })
    })();
    match result {
        Ok(Some(binding)) => {
            animation.bindings = binding;
            animation.ready = true;
            info!(
                "GAME_CHARACTER_READY bones={} source=physical_stock_pose",
                animation.bindings.len()
            );
        }
        Ok(None) => (),
        Err(message) => {
            error!("{message}");
            exit.write(AppExit::error());
        }
    }
}
fn present(
    history: Res<crate::presentation::Presentation>,
    skater: Res<SkaterRuntime>,
    replay: Res<crate::replay::Replay>,
    time: Res<Time<Fixed>>,
    animation: Res<AnimationStatus>,
    mut nodes: Query<&mut Transform>,
) {
    if !animation.ready {
        return;
    }
    // Existing GLB was exported through Blender: its bone-local axes are
    // rotated -90 degrees about X relative to the native frames. Both files'
    // world positions are Y-up. This is a skin basis change, not a physics turn.
    let basis = render_basis();
    let Some((previous, current, alpha)) = history.view(&replay, time.overstep_fraction()) else {
        let pose: Vec<_> = skater
            .render_pose
            .iter()
            .copied()
            .map(native_matrix)
            .collect();
        for (entity, transform) in animation.pose_transforms(&pose) {
            if let Ok(mut node) = nodes.get_mut(entity) {
                *node = transform;
            }
        }
        return;
    };
    for binding in &animation.bindings {
        // Blend bone-local rotations, not matrix entries or independent world
        // positions: joints retain their hierarchy while limbs turn.
        let local = |snapshot: &crate::presentation::Snapshot| {
            let global = snapshot.bones[binding.bone] * basis;
            Transform::from_matrix(if let Some(parent) = binding.parent_bone {
                (snapshot.bones[parent] * basis).inverse() * global
            } else {
                global
            })
        };
        if let Ok(mut transform) = nodes.get_mut(binding.entity) {
            *transform = crate::presentation::blend(local(previous), local(current), alpha);
        }
    }
}
fn render_basis() -> Mat4 {
    Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W)
}
pub(crate) fn native_matrix(matrix: NativeMatrix) -> Mat4 {
    Mat4::from_cols(
        Vec3::from_array(matrix[0][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(matrix[1][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(matrix[2][..3].try_into().unwrap()).extend(0.0),
        Vec3::from_array(matrix[3][..3].try_into().unwrap()).extend(1.0),
    )
}

/// Vehicle clips use the same native model-space skeleton matrices as the SDK.
///
/// `layers` are applied in order, each easing the result towards one target pose
/// by its own weight. Steering is one layer and an air trick is another, which
/// is what lets a trick be a single authored frame rather than an animation: the
/// weight is how far the rider has thrown it, so a half-extended trick reads as
/// half-extended instead of snapping between two poses.
pub(crate) fn vehicle_pose(world: &mut World, pose: &[Mat4], layers: &[(&[Mat4], f32)]) {
    let board = world
        .resource::<crate::physics::SkaterRuntime>()
        .animation
        .evaluator
        .frames
        .bone_names
        .iter()
        .position(|n| n == "SKATEBOARD_ROOT");
    world.resource_scope(|world, animation: Mut<AnimationStatus>| {
        let basis = render_basis();
        for binding in &animation.bindings {
            let Some(global) = pose.get(binding.bone).map(|m| *m * basis) else {
                continue;
            };
            let local = if let Some(parent) = binding.parent_bone {
                let Some(parent) = pose.get(parent) else {
                    continue;
                };
                (*parent * basis).inverse() * global
            } else {
                global
            };
            let mut target = Transform::from_matrix(local);
            for (layer, weight) in layers {
                let weight = weight.clamp(0., 1.);
                if weight <= 0. {
                    continue;
                }
                if let Some(global) = layer.get(binding.bone).map(|m| *m * basis) {
                    let local = if let Some(parent) = binding.parent_bone {
                        layer
                            .get(parent)
                            .map(|p| (*p * basis).inverse() * global)
                            .unwrap_or(global)
                    } else {
                        global
                    };
                    target =
                        crate::presentation::blend(target, Transform::from_matrix(local), weight);
                }
            }
            if let Some(mut transform) = world.get_mut::<Transform>(binding.entity) {
                *transform = target;
                if Some(binding.bone) == board {
                    transform.scale = Vec3::splat(0.001);
                }
            }
        }
    });
}

pub(crate) fn capture_vehicle_visual(world: &mut World) -> Vec<(Entity, Transform)> {
    let mut ids: Vec<_> = world
        .resource::<AnimationStatus>()
        .bindings
        .iter()
        .map(|b| b.entity)
        .collect();
    ids.extend(
        world
            .query_filtered::<Entity, With<crate::world::PlayerRoot>>()
            .iter(world),
    );
    ids.into_iter()
        .filter_map(|id| world.get::<Transform>(id).copied().map(|t| (id, t)))
        .collect()
}
pub(crate) fn blend_vehicle_visual(world: &mut World, from: &[(Entity, Transform)], alpha: f32) {
    for &(id, previous) in from {
        if let Some(mut current) = world.get_mut::<Transform>(id) {
            *current = crate::presentation::blend(previous, *current, alpha);
        }
    }
}

/// Read the final displayed hierarchy, including vehicle steering and stance blends.
pub(crate) fn network_visual(world: &mut World) -> skate_net::packed::PoseState {
    let root = world
        .query_filtered::<&Transform, With<PlayerRoot>>()
        .iter(world)
        .next()
        .copied()
        .unwrap_or_default();
    let status = world.resource::<AnimationStatus>();
    let mut locals = std::collections::BTreeMap::new();
    for b in &status.bindings {
        if let Some(t) = world.get::<Transform>(b.entity) {
            locals
                .entry(b.bone)
                .or_insert((b.parent_bone, t.to_matrix()));
        }
    }
    fn global(
        i: usize,
        locals: &std::collections::BTreeMap<usize, (Option<usize>, Mat4)>,
        depth: usize,
    ) -> Mat4 {
        if depth > 128 {
            return Mat4::IDENTITY;
        }
        let Some(&(parent, m)) = locals.get(&i) else {
            return Mat4::IDENTITY;
        };
        parent.map_or(m, |p| global(p, locals, depth + 1) * m)
    }
    let anchors = crate::physics::network::anchors(world.resource::<SkaterRuntime>());
    let basis = render_basis().inverse();
    skate_net::packed::PoseState {
        root: crate::physics::network::pose(root.to_matrix()),
        bones: anchors
            .into_iter()
            .map(|i| skate_net::Bone {
                index: i as u16,
                pose: crate::physics::network::pose(if locals.contains_key(&i) {
                    global(i, &locals, 0) * basis
                } else {
                    native_matrix(world.resource::<SkaterRuntime>().render_pose[i])
                }),
            })
            .collect(),
    }
}
#[cfg(test)]
mod online_swap_tests {
    use super::*;
    use bevy::{ecs::system::SystemState, mesh::skinning::SkinnedMesh};
    #[test]
    fn online_appearance_swap_seeds_new_rig_and_keeps_animating() {
        let mut world = World::new();
        let mut roots = vec![];
        for _ in 0..2 {
            let root = world.spawn_empty().id();
            let hip = world
                .spawn((Name::new("HIPS"), Transform::default(), ChildOf(root)))
                .id();
            let head = world
                .spawn((Name::new("HEAD"), Transform::default(), ChildOf(hip)))
                .id();
            world.spawn((
                ChildOf(root),
                SkinnedMesh {
                    inverse_bindposes: default(),
                    joints: vec![hip, head],
                },
            ));
            roots.push((root, hip, head));
        }
        let names = vec!["HIPS".to_owned(), "HEAD".to_owned()];
        let bind = |world: &mut World, root| {
            let mut query: SystemState<(
                Query<(Entity, &SkinnedMesh)>,
                Query<(&Name, &Transform)>,
                Query<&ChildOf>,
            )> = SystemState::new(world);
            let (skins, nodes, parents) = query.get(world);
            AnimationStatus::for_scene(root, &names, &skins, &nodes, &parents).unwrap()
        };
        let old = bind(&mut world, roots[0].0);
        let pose = [
            Mat4::from_translation(Vec3::new(1., 2., 3.)),
            Mat4::from_rotation_z(0.7),
        ];
        for (entity, t) in old.pose_transforms(&pose) {
            *world.get_mut::<Transform>(entity).unwrap() = t;
        }
        let replacement = bind(&mut world, roots[1].0);
        assert_eq!(
            *world.get::<Transform>(roots[1].1).unwrap(),
            Transform::default()
        );
        for (entity, t) in replacement.pose_transforms(&pose) {
            *world.get_mut::<Transform>(entity).unwrap() = t;
        }
        assert_eq!(
            world.get::<Transform>(roots[0].1),
            world.get::<Transform>(roots[1].1)
        );
        assert_eq!(
            world.get::<Transform>(roots[0].2),
            world.get::<Transform>(roots[1].2)
        );
        let next = [
            Mat4::from_translation(Vec3::new(2., 3., 4.)),
            Mat4::from_rotation_z(1.2),
        ];
        for (entity, t) in replacement.pose_transforms(&next) {
            *world.get_mut::<Transform>(entity).unwrap() = t;
        }
        assert_ne!(
            world.get::<Transform>(roots[0].1),
            world.get::<Transform>(roots[1].1)
        );
        assert_ne!(
            world.get::<Transform>(roots[0].2),
            world.get::<Transform>(roots[1].2)
        );
    }
}
