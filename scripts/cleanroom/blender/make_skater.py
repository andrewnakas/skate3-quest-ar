"""Build the clean tier's skater in Blender from MakeHuman CC0 assets (MPFB 2) and export it as glTF.

    Blender -b --python scripts/cleanroom/blender/make_skater.py -- <out.glb> [preview.png]

Needs the MPFB extension and the MakeHuman system asset pack (CC0) in MPFB's user data dir
(see scripts/cleanroom/README.md). The body shape is set here (our own phenotype), the rig is
MPFB's game-engine rig with every bone renamed to our own scheme, and the result is one skinned
mesh per material plus the armature.
"""
import importlib
import json
import math
import os
import sys
from pathlib import Path

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
import skater_anims  # noqa: E402

ARGS = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = Path(ARGS[0]) if ARGS else Path("skater.glb")
PREVIEW = Path(ARGS[1]) if len(ARGS) > 1 else None

M = "bl_ext.blender_org.mpfb.services."
HumanService = importlib.import_module(M + "humanservice").HumanService
LocationService = importlib.import_module(M + "locationservice").LocationService
DATA = Path(LocationService.get_user_data())

TANK_ARM = 0.12   # a shirt face this much on the arms is a sleeve (cut for the tank top)
PANTS_DARK = 0.55  # the denim's brightness kept, made grey: dark charcoal pants

# Our skater: a lean young adult, a little taller than average, long-limbed.
PRESET = {
    "phenotype": {
        "gender": 1.0,
        "age": 0.42,
        "muscle": 0.6,
        "weight": 0.41,
        "height": 0.6,
        "proportions": 0.66,
        "cupsize": 0.5,
        "firmness": 0.5,
        "race": {"african": 0.3, "asian": 0.2, "caucasian": 0.5},
    },
    "skin_mhmat": "young_asian_male/young_asian_male.mhmat",  # tanned
    "skin_material_type": "MAKESKIN",
    "eyes": "high-poly/high-poly.mhclo",
    "eyes_material_type": "MAKESKIN",
    "eyebrows": "eyebrow001/eyebrow001.mhclo",
    "eyelashes": "eyelashes01/eyelashes01.mhclo",
    "hair": "short04/short04.mhclo",  # cropped close, dark
    "clothes": [
        "male_casualsuit06/male_casualsuit06.mhclo",
        "shoes05/shoes05.mhclo",  # white trainers
    ],
    "clothes_material_type": "MAKESKIN",
    "rig": "game_engine",
}

# MPFB game-engine bone -> our name. Anything not listed keeps a prefixed name.
BONES = {
    "root": "base",
    "pelvis": "hips",
    "spine_01": "back_low",
    "spine_02": "back_mid",
    "spine_03": "chest",
    "neck_01": "neck",
    "head": "skull",
    "clavicle_l": "collar.L",
    "upperarm_l": "arm.L",
    "lowerarm_l": "elbow.L",
    "hand_l": "wrist.L",
    "clavicle_r": "collar.R",
    "upperarm_r": "arm.R",
    "lowerarm_r": "elbow.R",
    "hand_r": "wrist.R",
    "thigh_l": "leg.L",
    "calf_l": "knee.L",
    "foot_l": "ankle.L",
    "ball_l": "toes.L",
    "thigh_r": "leg.R",
    "calf_r": "knee.R",
    "foot_r": "ankle.R",
    "ball_r": "toes.R",
}


CLOTH_OFFSET = 0.006  # m


def build():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    info = dict(PRESET)
    info.update({"targets": [], "expressions": [], "makeup": [], "proxy": "", "teeth": "", "tongue": "",
                 "color_adjustments": {}, "eyes_material_settings": {}, "skin_material_settings": {}})
    settings = HumanService.get_default_deserialization_settings()
    settings["subdiv_levels"] = 0
    settings["override_skin_model"] = "MAKESKIN"
    return HumanService.deserialize_from_dict(info, settings)


