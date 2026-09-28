"""Headless smoke test for the complete Blender addon workflow."""

from pathlib import Path
import math
import struct
import sys
import tempfile

import bpy
import numpy


TOOL_ROOT = Path(__file__).resolve().parent
if str(TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(TOOL_ROOT))

import owned_world_material_addon as addon
from analyze_skate import VERTEX_BYTES_V12, analyze_package


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def collision_object(
    name: str,
    vertices: list[tuple[float, float, float]],
    faces: list[tuple[int, int, int]],
    material_name: str,
) -> bpy.types.Object:
    mesh = bpy.data.meshes.new(f"{name}Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.data.collections[addon.exporter.COLLISION_COLLECTION].objects.link(obj)
    obj["ow_material"] = material_name
    return obj


def main() -> None:
    addon.register()
    try:
        bpy.ops.object.select_all(action="SELECT")
        bpy.ops.object.delete(use_global=False)
        for collection in list(bpy.data.collections):
            bpy.data.collections.remove(collection)

        require(
            bpy.ops.skate_map.prepare_scene() == {"FINISHED"},
            "Prepare Scene operator failed",
        )

        bpy.ops.mesh.primitive_plane_add(size=20.0, location=(0.0, 0.0, 0.0))
        floor = bpy.context.object
        floor.name = "TestFloor"
        material = bpy.data.materials.new("TestConcrete")
        floor.data.materials.append(material)
        material.owned_world.roughness = 0.67
        floor.owned_world_physics.physics_type = "BOX3D_DYNAMIC"
        floor.owned_world_physics.box3d_collision_shape = "BOX"
        floor.owned_world_physics.box3d_density = 240.0
        floor.owned_world_physics.box3d_friction = 0.72
        floor.owned_world_physics.box3d_restitution = 0.11
        floor.owned_world_physics.box3d_linear_damping = 0.08
        floor.owned_world_physics.box3d_angular_damping = 0.21
        floor.owned_world_physics.box3d_gravity_scale = 0.9

        alpha_image = bpy.data.images.new(
            "RoadMarking_A.tga", width=1, height=1, alpha=True
        )
        alpha_material = bpy.data.materials.new("RoadMarking_A")
        alpha_material["ow_albedo_image"] = alpha_image.name
        alpha_material["ow_alpha_mode"] = 0
        require(
            addon.exporter._effective_alpha_mode(alpha_material) == 1,
            "Alpha-named RGBA material was incorrectly exported opaque",
        )
        alpha_material["ow_force_opaque"] = True
        require(
            addon.exporter._effective_alpha_mode(alpha_material) == 0,
            "Explicit force-opaque override was ignored",
        )
        placeholder_image = bpy.data.images.new(
            "none", width=1, height=1, alpha=True
        )
        alpha_material["ow_normal_image"] = placeholder_image.name
        referenced_images, referenced_image_ids = (
            addon.exporter._referenced_images([alpha_material])
        )
        require(
            placeholder_image not in referenced_images
            and placeholder_image.as_pointer() not in referenced_image_ids
            and addon.exporter._texture_id(
                alpha_material, "ow_normal_image", referenced_image_ids
            )
            == 0,
            "Placeholder 'none' image was exported as a real texture",
        )
        require(
            not addon.exporter._has_nonblank_rgb(bytes((0, 0, 0, 255)))
            and addon.exporter._has_nonblank_rgb(bytes((0, 1, 0, 255))),
            "Package-level blank RGB detection is incorrect",
        )

        uv_only_mesh = bpy.data.meshes.new("UvOnlyRegressionMesh")
        uv_only_mesh.from_pydata(
            [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0)],
            [],
            [(0, 1, 2)],
        )
        uv_only_mesh.update()
        uv_only = uv_only_mesh.uv_layers.new(name="UVMap")
        uv0, uv1 = addon.exporter._visual_uv_layers(
            uv_only_mesh, "UvOnlyRegression"
        )
        require(
            uv0 == uv_only and uv1 == uv_only,
            "UVMap-only unlit mesh did not reuse its primary UV stream",
        )

        def duplicate_surface_mask(opposite_winding: bool):
            mesh = bpy.data.meshes.new("DuplicateSurfaceRegressionMesh")
            mesh.from_pydata(
                [
                    (0.0, 0.0, 0.0),
                    (1.0, 0.0, 0.0),
                    (1.0, 1.0, 0.0),
                    (0.0, 1.0, 0.0),
                    (0.0, 0.0, 0.02),
                    (1.0, 0.0, 0.02),
                    (1.0, 1.0, 0.02),
                    (0.0, 1.0, 0.02),
                ],
                [],
                [
                    (0, 1, 2),
                    (0, 2, 3),
                    (
                        (4, 7, 6)
                        if opposite_winding
                        else (4, 5, 6)
                    ),
                    (
                        (4, 6, 5)
                        if opposite_winding
                        else (4, 6, 7)
                    ),
                ],
            )
            mesh.materials.append(material)
            mesh.materials.append(material)
            mesh.polygons[0].material_index = 0
            mesh.polygons[1].material_index = 0
            mesh.polygons[2].material_index = 1
            mesh.polygons[3].material_index = 1
            mesh.update()
            mesh.calc_loop_triangles()
            positions = numpy.empty(
                len(mesh.vertices) * 3, dtype=numpy.float32
            ).reshape((-1, 3))
            mesh.vertices.foreach_get("co", positions.reshape(-1))
            loop_vertices = numpy.empty(
                len(mesh.loops), dtype=numpy.int32
            )
            mesh.loops.foreach_get("vertex_index", loop_vertices)
            triangle_loops = numpy.empty(
                len(mesh.loop_triangles) * 3, dtype=numpy.int32
            ).reshape((-1, 3))
            triangle_polygons = numpy.empty(
                len(mesh.loop_triangles), dtype=numpy.int32
            )
            mesh.loop_triangles.foreach_get(
                "loops", triangle_loops.reshape(-1)
            )
            mesh.loop_triangles.foreach_get(
                "polygon_index", triangle_polygons
            )
            return addon.exporter._duplicate_visual_surface_keep_mask(
                mesh,
                loop_vertices[triangle_loops],
                triangle_polygons,
                positions,
            )

        require(
            int(duplicate_surface_mask(False).sum()) == 2,
            "Same-facing near-duplicate visual surface was not collapsed",
        )
        require(
            int(duplicate_surface_mask(True).sum()) == 4,
            "Intentional opposite-facing visual surface was removed",
        )

        require(
            bpy.ops.skate_map.assign_selected(role="VISUAL") == {"FINISHED"},
            "Visual assignment failed",
        )
        require(
            bpy.ops.skate_map.assign_selected(role="COLLISION") == {"FINISHED"},
            "Collision assignment failed",
        )
        floor["ow_upward_surface"] = True
        require(
            bpy.ops.skate_map.create_uv_layers() == {"FINISHED"},
            "UV layer helper failed",
        )
        # Imported meshes can retain an empty material slot that no polygon
        # uses. The vectorized exporter must validate referenced slots only,
        # matching the scalar path, instead of rejecting harmless source data.
        bpy.context.view_layer.objects.active = floor
        floor.select_set(True)
        require(
            bpy.ops.object.material_slot_add() == {"FINISHED"},
            "Could not create an unused empty material slot",
        )
        require(
            len(floor.material_slots) == 2
            and floor.material_slots[1].material is None
            and all(polygon.material_index == 0 for polygon in floor.data.polygons),
            "Unused empty material slot regression setup is invalid",
        )

        # Independently editable maps commonly keep a low-detail collision
        # proxy separate from the rendered Blender object. The explicit owner
        # must associate that proxy's triangle range with the visual MOBJ
        # record without requiring both roles to share one Blender object.
        proxy_mesh = bpy.data.meshes.new("ProxyOwnedVisualMesh")
        proxy_mesh.from_pydata(
            [(20.0, 0.0, 0.0), (21.0, 0.0, 0.0), (20.0, 1.0, 0.0)],
            [],
            [(0, 1, 2)],
        )
        proxy_mesh.update()
        proxy_material = material.copy()
        proxy_material.name = "ProxyOwnedMaterial"
        proxy_mesh.materials.append(proxy_material)
        proxy_mesh.uv_layers.new(name="UVMap")
        proxy_mesh.uv_layers.new(name="Lightmap")
        proxy_visual = bpy.data.objects.new(
            "ProxyOwnedVisual", proxy_mesh
        )
        bpy.data.collections[
            addon.exporter.NO_COLLISION_COLLECTION
        ].objects.link(proxy_visual)
        proxy_collision = collision_object(
            "ProxyOwnedCollision",
            [(20.0, 0.0, 0.0), (21.0, 0.0, 0.0), (20.0, 1.0, 0.0)],
            [(0, 1, 2)],
            proxy_material.name,
        )
        proxy_owner = proxy_collision.data.attributes.new(
            addon.exporter.EDITOR_COLLISION_OWNER_ATTRIBUTE,
            type="INT",
            domain="FACE",
        )
        proxy_owner_id = int(
            addon.exporter._stable_object_id(proxy_visual.name_full)
        )
        proxy_owner.data[0].value = (
            proxy_owner_id
            if proxy_owner_id < 0x80000000
            else proxy_owner_id - 0x100000000
        )

        # Retail imports carry their authored frame as hidden point
        # attributes. Verify that it takes precedence over Blender tangent
        # generation while the ordinary TestFloor above still exercises the
        # custom-map fallback.
        retail_mesh = bpy.data.meshes.new("RetailFrameMesh")
        retail_mesh.from_pydata(
            [(100.0, 0.0, 0.0), (101.0, 0.0, 0.0), (100.0, 1.0, 0.0)],
            [],
            [(0, 1, 2)],
        )
        retail_mesh.update()
        retail_mesh.materials.append(material)
        retail_mesh.uv_layers.new(name="UVMap")
        retail_mesh.uv_layers.new(name="Lightmap")
        retail_normal = retail_mesh.attributes.new(
            addon.exporter.RETAIL_NORMAL_ATTRIBUTE,
            type="FLOAT_VECTOR",
            domain="POINT",
        )
        retail_normal.data.foreach_set(
            "vector", (0.0, 0.0, 1.0) * 3
        )
        retail_tangent = retail_mesh.attributes.new(
            addon.exporter.RETAIL_TANGENT_ATTRIBUTE,
            type="FLOAT_VECTOR",
            domain="POINT",
        )
        retail_tangent.data.foreach_set(
            "vector", (1.0, 0.0, 0.0) * 3
        )
        retail_handedness = retail_mesh.attributes.new(
            addon.exporter.RETAIL_HANDEDNESS_ATTRIBUTE,
            type="FLOAT",
            domain="POINT",
        )
        retail_handedness.data.foreach_set("value", (-1.0,) * 3)
        retail_frame = bpy.data.objects.new("RetailFrame", retail_mesh)
        retail_frame["ow_physics_type"] = "PRESENTATION_ONLY"
        retail_frame["ow_editor_editable"] = False
        bpy.data.collections[
            addon.exporter.VISUAL_COLLECTION
        ].objects.link(retail_frame)

        # Old add-on versions could leave imported scale-reference meshes in
        # both export collections. Existing scenes must ignore them without
        # requiring the author to repair collection membership manually.
        helper_mesh = bpy.data.meshes.new("character_size_mesh")
        helper_mesh.from_pydata(
            [(0.0, 0.0, 0.0), (0.0, 0.0, 2.0), (0.25, 0.0, 0.0)],
            [],
            [(0, 1, 2)],
        )
        helper_mesh.update()
        helper = bpy.data.objects.new("character_size", helper_mesh)
        bpy.data.collections[
            addon.exporter.VISUAL_COLLECTION
        ].objects.link(helper)
        bpy.data.collections[
            addon.exporter.COLLISION_COLLECTION
        ].objects.link(helper)

        cleanup_proxy = collision_object(
            "CollisionCleanupRegression",
            [
                (30.0, 0.0, 0.0),
                (31.0, 0.0, 0.0),
                (30.0, 1.0, 0.0),
                # A separate vertex set with identical positions verifies
                # position-based duplicate cleanup.
                (30.0, 0.0, 0.0),
                (31.0, 0.0, 0.0),
                (30.0, 1.0, 0.0),
                # Collinear source geometry must be skipped without editing
                # this mesh or any visual UVs.
                (35.0, 0.0, 0.0),
                (36.0, 0.0, 0.0),
                (37.0, 0.0, 0.0),
            ],
            [(0, 1, 2), (3, 4, 5), (6, 7, 8)],
            material.name,
        )

        retail_two_sided = collision_object(
            "RetailTwoSidedRegression",
            [
                (50.0, 0.0, 0.0),
                (51.0, 0.0, 0.0),
                (50.0, 1.0, 0.0),
            ],
            [(0, 1, 2), (2, 1, 0)],
            material.name,
        )
        retail_two_sided["ow_preserve_opposite_wound_collision"] = True
        retail_two_sided["ow_preserve_retail_edge_codes"] = True
        expected_retail_codes = (
            (26, 90, 98),
            (34, 88, 122),
        )
        for corner, attribute_name in enumerate(
            addon.exporter.RETAIL_EDGE_CODE_ATTRIBUTES
        ):
            attribute = retail_two_sided.data.attributes.new(
                attribute_name,
                type="INT",
                domain="FACE",
            )
            attribute.data.foreach_set(
                "value",
                [codes[corner] for codes in expected_retail_codes],
            )
        retail_triangles, retail_audit = (
            addon.exporter.audit_collision_geometry([retail_two_sided])
        )
        require(
            len(retail_triangles) == 2
            and retail_audit.skipped_duplicates == 0
            and tuple(
                triangle[-1] for triangle in retail_triangles
            )
            == expected_retail_codes,
            "Retail reverse-wound collision metadata was discarded",
        )

        downward_a = collision_object(
            "WrongFacingA",
            [(40.0, 0.0, 0.0), (40.0, 1.0, 0.0), (41.0, 0.0, 0.0)],
            [(0, 1, 2)],
            material.name,
        )
        downward_b = collision_object(
            "WrongFacingB",
            [(45.0, 0.0, 0.0), (45.0, 1.0, 0.0), (46.0, 0.0, 0.0)],
            [(0, 1, 2)],
            material.name,
        )
        downward_a["ow_upward_surface"] = True
        downward_b["ow_upward_surface"] = True
        _triangles, wrong_facing_audit = (
            addon.exporter.audit_collision_geometry(
                [downward_a, downward_b]
            )
        )
        require(
            sum(
                "face downward or vertically" in issue
                for issue in wrong_facing_audit.issues
            )
            == 2,
            "Collision audit did not report every bad mesh in one pass",
        )
        for obj in (downward_a, downward_b):
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            bpy.data.meshes.remove(mesh)

        scene = bpy.context.scene
        point_data = bpy.data.lights.new("TestPointData", type="POINT")
        point_data.color = (0.15, 0.45, 1.0)
        point_data.energy = 1200.0
        point_data.cutoff_distance = 12.0
        point = bpy.data.objects.new("TestPoint", point_data)
        scene.collection.objects.link(point)
        point.location = (-2.0, 0.0, 3.0)

        spot_data = bpy.data.lights.new("TestSpotData", type="SPOT")
        spot_data.color = (1.0, 0.18, 0.08)
        spot_data.energy = 1600.0
        spot_data.cutoff_distance = 16.0
        spot_data.spot_size = 0.9
        spot_data.spot_blend = 0.35
        spot = bpy.data.objects.new("TestSpot", spot_data)
        scene.collection.objects.link(spot)
        spot.location = (2.0, -2.0, 4.0)

        area_data = bpy.data.lights.new("TestAreaData", type="AREA")
        area_data.color = (0.35, 1.0, 0.25)
        area_data.energy = 900.0
        area_data.cutoff_distance = 10.0
        area_data.shape = "RECTANGLE"
        area_data.size = 2.0
        area_data.size_y = 1.0
        area = bpy.data.objects.new("TestArea", area_data)
        scene.collection.objects.link(area)
        area.location = (0.0, 3.0, 3.0)
        area.rotation_euler = (0.35, 0.0, math.pi)

        sun_data = bpy.data.lights.new("TestSunData", type="SUN")
        sun_data.color = (1.0, 0.72, 0.42)
        sun_data.energy = 2.5
        sun = bpy.data.objects.new("TestSun", sun_data)
        scene.collection.objects.link(sun)
        sun.rotation_euler = (0.7, 0.0, -0.5)

        scene.cursor.location = (0.0, 0.0, 1.0)
        require(
            bpy.ops.skate_map.set_spawn() == {"FINISHED"},
            "Spawn helper failed",
        )

        output = Path(tempfile.gettempdir()) / "addon_workflow_test.skate"
        cache = output.with_name(output.name + ".export-cache.json")
        for path in (output, cache):
            path.unlink(missing_ok=True)

        settings = scene.owned_world
        settings.map_name = "Addon Workflow Test"
        settings.output_path = str(output)
        settings.cycle_seconds = 30.0
        settings.cycle_ping_pong = True
        settings.start_hour = 8.0
        settings.end_hour = 18.0
        settings.dynamic_lighting_enabled_by_default = False

        # AI routes are experimental and optional. Their collection must not
        # be required for validation or export.
        npc_collection = bpy.data.collections.get(
            addon.exporter.NPC_PATH_COLLECTION
        )
        require(npc_collection is not None, "Prepare Scene missed NPC paths")
        bpy.data.collections.remove(npc_collection)
        grind_collection = bpy.data.collections.get(
            addon.exporter.GRIND_COLLECTION
        )
        require(grind_collection is not None, "Prepare Scene missed grinds")
        bpy.data.collections.remove(grind_collection)

        require(
            bpy.ops.skate_map.validate() == {"FINISHED"},
            settings.validation_details or "Validation failed",
        )
        require(
            "zero-area collision triangle" in settings.validation_details
            and "duplicate collision triangle" in settings.validation_details,
            "Validation did not explain automatic collision cleanup",
        )
        require(
            "OW_NPC_PATHS" not in settings.validation_details,
            "Optional NPC paths were reported as an export problem",
        )
        require(
            "OW_GRIND" not in settings.validation_details,
            "Optional grind collection was reported as an export problem",
        )
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            settings.last_status,
        )
        require(output.is_file(), "Quick Export did not create an SKATE")
        require(cache.is_file(), "Quick Export did not create its cache")
        require(
            output.read_bytes()[:8] == b"SKATE15\0",
            "Exported package has the wrong magic",
        )
        analysis = analyze_package(output, include_payloads=True)
        require(
            analysis["dynamic_lighting_enabled_by_default"] is False,
            "Full export did not store the map's dynamic-lighting default",
        )
        require(
            "MOBJ" in analysis["extension_tags"],
            "Exported package did not preserve Blender object records",
        )
        object_records = analysis["map_objects"]
        require(
            [record["name"] for record in object_records]
            == ["TestFloor", "ProxyOwnedVisual"],
            "Static Blender objects leaked into editable object records",
        )
        require(
            analysis["counts"]["indices"]
            >= sum(record["index_count"] for record in object_records)
            and analysis["counts"]["collision_triangles"]
            >= sum(
                record["collision_count"] for record in object_records
            ),
            "MOBJ parsing overwrote the package-level geometry counts",
        )
        require(
            len({record["id"] for record in object_records}) == 2
            and all(record["id"] != 0 for record in object_records),
            "Blender object IDs are zero or not unique",
        )
        require(
            object_records[0]["index_count"] == 6
            and object_records[0]["collision_count"] == 2,
            "Visual/collision ownership was not associated with TestFloor",
        )
        require(
            object_records[0]["physics"]["type"] == 2
            and object_records[0]["physics"]["shape"] == 0
            and abs(object_records[0]["physics"]["density"] - 240.0)
            < 1.0e-5
            and abs(object_records[0]["physics"]["friction"] - 0.72)
            < 1.0e-5
            and abs(object_records[0]["physics"]["restitution"] - 0.11)
            < 1.0e-5
            and abs(
                object_records[0]["physics"]["linear_damping"] - 0.08
            )
            < 1.0e-5
            and abs(
                object_records[0]["physics"]["angular_damping"] - 0.21
            )
            < 1.0e-5
            and abs(object_records[0]["physics"]["gravity_scale"] - 0.9)
            < 1.0e-5,
            "Box3D object metadata did not survive export and analysis",
        )
        proxy_record = next(
            record
            for record in object_records
            if record["name"] == "ProxyOwnedVisual"
        )
        require(
            proxy_record["index_count"] == 3
            and proxy_record["collision_count"] == 1,
            "Face-level collision ownership was not associated with its "
            "visual MOBJ owner",
        )
        require(
            proxy_record["physics"]["type"] == 0,
            "Physics was not disabled by default for an ordinary object",
        )
        retail_frame_records = []
        vertex_bytes = analysis["_vertex_bytes"]
        for offset in range(
            0,
            len(vertex_bytes),
            VERTEX_BYTES_V12,
        ):
            position = struct.unpack_from("<3f", vertex_bytes, offset)
            if position[0] < 90.0:
                continue
            retail_frame_records.append(
                (
                    struct.unpack_from("<3f", vertex_bytes, offset + 12),
                    struct.unpack_from("<4b", vertex_bytes, offset + 52),
                )
            )
        require(
            retail_frame_records
            and all(
                normal == (0.0, 1.0, 0.0)
                and frame == (0, 0, 127, -127)
                for normal, frame in retail_frame_records
            ),
            "Retail tangent did not reconstruct the expected runtime binormal",
        )
        collision_bytes = analysis["_collision_bytes"]
        native_codes = []
        for offset in range(0, len(collision_bytes), 48):
            record = struct.unpack_from("<9fII4B", collision_bytes, offset)
            if record[14] == 1:
                native_codes.append(tuple(record[11:14]))
        require(
            tuple(native_codes) == expected_retail_codes,
            "SKATE package changed native retail collision edge codes",
        )
        _, _, counts = addon.exporter._read_package_header(output)
        collision_triangle_count = counts[4]
        local_light_count = counts[-2]
        npc_route_count = counts[-1]
        require(
            collision_triangle_count == 7,
            "Degenerate or duplicate collision reached the package "
            f"(got {collision_triangle_count}, expected 7)",
        )
        require(
            local_light_count == 3,
            "Point, Spot, and Area lights were not exported",
        )
        require(npc_route_count == 0, "Unexpected NPC routes were exported")
        require(
            abs(float(material.get("ow_roughness", 0.0)) - 0.67) < 1.0e-5,
            "Material panel did not synchronize exporter metadata",
        )
        require(
            settings.last_status.startswith("Exported successfully"),
            "Friendly export result was not retained",
        )
        require(
            "No mesh dissolve or UV changes are required"
            in settings.validation_details,
            "Quick Export did not retain collision cleanup guidance",
        )
        settings.dynamic_lighting_enabled_by_default = True
        settings.export_mode = "METADATA"
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            "Lighting-only export did not patch the dynamic-lighting default",
        )
        lighting_only = analyze_package(output)
        require(
            lighting_only["dynamic_lighting_enabled_by_default"] is True,
            "Lighting-only export left a stale dynamic-lighting default",
        )
        settings.export_mode = "AUTO"
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            "Incremental export failed after collision sanitation",
        )
        _, _, cached_counts = addon.exporter._read_package_header(output)
        require(
            cached_counts == counts,
            "Incremental cache changed sanitized package counts",
        )

        # Authors can export a finished map with no selectable existing
        # objects. Rendering, collision, materials, and other package counts
        # must stay intact, while both face-level and proxy ownership metadata
        # are ignored and the MOBJ table becomes empty.
        settings.export_editable_objects = False
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            "Non-editable map export failed",
        )
        non_editable = analyze_package(output, include_payloads=True)
        require(
            "MOBJ" in non_editable["extension_tags"]
            and not non_editable["map_objects"],
            "Non-editable export retained selectable map object records",
        )
        require(
            non_editable["counts"] == analysis["counts"],
            "Non-editable export changed visual, collision, or feature counts",
        )
        require(
            non_editable["_collision_bytes"],
            "Non-editable export discarded static collision",
        )

        # The global mode participates in the content fingerprint. Switching
        # it back on must rebuild the object table instead of accepting the
        # non-editable package as an incremental cache hit.
        settings.export_editable_objects = True
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            "Re-enabling editable map objects failed",
        )
        editable_again = analyze_package(output)
        require(
            [record["name"] for record in editable_again["map_objects"]]
            == ["TestFloor", "ProxyOwnedVisual"],
            "Editable object records were not restored after changing mode",
        )

        blend_path = output.with_suffix(".blend")
        bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))
        require(blend_path.is_file(), "Workflow test scene was not saved")
        print(
            "ADDON_WORKFLOW_OK",
            output,
            output.stat().st_size,
            settings.last_status,
        )
    finally:
        addon.unregister()


if __name__ == "__main__":
    try:
        main()
    except Exception:
        import traceback

        traceback.print_exc()
        sys.exit(1)
