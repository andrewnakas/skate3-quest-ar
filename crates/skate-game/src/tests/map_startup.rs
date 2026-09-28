//! Production startup/frame integration, following physics_startup.rs.
//! Parent physics.rs must declare this as a child module with:
//! #[cfg(test)] #[path = "tests/map_startup.rs"] mod map_startup;
//! Private inputs are loaded at runtime; these checks do not establish TU3 parity.
use super::*;
use crate::{camera::CameraRuntime, graph_runtime::StockGraphs, input::ControllerInput};
use skate_data::skate_map::SkateMap;
use std::path::{Path, PathBuf};

const TICKS: u64 = 24;
// Numerical assertion tolerances, not simulation settings.
const POSE_EPSILON: f32 = 0.002;
const OFFSET: [f32; 3] = [137.0, 7.0, -83.0];

fn asset_root() -> PathBuf {
    std::env::var_os("SKATE3_ASSET_ROOT")
        .map(PathBuf::from)
        .expect("set SKATE3_ASSET_ROOT to the current clone's converted stock assets")
}

fn demo() -> SkateMap {
    SkateMap::parse(include_bytes!("../../../../maps/format-demo.skate")).unwrap()
}

// Independent +90 degree Y rotation: catches a reversed heading convention.
fn rotate([x, y, z]: [f32; 3]) -> [f32; 3] {
    [z, y, -x]
}

fn relocate(point: [f32; 3]) -> [f32; 3] {
    let rotated = rotate(point);
    std::array::from_fn(|i| rotated[i] + OFFSET[i])
}

fn xyz(v: Vector3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn close(actual: [f32; 3], expected: [f32; 3], context: &str) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, b)| (a - b).abs() < POSE_EPSILON),
        "{context}: actual={actual:?}, expected={expected:?}"
    );
}

struct Gameplay {
    physics: GamePhysics,
    skater: SkaterRuntime,
    controls: PlayerControls,
    input: ControllerInput,
    camera: CameraRuntime,
}

impl Gameplay {
    fn load(root: &Path, graphs: &StockGraphs, map: &SkateMap) -> Self {
        crate::skate_world::validate_runtime(map).unwrap();
        // Deliberately pass Course: Some(map) must supersede the terrain.
        let physics = GamePhysics::load_with_world(root, ground::Terrain::Course, Some(map))
            .unwrap_or_else(|error| panic!("Map {:?} physics startup: {error}", map.name));
        let skater = SkaterRuntime::load(root, graphs, &physics, "normal").unwrap();
        Self {
            physics,
            skater,
            controls: PlayerControls::default(),
            input: ControllerInput::default(),
            camera: CameraRuntime::load(root).unwrap(),
        }
    }

    fn advance(&mut self, graphs: &StockGraphs) {
        // A connected, neutral controller exercises the normal graph/input path.
        // No forced state, velocity, contact, camera or animation overrides.
        self.input
            .sample_raw_for_test(skate_core::input::xbox::XboxState {
                buttons: 0,
                triggers: [0; 2],
                left: [0; 2],
                right: [0; 2],
            });
        let mut actions = self.input.player_actions();
        self.controls
            .update_for_physics(&mut actions, &self.physics, &self.skater, &self.camera)
            .unwrap();
        frame::advance(
            &mut self.physics,
            &mut self.skater,
            &mut self.controls,
            graphs,
            &mut actions,
            true,
            &mut self.camera,
        )
        .unwrap_or_else(|error| panic!("Map gameplay tick {}: {error}", self.physics.ticks));
        self.assert_publication();
    }

