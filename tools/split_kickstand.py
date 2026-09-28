"""Split a prepared bike's side stand into its own GLB node, so the host can
hide it while the bike is being ridden.

    python tools/split_kickstand.py sdk/examples/freestyle-mx/bike.glb --apply

A side stand is the one part of a motorcycle that is *only* correct when the
bike is parked: left in place it trails along the ground through every corner
and jump. It is also easy to find without being told where it is, because it is
the only part that is all of: on one side of the centre plane, long and thin,
and reaching down to the wheels' own contact height. Nothing else on a bike
does that -- pegs stop well short, the exhaust is up in the bodywork, and the
swingarm and chain straddle the centre.

The geometry is not copied. glTF lets several primitives share one set of
attribute accessors, so this writes a new index buffer holding just the stand's
triangles and shortens the original's to exclude them; both still point at the
same vertices.
"""
import argparse
import json
import math
import shutil
import struct
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("model", type=Path)
p.add_argument("--apply", action="store_true")
p.add_argument("--name", default="kickstand")
args = p.parse_args()

raw = args.model.read_bytes()
json_len = struct.unpack_from("<I", raw, 12)[0]
doc = json.loads(raw[20:20 + json_len])
blob = bytearray(raw[20 + json_len + 8:])
materials = [m.get("name", "?") for m in doc.get("materials", [])]
INDEX = {5121: ("<B", 1), 5123: ("<H", 2), 5125: ("<I", 4)}


def positions(accessor):
    a = doc["accessors"][accessor]
    bv = doc["bufferViews"][a["bufferView"]]
    off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride") or 12
    return [struct.unpack_from("<3f", blob, off + i * stride) for i in range(a["count"])]


def indices(accessor):
    a = doc["accessors"][accessor]
    bv = doc["bufferViews"][a["bufferView"]]
    off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    fmt, size = INDEX[a["componentType"]]
    return [struct.unpack_from(fmt, blob, off + i * size)[0] for i in range(a["count"])]


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


parents = {}
for i, node in enumerate(doc["nodes"]):
    for c in node.get("children", []):
        parents[c] = i


def world_of(i):
    m = node_matrix(doc["nodes"][i])
    return matmul(world_of(parents[i]), m) if i in parents else m


def apply(m, v):
    return tuple(sum(m[r][c] * v[c] for c in range(3)) + m[r][3] for r in range(3))


# How low the wheels reach: the stand has to come down to about here.
ground = min(
    apply(world_of(i), q)[1]
    for i, node in enumerate(doc["nodes"]) if "mesh" in node
    for prim in doc["meshes"][node["mesh"]]["primitives"]
    for q in positions(prim["attributes"]["POSITION"])
)

best = None
for node_index, node in [(i, n) for i, n in enumerate(doc["nodes"]) if "mesh" in n]:
    world = world_of(node_index)
    for pi, prim in enumerate(doc["meshes"][node["mesh"]]["primitives"]):
        pts = positions(prim["attributes"]["POSITION"])
        idx = indices(prim["indices"])
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
        for root, group in groups.items():
            if len(group) < 40:
                continue
            w = [apply(world, pts[v]) for v in group]
            xs = [q[0] for q in w]
            ys = [q[1] for q in w]
            zs = [q[2] for q in w]
            one_side = min(xs) > 0.03 or max(xs) < -0.03
            drops = min(ys) < ground + 0.06
            slim = (max(zs) - min(zs)) < 0.25 and (max(xs) - min(xs)) < 0.3
            tall = (max(ys) - min(ys)) > 0.25
            if one_side and drops and slim and tall:
                score = max(ys) - min(ys)
                if best is None or score > best[0]:
                    best = (score, node_index, pi, root, set(group), w, group)

if not best:
    raise SystemExit("No side stand found; this bike may not have one.")
_, node_index, pi, root, members, world_pts, group = best
prim = doc["meshes"][doc["nodes"][node_index]["mesh"]]["primitives"][pi]
xs = [q[0] for q in world_pts]
ys = [q[1] for q in world_pts]
zs = [q[2] for q in world_pts]
print(f"side stand: '{materials[prim['material']]}' {len(group)} verts "
      f"x[{min(xs):+.2f},{max(xs):+.2f}] y[{min(ys):+.2f},{max(ys):+.2f}] z[{min(zs):+.2f},{max(zs):+.2f}]"
      f"  (wheels reach {ground:+.2f})")
if not args.apply:
    raise SystemExit(0)

idx = indices(prim["indices"])
mine, rest = [], []
for t in range(0, len(idx) - 2, 3):
    tri = idx[t:t + 3]
    (mine if all(v in members for v in tri) else rest).append(tri)
print(f"moving {len(mine)} triangles, leaving {len(rest)}")

component = doc["accessors"][prim["indices"]]["componentType"]
fmt, size = INDEX[component]
# Shorten the original in place; the leftover bytes are simply unreferenced.
original = doc["accessors"][prim["indices"]]
bv = doc["bufferViews"][original["bufferView"]]
off = bv.get("byteOffset", 0) + original.get("byteOffset", 0)
flat = [v for tri in rest for v in tri]
for i, v in enumerate(flat):
    struct.pack_into(fmt, blob, off + i * size, v)
original["count"] = len(flat)

# The stand's own indices go on the end of the buffer.
while len(blob) % 4:
    blob.append(0)
start = len(blob)
for tri in mine:
    for v in tri:
        blob.extend(struct.pack(fmt, v))
doc["bufferViews"].append({"buffer": 0, "byteOffset": start, "byteLength": len(blob) - start,
                           "target": 34963})
doc["accessors"].append({"bufferView": len(doc["bufferViews"]) - 1, "componentType": component,
                         "count": len(mine) * 3, "type": "SCALAR"})
doc["meshes"].append({"name": args.name, "primitives": [{
    "attributes": dict(prim["attributes"]),
    "indices": len(doc["accessors"]) - 1,
    **({"material": prim["material"]} if "material" in prim else {}),
}]})
doc["nodes"].append({"name": args.name, "mesh": len(doc["meshes"]) - 1})
new_node = len(doc["nodes"]) - 1
# Same parent as the mesh it came from, so it keeps that node's transform.
holder = parents.get(node_index)
if holder is None:
    doc["scenes"][doc.get("scene", 0)]["nodes"].append(new_node)
else:
    doc["nodes"][holder].setdefault("children", []).append(new_node)
doc["buffers"][0]["byteLength"] = len(blob)

# Beside the package, never inside it: `package_mod.py` ships every file in
# the mod folder, and a spare copy of the model trebled the archive.
backup = args.model.parent.parent / f"{args.model.stem}.glb.before-kickstand"
if not backup.exists():
    shutil.copy2(args.model, backup)
encoded = json.dumps(doc, separators=(",", ":")).encode()
encoded += b" " * (-len(encoded) % 4)
body = bytes(blob) + b"\0" * (-len(blob) % 4)
out = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(encoded) + 8 + len(body))
out += struct.pack("<I4s", len(encoded), b"JSON") + encoded
out += struct.pack("<I4s", len(body), b"BIN\0") + body
args.model.write_bytes(out)
print(f"wrote {args.model}: node '{args.name}' added")
