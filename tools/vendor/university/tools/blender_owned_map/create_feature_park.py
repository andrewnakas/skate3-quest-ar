"""Build the large Blender-authored SKATE v15 feature park.

The park deliberately contains only features represented by the current
Blender addon and SKATE package: static visuals/collision, PBR and alpha
materials, native Skate surface channels, grinds, native-AI routes, hinged
doors, local lights, baked indirect lighting, and the day/night sky.
"""

from __future__ import annotations

from pathlib import Path
import math
import sys

import bpy
from mathutils import Vector


TOOL_ROOT = Path(__file__).resolve().parent
ROOT = Path(__file__).resolve().parents[2]
if str(TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(TOOL_ROOT))
AUTO_MAP_SCRIPT_ROOT = (
    TOOL_ROOT / "agent_workflow" / "sk8-auto-map" / "scripts"
)
if str(AUTO_MAP_SCRIPT_ROOT) not in sys.path:
    sys.path.insert(0, str(AUTO_MAP_SCRIPT_ROOT))

import create_bake_showcase as base  # noqa: E402
from generate_grinds import generate_grinds  # noqa: E402
from owned_world_material_addon.exporter import export_scene  # noqa: E402


BLEND_PATH = ROOT / "maps" / "blender_bake_showcase.blend"
PACKAGE_PATH = ROOT / "maps" / "blender_bake_showcase.skate"
base.TEXTURE_DIR = (
    ROOT / "maps" / "blender_bake_showcase_textures"
)
MANUAL_GRIND_SPECS: list[
    tuple[str, list[tuple[float, float, float]]]
] = []


def copy_uv_layer(
    obj: bpy.types.Object,
    source_name: str = "UVMap",
    target_name: str = "Lightmap",
) -> None:
    source = obj.data.uv_layers.get(source_name)
    if source is None:
        raise RuntimeError(f"{obj.name} has no {source_name} layer")
    target = obj.data.uv_layers.get(target_name)
    if target is None:
        target = obj.data.uv_layers.new(name=target_name)
    for index, value in enumerate(source.data):
        target.data[index].uv = value.uv


def add_text(
    text: str,
    location: tuple[float, float, float],
    material: bpy.types.Material,
    collection: bpy.types.Collection,
    *,
    size: float = 1.0,
) -> bpy.types.Object:
    curve = bpy.data.curves.new(f"{text}_TextCurve", type="FONT")
    curve.body = text
    curve.align_x = "CENTER"
    curve.align_y = "CENTER"
    curve.size = size
    # Keep signage readable without generating tens of thousands of beveled
    # font triangles.  The package exporter currently crosses Blender's
    # Python boundary per loop corner, so flat low-resolution lettering is
    # materially faster to publish while looking identical at gameplay range.
    curve.resolution_u = 1
    curve.extrude = 0.008
    curve.bevel_depth = 0.0
    obj = bpy.data.objects.new(f"SignText_{text}", curve)
    collection.objects.link(obj)
    obj.location = location
    # Text's local +Z faces outward; rotate it onto a vertical plane facing
    # the player approaching along +Y.
    obj.rotation_euler = (math.pi * 0.5, 0.0, 0.0)
    obj.data.materials.append(material)
    bpy.ops.object.select_all(action="DESELECT")
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.convert(target="MESH")
    obj = bpy.context.object
    base.metric_uv(obj, metres_per_tile=1.0)
    obj.select_set(False)
    return obj


def add_zone_sign(
    title: str,
    center_x: float,
    y: float,
    material: bpy.types.Material,
    text_material: bpy.types.Material,
    visual: bpy.types.Collection,
) -> None:
    base.add_box(
        f"{title}_SignBoard",
        (center_x, y, 3.7),
        (16.0, 0.32, 3.0),
        material,
        visual,
        None,
        bevel=0.16,
    )
    add_text(
        title,
        (center_x, y - 0.19, 3.72),
        text_material,
        visual,
        size=1.05,
    )


def add_surface_pad(
    name: str,
    center: tuple[float, float],
    dimensions: tuple[float, float],
    material: bpy.types.Material,
    visual: bpy.types.Collection,
    collision: bpy.types.Collection,
) -> None:
    base.add_box(
        name,
        (center[0], center[1], 0.11),
        (dimensions[0], dimensions[1], 0.22),
        material,
        visual,
        collision,
        bevel=0.045,
    )


