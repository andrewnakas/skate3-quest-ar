//! Native vehicle host: isolated Rapier world, mod ownership and driver lifecycle.
mod animations;
mod audio;
mod engine_sound;
mod interpolation;
pub(crate) mod network;
use bevy::prelude::*;
use serde_json::{Value, json};
use skate_mods::Command;
use skate_vehicles::{Controls, Simulation, VehicleDefinition};
use std::collections::BTreeMap;
/// A model node that is only correct while the bike is parked -- the side
/// stand. Tagged once on the way past, like `WheelVisual`, so the per-frame
/// work is a visibility write rather than a name search.
#[derive(Component)]
struct ParkedOnly(u64);

#[derive(Component, Clone)]
struct WheelVisual {
    vehicle: u64,
    index: usize,
    rest: Transform,
}
struct Instance {
    id: u64,
    entity: Entity,
    scene: Handle<Scene>,
    clips: animations::Clips,
    last_control: f32,
    definition_path: String,
}
struct Driver {
    owner: String,
    key: String,
    phase: &'static str,
    time: f32,
    return_position: [f32; 3],
    return_heading: f32,
}
#[derive(Resource, Default)]
pub(crate) struct Vehicles {
    simulation: Simulation,
    remote: BTreeMap<u64, network::Target>,
    skaters: BTreeMap<(u64, usize), skate_vehicles::rapier3d::prelude::RigidBodyHandle>,
    owned: BTreeMap<(String, String), Instance>,
    driver: Option<Driver>,
    pub(super) events: Vec<Value>,
    clock: f32,
    hidden: Vec<(Entity, Visibility)>,
    pub(crate) pose: Option<Vec<Mat4>>,
    pub(crate) network_pose: Option<skate_net::packed::PoseState>,
    last_visual: Vec<(Entity, Transform)>,
    blend_from: Vec<(Entity, Transform)>,
    visual_phase: String,
    blend_time: f32,
    steering_visual: f32,
    crash_handoff: bool,
    previous_motion: BTreeMap<u64, interpolation::Motion>,
    rendered_motion: BTreeMap<u64, interpolation::Motion>,
    posture: Posture,
    follow: Follow,
}

/// Where the chase camera is looking, smoothed.
///
/// The camera used to take its direction straight from the chassis, projected
/// flat. That works while the bike is upright and fails completely the moment
/// it is not: halfway through a backflip the forward axis points at the sky,
/// its horizontal projection is nearly zero, and the direction it normalises
/// to is noise. The camera snapped through a half turn and back on every flip.
///
/// So the yaw is a filtered state rather than a per-frame reading, and what it
/// follows is *where the bike is going*, not where it is pointing. A whip
/// swings the bike most of a quarter turn and lands it straight; a camera
/// bolted to the nose swings through all of that and reads as the world
/// spinning rather than the bike going sideways.
#[derive(Default)]
struct Follow {
    yaw: f32,
    ready: bool,
}

impl Follow {
    /// Shortest signed angle from `from` to `to`, wrapped to +/-pi. Heading
    /// crosses the wrap point in the middle of an ordinary corner, and an
    /// unwrapped difference reads that as most of a rotation the wrong way.
    fn shortest(from: f32, to: f32) -> f32 {
        let mut d = (to - from) % std::f32::consts::TAU;
        if d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        } else if d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        d
    }

    fn approach(&mut self, target: f32, rate: f32, dt: f32) {
        if !self.ready {
            self.yaw = target;
            self.ready = true;
            return;
        }
        self.yaw += Self::shortest(self.yaw, target) * (1. - (-rate * dt).exp());
    }
}

/// How much of each rider posture the ride is currently asking for, 0..1 each.
/// Smoothed, because these follow physics state that steps at the fixed rate
/// and a rider who snapped between stances would read as a glitch rather than
/// as a rider.
#[derive(Default)]
struct Posture {
    stand: f32,
    crouch: f32,
    back: f32,
    forward: f32,
    lean: f32,
}

impl Posture {
    /// Ease every weight toward its target with a frame-rate independent
    /// filter. Standing up is quicker than sitting back down, the way a rider
    /// pops up for a jump and settles afterwards.
    fn approach(&mut self, target: &Posture, dt: f32) {
        let ease = |current: &mut f32, want: f32, rate: f32| {
            *current += (want - *current) * (1. - (-rate * dt).exp());
        };
        let popping = target.stand > self.stand;
        ease(&mut self.stand, target.stand, if popping { 14. } else { 7. });
        ease(&mut self.crouch, target.crouch, 16.);
        ease(&mut self.back, target.back, 10.);
        ease(&mut self.forward, target.forward, 10.);
        ease(&mut self.lean, target.lean, 8.);
    }
}
#[cfg(test)]
mod follow_tests {
    use super::Follow;

    /// The wrap is the whole reason this is not a lerp: a bike pointing just
    /// west of north and a camera just east of it are two degrees apart, and
    /// a naive difference calls it 358 and spins the camera the long way.
    #[test]
    fn the_shortest_way_round_is_taken_across_the_wrap() {
        let pi = std::f32::consts::PI;
        assert!((Follow::shortest(pi - 0.05, -pi + 0.05) - 0.1).abs() < 1e-4);
        assert!((Follow::shortest(-pi + 0.05, pi - 0.05) + 0.1).abs() < 1e-4);
        assert!((Follow::shortest(0., 1.) - 1.).abs() < 1e-6);
    }

