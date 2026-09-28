"""Human-friendly Blender authoring and export tools for SKATE v15.

This addon and exporter are original project code. They do not import,
invoke, redistribute, or depend on ArenaBuilder.
"""

from __future__ import annotations

from array import array
import bpy
import hashlib
import importlib
import math
from pathlib import Path
import re
import traceback
from bpy.app.handlers import persistent
from bpy.props import (
    BoolProperty,
    EnumProperty,
    FloatProperty,
    FloatVectorProperty,
    IntProperty,
    PointerProperty,
    StringProperty,
)
from bpy.types import (
    Material,
    Object,
    Operator,
    Panel,
    PropertyGroup,
    Scene,
    UIList,
)
from bpy_extras.io_utils import ExportHelper

from . import exporter as _exporter

# Blender may replace an installed extension while keeping its Python package
# alive. Reload the implementation module so a newly installed panel cannot
# call an older cached exporter with an incompatible function signature.
exporter = importlib.reload(_exporter)


bl_info = {
    "name": "Owned World Authoring",
    "author": "Skate 3 Custom Engine Layer contributors",
    "version": (1, 15, 0),
    "blender": (5, 0, 0),
    "location": "3D View > Sidebar > Skate 3 Map",
    "description": "Create, validate, and export Skate 3 Custom Engine maps",
    "category": "Import-Export",
}


_SYNCING = False
BASE_COLOR_BAKE_SCHEMA = 4
_ALPHA_SCAN_CACHE: dict[int, tuple[bool, bool]] = {}


_AUDIO_NAMES = """
Undefined|Asphalt_Smooth|Asphalt_Rough|Concrete_Polished|Concrete_Rough|
Concrete_Aggregate|Wood_Ramp|Plywood|Dirt|Metal|Grass|
Metal_Solid_Round_1|Metal_Solid_Round_1_Up|Metal_Solid_Round_2|
Metal_Solid_Square_1|Metal_Solid_Square_2|Metal_Hollow_Round_1|
Metal_Hollow_Round_1_Dead|Metal_Hollow_Round_1_Dn|Metal_Hollow_Round_2|
Metal_Hollow_Round_2_Dead|Metal_Hollow_Round_2_Dn|Metal_Hollow_Round_3|
Metal_Hollow_Round_4|Metal_Hollow_Square_1|Metal_Hollow_Square_2|
Metal_Hollow_Square_3|Metal_Hollow_Square_3_Dead|Metal_Hollow_Square_4|
Metal_Hollow_1|Metal_Hollow_2|Metal_Sheet|Metal_Complex_1|Metal_Complex_2|
Metal_Complex_3|Metal_Complex_4|Metal_Complex_5|Metal_Complex_6|
Metal_Complex_7|Metal_Complex_8|Metal_Complex_Debris|Wood_1|Wood_1_Up|
Wood_2|Wood_3|Wood_3_Up|Wood_4|Plastic_1|Plastic_2|Plastic_3|Plastic_4|
Glass_Thick_Large|Glass_Thin_Small|Concrete_Curb|Concrete_Bench|Leaves|
Bush|Pottery|Paper|Cardboard|Garbage_Bag|Garbage_Spill|Bottle|
Tile_Ceramic|Marble_or_Slate|Brick_Smooth|Brick_Coarse|Manhole_Metal|
Metal_Grate_Sewer|Metal_Grate_Planter|DeepSnow|PackedSnow|Ice|Antennas|
Chandelier|Plexiglass_Small|Plexiglass_Large|Potted_Plant|Crumpled_Paper|
Cloth|Pop_Can|Paper_Cup|Wire_Cable|VolleyBall|OilDrum|DMORail|Fruit|
Plastic_Bottle|Drum_Pylon|Metal_Rail_4|Wood_5|Metal_Ramp|
Complex_Plastic_1|Max_Mappable_Surface
"""
_AUDIO_NAMES = [
    name.strip()
    for name in _AUDIO_NAMES.replace("\n", "").split("|")
]
AUDIO_ITEMS = [
    (str(index), name, f"Native Skate 3 audio surface {index}")
    for index, name in enumerate(_AUDIO_NAMES)
]

PHYSICS_ITEMS = [
    ("0", "Undefined", "Default behavior"),
    ("1", "Smooth", "Fast smooth riding"),
    ("2", "Rough", "Medium-friction riding"),
    ("3", "Slow", "Slows the rider"),
    ("4", "Slippery", "Low-friction riding"),
    ("5", "VerySlow", "Very slow / breadcrumb blocked"),
    ("6", "Unrideable", "Cannot be ridden"),
    ("7", "DoNotAlign", "Do not align board to this surface"),
    ("8", "Stair", "Native stair behavior"),
    ("9", "InstantBail", "Immediately forces a bail"),
    ("10", "SlipperyRagdoll", "Low-friction bail behavior"),
    ("11", "BouncyRagdoll", "Bouncy bail behavior"),
    ("12", "Water", "Native water/swimming behavior"),
    ("13", "Retail13", "Retail collision behavior 13"),
]

PATTERN_ITEMS = [
    ("0", "None", "No native surface pattern"),
    ("1", "SpiderCrack", "Cracked surface"),
    ("2", "Square2x2", "2 by 2 tiles"),
    ("3", "Square4x4", "4 by 4 tiles"),
    ("4", "Square8x8", "8 by 8 tiles"),
    ("5", "Square12x12", "12 by 12 tiles"),
    ("6", "Square24x24", "24 by 24 tiles"),
    ("7", "IrregularSmall", "Small irregular pattern"),
    ("8", "IrregularMedium", "Medium irregular pattern"),
    ("9", "IrregularLarge", "Large irregular pattern"),
    ("10", "Slats", "Slatted surface"),
    ("11", "Sidewalk", "Sidewalk pattern"),
    ("12", "BrickTileRandomSize", "Random-size brick tile"),
    ("13", "MiniTile", "Small tile pattern"),
    ("14", "Special1", "Native special pattern 1"),
    ("15", "Special2", "Native special pattern 2"),
]

ALPHA_ITEMS = [
    ("0", "Opaque", "Fully opaque"),
    ("1", "Cutout", "Alpha-tested leaves, grates, and fences"),
    ("2", "Blend", "Alpha-blended glass and soft surfaces"),
]

PRESENTATION_ITEMS = [
    ("STANDARD", "Standard", "Opaque or transparent world surface"),
    (
        "DECAL",
        "Alpha Decal",
        "Non-colliding alpha surface intended for signs, stains, and overlays",
    ),
    (
        "VEGETATION",
        "Tree / Vegetation",
        "Non-colliding alpha-cutout foliage optimized for clean edges",
    ),
]

PRESET_ITEMS = [
    ("CONCRETE", "Polished Concrete", ""),
    ("ROUGH_CONCRETE", "Rough Concrete", ""),
    ("ASPHALT", "Rough Asphalt", ""),
    ("WOOD", "Wood Ramp", ""),
    ("METAL_RAIL", "Metal Rail", ""),
    ("METAL_SHEET", "Metal Sheet", ""),
    ("GRASS", "Grass", ""),
    ("TILE", "Ceramic Tile", ""),
    ("GLASS", "Glass", ""),
    ("ICE", "Ice / Slippery", ""),
    ("STAIRS", "Concrete Stairs", ""),
    ("WATER", "Water", ""),
    ("INSTANT_BAIL", "Instant Bail", ""),
    ("DECAL", "Alpha Decal", ""),
    ("TREE", "Tree / Vegetation", ""),
]

PRESETS = {
    "CONCRETE": (3, 1, 0, 0.72, 0.0, 0),
    "ROUGH_CONCRETE": (4, 2, 0, 0.90, 0.0, 0),
    "ASPHALT": (2, 2, 7, 0.94, 0.0, 0),
    "WOOD": (6, 1, 10, 0.72, 0.0, 0),
    "METAL_RAIL": (11, 1, 0, 0.22, 0.88, 0),
    "METAL_SHEET": (31, 1, 0, 0.34, 0.78, 0),
    "GRASS": (10, 3, 7, 0.96, 0.0, 0),
    "TILE": (63, 1, 4, 0.38, 0.0, 0),
    "GLASS": (51, 1, 0, 0.08, 0.0, 2),
    "ICE": (72, 4, 0, 0.08, 0.0, 0),
    "STAIRS": (53, 8, 11, 0.84, 0.0, 0),
    "WATER": (0, 12, 0, 0.04, 0.0, 2),
    "INSTANT_BAIL": (4, 9, 0, 0.85, 0.0, 0),
    "DECAL": (3, 1, 0, 0.72, 0.0, 2),
    "TREE": (55, 3, 7, 0.88, 0.0, 1),
}

PHYSICS_TYPE_ITEMS = [
    (
        "STATIC",
        "Not Simulated",
        "Ordinary static world geometry; Box3D is disabled",
    ),
    (
        "BOX3D_STATIC",
        "Box3D Static",
        "A fixed Box3D collision body for dynamic owned-world objects",
    ),
    (
        "BOX3D_DYNAMIC",
        "Box3D Dynamic",
        "An independently falling and colliding Box3D rigid body",
    ),
    (
        "HINGED_DOOR",
        "Hinged Door",
        "Contact-driven rigid door constrained to one hinge axis",
    ),
]

BOX3D_SHAPE_ITEMS = [
    ("BOX", "Box", "Fast box fitted to this object's local bounds"),
    (
        "SPHERE",
        "Sphere",
        "Sphere fitted around this object's local bounds",
    ),
    (
        "CONVEX_HULL",
        "Convex Hull",
        "Convex hull generated from this object's collision vertices",
    ),
]


def _sync(settings: "OwnedWorldMaterialSettings") -> None:
    if _SYNCING:
        return
    material = settings.id_data
    if not isinstance(material, Material):
        return
    # Once an author changes any value in the material panel, preserve that
    # choice on future automatic scene preparation. Materials carrying the
    # True marker are still owned by the importer and may be refreshed from
    # their Blender shader graph to heal stale/partial imports.
    material["ow_auto_imported"] = False
    material["ow_audio_surface"] = int(settings.audio_surface)
    material["ow_physics_surface"] = int(settings.physics_surface)
    material["ow_surface_pattern"] = int(settings.surface_pattern)
    material["ow_alpha_mode"] = int(settings.alpha_mode)
    material["ow_alpha_mode_authored"] = True
    material["ow_alpha_cutoff"] = float(settings.alpha_cutoff)
    material["ow_roughness"] = float(settings.roughness)
    material["ow_metallic"] = float(settings.metallic)
    material["ow_friction"] = float(settings.friction)
    material["ow_restitution"] = float(settings.restitution)
    material["ow_emissive"] = float(settings.emissive_strength)
    material["ow_baked_strength"] = float(settings.baked_strength)
    material["ow_collision_enabled"] = bool(settings.collision_enabled)
    material["ow_presentation_type"] = settings.presentation_type
    for prop, image in (
        ("ow_albedo_image", settings.albedo_image),
        ("ow_lightmap_image", settings.lightmap_image),
        ("ow_normal_image", settings.normal_image),
        ("ow_orm_image", settings.orm_image),
        ("ow_emissive_image", settings.emissive_image),
    ):
        material[prop] = image.name if image is not None else ""


def _updated(self, _context) -> None:
    _sync(self)


def _sync_physics(settings: "OwnedWorldPhysicsSettings") -> None:
    if _SYNCING:
        return
    obj = settings.id_data
    if not isinstance(obj, Object):
        return
    obj["ow_physics_type"] = settings.physics_type
    obj["ow_box3d_collision_shape"] = settings.box3d_collision_shape
    obj["ow_box3d_density"] = float(settings.box3d_density)
    obj["ow_box3d_friction"] = float(settings.box3d_friction)
    obj["ow_box3d_restitution"] = float(settings.box3d_restitution)
    obj["ow_box3d_linear_damping"] = float(settings.box3d_linear_damping)
    obj["ow_box3d_angular_damping"] = float(settings.box3d_angular_damping)
    obj["ow_box3d_gravity_scale"] = float(settings.box3d_gravity_scale)
    obj["ow_box3d_enable_sleep"] = bool(settings.box3d_enable_sleep)
    obj["ow_box3d_initially_awake"] = bool(settings.box3d_initially_awake)
    obj["ow_box3d_break_group"] = int(settings.box3d_break_group)
    obj["ow_box3d_break_speed_threshold"] = float(
        settings.box3d_break_speed_threshold
    )
    obj["ow_box3d_break_impulse_scale"] = float(
        settings.box3d_break_impulse_scale
    )
    obj["ow_box3d_break_angular_impulse"] = float(
        settings.box3d_break_angular_impulse
    )
    obj["ow_box3d_break_gravity_scale"] = float(
        settings.box3d_break_gravity_scale
    )
    if settings.collision_material is not None:
        obj["ow_material"] = settings.collision_material.name
    obj["ow_upward_surface"] = bool(settings.upward_surface)
    obj["ow_hinge_axis"] = tuple(settings.hinge_axis)
    obj["ow_door_min_angle_degrees"] = math.degrees(
        float(settings.minimum_angle)
    )
    obj["ow_door_max_angle_degrees"] = math.degrees(
        float(settings.maximum_angle)
    )
    obj["ow_door_initial_angle_degrees"] = math.degrees(
        float(settings.initial_angle)
    )
    obj["ow_door_mass"] = float(settings.mass)
    obj["ow_door_angular_damping"] = float(settings.angular_damping)
    obj["ow_door_return_spring_strength"] = float(
        settings.return_spring_strength
    )
    obj["ow_door_maximum_angular_speed"] = float(
        settings.maximum_angular_speed
    )
    obj["ow_door_contact_impulse_scale"] = float(
        settings.contact_impulse_scale
    )
    obj["ow_door_friction"] = float(settings.friction)
    obj["ow_door_restitution"] = float(settings.restitution)


def _physics_updated(self, _context) -> None:
    _sync_physics(self)


class OwnedWorldMaterialSettings(PropertyGroup):
    presentation_type: EnumProperty(
        name="Presentation Type",
        items=PRESENTATION_ITEMS,
        default="STANDARD",
        update=_updated,
    )
    audio_surface: EnumProperty(
        name="Wheel / Grind Sound",
        items=AUDIO_ITEMS,
        default="3",
        update=_updated,
    )
    physics_surface: EnumProperty(
        name="Physics Behavior",
        items=PHYSICS_ITEMS,
        default="1",
        update=_updated,
    )
    surface_pattern: EnumProperty(
        name="Contact Pattern",
        items=PATTERN_ITEMS,
        default="0",
        update=_updated,
    )
    collision_enabled: BoolProperty(
        name="Collision Enabled",
        default=True,
        update=_updated,
    )
    alpha_mode: EnumProperty(
        name="Transparency",
        items=ALPHA_ITEMS,
        default="0",
        update=_updated,
    )
    alpha_cutoff: FloatProperty(
        name="Cutout Threshold",
        min=0.0,
        max=1.0,
        default=0.5,
        update=_updated,
    )
    roughness: FloatProperty(
        name="Fallback Roughness",
        min=0.0,
        max=1.0,
        default=0.78,
        update=_updated,
    )
    metallic: FloatProperty(
        name="Fallback Metallic",
        min=0.0,
        max=1.0,
        default=0.0,
        update=_updated,
    )
    friction: FloatProperty(
        name="Contact Friction",
        description="Physical friction exported for this surface",
        min=0.0,
        max=2.0,
        default=0.82,
        update=_updated,
    )
    restitution: FloatProperty(
        name="Bounce",
        min=0.0,
        max=1.0,
        default=0.0,
        update=_updated,
    )
    emissive_strength: FloatProperty(
        name="Emissive Strength",
        min=0.0,
        max=100.0,
        default=0.0,
        update=_updated,
    )
    baked_strength: FloatProperty(
        name="Baked Lighting Strength",
        min=0.0,
        max=8.0,
        default=1.0,
        update=_updated,
    )
    albedo_image: PointerProperty(
        name="Base Color / Albedo",
        type=bpy.types.Image,
        update=_updated,
    )
    lightmap_image: PointerProperty(
        name="Baked Lighting",
        type=bpy.types.Image,
        update=_updated,
    )
    normal_image: PointerProperty(
        name="Normal Map",
        type=bpy.types.Image,
        update=_updated,
    )
    orm_image: PointerProperty(
        name="ORM (AO/Rough/Metal)",
        type=bpy.types.Image,
        update=_updated,
    )
    emissive_image: PointerProperty(
        name="Emissive Map",
        type=bpy.types.Image,
        update=_updated,
    )


class OwnedWorldPhysicsSettings(PropertyGroup):
    collision_material: PointerProperty(
        name="Collision Material",
        description=(
            "Visual material that supplies this collision object's sound "
            "and physics behavior"
        ),
        type=Material,
        update=_physics_updated,
    )
    upward_surface: BoolProperty(
        name="Rideable Top Surface Only",
        description=(
            "Require every collision triangle to face upward; useful for "
            "floors and ramps"
        ),
        default=False,
        update=_physics_updated,
    )
    physics_type: EnumProperty(
        name="Physics Type",
        items=PHYSICS_TYPE_ITEMS,
        default="STATIC",
        update=_physics_updated,
    )
    box3d_collision_shape: EnumProperty(
        name="Collision Shape",
        description="Safe collision approximation used by Box3D",
        items=BOX3D_SHAPE_ITEMS,
        default="BOX",
        update=_physics_updated,
    )
    box3d_density: FloatProperty(
        name="Density",
        description=(
            "Mass per cubic metre; Box3D derives mass from density and shape"
        ),
        default=100.0,
        min=1.0,
        max=5000.0,
        update=_physics_updated,
    )
    box3d_friction: FloatProperty(
        name="Friction",
        default=0.55,
        min=0.0,
        max=2.0,
        update=_physics_updated,
    )
    box3d_restitution: FloatProperty(
        name="Bounce",
        default=0.05,
        min=0.0,
        max=1.0,
        update=_physics_updated,
    )
    box3d_linear_damping: FloatProperty(
        name="Linear Damping",
        description="Drag applied to straight-line motion",
        default=0.05,
        min=0.0,
        max=20.0,
        update=_physics_updated,
    )
    box3d_angular_damping: FloatProperty(
        name="Angular Damping",
        description="Drag applied to rotation",
        default=0.15,
        min=0.0,
        max=20.0,
        update=_physics_updated,
    )
    box3d_gravity_scale: FloatProperty(
        name="Gravity",
        description="Multiplier for world gravity; zero makes the body float",
        default=1.0,
        min=0.0,
        max=4.0,
        update=_physics_updated,
    )
    box3d_enable_sleep: BoolProperty(
        name="Allow Sleep",
        description="Let settled dynamic bodies stop consuming solver time",
        default=True,
        update=_physics_updated,
    )
    box3d_initially_awake: BoolProperty(
        name="Start Awake",
        description="Begin active instead of waiting for another body",
        default=True,
        update=_physics_updated,
    )
    box3d_break_group: IntProperty(
        name="Break Group",
        description=(
            "Zero disables breaking; matching nonzero groups release together"
        ),
        default=0,
        min=0,
        max=65535,
        update=_physics_updated,
    )
    box3d_break_speed_threshold: FloatProperty(
        name="Impact Speed",
        description="Minimum player impact speed needed to shatter the group",
        default=2.5,
        min=0.1,
        max=30.0,
        unit="VELOCITY",
        update=_physics_updated,
    )
    box3d_break_impulse_scale: FloatProperty(
        name="Shatter Force",
        description="How strongly the player's motion scatters released pieces",
        default=0.45,
        min=0.0,
        max=10.0,
        update=_physics_updated,
    )
    box3d_break_angular_impulse: FloatProperty(
        name="Shard Spin",
        description="Deterministic rotational kick applied on shatter",
        default=0.08,
        min=0.0,
        max=10.0,
        update=_physics_updated,
    )
    box3d_break_gravity_scale: FloatProperty(
        name="Released Gravity",
        description="Gravity applied after the break group is released",
        default=1.0,
        min=0.0,
        max=4.0,
        update=_physics_updated,
    )
    hinge_axis: FloatVectorProperty(
        name="Hinge Axis",
        description="World-space hinge axis; Z is vertical in Blender",
        size=3,
        default=(0.0, 0.0, 1.0),
        subtype="DIRECTION",
        update=_physics_updated,
    )
    minimum_angle: FloatProperty(
        name="Minimum Angle",
        subtype="ANGLE",
        default=-1.8325957,
        min=-3.1415927,
        max=0.0,
        update=_physics_updated,
    )
    maximum_angle: FloatProperty(
        name="Maximum Angle",
        subtype="ANGLE",
        default=1.8325957,
        min=0.0,
        max=3.1415927,
        update=_physics_updated,
    )
    initial_angle: FloatProperty(
        name="Initial Angle",
        subtype="ANGLE",
        default=0.0,
        min=-3.1415927,
        max=3.1415927,
        update=_physics_updated,
    )
    mass: FloatProperty(
        name="Mass",
        description="Door-leaf mass in kilograms",
        default=32.0,
        min=0.1,
        max=1000.0,
        unit="MASS",
        update=_physics_updated,
    )
    angular_damping: FloatProperty(
        name="Angular Damping",
        default=2.2,
        min=0.0,
        max=20.0,
        update=_physics_updated,
    )
    return_spring_strength: FloatProperty(
        name="Self-Close Strength",
        description=(
            "Angular spring strength that returns the leaf to its initial "
            "closed angle; zero disables self-closing"
        ),
        default=0.0,
        min=0.0,
        max=40.0,
        update=_physics_updated,
    )
    maximum_angular_speed: FloatProperty(
        name="Maximum Swing Speed",
        description="Maximum hinge speed in radians per second",
        default=8.0,
        min=0.05,
        max=30.0,
        update=_physics_updated,
    )
    contact_impulse_scale: FloatProperty(
        name="Push Response",
        description=(
            "Scales physical momentum transferred from the player to the door"
        ),
        default=1.0,
        min=0.0,
        max=4.0,
        update=_physics_updated,
    )
    friction: FloatProperty(
        name="Contact Friction",
        default=0.55,
        min=0.0,
        max=2.0,
        update=_physics_updated,
    )
    restitution: FloatProperty(
        name="Bounce",
        default=0.02,
        min=0.0,
        max=1.0,
        update=_physics_updated,
    )


