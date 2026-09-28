//! Shadow-only batches for static opaque retail geometry. Main meshes and all
//! collision data remain untouched. Alpha-tested/blended casters keep their own
//! materials so foliage silhouettes and transparency are not approximated.
use crate::map_render::AssetSink;
use bevy::{
    camera::visibility::{RenderLayers, VisibilitySystems},
    prelude::*,
};
use std::collections::HashMap;

pub(crate) const LAYER: usize = 27;
const CELL: f32 = 64.;

pub(super) fn eligible(material: &crate::retail_render::RetailWorldMaterial) -> bool {
    matches!(material.alpha, AlphaMode::Opaque) && material.params.mode.z < 0.
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(
        PostUpdate,
        include_world_layer.before(VisibilitySystems::CheckVisibility),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn shadow_layer_reaches_world_lights_but_not_cameras_or_player_only_lights() {
        let mut world = World::new();
        let directional = world.spawn(DirectionalLight::default()).id();
        let point = world.spawn(PointLight::default()).id();
        let spot = world.spawn(SpotLight::default()).id();
        let player = world
            .spawn((DirectionalLight::default(), RenderLayers::layer(28)))
            .id();
        let isolated = world
            .spawn((PointLight::default(), RenderLayers::layer(9)))
            .id();
        let camera = world
            .spawn((Camera3d::default(), RenderLayers::from_layers(&[0, 28])))
            .id();
        world.run_system_once(include_world_layer).unwrap();
        for entity in [directional, point, spot] {
            let layers = world.get::<RenderLayers>(entity).unwrap();
            assert!(layers.intersects(&RenderLayers::default()));
            assert!(layers.intersects(&RenderLayers::layer(LAYER)));
        }
        for entity in [player, isolated, camera] {
            assert!(
                !world
                    .get::<RenderLayers>(entity)
                    .unwrap()
                    .intersects(&RenderLayers::layer(LAYER))
            );
        }
    }

    #[test]
    fn spatial_shadow_batches_keep_boundary_crossing_triangles_normals_and_cull_modes() {
        let mut map = skate_data::skate_map::SkateMap::parse(include_bytes!(
            "../../../maps/format-demo.skate"
        ))
        .unwrap();
        map.geometry.vertices.truncate(3);
        for (v, p) in
            map.geometry
                .vertices
                .iter_mut()
                .zip([[-130., 0., 0.], [130., 0., 0.], [0., 200., 1.]])
        {
            v.position = p;
        }
        map.geometry.vertices[0].normal = [-0., 0.25, 0.75];
        let mut geometry = ShadowGeometry::default();
        geometry.add(&map.geometry, &[0, 1, 2], false);
        geometry.add(&map.geometry, &[2, 1, 0], true);
        assert_eq!(geometry.batches.len(), 2);
        assert_eq!(geometry.batches[0], (false, vec![0, 1, 2]));
        assert_eq!(geometry.batches[1], (true, vec![2, 1, 0]));
        let mut commands = crate::map_render::SceneCommands::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        geometry.spawn(&map.geometry, &mut commands, &mut meshes, &mut materials);
        // Scene publication is tested through the normal map-retirement path;
        // inspect generated CPU mesh data here without constructing a renderer.
        assert_eq!(meshes.len(), 2);
        for (_, mesh) in meshes.iter() {
            let bevy::mesh::VertexAttributeValues::Float32x3(positions) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!()
            };
            assert!(positions.contains(&[-130., 0., 0.]));
            assert!(positions.contains(&[130., 0., 0.]));
            let bevy::mesh::VertexAttributeValues::Float32x3(normals) =
                mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap()
            else {
                panic!()
            };
            assert!(
                normals
                    .iter()
                    .any(|n| n.map(f32::to_bits) == [-0.0f32, 0.25, 0.75].map(f32::to_bits))
            );
        }
        assert!(materials.iter().any(|(_, m)| m.cull_mode.is_none()));
        assert!(
            materials
                .iter()
                .any(|(_, m)| m.cull_mode == Some(bevy::render::render_resource::Face::Back))
        );
    }
}

fn include_world_layer(
    mut commands: Commands,
    lights: Query<
        (Entity, Option<&RenderLayers>),
        Or<(With<DirectionalLight>, With<PointLight>, With<SpotLight>)>,
    >,
) {
    for (entity, layers) in &lights {
        let layers = layers.cloned().unwrap_or_default();
        // World lights gain the shadow-only geometry. Player-only layer 28 and
        // other isolated light masks retain their exact existing membership.
        if layers.intersects(&RenderLayers::default())
            && !layers.intersects(&RenderLayers::layer(LAYER))
        {
            commands.entity(entity).insert(layers.with(LAYER));
        }
    }
}

#[derive(Default)]
pub(super) struct ShadowGeometry {
    lookup: HashMap<([i32; 3], bool), usize>,
    pub(super) batches: Vec<(bool, Vec<u32>)>,
}
impl ShadowGeometry {
    pub(super) fn add(
        &mut self,
        geometry: &skate_data::skate_map::Geometry,
        indices: &[u32],
        two_sided: bool,
    ) {
        for triangle in indices.chunks_exact(3) {
            let center = triangle
                .iter()
                .map(|&i| Vec3::from_array(geometry.vertices[i as usize].position) / 3.)
                .sum::<Vec3>();
            let cell = center.to_array().map(|v| (v / CELL).floor() as i32);
            let batch = *self.lookup.entry((cell, two_sided)).or_insert_with(|| {
                let index = self.batches.len();
                self.batches.push((two_sided, Vec::new()));
                index
            });
            // Full triangles, including those crossing a cell boundary. Bevy
            // derives conservative bounds from all their positions, not the cell.
            self.batches[batch].1.extend_from_slice(triangle);
        }
    }

    pub(super) fn spawn(
        self,
        geometry: &skate_data::skate_map::Geometry,
        commands: &mut crate::map_render::SceneCommands,
        meshes: &mut impl AssetSink<Mesh>,
        materials: &mut impl AssetSink<StandardMaterial>,
    ) {
        eprintln!(
            "SKATE_SHADOW_BATCHES count={} triangles={}",
            self.batches.len(),
            self.batches.iter().map(|(_, i)| i.len() / 3).sum::<usize>()
        );
        let mut handles: [Option<Handle<StandardMaterial>>; 2] = [None, None];
        for (two_sided, indices) in self.batches {
            let material = handles[usize::from(two_sided)]
                .get_or_insert_with(|| {
                    materials.add(StandardMaterial {
                        unlit: true,
                        double_sided: two_sided,
                        cull_mode: if two_sided {
                            None
                        } else {
                            Some(bevy::render::render_resource::Face::Back)
                        },
                        ..default()
                    })
                })
                .clone();
            let mut remap = HashMap::new();
            let mut positions = Vec::new();
            let mut normals = Vec::new();
            let local: Vec<u32> = indices
                .into_iter()
                .map(|i| {
                    *remap.entry(i).or_insert_with(|| {
                        let v = &geometry.vertices[i as usize];
                        let index = positions.len() as u32;
                        positions.push(v.position);
                        // Shadow normal bias still sees the original normal, not a
                        // recomputed face normal from simplified geometry.
                        normals.push(v.normal);
                        index
                    })
                })
                .collect();
            let mesh = Mesh::new(
                bevy::mesh::PrimitiveTopology::TriangleList,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_indices(bevy::mesh::Indices::U32(local));
            commands.spawn((
                Name::new("Retail opaque shadow batch"),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material),
                Transform::default(),
                RenderLayers::layer(LAYER),
            ));
        }
    }
}
