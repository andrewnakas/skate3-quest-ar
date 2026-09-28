use super::appearance::{Appearances, Look, RemoteCharacter};
use super::*;
use crate::customiser_material::SkaterMaterial;
use bevy::{
    mesh::{morph::MorphWeights, skinning::SkinnedMesh},
    scene::SceneInstance,
};
struct Candidate {
    root: Entity,
    scenes: Vec<Entity>,
    look: Look,
    started: Instant,
}
#[derive(Component)]
struct OutfitPiece {
    id: String,
    mid: String,
}

use skate_net::interpolation::{Buffer, Clock, position};
#[derive(Resource)]
pub(super) struct RemoteSkins {
    reported: f64,
    actors: BTreeMap<u64, RemoteSkin>,
    rest: Vec<Mat4>,
    parents: Vec<i32>,
    board: Option<usize>,
}
#[derive(Default)]
struct RemoteSkin {
    root: Option<Entity>,
    bindings: Vec<(Entity, usize, Option<usize>)>,
    visible: Option<Entity>,
    pending: Option<Candidate>,
    requested: Option<[u8; 32]>,

    positions: Buffer<[f32; 3]>,
    roots: Buffer<Transform>,
    poses: Buffer<Vec<Transform>>,
    clock: Clock,
    epoch: u64,
}
pub(super) struct RemoteRenderPlugin;
impl Plugin for RemoteRenderPlugin {
    fn build(&self, app: &mut App) {
        let idle = app
            .world()
            .resource::<crate::assets::AssetManifest>()
            .0
            .initial_animation
            .clone();
        let skater = app.world().resource::<SkaterRuntime>();
        let pose = skater
            .animation
            .evaluator
            .evaluate(&[
                skate_core::animation::playback_tree::PoseCommand::Clip {
                    name: idle,
                    previous_time: 0.,
                    time: 0.,
                    loops: 0,
                },
                skate_core::animation::playback_tree::PoseCommand::Pose {
                    name: "RIG_TPOSE".into(),
                },
                skate_core::animation::playback_tree::PoseCommand::Add { motion_is_a: true },
            ])
            .unwrap_or_else(|_| skater.animation.pose.clone());
        let rest = pose
            .iter()
            .copied()
            .map(skate_core::animation::output::sqt_to_matrix)
            .map(crate::animation::native_matrix)
            .collect();
        let parents = skater
            .animation
            .evaluator
            .frames
            .parents
            .iter()
            .map(|&p| p as i32)
            .collect();
        let board = skater
            .animation
            .evaluator
            .frames
            .bone_names
            .iter()
            .position(|n| n == "SKATEBOARD_ROOT");
        app.insert_resource(RemoteSkins {
            board,
            reported: 0.,
            actors: BTreeMap::new(),
            rest,
            parents,
        })
        .add_systems(
            Update,
            (spawn, bind, present)
                .chain()
                .after(crate::modding::vehicles::present),
        );
    }
}
pub(super) fn spawn(
    mut commands: Commands,
    net: Res<Multiplayer>,
    looks: Res<Appearances>,
    mut skins: ResMut<RemoteSkins>,
    server: Res<AssetServer>,
    parts: Res<crate::customiser_parts::Parts>,
    models: Res<crate::custom_models::CustomModels>,
) {
    let removed: Vec<_> = skins
        .actors
        .keys()
        .filter(|id| !net.remotes.contains_key(id))
        .copied()
        .collect();
    for id in removed {
        if let Some(root) = skins.actors.remove(&id).and_then(|s| s.root) {
            commands.entity(root).despawn();
        }
    }
    for &id in net.remotes.keys() {
        let skin = skins.actors.entry(id).or_insert_with(|| RemoteSkin {
            clock: Clock::for_connection(net.loopback),
            ..default()
        });
        let root = *skin.root.get_or_insert_with(|| {
            commands
                .spawn((
                    RemoteCharacter,
                    Transform::default(),
                    Visibility::Inherited,
                    Name::new("Remote skater"),
                ))
                .id()
        });
        let compatible = net
            .lobby
            .as_ref()
            .and_then(|l| l.actors.get(&id))
            .is_some_and(|a| a.info.rig == net.info.rig);
        let (key, look) = if skin.visible.is_some() && compatible {
            looks
                .looks
                .get(&id)
                .cloned()
                .unwrap_or(([0; 32], Look::Stock))
        } else {
            ([0; 32], Look::Stock)
        };
        if skin.requested == Some(key) {
            continue;
        }
        skin.requested = Some(key);
        if let Some(old) = skin.pending.take() {
            commands.entity(old.root).despawn();
        }
        let mut scenes = vec![];
        let mut specs = vec![];
        let look = match look {
            Look::Outfit(profile) => match parts.resolve(&profile) {
                Ok(p) => Look::Outfit(p),
                Err(e) => {
                    warn!("Remote outfit: {e}");
                    Look::Stock
                }
            },
            other => other,
        };
        match &look {
            Look::Stock => specs.push(("private/skater.glb".to_owned(), None)),
            Look::Native(key) => {
                if let Some(path) = models.online_native_path(key) {
                    specs.push((path, None));
                } else {
                    specs.push(("private/skater.glb".to_owned(), None));
                }
            }
            Look::Imported(path) => specs.push((path.clone(), None)),
            Look::Outfit(profile) => {
                for v in profile["selections"]
                    .as_object()
                    .into_iter()
                    .flat_map(|s| s.values())
                {
                    let Some((id, mid)) = v["asset_id"].as_str().zip(v["material_id"].as_str())
                    else {
                        continue;
                    };
                    if let Some(part) = parts.library.models.get(id) {
                        specs.push((part.scene.clone(), Some((id.to_owned(), mid.to_owned()))));
                    }
                }
            }
        }
        if specs.is_empty() || specs.len() > 32 {
            continue;
        }
        let candidate = commands
            .spawn((Transform::default(), Visibility::Hidden, ChildOf(root)))
            .id();
        if let Look::Native(key) = &look {
            commands
                .entity(candidate)
                .insert(crate::custom_models::NativeModelRoot(key.clone()));
        }
        if matches!(look, Look::Imported(_)) {
            commands
                .entity(candidate)
                .insert(crate::custom_models::CustomModelRoot);
        }
        for (path, piece) in specs {
            let mut e = commands.spawn((
                SceneRoot(server.load(GltfAssetLabel::Scene(0).from_asset(path))),
                ChildOf(candidate),
            ));
            if let Some((id, mid)) = piece {
                e.insert((
                    crate::customiser_parts::PartRoot(id.clone()),
                    OutfitPiece { id, mid },
                ));
            }
            scenes.push(e.id());
        }
        skin.pending = Some(Candidate {
            root: candidate,
            scenes,
            look,
            started: Instant::now(),
        });
    }
}