def _sync_npc_path(settings: "OwnedWorldNpcPathSettings") -> None:
    if _SYNCING:
        return
    obj = settings.id_data
    if not isinstance(obj, Object):
        return
    obj["ow_npc_skater_count"] = int(settings.skater_count)
    obj["ow_npc_speed"] = float(settings.speed)
    obj["ow_npc_spawn_spacing"] = float(settings.spawn_spacing)


def _npc_path_updated(self, _context) -> None:
    _sync_npc_path(self)


class OwnedWorldNpcPathSettings(PropertyGroup):
    skater_count: IntProperty(
        name="Skaters",
        description="Number of native AI skaters assigned to this route",
        default=1,
        min=1,
        max=32,
        update=_npc_path_updated,
    )
    speed: FloatProperty(
        name="Route Speed",
        description="Target movement speed along this route in metres/second",
        default=5.5,
        min=0.1,
        max=30.0,
        unit="VELOCITY",
        update=_npc_path_updated,
    )
    spawn_spacing: FloatProperty(
        name="Spawn Spacing",
        description="Distance between skaters when a route has a population",
        default=3.0,
        min=0.0,
        max=100.0,
        unit="LENGTH",
        update=_npc_path_updated,
    )


def _sync_scene(settings: "OwnedWorldSceneSettings") -> None:
    if _SYNCING:
        return
    scene = settings.id_data
    if not isinstance(scene, Scene):
        return
    scene["ow_map_name"] = settings.map_name.strip() or "Untitled Map"
    scene["ow_cycle_seconds"] = float(settings.cycle_seconds)
    scene["ow_start_hour"] = float(settings.start_hour)
    scene["ow_end_hour"] = float(settings.end_hour)
    scene["ow_cycle_ping_pong"] = bool(settings.cycle_ping_pong)
    scene["ow_dynamic_lighting_enabled_by_default"] = bool(
        settings.dynamic_lighting_enabled_by_default
    )
    scene["ow_orbit_azimuth"] = float(settings.orbit_azimuth)
    scene["ow_sky_zenith"] = tuple(settings.sky_zenith)
    scene["ow_sky_horizon"] = tuple(settings.sky_horizon)
    scene["ow_sky_nadir"] = tuple(settings.sky_nadir)
    scene["ow_twilight_zenith"] = tuple(settings.twilight_zenith)
    scene["ow_twilight_horizon"] = tuple(settings.twilight_horizon)
    scene["ow_twilight_nadir"] = tuple(settings.twilight_nadir)
    scene["ow_night_zenith"] = tuple(settings.night_zenith)
    scene["ow_night_horizon"] = tuple(settings.night_horizon)
    scene["ow_night_nadir"] = tuple(settings.night_nadir)
    scene["ow_sun_color"] = tuple(settings.sun_color)
    scene["ow_moon_color"] = tuple(settings.moon_color)
    scene["ow_sun_intensity"] = float(settings.sun_intensity)
    scene["ow_moon_intensity"] = float(settings.moon_intensity)
    scene["ow_day_ambient"] = float(settings.day_ambient)
    scene["ow_night_ambient"] = float(settings.night_ambient)
    scene["ow_sky_tint"] = tuple(settings.sky_tint)
    spawn = bpy.data.objects.get(exporter.SPAWN_OBJECT)
    if spawn is not None:
        spawn["ow_heading_radians"] = float(settings.spawn_heading)


def _scene_updated(self, _context) -> None:
    _sync_scene(self)


def _material_list_updated(
    settings: "OwnedWorldSceneSettings", context
) -> None:
    if _SYNCING or not bpy.data.materials:
        return
    scene = settings.id_data
    if not isinstance(scene, Scene):
        return
    active_context = context or bpy.context
    view_layer = getattr(active_context, "view_layer", None)
    if view_layer is None:
        return
    index = max(
        0, min(int(settings.material_list_index), len(bpy.data.materials) - 1)
    )
    material = bpy.data.materials[index]
    matches = [
        obj
        for obj in view_layer.objects
        if obj.type == "MESH"
        and obj.name != exporter.SPAWN_OBJECT
        and any(slot.material == material for slot in obj.material_slots)
    ]
    for obj in view_layer.objects:
        try:
            obj.select_set(False)
        except RuntimeError:
            pass
    selected: list[bpy.types.Object] = []
    for obj in matches:
        if obj.hide_get() or obj.hide_select:
            continue
        try:
            obj.select_set(True)
            selected.append(obj)
        except RuntimeError:
            pass
    if selected:
        view_layer.objects.active = selected[0]


class OwnedWorldSceneSettings(PropertyGroup):
    material_list_index: IntProperty(
        name="Active Map Material",
        description="Highlighted material in the map-group assignment list",
        default=0,
        min=0,
        update=_material_list_updated,
    )
    map_name: StringProperty(
        name="Map Name",
        description="Name shown in the in-game Maps menu",
        default="My Skate Map",
        update=_scene_updated,
    )
    output_path: StringProperty(
        name="SKATE File",
        description="Destination package loaded by the game",
        default="//my_skate_map.skate",
        subtype="FILE_PATH",
    )
    cycle_seconds: FloatProperty(
        name="Day/Night Duration",
        description="Seconds per cycle; zero freezes the lighting",
        default=96.0,
        min=0.0,
        max=1800.0,
        unit="TIME",
        update=_scene_updated,
    )
    start_hour: FloatProperty(
        name="Start Hour",
        default=9.0,
        min=0.0,
        max=24.0,
        update=_scene_updated,
    )
    end_hour: FloatProperty(
        name="End Hour",
        default=17.0,
        min=0.0,
        max=24.0,
        update=_scene_updated,
    )
    cycle_ping_pong: BoolProperty(
        name="Move Between Start and End",
        description="Animate back and forth without passing through night",
        default=True,
        update=_scene_updated,
    )
    dynamic_lighting_enabled_by_default: BoolProperty(
        name="Dynamic Lighting On by Default",
        description=(
            "Initial per-map state for sun, moon, ambient light, and world "
            "shadows; players may still toggle it for the current session"
        ),
        default=True,
        update=_scene_updated,
    )
    orbit_azimuth: FloatProperty(
        name="Sun Orbit Direction",
        subtype="ANGLE",
        default=0.62,
        min=-math.pi,
        max=math.pi,
        update=_scene_updated,
    )
    spawn_heading: FloatProperty(
        name="Spawn Heading",
        subtype="ANGLE",
        default=0.0,
        min=-math.pi,
        max=math.pi,
        update=_scene_updated,
    )
    sky_zenith: FloatVectorProperty(
        name="Sky Top",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.09, 0.34, 0.72),
        update=_scene_updated,
    )
    sky_horizon: FloatVectorProperty(
        name="Sky Horizon",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.58, 0.78, 0.98),
        update=_scene_updated,
    )
    sky_nadir: FloatVectorProperty(
        name="Sky Bottom",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.18, 0.25, 0.34),
        update=_scene_updated,
    )
    twilight_zenith: FloatVectorProperty(
        name="Twilight Top",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.045, 0.10, 0.26),
        update=_scene_updated,
    )
    twilight_horizon: FloatVectorProperty(
        name="Twilight Horizon",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(1.0, 0.32, 0.10),
        update=_scene_updated,
    )
    twilight_nadir: FloatVectorProperty(
        name="Twilight Bottom",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.05, 0.035, 0.06),
        update=_scene_updated,
    )
    night_zenith: FloatVectorProperty(
        name="Night Top",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.007, 0.015, 0.045),
        update=_scene_updated,
    )
    night_horizon: FloatVectorProperty(
        name="Night Horizon",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.045, 0.085, 0.17),
        update=_scene_updated,
    )
    night_nadir: FloatVectorProperty(
        name="Night Bottom",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.008, 0.014, 0.032),
        update=_scene_updated,
    )
    sun_color: FloatVectorProperty(
        name="Sun Colour",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(1.0, 0.92, 0.78),
        update=_scene_updated,
    )
    moon_color: FloatVectorProperty(
        name="Moon Colour",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(0.42, 0.56, 0.92),
        update=_scene_updated,
    )
    sun_intensity: FloatProperty(
        name="Sun Strength",
        default=1.25,
        min=0.0,
        max=4.0,
        update=_scene_updated,
    )
    moon_intensity: FloatProperty(
        name="Moon Strength",
        default=0.18,
        min=0.0,
        max=2.0,
        update=_scene_updated,
    )
    day_ambient: FloatProperty(
        name="Day Ambient",
        default=0.32,
        min=0.0,
        max=1.0,
        update=_scene_updated,
    )
    night_ambient: FloatProperty(
        name="Night Ambient",
        default=0.11,
        min=0.0,
        max=1.0,
        update=_scene_updated,
    )
    sky_tint: FloatVectorProperty(
        name="Sky Colour",
        description="RGB tint multiplied over the day, twilight, and night sky palettes",
        subtype="COLOR",
        size=3,
        min=0.0,
        max=4.0,
        default=(1.0, 1.0, 1.0),
        update=_scene_updated,
    )
    show_advanced_lighting: BoolProperty(
        name="Advanced Lighting",
        description="Show twilight, night, sun and ambient map defaults",
        default=False,
    )
    show_sky_colours: BoolProperty(
        name="Sky Colour Palettes",
        description="Show the day, twilight, and night sky colour controls",
        default=False,
    )
    export_mode: EnumProperty(
        name="Export Mode",
        items=(
            (
                "AUTO",
                "Fast / Automatic",
                "Reuse unchanged map data and rebuild only when needed",
            ),
            (
                "FORCE",
                "Force Full Rebuild",
                "Ignore the incremental cache and rebuild everything",
            ),
            (
                "METADATA",
                "Lighting / Spawn Only",
                "Patch world and spawn settings into an existing package",
            ),
        ),
        default="AUTO",
    )
    export_editable_objects: BoolProperty(
        name="Editable Map Objects",
        description=(
            "Include existing map meshes as selectable editor objects; "
            "disable this to keep the exported map fully static while "
            "preserving its rendering, collision, materials, and grind paths"
        ),
        default=True,
    )
    last_status: StringProperty(
        name="Last Result",
        default="Run Validate Map before your first export.",
    )
    validation_details: StringProperty(
        name="Validation Details",
        default="",
        options={"HIDDEN"},
    )


class OWPHYSICS_OT_hinge_from_cursor(Operator):
    bl_idname = "owphysics.hinge_from_cursor"
    bl_label = "Set Hinge From 3D Cursor"
    bl_description = (
        "Store the current 3D cursor as this door's world-space hinge line"
    )
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        obj = context.object
        if obj is None or obj.type != "MESH":
            self.report({"ERROR"}, "Select a mesh object")
            return {"CANCELLED"}
        obj["ow_hinge_position"] = tuple(context.scene.cursor.location)
        obj.owned_world_physics.physics_type = "HINGED_DOOR"
        _sync_physics(obj.owned_world_physics)
        self.report({"INFO"}, f"Set hinge for {obj.name}")
        return {"FINISHED"}


def _ensure_collection(
    scene: bpy.types.Scene, name: str
) -> bpy.types.Collection:
    collection = bpy.data.collections.get(name)
    if collection is None:
        collection = bpy.data.collections.new(name)
    if collection.name not in scene.collection.children:
        scene.collection.children.link(collection)
    return collection


def _link_object(
    collection: bpy.types.Collection, obj: bpy.types.Object
) -> None:
    if obj.name not in collection.objects:
        collection.objects.link(obj)


def _group_collections() -> tuple[str, ...]:
    return exporter.GROUP_COLLECTIONS


def _object_groups(obj: bpy.types.Object) -> list[str]:
    names = set(_group_collections())
    return [
        collection.name
        for collection in obj.users_collection
        if collection.name in names
    ]


def _move_to_group(
    scene: bpy.types.Scene, obj: bpy.types.Object, group_name: str
) -> None:
    target = _ensure_collection(scene, group_name)
    for collection in tuple(obj.users_collection):
        if collection.name in _group_collections() and collection != target:
            collection.objects.unlink(obj)
    _link_object(target, obj)


def _migrate_legacy_groups(scene: bpy.types.Scene) -> int:
    legacy_visual = bpy.data.collections.get(
        exporter.LEGACY_VISUAL_COLLECTION
    )
    legacy_collision = bpy.data.collections.get(
        exporter.LEGACY_COLLISION_COLLECTION
    )
    visual_ids = {
        obj.as_pointer()
        for obj in legacy_visual.all_objects
    } if legacy_visual is not None else set()
    collision_ids = {
        obj.as_pointer()
        for obj in legacy_collision.all_objects
    } if legacy_collision is not None else set()
    migrated = 0
    for obj in scene.objects:
        identity = obj.as_pointer()
        if _object_groups(obj):
            continue
        if identity in visual_ids and identity in collision_ids:
            _move_to_group(
                scene, obj, exporter.PRESENTATION_COLLISION_COLLECTION
            )
        elif identity in visual_ids:
            _move_to_group(scene, obj, exporter.NO_COLLISION_COLLECTION)
        elif identity in collision_ids:
            _move_to_group(scene, obj, exporter.NO_PRESENTATION_COLLECTION)
        else:
            continue
        migrated += 1
    for old_name, new_name in (
        (exporter.LEGACY_GRIND_COLLECTION, exporter.GRIND_COLLECTION),
        (exporter.LEGACY_NPC_PATH_COLLECTION, exporter.NPC_PATH_COLLECTION),
    ):
        old = bpy.data.collections.get(old_name)
        if old is None:
            continue
        for obj in tuple(old.all_objects):
            if not _object_groups(obj):
                _move_to_group(scene, obj, new_name)
                migrated += 1
    for old_name in (
        exporter.LEGACY_VISUAL_COLLECTION,
        exporter.LEGACY_COLLISION_COLLECTION,
        exporter.LEGACY_GRIND_COLLECTION,
        exporter.LEGACY_NPC_PATH_COLLECTION,
    ):
        old = bpy.data.collections.get(old_name)
        if old is None:
            continue
        for obj in tuple(old.objects):
            if _object_groups(obj):
                old.objects.unlink(obj)
        if not old.objects and not old.children:
            bpy.data.collections.remove(old)
    return migrated


def _selected_of_type(context, object_type: str) -> list[bpy.types.Object]:
    return [
        obj for obj in context.selected_objects if obj.type == object_type
    ]


_AUTO_HELPER_OBJECT_MARKERS = (
    "character_size",
    "character size",
    "player_size",
    "player size",
    "scale_reference",
    "scale reference",
)


def _object_semantics(obj: bpy.types.Object) -> str:
    names = [obj.name, getattr(obj.data, "name", "")]
    current = obj.parent
    while current is not None:
        names.append(current.name)
        current = current.parent
    names.extend(
        slot.material.name
        for slot in obj.material_slots
        if slot.material is not None
    )
    return " ".join(names).lower()


def _is_auto_helper_object(obj: bpy.types.Object) -> bool:
    object_identity = (
        f"{obj.name} {getattr(obj.data, 'name', '')}".lower()
    )
    return any(
        marker in object_identity
        for marker in _AUTO_HELPER_OBJECT_MARKERS
    )


def _auto_visual_candidate(obj: bpy.types.Object) -> bool:
    name = obj.name.lower()
    lod_match = re.search(r"(^|[_. -])lod([1-9])($|[_. -])", name)
    has_lod0_sibling = bool(
        lod_match
        and obj.parent is not None
        and any(
            child.type == "MESH"
            and re.search(
                r"(^|[_. -])lod0($|[_. -])", child.name.lower()
            )
            for child in obj.parent.children
        )
    )
    return (
        obj.type == "MESH"
        and len(obj.data.polygons) > 0
        and not obj.hide_render
        and bool(obj.get("ow_export_visual", True))
        and not _is_auto_helper_object(obj)
        and not bool(obj.get("sk8_explicitly_ignored", False))
        and not has_lod0_sibling
    )


def _walk_node_output_images(
    node,
    output_socket,
    images: list[bpy.types.Image],
    visited: set[tuple[int, str, tuple[int, ...]]],
    group_stack: tuple[bpy.types.Node, ...] = (),
) -> None:
    socket_identity = str(
        getattr(output_socket, "identifier", "") or output_socket.name
    )
    stack_identity = tuple(
        group_node.as_pointer() for group_node in group_stack
    )
    identity = (node.as_pointer(), socket_identity, stack_identity)
    if identity in visited:
        return
    visited.add(identity)
    if node.type == "TEX_IMAGE" and node.image is not None:
        if node.image not in images:
            images.append(node.image)
        return
    if node.type == "GROUP" and node.node_tree is not None:
        matched_output = False
        for group_output in node.node_tree.nodes:
            if group_output.type != "GROUP_OUTPUT":
                continue
            inner_socket = group_output.inputs.get(output_socket.name)
            if inner_socket is None:
                inner_socket = next(
                    (
                        candidate
                        for candidate in group_output.inputs
                        if str(getattr(candidate, "identifier", ""))
                        == socket_identity
                    ),
                    None,
                )
            if inner_socket is None:
                continue
            matched_output = True
            for link in inner_socket.links:
                _walk_node_output_images(
                    link.from_node,
                    link.from_socket,
                    images,
                    visited,
                    (*group_stack, node),
                )
        if matched_output:
            return
    if node.type == "GROUP_INPUT" and group_stack:
        group_node = group_stack[-1]
        outer_socket = group_node.inputs.get(output_socket.name)
        if outer_socket is None:
            outer_socket = next(
                (
                    candidate
                    for candidate in group_node.inputs
                    if str(getattr(candidate, "identifier", ""))
                    == socket_identity
                ),
                None,
            )
        if outer_socket is not None:
            for link in outer_socket.links:
                _walk_node_output_images(
                    link.from_node,
                    link.from_socket,
                    images,
                    visited,
                    group_stack[:-1],
                )
        return
    if node.type in {"MIX", "MIX_RGB"}:
        # A Mix node's factor controls the blend; it is not itself the
        # resulting colour/normal. Generic input-order traversal visited the
        # factor first, so layered ground materials exported a height/blend
        # mask as their visible albedo. Prefer the actual A/B value inputs.
        if node.type == "MIX":
            suffix = {
                "FLOAT": "_Float",
                "VECTOR": "_Vector",
                "RGBA": "_Color",
            }.get(str(getattr(node, "data_type", "RGBA")), "_Color")
            value_inputs = [
                socket
                for socket in node.inputs
                if socket.name in {"A", "B"}
                and str(getattr(socket, "identifier", "")).endswith(suffix)
            ]
            factor_inputs = [
                socket
                for socket in node.inputs
                if socket.name == "Factor"
                and (
                    str(getattr(socket, "identifier", "")).endswith("_Float")
                    or not str(getattr(socket, "identifier", ""))
                )
            ]
        else:
            value_inputs = [
                socket
                for socket in node.inputs
                if socket.name in {"Color1", "Color2"}
            ]
            factor_inputs = [
                socket for socket in node.inputs if socket.name == "Fac"
            ]

        mixed_images: list[bpy.types.Image] = []
        for input_socket in value_inputs:
            for link in input_socket.links:
                _walk_node_output_images(
                    link.from_node,
                    link.from_socket,
                    mixed_images,
                    visited,
                    group_stack,
                )
        if mixed_images:
            # When the current one-texture package cannot preserve both
            # layers, retain the highest-detail visible layer. Never replace
            # it with a usually higher-resolution control mask.
            mixed_images.sort(
                key=lambda image: int(image.size[0]) * int(image.size[1]),
                reverse=True,
            )
            for image in mixed_images:
                if image not in images:
                    images.append(image)
            return

        defaults = [
            tuple(float(value) for value in socket.default_value)
            if hasattr(socket.default_value, "__len__")
            else (float(socket.default_value),)
            for socket in value_inputs
        ]
        if len(defaults) >= 2 and defaults[0] != defaults[1]:
            # If A/B are unequal constants, the factor really is the output
            # data (for example an opacity mask between zero and one).
            for input_socket in factor_inputs:
                for link in input_socket.links:
                    _walk_node_output_images(
                        link.from_node,
                        link.from_socket,
                        images,
                        visited,
                        group_stack,
                    )
        return
    for input_socket in node.inputs:
        for link in input_socket.links:
            _walk_node_output_images(
                link.from_node,
                link.from_socket,
                images,
                visited,
                group_stack,
            )


