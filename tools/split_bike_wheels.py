"""Split a motorcycle blend into body / wheel_front / wheel_rear and export a GLB.

Downloaded models are routinely grouped by material rather than by part, so a
single "mesh" spans the whole bike and no wheel can be addressed on its own. This
breaks every mesh into loose (connected) pieces, finds the two wheels among them
geometrically, joins each wheel's pieces back together under a known name, and
exports. prepare_bike.py then has named wheels to work with.

    blender -b "<model.blend>" --python tools/split_bike_wheels.py -- <out.glb>

A wheel is found as a loose piece that is round in side view and thin across the
bike; the frontmost and rearmost such pieces seed the two groups, and every piece
whose centre falls inside a seed's disc (rim, spokes, hub, disc, tyre) joins it.
"""
import sys
import bpy
from mathutils import Vector

out = sys.argv[sys.argv.index("--") + 1]

try:
    bpy.ops.file.find_missing_files(directory=bpy.path.abspath("//../textures/"))
except Exception as e:
    print(f"find_missing_files: {e}")
try:
    bpy.ops.file.pack_all()
except Exception as e:
    print(f"pack_all: {e}")

for o in bpy.data.objects:
    o.hide_viewport = False
    o.hide_render = False
    try:
        o.hide_set(False)
    except Exception:
        pass

# --- break everything into connected pieces ---------------------------------
bpy.ops.object.select_all(action="DESELECT")
originals = [o for o in bpy.data.objects if o.type == "MESH"]
for o in originals:
    bpy.context.view_layer.objects.active = o
    o.select_set(True)
    try:
        bpy.ops.object.mode_set(mode="EDIT")
        bpy.ops.mesh.select_all(action="SELECT")
        bpy.ops.mesh.separate(type="LOOSE")
    except Exception as e:
        print(f"separate {o.name}: {e}")
    finally:
        bpy.ops.object.mode_set(mode="OBJECT")
    o.select_set(False)

pieces = [o for o in bpy.data.objects if o.type == "MESH" and len(o.data.vertices)]
print(f"loose pieces: {len(pieces)}")


def world_bounds(o):
    """From actual vertices. `bound_box` is cached and reads stale straight after
    a join, which silently reports a body taller than the whole model."""
    m = o.matrix_world
    points = [m @ v.co for v in o.data.vertices]
    lo = Vector((min(p[i] for p in points) for i in range(3)))
    hi = Vector((max(p[i] for p in points) for i in range(3)))
    return lo, hi


info = {o: world_bounds(o) for o in pieces}
lo_all = Vector((min(info[o][0][i] for o in pieces) for i in range(3)))
hi_all = Vector((max(info[o][1][i] for o in pieces) for i in range(3)))
extent = hi_all - lo_all
# Blender is Z-up: the bike's length is the longer of X and Y.
length_axis = 0 if extent[0] > extent[1] else 1
width_axis = 1 - length_axis
up_axis = 2
print(f"extent {tuple(round(v, 3) for v in extent)}, length axis {'XYZ'[length_axis]}")


def wheel_like(o):
    """A tyre is very nearly a circle in side view. Measured on a real model the
    tyres score 0.002 to 0.046 while the next roundest part is 0.106, so this
    threshold is a gap, not a guess: loosening it to 0.2 lets body panels in."""
    lo, hi = info[o]
    size = hi - lo
    diameter = max(size[up_axis], size[length_axis])
    if diameter < extent[length_axis] * 0.2:
        return False
    roundness = abs(size[up_axis] - size[length_axis]) / diameter
    return roundness < 0.06 and size[width_axis] < diameter * 0.7


candidates = [o for o in pieces if wheel_like(o)]
print(f"wheel-shaped pieces: {len(candidates)}")
if len(candidates) < 2:
    raise SystemExit("Could not find two wheel-shaped pieces; split them by hand in Blender.")

centre_of = lambda o: (info[o][0][length_axis] + info[o][1][length_axis]) / 2
diameter_of = lambda o: max(
    info[o][1][up_axis] - info[o][0][up_axis],
    info[o][1][length_axis] - info[o][0][length_axis],
)
# Seed each end with its *largest* wheel-shaped piece, which is the tyre. Picking
# the outermost instead can land on a brake disc or a rim and give a container
# too small to hold the tyre.
midpoint = (lo_all[length_axis] + hi_all[length_axis]) / 2
ahead = [o for o in candidates if centre_of(o) > midpoint]
behind = [o for o in candidates if centre_of(o) <= midpoint]
if not ahead or not behind:
    raise SystemExit("Wheel-shaped pieces all fell on one end; split them by hand in Blender.")