fn bind(
    mut commands: Commands,
    net: Res<Multiplayer>,
    mut skins: ResMut<RemoteSkins>,
    skater: Res<SkaterRuntime>,
    meshes: Query<(Entity, &SkinnedMesh)>,
    nodes: Query<(&Name, &Transform)>,
    parents: Query<&ChildOf>,
    instances: Query<&SceneInstance>,
    spawner: Res<SceneSpawner>,
    server: Res<AssetServer>,
    mut parts: ResMut<crate::customiser_parts::Parts>,
    mut materials: ResMut<Assets<SkaterMaterial>>,
    pieces: Query<&OutfitPiece>,
    mut morphs: Query<(Entity, &mut MorphWeights)>,
) {
    let initial_poses: BTreeMap<_, _> = skins
        .actors
        .iter()
        .filter(|(_, skin)| skin.pending.is_some())
        .map(|(&id, _)| {
            let bones = net
                .remotes
                .get(&id)
                .and_then(|r| r.poses.back())
                .map(|p| p.bones.as_slice())
                .unwrap_or(&[]);
            (id, globals(bones, &skins))
        })
        .collect();
    for (id, skin) in skins.actors.iter_mut() {
        let Some(p) = skin.pending.as_ref() else {
            continue;
        };
        if p.started.elapsed() > Duration::from_secs(120) {
            warn!("Remote appearance loading timed out; previous skater retained");
            commands.entity(p.root).despawn();
            skin.pending = None;
            continue;
        }
        if p.scenes.iter().any(|&e| {
            !instances
                .get(e)
                .is_ok_and(|i| spawner.instance_is_ready(**i))
        }) {
            continue;
        }
        if let Look::Outfit(profile) = &p.look {
            for e in &p.scenes {
                if let Ok(piece) = pieces.get(*e) {
                    parts.warm(&piece.mid, &server, &mut materials);
                }
            }
            if !parts.tattoos_ready(profile, &server)
                || p.scenes.iter().any(|&e| {
                    pieces
                        .get(e)
                        .is_ok_and(|piece| !parts.material_ready(&piece.mid, &server))
                })
            {
                continue;
            }
        }
        let bindings = match crate::animation::AnimationStatus::for_scene(
            p.root,
            &skater.animation.evaluator.frames.bone_names,
            &meshes,
            &nodes,
            &parents,
        ) {
            Ok(b) => b.online_bindings(),
            Err(e) => {
                warn!("Remote appearance rejected: {e}");
                commands.entity(p.root).despawn();
                skin.pending = None;
                continue;
            }
        };
        if let Look::Outfit(profile) = &p.look {
            for &scene in &p.scenes {
                let Ok(piece) = pieces.get(scene) else {
                    continue;
                };
                let Some(material) =
                    parts.profile_material(&piece.id, &piece.mid, profile, &materials)
                else {
                    continue;
                };
                let handle = materials.add(material);
                for (e, _) in &meshes {
                    if parents.iter_ancestors(e).any(|p| p == scene) {
                        commands
                            .entity(e)
                            .remove::<MeshMaterial3d<StandardMaterial>>()
                            .insert(MeshMaterial3d(handle.clone()));
                    }
                }
            }
            let weights: Vec<f32> = parts
                .library
                .morphs
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    profile["morphs"][n]
                        .as_f64()
                        .unwrap_or(if (2..19).contains(&i) { 0.25 } else { 0. })
                        .clamp(0., 0.5) as f32
                })
                .collect();
            for (e, mut m) in &mut morphs {
                if parents.iter_ancestors(e).any(|e| e == p.root)
                    && m.weights().len() == weights.len()
                {
                    m.weights_mut().copy_from_slice(&weights);
                }
            }
        }
        for (e, _) in &meshes {
            if parents.iter_ancestors(e).any(|e| e == p.root) {
                commands.entity(e).insert((
                    bevy::camera::visibility::NoFrustumCulling,
                    bevy::camera::visibility::RenderLayers::from_layers(&[0, 28]),
                ));
            }
        }
        let pose = &initial_poses[id];
        let basis = Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W);
        for &(entity, bone, parent) in &bindings {
            let global = pose[bone] * basis;
            let local = parent.map_or(global, |parent| (pose[parent] * basis).inverse() * global);
            commands
                .entity(entity)
                .insert(Transform::from_matrix(local));
        }
        if let Some(old) = skin.visible.replace(p.root) {
            commands.entity(old).despawn();
        }
        commands.entity(p.root).insert(Visibility::Inherited);
        info!(
            "ONLINE_CHARACTER_VISIBLE peer={id} joints={} kind={}",
            bindings.len(),
            if matches!(p.look, Look::Imported(_)) {
                "import"
            } else {
                "retail"
            }
        );
        skin.bindings = bindings;
        skin.poses = Buffer::default();
        skin.pending = None;
    }
}