def add_rail_polyline(
    name: str,
    points: list[tuple[float, float, float]],
    material: bpy.types.Material,
    visual: bpy.types.Collection,
    collision: bpy.types.Collection,
    grinds: bpy.types.Collection,
    *,
    radius: float = 0.085,
) -> None:
    for index, (start, end) in enumerate(zip(points, points[1:]), 1):
        start_vector = Vector(start)
        end_vector = Vector(end)
        direction = end_vector - start_vector
        bpy.ops.mesh.primitive_cylinder_add(
            vertices=20,
            radius=radius,
            depth=direction.length,
            location=(start_vector + end_vector) * 0.5,
        )
        segment = bpy.context.object
        segment.name = f"{name}_Segment_{index:02d}"
        segment.rotation_mode = "QUATERNION"
        segment.rotation_quaternion = direction.to_track_quat("Z", "Y")
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
        base.move_to(segment, visual)
        segment.data.materials.append(material)
        base.metric_uv(segment, metres_per_tile=1.0)
        collider = segment.copy()
        collider.data = segment.data.copy()
        collider.name = f"{name}_Collision_{index:02d}"
        collider["ow_material"] = material.name
        collider["ow_map_object_owner"] = name
        collider["ow_upward_surface"] = False
        collider["sk8_auto_grind_exclude"] = True
        collision.objects.link(collider)
    # Skate's native grind spline describes the contact line above the rail,
    # not the cylinder centreline. Keeping it slightly above the collision
    # surface prevents the restored physical rail from masking grind pickup.
    grind_clearance = radius + 0.015
    grind_points = [
        (point[0], point[1], point[2] + grind_clearance)
        for point in points
    ]
    MANUAL_GRIND_SPECS.append((name, grind_points))


def prepare_editable_objects(
    visual: bpy.types.Collection,
    collision: bpy.types.Collection,
    grinds: bpy.types.Collection,
) -> list[bpy.types.Object]:
    """Associate each static feature's render, collision, and grind records."""
    for rail_name in ("StraightRail", "DownRail", "ArcRail"):
        segments = sorted(
            (
                obj
                for obj in visual.objects
                if obj.type == "MESH"
                and obj.name.startswith(f"{rail_name}_Segment_")
            ),
            key=lambda obj: obj.name,
        )
        if not segments:
            raise RuntimeError(f"{rail_name} has no visual segments")
        bpy.ops.object.select_all(action="DESELECT")
        for segment in segments:
            segment.select_set(True)
        bpy.context.view_layer.objects.active = segments[0]
        if len(segments) > 1:
            bpy.ops.object.join()
        rail = segments[0]
        rail.name = rail_name
    visual_names = {
        obj.name for obj in visual.objects if obj.type == "MESH"
    }
    for collider in collision.objects:
        if collider.type != "MESH":
            continue
        owner_name = str(collider.get("ow_map_object_owner", "")).strip()
        if not owner_name and collider.name.startswith("COL_"):
            owner_name = collider.name[4:]
        if owner_name not in visual_names:
            raise RuntimeError(
                f"collision {collider.name!r} has no editable visual owner "
                f"{owner_name!r}"
            )
        collider["ow_map_object_owner"] = owner_name

    return base.prepare_shared_lightmap_uvs(visual)


def add_manual_grind_curves(grinds: bpy.types.Collection) -> None:
    """Create fresh validation curves without the automatic edge generator."""
    if len(MANUAL_GRIND_SPECS) != 3:
        raise RuntimeError(
            "Feature Park must define exactly three manual grind rails"
        )
    for owner_name, points in MANUAL_GRIND_SPECS:
        owner = bpy.data.objects.get(owner_name)
        if owner is None or owner.type != "MESH":
            raise RuntimeError(
                f"manual grind owner {owner_name!r} is unavailable"
            )
        grind_name = f"{owner_name}_ManualGrind"
        base.add_grind_curve(grind_name, points, grinds)
        grind = grinds.objects.get(grind_name)
        if grind is None:
            raise RuntimeError(
                f"manual grind {grind_name!r} was not created"
            )
        world_matrix = grind.matrix_world.copy()
        grind.parent = owner
        grind.matrix_world = world_matrix


def add_npc_route(
    name: str,
    points: list[tuple[float, float, float]],
    collection: bpy.types.Collection,
    *,
    skaters: int,
    speed: float,
    spacing: float,
    closed: bool = True,
) -> None:
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    spline = curve.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for target, point in zip(spline.points, points):
        target.co = (*point, 1.0)
    spline.use_cyclic_u = closed
    obj = bpy.data.objects.new(name, curve)
    obj["ow_npc_skater_count"] = skaters
    obj["ow_npc_speed"] = speed
    obj["ow_npc_spawn_spacing"] = spacing
    collection.objects.link(obj)