def _node_images_upstream(
    socket,
    group_stack: tuple[bpy.types.Node, ...] = (),
) -> list[bpy.types.Image]:
    images: list[bpy.types.Image] = []
    visited: set[tuple[int, str, tuple[int, ...]]] = set()
    for link in socket.links:
        _walk_node_output_images(
            link.from_node,
            link.from_socket,
            images,
            visited,
            group_stack,
        )
    return images


def _walk_effective_input(
    node,
    output_socket,
    group_stack: tuple[bpy.types.Node, ...],
    visited: set[tuple[int, str, tuple[int, ...]]],
) -> bool:
    socket_identity = str(
        getattr(output_socket, "identifier", "") or output_socket.name
    )
    stack_identity = tuple(
        group_node.as_pointer() for group_node in group_stack
    )
    identity = (node.as_pointer(), socket_identity, stack_identity)
    if identity in visited:
        return False
    visited.add(identity)
    if node.type == "GROUP_INPUT" and group_stack:
        group_node = group_stack[-1]
        outer_socket = group_node.inputs.get(output_socket.name)
        if outer_socket is None:
            outer_socket = next(
                (
                    candidate
                    for candidate in group_node.inputs
                    if str(getattr(candidate, "identifier", ""))
                    == socket_identity
                ),
                None,
            )
        if outer_socket is None:
            return False
        return any(
            _walk_effective_input(
                link.from_node,
                link.from_socket,
                group_stack[:-1],
                visited,
            )
            for link in outer_socket.links
        )
    # A concrete node outside a Group Input is authored graph input, whether
    # it is an image, a procedural operation, or a constant RGB/value node.
    return True


def _socket_has_effective_input(
    node: bpy.types.Node | None,
    socket_name: str,
    group_stack: tuple[bpy.types.Node, ...] = (),
) -> bool:
    if node is None:
        return False
    socket = node.inputs.get(socket_name)
    if socket is None:
        return False
    return any(
        _walk_effective_input(
            link.from_node,
            link.from_socket,
            group_stack,
            set(),
        )
        for link in socket.links
    )


def _nodes_recursive(node_tree, visited_trees: set[int] | None = None):
    if node_tree is None:
        return
    if visited_trees is None:
        visited_trees = set()
    identity = node_tree.as_pointer()
    if identity in visited_trees:
        return
    visited_trees.add(identity)
    for node in node_tree.nodes:
        yield node
        if node.type == "GROUP" and node.node_tree is not None:
            yield from _nodes_recursive(node.node_tree, visited_trees)


def _walk_shader_context(
    node,
    output_socket,
    group_stack: tuple[bpy.types.Node, ...],
    visited: set[tuple[int, str, tuple[int, ...]]],
    shader_type: str,
) -> tuple[bpy.types.Node, tuple[bpy.types.Node, ...]] | None:
    socket_identity = str(
        getattr(output_socket, "identifier", "") or output_socket.name
    )
    stack_identity = tuple(
        group_node.as_pointer() for group_node in group_stack
    )
    identity = (node.as_pointer(), socket_identity, stack_identity)
    if identity in visited:
        return None
    visited.add(identity)
    if node.type == shader_type:
        return node, group_stack
    if node.type == "GROUP" and node.node_tree is not None:
        for group_output in node.node_tree.nodes:
            if group_output.type != "GROUP_OUTPUT":
                continue
            inner_socket = group_output.inputs.get(output_socket.name)
            if inner_socket is None:
                inner_socket = next(
                    (
                        candidate
                        for candidate in group_output.inputs
                        if str(getattr(candidate, "identifier", ""))
                        == socket_identity
                    ),
                    None,
                )
            if inner_socket is None:
                continue
            for link in inner_socket.links:
                result = _walk_shader_context(
                    link.from_node,
                    link.from_socket,
                    (*group_stack, node),
                    visited,
                    shader_type,
                )
                if result is not None:
                    return result
        return None
    if node.type == "GROUP_INPUT" and group_stack:
        group_node = group_stack[-1]
        outer_socket = group_node.inputs.get(output_socket.name)
        if outer_socket is None:
            outer_socket = next(
                (
                    candidate
                    for candidate in group_node.inputs
                    if str(getattr(candidate, "identifier", ""))
                    == socket_identity
                ),
                None,
            )
        if outer_socket is not None:
            for link in outer_socket.links:
                result = _walk_shader_context(
                    link.from_node,
                    link.from_socket,
                    group_stack[:-1],
                    visited,
                    shader_type,
                )
                if result is not None:
                    return result
        return None
    for input_socket in node.inputs:
        for link in input_socket.links:
            result = _walk_shader_context(
                link.from_node,
                link.from_socket,
                group_stack,
                visited,
                shader_type,
            )
            if result is not None:
                return result
    return None


def _principled_context(
    material: bpy.types.Material,
) -> tuple[bpy.types.Node | None, tuple[bpy.types.Node, ...]]:
    tree = material.node_tree
    if tree is None:
        return None, ()
    outputs = sorted(
        (
            node
            for node in tree.nodes
            if node.type == "OUTPUT_MATERIAL"
            and bool(getattr(node, "is_active_output", True))
        ),
        key=lambda node: node.name.casefold(),
    )
    for output in outputs:
        surface = output.inputs.get("Surface")
        if surface is None:
            continue
        for link in surface.links:
            result = _walk_shader_context(
                link.from_node,
                link.from_socket,
                (),
                set(),
                "BSDF_PRINCIPLED",
            )
            if result is not None:
                return result
    return None, ()


def _glass_context(
    material: bpy.types.Material,
) -> tuple[bpy.types.Node | None, tuple[bpy.types.Node, ...]]:
    tree = material.node_tree
    if tree is None:
        return None, ()
    outputs = sorted(
        (
            node
            for node in tree.nodes
            if node.type == "OUTPUT_MATERIAL"
            and bool(getattr(node, "is_active_output", True))
        ),
        key=lambda node: node.name.casefold(),
    )
    for output in outputs:
        surface = output.inputs.get("Surface")
        if surface is None:
            continue
        for link in surface.links:
            result = _walk_shader_context(
                link.from_node,
                link.from_socket,
                (),
                set(),
                "BSDF_GLASS",
            )
            if result is not None:
                return result
    return None, ()


def _principled_node(
    material: bpy.types.Material,
) -> bpy.types.Node | None:
    return _principled_context(material)[0]


def _socket_image(
    node: bpy.types.Node | None,
    socket_name: str,
    group_stack: tuple[bpy.types.Node, ...] = (),
) -> bpy.types.Image | None:
    if node is None:
        return None
    socket = node.inputs.get(socket_name)
    if socket is None:
        return None
    images = _node_images_upstream(socket, group_stack)
    return images[0] if images else None


def _resolve_effective_source(
    node,
    output_socket,
    group_stack: tuple[bpy.types.Node, ...],
    visited: set[tuple[int, str, tuple[int, ...]]] | None = None,
):
    """Resolve Group Input/Output plumbing to the concrete upstream node."""
    if visited is None:
        visited = set()
    socket_identity = str(
        getattr(output_socket, "identifier", "") or output_socket.name
    )
    stack_identity = tuple(
        group_node.as_pointer() for group_node in group_stack
    )
    identity = (node.as_pointer(), socket_identity, stack_identity)
    if identity in visited:
        return None
    visited.add(identity)
    if node.type == "GROUP_INPUT" and group_stack:
        group_node = group_stack[-1]
        outer_socket = group_node.inputs.get(output_socket.name)
        if outer_socket is None:
            outer_socket = next(
                (
                    candidate
                    for candidate in group_node.inputs
                    if str(getattr(candidate, "identifier", ""))
                    == socket_identity
                ),
                None,
            )
        if outer_socket is not None:
            for link in outer_socket.links:
                resolved = _resolve_effective_source(
                    link.from_node,
                    link.from_socket,
                    group_stack[:-1],
                    visited,
                )
                if resolved is not None:
                    return resolved
        return None
    if node.type == "GROUP" and node.node_tree is not None:
        for group_output in node.node_tree.nodes:
            if group_output.type != "GROUP_OUTPUT":
                continue
            inner_socket = group_output.inputs.get(output_socket.name)
            if inner_socket is None:
                inner_socket = next(
                    (
                        candidate
                        for candidate in group_output.inputs
                        if str(getattr(candidate, "identifier", ""))
                        == socket_identity
                    ),
                    None,
                )
            if inner_socket is None:
                continue
            for link in inner_socket.links:
                resolved = _resolve_effective_source(
                    link.from_node,
                    link.from_socket,
                    (*group_stack, node),
                    visited,
                )
                if resolved is not None:
                    return resolved
        return None
    return node, output_socket, group_stack


def _effective_input_source(
    node: bpy.types.Node | None,
    socket_name: str,
    group_stack: tuple[bpy.types.Node, ...] = (),
):
    if node is None:
        return None
    socket = node.inputs.get(socket_name)
    if socket is None:
        return None
    for link in socket.links:
        resolved = _resolve_effective_source(
            link.from_node, link.from_socket, group_stack
        )
        if resolved is not None:
            return resolved
    return None


def _mix_value_and_factor_inputs(node):
    if node.type == "MIX":
        suffix = {
            "FLOAT": "_Float",
            "VECTOR": "_Vector",
            "RGBA": "_Color",
        }.get(str(getattr(node, "data_type", "RGBA")), "_Color")
        values = [
            socket
            for socket in node.inputs
            if socket.name in {"A", "B"}
            and str(getattr(socket, "identifier", "")).endswith(suffix)
        ]
        factors = [
            socket
            for socket in node.inputs
            if socket.name == "Factor"
            and (
                str(getattr(socket, "identifier", "")).endswith("_Float")
                or not str(getattr(socket, "identifier", ""))
            )
        ]
    elif node.type == "MIX_RGB":
        values = [
            socket
            for socket in node.inputs
            if socket.name in {"Color1", "Color2"}
        ]
        factors = [
            socket for socket in node.inputs if socket.name == "Fac"
        ]
    else:
        return [], []
    return values, factors


def _resolved_socket_source(
    socket: bpy.types.NodeSocket,
    group_stack: tuple[bpy.types.Node, ...],
):
    for link in socket.links:
        resolved = _resolve_effective_source(
            link.from_node, link.from_socket, group_stack
        )
        if resolved is not None:
            return resolved
    return None


def _direct_image_from_socket(
    socket: bpy.types.NodeSocket,
    group_stack: tuple[bpy.types.Node, ...],
) -> tuple[bpy.types.Image | None, bpy.types.NodeSocket | None]:
    """Resolve only plumbing, not arbitrary shader operations."""
    resolved = _resolved_socket_source(socket, group_stack)
    visited: set[int] = set()
    while resolved is not None:
        node, output, source_stack = resolved
        identity = node.as_pointer()
        if identity in visited:
            return None, None
        visited.add(identity)
        if node.type == "TEX_IMAGE" and node.image is not None:
            return node.image, output
        if node.type == "REROUTE":
            input_socket = node.inputs[0] if node.inputs else None
            if input_socket is None:
                return None, None
            resolved = _resolved_socket_source(input_socket, source_stack)
            continue
        return None, None
    return None, None


def _direct_input_image(
    node: bpy.types.Node | None,
    socket_name: str,
    group_stack: tuple[bpy.types.Node, ...] = (),
) -> bpy.types.Image | None:
    if node is None:
        return None
    socket = node.inputs.get(socket_name)
    if socket is None:
        return None
    return _direct_image_from_socket(socket, group_stack)[0]


def _direct_factor_image(
    socket: bpy.types.NodeSocket,
    group_stack: tuple[bpy.types.Node, ...],
) -> tuple[bpy.types.Image | None, int]:
    resolved = _resolved_socket_source(socket, group_stack)
    if resolved is None:
        return None, 0
    node, output, source_stack = resolved
    if node.type == "TEX_IMAGE" and node.image is not None:
        output_name = str(output.name).casefold()
        return node.image, 4 if output_name == "alpha" else 0
    if node.type not in {"SEPARATE_COLOR", "SEPARATE_XYZ"}:
        return None, -1
    input_socket = node.inputs.get("Color")
    if input_socket is None:
        input_socket = node.inputs.get("Vector")
    if input_socket is None and node.inputs:
        input_socket = node.inputs[0]
    if input_socket is None:
        return None, -1
    image, _image_output = _direct_image_from_socket(
        input_socket, source_stack
    )
    if image is None:
        return None, -1
    output_name = str(output.name).casefold()
    channel = {
        "red": 1,
        "r": 1,
        "x": 1,
        "green": 2,
        "g": 2,
        "y": 2,
        "blue": 3,
        "b": 3,
        "z": 3,
        "alpha": 4,
    }.get(output_name, 0)
    return image, channel


def _ordinary_mix_layers(
    shader: bpy.types.Node | None,
    group_stack: tuple[bpy.types.Node, ...],
) -> tuple[
    bpy.types.Image | None,
    bpy.types.Image | None,
    bpy.types.Image | None,
    float,
    int,
]:
    """Return A, B, factor image, scalar factor and factor channel."""
    source = _effective_input_source(
        shader, "Base Color", group_stack
    )
    if source is None:
        return None, None, None, 0.0, 0
    node, _output, source_stack = source
    if node.type not in {"MIX", "MIX_RGB"}:
        return None, None, None, 0.0, 0
    if (
        str(getattr(node, "blend_type", "MIX")) != "MIX"
    ):
        return None, None, None, 0.0, 0
    value_inputs, factor_inputs = _mix_value_and_factor_inputs(node)
    if len(value_inputs) < 2 or not factor_inputs:
        return None, None, None, 0.0, 0
    layers = [
        _direct_image_from_socket(socket, source_stack)[0]
        for socket in value_inputs[:2]
    ]
    if layers[0] is None or layers[1] is None:
        return None, None, None, 0.0, 0
    factor_socket = factor_inputs[0]
    factor = max(
        0.0, min(1.0, float(factor_socket.default_value))
    )
    factor_channel = 0
    if factor_socket.links:
        factor_image, factor_channel = _direct_factor_image(
            factor_socket, source_stack
        )
        if factor_channel < 0:
            return None, None, None, 0.0, 0
    else:
        factor_image = None
    return (
        layers[0],
        layers[1],
        factor_image,
        factor,
        factor_channel,
    )


def _image_address_mode(
    material: bpy.types.Material,
    image: bpy.types.Image | None,
) -> int:
    if image is None:
        return 0
    extensions = {
        str(getattr(node, "extension", "REPEAT"))
        for node in _nodes_recursive(material.node_tree)
        if node.type == "TEX_IMAGE" and node.image == image
    }
    if len(extensions) != 1:
        return 0
    return {
        "REPEAT": 0,
        "EXTEND": 1,
        "CLIP": 2,
        "MIRROR": 3,
    }.get(extensions.pop(), 0)


def _image_uv_map_name(
    material: bpy.types.Material,
    image: bpy.types.Image | None,
) -> str:
    """Resolve an unambiguous UV Map node feeding an Image Texture."""
    if image is None or material.node_tree is None:
        return ""
    image_nodes = [
        node
        for node in _nodes_recursive(material.node_tree)
        if node.type == "TEX_IMAGE" and node.image == image
    ]
    if len(image_nodes) != 1:
        return ""
    pending = list(image_nodes[0].inputs.get("Vector").links)
    visited: set[int] = set()
    while pending:
        link = pending.pop(0)
        node = link.from_node
        identity = node.as_pointer()
        if identity in visited:
            continue
        visited.add(identity)
        if node.type == "UVMAP":
            return str(node.uv_map)
        for input_socket in node.inputs:
            pending.extend(input_socket.links)
    return ""


def _image_uv_transform(
    material: bpy.types.Material,
    image: bpy.types.Image | None,
) -> tuple[float, float, float, float, float]:
    """Read the common UV -> Mapping -> Image affine transform."""
    identity = (1.0, 1.0, 0.0, 0.0, 0.0)
    if image is None or material.node_tree is None:
        return identity
    image_nodes = [
        node
        for node in _nodes_recursive(material.node_tree)
        if node.type == "TEX_IMAGE" and node.image == image
    ]
    if len(image_nodes) != 1:
        return identity
    vector_socket = image_nodes[0].inputs.get("Vector")
    if vector_socket is None or len(vector_socket.links) != 1:
        return identity
    mapping = vector_socket.links[0].from_node
    if (
        mapping.type != "MAPPING"
        or str(getattr(mapping, "vector_type", "POINT")) != "POINT"
    ):
        return identity
    location = mapping.inputs.get("Location")
    rotation = mapping.inputs.get("Rotation")
    scale = mapping.inputs.get("Scale")
    if (
        location is None
        or rotation is None
        or scale is None
        or location.links
        or rotation.links
        or scale.links
    ):
        return identity
    return (
        float(scale.default_value[0]),
        float(scale.default_value[1]),
        float(rotation.default_value[2]),
        float(location.default_value[0]),
        float(location.default_value[1]),
    )


def _socket_float(
    node: bpy.types.Node | None, socket_name: str, fallback: float
) -> float:
    if node is None:
        return fallback
    socket = node.inputs.get(socket_name)
    return float(socket.default_value) if socket is not None else fallback


def _socket_color(
    node: bpy.types.Node | None,
    socket_name: str,
    fallback: tuple[float, float, float],
) -> tuple[float, float, float]:
    if node is None:
        return fallback
    socket = node.inputs.get(socket_name)
    if socket is None:
        return fallback
    return tuple(float(value) for value in socket.default_value[:3])


