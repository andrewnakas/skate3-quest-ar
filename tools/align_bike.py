"""Straighten a prepared bike GLB's front end, in place. Stdlib only.

The source models these bikes come from are usually posed, not neutral: the one
this was written for has its bars, triple clamps, forks, fender, number plate
and front wheel all turned about 23 degrees about the steering axis, which
reads in game as a bike permanently trying to turn. `prepare_bike.py` squares
the *chassis* using the two axle centres, and cannot see this: turning the bars
barely moves the front axle, so the wheelbase it aligns to is already straight.

    python tools/align_bike.py sdk/examples/freestyle-mx/bike.glb --report
    python tools/align_bike.py sdk/examples/freestyle-mx/bike.glb --apply

How the turn is measured, rather than eyeballed: the two fork legs are the only
pair of long thin parallel tubes in the model, they are rigidly part of the
front end, and on a straight bike they are mirror images about the centre plane
-- same height, same distance forward. Turning the bars swings one forward and
the other back, so the angle of the line joining them, seen from above, *is*
the steering angle. Their common long axis is the steering axis, which gives
the rake for free.

Everything forward of the steering head then rotates back by that angle about
that axis. The cut is a plane through the head perpendicular to the steering
axis, which is exactly the surface the real steering head pivots on, so nothing
behind it (tank, radiators, frame, seat) is touched.
"""
import argparse
import json
import math
import shutil
import struct
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("model", type=Path)
p.add_argument("--apply", action="store_true", help="Write the model back")
p.add_argument("--report", action="store_true", help="Measure and print only")
p.add_argument("--angle", type=float, default=None, help="Override the measured turn, degrees")
p.add_argument("--bars", type=float, default=0.55,
               help="Height above which forward geometry is bar-mounted hardware")
p.add_argument("--cut", type=float, default=0.10,
               help="Move the steering-head cut back by this many metres, so swept-back "
                    "handlebars stay with the front end they belong to")
args = p.parse_args()

raw = args.model.read_bytes()
magic, version, total = struct.unpack_from("<III", raw, 0)
if magic != 0x46546C67 or version != 2:
    raise SystemExit("Not a GLB 2 file")
json_len = struct.unpack_from("<I", raw, 12)[0]
doc = json.loads(raw[20:20 + json_len])
bin_start = 20 + json_len + 8
blob = bytearray(raw[bin_start:])
materials = [m.get("name", "?") for m in doc.get("materials", [])]


def view_of(accessor):
    a = doc["accessors"][accessor]
    bv = doc["bufferViews"][a["bufferView"]]
    offset = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride") or 12
    return offset, stride, a["count"]


def read_vec3(accessor):
    offset, stride, count = view_of(accessor)
    return [struct.unpack_from("<3f", blob, offset + i * stride) for i in range(count)]


def write_vec3(accessor, values):
    offset, stride, count = view_of(accessor)
    assert len(values) == count
    for i, v in enumerate(values):
        struct.pack_into("<3f", blob, offset + i * stride, *v)


def read_indices(accessor):
    a = doc["accessors"][accessor]
    bv = doc["bufferViews"][a["bufferView"]]
    offset = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    fmt, size = {5121: ("<B", 1), 5123: ("<H", 2), 5125: ("<I", 4)}[a["componentType"]]
    return [struct.unpack_from(fmt, blob, offset + i * size)[0] for i in range(a["count"])]


# --- node transforms, so everything is measured in chassis-local space -------
def node_matrix(node):
    if "matrix" in node:
        m = node["matrix"]
        return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    s = node.get("scale", [1, 1, 1])
    t = node.get("translation", [0, 0, 0])
    rot = [[1 - 2 * y * y - 2 * z * z, 2 * x * y - 2 * z * w, 2 * x * z + 2 * y * w],
           [2 * x * y + 2 * z * w, 1 - 2 * x * x - 2 * z * z, 2 * y * z - 2 * x * w],
           [2 * x * z - 2 * y * w, 2 * y * z + 2 * x * w, 1 - 2 * x * x - 2 * y * y]]
    return [[rot[r][c] * s[c] for c in range(3)] + [t[r]] for r in range(3)] + [[0, 0, 0, 1]]


def matmul(a, b):
    return [[sum(a[r][k] * b[k][c] for k in range(4)) for c in range(4)] for r in range(4)]


def transform(m, v, point=True):
    w = 1.0 if point else 0.0
    return tuple(sum(m[r][c] * v[c] for c in range(3)) + m[r][3] * w for r in range(3))


def invert_rigid(m):
    """Inverse of a similarity transform (rotation * uniform scale + offset)."""
    scale = math.sqrt(sum(m[r][0] ** 2 for r in range(3)))
    rot = [[m[r][c] / scale for c in range(3)] for r in range(3)]
    inv = [[rot[c][r] / scale for c in range(3)] for r in range(3)]
    t = [m[r][3] for r in range(3)]
    return [inv[r] + [-sum(inv[r][c] * t[c] for c in range(3))] for r in range(3)] + [[0, 0, 0, 1]]


parents = {}
for i, node in enumerate(doc["nodes"]):
    for c in node.get("children", []):
        parents[c] = i


