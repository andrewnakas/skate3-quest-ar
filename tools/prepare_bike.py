"""Prepare a user-supplied motorcycle GLB; never downloads or commits third-party assets.

Unlike prepare_mario_kart.py, which hardcodes the mesh indices of one specific
archive, this finds the two wheels itself: a wheel is round in side view and thin
across the bike, and the two extremes along the length are front and rear. Use
--list to see what it found, and --front/--rear to override it.

    python tools/prepare_bike.py <bike.glb|bike.zip> --list
    python tools/prepare_bike.py <bike.glb|bike.zip> sdk/examples/freestyle-mx

Scale comes from the wheelbase rather than the overall length, because bodywork
and mirrors vary far more between models than where the axles sit. The chassis
origin lands --ride-height above the axle line, roughly the swingarm pivot, and
the emitted wheel positions are suspension mountings: the host hangs each wheel
below its mounting by up to --suspension metres.

Pure standard library on purpose, so it runs without a NumPy install.
"""
import argparse, itertools, json, struct, zipfile
from pathlib import Path

IDENTITY = [[float(i == j) for j in range(4)] for i in range(4)]


def matmul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def transform(m, v):
    return [sum(m[i][k] * v[k] for k in range(3)) + m[i][3] for i in range(3)]


def det3(m):
    return (
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    )


p = argparse.ArgumentParser()
p.add_argument("source", type=Path, help="GLB, or a ZIP containing exactly one GLB")
p.add_argument("output", type=Path, nargs="?", help="Package folder to write bike.glb into")
p.add_argument("--list", action="store_true", help="Print every mesh and its bounds, then stop")
p.add_argument("--front", type=int, nargs="+", help="Mesh indices forming the front wheel")
p.add_argument("--rear", type=int, nargs="+", help="Mesh indices forming the rear wheel")
p.add_argument("--wheelbase", type=float, default=1.48, help="Metres between axles; a 250 is ~1.48")
p.add_argument("--suspension", type=float, default=0.3, help="Match vehicle.json suspension_length")
p.add_argument("--ride-height", type=float, default=0.15, help="Chassis origin above the axle line")
args = p.parse_args()

data = args.source.read_bytes()
if args.source.suffix.lower() == ".zip":
    with zipfile.ZipFile(args.source) as archive:
        names = [n for n in archive.namelist() if n.lower().endswith(".glb")]
        if len(names) != 1:
            raise SystemExit(f"Expected exactly one GLB in the archive, found {names}")
        data = archive.read(names[0])
if struct.unpack_from("<I", data, 0)[0] != 0x46546C67:
    raise SystemExit("Not a binary glTF (.glb). Export GLB with embedded textures.")
n = struct.unpack_from("<I", data, 12)[0]
gltf = json.loads(data[20 : 20 + n])
binary = data[20 + n :]

meshes, bounds, mesh_names = {}, {}, {}


def visit(index, parent):
    node = gltf["nodes"][index]
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    sx, sy, sz = node.get("scale", [1, 1, 1])
    rotation = [
        [1 - 2 * y * y - 2 * z * z, 2 * x * y - 2 * z * w, 2 * x * z + 2 * y * w],
        [2 * x * y + 2 * z * w, 1 - 2 * x * x - 2 * z * z, 2 * y * z - 2 * x * w],
        [2 * x * z - 2 * y * w, 2 * y * z + 2 * x * w, 1 - 2 * x * x - 2 * y * y],
    ]
    t = node.get("translation", [0, 0, 0])
    matrix = [
        [rotation[i][0] * sx, rotation[i][1] * sy, rotation[i][2] * sz, t[i]] for i in range(3)
    ] + [[0.0, 0.0, 0.0, 1.0]]
    matrix = matmul(parent, matrix)
    if "mesh" in node:
        mesh = node["mesh"]
        meshes[mesh] = matrix
        mesh_names[mesh] = node.get("name", "")
        points = []
        for primitive in gltf["meshes"][mesh]["primitives"]:
            accessor = gltf["accessors"][primitive["attributes"]["POSITION"]]
            points.extend(
                transform(matrix, list(corner))
                for corner in itertools.product(*zip(accessor["min"], accessor["max"]))
            )
        bounds[mesh] = (
            [min(q[i] for q in points) for i in range(3)],
            [max(q[i] for q in points) for i in range(3)],
        )
    for child in node.get("children", []):
        visit(child, matrix)