def add_light(
    name: str,
    light_type: str,
    location: tuple[float, float, float],
    color: tuple[float, float, float],
    energy: float,
    influence: float,
    *,
    target: tuple[float, float, float] | None = None,
    size: float = 0.3,
    spot_size: float = 0.9,
) -> bpy.types.Object:
    data = bpy.data.lights.new(f"{name}_Data", type=light_type)
    data.color = color
    data.energy = energy
    data.cutoff_distance = influence
    if light_type == "AREA":
        data.shape = "DISK"
        data.size = size
    else:
        data.shadow_soft_size = size
    if light_type == "SPOT":
        data.spot_size = spot_size
        data.spot_blend = 0.55
    obj = bpy.data.objects.new(name, data)
    bpy.context.scene.collection.objects.link(obj)
    obj.location = location
    if target is not None:
        direction = Vector(target) - obj.location
        obj.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
    obj["ow_feature_park_zone"] = "LOCAL_LIGHT_LAB"
    return obj


def add_hinged_door(
    material: bpy.types.Material,
    visual: bpy.types.Collection,
) -> bpy.types.Object:
    door = base.add_box(
        "FeaturePark_PhysicsDoor",
        (35.0, 39.0, 1.65),
        (3.2, 0.16, 3.3),
        material,
        visual,
        None,
        bevel=0.07,
    )
    copy_uv_layer(door)
    door["ow_physics_type"] = "HINGED_DOOR"
    door["ow_door_name"] = "Feature Park Return Door"
    door["ow_hinge_position"] = (33.4, 39.0, 0.0)
    door["ow_hinge_axis"] = (0.0, 0.0, 1.0)
    door["ow_door_min_angle_degrees"] = -105.0
    door["ow_door_max_angle_degrees"] = 105.0
    door["ow_door_initial_angle_degrees"] = 0.0
    door["ow_door_mass"] = 78.0
    door["ow_door_angular_damping"] = 2.6
    door["ow_door_return_spring_strength"] = 1.7
    door["ow_door_maximum_angular_speed"] = 1.35
    door["ow_door_contact_impulse_scale"] = 0.52
    door["ow_door_friction"] = 0.58
    door["ow_door_restitution"] = 0.015
    door["ow_door_collision_material"] = material.name
    return door