    fn assert_publication(&self) {
        let tick = self.physics.ticks;
        assert!(!self.physics.failed, "Physics failed at tick {tick}");
        for body in self
            .physics
            .board
            .bodies()
            .iter()
            .chain(self.skater.skeleton.bodies())
        {
            assert!(
                xyz(body.rates.position).into_iter().all(f32::is_finite),
                "Non-finite body position at tick {tick}"
            );
            assert!(
                body.rates
                    .basis
                    .columns
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite()),
                "Non-finite body orientation at tick {tick}"
            );
        }
        assert!(!self.skater.render_pose.is_empty());
        assert!(
            self.skater
                .render_pose
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite()),
            "Non-finite render pose at tick {tick}"
        );
        assert!(
            self.skater
                .animated_skeleton
                .roots
                .animation_to_world
                .iter()
                .flatten()
                .all(|v| v.is_finite()),
            "Non-finite player root at tick {tick}"
        );
        assert_eq!(self.skater.pose_generation, tick);
        assert_eq!(self.skater.animation.ticks, tick);
        let camera = self
            .camera
            .frame
            .as_ref()
            .expect("Gameplay camera must publish");
        assert!(camera.position.iter().all(|v| v.is_finite()));
        assert!(camera.basis.columns.iter().flatten().all(|v| v.is_finite()));
        assert!(camera.field_of_view_degrees.is_finite());
        assert!(camera.field_of_view_degrees > 0.0 && camera.field_of_view_degrees < 180.0);
        let subject = self
            .camera
            .latest_subject
            .as_ref()
            .expect("Missing camera subject");
        assert_eq!(
            subject.tick,
            tick - 1,
            "Camera consumed stale physical output"
        );
        let published_com = self
            .skater
            .player_input
            .physical
            .reckoning
            .vector_64
            .map(f32::from_bits);
        close(
            subject.pose.center_of_mass[..3].try_into().unwrap(),
            published_com[..3].try_into().unwrap(),
            "Camera must consume the completed physical COM publication",
        );
    }

    fn run_supported_startup(&mut self, graphs: &StockGraphs, map: &SkateMap) {
        let mut saw_solved_wheel = false;
        let mut grounded_ticks = 0;
        for _ in 0..TICKS {
            self.advance(graphs);
            grounded_ticks += usize::from(self.physics.riding.ground.wheel_contact_count > 0);
            for report in self.physics.board.contact_reports() {
                assert!(
                    xyz(report.normal_force_on_a)
                        .into_iter()
                        .all(f32::is_finite)
                );
                saw_solved_wheel |= report.part.index() < 4
                    && matches!(
                        report.other,
                        skate_core::physics::board_step::CollisionBody::StaticWorld
                    )
                    && xyz(report.normal_force_on_a)
                        .into_iter()
                        .any(|force| force != 0.0);
            }
        }
        assert_eq!(self.physics.ticks, TICKS);
        assert!(
            grounded_ticks > 0,
            "Map {:?}: wheel queries never found support",
            map.name
        );
        assert!(
            self.physics.riding.ground.wheel_contact_count > 0,
            "Map {:?}: no wheel support after {TICKS} neutral gameplay ticks",
            map.name
        );
        assert!(
            saw_solved_wheel,
            "Map {:?}: no solved wheel contact reaction",
            map.name
        );
        eprintln!(
            "Map {:?}: spawn={:?}, yaw={}, ticks={}, grounded_ticks={}, deck={:?}, camera={:?}",
            map.name,
            map.spawn,
            map.heading,
            self.physics.ticks,
            grounded_ticks,
            self.physics.board.part_transforms()[BodyId::Deck.index()].translation,
            self.camera.frame
        );
    }
}