fn globals(bones: &[skate_net::Bone], skin: &RemoteSkins) -> Vec<Mat4> {
    let mut result = vec![None; skin.rest.len()];
    for b in bones {
        if let Some(slot) = result.get_mut(b.index as usize) {
            *slot = Some(network::matrix(b.pose));
        }
    }
    fn visit(i: usize, skin: &RemoteSkins, result: &mut [Option<Mat4>]) -> Mat4 {
        if let Some(m) = result[i] {
            return m;
        }
        let p = skin.parents[i];
        let matrix = if p >= 0 {
            visit(p as usize, skin, result) * skin.rest[i]
        } else {
            skin.rest[i]
        };
        result[i] = Some(matrix);
        matrix
    }
    for i in 0..result.len() {
        visit(i, skin, &mut result);
    }
    result.into_iter().map(Option::unwrap).collect()
}
fn present(
    mut net: ResMut<Multiplayer>,
    vehicles: Res<crate::modding::vehicles::Vehicles>,
    mut skins: ResMut<RemoteSkins>,
    mut nodes: Query<&mut Transform>,
) {
    let basis = Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W);
    let now = net.started.elapsed().as_secs_f64();
    for (&id, remote) in &net.remotes {
        let Some(mut skin) = skins.actors.remove(&id) else {
            continue;
        };
        if skin.epoch != remote.epoch {
            skin.positions = Buffer::default();
            skin.roots = Buffer::default();
            skin.poses = Buffer::default();
            skin.clock = Clock::for_connection(net.loopback);
            skin.epoch = remote.epoch;
        }
        for sample in &remote.roots {
            if skin
                .roots
                .samples
                .back()
                .is_some_and(|p| p.time >= sample.captured)
            {
                continue;
            }
            if skin.roots.insert(
                sample.captured,
                Transform::from_matrix(network::matrix(sample.pose)),
            ) {
                skin.positions.insert(sample.captured, sample.pose.p);
                skin.clock.observe(0, sample.captured, sample.received);
            }
        }
        // Each animation sample is resolved once; physics arrivals never duplicate it.
        if !skin.bindings.is_empty() {
            for sample in &remote.poses {
                if skin
                    .poses
                    .samples
                    .back()
                    .is_some_and(|p| p.time >= sample.captured)
                {
                    continue;
                }
                let g = globals(&sample.bones, &skins);
                let locals = skin
                    .bindings
                    .iter()
                    .map(|&(_, i, parent)| {
                        Transform::from_matrix(
                            parent
                                .map_or(g[i] * basis, |p| (g[p] * basis).inverse() * g[i] * basis),
                        )
                    })
                    .collect();
                if skin.poses.insert(sample.captured, locals) {
                    skin.clock.observe(1, sample.captured, sample.received);
                }
            }
        }
        if let (Some(root), Some(time)) = (skin.root, skin.clock.step(now)) {
            if let Some((a, b, alpha)) = skin.roots.pair(time) {
                let mut transform = crate::presentation::blend(
                    skin.roots.samples[a].value,
                    skin.roots.samples[b].value,
                    alpha,
                );
                if let Some(p) = position(&skin.positions, time) {
                    transform.translation = Vec3::from_array(p);
                }
                if let Ok(mut t) = nodes.get_mut(root) {
                    *t = transform;
                }
            }
            if let Some((a, b, alpha)) = skin.poses.pair(time) {
                let ag = &skin.poses.samples[a].value;
                let bg = &skin.poses.samples[b].value;
                for ((&(entity, _, _), a), b) in skin.bindings.iter().zip(ag).zip(bg) {
                    if let Ok(mut t) = nodes.get_mut(entity) {
                        *t = crate::presentation::blend(*a, *b, alpha);
                    }
                }
            }
        }
        if let Some(root) = skin.root {
            if let Some(attached) = crate::modding::vehicles::network::attached_root(&vehicles, id)
            {
                if let Ok(mut t) = nodes.get_mut(root) {
                    *t = attached;
                }
            }
        }
        let seated = remote.body.enabled & (1u64 << 62) != 0;
        for &(entity, i, _) in &skin.bindings {
            if skins.board == Some(i) {
                if let Ok(mut t) = nodes.get_mut(entity) {
                    t.scale = Vec3::splat(if seated { 0.001 } else { 1. });
                }
            }
        }
        skins.actors.insert(id, skin);
    }
    if now < skins.reported || now - skins.reported >= 1. {
        let delay = skins
            .actors
            .values()
            .map(|s| s.clock.delay)
            .fold(0., f64::max);
        let stalls: u64 = skins.actors.values().map(|s| s.clock.underruns).sum();
        net.visual_status = format!(
            "Visual interpolation: {:.0} ms target buffer | stalls {}",
            delay * 1000.,
            stalls
        );
        if net.active() {
            info!("MULTIPLAYER_INTERPOLATION {}", net.visual_status);
        }
        skins.reported = now;
    }
}

