"""Export the currently opened .blend to a GLB with textures embedded.

Run headless; the blend file is the argument to Blender itself, so its relative
texture paths still resolve:

    blender -b "<model.blend>" --python tools/blend_to_glb.py -- <out.glb>

Kept separate from prepare_bike.py because that one is pure standard library and
runs anywhere, while this needs Blender's own interpreter.
"""
import sys
import bpy

out = sys.argv[sys.argv.index("--") + 1]

# Textures shipped beside the blend are often still relative or missing; pack
# whatever resolves so the GLB is self-contained. The host rejects any GLB that
# references an external buffer or image.
try:
    bpy.ops.file.find_missing_files(directory=bpy.path.abspath("//../textures/"))
except Exception as e:
    print(f"find_missing_files: {e}")
try:
    bpy.ops.file.pack_all()
except Exception as e:
    print(f"pack_all: {e}")

meshes = [o for o in bpy.data.objects if o.type == "MESH"]
print(f"objects: {len(bpy.data.objects)}, meshes: {len(meshes)}")
for o in meshes:
    o.hide_viewport = False
    o.hide_render = False
    o.hide_set(False)
print(f"materials: {len(bpy.data.materials)}, images: {len(bpy.data.images)}")

bpy.ops.export_scene.gltf(
    filepath=out,
    export_format="GLB",
    use_selection=False,
    use_visible=False,
    use_renderable=False,
    export_apply=True,  # bake modifiers so the exported mesh is what you see
    export_yup=True,
    export_texcoords=True,
    export_normals=True,
    export_materials="EXPORT",
    export_animations=False,
    export_skins=False,
)
print(f"wrote {out}")