    #[test]
    fn the_first_frame_snaps_and_the_rest_ease() {
        let mut f = Follow::default();
        f.approach(2., 5., 1. / 60.);
        assert_eq!(f.yaw, 2., "a fresh camera should start where it is aimed");
        f.approach(3., 5., 1. / 60.);
        assert!(f.yaw > 2. && f.yaw < 2.2, "should ease, not jump: {}", f.yaw);
    }

    /// A backflip spins the chassis through every heading. The camera is fed
    /// the direction of travel instead, so it should barely move -- this is
    /// the failure the filter exists to prevent.
    #[test]
    fn a_flip_does_not_swing_the_camera() {
        let mut f = Follow::default();
        f.approach(0., 5., 1. / 60.);
        let mut worst: f32 = 0.;
        for tick in 0..120 {
            // Travel stays north through the whole rotation.
            f.approach(0., 1.6, 1. / 60.);
            worst = worst.max(f.yaw.abs());
            let _ = tick;
        }
        assert!(worst < 0.01, "camera drifted {worst} rad while flipping");
    }

    #[test]
    fn a_whip_is_followed_smoothly_rather_than_snapped_to() {
        let mut f = Follow::default();
        f.approach(0., 5., 1. / 60.);
        // Bike sent 70 degrees sideways; the camera should take real time.
        let target = 1.22_f32;
        f.approach(target, 1.6, 1. / 60.);
        assert!(f.yaw < target * 0.1, "snapped to the whip: {}", f.yaw);
        for _ in 0..90 {
            f.approach(target, 1.6, 1. / 60.);
        }
        assert!(
            (f.yaw - target).abs() < 0.2,
            "should have caught up by now: {}",
            f.yaw
        );
    }
}