#[test]
#[ignore = "requires SKATE3_ASSET_ROOT with private stock skater, graph and camera assets"]
fn moved_demo_starts_board_skater_and_camera_at_authored_spawn_and_yaw() {
    let root = asset_root();
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = StockGraphs::load(&root, &assets).unwrap();
    let original = demo();
    let mut moved = demo();
    moved.spawn = relocate(original.spawn);
    moved.heading += std::f32::consts::FRAC_PI_2;
    for vertex in &mut moved.geometry.vertices {
        vertex.position = relocate(vertex.position);
        vertex.normal = rotate(vertex.normal);
    }
    for triangle in &mut moved.geometry.collision {
        triangle.points = triangle.points.map(relocate);
    }
    let baseline = Gameplay::load(&root, &graphs, &original);
    let mut gameplay = Gameplay::load(&root, &graphs, &moved);
    assert_eq!(
        gameplay.physics.world.triangles().len(),
        moved.geometry.collision.len()
    );
    for (loaded, authored) in gameplay
        .physics
        .world
        .triangles()
        .iter()
        .zip(&moved.geometry.collision)
    {
        for (actual, expected) in loaded.triangle.vertices.iter().zip(authored.points) {
            close(
                xyz(*actual),
                expected,
                "Production world must contain moved map collision",
            );
        }
    }
    for (actual, original) in gameplay
        .physics
        .board
        .part_transforms()
        .iter()
        .zip(baseline.physics.board.part_transforms())
    {
        close(
            xyz(actual.translation),
            relocate(xyz(original.translation)),
            "Board part spawn",
        );
        for (actual, original) in actual.basis.columns.iter().zip(original.basis.columns) {
            close(*actual, rotate(original), "Board part yaw");
        }
    }
    let wheel = gameplay.physics.board.part_transforms()[0].translation;
    assert!(
        (wheel.y - gameplay.physics.settings.wheel_radius - moved.spawn[1]).abs() < POSE_EPSILON,
        "Map spawn must anchor the authored wheel bottom to the ground"
    );
    assert_eq!(gameplay.skater.skeleton.bodies().len(), 26);
    for (actual, original) in gameplay
        .skater
        .skeleton
        .part_transforms()
        .iter()
        .zip(baseline.skater.skeleton.part_transforms())
    {
        close(
            actual[3][..3].try_into().unwrap(),
            relocate(original[3][..3].try_into().unwrap()),
            "Skater body spawn",
        );
        for axis in 0..3 {
            close(
                actual[axis][..3].try_into().unwrap(),
                rotate(original[axis][..3].try_into().unwrap()),
                "Skater body yaw",
            );
        }
    }
    let actual = gameplay.skater.animated_skeleton.roots.animation_to_world;
    let original_root = baseline.skater.animated_skeleton.roots.animation_to_world;
    close(
        actual[3][..3].try_into().unwrap(),
        relocate(original_root[3][..3].try_into().unwrap()),
        "Presented player root spawn",
    );
    for axis in 0..3 {
        close(
            actual[axis][..3].try_into().unwrap(),
            rotate(original_root[axis][..3].try_into().unwrap()),
            "Presented player root yaw",
        );
    }
    gameplay.run_supported_startup(&graphs, &moved);
    let deck = gameplay.physics.board.part_transforms()[BodyId::Deck.index()].translation;
    assert!(
        deck.y > moved.spawn[1],
        "Deck fell below the moved demo floor"
    );
    let camera = gameplay.camera.frame.as_ref().unwrap();
    // A broad startup acceptance bound: catches a camera left at the origin.
    // It does not prescribe a chase distance or replace the stock camera tuning.
    let camera_distance = camera.position[..3]
        .iter()
        .zip(moved.spawn)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!(
        camera_distance < 20.0,
        "Camera did not follow moved spawn: {camera_distance}m"
    );
}

#[test]
#[ignore = "requires SKATE3_ASSET_ROOT and SKATE_MAP_TEST_PATH pointing to a private extracted .skate map"]
fn private_extracted_map_supports_production_gameplay_startup() {
    let root = asset_root();
    let path = std::env::var_os("SKATE_MAP_TEST_PATH")
        .map(PathBuf::from)
        .expect("set SKATE_MAP_TEST_PATH to the extracted .skate package");
    let map = SkateMap::load(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let assets = skate_data::GameAssets::load(&root).unwrap();
    let graphs = StockGraphs::load(&root, &assets).unwrap();
    let mut gameplay = Gameplay::load(&root, &graphs, &map);
    assert!(!gameplay.physics.world.triangles().is_empty());
    let [x, y, z] = map.spawn;
    let support = gameplay
        .physics
        .world
        .query_thin_line(Vector3::new(x, y + 1.0, z), Vector3::new(x, y - 10.0, z))
        .unwrap();
    assert!(
        support.is_some(),
        "{}: authored spawn has no supporting map collision",
        path.display()
    );
    let deck = gameplay.physics.board.part_transforms()[BodyId::Deck.index()];
    close(
        xyz(deck.translation),
        [
            x,
            y + gameplay.physics.settings.wheel_radius
                - gameplay.physics.settings.authored[0].translation.y,
            z,
        ],
        "Private map deck spawn",
    );
    let (sin, cos) = map.heading.sin_cos();
    close(
        deck.basis.columns[2],
        [sin, 0.0, cos],
        "Private map deck heading",
    );
    eprintln!(
        "Private map {}: {} collision triangles, spawn support={support:?}",
        path.display(),
        gameplay.physics.world.triangles().len()
    );
    gameplay.run_supported_startup(&graphs, &map);
}