def make_materials() -> tuple[dict[str, bpy.types.Material], list]:
    concrete_tex = base.make_texture(
        "T_Concrete", (0.31, 0.34, 0.38), (0.48, 0.52, 0.57), 16
    )
    orange_tex = base.make_texture(
        "T_OrangePaint", (0.68, 0.12, 0.025), (0.98, 0.34, 0.04), 8,
        stripe=True,
    )
    brick_tex = base.make_texture(
        "T_DarkBrick", (0.095, 0.045, 0.04), (0.30, 0.09, 0.055), 18
    )
    metal_tex = base.make_texture(
        "T_Metal", (0.16, 0.19, 0.23), (0.48, 0.53, 0.58), 24
    )
    cyan_tex = base.make_texture(
        "T_CyanEmitter", (0.015, 0.78, 1.0), (0.10, 1.0, 0.94), 4
    )
    amber_tex = base.make_texture(
        "T_AmberEmitter", (1.0, 0.07, 0.012), (1.0, 0.48, 0.025), 4
    )
    white_tex = base.make_texture(
        "T_SignWhite", (0.94, 0.98, 1.0), (0.94, 0.98, 1.0), 1
    )
    asphalt_tex = base.make_texture(
        "T_Asphalt", (0.04, 0.045, 0.052), (0.13, 0.14, 0.15), 30
    )
    wood_tex = base.make_texture(
        "T_WoodRamp", (0.24, 0.075, 0.025), (0.62, 0.30, 0.07), 10,
        stripe=True,
    )
    tile_tex = base.make_texture(
        "T_CeramicTile", (0.10, 0.31, 0.62), (0.72, 0.90, 1.0), 8
    )
    grass_tex = base.make_texture(
        "T_Grass", (0.025, 0.15, 0.035), (0.13, 0.43, 0.08), 32
    )
    ice_tex = base.make_texture(
        "T_Ice", (0.30, 0.61, 0.84), (0.78, 0.95, 1.0), 6
    )
    glass_tex = base.make_texture(
        "T_Glass", (0.06, 0.30, 0.48), (0.20, 0.72, 0.84), 4,
        alpha=0.30,
    )
    fence_tex = base.make_texture(
        "T_FenceCutout", (0.05, 0.06, 0.07), (0.78, 0.82, 0.86), 8,
        cutout_grid=True,
    )
    hazard_tex = base.make_texture(
        "T_Hazard", (0.52, 0.01, 0.008), (1.0, 0.76, 0.015), 8,
        stripe=True,
    )

    normal_concrete = base.make_normal_texture("N_Concrete", 0.13, 18.0)
    normal_wood = base.make_normal_texture("N_Wood", 0.18, 7.0)
    normal_metal = base.make_normal_texture("N_Metal", 0.035, 24.0)
    normal_grass = base.make_normal_texture("N_Grass", 0.28, 21.0)
    normal_tile = base.make_normal_texture("N_Tile", 0.08, 8.0)
    normal_flat = base.make_normal_texture("N_Flat", 0.0, 1.0)

    orm_concrete = base.make_orm_texture("ORM_Concrete", 0.84, 0.0)
    orm_rough = base.make_orm_texture("ORM_Rough", 0.95, 0.0, ao=0.92)
    orm_wood = base.make_orm_texture("ORM_Wood", 0.72, 0.0)
    orm_metal = base.make_orm_texture("ORM_Metal", 0.22, 0.92)
    orm_tile = base.make_orm_texture("ORM_Tile", 0.30, 0.0)
    orm_grass = base.make_orm_texture("ORM_Grass", 0.97, 0.0, ao=0.82)
    orm_glass = base.make_orm_texture("ORM_Glass", 0.05, 0.0)
    orm_ice = base.make_orm_texture("ORM_Ice", 0.07, 0.0)

    materials = {
        "concrete": base.make_material(
            "Concrete", concrete_tex, (0.72, 0.75, 0.79), 0.86,
            normal_image=normal_concrete, orm_image=orm_concrete,
            audio_surface=4, physics_surface=2, surface_pattern=11,
        ),
        "orange": base.make_material(
            "OrangeRamp", orange_tex, (1.0, 0.50, 0.10), 0.68,
            normal_image=normal_wood, orm_image=orm_wood,
            audio_surface=6, physics_surface=1, surface_pattern=10,
        ),
        "brick": base.make_material(
            "Brick", brick_tex, (0.62, 0.22, 0.12), 0.82, flags=1 | 4,
            normal_image=normal_concrete, orm_image=orm_rough,
            audio_surface=66, physics_surface=2, surface_pattern=12,
        ),
        "metal": base.make_material(
            "GrindMetal", metal_tex, (0.72, 0.78, 0.84), 0.22,
            flags=1 | 2, normal_image=normal_metal, orm_image=orm_metal,
            metallic=0.92, audio_surface=11, physics_surface=1,
        ),
        "cyan": base.make_material(
            "CyanEmitter", cyan_tex, (0.04, 0.94, 1.0), 0.16,
            emissive=4.2, normal_image=normal_flat, orm_image=orm_glass,
            emissive_image=cyan_tex, collision_enabled=False,
        ),
        "amber": base.make_material(
            "AmberEmitter", amber_tex, (1.0, 0.22, 0.025), 0.16,
            emissive=4.8, normal_image=normal_flat, orm_image=orm_glass,
            emissive_image=amber_tex, collision_enabled=False,
        ),
        "label": base.make_material(
            "SignTextEmissive", white_tex, (1.0, 1.0, 1.0), 0.28,
            emissive=1.6, normal_image=normal_flat, orm_image=orm_tile,
            emissive_image=white_tex, collision_enabled=False,
        ),
        "asphalt": base.make_material(
            "TEST_01_Asphalt_Rough", asphalt_tex, (0.72, 0.74, 0.78), 0.95,
            normal_image=normal_concrete, orm_image=orm_rough,
            audio_surface=2, physics_surface=2, surface_pattern=7,
        ),
        "polished": base.make_material(
            "TEST_02_Concrete_Polished", concrete_tex,
            (0.92, 0.92, 0.92), 0.28,
            normal_image=normal_concrete, orm_image=orm_tile,
            audio_surface=3, physics_surface=1,
        ),
        "wood": base.make_material(
            "TEST_03_Wood_Ramp", wood_tex, (0.95, 0.76, 0.42), 0.72,
            normal_image=normal_wood, orm_image=orm_wood,
            audio_surface=6, physics_surface=1, surface_pattern=10,
        ),
        "metal_sheet": base.make_material(
            "TEST_04_Metal_Sheet", metal_tex, (0.78, 0.84, 0.90), 0.22,
            normal_image=normal_metal, orm_image=orm_metal, metallic=0.92,
            audio_surface=31, physics_surface=1,
        ),
        "tile": base.make_material(
            "TEST_05_Ceramic_Tile", tile_tex, (0.78, 0.90, 1.0), 0.30,
            normal_image=normal_tile, orm_image=orm_tile,
            audio_surface=63, physics_surface=1, surface_pattern=4,
        ),
        "grass": base.make_material(
            "TEST_06_Grass_Slow", grass_tex, (0.58, 0.88, 0.52), 0.97,
            normal_image=normal_grass, orm_image=orm_grass,
            audio_surface=10, physics_surface=3, surface_pattern=7,
        ),
        "ice": base.make_material(
            "TEST_07_Ice_Slippery", ice_tex, (0.72, 0.92, 1.0), 0.07,
            normal_image=normal_flat, orm_image=orm_ice,
            audio_surface=72, physics_surface=4,
        ),
        "glass": base.make_material(
            "TEST_Glass_Transparent", glass_tex, (0.65, 0.92, 1.0), 0.05,
            normal_image=normal_flat, orm_image=orm_glass,
            alpha_mode=2, audio_surface=51, physics_surface=1,
        ),
        "fence": base.make_material(
            "TEST_Fence_Cutout", fence_tex, (0.82, 0.86, 0.90), 0.30,
            normal_image=normal_metal, orm_image=orm_metal, metallic=0.78,
            alpha_mode=1, alpha_cutoff=0.5, audio_surface=68,
        ),
        "hazard": base.make_material(
            "TEST_Hazard_InstantBail", hazard_tex, (1.0, 0.32, 0.05), 0.84,
            normal_image=normal_concrete, orm_image=orm_rough,
            audio_surface=4, physics_surface=9,
        ),
        "door": base.make_material(
            "TEST_Physics_Door", wood_tex, (0.82, 0.43, 0.16), 0.58,
            normal_image=normal_wood, orm_image=orm_wood,
            audio_surface=6, physics_surface=1,
        ),
    }
    materials["door"]["ow_baked_strength"] = 0.0
    return materials, list(materials.values())


