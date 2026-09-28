"""Headless regression test for exporting an ordinary unconfigured .blend."""

from pathlib import Path
import math
import sys
import tempfile

import bpy


TOOL_ROOT = Path(__file__).resolve().parent
if str(TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(TOOL_ROOT))

import owned_world_material_addon as addon
from analyze_skate import analyze_package
from compare_skate import compare_packages


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def make_plane(
    name: str,
    x_offset: float,
    material: bpy.types.Material | None = None,
) -> bpy.types.Object:
    mesh = bpy.data.meshes.new(f"{name}Mesh")
    mesh.from_pydata(
        [
            (x_offset - 1.0, -1.0, 0.0),
            (x_offset + 1.0, -1.0, 0.0),
            (x_offset + 1.0, 1.0, 0.0),
            (x_offset - 1.0, 1.0, 0.0),
        ],
        [],
        [(0, 1, 2, 3)],
    )
    mesh.update()
    if material is not None:
        mesh.materials.append(material)
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def main() -> None:
    addon.register()
    try:
        bpy.ops.object.select_all(action="SELECT")
        bpy.ops.object.delete(use_global=False)
        for collection in list(bpy.data.collections):
            bpy.data.collections.remove(collection)
        for material in list(bpy.data.materials):
            bpy.data.materials.remove(material)

        image = bpy.data.images.new(
            "AutoImportAlbedo", width=2, height=2, alpha=True
        )
        image.pixels = [
            0.8, 0.2, 0.1, 1.0,
            0.8, 0.2, 0.1, 1.0,
            0.8, 0.2, 0.1, 1.0,
            0.8, 0.2, 0.1, 1.0,
        ]
        image.pack()
        require(
            image.packed_file is not None,
            "Packed-image cache regression setup failed",
        )
        stray_image = bpy.data.images.new(
            "Road_ORM_But_Unconnected", width=1, height=1, alpha=True
        )
        deterministic_material = bpy.data.materials.new(
            "DeterministicTextureRoles"
        )
        deterministic_material.use_nodes = True
        stray_node = deterministic_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        stray_node.image = stray_image
        addon._auto_configure_material(
            deterministic_material, generate_maps=True
        )
        require(
            not deterministic_material.get("ow_albedo_image", "")
            and not deterministic_material.get("ow_orm_image", ""),
            "An unconnected texture was assigned from its filename",
        )
        height_image = bpy.data.images.new(
            "HeightIsNotNormal", width=2, height=2, alpha=False
        )
        bump_material = bpy.data.materials.new("BumpCompatibility")
        bump_material.use_nodes = True
        bump_shader = next(
            node
            for node in bump_material.node_tree.nodes
            if node.type == "BSDF_PRINCIPLED"
        )
        bump = bump_material.node_tree.nodes.new("ShaderNodeBump")
        height = bump_material.node_tree.nodes.new("ShaderNodeTexImage")
        height.image = height_image
        bump_material.node_tree.links.new(
            height.outputs["Color"], bump.inputs["Height"]
        )
        bump_material.node_tree.links.new(
            bump.outputs["Normal"], bump_shader.inputs["Normal"]
        )
        addon._auto_configure_material(
            bump_material, generate_maps=True
        )
        require(
            not bump_material.get("ow_normal_image", ""),
            "A scalar Blender Bump height map was mislabelled as an RGB "
            "tangent normal map",
        )
        procedural_emission = bpy.data.materials.new(
            "UnsupportedBlackProceduralEmission"
        )
        procedural_emission.use_nodes = True
        procedural_shader = next(
            node
            for node in procedural_emission.node_tree.nodes
            if node.type == "BSDF_PRINCIPLED"
        )
        black_rgb = procedural_emission.node_tree.nodes.new("ShaderNodeRGB")
        black_rgb.outputs["Color"].default_value = (0.0, 0.0, 0.0, 1.0)
        procedural_emission.node_tree.links.new(
            black_rgb.outputs["Color"],
            procedural_shader.inputs["Emission Color"],
        )
        procedural_shader.inputs["Emission Strength"].default_value = 1.0
        addon._auto_configure_material(
            procedural_emission, generate_maps=True
        )
        procedural_emissive_value = float(
            procedural_emission.get("ow_emissive", -1.0)
        )
        procedural_emissive_image = str(
            procedural_emission.get("ow_emissive_image", "")
        )
        require(
            abs(procedural_emissive_value) < 1.0e-6
            and not procedural_emissive_image,
            "Unsupported black procedural emission generated a black texture "
            f"(strength={procedural_emissive_value}, "
            f"image={procedural_emissive_image!r})",
        )
        material = bpy.data.materials.new("Concrete_AutoImport")
        material.use_nodes = True
        shader = next(
            node
            for node in material.node_tree.nodes
            if node.type == "BSDF_PRINCIPLED"
        )
        shader.inputs["Roughness"].default_value = 0.63
        shader.inputs["Metallic"].default_value = 0.24
        texture_group = bpy.data.node_groups.new(
            "ImportedAlbedoNodeGroup", "ShaderNodeTree"
        )
        texture_group.interface.new_socket(
            name="Color",
            in_out="OUTPUT",
            socket_type="NodeSocketColor",
        )
        group_output = texture_group.nodes.new("NodeGroupOutput")
        texture = texture_group.nodes.new("ShaderNodeTexImage")
        texture.image = image
        explicit_uv = texture_group.nodes.new("ShaderNodeUVMap")
        explicit_uv.uv_map = "TEXCOORD"
        mapping = texture_group.nodes.new("ShaderNodeMapping")
        mapping.vector_type = "POINT"
        mapping.inputs["Scale"].default_value = (2.0, 3.0, 1.0)
        mapping.inputs["Location"].default_value = (0.25, -0.5, 0.0)
        texture_group.links.new(
            explicit_uv.outputs["UV"], mapping.inputs["Vector"]
        )
        texture_group.links.new(
            mapping.outputs["Vector"], texture.inputs["Vector"]
        )
        texture_group.links.new(
            texture.outputs["Color"], group_output.inputs["Color"]
        )
        group_node = material.node_tree.nodes.new("ShaderNodeGroup")
        group_node.node_tree = texture_group
        material.node_tree.links.new(
            group_node.outputs["Color"], shader.inputs["Base Color"]
        )

        nested_image = bpy.data.images.new(
            "NestedShaderAlbedo", width=2, height=2, alpha=True
        )
        nested_image.pixels = [0.2, 0.5, 0.9, 1.0] * 4
        nested_image.pack()
        nested_tree = bpy.data.node_groups.new(
            "ImportedNestedShader", "ShaderNodeTree"
        )
        nested_tree.interface.new_socket(
            name="TextureColor",
            in_out="INPUT",
            socket_type="NodeSocketColor",
        )
        nested_tree.interface.new_socket(
            name="BSDF",
            in_out="OUTPUT",
            socket_type="NodeSocketShader",
        )
        nested_input = nested_tree.nodes.new("NodeGroupInput")
        nested_output = nested_tree.nodes.new("NodeGroupOutput")
        nested_shader = nested_tree.nodes.new("ShaderNodeBsdfPrincipled")
        nested_tree.links.new(
            nested_input.outputs["TextureColor"],
            nested_shader.inputs["Base Color"],
        )
        nested_tree.links.new(
            nested_shader.outputs["BSDF"], nested_output.inputs["BSDF"]
        )
        nested_material = bpy.data.materials.new("NestedImportedShader")
        nested_material.use_nodes = True
        nested_material.node_tree.nodes.clear()
        nested_material_output = nested_material.node_tree.nodes.new(
            "ShaderNodeOutputMaterial"
        )
        nested_group = nested_material.node_tree.nodes.new(
            "ShaderNodeGroup"
        )
        nested_group.node_tree = nested_tree
        nested_texture = nested_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        nested_texture.image = nested_image
        nested_material.node_tree.links.new(
            nested_texture.outputs["Color"],
            nested_group.inputs["TextureColor"],
        )
        nested_material.node_tree.links.new(
            nested_group.outputs["BSDF"],
            nested_material_output.inputs["Surface"],
        )
        addon._auto_configure_material(
            nested_material, generate_maps=True
        )
        require(
            nested_material.get("ow_albedo_image") == nested_image.name,
            "An image feeding a Principled shader through a group input was "
            "not resolved from the connected Blender graph",
        )

        layer_small = bpy.data.images.new(
            "LayeredGroundSmallColor", width=2, height=2, alpha=True
        )
        layer_large = bpy.data.images.new(
            "LayeredGroundLargeColor", width=4, height=4, alpha=True
        )
        layer_mask = bpy.data.images.new(
            "LayeredGroundHeightMask", width=8, height=8, alpha=True
        )
        for layer_image, colour in (
            (layer_small, (0.15, 0.25, 0.35, 1.0)),
            (layer_large, (0.55, 0.45, 0.25, 1.0)),
            (layer_mask, (0.25, 0.25, 0.25, 1.0)),
        ):
            layer_image.pixels = (
                list(colour) * (layer_image.size[0] * layer_image.size[1])
            )
            layer_image.pack()
            layer_image.use_fake_user = True
        layered_material = bpy.data.materials.new("LayeredGround")
        layered_material.use_nodes = True
        layered_shader = next(
            node
            for node in layered_material.node_tree.nodes
            if node.type == "BSDF_PRINCIPLED"
        )
        mix = layered_material.node_tree.nodes.new("ShaderNodeMix")
        mix.data_type = "RGBA"
        small_node = layered_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        small_node.image = layer_small
        large_node = layered_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        large_node.image = layer_large
        mask_node = layered_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        mask_node.image = layer_mask
        mix_factor = next(
            socket
            for socket in mix.inputs
            if socket.name == "Factor"
            and socket.identifier.endswith("_Float")
        )
        mix_a = next(
            socket
            for socket in mix.inputs
            if socket.name == "A"
            and socket.identifier.endswith("_Color")
        )
        mix_b = next(
            socket
            for socket in mix.inputs
            if socket.name == "B"
            and socket.identifier.endswith("_Color")
        )
        mix_result = next(
            socket
            for socket in mix.outputs
            if socket.name == "Result"
            and socket.identifier.endswith("_Color")
        )
        layered_material.node_tree.links.new(
            mask_node.outputs["Color"], mix_factor
        )
        layered_material.node_tree.links.new(
            small_node.outputs["Color"], mix_a
        )
        layered_material.node_tree.links.new(
            large_node.outputs["Color"], mix_b
        )
        layered_material.node_tree.links.new(
            mix_result, layered_shader.inputs["Base Color"]
        )
        addon._auto_configure_material(
            layered_material, generate_maps=True
        )
        require(
            layered_material.get("ow_albedo_image") == layer_small.name
            and layered_material.get("ow_secondary_albedo_image")
            == layer_large.name
            and layered_material.get("ow_blend_mask_image")
            == layer_mask.name,
            "A supported Blender Mix material did not retain both visible "
            "colour layers and its factor texture",
        )
        require(
            int(layered_material.get("ow_blend_mask_channel", -1)) == 0,
            "A colour factor texture did not retain luminance semantics",
        )

        complex_image = bpy.data.images.new(
            "ObjectTintComplexAlbedo", width=2, height=2, alpha=True
        )
        complex_image.pixels = [
            0.4, 0.5, 0.6, 0.0,
            0.4, 0.5, 0.6, 1.0,
            0.4, 0.5, 0.6, 0.25,
            0.4, 0.5, 0.6, 0.75,
        ]
        complex_image.pack()
        complex_material = bpy.data.materials.new("ObjectTintComplex")
        complex_material.use_nodes = True
        complex_shader = next(
            node
            for node in complex_material.node_tree.nodes
            if node.type == "BSDF_PRINCIPLED"
        )
        # A Base Color bake must capture the colour graph itself. Cycles'
        # diffuse pass turns a fully metallic Principled surface black.
        complex_shader.inputs["Metallic"].default_value = 1.0
        complex_texture = complex_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        complex_texture.image = complex_image
        object_info = complex_material.node_tree.nodes.new(
            "ShaderNodeObjectInfo"
        )
        multiply = complex_material.node_tree.nodes.new(
            "ShaderNodeMixRGB"
        )
        multiply.blend_type = "MULTIPLY"
        multiply.inputs["Fac"].default_value = 1.0
        vertex_color = complex_material.node_tree.nodes.new(
            "ShaderNodeVertexColor"
        )
        vertex_color.layer_name = "TINT"
        vertex_multiply = complex_material.node_tree.nodes.new(
            "ShaderNodeMixRGB"
        )
        vertex_multiply.blend_type = "MULTIPLY"
        vertex_multiply.inputs["Fac"].default_value = 1.0
        complex_material.node_tree.links.new(
            complex_texture.outputs["Color"], multiply.inputs["Color1"]
        )
        complex_material.node_tree.links.new(
            object_info.outputs["Color"], multiply.inputs["Color2"]
        )
        complex_material.node_tree.links.new(
            multiply.outputs["Color"], vertex_multiply.inputs["Color1"]
        )
        complex_material.node_tree.links.new(
            vertex_color.outputs["Color"],
            vertex_multiply.inputs["Color2"],
        )
        complex_material.node_tree.links.new(
            vertex_multiply.outputs["Color"],
            complex_shader.inputs["Base Color"],
        )
        complex_material.node_tree.links.new(
            complex_texture.outputs["Alpha"],
            complex_shader.inputs["Alpha"],
        )
        addon._auto_configure_material(
            complex_material, generate_maps=True
        )
        require(
            bool(
                complex_material.get(
                    "ow_requires_base_color_bake", False
                )
            )
            and bool(
                complex_material.get("ow_uses_object_color", False)
            ),
            "Unsupported Base Color graph was not marked for flattening or "
            "its Object Info colour dependency was lost",
        )

        packed_mask_image = bpy.data.images.new(
            "OpaqueColorWithPackedAlpha", width=2, height=2, alpha=True
        )
        packed_mask_image.pixels = [
            0.4, 0.5, 0.6, 0.0,
            0.4, 0.5, 0.6, 1.0,
            0.4, 0.5, 0.6, 0.25,
            0.4, 0.5, 0.6, 0.75,
        ]
        packed_mask_material = bpy.data.materials.new(
            "OpaquePackedAlpha"
        )
        packed_mask_material.use_nodes = True
        packed_mask_shader = next(
            node
            for node in packed_mask_material.node_tree.nodes
            if node.type == "BSDF_PRINCIPLED"
        )
        packed_mask_node = packed_mask_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        packed_mask_node.image = packed_mask_image
        packed_mask_material.node_tree.links.new(
            packed_mask_node.outputs["Color"],
            packed_mask_shader.inputs["Base Color"],
        )
        addon._auto_configure_material(
            packed_mask_material, generate_maps=True
        )
        require(
            int(packed_mask_material.get("ow_alpha_mode", -1)) == 0
            and not packed_mask_shader.inputs["Alpha"].links,
            "Packed albedo alpha was incorrectly invented as transparency",
        )

        glass_material = bpy.data.materials.new("PhysicalGlassFallback")
        glass_material.use_nodes = True
        glass_tree = glass_material.node_tree
        glass_output = next(
            node for node in glass_tree.nodes if node.type == "OUTPUT_MATERIAL"
        )
        for link in list(glass_output.inputs["Surface"].links):
            glass_tree.links.remove(link)
        glass_shader = glass_tree.nodes.new("ShaderNodeBsdfGlass")
        glass_shader.inputs["Color"].default_value = (
            0.2,
            0.55,
            0.8,
            1.0,
        )
        glass_shader.inputs["Roughness"].default_value = 0.1
        glass_object_info = glass_tree.nodes.new("ShaderNodeObjectInfo")
        glass_tree.links.new(
            glass_object_info.outputs["Color"],
            glass_shader.inputs["Color"],
        )
        glass_tree.links.new(
            glass_shader.outputs["BSDF"],
            glass_output.inputs["Surface"],
        )
        addon._auto_configure_material(
            glass_material, generate_maps=True
        )
        glass_image = bpy.data.images.get(
            str(glass_material.get("ow_albedo_image", ""))
        )
        require(
            glass_image is not None
            and glass_image.get("ow_generated_map") == "GLASS"
            and int(glass_material.get("ow_alpha_mode", -1)) == 2,
            "A physical Blender Glass BSDF did not receive the basic "
            "transparent runtime fallback",
        )
        glass_pixels = list(glass_image.pixels[:])
        require(
            all(
                abs(glass_pixels[index + 3] - 0.32) < 0.01
                for index in range(0, len(glass_pixels), 4)
            )
            and min(glass_pixels[0::4]) > 0.1
            and min(glass_pixels[2::4]) > 0.5,
            "The basic Glass BSDF fallback lost its tint or opacity",
        )
        require(
            tuple(glass_material.get("ow_display_color", ()))
            == (1.0, 1.0, 1.0)
            and not bool(
                glass_material.get("ow_uses_object_color", True)
            ),
            "Unsupported custom Glass BSDF tinting leaked into the neutral "
            "runtime fallback",
        )

        floor = make_plane("OrdinaryVisibleFloor", 0.0, material)
        make_plane("LayeredGroundPatch", 3.0, layered_material)
        complex_white = make_plane(
            "ObjectTintComplexWhite", 13.0, complex_material
        )
        complex_blue = make_plane(
            "ObjectTintComplexBlue", 16.0, complex_material
        )
        complex_white.color = (1.0, 1.0, 1.0, 1.0)
        complex_blue.color = (0.25, 0.5, 0.75, 1.0)
        make_plane("PhysicalGlassPanel", 19.0, glass_material)
        for obj, minimum_u, maximum_u in (
            (complex_white, 0.0, 0.5),
            (complex_blue, 0.5, 1.0),
        ):
            tint = obj.data.color_attributes.new(
                name="TINT",
                type="FLOAT_COLOR",
                domain="CORNER",
            )
            for item in tint.data:
                item.color = (1.0, 1.0, 1.0, 1.0)
            uv = obj.data.uv_layers.new(name="UVMap")
            for item, coordinate in zip(
                uv.data,
                (
                    (minimum_u, 0.0),
                    (maximum_u, 0.0),
                    (maximum_u, 1.0),
                    (minimum_u, 1.0),
                ),
                strict=True,
            ):
                item.uv = coordinate
        imported_uv = floor.data.uv_layers.new(name="TEXCOORD")
        imported_uv.data[0].uv = (-1.7014118e38, 0.0)
        cutout_image = bpy.data.images.new(
            "ImportedLeafCutout", width=2, height=2, alpha=True
        )
        cutout_image.pixels = [
            0.1, 0.6, 0.2, 0.0,
            0.1, 0.6, 0.2, 1.0,
            0.1, 0.6, 0.2, 0.0,
            0.1, 0.6, 0.2, 1.0,
        ]
        cutout_image.pack()
        cutout_material = bpy.data.materials.new("ImportedLeaf")
        cutout_material.use_nodes = True
        cutout_shader = next(
            node
            for node in cutout_material.node_tree.nodes
            if node.type == "BSDF_PRINCIPLED"
        )
        cutout_texture = cutout_material.node_tree.nodes.new(
            "ShaderNodeTexImage"
        )
        cutout_texture.image = cutout_image
        cutout_material.node_tree.links.new(
            cutout_texture.outputs["Color"],
            cutout_shader.inputs["Base Color"],
        )
        cutout_material.node_tree.links.new(
            cutout_texture.outputs["Alpha"],
            cutout_shader.inputs["Alpha"],
        )
        # Simulate a scene prepared by an older addon: Skate semantics exist,
        # but there is no importer-owned or field-authored marker yet.
        cutout_material["ow_audio_surface"] = 55
        cutout_material["ow_physics_surface"] = 3
        cutout_material["ow_alpha_mode"] = 0
        leaf_card = make_plane("ImportedLeafCard", 3.0, cutout_material)
        shared_leaf_card = make_plane(
            "ImportedLeafCard_AlternateRole", 4.0, cutout_material
        )
        floor.location = (1.25, -0.75, 0.5)
        floor.rotation_euler = (0.1, 0.2, 0.3)
        floor.scale = (1.2, 0.8, 1.1)
        solidify = floor.modifiers.new("ExportedSolidify", "SOLIDIFY")
        solidify.thickness = 0.2
        collider = make_plane("FloorCollider", 5.0)
        grass_material = bpy.data.materials.new("Soft_Grass_Planter")
        grass_planter = make_plane(
            "Soft_Grass_Planter_Block", 7.5, grass_material
        )
        foliage_material = bpy.data.materials.new("Dense_Shrub_Foliage")
        foliage = make_plane(
            "Dense_Shrub_Foliage_Decoration", 9.0, foliage_material
        )
        scale_reference = make_plane("character_size", 10.0)
        lod_parent = bpy.data.objects.new("ImportedTree", None)
        bpy.context.scene.collection.objects.link(lod_parent)
        lod0 = make_plane("ImportedTree_LOD0", 11.0, material)
        lod1 = make_plane("ImportedTree_LOD1", 11.0, material)
        lod0.parent = lod_parent
        lod1.parent = lod_parent
        legacy_visual = bpy.data.collections.new(
            addon.exporter.LEGACY_VISUAL_COLLECTION
        )
        legacy_collision = bpy.data.collections.new(
            addon.exporter.LEGACY_COLLISION_COLLECTION
        )
        bpy.context.scene.collection.children.link(legacy_visual)
        bpy.context.scene.collection.children.link(legacy_collision)
        legacy_visual.objects.link(floor)
        legacy_collision.objects.link(floor)
        legacy_collision.objects.link(collider)

        fake_light = bpy.data.objects.new("Point Light Imported Empty", None)
        bpy.context.scene.collection.objects.link(fake_light)

        light_data = bpy.data.lights.new("RealPointData", type="POINT")
        light_data.energy = 800.0
        real_light = bpy.data.objects.new("RealPoint", light_data)
        bpy.context.scene.collection.objects.link(real_light)
        real_light.location = (0.0, 0.0, 3.0)
        exported_light = next(
            light
            for light in addon.exporter._export_local_lights()
            if light.name == real_light.name
        )
        expected_radius = math.sqrt(
            (light_data.energy / 100.0)
            / bpy.context.scene.eevee.light_threshold
        )
        require(
            abs(exported_light.influence_radius - expected_radius) < 1.0e-4
            and abs(exported_light.influence_radius - 40.0) > 1.0,
            "A disabled Blender Custom Distance exported its inactive "
            "40-metre cutoff instead of Eevee's power-based radius",
        )

        scene = bpy.context.scene
        scene.cursor.location = (0.0, 0.0, 1.0)
        require(
            bpy.ops.skate_map.set_spawn() == {"FINISHED"},
            "Spawn helper failed before automatic scene preparation",
        )
        spawn = bpy.data.objects[addon.exporter.SPAWN_OBJECT]
        require(spawn.type == "MESH", "Spawn locator is not a movable mesh")
        require(
            abs(spawn.dimensions.x - 4.0) < 1.0e-5
            and abs(spawn.dimensions.y - 4.0) < 1.0e-5,
            "Spawn locator is not a 4x4 metre pad",
        )
        require(
            tuple(spawn.location) == (0.0, 0.0, 0.0),
            "Spawn locator still followed the 3D cursor",
        )

        output = Path(tempfile.gettempdir()) / "auto_import_workflow.skate"
        cache = output.with_name(output.name + ".export-cache.json")
        for path in (output, cache):
            path.unlink(missing_ok=True)
        scene.owned_world.map_name = "Automatic Import Test"
        scene.owned_world.output_path = str(output)

        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            scene.owned_world.validation_details
            or scene.owned_world.last_status,
        )
        require(output.is_file(), "Automatic export did not create a package")
        baked_base = bpy.data.images.get(
            str(
                complex_material.get(
                    "ow_baked_base_color_image", ""
                )
            )
        )
        require(
            baked_base is not None
            and baked_base.packed_file is not None
            and baked_base.use_fake_user
            and complex_material.get("ow_albedo_image")
            == baked_base.name,
            "Complex Blender Base Color was not flattened into a persistent "
            "export texture",
        )
        baked_pixels = list(baked_base.pixels[:])
        require(
            min(baked_pixels[3::4]) < 0.05
            and max(baked_pixels[3::4]) > 0.95
            and bool(baked_base.get("ow_bake_alpha_preserved", False))
            and all(
                baked_pixels[index]
                or baked_pixels[index + 1]
                or baked_pixels[index + 2]
                for index in range(0, len(baked_pixels), 4)
            ),
            "Complex Base Color lost its metallic colour graph, neutral "
            "vertex TINT, or complete texture-domain coverage",
        )
        analyzed = analyze_package(output, include_payloads=True)
        exported_glass = next(
            (
                record
                for record in analyzed["_materials"]
                if str(record["name"]).startswith(
                    "PhysicalGlassFallback"
                )
            ),
            None,
        )
        require(
            exported_glass is not None
            and int(exported_glass["alpha_mode"]) == 2,
            "The physical Glass BSDF fallback was not exported as a "
            "transparent material",
        )
        require(
            tuple(
                round(float(value), 3)
                for value in exported_glass["display_color"]
            )
            == (1.0, 1.0, 1.0),
            "The exported glass fallback retained an unsupported object "
            "colour multiplier",
        )
        object_tint_variants = [
            record
            for record in analyzed["_materials"]
            if str(record["name"]).startswith("ObjectTintComplex")
        ]
        require(
            len(object_tint_variants) == 2,
            "One shared Blender material with two Object Info colours did "
            "not export as two lightweight material variants",
        )
        exported_colours = {
            tuple(round(float(value), 3) for value in record["display_color"])
            for record in object_tint_variants
        }
        require(
            exported_colours
            == {(1.0, 1.0, 1.0), (0.25, 0.5, 0.75)},
            "Object Info colour variants did not retain their Blender "
            f"multipliers: {sorted(exported_colours)!r}",
        )
        default_group = bpy.data.collections.get(
            addon.exporter.PRESENTATION_COLLISION_COLLECTION
        )
        collider_group = bpy.data.collections.get(
            addon.exporter.NO_PRESENTATION_COLLECTION
        )
        no_collision_group = bpy.data.collections.get(
            addon.exporter.NO_COLLISION_COLLECTION
        )
        require(default_group is not None, "Automatic export missed Group 1")
        require(collider_group is not None, "Automatic export missed Group 2")
        require(no_collision_group is not None, "Automatic export missed Group 3")
        require(
            bpy.data.collections.get(
                addon.exporter.LEGACY_VISUAL_COLLECTION
            )
            is None
            and bpy.data.collections.get(
                addon.exporter.LEGACY_COLLISION_COLLECTION
            )
            is None,
            "Legacy role collections were not cleaned up after migration",
        )
        require(
            addon._object_groups(floor)
            == [addon.exporter.PRESENTATION_COLLISION_COLLECTION],
            "Ordinary mesh was not assigned exclusively to Group 1",
        )
        require(
            addon._object_groups(collider)
            == [addon.exporter.NO_PRESENTATION_COLLECTION],
            "Explicit collider was not assigned exclusively to Group 2",
        )
        for semantic_name_object, semantic_material in (
            (grass_planter, grass_material),
            (foliage, foliage_material),
        ):
            require(
                addon._object_groups(semantic_name_object)
                == [addon.exporter.PRESENTATION_COLLISION_COLLECTION],
                f"{semantic_name_object.name} was guessed from its name "
                "instead of receiving the conservative Group 1 baseline",
            )
            require(
                bool(
                    semantic_material.get("ow_collision_enabled", False)
                ),
                f"{semantic_material.name} was guessed non-colliding from "
                "its name",
            )
        require(
            not addon._object_groups(scale_reference),
            "Character scale helper was incorrectly exported as map geometry",
        )
        require(
            not addon._object_groups(lod1),
            "Secondary imported LOD was incorrectly exported",
        )
        require(
            floor.name in default_group.all_objects
            and collider.name in collider_group.all_objects,
            "Five-group automatic classification is incomplete",
        )
        scene.owned_world.material_list_index = bpy.data.materials.find(
            material.name
        )
        require(
            floor.select_get()
            and bpy.context.view_layer.objects.active == floor
            and not collider.select_get(),
            "Clicking a material-list row did not select its scene meshes",
        )
        require(
            bpy.ops.skate_map.set_material_group(
                group=addon.exporter.NO_COLLISION_COLLECTION
            )
            == {"FINISHED"}
            and addon._object_groups(floor)
            == [addon.exporter.NO_COLLISION_COLLECTION],
            "Material-list Group 3 button did not move its mesh users",
        )
        require(
            bpy.ops.skate_map.set_material_group(
                group=addon.exporter.PRESENTATION_COLLISION_COLLECTION
            )
            == {"FINISHED"}
            and addon._object_groups(floor)
            == [addon.exporter.PRESENTATION_COLLISION_COLLECTION],
            "Material-list Group 1 button did not restore its mesh users",
        )
        no_collision_group.objects.link(floor)
        exclusive_issues, _warnings, _stats = addon._validate_scene(
            bpy.context, inspect_geometry=False
        )
        require(
            any(
                "must belong to exactly one map group" in issue
                for issue in exclusive_issues
            ),
            "Validation did not reject multiple group memberships",
        )
        addon._move_to_group(
            scene,
            floor,
            addon.exporter.PRESENTATION_COLLISION_COLLECTION,
        )
        addon._move_to_group(
            scene,
            shared_leaf_card,
            addon.exporter.NO_COLLISION_COLLECTION,
        )
        shared_material_issues, _warnings, _stats = addon._validate_scene(
            bpy.context, inspect_geometry=False
        )
        require(
            not shared_material_issues,
            "Validation rejected one visual material used by objects with "
            "different collision roles: " + "; ".join(shared_material_issues),
        )
        require(
            floor.data.uv_layers.get("UVMap") is not None
            and floor.data.uv_layers.get("Lightmap") is not None,
            "Automatic export did not create required UV layers",
        )
        for layer_name in ("TEXCOORD", "UVMap", "Lightmap"):
            require(
                all(
                    float("-inf") < float(coordinate) < float("inf")
                    and abs(float(coordinate)) <= 1.0e6
                    for item in floor.data.uv_layers[layer_name].data
                    for coordinate in item.uv
                ),
                f"Generated {layer_name} contains uninitialized UV data",
            )
        generated_grass_uvs = {
            tuple(round(float(value), 6) for value in item.uv)
            for item in grass_planter.data.uv_layers["UVMap"].data
        }
        require(
            len(generated_grass_uvs) > 1
            and bool(grass_planter.data.get("ow_generated_box_uv", False)),
            "A mesh without source UVs still received a collapsed zero UV",
        )
        require(
            material.get("ow_albedo_image") == image.name,
            "Principled Base Color image was not imported",
        )
        require(
            material.get("ow_uv_map") == "TEXCOORD",
            "An explicit Blender UV Map node was not preserved",
        )
        require(
            tuple(material.get("ow_uv_transform", ()))
            == (2.0, 3.0, 0.0, 0.25, -0.5),
            "A common Blender Mapping-node UV transform was not preserved",
        )
        require(
            int(cutout_material.get("ow_alpha_mode", 0)) == 1
            and bool(cutout_shader.inputs["Alpha"].links),
            "Auto Prepare did not preserve shader-authored transparency",
        )
        require(
            int(cutout_material.get("ow_audio_surface", -1)) == 55
            and int(cutout_material.get("ow_physics_surface", -1)) == 3,
            "Legacy alpha repair overwrote authored Skate surface semantics",
        )
        require(
            abs(float(material.get("ow_roughness", 0.0)) - 0.63) < 1.0e-5,
            "Principled roughness was not imported",
        )
        require(
            bool(material.get("ow_auto_imported", False)),
            "Automatically imported material was not marked refreshable",
        )
        require(
            int(material.get("ow_cull_mode", 0)) == 1,
            "Blender's default two-sided material policy was not preserved",
        )
        normal = bpy.data.images.get(
            str(material.get("ow_normal_image", ""))
        )
        orm = bpy.data.images.get(str(material.get("ow_orm_image", "")))
        require(
            normal is None and orm is not None,
            "Auto Prepare invented a normal map or lost scalar metallic",
        )
        require(
            orm.packed_file is not None
            and orm.use_fake_user,
            "Generated metallic ORM map was not made persistent",
        )
        require(
            abs(scene.owned_world.day_ambient - 0.32) < 1e-6
            and abs(scene.owned_world.night_ambient - 0.11) < 1e-6,
            "Local lights rewrote global ambient lighting",
        )
        depsgraph = bpy.context.evaluated_depsgraph_get()
        evaluated_mesh, evaluated_object = addon.exporter._mesh_for_export(
            floor, depsgraph, preserve_all_data_layers=True
        )
        try:
            require(
                len(evaluated_mesh.polygons) > len(floor.data.polygons),
                "Build did not evaluate the Solidify modifier",
            )
        finally:
            if evaluated_object is not None:
                evaluated_object.to_mesh_clear()

        # Older add-on builds could save an auto-imported material before its
        # Blender image links were available, leaving an empty albedo field
        # that every later export treated as authoritative. Automatic
        # materials must heal from their shader graph on the next export.
        material["ow_albedo_image"] = ""
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            "Automatic export did not refresh stale material metadata",
        )
        require(
            material.get("ow_albedo_image") == image.name,
            "Stale auto-imported albedo metadata was not healed",
        )

        # A real UI edit transfers ownership to the author, preventing future
        # automatic preparation from overwriting deliberate choices.
        material.owned_world.roughness = 0.51
        require(
            not bool(material.get("ow_auto_imported", True)),
            "Manual material edit did not disable automatic overwrites",
        )
        require(
            not addon._auto_configure_material(material),
            "Automatic preparation overwrote a manually edited material",
        )
        require(
            abs(float(material.get("ow_roughness", 0.0)) - 0.51) < 1.0e-5,
            "Manual material value was not preserved",
        )
        material["ow_albedo_image"] = ""
        require(
            addon._auto_configure_material(
                material, generate_maps=True
            )
            and material.get("ow_albedo_image") == image.name
            and abs(float(material.get("ow_roughness", 0.0)) - 0.51)
            < 1.0e-5
            and not bool(material.get("ow_auto_imported", True)),
            "Auto Prepare did not fill a missing authored albedo while "
            "preserving manual scalar settings",
        )

        # Older add-on builds saved generated images as transient datablocks.
        # On reload Blender retained their names and metadata but restored
        # black pixels. Authored materials preserve deliberate source maps,
        # but add-on-owned metallic ORM maps must still be regenerated and
        # packed.
        generated_name = str(material["ow_orm_image"])
        previous = bpy.data.images.get(generated_name)
        require(previous is not None, "Missing generated ORM setup")
        bpy.data.images.remove(previous)
        broken = bpy.data.images.new(
            generated_name, width=4, height=4, alpha=True
        )
        broken["ow_generated_map"] = "ORM"
        broken["ow_generated_for"] = material.name
        broken.use_fake_user = True
        _stats, repair_warnings = addon._auto_prepare_scene(bpy.context)
        require(not repair_warnings, "Legacy generated-map repair warned")
        repaired_orm = bpy.data.images[generated_name]
        repaired_orm_pixels = list(repaired_orm.pixels[:])
        require(
            any(
                repaired_orm_pixels[index]
                or repaired_orm_pixels[index + 1]
                or repaired_orm_pixels[index + 2]
                for index in range(0, len(repaired_orm_pixels), 4)
            )
            and repaired_orm.packed_file is not None,
            "Auto Prepare did not repair a black legacy generated ORM map",
        )
        require(
            abs(float(material.get("ow_roughness", 0.0)) - 0.51) < 1.0e-5
            and not bool(material.get("ow_auto_imported", True)),
            "Generated-map repair overwrote authored material settings",
        )

        shader.inputs["Roughness"].default_value = 0.41
        bpy.ops.object.select_all(action="DESELECT")
        floor.select_set(True)
        bpy.context.view_layer.objects.active = floor
        require(
            bpy.ops.owmaterial.apply_preset(preset="TREE") == {"FINISHED"},
            "Tree material preset failed",
        )
        require(
            material.get("ow_presentation_type") == "VEGETATION"
            and int(material.get("ow_alpha_mode", 0)) == 1
            and not bool(material.get("ow_collision_enabled", True)),
            "Tree preset did not configure optimized cutout presentation",
        )
        require(
            bpy.ops.owmaterial.sync_materials() == {"FINISHED"},
            "Sync Materials operator failed",
        )
        require(
            abs(float(material.get("ow_roughness", 0.0)) - 0.41) < 1.0e-5,
            "Sync Materials did not refresh shader roughness",
        )
        orm = bpy.data.images[str(material["ow_orm_image"])]
        orm_pixels = [0.0] * (orm.size[0] * orm.size[1] * 4)
        orm.pixels.foreach_get(orm_pixels)
        require(
            abs(orm_pixels[1] - 0.41) < 0.01,
            "Sync Materials did not overwrite generated roughness data",
        )

        sharp_mesh = bpy.data.meshes.new("SharpRailMesh")
        sharp_mesh.from_pydata(
            [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (2.0, 0.0, 0.0)],
            [(0, 1), (1, 2)],
            [],
        )
        sharp_mesh.update()
        sharp_attribute = sharp_mesh.attributes.new(
            "sharp_edge", "BOOLEAN", "EDGE"
        )
        for value in sharp_attribute.data:
            value.value = True
        sharp_object = bpy.data.objects.new("SharpRail", sharp_mesh)
        scene.collection.objects.link(sharp_object)
        bpy.ops.object.select_all(action="DESELECT")
        sharp_object.select_set(True)
        bpy.context.view_layer.objects.active = sharp_object
        require(
            bpy.ops.skate_map.grinds_from_sharp_edges() == {"FINISHED"},
            "Sharp-edge grind generation failed",
        )
        grind_object = bpy.context.object
        require(
            grind_object.type == "CURVE"
            and len(grind_object.data.splines) == 1
            and len(grind_object.data.splines[0].points) == 3,
            "Sharp edges were not consolidated into one three-point spline",
        )
        require(
            addon._object_groups(grind_object)
            == [addon.exporter.GRIND_COLLECTION],
            "Generated grind spline was not placed exclusively in Group 4",
        )
        grind_data = grind_object.data
        bpy.data.objects.remove(grind_object, do_unlink=True)
        bpy.data.curves.remove(grind_data)
        bpy.data.objects.remove(sharp_object, do_unlink=True)
        bpy.data.meshes.remove(sharp_mesh)
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            "Automatic export failed after a manual material edit",
        )
        collider_fallback = bpy.data.materials.get(
            str(collider.get("ow_material", ""))
        )
        require(
            collider_fallback is not None
            and bool(
                collider_fallback.get("ow_collision_enabled", True)
            ),
            "Collider-only mesh did not receive a visible colliding fallback "
            "material",
        )

        _, _, counts = addon.exporter._read_package_header(output)
        require(
            counts[-2] == 1,
            "Only the real Blender Light should have been exported",
        )

        scalar_output = output.with_name(
            "auto_import_workflow_scalar.skate"
        )
        scalar_output.unlink(missing_ok=True)
        original_numpy = addon.exporter.numpy
        addon.exporter.numpy = None
        try:
            addon.exporter.export_scene(
                scalar_output, force_rebuild=True
            )
        finally:
            addon.exporter.numpy = original_numpy
        _, _, scalar_counts = addon.exporter._read_package_header(
            scalar_output
        )
        require(
            scalar_counts == counts,
            "Bulk and scalar geometry paths produced different counts",
        )
        semantic_failures, _, _, _ = compare_packages(
            scalar_output, output
        )
        require(
            not semantic_failures,
            "Bulk and scalar geometry paths produced different decoded "
            f"content: {'; '.join(semantic_failures)}",
        )

        persistence_blend = Path(tempfile.gettempdir()) / (
            "auto_import_generated_map_persistence.blend"
        )
        persistence_blend.unlink(missing_ok=True)
        persisted_material_name = material.name
        persisted_complex_material_name = complex_material.name
        require(
            bpy.ops.wm.save_as_mainfile(filepath=str(persistence_blend))
            == {"FINISHED"},
            "Could not save generated-map persistence fixture",
        )
        require(
            bpy.ops.wm.open_mainfile(
                filepath=str(persistence_blend), load_ui=False
            )
            == {"FINISHED"},
            "Could not reopen generated-map persistence fixture",
        )
        reloaded_material = bpy.data.materials[persisted_material_name]
        reloaded_orm = bpy.data.images[
            str(reloaded_material["ow_orm_image"])
        ]
        reloaded_complex = bpy.data.materials[persisted_complex_material_name]
        reloaded_bake = bpy.data.images[
            str(reloaded_complex["ow_baked_base_color_image"])
        ]
        pixels = list(reloaded_orm.pixels[:])
        require(
            reloaded_orm.packed_file is not None
            and any(
                pixels[index]
                or pixels[index + 1]
                or pixels[index + 2]
                for index in range(0, len(pixels), 4)
            ),
            "Generated ORM map became black after save/reopen",
        )
        require(
            reloaded_bake.packed_file is not None
            and any(
                reloaded_bake.pixels[index]
                or reloaded_bake.pixels[index + 1]
                or reloaded_bake.pixels[index + 2]
                for index in range(0, len(reloaded_bake.pixels), 4)
            ),
            "Flattened Blender Base Color became black after save/reopen",
        )
        reloaded_scene = bpy.context.scene
        require(
            bpy.ops.skate_map.quick_export() == {"FINISHED"},
            reloaded_scene.owned_world.validation_details
            or "Export failed after generated-map save/reopen",
        )
        print(
            "AUTO_IMPORT_WORKFLOW_OK",
            output,
            output.stat().st_size,
            reloaded_scene.owned_world.last_status,
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