def world_of(i):
    m = node_matrix(doc["nodes"][i])
    return matmul(world_of(parents[i]), m) if i in parents else m


mesh_nodes = [(i, n) for i, n in enumerate(doc["nodes"]) if "mesh" in n]


def islands(prim):
    """Connected vertex groups, welded by position so shared corners merge."""
    pts = read_vec3(prim["attributes"]["POSITION"])
    idx = read_indices(prim["indices"])
    key, weld = {}, []
    for q in pts:
        weld.append(key.setdefault((round(q[0], 4), round(q[1], 4), round(q[2], 4)), len(key)))
    parent = list(range(len(key)))

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    for t in range(0, len(idx) - 2, 3):
        roots = [find(weld[idx[t + k]]) for k in range(3)]
        for r in roots[1:]:
            if r != roots[0]:
                parent[r] = roots[0]
    groups = {}
    for v, w in enumerate(weld):
        groups.setdefault(find(w), []).append(v)
    return pts, list(groups.values())


# --- find the fork legs ------------------------------------------------------
# A fork leg is long, thin, and forward of the middle of the bike. There are
# exactly two, they are the same size, and they straddle the centre plane.
candidates = []
for node_index, node in mesh_nodes:
    world = world_of(node_index)
    for pi, prim in enumerate(doc["meshes"][node["mesh"]]["primitives"]):
        pts, groups = islands(prim)
        for g in groups:
            if len(g) < 60:
                continue
            w = [transform(world, pts[v]) for v in g]
            lo = [min(q[k] for q in w) for k in range(3)]
            hi = [max(q[k] for q in w) for k in range(3)]
            span = [hi[k] - lo[k] for k in range(3)]
            centre = [(hi[k] + lo[k]) / 2 for k in range(3)]
            length = math.hypot(span[1], span[2])
            if length > 0.35 and span[0] < 0.15 and centre[2] > 0.2 and length > 3 * span[0]:
                candidates.append((len(g), centre, span, node_index, pi, materials[prim["material"]]
                                   if "material" in prim else "-"))

candidates.sort(key=lambda c: -c[0])
pair = None
for i in range(len(candidates)):
    for j in range(i + 1, len(candidates)):
        a, b = candidates[i], candidates[j]
        if a[1][0] * b[1][0] < 0 and abs(a[0] - b[0]) < max(a[0], b[0]) * 0.35:
            pair = (a, b)
            break
    if pair:
        break
if not pair:
    raise SystemExit("Could not find a pair of fork legs; align this model by hand.")

a, b = pair
print(f"fork legs: '{a[5]}' n={a[0]} at ({a[1][0]:+.3f},{a[1][1]:+.3f},{a[1][2]:+.3f})"
      f" and n={b[0]} at ({b[1][0]:+.3f},{b[1][1]:+.3f},{b[1][2]:+.3f})")

left, right = (a, b) if a[1][0] > b[1][0] else (b, a)
across = [left[1][k] - right[1][k] for k in range(3)]
turn = math.atan2(across[2], across[0])
# The steering axis: the legs' shared long direction, pointing up and raked
# back. Taken from their spans, which is a pair of measurements rather than an
# assumed rake angle.
span = [(left[2][k] + right[2][k]) / 2 for k in range(3)]
axis_len = math.hypot(span[1], span[2])
axis = (0.0, span[1] / axis_len, -span[2] / axis_len)
head = [(left[1][k] + right[1][k]) / 2 for k in range(3)]
print(f"steering axis through ({head[0]:+.3f},{head[1]:+.3f},{head[2]:+.3f}) "
      f"direction ({axis[0]:+.3f},{axis[1]:+.3f},{axis[2]:+.3f}), rake "
      f"{math.degrees(math.atan2(-axis[2], axis[1])):.1f} deg")
print(f"front end is turned {math.degrees(turn):+.2f} deg")

# Right-hand rule about the steering axis, which points up: a front end turned
# by `turn` is put back by rotating through `turn` again, not through its
# negation. Measured, not reasoned about -- the negation doubled a 21 degree
# turn into 41, which is the only way this sign is worth deciding.
correction = math.radians(args.angle) if args.angle is not None else turn
print(f"applying {math.degrees(correction):+.2f} deg about the steering axis")

if args.report or not args.apply:
    raise SystemExit(0)


def rotate_about(point, origin, axis, angle, is_point=True):
    """Rodrigues rotation about an arbitrary axis through `origin`."""
    v = [point[k] - (origin[k] if is_point else 0.0) for k in range(3)]
    c, s = math.cos(angle), math.sin(angle)
    dot = sum(v[k] * axis[k] for k in range(3))
    cross = (axis[1] * v[2] - axis[2] * v[1],
             axis[2] * v[0] - axis[0] * v[2],
             axis[0] * v[1] - axis[1] * v[0])
    out = [v[k] * c + cross[k] * s + axis[k] * dot * (1 - c) for k in range(3)]
    return tuple(out[k] + (origin[k] if is_point else 0.0) for k in range(3))