def build_static_park(
    material: dict[str, bpy.types.Material],
    visual: bpy.types.Collection,
    collision: bpy.types.Collection,
    grinds: bpy.types.Collection,
    npc_paths: bpy.types.Collection,
) -> None:
    # Keep a broad safety apron around the authored test zones and NPC routes.
    # The 360 x 336 metre foundation is three times the original width and
    # depth, while raised test pads sit above it to avoid coplanar collision.
    base.add_box(
        "FeatureParkFoundation",
        (0.0, 5.0, -0.32),
        (360.0, 336.0, 0.64),
        material["concrete"],
        visual,
        collision,
        bevel=0.08,
    )

    # Central material runway: seven long contact pads plus a clearly
    # separated bail strip. Riding straight forward traverses every surface.
    runway = (
        ("ASPHALT", material["asphalt"]),
        ("POLISHED CONCRETE", material["polished"]),
        ("WOOD", material["wood"]),
        ("METAL", material["metal_sheet"]),
        ("CERAMIC TILE", material["tile"]),
        ("GRASS", material["grass"]),
        ("ICE", material["ice"]),
    )
    for index, (label, surface) in enumerate(runway):
        y = -35.0 + index * 9.0
        add_surface_pad(
            f"SurfacePad_{index + 1:02d}_{label.replace(' ', '_')}",
            (0.0, y),
            (11.0, 8.0),
            surface,
            visual,
            collision,
        )
    add_surface_pad(
        "InstantBailHazard",
        (0.0, 31.0),
        (11.0, 5.0),
        material["hazard"],
        visual,
        collision,
    )
    add_zone_sign(
        "SURFACE RUNWAY", 0.0, -44.0, material["brick"],
        material["label"], visual,
    )

    # Left skate-geometry zone.
    add_zone_sign(
        "SKATE GEOMETRY", -37.0, -18.0, material["brick"],
        material["label"], visual,
    )
    base.add_wedge(
        "WideBank", -53.0, -43.0, -10.0, 2.0, 0.0, 4.2,
        material["orange"], visual, collision,
    )
    base.add_curved_ramp(
        "QuarterPipe", -43.0, 7.0, 17.0, 10.0, 5.2,
        material["orange"], visual, collision,
    )
    base.add_stair_set(
        "SixStairSet", -53.0, 22.0, 30.0, 1.0, 0.32, 6, 4.0,
        material["polished"], visual, collision,
    )
    base.add_box(
        "LongManualPad", (-30.0, -7.0, 0.32), (12.0, 5.0, 0.64),
        material["brick"], visual, collision, bevel=0.10,
    )
    base.add_box(
        "LowLedge", (-28.0, 6.0, 0.42), (11.0, 1.2, 0.84),
        material["metal_sheet"], visual, collision, bevel=0.05,
    )
    base.add_box(
        "HighLedge", (-31.0, 12.0, 0.72), (9.0, 1.4, 1.44),
        material["polished"], visual, collision, bevel=0.06,
    )

    # Grind zone with straight, sloped, and curved/polyline rails.
    add_zone_sign(
        "GRIND LAB", -20.0, 26.0, material["brick"],
        material["label"], visual,
    )
    add_rail_polyline(
        "StraightRail",
        [(-31.0, 34.0, 0.62), (-17.0, 34.0, 0.62)],
        material["metal"], visual, collision, grinds,
        radius=0.07,
    )
    add_rail_polyline(
        "DownRail",
        [(-31.0, 40.0, 0.95), (-24.0, 40.0, 0.72),
         (-17.0, 40.0, 0.50)],
        material["metal"], visual, collision, grinds,
        radius=0.07,
    )
    add_rail_polyline(
        "ArcRail",
        [
            (-30.0, 47.0, 0.62),
            (-27.0, 49.0, 0.62),
            (-23.0, 49.8, 0.62),
            (-19.0, 49.0, 0.62),
            (-16.0, 47.0, 0.62),
        ],
        material["metal"], visual, collision, grinds,
        radius=0.07,
    )
    for x, y, height in (
        (-31.0, 34.0, 0.62), (-17.0, 34.0, 0.62),
        (-31.0, 40.0, 0.95), (-17.0, 40.0, 0.50),
        (-30.0, 47.0, 0.62), (-16.0, 47.0, 0.62),
    ):
        base.add_box(
            f"RailPost_{x}_{y}", (x, y, height * 0.5),
            (0.16, 0.16, height), material["metal"], visual, collision,
        )

    # Retained in Blender as experimental authoring, but deliberately kept out
    # of OW_NPC_PATHS so this acceptance map requests zero AI skaters.
    add_npc_route(
        "OuterFlowLoop",
        [
            (-48.0, -43.0, 0.18),
            (-15.0, -50.0, 0.18),
            (24.0, -45.0, 0.18),
            (50.0, -22.0, 0.18),
            (50.0, 24.0, 0.18),
            (23.0, 49.0, 0.18),
            (-12.0, 51.0, 0.18),
            (-49.0, 27.0, 0.18),
        ],
        npc_paths,
        skaters=2,
        speed=6.2,
        spacing=14.0,
    )
    add_npc_route(
        "InnerCruiseLoop",
        [
            (-17.0, -24.0, 0.18),
            (16.0, -24.0, 0.18),
            (29.0, -4.0, 0.18),
            (18.0, 22.0, 0.18),
            (-15.0, 22.0, 0.18),
            (-28.0, 1.0, 0.18),
        ],
        npc_paths,
        skaters=1,
        speed=4.8,
        spacing=8.0,
    )

    # Right material/rendering gallery.
    add_zone_sign(
        "MATERIAL GALLERY", 36.0, -18.0, material["brick"],
        material["label"], visual,
    )
    base.add_box(
        "TransparentGlassWall", (24.0, -7.0, 2.5), (0.18, 8.0, 5.0),
        material["glass"], visual, collision, bevel=0.03,
    )
    base.add_box(
        "AlphaCutoutFence", (30.0, -7.0, 2.5), (0.12, 8.0, 5.0),
        material["fence"], visual, collision,
    )
    base.add_box(
        "CyanEmissiveWall", (37.0, -7.0, 2.5), (0.28, 8.0, 5.0),
        material["cyan"], visual, None, bevel=0.06,
    )
    base.add_box(
        "AmberEmissiveWall", (44.0, -7.0, 2.5), (0.28, 8.0, 5.0),
        material["amber"], visual, None, bevel=0.06,
    )
    for index, (x, surface) in enumerate(
        (
            (24.0, material["polished"]),
            (31.0, material["metal_sheet"]),
            (38.0, material["wood"]),
            (45.0, material["tile"]),
        ),
        1,
    ):
        base.add_box(
            f"PBRPedestal_{index:02d}", (x, 5.0, 0.65),
            (4.2, 4.2, 1.3), surface, visual, collision, bevel=0.18,
        )
        bpy.ops.mesh.primitive_uv_sphere_add(
            segments=32, ring_count=16, radius=1.25, location=(x, 5.0, 2.35)
        )
        sphere = bpy.context.object
        sphere.name = f"PBRMaterialSphere_{index:02d}"
        base.move_to(sphere, visual)
        sphere.data.materials.append(surface)
        base.metric_uv(sphere, metres_per_tile=2.0)

    # Local light lab. Each light has a neutral pedestal and a different
    # type/colour so Point, Spot, and Area behaviour is obvious at night.
    add_zone_sign(
        "LOCAL LIGHT LAB", 36.0, 15.0, material["brick"],
        material["label"], visual,
    )
    for index, x in enumerate((27.0, 36.0, 45.0), 1):
        base.add_box(
            f"LightPedestal_{index:02d}", (x, 23.0, 0.5),
            (5.0, 5.0, 1.0), material["polished"], visual, collision,
            bevel=0.12,
        )
    add_light(
        "FeaturePointBlue", "POINT", (27.0, 23.0, 4.0),
        (0.12, 0.38, 1.0), 1050.0, 12.0, size=0.28,
    )
    add_light(
        "FeatureSpotRed", "SPOT", (36.0, 23.0, 6.5),
        (1.0, 0.08, 0.025), 1450.0, 15.0,
        target=(36.0, 23.0, 0.0), size=0.18, spot_size=0.95,
    )
    add_light(
        "FeatureAreaGreen", "AREA", (45.0, 23.0, 5.5),
        (0.18, 1.0, 0.30), 900.0, 11.0,
        target=(45.0, 23.0, 0.0), size=2.4,
    )

    # Hinged-door test bay. The opening is large and isolated enough to walk
    # or skate through repeatedly without nearby collision ambiguity.
    add_zone_sign(
        "PHYSICS DOOR", 35.0, 31.0, material["brick"],
        material["label"], visual,
    )
    base.add_box(
        "DoorWallLeft", (27.5, 39.0, 2.2), (11.8, 0.7, 4.4),
        material["brick"], visual, collision, bevel=0.08,
    )
    base.add_box(
        "DoorWallRight", (42.5, 39.0, 2.2), (11.8, 0.7, 4.4),
        material["brick"], visual, collision, bevel=0.08,
    )
    base.add_box(
        "DoorHeader", (35.0, 39.0, 4.1), (3.2, 0.7, 0.6),
        material["brick"], visual, collision, bevel=0.06,
    )

    # Baked-indirect alcove: the roof and side walls make the coloured
    # indirect contribution easy to compare against direct dynamic lights.
    base.add_box(
        "BakeAlcoveRoof", (0.0, 48.0, 5.0), (18.0, 12.0, 0.5),
        material["metal"], visual, collision, bevel=0.10,
    )
    for x in (-8.75, 8.75):
        base.add_box(
            f"BakeAlcoveWall_{x}", (x, 48.0, 2.5), (0.5, 12.0, 5.0),
            material["brick"], visual, collision, bevel=0.08,
        )
    base.add_box(
        "BakeAlcoveCyanPanel", (-8.45, 48.0, 2.5), (0.18, 5.0, 3.8),
        material["cyan"], visual, None, bevel=0.04,
    )
    base.add_box(
        "BakeAlcoveAmberPanel", (8.45, 48.0, 2.5), (0.18, 5.0, 3.8),
        material["amber"], visual, None, bevel=0.04,
    )
    add_zone_sign(
        "BAKED INDIRECT", 0.0, 40.5, material["brick"],
        material["label"], visual,
    )