impl Vehicles {
    pub(crate) fn occupied(&self) -> bool {
        self.driver.is_some()
    }
    pub(super) fn player_pose(&self) -> Option<([f32; 3], f32)> {
        let d = self.driver.as_ref()?;
        let i = self.owned.get(&(d.owner.clone(), d.key.clone()))?;
        let (p, q) = self.simulation.pose(i.id)?;
        let q = Quat::from_array(q);
        let forward = q * Vec3::Z;
        let seat = Vec3::from_array(self.simulation.vehicles[&i.id].definition.seat);
        Some((
            (Vec3::from_array(p) + q * seat).to_array(),
            forward.x.atan2(forward.z),
        ))
    }
    pub(crate) fn camera(&self) -> Option<Transform> {
        let d = self.driver.as_ref()?;
        let i = self.owned.get(&(d.owner.clone(), d.key.clone()))?;
        let pose = self.rendered_motion.get(&i.id)?.body;
        let q = pose.rotation;
        let def = &self.simulation.vehicles[&i.id].definition;
        let center = pose.translation + Vec3::Y * 0.5;
        let _ = q;
        // `follow.yaw` is filtered in `present`; here it is only read, so the
        // camera cannot inherit a frame of chassis noise.
        let (sin, cos) = self.follow.yaw.sin_cos();
        let forward = Vec3::new(sin, 0., cos);
        Some(
            Transform::from_translation(
                center - forward * def.camera_distance + Vec3::Y * def.camera_height,
            )
            .looking_at(center + forward, Vec3::Y),
        )
    }
}
pub(super) fn install(app: &mut App) {
    audio::install(app);
    app.init_resource::<Vehicles>()
        .add_systems(
            FixedUpdate,
            tick.after(super::fixed).run_if(network::simulation_active),
        )
        .add_systems(
            Update,
            present
                .after(crate::app::FrameSet::Animation)
                .before(crate::camera::present),
        );
}
pub(super) fn snapshot(world: &World) -> Value {
    let v = world.resource::<Vehicles>();
    let mut out = serde_json::Map::new();
    for ((owner, key), instance) in &v.owned {
        let Some((p, q)) = v.simulation.pose(instance.id) else {
            continue;
        };
        let car = &v.simulation.vehicles[&instance.id];
        let phase = v
            .driver
            .as_ref()
            .filter(|d| d.owner == *owner && d.key == *key)
            .map_or("parked", |d| d.phase);
        // Velocity, spin and wheel contacts are what a freestyle scorer needs:
        // rotation accumulates from the angular velocity, and the contact count
        // is what separates an air from a landing.
        let (linear, angular, contacts) = v
            .simulation
            .telemetry(instance.id)
            .unwrap_or(([0.; 3], [0.; 3], 0));
        out.entry(owner.clone()).or_insert(json!({})).as_object_mut().unwrap().insert(key.clone(),json!({"position":p,"rotation":q,"heading":({let f=Quat::from_array(q)*Vec3::Z;f.x.atan2(f.z)}),"speed":car.controller.current_vehicle_speed,"velocity":linear,"angular_velocity":angular,"wheel_contacts":contacts,"airborne":contacts==0,"wheel_speed":v.simulation.wheel_speed(instance.id),"phase":phase,"occupied":phase!="parked","ready":world.resource::<AssetServer>().is_loaded_with_dependencies(instance.scene.id())}));
    }
    Value::Object(out)
}
fn event(v: &mut Vehicles, owner: &str, key: &str, name: &str) {
    v.events.push(json!({"name":name,"owner":owner,"key":key}));
}
fn exit_now(world: &mut World, v: &mut Vehicles, forced: bool) -> Result<(), String> {
    let Some(driver) = v.driver.take() else {
        return Ok(());
    };
    let mut position = driver.return_position;
    let mut heading = driver.return_heading;
    if !forced {
        if let Some(i) = v.owned.get(&(driver.owner.clone(), driver.key.clone())) {
            if let Some((p, q)) = v.simulation.pose(i.id) {
                let q = Quat::from_array(q);
                let offset = Vec3::from_array(v.simulation.vehicles[&i.id].definition.exit);
                let candidate = (Vec3::from_array(p) + q * offset).to_array();
                if let Some(floor) = v.simulation.floor(candidate) {
                    position = [floor[0], floor[1] + 0.15, floor[2]];
                    let forward = q * Vec3::Z;
                    heading = forward.x.atan2(forward.z);
                }
            }
        }
    }
    if let Some(i) = v.owned.get(&(driver.owner.clone(), driver.key.clone())) {
        v.simulation.set_occupied(i.id, false);
    }
    v.pose = None;
    for (entity, visibility) in v.hidden.drain(..) {
        if let Some(mut current) = world.get_mut::<Visibility>(entity) {
            *current = visibility;
        }
    }
    let (sin, cos) = heading.sin_cos();
    let matrix = [
        [cos, 0., -sin, 0.],
        [0., 1., 0., 0.],
        [sin, 0., cos, 0.],
        [position[0], position[1], position[2], 0.],
    ];
    let mut skater = world.resource_mut::<crate::physics::SkaterRuntime>();
    if skater.player_input.pending_teleport().is_none() {
        skater
            .player_input
            .request_teleport(matrix)
            .map_err(|e| e.to_string())?;
        skater.teleport_state.request_manual(matrix, false);
    }
    event(v, &driver.owner, &driver.key, "vehicle_exited");
    Ok(())
}
fn eject_now(
    world: &mut World,
    v: &mut Vehicles,
    ejection: skate_vehicles::Ejection,
) -> Result<(), String> {
    let Some(driver) = v.driver.as_ref() else {
        return Ok(());
    };
    let owner = driver.owner.clone();
    let key = driver.key.clone();
    let id = v.owned[&(owner.clone(), key.clone())].id;
    let (_, rotation) = v.simulation.pose(id).ok_or("Missing crash vehicle")?;
    let forward = Quat::from_array(rotation) * Vec3::Z;
    let heading = forward.x.atan2(forward.z);
    let (sin, cos) = heading.sin_cos();
    // The native reset initializes a full upright body. Start clear of the seat,
    // then its ordinary ragdoll/contact solver takes over immediately.
    let p = Vec3::from_array(ejection.position) + Vec3::Y * 0.25;
    let matrix = [
        [cos, 0., -sin, 0.],
        [0., 1., 0., 0.],
        [sin, 0., cos, 0.],
        [p.x, p.y, p.z, 0.],
    ];
    {
        let mut skater = world.resource_mut::<crate::physics::SkaterRuntime>();
        skater
            .player_input
            .request_teleport(matrix)
            .map_err(|e| e.to_string())?;
        skater.teleport_state.request_vehicle_ejection(
            matrix,
            ejection.velocity,
            ejection.angular_velocity,
        );
    }
    v.simulation.set_occupied(id, false);
    v.simulation.vehicles.get_mut(&id).unwrap().controls = Controls {
        brake: 0.2,
        ..Default::default()
    };
    v.driver = None;
    v.pose = None;
    v.crash_handoff = true;
    for (entity, visibility) in v.hidden.drain(..) {
        if let Some(mut current) = world.get_mut::<Visibility>(entity) {
            *current = visibility;
        }
    }
    v.events
        .push(json!({"name":"vehicle_bailed","owner":owner,"key":key,
        "reason":ejection.reason,"position":ejection.position,"velocity":ejection.velocity,
        "angular_velocity":ejection.angular_velocity}));
    Ok(())
}
pub(super) fn retire(world: &mut World, owner: &str) {
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        if v.driver.as_ref().is_some_and(|d| d.owner == owner) {
            let _ = exit_now(world, &mut v, true);
        }
        let keys: Vec<_> = v
            .owned
            .keys()
            .filter(|(o, _)| o == owner)
            .cloned()
            .collect();
        for key in keys {
            if let Some(i) = v.owned.remove(&key) {
                v.simulation.remove(i.id);
                v.previous_motion.remove(&i.id);
                v.rendered_motion.remove(&i.id);
                v.remote.remove(&i.id);
                world.despawn(i.entity);
            }
        }
    });
}
pub(super) fn clear(world: &mut World) {
    // A map transition has already installed a new skater: never teleport it back to the old map.
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        for (_, i) in std::mem::take(&mut v.owned) {
            world.despawn(i.entity);
        }
        for (id, visibility) in v.hidden.drain(..) {
            if let Some(mut current) = world.get_mut::<Visibility>(id) {
                *current = visibility;
            }
        }
        v.driver = None;
        v.pose = None;
        v.last_visual.clear();
        v.blend_from.clear();
        v.visual_phase.clear();
        v.steering_visual = 0.;
        v.crash_handoff = false;
        v.posture = Posture::default();
        v.follow = Follow::default();
        v.previous_motion.clear();
        v.rendered_motion.clear();
        v.remote.clear();
        v.skaters.clear();
        v.network_pose = None;
        v.simulation = Simulation::default();
        v.events.clear();
    });
}
fn ensure_ground(world: &World, v: &mut Vehicles) -> Result<(), String> {
    if v.simulation.world.colliders.is_empty() {
        v.simulation.ground(
            world
                .resource::<crate::physics::GamePhysics>()
                .world_triangles()
                .iter()
                .map(|t| t.triangle.vertices.map(|p| [p.x, p.y, p.z])),
        )?;
    }
    Ok(())
}
pub(super) fn command(
    world: &mut World,
    root: &std::path::Path,
    owner: &str,
    command: Command,
) -> Result<(), String> {
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        let key = match &command {
            Command::VehicleTune { key, .. }
            | Command::VehicleSpawn { key, .. }
            | Command::VehicleRemove { key }
            | Command::VehicleEnter { key }
            | Command::VehicleExit { key }
            | Command::VehicleReset { key, .. }
            | Command::VehicleControl { key, .. } => key.clone(),
            _ => unreachable!(),
        };
        let owned_key = (owner.to_owned(), key.clone());
        match command {
            Command::VehicleTune { tuning, .. } => {
                let id = v.owned.get(&owned_key).ok_or("Unknown vehicle")?.id;
                let car = v.simulation.vehicles.get_mut(&id).unwrap();
                car.definition = tuning.apply(&car.definition)?;
            }
            Command::VehicleSpawn {
                definition,
                position,
                heading,
                ..
            } => {
                if v.owned.contains_key(&owned_key) {
                    return Err("Vehicle key already spawned; remove it before respawning".into());
                }
                if v.owned.keys().filter(|(o, _)| o == owner).count() >= 8
                    || v.owned
                        .keys()
                        .filter(|(o, _)| o.starts_with('@') == owner.starts_with('@'))
                        .count()
                        >= if owner.starts_with('@') { 288 } else { 32 }
                {
                    return Err("Vehicle limit: 8 per mod, 32 total".into());
                }
                let bytes = skate_mods::read_bounded(root, &definition, 128 * 1024)?;
                let def: VehicleDefinition =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                def.validate()?;
                let bytes = skate_mods::read_bounded(root, &def.model, 32 * 1024 * 1024)?;
                validate_glb(&bytes)?;
                let clips = animations::Clips::load(
                    root,
                    &def,
                    &world
                        .resource::<crate::physics::SkaterRuntime>()
                        .animation
                        .evaluator
                        .frames
                        .bone_names,
                )?;
                let model = root
                    .join(&def.model)
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                let package_root = super::package_root()
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                let relative = model
                    .strip_prefix(package_root)
                    .map_err(|_| "Model outside mod packages")?
                    .to_string_lossy()
                    .replace('\\', "/");
                let scene = world
                    .resource::<AssetServer>()
                    .load(GltfAssetLabel::Scene(0).from_asset(format!("mods://{relative}")));
                ensure_ground(world, &mut v)?;
                let id = v.simulation.spawn(def.clone(), position, heading)?;
                let entity = world
                    .spawn((
                        Transform::from_translation(Vec3::from_array(position))
                            .with_rotation(Quat::from_rotation_y(heading)),
                        Visibility::default(),
                    ))
                    .id();
                world.spawn((
                    SceneRoot(scene.clone()),
                    Transform::from_translation(Vec3::from_array(def.model_offset))
                        .with_rotation(Quat::from_rotation_y(def.model_yaw))
                        .with_scale(Vec3::splat(def.model_scale)),
                    ChildOf(entity),
                ));
                let clock = v.clock;
                v.owned.insert(
                    owned_key,
                    Instance {
                        id,
                        entity,
                        scene,
                        clips,
                        last_control: clock,
                        definition_path: definition,
                    },
                );
                event(&mut v, owner, &key, "vehicle_spawned");
            }
            Command::VehicleRemove { .. } => {
                if v.driver
                    .as_ref()
                    .is_some_and(|d| d.owner == owner && d.key == key)
                {
                    exit_now(world, &mut v, true)?;
                }
                if let Some(i) = v.owned.remove(&owned_key) {
                    v.simulation.remove(i.id);
                    v.previous_motion.remove(&i.id);
                    v.rendered_motion.remove(&i.id);
                    v.remote.remove(&i.id);
                    world.despawn(i.entity);
                    event(&mut v, owner, &key, "vehicle_removed");
                }
            }
            Command::VehicleEnter { .. } => {
                if v.driver.is_some()
                    || world.resource::<crate::replay::Replay>().active
                    || world
                        .resource::<crate::map_transition::MapTransition>()
                        .busy()
                {
                    return Ok(());
                }
                let i = v.owned.get(&owned_key).ok_or("Unknown vehicle")?;
                if !world
                    .resource::<AssetServer>()
                    .is_loaded_with_dependencies(i.scene.id())
                {
                    return Ok(());
                }
                if !world.resource::<crate::animation::AnimationStatus>().ready {
                    return Ok(());
                }
                let skater = world.resource::<crate::physics::SkaterRuntime>();
                if skater.player_input.pending_teleport().is_some()
                    || world
                        .resource::<crate::physics::GamePhysics>()
                        .board_wiping_out
                {
                    return Ok(());
                }
                let m = skater.animated_skeleton.roots.animation_to_world;
                let p = [m[3][0], m[3][1], m[3][2]];
                let car = v.simulation.pose(i.id).ok_or("Missing vehicle body")?.0;
                if Vec3::from_array(p).distance(Vec3::from_array(car)) > 4.
                    || v.simulation.vehicles[&i.id]
                        .controller
                        .current_vehicle_speed
                        .abs()
                        > 3.
                {
                    return Ok(());
                }
                v.driver = Some(Driver {
                    owner: owner.into(),
                    key: key.clone(),
                    phase: "entering",
                    time: 0.,
                    return_position: p,
                    return_heading: m[2][0].atan2(m[2][2]),
                });
                event(&mut v, owner, &key, "vehicle_entering");
            }
            Command::VehicleExit { .. } => {
                if let Some(d) = &v.driver {
                    if d.owner != owner || d.key != key {
                        return Ok(());
                    }
                    if d.phase != "driving" {
                        return Ok(());
                    }
                }
                if let Some(i) = v.owned.get(&owned_key) {
                    if v.simulation.vehicles[&i.id]
                        .controller
                        .current_vehicle_speed
                        .abs()
                        > 3.
                    {
                        event(&mut v, owner, &key, "vehicle_exit_blocked");
                        return Ok(());
                    }
                }
                if let Some(d) = &mut v.driver {
                    d.phase = "exiting";
                    d.time = 0.;
                }
            }
            Command::VehicleControl { controls, .. } => {
                let clock = v.clock;
                let i = v.owned.get_mut(&owned_key).ok_or("Unknown vehicle")?;
                i.last_control = clock;
                let id = i.id;
                v.simulation.vehicles.get_mut(&id).unwrap().controls = controls;
            }
            Command::VehicleReset {
                position, heading, ..
            } => {
                let i = v.owned.get(&owned_key).ok_or("Unknown vehicle")?;
                let id = i.id;
                v.simulation.reset(id, position, heading)?;
                v.previous_motion.remove(&id);
                v.rendered_motion.remove(&id);
                event(&mut v, owner, &key, "vehicle_reset");
            }
            _ => unreachable!(),
        }
        Ok(())
    })
}
fn validate_glb(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 20
        || &bytes[..4] != b"glTF"
        || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 2
    {
        return Err("Vehicle model must be GLB 2".into());
    }
    let n = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json: Value = serde_json::from_slice(bytes.get(20..20 + n).ok_or("Truncated GLB")?)
        .map_err(|e| e.to_string())?;
    for field in ["buffers", "images"] {
        for entry in json[field].as_array().into_iter().flatten() {
            if entry.get("uri").is_some() {
                return Err("Vehicle GLB must embed all buffers and textures".into());
            }
        }
    }
    Ok(())
}
fn tick(world: &mut World) {
    if world.resource::<crate::replay::Replay>().active {
        return;
    }
    let dt = world.resource::<Time<Fixed>>().delta_secs();
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        v.clock += dt;
        network::skater_proxies(world, &mut v);
        network::advance(&mut v, dt);
        let clock = v.clock;
        let parked: Vec<_> = v
            .owned
            .values()
            .filter(|i| clock - i.last_control > 0.25)
            .map(|i| i.id)
            .collect();
        for id in parked {
            v.simulation.vehicles.get_mut(&id).unwrap().controls = Controls {
                brake: 0.2,
                ..Default::default()
            };
        }
        let driver_info = v
            .driver
            .as_ref()
            .map(|d| (d.owner.clone(), d.key.clone(), d.phase));
        if let Some((owner, key, phase)) = &driver_info {
            if *phase != "driving" {
                if let Some(i) = v.owned.get(&(owner.clone(), key.clone())) {
                    let id = i.id;
                    v.simulation.vehicles.get_mut(&id).unwrap().controls = Controls {
                        brake: 1.,
                        ..Default::default()
                    };
                }
            }
        }
        let occupied_id = v
            .driver
            .as_ref()
            .and_then(|d| v.owned.get(&(d.owner.clone(), d.key.clone())))
            .map(|i| i.id);
        let ids: Vec<_> = v.simulation.vehicles.keys().copied().collect();
        for id in ids {
            if !v.remote.contains_key(&id) {
                v.simulation.set_occupied(id, Some(id) == occupied_id);
            }
        }
        if !v.owned.is_empty() {
            v.previous_motion = v
                .simulation
                .vehicles
                .keys()
                .filter_map(|&id| network::motion(&v, id).map(|m| (id, m)))
                .collect();
            v.simulation.step(dt);
        }
        if let Some(id) = occupied_id {
            if let Some(ejection) = v.simulation.take_ejection(id) {
                if let Err(error) = eject_now(world, &mut v, ejection) {
                    warn!("Vehicle ejection: {error}");
                }
                return;
            }
        }
        if let Some((owner, key, phase)) = driver_info {
            if let Some(i) = v.owned.get(&(owner.clone(), key.clone())) {
                let def = &v.simulation.vehicles[&i.id].definition;
                let duration = i.clips.duration(if phase == "entering" {
                    def.animations.enter.as_ref()
                } else {
                    def.animations.exit.as_ref()
                });
                if let Some(d) = &mut v.driver {
                    d.time += dt;
                }
                if phase != "driving" && v.driver.as_ref().unwrap().time >= duration {
                    if phase == "exiting" {
                        if let Err(e) = exit_now(world, &mut v, false) {
                            warn!("Vehicle exit: {e}");
                        }
                    } else {
                        let d = v.driver.as_mut().unwrap();
                        d.phase = "driving";
                        d.time = 0.;
                        event(&mut v, &owner, &key, "vehicle_entered");
                    }
                }
            }
        }
    });
}
pub(crate) fn present(world: &mut World) {
    let dt = world.resource::<Time<Virtual>>().delta_secs();
    let alpha = world.resource::<Time<Fixed>>().overstep_fraction();
    let phase = world
        .resource::<Vehicles>()
        .driver
        .as_ref()
        .map_or("vanilla", |d| d.phase)
        .to_owned();
    let current = crate::animation::capture_vehicle_visual(world);
    {
        let mut v = world.resource_mut::<Vehicles>();
        if phase != v.visual_phase {
            v.blend_from = if v.last_visual.is_empty() {
                current
            } else {
                v.last_visual.clone()
            };
            v.visual_phase = phase;
            v.blend_time = 0.;
        }
        v.blend_time += dt;
    }
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        let failures: Vec<_> = v
            .owned
            .iter()
            .filter_map(|((owner, key), i)| {
                if let Some(bevy::asset::LoadState::Failed(error)) =
                    world.resource::<AssetServer>().get_load_state(i.scene.id())
                {
                    Some((
                        owner.clone(),
                        format!("Vehicle {key} model failed: {error}"),
                    ))
                } else {
                    None
                }
            })
            .collect();
        for (owner, error) in failures {
            world
                .resource_mut::<super::Mods>()
                .manager
                .fail(&owner, error);
        }
        // Chassis, wheels, rider and camera share one fixed-step render sample.
        v.rendered_motion = v
            .simulation
            .vehicles
            .keys()
            .filter_map(|&id| {
                let current = network::motion(&v, id)?;
                let sample = v.previous_motion.get(&id).map_or_else(
                    || current.clone(),
                    |previous| previous.sample(&current, alpha),
                );
                Some((id, sample))
            })
            .collect();
        for i in v.owned.values() {
            if let Some(sample) = v.rendered_motion.get(&i.id) {
                if let Some(mut t) = world.get_mut::<Transform>(i.entity) {
                    // Wheels and bodywork are children of this entity, so the
                    // whole bike leans with it while the collider and the
                    // suspension raycasts stay upright underneath.
                    *t = sample.leaned();
                }
            }
        }
        // Tag the parked-only nodes, using the same ancestry walk the wheels
        // use so a node of the same name under another vehicle is not caught.
        let mut parked = Vec::new();
        for instance in v.owned.values() {
            for name in &v.simulation.vehicles[&instance.id].definition.parked_nodes {
                let matches: Vec<_> = world
                    .query_filtered::<(Entity, &Name), Without<ParkedOnly>>()
                    .iter(world)
                    .filter(|(_, n)| n.as_str() == name)
                    .map(|(e, _)| e)
                    .collect();
                for entity in matches {
                    let mut ancestor = entity;
                    for _ in 0..128 {
                        let Some(parent) = world.get::<ChildOf>(ancestor) else {
                            break;
                        };
                        ancestor = parent.parent();
                        if ancestor == instance.entity {
                            parked.push((entity, ParkedOnly(instance.id)));
                            break;
                        }
                    }
                }
            }
        }
        for (entity, marker) in parked {
            world.entity_mut(entity).insert(marker);
        }
        let ridden: std::collections::BTreeSet<u64> = v
            .simulation
            .vehicles
            .keys()
            .copied()
            .filter(|&id| network::occupied(&v, id))
            .collect();
        for (marker, mut visibility) in world
            .query::<(&ParkedOnly, &mut Visibility)>()
            .iter_mut(world)
        {
            let want = if ridden.contains(&marker.0) {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            if *visibility != want {
                *visibility = want;
            }
        }
        let mut wheels = Vec::new();
        for instance in v.owned.values() {
            for (index, wheel) in v.simulation.vehicles[&instance.id]
                .definition
                .wheels
                .iter()
                .enumerate()
            {
                let Some(name) = &wheel.node else {
                    continue;
                };
                let matches: Vec<_> = world
                    .query_filtered::<(Entity, &Name, &Transform), Without<WheelVisual>>()
                    .iter(world)
                    .filter(|(_, n, _)| n.as_str() == name)
                    .map(|(e, _, t)| (e, *t))
                    .collect();
                for (entity, rest) in matches {
                    let mut ancestor = entity;
                    let mut owned = false;
                    for _ in 0..128 {
                        let Some(parent) = world.get::<ChildOf>(ancestor) else {
                            break;
                        };
                        ancestor = parent.parent();
                        if ancestor == instance.entity {
                            owned = true;
                            break;
                        }
                    }
                    if owned {
                        wheels.push((
                            entity,
                            WheelVisual {
                                vehicle: instance.id,
                                index,
                                rest,
                            },
                        ));
                    }
                }
            }
        }
        for (entity, wheel) in wheels {
            world.entity_mut(entity).insert(wheel);
        }
        for (wheel, mut transform) in world
            .query::<(&WheelVisual, &mut Transform)>()
            .iter_mut(world)
        {
            if let Some(sample) = v.rendered_motion.get(&wheel.vehicle) {
                if let Some(offset) = sample.wheels.get(wheel.index) {
                    *transform = wheel.rest;
                    transform.translation += offset.translation;
                    transform.rotation = wheel.rest.rotation * offset.rotation;
                }
            }
        }
        v.pose = None;
        let Some(d) = &v.driver else {
            return;
        };
        let Some(i) = v.owned.get(&(d.owner.clone(), d.key.clone())) else {
            return;
        };
        let car = &v.simulation.vehicles[&i.id];
        let a = &car.definition.animations;
        let name = match d.phase {
            "entering" => a.enter.as_ref(),
            "exiting" => a.exit.as_ref(),
            _ => {
                if car.controls.brake > 0.2 {
                    a.brake.as_ref()
                } else if car.controller.current_vehicle_speed < -0.5 {
                    a.reverse.as_ref()
                } else if car.controller.current_vehicle_speed.abs() > 0.5 {
                    a.drive.as_ref()
                } else {
                    a.idle.as_ref()
                }
            }
        }
        .or(a.drive.as_ref());
        let target_steering = car.controls.steering;
        let steering =
            v.steering_visual + (target_steering - v.steering_visual) * (1. - (-12. * dt).exp());
        // What the ride is asking the rider's body to do. Each of these is
        // one authored held pose; the stack below eases the seated pose
        // towards all of them at once, so a rider standing on the pegs with
        // his weight back through a lean is all three at their own weights
        // rather than a separate authored pose for every combination.
        let bike = v.simulation.bike_state(i.id).unwrap_or_default();
        let riding = d.phase == "driving";
        let wants = if riding {
            let sag = 0.35;
            Posture {
                // Up off the seat in the air, under the brakes, and rising
                // with the throttle: an attack stance, not a commuter.
                stand: (bike.airborne as u8 as f32)
                    .max(car.controls.brake * 0.8)
                    .max((-car.controls.weight).max(0.) * 0.7)
                    .max(car.controls.throttle.max(0.) * 0.3)
                    .clamp(0., 1.),
                // Absorb: whatever the suspension is doing past its static
                // sag, plus the crouch that loads it in the first place.
                crouch: (((bike.compression - sag) / (1. - sag)).clamp(0., 1.) * 1.2
                    + bike.preload * 0.5
                    + bike.landing * 1.5)
                    .clamp(0., 1.),
                back: car.controls.weight.max(0.),
                forward: (-car.controls.weight).max(0.),
                lean: (bike.lean / car.definition.bike.lean_max.max(0.05)).clamp(-1., 1.),
            }
        } else {
            Posture::default()
        };
        // `v.driver` and `v.owned` are borrowed for the rest of this block, so
        // the posture is advanced on a detached copy and stored back at the end.
        let mut posture = Posture {
            stand: v.posture.stand,
            crouch: v.posture.crouch,
            back: v.posture.back,
            forward: v.posture.forward,
            lean: v.posture.lean,
        };
        posture.approach(&wants, dt);
        let layer = |name: &Option<String>, weight: f32| {
            (weight > 0.002)
                .then(|| i.clips.pose(name.as_ref(), d.time, true).map(|p| (p, weight)))
                .flatten()
        };
        // Negative lean is toward driver-left, matching `bike`'s sign table.
        let postures: Vec<_> = [
            layer(&a.lean_left, (-posture.lean).max(0.)),
            layer(&a.lean_right, posture.lean.max(0.)),
            layer(&a.weight_back, posture.back),
            layer(&a.weight_forward, posture.forward),
            layer(&a.stand, posture.stand),
            layer(&a.crouch, posture.crouch),
        ]
        .into_iter()
        .flatten()
        .collect();
        let pose = i.clips.pose(name, d.time, d.phase == "driving");
        let turn = if d.phase == "driving" {
            i.clips.pose(
                if steering >= 0. {
                    a.steer_left.as_ref()
                } else {
                    a.steer_right.as_ref()
                },
                d.time,
                true,
            )
        } else {
            None
        };
        // An air trick is one held pose blended in by how far the rider has
        // thrown it, so `trick_extend` drives the weight directly.
        let trick = (|| {
            if d.phase != "driving" {
                return None;
            }
            let id = car.controls.trick.checked_sub(1)? as usize;
            let clip = a.tricks.get(id)?;
            let extend = car.controls.trick_extend.clamp(0., 1.);
            if extend <= 0.001 {
                return None;
            }
            Some((i.clips.pose(Some(clip), d.time, true)?, extend))
        })();
        let followed = i.id;
        // The physics body never rolls: the bike's lean lives in `Motion`, and
        // the rider rides the leaned frame, not the upright chassis.
        let body = v.rendered_motion[&i.id].leaned();
        let q = body.rotation;
        let seat = body.transform_point(Vec3::from_array(car.definition.seat));
        v.steering_visual = steering;
        let roots: Vec<_> = world
            .query_filtered::<Entity, With<crate::world::PlayerRoot>>()
            .iter(world)
            .collect();
        for entity in roots {
            if !v.hidden.iter().any(|(id, _)| *id == entity) {
                if let Some(vis) = world.get::<Visibility>(entity) {
                    v.hidden.push((entity, *vis));
                }
            }
            if let Some(mut vis) = world.get_mut::<Visibility>(entity) {
                *vis = if pose.is_some() {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                *transform = Transform::from_translation(seat).with_rotation(q);
            }
        }
        if let Some(pose) = &pose {
            let mut layers: Vec<(&[Mat4], f32)> = Vec::new();
            if let Some(turn) = turn.as_deref() {
                layers.push((turn, steering.abs()));
            }
            // Posture before the trick: a thrown trick is the rider leaving
            // whatever stance he was in, so it has to blend in last.
            layers.extend(postures.iter().map(|(p, w)| (p.as_slice(), *w)));
            if let Some((frames, extend)) = &trick {
                layers.push((frames.as_slice(), *extend));
            }
            crate::animation::vehicle_pose(world, pose, &layers);
        }
        v.posture = posture;
        v.pose = pose;
        // Where the camera should be looking, decided once per frame.
        //
        // Preference order matters. Travel direction is used whenever the bike
        // is actually going somewhere, because it survives a flip, a whip and
        // a slide untouched. The nose is the fallback for a stationary bike,
        // and it is only trusted while it still has a horizontal direction to
        // give: past `UPRIGHT_ENOUGH` the projection is mostly rounding error.
        const UPRIGHT_ENOUGH: f32 = 0.35;
        let body = v.rendered_motion[&followed].body;
        let nose = body.rotation * Vec3::Z;
        let travel = v
            .simulation
            .telemetry(followed)
            .map(|(velocity, _, _)| Vec3::from_array(velocity).with_y(0.))
            .unwrap_or(Vec3::ZERO);
        let target = if travel.length() > 2.5 {
            Some(travel.normalize())
        } else if nose.with_y(0.).length() > UPRIGHT_ENOUGH {
            Some(nose.with_y(0.).normalize())
        } else {
            // Nose at the sky, going nowhere: hold the last heading rather
            // than chase a direction that is not there.
            None
        };
        if let Some(target) = target {
            // Ease harder on the ground, where the camera should stay behind
            // the bike, than in the air, where a lazy camera is what keeps a
            // whip or a flip readable instead of nauseating.
            let airborne = v.simulation.bike_state(followed).is_some_and(|b| b.airborne);
            let rate = if airborne { 1.6 } else { 5.0 };
            let yaw = target.x.atan2(target.z);
            v.follow.approach(yaw, rate, dt);
        }
    });
    world.resource_scope(|world, mut v: Mut<Vehicles>| {
        let duration = if v.visual_phase == "vanilla" {
            if v.crash_handoff { 0.12 } else { 0.5 }
        } else {
            0.4
        };
        if v.blend_time < duration {
            let t = (v.blend_time / duration).clamp(0., 1.);
            crate::animation::blend_vehicle_visual(world, &v.blend_from, t * t * (3. - 2. * t));
        } else {
            v.blend_from.clear();
            v.crash_handoff = false;
        }
        v.last_visual = crate::animation::capture_vehicle_visual(world);
        v.network_pose = if v.driver.is_some() || !v.blend_from.is_empty() {
            Some(crate::animation::network_visual(world))
        } else {
            None
        };
    });
}