#[cfg(test)]
mod online_owned_tests {
    use super::*;
    #[test]
    #[ignore = "requires SKATE3_ASSET_ROOT prepared owned assets"]
    fn online_appearance_owned_male_and_female_outfits_bind_every_clothing_rig() {
        use bevy::{
            asset::AssetPlugin, ecs::system::SystemState, gltf::GltfPlugin, image::ImagePlugin,
            mesh::MeshPlugin, scene::ScenePlugin,
        };
        let assets = std::path::PathBuf::from(std::env::var("SKATE3_ASSET_ROOT").unwrap());
        let banks = skate_data::animation_banks::AnimationBanks::load(&assets).unwrap();
        let names = skate_data::animation_frames::AnimationFrames::from_banks(&banks)
            .unwrap()
            .bone_names;
        let library: crate::customiser_parts::Library = serde_json::from_slice(
            &std::fs::read(
                crate::customiser_parts::asset_directory(&assets).join("library-v3.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let parts = crate::customiser_parts::Parts::for_test(library);
        let mut app = App::new();
        crate::custom_models::register_source(&mut app);
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            },
            ImagePlugin::default(),
            MeshPlugin,
            ScenePlugin,
            GltfPlugin::default(),
        ));
        app.init_asset::<StandardMaterial>()
            .init_asset::<AnimationClip>()
            .register_type::<MeshMaterial3d<StandardMaterial>>();
        app.finish();
        app.cleanup();
        if let Ok(path) = std::env::var("SKATE_ONLINE_TEST_GLB") {
            let bytes = std::fs::read(path).unwrap();
            super::super::appearance::validate_glb(&bytes).unwrap();
            let cache = super::super::appearance::cache_directory();
            std::fs::create_dir_all(cache).unwrap();
            let mut payload = vec![0];
            payload.extend(bytes);
            let received = super::super::appearance_transfer::transfer_over_udp(payload);
            assert_eq!(received[0], 0);
            super::super::appearance::validate_glb(&received[1..]).unwrap();
            std::fs::write(cache.join("received-test.glb"), &received[1..]).unwrap();
            let handle = app
                .world()
                .resource::<AssetServer>()
                .load(GltfAssetLabel::Scene(0).from_asset("online-characters://received-test.glb"));
            let root = app
                .world_mut()
                .spawn((SceneRoot(handle), Transform::default(), Visibility::Hidden))
                .id();
            let started = Instant::now();
            loop {
                app.update();
                let w = app.world();
                if w.get::<SceneInstance>(root)
                    .is_some_and(|i| w.resource::<SceneSpawner>().instance_is_ready(**i))
                {
                    break;
                }
                assert!(
                    started.elapsed().as_secs() < 45,
                    "received online import failed to load"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let mut queries: SystemState<(
                Query<(Entity, &SkinnedMesh)>,
                Query<(&Name, &Transform)>,
                Query<&ChildOf>,
            )> = SystemState::new(app.world_mut());
            let (meshes, nodes, parents) = queries.get(app.world());
            let animation = crate::animation::AnimationStatus::for_scene(
                root, &names, &meshes, &nodes, &parents,
            )
            .unwrap();
            let joints = animation.online_bindings();
            assert!(!joints.is_empty());
            // A received model must accept poses before display, then keep moving.
            let first: Vec<_> = (0..names.len())
                .map(|i| Mat4::from_translation(Vec3::new(i as f32 * 0.01, 1., 0.)))
                .collect();
            for (joint, pose) in animation.pose_transforms(&first) {
                assert!(pose.to_matrix().is_finite());
                *app.world_mut().get_mut::<Transform>(joint).unwrap() = pose;
            }
            *app.world_mut().get_mut::<Visibility>(root).unwrap() = Visibility::Inherited;
            let second: Vec<_> = first
                .iter()
                .enumerate()
                .map(|(i, m)| *m * Mat4::from_rotation_z(0.01 * (i + 1) as f32))
                .collect();
            for (joint, pose) in animation.pose_transforms(&second) {
                assert!(pose.to_matrix().is_finite());
                assert_ne!(*app.world().get::<Transform>(joint).unwrap(), pose);
                *app.world_mut().get_mut::<Transform>(joint).unwrap() = pose;
            }
            app.world_mut().entity_mut(root).despawn();
            app.update();
            std::fs::remove_file(cache.join("received-test.glb")).unwrap();
        }
        for gender in ["male", "female"] {
            let profile = parts.resolve(&parts.library.defaults[gender]).unwrap();
            let candidate = app
                .world_mut()
                .spawn((Transform::default(), Visibility::Hidden))
                .id();
            let paths: Vec<_> = profile["selections"]
                .as_object()
                .unwrap()
                .values()
                .map(|v| {
                    parts.library.models[v["asset_id"].as_str().unwrap()]
                        .scene
                        .clone()
                })
                .collect();
            assert!(paths.len() > 3);
            let mut scenes = vec![];
            for path in paths {
                let handle = app
                    .world()
                    .resource::<AssetServer>()
                    .load(GltfAssetLabel::Scene(0).from_asset(path));
                scenes.push(
                    app.world_mut()
                        .spawn((SceneRoot(handle), ChildOf(candidate)))
                        .id(),
                );
            }
            let started = Instant::now();
            loop {
                app.update();
                let world = app.world();
                if scenes.iter().all(|&e| {
                    world
                        .get::<SceneInstance>(e)
                        .is_some_and(|i| world.resource::<SceneSpawner>().instance_is_ready(**i))
                }) {
                    break;
                }
                assert!(
                    started.elapsed().as_secs() < 45,
                    "{gender} scene loading timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let mut queries: SystemState<(
                Query<(Entity, &SkinnedMesh)>,
                Query<(&Name, &Transform)>,
                Query<&ChildOf>,
            )> = SystemState::new(app.world_mut());
            let (meshes, nodes, parents) = queries.get(app.world());
            let binding = crate::animation::AnimationStatus::for_scene(
                candidate, &names, &meshes, &nodes, &parents,
            )
            .unwrap()
            .online_bindings();
            let expected: std::collections::HashSet<_> = meshes
                .iter()
                .filter(|(e, _)| parents.iter_ancestors(*e).any(|e| e == candidate))
                .flat_map(|(_, m)| m.joints.iter().copied())
                .collect();
            assert_eq!(binding.len(), expected.len());
            for scene in scenes {
                assert!(
                    binding
                        .iter()
                        .any(|(e, _, _)| parents.iter_ancestors(*e).any(|e| e == scene)),
                    "all outfit scene rigs must animate"
                );
            }
            app.world_mut().entity_mut(candidate).despawn();
            app.update();
        }
    }
}