def build_scene() -> None:
    base.reset_scene()
    MANUAL_GRIND_SPECS.clear()
    visual = base.make_collection("OW_VISUAL")
    collision = base.make_collection("OW_COLLISION")
    grinds = base.make_collection("OW_GROUP_4_GRINDS")
    base.make_collection("OW_NPC_PATHS")
    npc_paths = base.make_collection("OW_NPC_PATHS_EXPERIMENTAL")
    materials, material_list = make_materials()
    build_static_park(materials, visual, collision, grinds, npc_paths)

    static_visuals = prepare_editable_objects(visual, collision, grinds)
    grind_stats = generate_grinds()
    print("Feature Park automatic grinds:", grind_stats)
    add_manual_grind_curves(grinds)
    print(
        "Feature Park manual grinds:",
        [name for name, _points in MANUAL_GRIND_SPECS],
    )
    lightmap = bpy.data.images.new(
        "LM_BakedIndirect_Showcase", width=1024, height=1024, alpha=True
    )
    lightmap.colorspace_settings.name = "Non-Color"
    base.add_bake_target(material_list, lightmap)

    world = bpy.data.worlds.new("OW_FeatureParkWorld")
    bpy.context.scene.world = world
    world.use_nodes = True
    background = world.node_tree.nodes.get("Background")
    background.inputs["Color"].default_value = (0.055, 0.09, 0.16, 1.0)
    background.inputs["Strength"].default_value = 0.28

    # These lights provide the baked indirect contrast and remain ordinary
    # runtime local lights after export.
    add_light(
        "BakeFillWarm", "AREA", (6.0, 47.0, 7.5),
        (1.0, 0.34, 0.08), 1150.0, 16.0,
        target=(0.0, 48.0, 1.0), size=5.0,
    )
    add_light(
        "BakeFillCool", "AREA", (-6.0, 47.0, 7.5),
        (0.06, 0.42, 1.0), 1050.0, 16.0,
        target=(0.0, 48.0, 1.0), size=5.0,
    )

    sun_data = bpy.data.lights.new("FeatureParkSun_Data", type="SUN")
    sun_data.color = (1.0, 0.78, 0.52)
    sun_data.energy = 2.7
    sun = bpy.data.objects.new("FeatureParkSun", sun_data)
    bpy.context.scene.collection.objects.link(sun)
    sun.rotation_euler = (math.radians(48.0), 0.0, math.radians(-34.0))

    spawn = bpy.data.objects.new("OW_SPAWN", None)
    bpy.context.scene.collection.objects.link(spawn)
    spawn.location = (0.0, -50.0, 0.78)
    spawn.empty_display_type = "ARROWS"
    spawn.empty_display_size = 1.4
    spawn["ow_heading_radians"] = 0.0

    scene = bpy.context.scene
    scene["ow_map_name"] = "blender_feature_park"
    scene["ow_sky_zenith"] = (0.055, 0.28, 0.68)
    scene["ow_sky_horizon"] = (0.56, 0.78, 1.0)
    scene["ow_sky_nadir"] = (0.15, 0.20, 0.30)
    scene["ow_cycle_seconds"] = 150.0
    scene["ow_start_hour"] = 7.0
    scene["ow_end_hour"] = 19.0
    scene["ow_cycle_ping_pong"] = True
    scene["ow_orbit_azimuth"] = 0.62
    scene["ow_twilight_zenith"] = (0.035, 0.075, 0.22)
    scene["ow_twilight_horizon"] = (1.0, 0.29, 0.07)
    scene["ow_twilight_nadir"] = (0.045, 0.028, 0.055)
    scene["ow_night_zenith"] = (0.004, 0.010, 0.038)
    scene["ow_night_horizon"] = (0.028, 0.060, 0.14)
    scene["ow_night_nadir"] = (0.006, 0.010, 0.024)
    scene["ow_moon_color"] = (0.38, 0.53, 0.95)
    scene["ow_moon_intensity"] = 0.20
    scene["ow_day_ambient"] = 0.30
    scene["ow_night_ambient"] = 0.10
    scene["ow_sky_tint"] = (1.0, 1.0, 1.0)

    base.configure_bake(static_visuals)
    print("Starting Feature Park indirect-light bake...")
    bpy.ops.object.bake(type="DIFFUSE")
    values = list(lightmap.pixels)
    rgb = [values[index] for index in range(0, len(values), 4)]
    print(
        "Feature Park bake result:",
        f"minimum={min(rgb):.4f}",
        f"maximum={max(rgb):.4f}",
        f"mean={sum(rgb) / len(rgb):.4f}",
    )
    base.persist_lightmap(lightmap, material_list)

    # Dynamic door is added after joining and baking static geometry so it
    # remains an independent rigid body in the exported package.
    add_hinged_door(materials["door"], visual)

    BLEND_PATH.parent.mkdir(parents=True, exist_ok=True)
    collision.hide_viewport = False
    grinds.hide_viewport = False
    npc_paths.hide_viewport = False
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str(BLEND_PATH), compress=True)
    export_scene(PACKAGE_PATH, force_rebuild=True)


if __name__ == "__main__":
    build_scene()