def _image_alpha_usage(
    image: bpy.types.Image | None,
) -> tuple[bool, bool]:
    """Return (uses_alpha, blend_instead_of_cutout) for imported colour art."""
    if (
        image is None
        or image.channels < 4
        or image.size[0] <= 0
        or image.size[1] <= 0
        or _is_generated_material_map(image)
    ):
        return False, False
    identity = image.as_pointer()
    cached = _ALPHA_SCAN_CACHE.get(identity)
    if cached is not None:
        return cached

    pixel_count = int(image.size[0]) * int(image.size[1])
    if pixel_count <= 0:
        return False, False
    values = array("f", [0.0]) * (pixel_count * 4)
    image.pixels.foreach_get(values)
    sample_stride = max(1, pixel_count // 16384)
    sample_indices = range(0, pixel_count, sample_stride)
    alpha_min = 1.0
    alpha_max = 0.0
    alpha_sum = 0.0
    low_alpha = 0
    opaque_alpha = 0
    sample_count = 0
    for pixel_index in sample_indices:
        alpha = max(
            0.0, min(1.0, float(values[pixel_index * 4 + 3]))
        )
        alpha_min = min(alpha_min, alpha)
        alpha_max = max(alpha_max, alpha)
        alpha_sum += alpha
        low_alpha += int(alpha <= 0.5)
        opaque_alpha += int(alpha >= 0.995)
        sample_count += 1
    alpha_mean = alpha_sum / sample_count
    low_fraction = low_alpha / sample_count
    opaque_fraction = opaque_alpha / sample_count
    # Ignore tiny compression/noise excursions in otherwise opaque images.
    # A real cutout has a measurable transparent field, or is globally
    # translucent. Sampling is bounded so Auto Prepare remains practical on
    # very large imported texture sets.
    uses_alpha = (
        alpha_min < 0.5
        and (low_fraction >= 0.002 or alpha_mean < 0.98)
    ) or alpha_mean < 0.90
    # Art with almost no fully opaque pixels is a soft transparency mask, not
    # a binary cutout. A 0.5 clip would erase faint stains and leave harsh
    # edges on decals. Real cutout cards such as leaves normally contain a
    # meaningful opaque field and remain on the cheaper cutout path.
    blend = uses_alpha and (
        alpha_max < 0.5 or opaque_fraction < 0.005
    )
    result = uses_alpha, blend
    _ALPHA_SCAN_CACHE[identity] = result
    return result


def _remove_legacy_inferred_alpha(node_tree) -> int:
    if node_tree is None:
        return 0
    removed = 0
    visited: set[int] = set()

    def visit(tree) -> None:
        nonlocal removed
        identity = tree.as_pointer()
        if identity in visited:
            return
        visited.add(identity)
        for node in list(tree.nodes):
            if (
                node.type == "TEX_IMAGE"
                and node.name == "SKATE Imported Albedo Alpha"
            ):
                tree.nodes.remove(node)
                removed += 1
            elif node.type == "GROUP" and node.node_tree is not None:
                visit(node.node_tree)

    visit(node_tree)
    return removed


def _infer_contact_preset(material: bpy.types.Material) -> str:
    name = material.name.lower()
    rules = (
        (("water",), "WATER"),
        (("glass", "window"), "GLASS"),
        (("ice",), "ICE"),
        (("grass", "dirt", "soil", "mud"), "GRASS"),
        (("stair", "step"), "STAIRS"),
        (("wood", "plywood", "timber"), "WOOD"),
        (("rail", "coping", "pipe", "metal"), "METAL_RAIL"),
        (("asphalt", "road", "tarmac"), "ASPHALT"),
        (("tile", "marble", "slate"), "TILE"),
        (("concrete", "cement", "brick", "stone"), "CONCRETE"),
    )
    for markers, preset in rules:
        if any(marker in name for marker in markers):
            return preset
    return "CONCRETE"


def _is_generated_material_map(
    image: bpy.types.Image | None,
    kind: str | None = None,
) -> bool:
    if image is None:
        return False
    generated_kind = str(image.get("ow_generated_map", "")).upper()
    return bool(generated_kind) and (
        kind is None or generated_kind == kind.upper()
    )


def _generated_image(
    material: bpy.types.Material,
    kind: str,
    pixels: list[float],
    size: int,
    *,
    non_color: bool,
) -> bpy.types.Image:
    safe_name = re.sub(r"[^A-Za-z0-9_.-]+", "_", material.name)[:48]
    name = f"OW_AUTO_{kind}_{safe_name}"
    image = bpy.data.images.get(name)
    if image is None:
        image = bpy.data.images.new(name, width=size, height=size, alpha=True)
    elif tuple(image.size) != (size, size):
        image.scale(size, size)
    image["ow_generated_map"] = kind
    image["ow_generated_for"] = material.name
    # Material image slots are stored as names in custom properties, which do
    # not count as Blender datablock users. Keep generated maps alive even
    # when no shader node references them directly.
    image.use_fake_user = True
    if non_color:
        try:
            image.colorspace_settings.name = "Non-Color"
        except TypeError:
            pass
    image.pixels.foreach_set(pixels)
    image.update()
    # Generated-image pixel buffers are transient unless packed. Without this
    # step Blender reloads the datablock at its default black generated colour,
    # causing the exporter to reject a normal or ORM map after reopening the
    # .blend.
    image.pack()
    return image


def _generate_material_maps(
    material: bpy.types.Material,
    *,
    roughness: float,
    metallic: float,
    emissive_color: tuple[float, float, float],
    emissive_strength: float,
    normal_source: bpy.types.Image | None,
    orm_source: bpy.types.Image | None,
    emissive_source: bpy.types.Image | None,
) -> tuple[
    bpy.types.Image | None,
    bpy.types.Image | None,
    bpy.types.Image | None,
]:
    # A missing normal map means the Principled shader's geometric normal.
    # Older builds invented a 32x32 micro-normal for every scalar-only
    # material. Besides changing Blender-authored presentation, that forced
    # the runtime down its derivative/TBN reconstruction path for every
    # visible pixel. Preserve only a normal map that the author supplied.
    normal_image = (
        None
        if _is_generated_material_map(normal_source, "NORMAL")
        else normal_source
    )

    orm_image = (
        None
        if _is_generated_material_map(orm_source, "ORM")
        else orm_source
    )
    # Roughness already has a scalar field in the SKATE material record and
    # missing ORM naturally means AO=1 and metallic=0 in the renderer. Only
    # synthesize ORM when a scalar metallic value cannot otherwise be carried
    # by the current package format.
    if orm_image is None and metallic > 1.0e-4:
        orm_image = _generated_image(
            material,
            "ORM",
            [1.0, roughness, metallic, 1.0] * 16,
            4,
            non_color=True,
        )

    emissive_image = (
        None
        if _is_generated_material_map(emissive_source, "EMISSIVE")
        else emissive_source
    )
    if emissive_image is None and emissive_strength > 0.0:
        maximum = max(1.0, *emissive_color)
        color = tuple(max(0.0, value / maximum) for value in emissive_color)
        emissive_image = _generated_image(
            material,
            "EMISSIVE",
            [*color, 1.0] * 16,
            4,
            non_color=False,
        )
    return normal_image, orm_image, emissive_image


def _auto_configure_material(
    material: bpy.types.Material,
    *,
    force: bool = False,
    generate_maps: bool = False,
) -> bool:
    global _SYNCING
    authored_keys = (
        "ow_albedo_image",
        "ow_secondary_albedo_image",
        "ow_blend_mask_image",
        "ow_lightmap_image",
        "ow_normal_image",
        "ow_orm_image",
        "ow_emissive_image",
        "ow_audio_surface",
        "ow_physics_surface",
    )
    preserve_authored = (
        not force
        and any(key in material for key in authored_keys)
        and not bool(material.get("ow_auto_imported", False))
    )

    # Older importer builds invented transparency whenever an albedo happened
    # to contain non-opaque alpha. That alpha often stores tint, roughness, or
    # layer masks rather than Blender surface opacity. Remove those exact
    # generated links and let the authored shader graph be authoritative.
    legacy_alpha_removed = _remove_legacy_inferred_alpha(material.node_tree)
    shader, shader_group_stack = _principled_context(material)
    is_glass = False
    color_socket_name = "Base Color"
    if shader is None:
        glass_shader, glass_stack = _glass_context(material)
        if glass_shader is not None:
            shader = glass_shader
            shader_group_stack = glass_stack
            is_glass = True
            color_socket_name = "Color"

    if is_glass:
        # SK8 does not reproduce Blender refraction, but a physical Glass BSDF
        # is unambiguously transparent. Convert it deterministically to a
        # compact neutral blend material instead of exporting the missing-map
        # purple fallback. Use the connected glass colour graph only to choose
        # the tint; opacity is a stable basic-glass approximation.
        glass_color_socket = shader.inputs.get(color_socket_name)
        if glass_color_socket is not None and not glass_color_socket.links:
            glass_tint = tuple(
                max(0.0, min(1.0, float(value)))
                for value in glass_color_socket.default_value[:3]
            )
        else:
            # A linked physical-glass colour graph commonly contains custom
            # engine parameters, cubemaps, or object tints that have no SK8
            # equivalent. Sampling one arbitrary upstream image can turn the
            # whole window magenta. Use a stable neutral window tint for the
            # explicitly accepted basic-glass fallback.
            glass_tint = (0.72, 0.82, 0.90)
        direct_base_image = _generated_image(
            material,
            "GLASS",
            [*glass_tint, 0.32] * 16,
            4,
            non_color=False,
        )
        mix_primary = None
        mix_secondary = None
        mix_mask = None
        mix_factor = 0.0
        mix_mask_channel = 0
        requires_base_bake = False
    else:
        direct_base_image = _direct_input_image(
            shader, color_socket_name, shader_group_stack
        )
        (
            mix_primary,
            mix_secondary,
            mix_mask,
            mix_factor,
            mix_mask_channel,
        ) = _ordinary_mix_layers(shader, shader_group_stack)
        base_socket = (
            shader.inputs.get(color_socket_name)
            if shader is not None
            else None
        )
        requires_base_bake = bool(
            base_socket is not None
            and base_socket.links
            and direct_base_image is None
            and mix_primary is None
        )
    base_socket = (
        shader.inputs.get(color_socket_name)
        if shader is not None
        else None
    )
    baked_base_image = bpy.data.images.get(
        str(material.get("ow_baked_base_color_image", ""))
    )
    base_image = (
        baked_base_image
        if requires_base_bake and baked_base_image is not None
        else direct_base_image
    )
    if base_image is None:
        # This image is only a visible emergency fallback until the complex
        # graph is baked. It must not make the graph look representable.
        base_image = _socket_image(
            shader, color_socket_name, shader_group_stack
        )
    if mix_primary is not None and mix_secondary is not None:
        base_image = mix_primary
    normal_source = _effective_input_source(
        shader, "Normal", shader_group_stack
    )
    normal_image = None
    if normal_source is not None:
        normal_node, _normal_output, normal_stack = normal_source
        if normal_node.type == "NORMAL_MAP":
            normal_image = _socket_image(
                normal_node, "Color", normal_stack
            )
        elif normal_node.type == "TEX_IMAGE":
            # A directly connected image is a conventional tangent normal.
            normal_image = normal_node.image
    # Emission routed through a complex imported shader group cannot be
    # represented by a single source image without evaluating its masks.
    # Keep direct Principled emission support, but do not mistake the group's
    # ordinary albedo input for self-illumination.
    emissive_image = _socket_image(shader, "Emission Color")
    orm_image = None
    lightmap_image = None
    if preserve_authored:
        base_image = (
            bpy.data.images.get(str(material.get("ow_albedo_image", "")))
            or base_image
        )
        lightmap_image = bpy.data.images.get(
            str(material.get("ow_lightmap_image", ""))
        )
        normal_image = (
            bpy.data.images.get(str(material.get("ow_normal_image", "")))
            or normal_image
        )
        orm_image = (
            bpy.data.images.get(str(material.get("ow_orm_image", "")))
            or orm_image
        )
        emissive_image = (
            bpy.data.images.get(str(material.get("ow_emissive_image", "")))
            or emissive_image
        )
    display_color = _socket_color(
        shader, color_socket_name, tuple(material.diffuse_color[:3])
    )
    # Blender ignores a socket's default colour while an albedo is linked.
    # Exporting the usual grey default as an extra tint would darken the
    # texture a second time in SK8.
    if base_image is not None and base_socket is not None and base_socket.links:
        display_color = (1.0, 1.0, 1.0)
    if is_glass:
        display_color = (1.0, 1.0, 1.0)
    roughness = max(
        0.0, min(1.0, _socket_float(shader, "Roughness", 0.78))
    )
    metallic = max(
        0.0, min(1.0, _socket_float(shader, "Metallic", 0.0))
    )
    emissive_color = _socket_color(
        shader, "Emission Color", (1.0, 1.0, 1.0)
    )
    emission_socket = (
        shader.inputs.get("Emission Color")
        if shader is not None
        else None
    )
    # A linked socket is not by itself exportable emission. Imported node
    # graphs often route an unsupported procedural/mask chain into the
    # Principled input while leaving its fallback colour black. Only a
    # deterministically resolved image or a non-black scalar colour can
    # contribute to the SKATE material.
    emission_linked = _socket_has_effective_input(
        shader, "Emission Color", shader_group_stack
    )
    emission_authored = bool(
        emissive_image is not None
        or (not emission_linked and max(emissive_color) > 1.0e-6)
    )
    # Blender 4/5 Principled materials default to black Emission Color but an
    # Emission Strength of 1. Black times one is still non-emissive. Copying
    # the scalar alone made every automatically adopted material self-lit.
    emissive = (
        max(0.0, _socket_float(shader, "Emission Strength", 0.0))
        if emission_authored
        else 0.0
    )
    if generate_maps:
        normal_image, orm_image, emissive_image = _generate_material_maps(
            material,
            roughness=roughness,
            metallic=metallic,
            emissive_color=emissive_color,
            emissive_strength=emissive,
            normal_source=normal_image,
            orm_source=orm_image,
            emissive_source=emissive_image,
        )

    alpha_socket = shader.inputs.get("Alpha") if shader is not None else None
    alpha_image = _socket_image(shader, "Alpha", shader_group_stack)
    alpha_value = (
        float(alpha_socket.default_value) if alpha_socket is not None else 1.0
    )
    inferred_alpha, inferred_blend = _image_alpha_usage(alpha_image)
    if is_glass:
        alpha_mode = 2
    elif alpha_image is not None and inferred_alpha:
        alpha_mode = 2 if inferred_blend else 1
    elif alpha_value < 0.999:
        alpha_mode = 2
    else:
        alpha_mode = 0

    changed = bool(legacy_alpha_removed)
    for property_name, image in (
        ("ow_albedo_image", base_image),
        ("ow_secondary_albedo_image", mix_secondary),
        ("ow_blend_mask_image", mix_mask),
        ("ow_lightmap_image", lightmap_image),
        ("ow_normal_image", normal_image),
        ("ow_orm_image", orm_image),
        ("ow_emissive_image", emissive_image),
    ):
        image_name = image.name if image is not None else ""
        existing_name = str(material.get(property_name, ""))
        existing_image = bpy.data.images.get(existing_name)
        if (
            preserve_authored
            and existing_name
            and not _is_generated_material_map(existing_image)
        ):
            continue
        if str(material.get(property_name, "")) != image_name:
            material[property_name] = image_name
            changed = True
    compatibility_values = {
        "ow_requires_base_color_bake": (
            requires_base_bake
            and not (
                bool(
                    material.get(
                        "ow_base_color_bake_failed", False
                    )
                )
                and int(
                    material.get(
                        "ow_base_color_bake_failed_schema", 0
                    )
                )
                == BASE_COLOR_BAKE_SCHEMA
            )
            and (
                not preserve_authored
                or bool(
                    material.get(
                        "ow_baked_base_color_image", ""
                    )
                )
                or bool(
                    material.get(
                        "ow_base_color_bake_failed", False
                    )
                )
            )
        ),
        "ow_blend_factor": mix_factor,
        "ow_blend_mask_channel": mix_mask_channel,
        "ow_albedo_address_mode": _image_address_mode(
            material, base_image
        ),
        "ow_secondary_address_mode": _image_address_mode(
            material, mix_secondary
        ),
        "ow_blend_mask_address_mode": _image_address_mode(
            material, mix_mask
        ),
        "ow_uv_map": _image_uv_map_name(material, base_image),
        "ow_secondary_uv_map": _image_uv_map_name(
            material, mix_secondary
        ),
        "ow_blend_mask_uv_map": _image_uv_map_name(
            material, mix_mask
        ),
        "ow_uv_transform": _image_uv_transform(
            material, base_image
        ),
        "ow_secondary_uv_transform": _image_uv_transform(
            material, mix_secondary
        ),
        "ow_blend_mask_uv_transform": _image_uv_transform(
            material, mix_mask
        ),
        "ow_cull_mode": (
            2 if bool(material.use_backface_culling) else 1
        ),
        "ow_uses_object_color": (
            False if is_glass else _material_uses_object_color(material)
        ),
    }
    for property_name, value in compatibility_values.items():
        existing = material.get(property_name)
        if (
            hasattr(value, "__len__")
            and not isinstance(value, str)
            and existing is not None
        ):
            differs = tuple(existing) != tuple(value)
        else:
            differs = existing != value
        if differs:
            material[property_name] = value
            changed = True
    if not preserve_authored:
        material["ow_display_color"] = display_color
        material["ow_flags"] = 1
        # Scene adoption only translates Blender-authored PBR data. Gameplay
        # semantics are explicit SKATE choices made by the author or map
        # agent, never inferred from material names.
        material["ow_audio_surface"] = 0
        material["ow_physics_surface"] = 0
        material["ow_surface_pattern"] = 0
        material["ow_alpha_mode"] = alpha_mode
        material["ow_alpha_cutoff"] = 0.5
        material["ow_roughness"] = roughness
        material["ow_metallic"] = metallic
        material["ow_friction"] = 0.82
        material["ow_restitution"] = 0.0
        material["ow_emissive"] = emissive
        material["ow_baked_strength"] = 1.0 if lightmap_image else 0.0
        material["ow_collision_enabled"] = True
        material["ow_presentation_type"] = "STANDARD"
        changed = True
    elif not bool(material.get("ow_alpha_mode_authored", False)):
        # Collision/contact choices may be agent-authored while presentation
        # opacity is still importer-owned. Keep those choices and heal stale
        # alpha metadata from the current Blender shader.
        if int(material.get("ow_alpha_mode", 0)) != alpha_mode:
            material["ow_alpha_mode"] = alpha_mode
            changed = True
    if is_glass:
        # This is part of the technical Glass-BSDF translation, not a gameplay
        # material choice. Clear stale importer/object tint even when contact
        # semantics were authored later, otherwise the neutral fallback can
        # still be multiplied back to the original magenta/green placeholder.
        if tuple(material.get("ow_display_color", ())) != (1.0, 1.0, 1.0):
            material["ow_display_color"] = (1.0, 1.0, 1.0)
            changed = True
        if int(material.get("ow_alpha_mode", 0)) != 2:
            material["ow_alpha_mode"] = 2
            changed = True
    if not preserve_authored:
        material["ow_auto_imported"] = True
    was_syncing = _SYNCING
    _SYNCING = True
    try:
        _hydrate_material(material)
    finally:
        _SYNCING = was_syncing
    return changed


def _material_uses_object_color(material: bpy.types.Material) -> bool:
    return any(
        node.type == "OBJECT_INFO"
        and any(
            output.name == "Color" and output.links
            for output in node.outputs
        )
        for node in _nodes_recursive(material.node_tree)
    )


def _base_color_bake_size(
    material: bpy.types.Material,
) -> tuple[int, int]:
    shader, stack = _principled_context(material)
    if shader is None:
        return 512, 512
    socket = shader.inputs.get("Base Color")
    images = _node_images_upstream(socket, stack) if socket else []
    widths = [int(image.size[0]) for image in images if image.size[0]]
    heights = [int(image.size[1]) for image in images if image.size[1]]
    return (
        max(64, min(1024, max(widths, default=512))),
        max(64, min(1024, max(heights, default=512))),
    )


def _base_color_bake_image(
    material: bpy.types.Material,
) -> bpy.types.Image:
    digest = hashlib.sha1(
        material.name_full.encode("utf-8")
    ).hexdigest()[:10]
    clean_name = re.sub(r"[^A-Za-z0-9_.-]+", "_", material.name)[:48]
    name = f"OW_BAKED_BASE_{clean_name}_{digest}"
    width, height = _base_color_bake_size(material)
    image = bpy.data.images.get(name)
    if image is not None and tuple(image.size) != (width, height):
        bpy.data.images.remove(image)
        image = None
    if image is None:
        image = bpy.data.images.new(
            name,
            width=width,
            height=height,
            alpha=True,
            float_buffer=False,
        )
    image.use_fake_user = True
    image["ow_generated_base_color_bake"] = True
    return image


def _bake_full_uv_domain_for_material(
    scene: bpy.types.Scene,
    material: bpy.types.Material,
    image: bpy.types.Image,
) -> None:
    mesh = bpy.data.meshes.new("OW_TEMP_BASE_COLOR_BAKE_MESH")
    mesh.from_pydata(
        (
            (0.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
            (1.0, 1.0, 0.0),
            (0.0, 1.0, 0.0),
        ),
        (),
        ((0, 1, 2, 3),),
    )
    mesh.update()
    mesh.materials.append(material)
    uv_names = {"UVMap"}
    uv_names.update(
        str(node.uv_map)
        for node in _nodes_recursive(material.node_tree)
        if node.type == "UVMAP" and str(node.uv_map)
    )
    for uv_name in sorted(uv_names, key=str.casefold):
        uv = mesh.uv_layers.new(name=uv_name)
        for loop in mesh.loops:
            point = mesh.vertices[loop.vertex_index].co
            uv.data[loop.index].uv = (point.x, point.y)
        uv.active_render = uv_name == "UVMap"
    mesh.uv_layers.active = mesh.uv_layers["UVMap"]
    # A Vertex Color node returns black when its named layer is absent. The
    # full-domain bake mesh is synthetic, so provide neutral white layers for
    # every explicitly referenced colour attribute. This keeps the flattened
    # texture neutral; object-level colour remains a lightweight export
    # variant, while imported meshes do not turn black merely because the
    # temporary bake plane lacked their TINT layer.
    color_attribute_names = {
        str(node.layer_name)
        for node in _nodes_recursive(material.node_tree)
        if node.type == "VERTEX_COLOR" and str(node.layer_name)
    }
    for attribute_name in sorted(color_attribute_names, key=str.casefold):
        attribute = mesh.color_attributes.new(
            name=attribute_name,
            type="FLOAT_COLOR",
            domain="CORNER",
        )
        for item in attribute.data:
            item.color = (1.0, 1.0, 1.0, 1.0)

    temporary = bpy.data.objects.new(
        "OW_TEMP_BASE_COLOR_BAKE", mesh
    )
    temporary.name = "OW_TEMP_BASE_COLOR_BAKE"
    scene.collection.objects.link(temporary)
    target = None
    emission_tree = None
    emission_color = None
    emission_strength = None
    emission_color_default = None
    emission_strength_default = None
    emission_color_links = []
    emission_strength_links = []
    alpha_bake_image = None
    try:
        # Evaluate every point in the complete texture domain. Baking one
        # source object's UV islands leaves untouched black texels that other
        # objects sharing the material can sample.
        temporary.color = (1.0, 1.0, 1.0, 1.0)
        temporary.hide_viewport = False
        temporary.hide_render = False
        temporary.hide_select = False
        for visibility_name in (
            "visible_camera",
            "visible_diffuse",
            "visible_glossy",
            "visible_shadow",
            "visible_transmission",
            "visible_volume_scatter",
        ):
            if hasattr(temporary, visibility_name):
                setattr(temporary, visibility_name, True)
        target = material.node_tree.nodes.new("ShaderNodeTexImage")
        target.name = "OW_TEMP_BASE_COLOR_BAKE_TARGET"
        target.image = image
        material.node_tree.nodes.active = target
        # Cycles DIFFUSE/COLOR bakes the post-BSDF diffuse lobe. Metallic
        # surfaces therefore become black even though their Blender Base
        # Color is valid. Route the resolved Base Color into the Principled
        # emission input temporarily and bake EMIT instead. This captures the
        # colour graph itself, independent of metallic, roughness, lighting,
        # or the viewing environment.
        shader, _stack = _principled_context(material)
        if shader is None:
            raise RuntimeError("material has no Principled Base Color")
        base_color = shader.inputs.get("Base Color")
        emission_color = (
            shader.inputs.get("Emission Color")
            or shader.inputs.get("Emission")
        )
        emission_strength = shader.inputs.get("Emission Strength")
        if (
            base_color is None
            or emission_color is None
            or emission_strength is None
        ):
            raise RuntimeError(
                "material Principled shader has no bakeable emission inputs"
            )
        emission_tree = shader.id_data
        emission_color_default = tuple(emission_color.default_value)
        emission_strength_default = float(emission_strength.default_value)
        emission_color_links = [
            (link.from_socket, link.to_socket)
            for link in emission_color.links
        ]
        emission_strength_links = [
            (link.from_socket, link.to_socket)
            for link in emission_strength.links
        ]
        for link in list(emission_color.links):
            emission_tree.links.remove(link)
        for link in list(emission_strength.links):
            emission_tree.links.remove(link)
        if base_color.links:
            emission_tree.links.new(
                base_color.links[0].from_socket, emission_color
            )
        else:
            emission_color.default_value = base_color.default_value
        emission_strength.default_value = 1.0
        bpy.ops.object.select_all(action="DESELECT")
        temporary.hide_render = False
        temporary.hide_set(False)
        temporary.select_set(True)
        bpy.context.view_layer.objects.active = temporary
        bpy.ops.object.bake(
            type="EMIT",
            use_clear=True,
            margin=8,
        )
        # An EMIT bake writes opaque alpha even when the evaluated Principled
        # Alpha input contains the cutout/blend mask. Bake that scalar graph
        # separately, then put it back into the generated albedo. The runtime
        # can now discard or blend the same pixels Blender does instead of
        # showing their hidden black RGB values as rectangular decals.
        alpha_socket = shader.inputs.get("Alpha")
        if (
            int(material.get("ow_alpha_mode", 0)) != 0
            and alpha_socket is not None
        ):
            alpha_bake_image = bpy.data.images.new(
                "OW_TEMP_BASE_ALPHA_BAKE",
                width=int(image.size[0]),
                height=int(image.size[1]),
                alpha=True,
                float_buffer=False,
            )
            try:
                alpha_bake_image.colorspace_settings.name = "Non-Color"
            except (AttributeError, TypeError):
                pass
            target.image = alpha_bake_image
            material.node_tree.nodes.active = target
            for link in list(emission_color.links):
                emission_tree.links.remove(link)
            if alpha_socket.links:
                emission_tree.links.new(
                    alpha_socket.links[0].from_socket,
                    emission_color,
                )
            else:
                alpha_value = max(
                    0.0,
                    min(1.0, float(alpha_socket.default_value)),
                )
                emission_color.default_value = (
                    alpha_value,
                    alpha_value,
                    alpha_value,
                    1.0,
                )
            bpy.ops.object.bake(
                type="EMIT",
                use_clear=True,
                margin=8,
            )
            base_pixels = array("f", [0.0]) * len(image.pixels)
            alpha_pixels = array("f", [0.0]) * len(
                alpha_bake_image.pixels
            )
            image.pixels.foreach_get(base_pixels)
            alpha_bake_image.pixels.foreach_get(alpha_pixels)
            for index in range(0, len(base_pixels), 4):
                base_pixels[index + 3] = max(
                    0.0,
                    min(1.0, alpha_pixels[index]),
                )
            image.pixels.foreach_set(base_pixels)
            image.update()
            image["ow_bake_alpha_preserved"] = True
            target.image = image
        else:
            image["ow_bake_alpha_preserved"] = False

        pixels = list(image.pixels[:])
        if not any(
            pixels[index] > 1.0e-6
            for index in range(3, len(pixels), 4)
        ):
            raise RuntimeError(
                "Blender bake produced no covered texels"
            )
        image["ow_bake_has_coverage"] = True
        image["ow_base_color_bake_schema"] = BASE_COLOR_BAKE_SCHEMA
        image["skate3_allow_blank_rgb"] = not any(
            pixels[index]
            or pixels[index + 1]
            or pixels[index + 2]
            for index in range(0, len(pixels), 4)
        )
        image.pack()
        image.use_fake_user = True
    finally:
        if emission_tree is not None:
            if emission_color is not None:
                for link in list(emission_color.links):
                    emission_tree.links.remove(link)
                if emission_color_default is not None:
                    emission_color.default_value = emission_color_default
                for from_socket, to_socket in emission_color_links:
                    emission_tree.links.new(from_socket, to_socket)
            if emission_strength is not None:
                for link in list(emission_strength.links):
                    emission_tree.links.remove(link)
                if emission_strength_default is not None:
                    emission_strength.default_value = (
                        emission_strength_default
                    )
                for from_socket, to_socket in emission_strength_links:
                    emission_tree.links.new(from_socket, to_socket)
        if target is not None and target.id_data is not None:
            material.node_tree.nodes.remove(target)
        if alpha_bake_image is not None:
            bpy.data.images.remove(alpha_bake_image)
        bpy.data.objects.remove(temporary, do_unlink=True)
        if mesh.users == 0:
            bpy.data.meshes.remove(mesh)


def _bake_complex_base_colors(
    context,
    visual_objects: list[bpy.types.Object],
) -> tuple[int, list[str]]:
    by_material: dict[int, tuple[bpy.types.Material, list[bpy.types.Object]]] = {}
    for obj in visual_objects:
        for slot in obj.material_slots:
            material = slot.material
            if material is None or not bool(
                material.get("ow_requires_base_color_bake", False)
            ):
                continue
            identity = material.as_pointer()
            if identity not in by_material:
                by_material[identity] = (material, [])
            by_material[identity][1].append(obj)
    pending = []
    for material, objects in by_material.values():
        existing = bpy.data.images.get(
            str(material.get("ow_baked_base_color_image", ""))
        )
        pixels: list[float] = []
        schema_matches = bool(
            existing is not None
            and int(
                existing.get("ow_base_color_bake_schema", 0)
            )
            == BASE_COLOR_BAKE_SCHEMA
        )
        covered = bool(
            schema_matches
            and existing.get("ow_bake_has_coverage", False)
        )
        if schema_matches and not covered:
            pixels = list(existing.pixels[:])
            covered = any(
                pixels[index] > 1.0e-6
                for index in range(3, len(pixels), 4)
            )
        if covered:
            existing["ow_bake_has_coverage"] = True
            if pixels:
                existing["skate3_allow_blank_rgb"] = not any(
                    pixels[index]
                    or pixels[index + 1]
                    or pixels[index + 2]
                    for index in range(0, len(pixels), 4)
                )
        else:
            pending.append((material, objects))
    if not pending:
        return 0, []

    scene = context.scene
    previous_engine = scene.render.engine
    previous_samples = int(scene.cycles.samples)
    previous_denoising = bool(scene.cycles.use_denoising)
    previous_margin = int(scene.render.bake.margin)
    previous_active = context.view_layer.objects.active
    previous_selected = list(context.selected_objects)
    baked = 0
    warnings: list[str] = []
    try:
        scene.render.engine = "CYCLES"
        scene.cycles.samples = 1
        scene.cycles.use_denoising = False
        scene.render.bake.margin = 8
        for index, (material, objects) in enumerate(pending, start=1):
            image = _base_color_bake_image(material)
            try:
                _bake_full_uv_domain_for_material(
                    scene, material, image
                )
            except Exception as error:
                shader, stack = _principled_context(material)
                fallback = _socket_image(
                    shader, "Base Color", stack
                )
                material["ow_baked_base_color_image"] = ""
                material["ow_albedo_image"] = (
                    fallback.name if fallback is not None else ""
                )
                material["ow_secondary_albedo_image"] = ""
                material["ow_blend_mask_image"] = ""
                material["ow_blend_factor"] = 0.0
                material["ow_uv_map"] = _image_uv_map_name(
                    material, fallback
                )
                material["ow_uv_transform"] = _image_uv_transform(
                    material, fallback
                )
                material["ow_albedo_address_mode"] = _image_address_mode(
                    material, fallback
                )
                material["ow_base_color_bake_failed"] = True
                material["ow_base_color_bake_failed_schema"] = (
                    BASE_COLOR_BAKE_SCHEMA
                )
                material["ow_requires_base_color_bake"] = False
                warnings.append(
                    f"{material.name}: Blender could not bake its complex "
                    f"Base Color across the full UV domain ({error}); "
                    "using the best direct image "
                    "fallback."
                )
                continue
            material["ow_baked_base_color_image"] = image.name
            material["ow_base_color_bake_failed"] = False
            material["ow_base_color_bake_failed_schema"] = (
                BASE_COLOR_BAKE_SCHEMA
            )
            material["ow_albedo_image"] = image.name
            material["ow_secondary_albedo_image"] = ""
            material["ow_blend_mask_image"] = ""
            material["ow_blend_factor"] = 0.0
            material["ow_uv_map"] = "UVMap"
            material["ow_uv_transform"] = (1.0, 1.0, 0.0, 0.0, 0.0)
            material["ow_albedo_address_mode"] = 0
            material["ow_display_color"] = (1.0, 1.0, 1.0)
            material["ow_uses_object_color"] = _material_uses_object_color(
                material
            )
            baked += 1
            if index == len(pending) or index % 10 == 0:
                print(
                    f"SKATE material bake: {index}/{len(pending)} "
                    f"{material.name}",
                    flush=True,
                )
    finally:
        scene.render.engine = previous_engine
        scene.cycles.samples = previous_samples
        scene.cycles.use_denoising = previous_denoising
        scene.render.bake.margin = previous_margin
        bpy.ops.object.select_all(action="DESELECT")
        for obj in previous_selected:
            if obj.name in context.view_layer.objects:
                obj.select_set(True)
        if (
            previous_active is not None
            and previous_active.name in context.view_layer.objects
        ):
            context.view_layer.objects.active = previous_active
    return baked, warnings


def _new_uv_layer(
    mesh: bpy.types.Mesh, name: str
) -> bpy.types.MeshUVLoopLayer:
    # Blender 5.1 can successfully add a UV layer while returning None from
    # MeshUVLoopLayers.new() for some imported meshes. Resolve the created
    # layer from the collection instead of trusting that return value.
    mesh.uv_layers.new(name=name)
    layer = mesh.uv_layers.get(name)
    if layer is None:
        raise RuntimeError(
            f"{mesh.name}: Blender could not create the {name!r} UV layer"
        )
    # Some imported Blender 5.1 meshes expose unset custom-data storage as a
    # large negative float sentinel instead of zero. Explicit initialization
    # keeps newly generated UVs deterministic and prevents those sentinels
    # from reaching texture sampling or the package analyzer.
    for item in layer.data:
        item.uv = (0.0, 0.0)
    return layer


def _sanitize_uv_layer(layer: bpy.types.MeshUVLoopLayer) -> int:
    repaired = 0
    for item in layer.data:
        u = float(item.uv[0])
        v = float(item.uv[1])
        clean_u = u if math.isfinite(u) and abs(u) <= 1.0e6 else 0.0
        clean_v = v if math.isfinite(v) and abs(v) <= 1.0e6 else 0.0
        if clean_u != u or clean_v != v:
            item.uv = (clean_u, clean_v)
            repaired += int(clean_u != u) + int(clean_v != v)
    return repaired


def _project_box_uv(
    mesh: bpy.types.Mesh,
    layer: bpy.types.MeshUVLoopLayer,
) -> None:
    """Create deterministic, normalized box UVs without context operators."""
    if not mesh.vertices:
        return
    minimum = [
        min(float(vertex.co[axis]) for vertex in mesh.vertices)
        for axis in range(3)
    ]
    maximum = [
        max(float(vertex.co[axis]) for vertex in mesh.vertices)
        for axis in range(3)
    ]
    extent = [
        max(maximum[axis] - minimum[axis], 1.0e-6)
        for axis in range(3)
    ]
    for polygon in mesh.polygons:
        dominant = max(
            range(3), key=lambda axis: abs(float(polygon.normal[axis]))
        )
        uv_axes = (
            (0, 1) if dominant == 2
            else ((0, 2) if dominant == 1 else (1, 2))
        )
        for loop_index in polygon.loop_indices:
            vertex = mesh.vertices[
                mesh.loops[loop_index].vertex_index
            ].co
            layer.data[loop_index].uv = (
                (float(vertex[uv_axes[0]]) - minimum[uv_axes[0]])
                / extent[uv_axes[0]],
                (float(vertex[uv_axes[1]]) - minimum[uv_axes[1]])
                / extent[uv_axes[1]],
            )
    mesh["ow_generated_box_uv"] = True


def _ensure_auto_uv_layers(
    mesh: bpy.types.Mesh,
) -> tuple[int, bool, int]:
    created = 0
    repaired = 0
    source = mesh.uv_layers.get("UVMap") or mesh.uv_layers.active
    missing_source = source is None
    if source is None:
        source = _new_uv_layer(mesh, "UVMap")
        _project_box_uv(mesh, source)
        created += 1
    else:
        repaired += _sanitize_uv_layer(source)

    if source.name != "UVMap" and mesh.uv_layers.get("UVMap") is None:
        uv_map = _new_uv_layer(mesh, "UVMap")
        for destination, original in zip(
            uv_map.data, source.data, strict=False
        ):
            destination.uv = original.uv
        source = uv_map
        created += 1
    else:
        source = mesh.uv_layers.get("UVMap") or source

    if mesh.uv_layers.get("Lightmap") is None:
        if len(mesh.uv_layers) >= 8:
            lightmap = mesh.uv_layers[-1]
            if lightmap != source:
                lightmap.name = "Lightmap"
        else:
            lightmap = _new_uv_layer(mesh, "Lightmap")
            for destination, original in zip(
                lightmap.data, source.data, strict=False
            ):
                destination.uv = original.uv
            created += 1
    repaired += _sanitize_uv_layer(mesh.uv_layers.get("Lightmap"))
    return created, missing_source, repaired


def _default_material() -> bpy.types.Material:
    material = bpy.data.materials.get("SKATE_Auto_Concrete")
    if material is None:
        material = bpy.data.materials.new("SKATE_Auto_Concrete")
        material.diffuse_color = (0.45, 0.45, 0.47, 1.0)
        _auto_configure_material(material)
    return material


def _first_collision_material(
    obj: bpy.types.Object,
    used_materials: set[str],
    fallback: bpy.types.Material,
) -> bpy.types.Material:
    for slot in obj.material_slots:
        material = slot.material
        if (
            material is not None
            and material.name in used_materials
            and bool(material.get("ow_collision_enabled", True))
        ):
            return material
    return fallback


def _auto_prepare_scene(
    context, *, include_existing_roles: bool = False
) -> tuple[str, list[str]]:
    global _SYNCING
    _ALPHA_SCAN_CACHE.clear()
    scene = context.scene
    for name in _group_collections():
        _ensure_collection(scene, name)
    migrated = _migrate_legacy_groups(scene)

    scene_meshes = [obj for obj in scene.objects if obj.type == "MESH"]
    for obj in scene_meshes:
        if obj.name == exporter.SPAWN_OBJECT or _is_auto_helper_object(obj):
            continue
        memberships = _object_groups(obj)
        if memberships:
            continue
        if not _auto_visual_candidate(obj):
            continue
        # A conservative, visible baseline is predictable and safe: ordinary
        # imported meshes collide until an explicit human/agent role says
        # otherwise. Names never decide collision or export roles.
        _move_to_group(
            scene, obj, exporter.PRESENTATION_COLLISION_COLLECTION
        )

    active_visuals = exporter._objects_from_collections(
        exporter.PRESENTATION_COLLISION_COLLECTION,
        exporter.NO_COLLISION_COLLECTION,
    )
    active_visuals = [
        obj
        for obj in active_visuals
        if obj.type == "MESH"
        and bool(obj.get("ow_export_visual", True))
        and not _is_auto_helper_object(obj)
    ]
    active_collision = exporter._objects_from_collections(
        exporter.PRESENTATION_COLLISION_COLLECTION,
        exporter.NO_PRESENTATION_COLLECTION,
    )
    active_collision = [
        obj
        for obj in active_collision
        if obj.type == "MESH" and not _is_auto_helper_object(obj)
    ]
    default_material = _default_material()
    configured_materials = 0
    created_uv_layers = 0
    repaired_uv_coordinates = 0
    missing_source_uvs: list[str] = []
    visited_meshes: set[int] = set()
    used_materials: set[str] = set()
    for obj in active_visuals:
        if not obj.data.materials:
            obj.data.materials.append(default_material)
        mesh_identity = obj.data.as_pointer()
        if mesh_identity not in visited_meshes:
            created, missing_source, repaired = _ensure_auto_uv_layers(
                obj.data
            )
            created_uv_layers += created
            repaired_uv_coordinates += repaired
            if missing_source:
                missing_source_uvs.append(obj.name)
            visited_meshes.add(mesh_identity)
        for slot in obj.material_slots:
            material = slot.material
            if material is None:
                continue
            memberships = _object_groups(obj)
            if len(memberships) == 1:
                material["ow_map_group"] = memberships[0]
            used_materials.add(material.name)
            configured_materials += int(
                _auto_configure_material(material, generate_maps=True)
            )

    baked_materials, bake_warnings = _bake_complex_base_colors(
        context, active_visuals
    )

    fallback = next(
        (
            bpy.data.materials.get(name)
            for name in sorted(used_materials)
            if bpy.data.materials.get(name) is not None
            and bool(
                bpy.data.materials[name].get(
                    "ow_collision_enabled", True
                )
            )
        ),
        default_material,
    )
    for obj in active_collision:
        material = _first_collision_material(obj, used_materials, fallback)
        obj["ow_material"] = material.name
        if "ow_upward_surface" not in obj:
            obj["ow_upward_surface"] = False
        was_syncing = _SYNCING
        _SYNCING = True
        try:
            _hydrate_physics(obj)
        finally:
            _SYNCING = was_syncing

    local_lights = exporter._visible_local_light_objects()

    if scene.owned_world.map_name == "My Skate Map" and bpy.data.filepath:
        scene.owned_world.map_name = Path(bpy.data.filepath).stem
    if (
        scene.owned_world.output_path == "//my_skate_map.skate"
        and bpy.data.filepath
    ):
        scene.owned_world.output_path = str(
            Path(bpy.data.filepath).with_suffix(".skate")
        )

    warnings: list[str] = list(bake_warnings)
    if missing_source_uvs:
        warnings.append(
            f"{len(missing_source_uvs)} mesh(es) had no source UVs; "
            "generated normalized box UVs so textures remain usable. "
            "Author UVs later for exact presentation."
        )
    if repaired_uv_coordinates:
        warnings.append(
            f"Reset {repaired_uv_coordinates} invalid imported UV "
            "coordinate(s) to zero."
        )
    stats = (
        f"Auto-prepared {len(active_visuals)} presentation object(s), "
        f"{len(active_collision)} collision object(s), "
        f"{configured_materials} Blender material(s), "
        f"{baked_materials} complex Base Color bake(s), "
        f"{created_uv_layers} UV layer(s), and "
        f"{len(local_lights)} real Blender local light(s)"
        + (f"; migrated {migrated} legacy assignment(s)." if migrated else ".")
    )
    return stats, warnings


def _validate_scene(
    context,
    *,
    inspect_geometry: bool = True,
) -> tuple[list[str], list[str], str]:
    issues: list[str] = []
    warnings: list[str] = []
    scene = context.scene
    settings = scene.owned_world

    spawn = bpy.data.objects.get(exporter.SPAWN_OBJECT)

    visual_objects = [
        obj
        for obj in exporter._objects_from_collections(
            exporter.PRESENTATION_COLLISION_COLLECTION,
            exporter.NO_COLLISION_COLLECTION,
        )
        if obj.type == "MESH"
        and bool(obj.get("ow_export_visual", True))
        and not _is_auto_helper_object(obj)
    ]
    collision_objects = [
        obj
        for obj in exporter._objects_from_collections(
            exporter.PRESENTATION_COLLISION_COLLECTION,
            exporter.NO_PRESENTATION_COLLECTION,
        )
        if obj.type == "MESH" and not _is_auto_helper_object(obj)
    ]
    grind_objects = [
        obj
        for obj in exporter._objects_from_collections(
            exporter.GRIND_COLLECTION
        )
        if obj.type == "CURVE"
    ]
    npc_path_objects = [
        obj
        for obj in exporter._objects_from_collections(
            exporter.NPC_PATH_COLLECTION
        )
        if obj.type == "CURVE"
    ]
    local_lights = exporter._visible_local_light_objects()
    sun_lights = [
        obj
        for obj in exporter._visible_light_objects()
        if obj.data.type == "SUN"
    ]
    if len(sun_lights) > 1:
        warnings.append(
            f"{len(sun_lights)} Sun lights are enabled; "
            f"{sun_lights[0].name!r} will control the world sun."
        )

    missing_groups = [
        name
        for name in _group_collections()
        if bpy.data.collections.get(name) is None
    ]
    if missing_groups:
        issues.append(
            "Missing map group collection(s): " + ", ".join(missing_groups)
        )
    if not visual_objects:
        issues.append("Groups 1 and 3 contain no presentation meshes.")

    for obj in scene.objects:
        if (
            obj.name != exporter.SPAWN_OBJECT
            and not _is_auto_helper_object(obj)
            and not bool(obj.get("sk8_explicitly_ignored", False))
            and obj.type in {"CURVE", "FONT", "SURFACE", "META"}
            and not _object_groups(obj)
            and not obj.hide_render
        ):
            warnings.append(
                f"{obj.name}: visible {obj.type} object is not exported; "
                "convert it to a mesh in Blender when it belongs in the map."
            )
            continue
        if (
            obj.name == exporter.SPAWN_OBJECT
            or _is_auto_helper_object(obj)
            or bool(obj.get("sk8_explicitly_ignored", False))
            or obj.type not in {"MESH", "CURVE"}
        ):
            continue
        memberships = _object_groups(obj)
        if (
            not memberships
            and obj.type == "MESH"
            and not _auto_visual_candidate(obj)
        ):
            # Imported source scenes commonly retain disabled meshes and
            # secondary LODs. Auto Prepare deliberately leaves them outside
            # the export groups, so validation must ignore them too.
            continue
        if len(memberships) != 1:
            issues.append(
                f"{obj.name}: must belong to exactly one map group "
                f"(currently {len(memberships)})."
            )
            continue
        expected_type = (
            "CURVE"
            if memberships[0]
            in {exporter.GRIND_COLLECTION, exporter.NPC_PATH_COLLECTION}
            else "MESH"
        )
        if obj.type != expected_type:
            issues.append(
                f"{obj.name}: {memberships[0]} requires "
                f"{expected_type} objects."
            )

    used_materials: set[str] = set()
    for obj in visual_objects:
        mesh = obj.data
        if mesh.uv_layers.get("UVMap") is None:
            issues.append(f"{obj.name}: missing UVMap UV layer.")
        if mesh.uv_layers.get("Lightmap") is None:
            issues.append(f"{obj.name}: missing Lightmap UV layer.")
        materials = [
            slot.material for slot in obj.material_slots
            if slot.material is not None
        ]
        if not materials:
            issues.append(f"{obj.name}: no material assigned.")
        used_materials.update(material.name for material in materials)
        for material in materials:
            for property_name in (
                "ow_uv_map",
                "ow_secondary_uv_map",
                "ow_blend_mask_uv_map",
            ):
                uv_name = str(material.get(property_name, ""))
                if uv_name and mesh.uv_layers.get(uv_name) is None:
                    warnings.append(
                        f"{obj.name}: material {material.name!r} requests "
                        f"missing UV layer {uv_name!r}; UVMap will be used."
                    )
        if (
            str(obj.get("ow_physics_type", "STATIC")) == "HINGED_DOOR"
            and "ow_hinge_position" not in obj
        ):
            warnings.append(
                f"{obj.name}: door hinge uses the object origin."
            )

    for material_name in sorted(used_materials, key=str.casefold):
        material = bpy.data.materials.get(material_name)
        if material is not None:
            shader, shader_group_stack = _principled_context(material)
        else:
            shader, shader_group_stack = None, ()
        if shader is None:
            warnings.append(
                f"{material_name}: no supported Principled BSDF reaches the "
                "Material Output; exporting the material's Blender viewport "
                "colour. Connect a Principled shader for exact presentation."
            )
            continue
        base_socket = shader.inputs.get("Base Color")
        base_source = _effective_input_source(
            shader, "Base Color", shader_group_stack
        )
        mix_layers = _ordinary_mix_layers(shader, shader_group_stack)
        if (
            base_socket is not None
            and base_socket.links
            and _socket_image(
                shader, "Base Color", shader_group_stack
            ) is None
            and mix_layers[0] is None
        ):
            warnings.append(
                f"{material.name}: linked Base Color is procedural or "
                "unsupported; exporting its Blender fallback colour. "
                "Bake or assign an Albedo image for exact presentation."
            )
        if (
            base_source is not None
            and base_source[0].type in {"MIX", "MIX_RGB"}
            and mix_layers[0] is None
        ):
            warnings.append(
                f"{material.name}: Base Color uses a Mix setup that cannot "
                "be represented directly. Only a standard two-image MIX "
                "with a scalar or image factor is retained; bake this graph "
                "for exact presentation."
            )
        primary_image, secondary_image, mask_image, _factor, _channel = (
            mix_layers
        )
        if secondary_image is not None and mask_image is not None:
            secondary_uv = _image_uv_map_name(material, secondary_image)
            mask_uv = _image_uv_map_name(material, mask_image)
            secondary_transform = _image_uv_transform(
                material, secondary_image
            )
            mask_transform = _image_uv_transform(material, mask_image)
            if (
                secondary_uv != mask_uv
                or any(
                    abs(left - right) > 1.0e-6
                    for left, right in zip(
                        secondary_transform, mask_transform
                    )
                )
            ):
                warnings.append(
                    f"{material.name}: the secondary colour and its blend "
                    "mask use different UV mapping. SK8 stores one shared "
                    "secondary UV channel, so bake or align those mappings "
                    "for exact presentation."
                )
        if primary_image is None:
            primary_image = _socket_image(
                shader, "Base Color", shader_group_stack
            )
        if primary_image is not None:
            primary_uv = _image_uv_map_name(material, primary_image)
            primary_transform = _image_uv_transform(
                material, primary_image
            )
            for role, image_name in (
                ("Normal", str(material.get("ow_normal_image", ""))),
                ("ORM", str(material.get("ow_orm_image", ""))),
                ("Emissive", str(material.get("ow_emissive_image", ""))),
            ):
                role_image = bpy.data.images.get(image_name)
                if role_image is None:
                    continue
                role_uv = _image_uv_map_name(material, role_image)
                role_transform = _image_uv_transform(material, role_image)
                if (
                    role_uv != primary_uv
                    or any(
                        abs(left - right) > 1.0e-6
                        for left, right in zip(
                            role_transform, primary_transform
                        )
                    )
                ):
                    warnings.append(
                        f"{material.name}: {role} uses UV mapping different "
                        "from Base Color. SK8 shares the base UV for these "
                        "maps, so bake or align that mapping for exact "
                        "presentation."
                    )
        normal_source = _effective_input_source(
            shader, "Normal", shader_group_stack
        )
        if normal_source is not None and normal_source[0].type == "BUMP":
            warnings.append(
                f"{material.name}: Blender Bump height shading cannot be "
                "represented as a tangent normal map and was left off. "
                "Bake it to a Normal Map for exact presentation."
            )
        explicit_orm = bpy.data.images.get(
            str(material.get("ow_orm_image", ""))
        )
        linked_pbr = [
            socket_name
            for socket_name in ("Roughness", "Metallic")
            if _socket_has_effective_input(
                shader, socket_name, shader_group_stack
            )
        ]
        if linked_pbr and explicit_orm is None:
            warnings.append(
                f"{material.name}: linked {'/'.join(linked_pbr)} requires "
                "an explicit packed ORM image for exact channel detail; "
                "exporting the material's scalar roughness/metallic values."
            )

    if not collision_objects:
        issues.append("Groups 1 and 2 contain no collision meshes.")
    for obj in collision_objects:
        material_name = str(obj.get("ow_material", ""))
        if not material_name:
            issues.append(
                f"{obj.name}: collision material is not assigned."
            )
        elif material_name not in used_materials:
            issues.append(
                f"{obj.name}: collision material {material_name!r} is not "
                "used by a presentation group."
            )
        owner_name = str(obj.get("ow_map_object_owner", "")).strip()
        if (
            settings.export_editable_objects
            and owner_name
            and owner_name not in {
                visual.name for visual in visual_objects
            }
        ):
            issues.append(
                f"{obj.name}: editable collision owner {owner_name!r} "
                "is not an exported presentation mesh."
            )
    collision_audit = None
    if collision_objects and inspect_geometry:
        _triangles, collision_audit = exporter.audit_collision_geometry(
            collision_objects,
            include_editor_ownership=settings.export_editable_objects,
        )
        issues.extend(collision_audit.issues)
        warnings.extend(collision_audit.warnings)

    for obj in grind_objects:
        if not any(
            len(spline.points) >= 2 or len(spline.bezier_points) >= 2
            for spline in obj.data.splines
        ):
            warnings.append(f"{obj.name}: grind curve has fewer than 2 points.")
    for obj in npc_path_objects:
        if not any(
            len(spline.points) >= 2 or len(spline.bezier_points) >= 2
            for spline in obj.data.splines
        ):
            issues.append(f"{obj.name}: NPC path has fewer than 2 points.")

    if spawn is None:
        issues.append("Missing OW_SPAWN; select Create Spawn Locator.")
    if not settings.map_name.strip():
        issues.append("Map Name cannot be empty.")

    raw_output = settings.output_path.strip()
    if not raw_output:
        issues.append("Choose an SKATE output file.")
    elif Path(bpy.path.abspath(raw_output)).suffix.lower() != ".skate":
        warnings.append("The .skate extension will be added automatically.")

    collision_stats = (
        f", {collision_audit.exported_triangles} usable collision triangle(s)"
        if collision_audit is not None
        else ""
    )
    stats = (
        f"{len(visual_objects)} visual object(s), "
        f"{len(collision_objects)} collision object(s), "
        f"{len(grind_objects)} grind curve(s), "
        f"{len(npc_path_objects)} NPC route(s), "
        f"{len(used_materials)} material(s), "
        f"{len(local_lights)} local light(s)"
        f"{collision_stats}"
    )
    return issues, warnings, stats


def _store_validation(
    settings: OwnedWorldSceneSettings,
    issues: list[str],
    warnings: list[str],
    stats: str,
) -> None:
    lines = [
        *(f"ERROR: {message}" for message in issues),
        *(f"WARNING: {message}" for message in warnings),
    ]
    settings.validation_details = "\n".join(lines)
    if issues:
        settings.last_status = (
            f"Map needs attention: {len(issues)} error(s), "
            f"{len(warnings)} warning(s)."
        )
    elif warnings:
        settings.last_status = (
            f"Ready to export with {len(warnings)} warning(s): {stats}."
        )
    else:
        settings.last_status = f"Ready to export: {stats}."


def _resolved_output(settings: OwnedWorldSceneSettings) -> Path:
    raw = settings.output_path.strip()
    if not raw:
        raw = "//my_skate_map.skate"
    output = Path(bpy.path.abspath(raw)).resolve()
    if output.suffix.lower() != ".skate":
        output = output.with_suffix(".skate")
    return output


def _run_export(context, output: Path) -> set[str]:
    settings = context.scene.owned_world
    _auto_stats, auto_warnings = _auto_prepare_scene(context)
    _sync_scene(settings)
    # The exporter performs the authoritative geometry audit once. Keeping
    # this preflight structural-only avoids evaluating huge collision meshes
    # twice during Quick Export.
    issues, warnings, stats = _validate_scene(
        context, inspect_geometry=False
    )
    warnings = [*auto_warnings, *warnings]
    _store_validation(settings, issues, warnings, stats)
    if issues:
        raise ValueError(
            "Map validation failed. Open the Skate 3 Map panel for details."
        )
    window_manager = context.window_manager
    last_percent = -1
    window_manager.progress_begin(0, 100)

    def update_progress(fraction: float, stage: str) -> None:
        nonlocal last_percent
        percent = max(0, min(100, int(round(fraction * 100.0))))
        settings.last_status = f"Exporting {percent}% — {stage}"
        window_manager.progress_update(percent)
        workspace = getattr(context, "workspace", None)
        if workspace is not None:
            try:
                workspace.status_text_set(
                    text=f"SKATE export: {percent}% — {stage}"
                )
            except (AttributeError, RuntimeError):
                pass
        if percent == last_percent:
            return
        last_percent = percent
        print(
            f"SKATE progress: {percent}% — {stage}",
            flush=True,
        )
        if bpy.app.background:
            return
        screen = getattr(context, "screen", None)
        if screen is not None:
            for area in screen.areas:
                area.tag_redraw()
        try:
            bpy.ops.wm.redraw_timer(type="DRAW_WIN_SWAP", iterations=1)
        except RuntimeError:
            pass

    try:
        result = exporter.export_scene(
            output,
            force_rebuild=settings.export_mode == "FORCE",
            metadata_only=settings.export_mode == "METADATA",
            export_editable_objects=settings.export_editable_objects,
            progress=update_progress,
        )
    finally:
        window_manager.progress_end()
        workspace = getattr(context, "workspace", None)
        if workspace is not None:
            try:
                workspace.status_text_set(text=None)
            except (AttributeError, RuntimeError):
                pass
    settings.output_path = str(result)
    settings.last_status = f"Exported successfully: {result.name}"
    collision_audit = exporter.LAST_COLLISION_AUDIT
    if collision_audit is not None:
        warnings.extend(collision_audit.warnings)
    settings.validation_details = "\n".join(
        f"WARNING: {message}" for message in warnings
    )
    return {"FINISHED"}


def _store_export_failure(
    settings: OwnedWorldSceneSettings,
    error: Exception,
) -> None:
    if isinstance(error, exporter.CollisionGeometryError):
        _store_validation(
            settings,
            error.issues,
            error.warnings,
            "collision geometry audit",
        )
        return
    settings.last_status = f"Export failed: {error}"
    if (
        isinstance(error, ValueError)
        and settings.validation_details.startswith("ERROR:")
    ):
        return
    settings.validation_details = f"ERROR: {error}"


def _ensure_spawn_locator(scene: bpy.types.Scene) -> bpy.types.Object:
    existing = bpy.data.objects.get(exporter.SPAWN_OBJECT)
    if existing is not None and existing.type == "MESH":
        return existing
    location = existing.location.copy() if existing is not None else None
    rotation = (
        existing.rotation_euler.copy() if existing is not None else None
    )
    if existing is not None:
        bpy.data.objects.remove(existing, do_unlink=True)
    mesh = bpy.data.meshes.new("OW_SPAWN_LOCATOR_MESH")
    # Four-metre square pad plus a raised arrow pointing along local +Y.
    vertices = [
        (-2.0, -2.0, 0.0),
        (2.0, -2.0, 0.0),
        (2.0, 2.0, 0.0),
        (-2.0, 2.0, 0.0),
        (-0.35, -1.15, 0.04),
        (0.35, -1.15, 0.04),
        (0.35, 0.65, 0.04),
        (0.9, 0.65, 0.04),
        (0.0, 1.65, 0.04),
        (-0.9, 0.65, 0.04),
        (-0.35, 0.65, 0.04),
    ]
    mesh.from_pydata(
        vertices,
        [(0, 1), (1, 2), (2, 3), (3, 0)],
        [(4, 5, 6, 7, 8, 9, 10)],
    )
    mesh.update()
    spawn = bpy.data.objects.new(exporter.SPAWN_OBJECT, mesh)
    scene.collection.objects.link(spawn)
    spawn.location = location if location is not None else (0.0, 0.0, 0.0)
    if rotation is not None:
        spawn.rotation_euler = rotation
    spawn.hide_render = True
    spawn.show_in_front = True
    spawn.color = (0.1, 0.9, 0.25, 0.65)
    spawn["ow_spawn_locator"] = True
    return spawn


class SKATE_OT_prepare_scene(Operator):
    bl_idname = "skate_map.prepare_scene"
    bl_label = "Prepare Scene"
    bl_description = (
        "Create the five exclusive map groups and movable spawn locator"
    )
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        for name in _group_collections():
            _ensure_collection(context.scene, name)
        _migrate_legacy_groups(context.scene)
        _ensure_spawn_locator(context.scene)
        _sync_scene(context.scene.owned_world)
        context.scene.owned_world.last_status = (
            "Scene prepared. Move objects between the five map groups as needed."
        )
        self.report({"INFO"}, "Owned World scene structure is ready")
        return {"FINISHED"}


class SKATE_OT_auto_prepare_scene(Operator):
    bl_idname = "skate_map.auto_prepare_scene"
    bl_label = "Adopt Blender Scene"
    bl_description = (
        "Adopt visible Blender meshes with a predictable collision-on "
        "default, translate Principled materials, and preserve explicit "
        "SKATE roles without guessing from names"
    )
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        try:
            stats, warnings = _auto_prepare_scene(
                context, include_existing_roles=True
            )
            settings = context.scene.owned_world
            settings.last_status = stats
            settings.validation_details = "\n".join(
                f"WARNING: {warning}" for warning in warnings
            )
            self.report({"INFO"}, stats)
            return {"FINISHED"}
        except Exception as error:
            traceback.print_exc()
            _store_export_failure(context.scene.owned_world, error)
            self.report({"ERROR"}, str(error))
            return {"CANCELLED"}


class SKATE_OT_assign_selected(Operator):
    bl_idname = "skate_map.assign_selected"
    bl_label = "Assign Selected"
    bl_description = "Assign selected objects to an SKATE role"
    bl_options = {"REGISTER", "UNDO"}

    role: EnumProperty(
        name="Role",
        items=(
            ("VISUAL", "Visual", "Rendered map geometry"),
            ("COLLISION", "Collision", "Authoritative collision geometry"),
            ("GRIND", "Grind", "Curve used as a grind path"),
            (
                "NPC_PATH",
                "NPC Path (Experimental)",
                "Experimental route data for native AI skaters",
            ),
        ),
    )

    def execute(self, context):
        if self.role in {"GRIND", "NPC_PATH"}:
            objects = _selected_of_type(context, "CURVE")
            collection_name = (
                exporter.GRIND_COLLECTION
                if self.role == "GRIND"
                else exporter.NPC_PATH_COLLECTION
            )
        else:
            objects = _selected_of_type(context, "MESH")
            collection_name = (
                exporter.VISUAL_COLLECTION
                if self.role == "VISUAL"
                else exporter.COLLISION_COLLECTION
            )
        if not objects:
            expected = (
                "curves" if self.role in {"GRIND", "NPC_PATH"} else "meshes"
            )
            self.report({"ERROR"}, f"Select one or more {expected}")
            return {"CANCELLED"}

        collection = _ensure_collection(context.scene, collection_name)
        missing_material: list[str] = []
        for obj in objects:
            if self.role == "VISUAL":
                destination = (
                    exporter.PRESENTATION_COLLISION_COLLECTION
                    if exporter.NO_PRESENTATION_COLLECTION
                    in _object_groups(obj)
                    else exporter.NO_COLLISION_COLLECTION
                )
                _move_to_group(context.scene, obj, destination)
                obj["ow_export_visual"] = True
            elif self.role == "COLLISION":
                destination = (
                    exporter.PRESENTATION_COLLISION_COLLECTION
                    if exporter.NO_COLLISION_COLLECTION in _object_groups(obj)
                    else exporter.NO_PRESENTATION_COLLECTION
                )
                _move_to_group(context.scene, obj, destination)
                material = obj.active_material
                if material is None:
                    material = next(
                        (
                            slot.material
                            for slot in obj.material_slots
                            if slot.material is not None
                        ),
                        None,
                    )
                if material is None:
                    missing_material.append(obj.name)
                else:
                    obj.owned_world_physics.collision_material = material
            elif self.role == "NPC_PATH":
                _move_to_group(
                    context.scene, obj, exporter.NPC_PATH_COLLECTION
                )
                _sync_npc_path(obj.owned_world_npc_path)
            else:
                _move_to_group(context.scene, obj, exporter.GRIND_COLLECTION)

        if missing_material:
            self.report(
                {"WARNING"},
                "Collision assigned, but these objects need a material: "
                + ", ".join(missing_material[:4]),
            )
        else:
            self.report(
                {"INFO"},
                f"Assigned {len(objects)} object(s) as {self.role.lower()}",
            )
        return {"FINISHED"}


_GROUP_SHORT_NAMES = {
    exporter.PRESENTATION_COLLISION_COLLECTION: "1  Presentation + Collision",
    exporter.NO_PRESENTATION_COLLECTION: "2  No Presentation",
    exporter.NO_COLLISION_COLLECTION: "3  No Collision",
    exporter.GRIND_COLLECTION: "4  Grinds",
    exporter.NPC_PATH_COLLECTION: "5  Pathing",
}


def _objects_using_material(
    scene: bpy.types.Scene, material: bpy.types.Material
) -> list[bpy.types.Object]:
    return [
        obj
        for obj in scene.objects
        if obj.type == "MESH"
        and any(slot.material == material for slot in obj.material_slots)
        and obj.name != exporter.SPAWN_OBJECT
        and not _is_auto_helper_object(obj)
        and (_object_groups(obj) or _auto_visual_candidate(obj))
    ]


def _material_group_status(
    scene: bpy.types.Scene, material: bpy.types.Material
) -> str:
    groups = {
        group
        for obj in _objects_using_material(scene, material)
        for group in _object_groups(obj)
    }
    if len(groups) == 1:
        return _GROUP_SHORT_NAMES[next(iter(groups))]
    if len(groups) > 1:
        return "Mixed groups"
    authored = str(material.get("ow_map_group", ""))
    return _GROUP_SHORT_NAMES.get(authored, "Unassigned")


class SKATE_UL_material_groups(UIList):
    bl_idname = "SKATE_UL_material_groups"

    def draw_item(
        self,
        context,
        layout,
        _data,
        item,
        icon,
        _active_data,
        _active_property,
        _index=0,
        _filter_flag=0,
    ):
        if item is None:
            return
        row = layout.row(align=True)
        row.label(text=item.name, icon_value=icon)
        row.label(text=_material_group_status(context.scene, item))


class SKATE_OT_set_material_group(Operator):
    bl_idname = "skate_map.set_material_group"
    bl_label = "Set Highlighted Material Group"
    bl_description = (
        "Move every scene mesh using the highlighted material into this "
        "exclusive map group"
    )
    bl_options = {"REGISTER", "UNDO"}

    group: EnumProperty(
        name="Group",
        items=(
            (
                exporter.PRESENTATION_COLLISION_COLLECTION,
                "1: Presentation + Collision",
                "Default rendered and collidable map geometry",
            ),
            (
                exporter.NO_PRESENTATION_COLLECTION,
                "2: No Presentation",
                "Collision-only meshes such as coping and effects",
            ),
            (
                exporter.NO_COLLISION_COLLECTION,
                "3: No Collision",
                "Rendered decals, vegetation, and phase-through geometry",
            ),
        ),
    )

    def execute(self, context):
        materials = bpy.data.materials
        if not materials:
            self.report({"ERROR"}, "This file has no materials")
            return {"CANCELLED"}
        index = max(
            0,
            min(
                int(context.scene.owned_world.material_list_index),
                len(materials) - 1,
            ),
        )
        material = materials[index]
        objects = _objects_using_material(context.scene, material)
        material["ow_map_group"] = self.group
        for obj in objects:
            _move_to_group(context.scene, obj, self.group)
            obj["ow_export_visual"] = (
                self.group != exporter.NO_PRESENTATION_COLLECTION
            )
        if not objects:
            self.report(
                {"WARNING"},
                f"Stored Group {_GROUP_SHORT_NAMES[self.group][0]} for "
                f"{material.name}, but no scene meshes use it",
            )
        else:
            self.report(
                {"INFO"},
                f"Moved {len(objects)} mesh(es) using {material.name} to "
                f"{_GROUP_SHORT_NAMES[self.group]}",
            )
        return {"FINISHED"}


class SKATE_OT_move_selected_to_group(Operator):
    bl_idname = "skate_map.move_selected_to_group"
    bl_label = "Move Selected to Map Group"
    bl_description = (
        "Unlink selected objects from every map group and link them to "
        "exactly one destination group"
    )
    bl_options = {"REGISTER", "UNDO"}

    group: EnumProperty(
        name="Group",
        items=(
            (
                exporter.PRESENTATION_COLLISION_COLLECTION,
                "1: Presentation + Collision",
                "Default rendered and collidable map geometry",
            ),
            (
                exporter.NO_PRESENTATION_COLLECTION,
                "2: No Presentation",
                "Collision-only meshes such as coping and effects",
            ),
            (
                exporter.NO_COLLISION_COLLECTION,
                "3: No Collision",
                "Rendered decals, vegetation, and phase-through geometry",
            ),
            (
                exporter.GRIND_COLLECTION,
                "4: Grinds",
                "Curve objects used as grind paths",
            ),
            (
                exporter.NPC_PATH_COLLECTION,
                "5: Pathing",
                "Curve objects used as pathing routes",
            ),
        ),
    )

    def execute(self, context):
        objects = list(context.selected_objects)
        expected = (
            "CURVE"
            if self.group
            in {exporter.GRIND_COLLECTION, exporter.NPC_PATH_COLLECTION}
            else "MESH"
        )
        objects = [obj for obj in objects if obj.type == expected]
        if not objects:
            self.report({"ERROR"}, f"Select one or more {expected} objects")
            return {"CANCELLED"}
        for obj in objects:
            _move_to_group(context.scene, obj, self.group)
            obj["ow_export_visual"] = self.group in {
                exporter.PRESENTATION_COLLISION_COLLECTION,
                exporter.NO_COLLISION_COLLECTION,
            }
            for slot in obj.material_slots:
                if slot.material is not None:
                    slot.material["ow_map_group"] = self.group
        self.report(
            {"INFO"}, f"Moved {len(objects)} object(s) to {self.group}"
        )
        return {"FINISHED"}


class SKATE_OT_create_uv_layers(Operator):
    bl_idname = "skate_map.create_uv_layers"
    bl_label = "Create Missing UV Layers"
    bl_description = (
        "Create UVMap and Lightmap layers, copying the active UVs when "
        "available"
    )
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        objects = _selected_of_type(context, "MESH")
        if not objects:
            self.report({"ERROR"}, "Select one or more mesh objects")
            return {"CANCELLED"}
        created = 0
        empty_created = 0
        for obj in objects:
            layers = obj.data.uv_layers
            source = layers.active
            uv_map = layers.get("UVMap")
            if uv_map is None:
                uv_map = _new_uv_layer(obj.data, "UVMap")
                created += 1
                if source is not None and source != uv_map:
                    for destination, original in zip(
                        uv_map.data, source.data, strict=False
                    ):
                        destination.uv = original.uv
                else:
                    empty_created += 1
            lightmap = layers.get("Lightmap")
            if lightmap is None:
                lightmap = _new_uv_layer(obj.data, "Lightmap")
                created += 1
                for destination, original in zip(
                    lightmap.data, uv_map.data, strict=False
                ):
                    destination.uv = original.uv
        if empty_created:
            self.report(
                {"WARNING"},
                "Layers created; meshes without existing UVs still need "
                "to be unwrapped",
            )
        else:
            self.report({"INFO"}, f"Created {created} missing UV layer(s)")
        return {"FINISHED"}


class OWMATERIAL_OT_sync_materials(Operator):
    bl_idname = "owmaterial.sync_materials"
    bl_label = "Sync Materials"
    bl_description = (
        "Refresh Owned World PBR settings from Principled shaders and "
        "overwrite the add-on generated normal, ORM, and emissive maps"
    )
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        materials: list[bpy.types.Material] = []
        seen: set[int] = set()
        objects = [
            obj for obj in context.selected_objects if obj.type == "MESH"
        ]
        if objects:
            for obj in objects:
                for slot in obj.material_slots:
                    material = slot.material
                    if (
                        material is not None
                        and material.as_pointer() not in seen
                    ):
                        seen.add(material.as_pointer())
                        materials.append(material)
        elif getattr(context, "material", None) is not None:
            materials.append(context.material)
        if not materials:
            self.report(
                {"ERROR"}, "Select meshes with materials or an active material"
            )
            return {"CANCELLED"}
        for material in materials:
            _auto_configure_material(
                material, force=True, generate_maps=True
            )
        self.report(
            {"INFO"},
            f"Synced {len(materials)} material(s) and regenerated PBR maps",
        )
        return {"FINISHED"}


def _sharp_edge_chains(mesh: bpy.types.Mesh) -> list[list[int]]:
    sharp_attribute = mesh.attributes.get("sharp_edge")
    sharp_edges: list[tuple[int, int, int]] = []
    for index, edge in enumerate(mesh.edges):
        marked = (
            bool(sharp_attribute.data[index].value)
            if sharp_attribute is not None
            and sharp_attribute.domain == "EDGE"
            else bool(getattr(edge, "use_edge_sharp", False))
        )
        if marked:
            sharp_edges.append((index, edge.vertices[0], edge.vertices[1]))
    adjacency: dict[int, list[tuple[int, int]]] = {}
    for edge_index, first, second in sharp_edges:
        adjacency.setdefault(first, []).append((edge_index, second))
        adjacency.setdefault(second, []).append((edge_index, first))
    used: set[int] = set()
    chains: list[list[int]] = []

    def walk(start: int, edge_index: int, next_vertex: int) -> list[int]:
        chain = [start, next_vertex]
        used.add(edge_index)
        previous = start
        current = next_vertex
        while len(adjacency.get(current, ())) == 2:
            candidates = [
                item
                for item in adjacency[current]
                if item[0] not in used and item[1] != previous
            ]
            if not candidates:
                break
            next_edge, destination = candidates[0]
            used.add(next_edge)
            chain.append(destination)
            previous, current = current, destination
        return chain

    for vertex, connected in adjacency.items():
        if len(connected) == 2:
            continue
        for edge_index, destination in connected:
            if edge_index not in used:
                chains.append(walk(vertex, edge_index, destination))
    for edge_index, first, second in sharp_edges:
        if edge_index not in used:
            chain = walk(first, edge_index, second)
            if chain[-1] != chain[0]:
                remaining = [
                    item
                    for item in adjacency.get(chain[-1], ())
                    if item[0] not in used
                ]
                while remaining:
                    next_edge, destination = remaining[0]
                    used.add(next_edge)
                    chain.append(destination)
                    remaining = [
                        item
                        for item in adjacency.get(chain[-1], ())
                        if item[0] not in used
                    ]
            chains.append(chain)
    return [chain for chain in chains if len(chain) >= 2]


class SKATE_OT_grinds_from_sharp_edges(Operator):
    bl_idname = "skate_map.grinds_from_sharp_edges"
    bl_label = "Create Grind Splines from Sharp Edges"
    bl_description = (
        "Generate consolidated grind curves from edges marked Sharp on "
        "selected meshes"
    )
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        sources = _selected_of_type(context, "MESH")
        if not sources:
            self.report({"ERROR"}, "Select one or more mesh objects")
            return {"CANCELLED"}
        created: list[bpy.types.Object] = []
        chain_count = 0
        for source in sources:
            chains = _sharp_edge_chains(source.data)
            if not chains:
                continue
            curve = bpy.data.curves.new(
                f"{source.name}_Grinds", type="CURVE"
            )
            curve.dimensions = "3D"
            curve.resolution_u = 1
            for chain in chains:
                spline = curve.splines.new("POLY")
                spline.points.add(len(chain) - 1)
                for point, vertex_index in zip(
                    spline.points, chain, strict=True
                ):
                    point.co = (*source.data.vertices[vertex_index].co, 1.0)
            obj = bpy.data.objects.new(f"{source.name}_Grinds", curve)
            context.scene.collection.objects.link(obj)
            obj.matrix_world = source.matrix_world.copy()
            _move_to_group(context.scene, obj, exporter.GRIND_COLLECTION)
            created.append(obj)
            chain_count += len(chains)
        if not created:
            self.report(
                {"ERROR"}, "Selected meshes contain no edges marked Sharp"
            )
            return {"CANCELLED"}
        bpy.ops.object.select_all(action="DESELECT")
        for obj in created:
            obj.select_set(True)
        context.view_layer.objects.active = created[-1]
        self.report(
            {"INFO"},
            f"Created {chain_count} consolidated grind spline(s) "
            f"in {len(created)} curve object(s)",
        )
        return {"FINISHED"}


class SKATE_OT_set_spawn(Operator):
    bl_idname = "skate_map.set_spawn"
    bl_label = "Create / Select Spawn Locator"
    bl_description = (
        "Create a movable four-metre spawn pad with a heading arrow, then "
        "select it for positioning and rotation"
    )
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        spawn = _ensure_spawn_locator(context.scene)
        bpy.ops.object.select_all(action="DESELECT")
        spawn.select_set(True)
        context.view_layer.objects.active = spawn
        self.report(
            {"INFO"},
            "Move and rotate OW_SPAWN; its arrow controls spawn heading",
        )
        return {"FINISHED"}


class SKATE_OT_validate(Operator):
    bl_idname = "skate_map.validate"
    bl_label = "Validate Map"
    bl_description = "Check the scene and explain anything blocking export"

    def execute(self, context):
        settings = context.scene.owned_world
        _auto_stats, auto_warnings = _auto_prepare_scene(context)
        _sync_scene(settings)
        issues, warnings, stats = _validate_scene(context)
        warnings = [*auto_warnings, *warnings]
        _store_validation(settings, issues, warnings, stats)
        if issues:
            self.report(
                {"ERROR"},
                f"Found {len(issues)} error(s); see the Skate 3 Map panel",
            )
            return {"CANCELLED"}
        self.report(
            {"INFO"},
            f"Map is ready ({len(warnings)} warning(s)): {stats}",
        )
        return {"FINISHED"}


class SKATE_OT_quick_export(Operator):
    bl_idname = "skate_map.quick_export"
    bl_label = "Quick Export"
    bl_description = "Export directly to the configured SKATE file"

    def execute(self, context):
        try:
            return _run_export(
                context, _resolved_output(context.scene.owned_world)
            )
        except Exception as error:
            traceback.print_exc()
            _store_export_failure(context.scene.owned_world, error)
            self.report({"ERROR"}, str(error))
            return {"CANCELLED"}


class SKATE_OT_export_dialog(Operator, ExportHelper):
    bl_idname = "skate_map.export_dialog"
    bl_label = "Export Skate 3 Custom Engine Map"
    bl_description = "Validate and export this scene as an SKATE package"
    filename_ext = ".skate"

    filter_glob: StringProperty(default="*.skate", options={"HIDDEN"})

    def invoke(self, context, event):
        self.filepath = str(_resolved_output(context.scene.owned_world))
        return ExportHelper.invoke(self, context, event)

    def execute(self, context):
        try:
            output = Path(self.filepath).resolve()
            if output.suffix.lower() != ".skate":
                output = output.with_suffix(".skate")
            return _run_export(context, output)
        except Exception as error:
            traceback.print_exc()
            _store_export_failure(context.scene.owned_world, error)
            self.report({"ERROR"}, str(error))
            return {"CANCELLED"}


class SKATE_OT_open_output_folder(Operator):
    bl_idname = "skate_map.open_output_folder"
    bl_label = "Open Output Folder"
    bl_description = "Open the folder containing the exported SKATE"

    def execute(self, context):
        folder = _resolved_output(context.scene.owned_world).parent
        folder.mkdir(parents=True, exist_ok=True)
        bpy.ops.wm.path_open(filepath=str(folder))
        return {"FINISHED"}


class OWMATERIAL_OT_apply_preset(Operator):
    bl_idname = "owmaterial.apply_preset"
    bl_label = "Apply Owned Material Preset"
    bl_options = {"REGISTER", "UNDO"}

    preset: EnumProperty(name="Preset", items=PRESET_ITEMS)

    def execute(self, context):
        material = getattr(context, "material", None)
        if material is None and context.object is not None:
            material = context.object.active_material
        if material is None:
            self.report({"ERROR"}, "No active material")
            return {"CANCELLED"}
        audio, physics, pattern, roughness, metallic, alpha = PRESETS[
            self.preset
        ]
        settings = material.owned_world
        settings.audio_surface = str(audio)
        settings.physics_surface = str(physics)
        settings.surface_pattern = str(pattern)
        settings.roughness = roughness
        settings.metallic = metallic
        settings.alpha_mode = str(alpha)
        if self.preset == "DECAL":
            settings.presentation_type = "DECAL"
            settings.collision_enabled = False
        elif self.preset == "TREE":
            settings.presentation_type = "VEGETATION"
            settings.collision_enabled = False
            settings.alpha_cutoff = 0.42
        else:
            settings.presentation_type = "STANDARD"
        _sync(settings)
        self.report({"INFO"}, f"Applied {self.preset} to {material.name}")
        return {"FINISHED"}


class OWMATERIAL_PT_material(Panel):
    bl_label = "Owned World Material"
    bl_idname = "OWMATERIAL_PT_material"
    bl_space_type = "PROPERTIES"
    bl_region_type = "WINDOW"
    bl_context = "material"

    @classmethod
    def poll(cls, context):
        return context.material is not None

    def draw(self, context):
        layout = self.layout
        settings = context.material.owned_world

        presets = layout.box()
        presets.label(text="Quick Presets")
        grid = presets.grid_flow(columns=2, even_columns=True)
        for identifier, label, _description in PRESET_ITEMS:
            operator = grid.operator(
                "owmaterial.apply_preset", text=label
            )
            operator.preset = identifier

        game = layout.box()
        game.label(text="Skate 3 Contact")
        game.prop(settings, "audio_surface")
        game.prop(settings, "physics_surface")
        game.prop(settings, "surface_pattern")
        game.prop(settings, "collision_enabled")
        game.prop(settings, "friction")
        game.prop(settings, "restitution")

        pbr = layout.box()
        pbr.label(text="PBR Presentation")
        pbr.operator(
            "owmaterial.sync_materials",
            text="Sync Materials from Shaders",
            icon="FILE_REFRESH",
        )
        pbr.prop(settings, "presentation_type")
        pbr.prop(settings, "albedo_image")
        pbr.prop(settings, "lightmap_image")
        pbr.prop(settings, "normal_image")
        pbr.prop(settings, "orm_image")
        pbr.prop(settings, "emissive_image")
        pbr.prop(settings, "roughness")
        pbr.prop(settings, "metallic")
        pbr.prop(settings, "baked_strength")
        pbr.prop(settings, "emissive_strength")
        pbr.prop(settings, "alpha_mode")
        if settings.alpha_mode == "1":
            pbr.prop(settings, "alpha_cutoff")
        pbr.label(text="ORM channels: R=AO, G=Roughness, B=Metallic")


class OWPHYSICS_PT_object(Panel):
    bl_label = "Owned World Physics"
    bl_idname = "OWPHYSICS_PT_object"
    bl_space_type = "PROPERTIES"
    bl_region_type = "WINDOW"
    bl_context = "physics"

    @classmethod
    def poll(cls, context):
        return context.object is not None and context.object.type == "MESH"

    def draw(self, context):
        layout = self.layout
        obj = context.object
        settings = obj.owned_world_physics
        collision = layout.box()
        collision.label(text="Collision Authoring")
        collision.prop(settings, "collision_material")
        collision.prop(settings, "upward_surface")
        layout.prop(settings, "physics_type")
        if settings.physics_type in {"BOX3D_STATIC", "BOX3D_DYNAMIC"}:
            body = layout.box()
            body.label(text="Box3D Body")
            body.prop(settings, "box3d_collision_shape")
            if settings.physics_type == "BOX3D_DYNAMIC":
                body.prop(settings, "box3d_density")
                body.prop(settings, "box3d_linear_damping")
                body.prop(settings, "box3d_angular_damping")
                body.prop(settings, "box3d_gravity_scale")
                body.prop(settings, "box3d_enable_sleep")
                if settings.box3d_enable_sleep:
                    body.prop(settings, "box3d_initially_awake")
                breaking = layout.box()
                breaking.label(text="Breakable Group")
                breaking.prop(settings, "box3d_break_group")
                if settings.box3d_break_group > 0:
                    breaking.prop(settings, "box3d_break_speed_threshold")
                    breaking.prop(settings, "box3d_break_impulse_scale")
                    breaking.prop(settings, "box3d_break_angular_impulse")
                    breaking.prop(settings, "box3d_break_gravity_scale")
            contact = layout.box()
            contact.label(text="Box3D Contact")
            contact.prop(settings, "box3d_friction")
            contact.prop(settings, "box3d_restitution")
            return
        if settings.physics_type != "HINGED_DOOR":
            return
        hinge = layout.box()
        hinge.label(text="Hinge")
        hinge.operator("owphysics.hinge_from_cursor")
        hinge.prop(settings, "hinge_axis")
        if "ow_hinge_position" in obj:
            hinge.label(
                text="Hinge: {:.2f}, {:.2f}, {:.2f}".format(
                    *obj["ow_hinge_position"]
                )
            )
        else:
            hinge.label(text="Hinge uses object origin", icon="INFO")
        limits = layout.box()
        limits.label(text="Opening Limits")
        limits.prop(settings, "minimum_angle")
        limits.prop(settings, "maximum_angle")
        limits.prop(settings, "initial_angle")
        body = layout.box()
        body.label(text="Rigid Body")
        body.prop(settings, "mass")
        body.prop(settings, "angular_damping")
        body.prop(settings, "return_spring_strength")
        body.prop(settings, "maximum_angular_speed")
        body.prop(settings, "contact_impulse_scale")
        body.prop(settings, "friction")
        body.prop(settings, "restitution")
        body.label(text="Push motion is contact-driven at runtime")


class OWNPCPATH_PT_object(Panel):
    bl_label = "Owned World NPC Path (Experimental)"
    bl_idname = "OWNPCPATH_PT_object"
    bl_space_type = "PROPERTIES"
    bl_region_type = "WINDOW"
    bl_context = "object"

    @classmethod
    def poll(cls, context):
        obj = context.object
        collection = bpy.data.collections.get(exporter.NPC_PATH_COLLECTION)
        return (
            obj is not None
            and obj.type == "CURVE"
            and collection is not None
            and obj.name in collection.all_objects
        )

    def draw(self, context):
        layout = self.layout
        settings = context.object.owned_world_npc_path
        layout.label(text="Experimental Native AI Route", icon="ERROR")
        layout.label(text="Reliable route following is not yet supported.")
        layout.prop(settings, "skater_count")
        layout.prop(settings, "speed")
        layout.prop(settings, "spawn_spacing")
        layout.label(
            text="Enable Cyclic on the spline for a continuous loop.",
            icon="INFO",
        )


def _draw_map_panel(layout, context) -> None:
    settings = context.scene.owned_world

    intro = layout.box()
    intro.label(text="Skate 3 Custom Engine Map", icon="WORLD")
    intro.label(text="Open a Blender map, set spawn, then export.")
    intro.operator(
        "skate_map.auto_prepare_scene",
        text="Auto Prepare Blender Map",
        icon="IMPORT",
    )
    intro.label(
        text="Export also auto-prepares an untouched scene.",
        icon="INFO",
    )

    authoring = layout.box()
    authoring.label(text="Material Map Groups", icon="MATERIAL")
    if bpy.data.materials:
        authoring.template_list(
            "SKATE_UL_material_groups",
            "",
            bpy.data,
            "materials",
            settings,
            "material_list_index",
            rows=6,
        )
        buttons = authoring.column(align=True)
        for group, label, icon in (
            (
                exporter.PRESENTATION_COLLISION_COLLECTION,
                "1  Presentation + Collision (Default)",
                "MESH_CUBE",
            ),
            (
                exporter.NO_PRESENTATION_COLLECTION,
                "2  No Presentation",
                "MOD_PHYSICS",
            ),
            (
                exporter.NO_COLLISION_COLLECTION,
                "3  No Collision",
                "CANCEL",
            ),
        ):
            operator = buttons.operator(
                "skate_map.set_material_group", text=label, icon=icon
            )
            operator.group = group
        authoring.label(
            text="Buttons affect every mesh using the highlighted material.",
            icon="INFO",
        )
    else:
        authoring.label(text="No materials in this file.", icon="ERROR")

    curves = authoring
    curves.separator()
    curves.label(text="Curve Groups use selected curves.", icon="CURVE_DATA")
    for group, label, icon in (
        (
            exporter.GRIND_COLLECTION,
            "4  Grinds",
            "CURVE_DATA",
        ),
        (
            exporter.NPC_PATH_COLLECTION,
            "5  Pathing",
            "OUTLINER_OB_CURVE",
        ),
    ):
        operator = curves.operator(
            "skate_map.move_selected_to_group", text=label, icon=icon
        )
        operator.group = group

    tools = layout.box()
    tools.label(text="Authoring Tools", icon="TOOL_SETTINGS")
    tools.operator("skate_map.create_uv_layers", icon="GROUP_UVS")
    tools.operator(
        "skate_map.grinds_from_sharp_edges", icon="CURVE_DATA"
    )
    tools.operator(
        "skate_map.prepare_scene",
        text="Create Empty Authoring Structure",
        icon="TOOL_SETTINGS",
    )
    tools.label(
        text="Material and door controls are in Properties.",
        icon="INFO",
    )

    spawn = layout.box()
    spawn.label(text="Player Spawn", icon="EMPTY_ARROWS")
    spawn.operator("skate_map.set_spawn", icon="ORIENTATION_GLOBAL")
    spawn.label(text="Move the pad; rotate its arrow for heading.", icon="INFO")

    world = layout.box()
    world.label(text="Map and Lighting", icon="LIGHT_SUN")
    world.label(
        text="Normal Blender lights export automatically.",
        icon="LIGHT",
    )
    world.prop(settings, "map_name")
    world.prop(settings, "cycle_seconds")
    world.prop(settings, "dynamic_lighting_enabled_by_default")
    world.prop(settings, "cycle_ping_pong")
    row = world.row(align=True)
    row.prop(settings, "start_hour")
    if settings.cycle_ping_pong:
        row.prop(settings, "end_hour")
    world.prop(settings, "orbit_azimuth")
    world.prop(
        settings,
        "show_sky_colours",
        icon="TRIA_DOWN" if settings.show_sky_colours else "TRIA_RIGHT",
        emboss=False,
    )
    if settings.show_sky_colours:
        sky = world.column(align=True)
        sky.label(text="Day Palette")
        sky.prop(settings, "sky_zenith")
        sky.prop(settings, "sky_horizon")
        sky.prop(settings, "sky_nadir")
    world.prop(
        settings,
        "show_advanced_lighting",
        icon="TRIA_DOWN" if settings.show_advanced_lighting else "TRIA_RIGHT",
    )
    if settings.show_advanced_lighting:
        if settings.show_sky_colours:
            grade = world.column(align=True)
            grade.label(text="Sky Colour")
            grade.prop(settings, "sky_tint")
            twilight = world.column(align=True)
            twilight.label(text="Twilight Palette")
            twilight.prop(settings, "twilight_zenith")
            twilight.prop(settings, "twilight_horizon")
            twilight.prop(settings, "twilight_nadir")
            night = world.column(align=True)
            night.label(text="Night Palette")
            night.prop(settings, "night_zenith")
            night.prop(settings, "night_horizon")
            night.prop(settings, "night_nadir")
        lights = world.column(align=True)
        lights.label(text="Directional and Ambient Light")
        lights.prop(settings, "sun_color")
        lights.prop(settings, "sun_intensity")
        lights.prop(settings, "moon_color")
        lights.prop(settings, "moon_intensity")
        lights.prop(settings, "day_ambient")
        lights.prop(settings, "night_ambient")

    output = layout.box()
    output.label(text="Validate and Export", icon="EXPORT")
    output.prop(settings, "output_path")
    output.prop(settings, "export_mode")
    editable_row = output.row()
    editable_row.enabled = settings.export_mode != "METADATA"
    editable_row.prop(settings, "export_editable_objects")
    if (
        settings.export_mode != "METADATA"
        and not settings.export_editable_objects
    ):
        output.label(
            text="Existing map geometry will be static in the editor.",
            icon="LOCKED",
        )
    row = output.row(align=True)
    row.operator("skate_map.validate", icon="CHECKMARK")
    row.operator("skate_map.quick_export", icon="FILE_TICK")
    row = output.row(align=True)
    row.operator("skate_map.export_dialog", text="Export As...", icon="EXPORT")
    row.operator(
        "skate_map.open_output_folder", text="Open Folder", icon="FILE_FOLDER"
    )

    status_icon = (
        "ERROR"
        if settings.last_status.startswith(("Map needs", "Export failed"))
        else "INFO"
    )
    output.label(text=settings.last_status, icon=status_icon)
    for line in settings.validation_details.splitlines()[:20]:
        icon = "ERROR" if line.startswith("ERROR:") else "INFO"
        output.label(text=line, icon=icon)
    remaining = len(settings.validation_details.splitlines()) - 20
    if remaining > 0:
        output.label(
            text=f"...and {remaining} more. Fix these first, then validate."
        )


class SKATE_PT_sidebar(Panel):
    bl_label = "Map Builder"
    bl_idname = "SKATE_PT_sidebar"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Skate 3 Map"

    def draw(self, context):
        _draw_map_panel(self.layout, context)


class SKATE_PT_scene(Panel):
    bl_label = "Skate 3 Custom Engine Map"
    bl_idname = "SKATE_PT_scene"
    bl_space_type = "PROPERTIES"
    bl_region_type = "WINDOW"
    bl_context = "scene"

    def draw(self, context):
        _draw_map_panel(self.layout, context)


def _hydrate_material(material: Material) -> None:
    settings = material.owned_world
    settings.audio_surface = str(
        max(0, min(93, int(material.get("ow_audio_surface", 3))))
    )
    settings.physics_surface = str(
        max(0, min(13, int(material.get("ow_physics_surface", 1))))
    )
    settings.surface_pattern = str(
        max(0, min(15, int(material.get("ow_surface_pattern", 0))))
    )
    settings.alpha_mode = str(
        max(0, min(2, int(material.get("ow_alpha_mode", 0))))
    )
    settings.alpha_cutoff = float(material.get("ow_alpha_cutoff", 0.5))
    settings.roughness = float(material.get("ow_roughness", 0.78))
    settings.metallic = float(material.get("ow_metallic", 0.0))
    settings.friction = float(material.get("ow_friction", 0.82))
    settings.restitution = float(material.get("ow_restitution", 0.0))
    settings.emissive_strength = float(material.get("ow_emissive", 0.0))
    settings.baked_strength = float(
        material.get("ow_baked_strength", 1.0)
    )
    settings.collision_enabled = bool(
        material.get("ow_collision_enabled", True)
    )
    presentation_type = str(
        material.get("ow_presentation_type", "STANDARD")
    )
    settings.presentation_type = (
        presentation_type
        if presentation_type in {"STANDARD", "DECAL", "VEGETATION"}
        else "STANDARD"
    )
    for attribute, property_name in (
        ("albedo_image", "ow_albedo_image"),
        ("lightmap_image", "ow_lightmap_image"),
        ("normal_image", "ow_normal_image"),
        ("orm_image", "ow_orm_image"),
        ("emissive_image", "ow_emissive_image"),
    ):
        setattr(
            settings,
            attribute,
            bpy.data.images.get(str(material.get(property_name, ""))),
        )


def _hydrate_physics(obj: Object) -> None:
    settings = obj.owned_world_physics
    settings.collision_material = bpy.data.materials.get(
        str(obj.get("ow_material", ""))
    )
    settings.upward_surface = bool(obj.get("ow_upward_surface", False))
    physics_type = str(obj.get("ow_physics_type", "STATIC"))
    settings.physics_type = (
        physics_type
        if physics_type
        in {"STATIC", "BOX3D_STATIC", "BOX3D_DYNAMIC", "HINGED_DOOR"}
        else "STATIC"
    )
    shape = str(obj.get("ow_box3d_collision_shape", "BOX"))
    settings.box3d_collision_shape = (
        shape if shape in {"BOX", "SPHERE", "CONVEX_HULL"} else "BOX"
    )
    settings.box3d_density = max(
        1.0, min(5000.0, float(obj.get("ow_box3d_density", 100.0)))
    )
    settings.box3d_friction = max(
        0.0, min(2.0, float(obj.get("ow_box3d_friction", 0.55)))
    )
    settings.box3d_restitution = max(
        0.0, min(1.0, float(obj.get("ow_box3d_restitution", 0.05)))
    )
    settings.box3d_linear_damping = max(
        0.0, min(
            20.0, float(obj.get("ow_box3d_linear_damping", 0.05))
        )
    )
    settings.box3d_angular_damping = max(
        0.0, min(
            20.0, float(obj.get("ow_box3d_angular_damping", 0.15))
        )
    )
    settings.box3d_gravity_scale = max(
        0.0, min(4.0, float(obj.get("ow_box3d_gravity_scale", 1.0)))
    )
    settings.box3d_enable_sleep = bool(
        obj.get("ow_box3d_enable_sleep", True)
    )
    settings.box3d_initially_awake = bool(
        obj.get("ow_box3d_initially_awake", True)
    )
    settings.box3d_break_group = max(
        0, min(65535, int(obj.get("ow_box3d_break_group", 0)))
    )
    settings.box3d_break_speed_threshold = max(
        0.1,
        min(
            30.0,
            float(obj.get("ow_box3d_break_speed_threshold", 2.5)),
        ),
    )
    settings.box3d_break_impulse_scale = max(
        0.0,
        min(10.0, float(obj.get("ow_box3d_break_impulse_scale", 0.45))),
    )
    settings.box3d_break_angular_impulse = max(
        0.0,
        min(10.0, float(obj.get("ow_box3d_break_angular_impulse", 0.08))),
    )
    settings.box3d_break_gravity_scale = max(
        0.0,
        min(4.0, float(obj.get("ow_box3d_break_gravity_scale", 1.0))),
    )
    settings.hinge_axis = tuple(obj.get("ow_hinge_axis", (0.0, 0.0, 1.0)))
    settings.minimum_angle = math.radians(
        float(obj.get("ow_door_min_angle_degrees", -105.0))
    )
    settings.maximum_angle = math.radians(
        float(obj.get("ow_door_max_angle_degrees", 105.0))
    )
    settings.initial_angle = math.radians(
        float(obj.get("ow_door_initial_angle_degrees", 0.0))
    )
    settings.mass = float(obj.get("ow_door_mass", 32.0))
    settings.angular_damping = float(
        obj.get("ow_door_angular_damping", 2.2)
    )
    settings.return_spring_strength = float(
        obj.get("ow_door_return_spring_strength", 0.0)
    )
    settings.maximum_angular_speed = float(
        obj.get("ow_door_maximum_angular_speed", 8.0)
    )
    settings.contact_impulse_scale = float(
        obj.get("ow_door_contact_impulse_scale", 1.0)
    )
    settings.friction = float(obj.get("ow_door_friction", 0.55))
    settings.restitution = float(obj.get("ow_door_restitution", 0.02))


def _hydrate_npc_path(obj: Object) -> None:
    settings = obj.owned_world_npc_path
    settings.skater_count = max(
        1, min(32, int(obj.get("ow_npc_skater_count", 1)))
    )
    settings.speed = max(
        0.1, min(30.0, float(obj.get("ow_npc_speed", 5.5)))
    )
    settings.spawn_spacing = max(
        0.0, min(100.0, float(obj.get("ow_npc_spawn_spacing", 3.0)))
    )


def _hydrate_scene(scene: Scene) -> None:
    settings = scene.owned_world
    settings.map_name = str(scene.get("ow_map_name", "My Skate Map"))
    settings.cycle_seconds = float(scene.get("ow_cycle_seconds", 96.0))
    settings.start_hour = float(scene.get("ow_start_hour", 9.0))
    settings.end_hour = float(scene.get("ow_end_hour", 17.0))
    settings.cycle_ping_pong = bool(
        scene.get("ow_cycle_ping_pong", True)
    )
    settings.dynamic_lighting_enabled_by_default = bool(
        scene.get("ow_dynamic_lighting_enabled_by_default", True)
    )
    settings.orbit_azimuth = float(scene.get("ow_orbit_azimuth", 0.62))
    settings.sky_zenith = tuple(
        scene.get("ow_sky_zenith", (0.09, 0.34, 0.72))
    )
    settings.sky_horizon = tuple(
        scene.get("ow_sky_horizon", (0.58, 0.78, 0.98))
    )
    settings.sky_nadir = tuple(
        scene.get("ow_sky_nadir", (0.18, 0.25, 0.34))
    )
    settings.twilight_zenith = tuple(
        scene.get("ow_twilight_zenith", (0.045, 0.10, 0.26))
    )
    settings.twilight_horizon = tuple(
        scene.get("ow_twilight_horizon", (1.0, 0.32, 0.10))
    )
    settings.twilight_nadir = tuple(
        scene.get("ow_twilight_nadir", (0.05, 0.035, 0.06))
    )
    settings.night_zenith = tuple(
        scene.get("ow_night_zenith", (0.007, 0.015, 0.045))
    )
    settings.night_horizon = tuple(
        scene.get("ow_night_horizon", (0.045, 0.085, 0.17))
    )
    settings.night_nadir = tuple(
        scene.get("ow_night_nadir", (0.008, 0.014, 0.032))
    )
    settings.sun_color = tuple(
        scene.get("ow_sun_color", (1.0, 0.92, 0.78))
    )
    settings.moon_color = tuple(
        scene.get("ow_moon_color", (0.42, 0.56, 0.92))
    )
    settings.sun_intensity = float(scene.get("ow_sun_intensity", 1.25))
    settings.moon_intensity = float(scene.get("ow_moon_intensity", 0.18))
    settings.day_ambient = float(scene.get("ow_day_ambient", 0.32))
    settings.night_ambient = float(scene.get("ow_night_ambient", 0.11))
    settings.sky_tint = tuple(
        scene.get("ow_sky_tint", (1.0, 1.0, 1.0))
    )
    spawn = bpy.data.objects.get(exporter.SPAWN_OBJECT)
    settings.spawn_heading = (
        exporter._spawn_heading(spawn)
        if spawn is not None
        else 0.0
    )


def _hydrate_all() -> None:
    global _SYNCING
    _SYNCING = True
    try:
        for material in bpy.data.materials:
            _hydrate_material(material)
        for obj in bpy.data.objects:
            if obj.type == "MESH":
                _hydrate_physics(obj)
            elif obj.type == "CURVE":
                _hydrate_npc_path(obj)
        for scene in bpy.data.scenes:
            _hydrate_scene(scene)
    finally:
        _SYNCING = False


@persistent
def _load_post(_unused) -> None:
    _hydrate_all()


def _file_export_menu(self, _context) -> None:
    self.layout.operator(
        SKATE_OT_export_dialog.bl_idname,
        text="Skate 3 Custom Engine Map (.skate)",
    )


CLASSES = (
    OwnedWorldMaterialSettings,
    OwnedWorldPhysicsSettings,
    OwnedWorldNpcPathSettings,
    OwnedWorldSceneSettings,
    SKATE_OT_prepare_scene,
    SKATE_OT_auto_prepare_scene,
    SKATE_OT_assign_selected,
    SKATE_UL_material_groups,
    SKATE_OT_set_material_group,
    SKATE_OT_move_selected_to_group,
    SKATE_OT_create_uv_layers,
    SKATE_OT_grinds_from_sharp_edges,
    SKATE_OT_set_spawn,
    SKATE_OT_validate,
    SKATE_OT_quick_export,
    SKATE_OT_export_dialog,
    SKATE_OT_open_output_folder,
    OWMATERIAL_OT_apply_preset,
    OWMATERIAL_OT_sync_materials,
    OWPHYSICS_OT_hinge_from_cursor,
    OWMATERIAL_PT_material,
    OWPHYSICS_PT_object,
    OWNPCPATH_PT_object,
    SKATE_PT_sidebar,
    SKATE_PT_scene,
)


def register() -> None:
    for cls in CLASSES:
        try:
            bpy.utils.register_class(cls)
        except ValueError:
            pass
    if not hasattr(Material, "owned_world"):
        Material.owned_world = PointerProperty(
            type=OwnedWorldMaterialSettings
        )
    if not hasattr(Object, "owned_world_physics"):
        Object.owned_world_physics = PointerProperty(
            type=OwnedWorldPhysicsSettings
        )
    if not hasattr(Object, "owned_world_npc_path"):
        Object.owned_world_npc_path = PointerProperty(
            type=OwnedWorldNpcPathSettings
        )
    if not hasattr(Scene, "owned_world"):
        Scene.owned_world = PointerProperty(type=OwnedWorldSceneSettings)
    if _load_post not in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.append(_load_post)
    bpy.types.TOPBAR_MT_file_export.append(_file_export_menu)
    _hydrate_all()


def unregister() -> None:
    try:
        bpy.types.TOPBAR_MT_file_export.remove(_file_export_menu)
    except RuntimeError:
        pass
    if _load_post in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.remove(_load_post)
    if hasattr(Scene, "owned_world"):
        del Scene.owned_world
    if hasattr(Object, "owned_world_physics"):
        del Object.owned_world_physics
    if hasattr(Object, "owned_world_npc_path"):
        del Object.owned_world_npc_path
    if hasattr(Material, "owned_world"):
        del Material.owned_world
    for cls in reversed(CLASSES):
        try:
            bpy.utils.unregister_class(cls)
        except RuntimeError:
            pass


if __name__ == "__main__":
    register()