def finish(basemesh):
    rig = basemesh.parent
    for b in rig.data.bones:
        b.name = BONES.get(b.name, "sk_" + b.name)
    rig.name = rig.data.name = "skater_rig"
    # (the tank top: the body keeps its skin where the shirt's sleeves would have hidden it)
    for ob in [o for o in bpy.data.objects if o.type == "MESH"]:
        arm = {g.index for g in ob.vertex_groups if g.name in ("arm.L", "arm.R", "elbow.L", "elbow.R")}
        dels = [g for g in ob.vertex_groups if g.name.startswith("Delete.")]
        if not arm or not dels:
            continue
        keep = [v.index for v in ob.data.vertices if sum(e.weight for e in v.groups if e.group in arm) > 0.02]
        for g in dels:
            g.remove(keep)
        print(f"SKATER tank top: {len(keep)} arm vertices kept out of the clothes' hiding")
    # Bake shape keys / modifiers (except the armature) so the export is a plain skinned mesh.
    for ob in [o for o in bpy.data.objects if o.type == "MESH"]:
        bpy.context.view_layer.objects.active = ob
        if ob.data.shape_keys:
            ob.shape_key_add(name="baked", from_mix=True)
            keys = ob.data.shape_keys.key_blocks
            for k in list(keys):
                if k.name != "baked":
                    ob.shape_key_remove(k)
            ob.shape_key_remove(ob.data.shape_keys.key_blocks["baked"])
        for m in list(ob.modifiers):
            if m.type in ("SUBSURF",):
                ob.modifiers.remove(m)
            elif m.type not in ("ARMATURE",):
                try:
                    bpy.ops.object.modifier_apply(modifier=m.name)
                except RuntimeError as e:
                    print("modifier", ob.name, m.name, e)
        # Helper geometry (MPFB's joint/clothing helpers) is masked out of the body.
        # The clothes sit a few millimetres proud of the skin: with an arm hanging close to the
        # body the skin poked out through the sleeves.
        if any("suit" in m.name for m in ob.data.materials if m):
            for v in ob.data.vertices:
                v.co += v.normal * CLOTH_OFFSET
        ob.name = "skater_" + ob.name.split(".")[0].lower()
    tank_top()
    dark_pants()
    hide_covered_skin()
    tris = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in bpy.data.objects if o.type == "MESH")
    heights = [v.co.z for o in bpy.data.objects if o.type == "MESH" for v in o.data.vertices]
    print(f"SKATER meshes={sum(1 for o in bpy.data.objects if o.type == 'MESH')} tris={tris} "
          f"bones={len(rig.data.bones)} height={max(heights) - min(heights):.3f}")


def tank_top():
    """The T-shirt cut to a tank top: its faces on the arms (weighted to the upper / lower arm bones)
    go, so the arms show bare to the shoulder."""
    import bmesh
    for ob in [o for o in bpy.data.objects if o.type == "MESH"]:
        if not any(m and "suit" in m.name for m in ob.data.materials):
            continue
        arm = {g.index for g in ob.vertex_groups if g.name in ("arm.L", "arm.R", "elbow.L", "elbow.R")}
        on_arm = [sum(e.weight for e in v.groups if e.group in arm) for v in ob.data.vertices]
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bm.verts.ensure_lookup_table()
        gone = [f for f in bm.faces if sum(on_arm[v.index] for v in f.verts) / len(f.verts) > TANK_ARM]
        bmesh.ops.delete(bm, geom=gone, context="FACES")
        bm.to_mesh(ob.data)
        bm.free()
        print(f"SKATER tank top: {len(gone)} sleeve faces removed")


def dark_pants():
    """Our own copy of the suit's texture with the denim taken down to a dark charcoal (the shirt,
    near white, stays)."""
    import numpy as np
    for m in bpy.data.materials:
        if "suit" not in m.name or not m.use_nodes:
            continue
        for n in m.node_tree.nodes:
            if n.type != "TEX_IMAGE" or not n.image or "diffuse" not in n.image.name.lower():
                continue
            img = n.image
            px = np.array(img.pixels[:], dtype=np.float32).reshape(-1, 4)
            r, g, b = px[:, 0], px[:, 1], px[:, 2]
            blue = (b - r > 0.05) & (b > 0.12)
            lum = 0.3 * r + 0.59 * g + 0.11 * b
            for c in range(3):
                px[blue, c] = lum[blue] * PANTS_DARK
            # (the shirt's orange logo lifted off: plain)
            logo = (r - b > 0.15) & ~blue
            for c in range(3):
                px[logo, c] = 0.86
            dark = img.copy()
            dark.name = "sk8_suit_dark"
            dark.pixels = px.ravel().tolist()
            dark.pack()
            n.image = dark
            print(f"SKATER pants: {int(blue.sum())} denim texels darkened")