# Which end is the front: the handlebars are the highest thing on a motorcycle
# and they sit over the front wheel, so the front is whichever end the top of the
# bike leans towards.
#
# Do NOT use tyre width. A front wheel looks WIDER here, not narrower, because
# its containment box swallows the fork lowers either side of the rim, while the
# rear's swingarm sits inside the tyre. Measured on a real model the front group
# came out 0.380 across and the rear 0.168 -- the exact opposite of the 21"/19"
# intuition, and following it points the whole bike backwards.
highest = max(pieces, key=lambda o: info[o][1][up_axis])
bars = centre_of(highest)
front_is_ahead = abs(bars - centre_of(max(ahead, key=diameter_of))) < abs(
    bars - centre_of(max(behind, key=diameter_of))
)
front_pool, rear_pool = (ahead, behind) if front_is_ahead else (behind, ahead)
front_seed = max(front_pool, key=diameter_of)
rear_seed = max(rear_pool, key=diameter_of)
print(
    f"top of bike at {bars:+.3f} along the length; front wheel is the "
    f"{'higher' if front_is_ahead else 'lower'} end"
)


def group_for(seed):
    """Everything that fits inside the tyre's own box: rim, spokes, hub, disc.

    Containment, not proximity to the axle: a fork leg or a swingarm has its
    centre near the axle but reaches far outside the wheel, and including one
    drags the whole front end into the group.
    """
    lo, hi = info[seed]
    pad = diameter_of(seed) * 0.02
    return [
        o
        for o in pieces
        if all(
            info[o][0][i] >= lo[i] - pad and info[o][1][i] <= hi[i] + pad for i in range(3)
        )
    ]


front, rear = group_for(front_seed), group_for(rear_seed)

# Drop the side stand. Nothing on a motorcycle reaches below the tyres' contact
# line except the stand it is parked on, so that is a safe test -- and a stand
# left deployed is very visible once the bike is moving and jumping.
wheel_bottom = min(info[o][0][up_axis] for o in front + rear)
stand = [
    o
    for o in pieces
    if o not in set(front) | set(rear) and info[o][0][up_axis] < wheel_bottom + 0.01
]
if stand:
    print(f"dropping {len(stand)} pieces below the tyre line (side stand)")
    for o in stand:
        pieces.remove(o)
        info.pop(o, None)
        bpy.data.objects.remove(o, do_unlink=True)
overlap = set(front) & set(rear)
front = [o for o in front if o not in overlap]
rear = [o for o in rear if o not in overlap]
body = [o for o in pieces if o not in set(front) | set(rear)]
print(f"front {len(front)}, rear {len(rear)}, body {len(body)}, dropped overlap {len(overlap)}")
if not front or not rear or not body:
    raise SystemExit("Wheel grouping produced an empty set; split them by hand in Blender.")


def join(objects, name):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objects:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    if len(objects) > 1:
        bpy.ops.object.join()
    joined = bpy.context.view_layer.objects.active
    joined.name = name
    joined.data.name = name
    # Bake the transform into the mesh so the exported node is identity. A joined
    # object inherits the first piece's rotation, and a rotated node makes any
    # downstream reader that transforms the accessor's local AABB corners inflate
    # the result -- a 0.68 m wheel measured as 0.92 m.
    joined.select_set(True)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    bpy.ops.object.select_all(action="DESELECT")
    return joined


for group, name in ((body, "bike_body"), (front, "wheel_front"), (rear, "wheel_rear")):
    joined = join(group, name)
    lo, hi = world_bounds(joined)
    size = hi - lo
    print(f"{name}: size {tuple(round(v, 3) for v in size)} centre {tuple(round(v, 3) for v in (lo + hi) / 2)}")

bpy.ops.export_scene.gltf(
    filepath=out,
    export_format="GLB",
    use_selection=False,
    use_visible=False,
    use_renderable=False,
    export_apply=True,
    export_yup=True,
    export_texcoords=True,
    export_normals=True,
    export_materials="EXPORT",
    export_animations=False,
    export_skins=False,
)
print(f"wrote {out}")