for root in gltf["scenes"][gltf.get("scene", 0)]["nodes"]:
    visit(root, IDENTITY)
if not bounds:
    raise SystemExit("No meshes found in the scene")

# Measure the bike from its own parts only. Exported models routinely carry a
# ground plane, a skybox or a bounding helper that is orders of magnitude bigger
# than the vehicle, and letting one into the extent ruins both the length-axis
# choice and every size test below.
diagonal = lambda m: sum((bounds[m][1][i] - bounds[m][0][i]) ** 2 for i in range(3)) ** 0.5
ordered = sorted(bounds, key=diagonal)
median = diagonal(ordered[len(ordered) // 2])
body = [m for m in bounds if diagonal(m) <= median * 8] or list(bounds)
oversized = sorted(set(bounds) - set(body))

# Longest horizontal axis is the bike's length; the other is its width.
extent = [
    max(bounds[m][1][i] for m in body) - min(bounds[m][0][i] for m in body) for i in range(3)
]
length_axis = 0 if extent[0] > extent[2] else 2
width_axis = 2 - length_axis

if args.list:
    for mesh, (lo, hi) in sorted(bounds.items()):
        size = [hi[i] - lo[i] for i in range(3)]
        mid = [(lo[i] + hi[i]) / 2 for i in range(3)]
        name = gltf["meshes"][mesh].get("name", "")
        print(
            f"mesh {mesh:3d} {name[:26]:26s} size {size[0]:7.3f}{size[1]:7.3f}{size[2]:7.3f}"
            f"  centre {mid[0]:7.3f}{mid[1]:7.3f}{mid[2]:7.3f}"
        )
    print(f"\nlength axis {'XYZ'[length_axis]}, bike extent {[round(e, 3) for e in extent]}")
    if oversized:
        print(f"ignored as scenery, far larger than the bike: {oversized}")
    raise SystemExit(0)


def wheel_like(mesh):
    """Round in side view, thin across the bike, and not a tiny fastener."""
    lo, hi = bounds[mesh]
    size = [hi[i] - lo[i] for i in range(3)]
    diameter = max(size[1], size[length_axis])
    # Reject fasteners and trim, comparing against the bike's length rather than
    # its height: a wheel is a big fraction of the length but can be well under
    # half the height once bars and screen are included.
    if diameter < extent[length_axis] * 0.12:
        return False
    roundness = abs(size[1] - size[length_axis]) / diameter
    return roundness < 0.2 and size[width_axis] < diameter * 0.7


def group_at(seed):
    """The tyre plus everything sitting inside it: rim, spokes, hub, disc."""
    lo, hi = bounds[seed]
    inside = lambda l, h: all(l[i] >= lo[i] - 1e-4 and h[i] <= hi[i] + 1e-4 for i in range(3))
    return [m for m, (l, h) in bounds.items() if inside(l, h)]


def named(fragment):
    return [m for m, n in mesh_names.items() if fragment in n.lower()]


if args.front and args.rear:
    front_group, rear_group = args.front, args.rear
elif named("wheel_front") and named("wheel_rear"):
    # Names beat geometry. Which end is the front cannot be inferred reliably --
    # a +Y-forward model exported Y-up lands facing -Z -- so when a preparation
    # step has already labelled the wheels, trust the labels.
    front_group, rear_group = named("wheel_front"), named("wheel_rear")
else:
    candidates = [m for m in body if wheel_like(m)]
    if len(candidates) < 2:
        raise SystemExit(
            f"Found {len(candidates)} wheel-shaped meshes; rerun with --list and pass "
            "--front/--rear explicitly, or split the wheels into their own objects in Blender."
        )
    centre_of = lambda m: (bounds[m][0][length_axis] + bounds[m][1][length_axis]) / 2
    front_group = group_at(max(candidates, key=centre_of))
    rear_group = group_at(min(candidates, key=centre_of))

groups = [("wheel_front", front_group, True), ("wheel_rear", rear_group, False)]


def axle(group):
    lo = [min(bounds[m][0][i] for m in group) for i in range(3)]
    hi = [max(bounds[m][1][i] for m in group) for i in range(3)]
    return [(lo[i] + hi[i]) / 2 for i in range(3)], (hi[1] - lo[1]) / 2


front_centre, front_radius = axle(front_group)
rear_centre, rear_radius = axle(rear_group)
measured = abs(front_centre[length_axis] - rear_centre[length_axis])
if measured < 1e-3:
    raise SystemExit("The two wheels are at the same place; pass --front/--rear explicitly")
scale = args.wheelbase / measured

# Flatten authored transforms, scale by the wheelbase, face +Z, and put the origin
# on the centreline `ride_height` above the axle line. Build the basis as a real
# rotation rather than swapping two axes: a bare swap has a negative determinant,
# which mirrors the model and inverts its triangle winding.
up = [0.0, 1.0, 0.0]
forward = [0.0, 0.0, 0.0]
forward[length_axis] = 1.0 if front_centre[length_axis] > rear_centre[length_axis] else -1.0
# +X is driver-left, given +Y up and +Z forward.
right = [
    up[1] * forward[2] - up[2] * forward[1],
    up[2] * forward[0] - up[0] * forward[2],
    up[0] * forward[1] - up[1] * forward[0],
]
mid = [(front_centre[i] + rear_centre[i]) / 2 for i in range(3)]
basis = [[c * scale for c in row] for row in (right, up, forward)]
normalize = [
    basis[i]
    + [-sum(basis[i][k] * mid[k] for k in range(3)) - (args.ride_height if i == 1 else 0.0)]
    for i in range(3)
] + [[0.0, 0.0, 0.0, 1.0]]
assert det3(normalize) > 0, "normalize must not mirror the model"

nodes, wheels = [{"name": "bike", "children": []}], []
wheel_meshes = {m for _, group, _ in groups for m in group}


def leaf(mesh, parent, matrix):
    i = len(nodes)
    # glTF node matrices are column-major.
    nodes.append(
        {
            "name": f"bike_mesh_{mesh}",
            "mesh": mesh,
            "matrix": [matrix[r][c] for c in range(4) for r in range(4)],
        }
    )
    nodes[parent].setdefault("children", []).append(i)


for mesh, matrix in meshes.items():
    if mesh not in wheel_meshes:
        leaf(mesh, 0, matmul(normalize, matrix))
for name, group, steering in groups:
    centre, radius = axle(group)
    centre = transform(normalize, centre)
    radius *= scale
    index = len(nodes)
    nodes.append({"name": name, "translation": list(centre), "children": []})
    nodes[0]["children"].append(index)
    inverse = [row[:] for row in IDENTITY]
    for i in range(3):
        inverse[i][3] = -centre[i]
    for mesh in group:
        leaf(mesh, index, matmul(inverse, matmul(normalize, meshes[mesh])))
    wheels.append(
        {
            "node": name,
            # A suspension mounting, not the axle: the host hangs the wheel below it.
            "position": [0.0, round(args.suspension - args.ride_height, 4), round(centre[2], 4)],
            "radius": round(radius, 4),
            "steering": steering,
            "driven": not steering,
        }
    )

gltf["nodes"] = nodes
gltf["scenes"] = [{"nodes": [0]}]
gltf["scene"] = 0
gltf.pop("animations", None)
gltf.pop("skins", None)
encoded = json.dumps(gltf, separators=(",", ":")).encode()
encoded += b" " * ((-len(encoded)) % 4)
result = (
    struct.pack("<III", 0x46546C67, 2, 20 + len(encoded) + len(binary))
    + struct.pack("<II", len(encoded), 0x4E4F534A)
    + encoded
    + binary
)
report = {
    "wheels": wheels,
    "bytes": len(result),
    "scale_applied": round(scale, 5),
    "ignored_as_scenery": oversized,
    "front_meshes": sorted(front_group),
    "rear_meshes": sorted(rear_group),
}
if args.output:
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "bike.glb").write_bytes(result)
    if len(result) > 32 * 1024 * 1024:
        report["warning"] = "Over the host's 32 MiB model limit; decimate or shrink textures."
else:
    report["note"] = "No output folder given, so nothing was written."
print(json.dumps(report, indent=2))