def hide_covered_skin():
    """Deletes the body's faces that lie wholly under the clothes (straight out from the skin, cloth
    within 12 cm): in a deep bend (a noseblunt) skin could otherwise push out through the shirt,
    whatever the offset. MPFB already drops most covered skin; this takes the rest near the hems.
    Skin at the hems, neck, hands and face stays."""
    import bmesh
    from mathutils.bvhtree import BVHTree
    meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    body = next((o for o in meshes if any(m and m.name.endswith("body") for m in o.data.materials)), None)
    covers = [o for o in meshes if any(m and ("suit" in m.name or "shoes" in m.name) for m in o.data.materials)]
    if body is None or not covers:
        print("SKATER skin: no body or clothes found")
        return
    dg = bpy.context.evaluated_depsgraph_get()
    trees = [(BVHTree.FromObject(o, dg), o.matrix_world.inverted()) for o in covers]
    # the body as it stands (armature applied), the same state the clothes' trees are built from
    body_eval = body.evaluated_get(dg)
    me = body_eval.to_mesh()
    covered = set()
    for v in me.vertices:
        p = body.matrix_world @ v.co
        n = (body.matrix_world.to_3x3() @ v.normal).normalized()
        hit = False
        for tree, inv in trees:
            q = inv @ p
            # straight out from the skin: cloth in the way, close by
            loc, _, _, _ = tree.ray_cast(q, (inv.to_3x3() @ n).normalized(), 0.12)
            if loc is not None:
                hit = True
                break
        if hit:
            covered.add(v.index)
    body_eval.to_mesh_clear()
    bm = bmesh.new()
    bm.from_mesh(body.data)
    gone = [f for f in bm.faces if all(v.index in covered for v in f.verts)]
    bmesh.ops.delete(bm, geom=gone, context="FACES")
    bm.to_mesh(body.data)
    bm.free()
    print(f"SKATER skin: {len(gone)} faces under the clothes removed ({len(covered)} covered vertices)")


def export():
    OUT.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=str(OUT),
        export_format="GLB",
        export_apply=False,
        export_skins=True,
        export_animations=True,
        export_animation_mode="ACTIONS",
        export_yup=True,
    )
    print("SKATER exported", OUT)


def preview(clips):
    """One small render per clip (its middle frame) next to a stand-in board, tiled into PREVIEW."""
    scene = bpy.context.scene
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scene.collection.objects.link(cam)
    cam.location = (2.2, -3.4, 1.3)
    cam.rotation_euler = (math.radians(80), 0, math.radians(33))
    scene.camera = cam
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
    sun.rotation_euler = (math.radians(50), 0, math.radians(30))
    scene.collection.objects.link(sun)
    bpy.ops.mesh.primitive_cube_add(size=1, location=(0, 0, -0.06))
    board = bpy.context.active_object
    board.scale = (0.8, 0.21, 0.02)
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x, scene.render.resolution_y = 300, 360
    # SK8_PREVIEW_MATERIAL=1: material colours and a bigger image (skin poking through clothes)
    if os.environ.get("SK8_PREVIEW_MATERIAL"):
        scene.display.shading.color_type = "TEXTURE"
        scene.render.resolution_x, scene.render.resolution_y = 600, 720
    rig = bpy.data.objects["skater_rig"]
    rig.animation_data_create()
    frames = []
    views = {"front": ((2.2, -3.4, 1.3), (80, 0, 33)), "side": ((4.0, 0.0, 0.8), (90, 0, 90))}
    if os.environ.get("SK8_PREVIEW_MATERIAL"):  # and from behind / above (the back of the shirt)
        views["back"] = ((-1.6, 3.2, 2.0), (65, 0, 206))
        views["top"] = ((0.6, 1.2, 3.4), (20, 0, 160))
    # SK8_PREVIEW_ONLY=bail,get_up: only clips whose names contain one of these
    only = [x for x in os.environ.get("SK8_PREVIEW_ONLY", "").split(",") if x]
    for name in clips:
        if only and not any(o in name for o in only):
            continue
        act = bpy.data.actions[name]
        rig.animation_data.action = act
        lo, hi = (int(f) for f in act.frame_range)
        # A filmstrip per clip and view: every third frame of a one-shot, eight frames of a loop.
        step = max(1, (hi - lo) // 8) if act.get("loop") else 3
        for view, (loc, rot) in views.items():
            cam.location = loc
            cam.rotation_euler = [math.radians(a) for a in rot]
            for f in range(lo, hi + 1, step):
                scene.frame_set(f)
                out = PREVIEW.with_name(f"{PREVIEW.stem}_{view}_{name}_{f - lo:03d}.png")
                scene.render.filepath = str(out)
                bpy.ops.render.render(write_still=True)
                frames.append(out)
    rig.animation_data.action = None
    bpy.data.objects.remove(board)
    bpy.data.objects.remove(cam)
    bpy.data.objects.remove(sun)
    print("SKATER previews", len(frames))


if __name__ == "__main__":  # importable (build/finish/preview/export) by other Blender scripts
    basemesh = build()
    finish(basemesh)
    clips = skater_anims.bake_clips(bpy.data.objects["skater_rig"])
    print("SKATER clips", clips)
    if PREVIEW:
        preview(clips)
    export()
