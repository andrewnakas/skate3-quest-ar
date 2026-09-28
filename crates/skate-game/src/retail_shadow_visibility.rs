//! Conservative spatial acceleration of Bevy directional shadow visibility.
//! Leaves use Bevy's original OBB test; only static map meshes enter the index.
use bevy::{
    camera::{
        primitives::{Aabb, CascadesFrusta, Frustum},
        visibility::{
            CascadesVisibleEntities, NoFrustumCulling, RenderLayers, SetViewVisibility,
            VisibilityRange, VisibilitySystems, VisibleEntityRanges,
        },
    },
    light::{NotShadowCaster, SimulationLightSystems},
    math::Affine3A,
    prelude::*,
    transform::TransformSystems,
};

#[derive(Component)]
pub(crate) struct StaticShadowCaster;
#[derive(Clone)]
struct Entry {
    entity: Entity,
    aabb: Aabb,
    transform: GlobalTransform,
    visible: bool,
    layers: RenderLayers,
}
impl Entry {
    fn bounds(&self) -> Aabb {
        let affine = self.transform.affine();
        Aabb {
            center: affine.transform_point3a(self.aabb.center),
            half_extents: affine.matrix3.abs() * self.aabb.half_extents
                + bevy::math::Vec3A::splat(0.01),
        }
    }
}
struct Node {
    bounds: Aabb,
    layers: RenderLayers,
    range: std::ops::Range<usize>,
    children: Option<(usize, usize)>,
}
#[derive(Resource, Default)]
struct Index {
    entries: Vec<Entry>,
    nodes: Vec<Node>,
    initialized: bool,
}
impl Index {
    fn rebuild(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.nodes.clear();
        if !self.entries.is_empty() {
            self.build(0..self.entries.len());
        }
        self.initialized = true;
    }
    fn build(&mut self, range: std::ops::Range<usize>) -> usize {
        let mut min = bevy::math::Vec3A::splat(f32::INFINITY);
        let mut max = -min;
        let mut layers = RenderLayers::none();
        for entry in &self.entries[range.clone()] {
            layers = layers.union(&entry.layers);
            let bounds = entry.bounds();
            min = min.min(bounds.min());
            max = max.max(bounds.max());
        }
        let bounds = Aabb {
            center: (min + max) * 0.5,
            half_extents: (max - min) * 0.5 + bevy::math::Vec3A::splat(0.01),
        };
        let node = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            layers,
            range: range.clone(),
            children: None,
        });
        if range.len() > 16 {
            let axis = if bounds.half_extents.x >= bounds.half_extents.y
                && bounds.half_extents.x >= bounds.half_extents.z
            {
                0
            } else if bounds.half_extents.y >= bounds.half_extents.z {
                1
            } else {
                2
            };
            self.entries[range.clone()].sort_unstable_by(|a, b| {
                a.bounds().center[axis].total_cmp(&b.bounds().center[axis])
            });
            let middle = range.start + range.len() / 2;
            let left = self.build(range.start..middle);
            let right = self.build(middle..range.end);
            self.nodes[node].children = Some((left, right));
        }
        node
    }
    fn visit(
        &self,
        node: usize,
        frustum: &Frustum,
        layers: &RenderLayers,
        output: &mut Vec<Entity>,
    ) {
        let node = &self.nodes[node];
        if !layers.intersects(&node.layers) {
            return;
        }
        if node.bounds.center.is_finite()
            && node.bounds.half_extents.is_finite()
            && !frustum.intersects_obb(&node.bounds, &Affine3A::IDENTITY, false, true)
        {
            return;
        }
        if let Some((left, right)) = node.children {
            self.visit(left, frustum, layers, output);
            self.visit(right, frustum, layers, output);
        } else {
            for entry in &self.entries[node.range.clone()] {
                if entry.visible
                    && layers.intersects(&entry.layers)
                    && frustum.intersects_obb(&entry.aabb, &entry.transform.affine(), false, true)
                {
                    output.push(entry.entity);
                }
            }
        }
    }
}