pub(super) fn input(world: &World) -> Value {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let pad = world
        .resource::<crate::input::ControllerInput>()
        .raw_input();
    let pressed = |key| if keys.pressed(key) { 1. } else { 0. };
    let stick = |v: f32| if v.abs() > 0.15 { v } else { 0. };
    // Left stick is the bike, right stick is the rider: the two-stick split the
    // freestyle motocross games use. The kart's original names (`pitch`,
    // `throttle`, `steering`, `brake`) keep their exact meanings, so nothing
    // here changes how an existing vehicle mod drives.
    let bike_y = (pressed(KeyCode::ArrowUp) - pressed(KeyCode::ArrowDown) + stick(pad.left[1]))
        .clamp(-1., 1.);
    let bike_x =
        (pressed(KeyCode::KeyA) - pressed(KeyCode::KeyD) - stick(pad.left[0])).clamp(-1., 1.);
    let rider_x =
        (pressed(KeyCode::KeyJ) - pressed(KeyCode::KeyL) - stick(pad.right[0])).clamp(-1., 1.);
    let rider_y =
        (pressed(KeyCode::KeyI) - pressed(KeyCode::KeyK) + stick(pad.right[1])).clamp(-1., 1.);
    json!({
        // Kart vocabulary, unchanged.
        "pitch": bike_y,
        "throttle": (pressed(KeyCode::KeyW) - pressed(KeyCode::KeyS) + pad.triggers[1] - pad.triggers[0]).clamp(-1., 1.),
        "steering": bike_x,
        "brake": if pad.buttons & 0x1000 != 0 { 1. } else { pressed(KeyCode::Space) },
        "handbrake": keys.pressed(KeyCode::ShiftLeft) || pad.buttons & 0x2000 != 0,
        "interact": keys.pressed(KeyCode::KeyE) || pad.buttons & 0x8000 != 0,
        "pad_buttons": pad.buttons,
        // Bike vocabulary. `weight` is the bike axis renamed for its sign: stick
        // back builds preload and lifts the nose. `whip` shares the bars axis,
        // which is safe because steering only acts in contact and whip only in
        // the air.
        "weight": -bike_y,
        "whip": bike_x,
        "lean": rider_x,
        "rider_y": rider_y,
        // Raw triggers, so a mod can split them: a bike wants RT throttle and LT
        // front brake rather than the kart's combined forward/reverse pedal.
        "trigger_l": pad.triggers[0],
        "trigger_r": pad.triggers[1],
        "trick_a": pad.buttons & 0x0100 != 0 || keys.pressed(KeyCode::KeyZ),
        "trick_b": pad.buttons & 0x0200 != 0 || keys.pressed(KeyCode::KeyX),
        "clutch": pad.buttons & 0x1000 != 0 || keys.pressed(KeyCode::KeyC),
    })
}
