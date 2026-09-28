"""Move non-rotating parts out of a prepared bike's wheel nodes. Stdlib only.

    python tools/unspin_wheel_parts.py sdk/examples/freestyle-mx/bike.glb
    python tools/unspin_wheel_parts.py sdk/examples/freestyle-mx/bike.glb --apply

`prepare_bike.py` finds the wheels by roundness, which sweeps up whatever sits
near them: on this model the front fork guard and a shock fitting ended up
inside `wheel_front`, so they span with the wheel. It reads as something stuck
to the tyre going round.

A real wheel part is an annulus about the axle -- a tyre, rim, hub or disc is
roughly centred on it and roughly symmetric about the axle plane. A guard or a
fork leg sits off to one side and well above. That is the test used here, and
it needs no knowledge of which model this is.

Nothing is rewritten geometrically. These are whole primitives, so they are
moved between meshes as JSON; both meshes point at the same vertex data, and
the receiving node is given the transform the wheel's mesh node already had,
which is the one that cancels the wheel pivot out.
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
args = p.parse_args()

raw = args.model.read_bytes()
json_len = struct.unpack_from("<I", raw, 12)[0]
doc = json.loads(raw[20:20 + json_len])
blob = raw[20 + json_len + 8:]
materials = [m.get("name", "?") for m in doc.get("materials", [])]


def positions(accessor):
    a = doc["accessors"][accessor]
    bv = doc["bufferViews"][a["bufferView"]]
    off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride") or 12
    return [struct.unpack_from("<3f", blob, off + i * stride) for i in range(a["count"])]


parents = {}
for i, node in enumerate(doc["nodes"]):
    for c in node.get("children", []):
        parents[c] = i


# Everything is compared in world space. The wheel's own mesh node carries a
# transform that cancels the wheel pivot out, so raw vertex coordinates and the
# pivot's translation live in different frames -- comparing them directly said
# the tyres were not part of the wheel.
def local(node):
    if "matrix" in node:
        m = node["matrix"]
        return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    sc = node.get("scale", [1, 1, 1])
    t = node.get("translation", [0, 0, 0])
    rot = [[1 - 2 * y * y - 2 * z * z, 2 * x * y - 2 * z * w, 2 * x * z + 2 * y * w],
           [2 * x * y + 2 * z * w, 1 - 2 * x * x - 2 * z * z, 2 * y * z - 2 * x * w],
           [2 * x * z - 2 * y * w, 2 * y * z + 2 * x * w, 1 - 2 * x * x - 2 * y * y]]
    return [[rot[r][c] * sc[c] for c in range(3)] + [t[r]] for r in range(3)] + [[0, 0, 0, 1]]


def mul(a, b):
    return [[sum(a[r][k] * b[k][c] for k in range(4)) for c in range(4)] for r in range(4)]


def world_of(i):
    m = local(doc["nodes"][i])
    return mul(world_of(parents[i]), m) if i in parents else m


def apply(m, v):
    return tuple(sum(m[r][c] * v[c] for c in range(3)) + m[r][3] for r in range(3))

moved_any = False
for wheel_index, wheel in enumerate(doc["nodes"]):
    name = wheel.get("name") or ""
    if "wheel" not in name:
        continue
    axle_m = world_of(wheel_index)
    axle = (axle_m[0][3], axle_m[1][3], axle_m[2][3])
    for child in wheel.get("children", []):
        node = doc["nodes"][child]
        if "mesh" not in node:
            continue
        mesh = doc["meshes"][node["mesh"]]
        keep, evict = [], []
        for prim in mesh["primitives"]:
            node_world = world_of(child)
            pts = [apply(node_world, q) for q in positions(prim["attributes"]["POSITION"])]
            # Whether the part is centred on the axle *in the wheel's own
            # plane*. Offset along the axle itself does not matter: a brake
            # disc sits well out to one side and still turns with the wheel.
            # Radial offset is what separates a tyre or rim -- which surround
            # the axle -- from a guard, a fork leg or a chain, which hang off
            # it. Distance alone is not enough either, because a tyre is a
            # thin annulus that never comes near the axle at all.
            centre = [sum(q[k] for q in pts) / len(pts) for k in range(3)]
            radial = [math.hypot(q[1] - axle[1], q[2] - axle[2]) for q in pts]
            offset = math.hypot(centre[1] - axle[1], centre[2] - axle[2])
            outer = max(radial)
            label = materials[prim["material"]] if "material" in prim else "-"
            rotates = offset < outer * 0.25
            print(f"  {name}/{label:18s} n={len(pts):6d} radius={outer:.2f} "
                  f"off-axle={offset:.2f} -> {'spins' if rotates else 'MOVE OUT'}")
            (keep if rotates else evict).append(prim)
        if not evict:
            continue
        moved_any = True
        if not args.apply:
            continue
        mesh["primitives"] = keep
        # The chassis mesh node carries the transform that puts this geometry
        # where it belongs; reuse it so nothing shifts.
        chassis = next(
            i for i, n in enumerate(doc["nodes"])
            if "mesh" in n and parents.get(i) is not None
            and (doc["nodes"][parents[i]].get("name") or "") == "bike"
        )
        doc["meshes"].append({"name": f"{name}_static", "primitives": evict})
        fixed = dict(doc["nodes"][chassis])
        fixed["name"] = f"{name}_static"
        fixed["mesh"] = len(doc["meshes"]) - 1
        fixed.pop("children", None)
        doc["nodes"].append(fixed)
        doc["nodes"][parents[chassis]].setdefault("children", []).append(len(doc["nodes"]) - 1)

if not moved_any:
    print("\nnothing to move: every part in the wheel nodes surrounds its axle.")
    raise SystemExit(0)
if not args.apply:
    print("\n(dry run; pass --apply to write)")
    raise SystemExit(0)

backup = args.model.parent.parent / f"{args.model.stem}.glb.before-unspin"
if not backup.exists():
    shutil.copy2(args.model, backup)
encoded = json.dumps(doc, separators=(",", ":")).encode()
encoded += b" " * (-len(encoded) % 4)
body = blob + b"\0" * (-len(blob) % 4)
out = struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(encoded) + 8 + len(body))
out += struct.pack("<I4s", len(encoded), b"JSON") + encoded
out += struct.pack("<I4s", len(body), b"BIN\0") + body
args.model.write_bytes(out)
print(f"\nwrote {args.model}")