type StaticFilter = (
    With<StaticShadowCaster>,
    With<Mesh3d>,
    Without<NotShadowCaster>,
    Without<DirectionalLight>,
    Without<VisibilityRange>,
    Without<NoFrustumCulling>,
);
fn refresh(
    mut index: ResMut<Index>,
    entries: Query<
        (
            Entity,
            &Aabb,
            &GlobalTransform,
            &InheritedVisibility,
            Option<&RenderLayers>,
        ),
        StaticFilter,
    >,
    changed: Query<
        (),
        (
            With<StaticShadowCaster>,
            Or<(
                Changed<Aabb>,
                Changed<GlobalTransform>,
                Changed<InheritedVisibility>,
                Changed<RenderLayers>,
                Added<StaticShadowCaster>,
            )>,
        ),
    >,
    mut removed_layers: RemovedComponents<RenderLayers>,
    mut removed_excluded: RemovedComponents<NotShadowCaster>,
    mut removed_range: RemovedComponents<VisibilityRange>,
    mut removed_no_cull: RemovedComponents<NoFrustumCulling>,
) {
    // Consume every event reader even when an earlier condition is true.
    let removed = removed_layers.read().count()
        + removed_excluded.read().count()
        + removed_range.read().count()
        + removed_no_cull.read().count();
    if index.initialized
        && index.entries.len() == entries.iter().len()
        && changed.is_empty()
        && removed == 0
    {
        return;
    }
    index.rebuild(
        entries
            .iter()
            .map(|(entity, aabb, transform, visibility, layers)| Entry {
                entity,
                aabb: *aabb,
                transform: *transform,
                visible: visibility.get(),
                layers: layers.cloned().unwrap_or_default(),
            })
            .collect(),
    );
}
fn check_indexed(
    mut commands: Commands,
    index: Res<Index>,
    mut lights: Query<
        (
            &DirectionalLight,
            &CascadesFrusta,
            &mut CascadesVisibleEntities,
            Option<&RenderLayers>,
            &ViewVisibility,
        ),
        Without<SpotLight>,
    >,
    dynamic: Query<
        (
            Entity,
            &InheritedVisibility,
            Option<&RenderLayers>,
            Option<&Aabb>,
            Option<&GlobalTransform>,
            Has<VisibilityRange>,
            Has<NoFrustumCulling>,
        ),
        (
            With<Mesh3d>,
            Without<NotShadowCaster>,
            Without<DirectionalLight>,
            Or<(
                Without<StaticShadowCaster>,
                With<VisibilityRange>,
                With<NoFrustumCulling>,
                Without<Aabb>,
                Without<GlobalTransform>,
            )>,
        ),
    >,
    ranges: Option<Res<VisibleEntityRanges>>,
) {
    let mut marked = Vec::new();
    for (light, frusta, mut visible, layers, light_visibility) in &mut lights {
        visible
            .entities
            .retain(|view, _| frusta.frusta.contains_key(view));
        for (view, frusta) in &frusta.frusta {
            let lists = visible.entities.entry(*view).or_default();
            lists.resize(frusta.len(), Default::default());
            for list in lists.iter_mut() {
                list.clear();
            }
            if !light.shadows_enabled || !light_visibility.get() {
                continue;
            }
            let layers = layers.cloned().unwrap_or_default();
            for (frustum, list) in frusta.iter().zip(lists.iter_mut()) {
                if !index.nodes.is_empty() {
                    index.visit(0, frustum, &layers, &mut list.entities);
                }
            }
            for (entity, inherited, mask, aabb, transform, range, no_cull) in &dynamic {
                if !inherited.get()
                    || !layers.intersects(mask.unwrap_or_default())
                    || (range
                        && ranges
                            .as_ref()
                            .is_some_and(|r| !r.entity_is_in_range_of_view(entity, *view)))
                {
                    continue;
                }
                for (frustum, list) in frusta.iter().zip(lists.iter_mut()) {
                    if no_cull
                        || aabb.zip(transform).is_none_or(|(a, t)| {
                            frustum.intersects_obb(a, &t.affine(), false, true)
                        })
                    {
                        list.push(entity);
                    }
                }
            }
            for list in lists.iter() {
                marked.extend(list.iter().copied());
            }
        }
    }
    // Match upstream's deferred writes so point-light checks may run in parallel.
    commands.queue(move |world: &mut World| {
        for entity in marked {
            if let Some(mut visible) = world.get_mut::<ViewVisibility>(entity) {
                visible.set_visible();
            }
        }
    });
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Index>().add_systems(
        PostUpdate,
        (
            refresh
                .after(VisibilitySystems::CalculateBounds)
                .after(TransformSystems::Propagate)
                .after(VisibilitySystems::VisibilityPropagate)
                .before(check_indexed),
            check_indexed
                .in_set(SimulationLightSystems::CheckLightVisibility)
                .after(VisibilitySystems::CalculateBounds)
                .after(TransformSystems::Propagate)
                .after(SimulationLightSystems::UpdateLightFrusta)
                .after(VisibilitySystems::CheckVisibility)
                .before(VisibilitySystems::MarkNewlyHiddenEntitiesInvisible),
        ),
    );
}
pub(super) fn finish(app: &mut App) {
    app.world_mut()
        .schedule_scope(PostUpdate, |world, schedule| {
            let removed = schedule
                .remove_systems_in_set(
                    bevy::light::check_dir_light_mesh_visibility,
                    world,
                    bevy::ecs::schedule::ScheduleCleanupPolicy::RemoveSystemsOnly,
                )
                .expect("replace directional visibility system");
            assert_eq!(
                removed, 1,
                "exactly one upstream directional visibility system"
            );
        });
}

#[cfg(test)]
#[path = "retail_shadow_visibility_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn benchmark_bounds(bounds: Vec<Aabb>) -> serde_json::Value {
    tests::benchmark_bounds(bounds)
}