# Forward of the steering head, measured along the steering axis' own forward
# perpendicular. This is the surface a real steering head turns on, so the cut
# never catches the tank, the radiators or the frame behind it.
forward = (0.0, -axis[2], axis[1])
# Handlebars sweep back towards the rider, so a cut exactly on the steering
# head leaves them behind with the frame and shears the front end in half.
# Moving it back a little takes them with it; the tank is a further 0.17 m
# behind that, so there is plenty of room between the two.
# Only the *plane* moves. The rotation origin has to stay on the steering axis:
# shifting it sideways turns the rotation into a rotation plus a translation,
# which slides the whole front end off the frame.
cut_origin = [head[k] - forward[k] * args.cut for k in range(3)]


def on_the_front(point):
    """Does this point turn with the bars?

    Forward of the steering-head cut, or bar-mounted hardware. The second test
    exists because levers, perches, switchgear and grip ends sit *behind* the
    steering axis -- bars sweep back -- so no plane through the head catches
    them without also taking the fuel tank. They are separated cleanly by
    height instead: on this bike the tank tops out at y=0.54 and the seat at
    y=0.52, while the lowest bar fitting is at y=0.58. The rear guard is up
    there too, hence the `z` floor.
    """
    if sum((point[k] - cut_origin[k]) * forward[k] for k in range(3)) > 0.0:
        return True
    return point[1] > args.bars and point[2] > -0.2
wheel_nodes = {i for i, n in enumerate(doc["nodes"]) if "wheel_front" in (n.get("name") or "")}


def under_front_wheel(index):
    while index is not None:
        if index in wheel_nodes:
            return True
        index = parents.get(index)
    return False


# Classify whole connected islands, not individual vertices. A plane cannot
# separate a swept-back handlebar from a forward-leaning fuel tank -- the bar
# ends reach back past the tank's front edge -- so a per-vertex test shears
# parts in half: it moved a third of the radiators and a tenth of the tank
# while leaving most of the bars behind. An island is a physical part, and a
# part either turns with the bars or it does not.
touched = 0
report = {}
for node_index, node in mesh_nodes:
    world = world_of(node_index)
    inverse = invert_rigid(world)
    # Anything under the front wheel node is the wheel itself: all of it turns.
    whole = under_front_wheel(node_index)
    for prim in doc["meshes"][node["mesh"]]["primitives"]:
        positions = read_vec3(prim["attributes"]["POSITION"])
        member = [False] * len(positions)
        if whole:
            member = [True] * len(positions)
        else:
            _, groups = islands(prim)
            for group in groups:
                points = [transform(world, positions[v]) for v in group]
                reach = max(q[2] for q in points) - min(q[2] for q in points)
                if reach > 0.6:
                    # Not one part. Some materials -- cable runs, black trim --
                    # are welded into a single island spanning the whole bike,
                    # and its centroid sits under the seat however far forward
                    # the cut goes. That is why the near grip stayed behind at
                    # every cut while the bars it belongs to turned. Judge
                    # these vertex by vertex instead.
                    for v, w in zip(group, points):
                        if on_the_front(w):
                            member[v] = True
                    continue
                centre = [sum(q[k] for q in points) / len(points) for k in range(3)]
                if on_the_front(centre):
                    for v in group:
                        member[v] = True
        moved = []
        for inside, q in zip(member, positions):
            if inside:
                moved.append(transform(inverse, rotate_about(transform(world, q), head, axis, correction)))
                touched += 1
            else:
                moved.append(q)
        write_vec3(prim["attributes"]["POSITION"], moved)
        name = materials[prim["material"]] if "material" in prim else "-"
        entry = report.setdefault(name, [0, 0])
        entry[0] += sum(member)
        entry[1] += len(member)
        if "NORMAL" in prim["attributes"]:
            normals = read_vec3(prim["attributes"]["NORMAL"])
            out = []
            for inside, q in zip(member, normals):
                if inside:
                    w = transform(world, q, point=False)
                    w = rotate_about(w, head, axis, correction, is_point=False)
                    out.append(transform(inverse, w, point=False))
                else:
                    out.append(q)
            write_vec3(prim["attributes"]["NORMAL"], out)

print(f"rotated {touched} vertices")
for name, (moved_n, total_n) in sorted(report.items(), key=lambda e: -e[1][0]):
    print(f"   {name:18s} {moved_n:6d}/{total_n:6d} moved")

# Beside the package, never inside it: `package_mod.py` ships every file in
# the mod folder, and a spare copy of the model trebled the archive.
backup = args.model.parent.parent / f"{args.model.stem}.glb.before-align"
if not backup.exists():
    shutil.copy2(args.model, backup)
    print(f"kept the original at {backup.name}")
encoded = json.dumps(doc, separators=(",", ":")).encode()
encoded += b" " * (-len(encoded) % 4)
body = bytes(blob)
body += b"\0" * (-len(body) % 4)
out = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(encoded) + 8 + len(body))
out += struct.pack("<I4s", len(encoded), b"JSON") + encoded
out += struct.pack("<I4s", len(body), b"BIN\0") + body
args.model.write_bytes(out)
print(f"wrote {args.model} ({len(out)} bytes)")
