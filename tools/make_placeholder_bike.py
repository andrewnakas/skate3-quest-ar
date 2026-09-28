"""Generate a placeholder motocross bike GLB, so the mod works with no download.

This is deliberately simple geometry: it exists to show orientation, lean, wheel
spin and suspension travel while the handling is tuned. Swap in a real model with
tools/prepare_bike.py and nothing else has to change.

    python tools/make_placeholder_bike.py sdk/examples/freestyle-mx

Dimensions follow sdk/examples/freestyle-mx/vehicle.json: the chassis origin sits
on the centreline 0.15 m above the axle line, wheels are at z = +/-0.74, and the
wheel nodes carry the names the definition refers to. No textures, so nothing
here can trip the host's external-URI rejection.
"""
import argparse, json, math, struct
from pathlib import Path

RIDE_HEIGHT = 0.15  # chassis origin above the axle line
AXLE_Y = -RIDE_HEIGHT
FRONT_Z, REAR_Z = 0.74, -0.74
FRONT_R, REAR_R = 0.33, 0.32

parts = []  # (positions, normals, indices, material)


def box(centre, size, material, yaw=0.0):
    cx, cy, cz = centre
    hx, hy, hz = (s / 2 for s in size)
    c, s = math.cos(yaw), math.sin(yaw)
    faces = [
        ([1, 0, 0], [(hx, -hy, -hz), (hx, hy, -hz), (hx, hy, hz), (hx, -hy, hz)]),
        ([-1, 0, 0], [(-hx, -hy, hz), (-hx, hy, hz), (-hx, hy, -hz), (-hx, -hy, -hz)]),
        ([0, 1, 0], [(-hx, hy, -hz), (-hx, hy, hz), (hx, hy, hz), (hx, hy, -hz)]),
        ([0, -1, 0], [(-hx, -hy, hz), (-hx, -hy, -hz), (hx, -hy, -hz), (hx, -hy, hz)]),
        ([0, 0, 1], [(-hx, -hy, hz), (hx, -hy, hz), (hx, hy, hz), (-hx, hy, hz)]),
        ([0, 0, -1], [(hx, -hy, -hz), (-hx, -hy, -hz), (-hx, hy, -hz), (hx, hy, -hz)]),
    ]
    positions, normals, indices = [], [], []
    for normal, corners in faces:
        base = len(positions)
        ny = normal
        # Yaw about X (the bike's pitch plane) so forks and swingarms can slope.
        rot = lambda p: (p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c)
        for p in corners:
            r = rot(p)
            positions.append((cx + r[0], cy + r[1], cz + r[2]))
            normals.append(rot(ny))
        indices += [base, base + 1, base + 2, base, base + 2, base + 3]
    parts.append((positions, normals, indices, material))


def cylinder(centre, radius, width, material, segments=24):
    """Axis along X, matching the wheel convention (+X axle, +Y steering)."""
    cx, cy, cz = centre
    hw = width / 2
    positions, normals, indices = [], [], []
    for i in range(segments + 1):
        a = i / segments * math.tau
        y, z = math.sin(a) * radius, math.cos(a) * radius
        for sx in (-hw, hw):
            positions.append((cx + sx, cy + y, cz + z))
            normals.append((0.0, math.sin(a), math.cos(a)))
    for i in range(segments):
        b = i * 2
        indices += [b, b + 1, b + 3, b, b + 3, b + 2]
    for sx, nx in ((hw, 1.0), (-hw, -1.0)):
        centre_index = len(positions)
        positions.append((cx + sx, cy, cz))
        normals.append((nx, 0.0, 0.0))
        for i in range(segments + 1):
            a = i / segments * math.tau
            positions.append((cx + sx, cy + math.sin(a) * radius, cz + math.cos(a) * radius))
            normals.append((nx, 0.0, 0.0))
        for i in range(segments):
            r = centre_index + 1 + i
            tri = [centre_index, r, r + 1]
            indices += tri if nx > 0 else tri[::-1]
    parts.append((positions, normals, indices, material))


ORANGE, BLACK, METAL, WHITE = 0, 1, 2, 3

# --- the bike, in chassis-local metres -------------------------------------
# Engine and frame mass sits low and central.
box((0.0, -0.02, -0.05), (0.20, 0.30, 0.55), METAL)
# Tank and shrouds.
box((0.0, 0.26, 0.20), (0.22, 0.20, 0.45), ORANGE)
# Seat, sloping back from the tank.
box((0.0, 0.34, -0.26), (0.16, 0.08, 0.60), BLACK)
# Rear subframe and fender.
box((0.0, 0.40, -0.60), (0.20, 0.06, 0.32), ORANGE)
# Swingarm, pivoting behind the engine down to the rear axle.
box((0.0, -0.08, REAR_Z / 2), (0.09, 0.09, 0.70), METAL, yaw=-0.06)
# Fork legs running down to the front axle.
for side in (-0.10, 0.10):
    box((side, 0.12, 0.62), (0.07, 0.72, 0.09), METAL, yaw=0.38)
# Front fender and number plate.
box((0.0, 0.30, 0.66), (0.24, 0.05, 0.36), WHITE)
# Handlebars and grips.
box((0.0, 0.62, 0.42), (0.68, 0.05, 0.05), METAL)
for side in (-0.30, 0.30):
    box((side, 0.62, 0.42), (0.10, 0.07, 0.07), BLACK)
# Footpegs, so the lean has something to read against.
for side in (-0.16, 0.16):
    box((side, -0.14, -0.10), (0.12, 0.03, 0.10), METAL)
body_parts = len(parts)

# Wheels last: each becomes its own named node the host drives.
cylinder((0.0, 0.0, 0.0), FRONT_R, 0.11, BLACK)
cylinder((0.0, 0.0, 0.0), FRONT_R * 0.45, 0.13, METAL)
front_parts = [body_parts, body_parts + 1]
cylinder((0.0, 0.0, 0.0), REAR_R, 0.14, BLACK)
cylinder((0.0, 0.0, 0.0), REAR_R * 0.45, 0.16, ORANGE)
rear_parts = [body_parts + 2, body_parts + 3]

# --- serialize --------------------------------------------------------------
buffer, views, accessors, meshes = bytearray(), [], [], []


def view(data, target):
    while len(buffer) % 4:
        buffer.append(0)
    views.append({"buffer": 0, "byteOffset": len(buffer), "byteLength": len(data), "target": target})
    buffer.extend(data)
    return len(views) - 1


for positions, normals, indices, material in parts:
    flat = [c for p in positions for c in p]
    lo = [min(p[i] for p in positions) for i in range(3)]
    hi = [max(p[i] for p in positions) for i in range(3)]
    accessors.append(
        {
            "bufferView": view(struct.pack(f"<{len(flat)}f", *flat), 34962),
            "componentType": 5126,
            "count": len(positions),
            "type": "VEC3",
            "min": lo,
            "max": hi,
        }
    )
    flat = [c for n in normals for c in n]
    accessors.append(
        {
            "bufferView": view(struct.pack(f"<{len(flat)}f", *flat), 34962),
            "componentType": 5126,
            "count": len(normals),
            "type": "VEC3",
        }
    )
    accessors.append(
        {
            "bufferView": view(struct.pack(f"<{len(indices)}H", *indices), 34963),
            "componentType": 5123,
            "count": len(indices),
            "type": "SCALAR",
        }
    )
    base = len(accessors) - 3
    meshes.append(
        {
            "primitives": [
                {
                    "attributes": {"POSITION": base, "NORMAL": base + 1},
                    "indices": base + 2,
                    "material": material,
                }
            ]
        }
    )


def pbr(name, colour, metallic, roughness):
    return {
        "name": name,
        "pbrMetallicRoughness": {
            "baseColorFactor": colour,
            "metallicFactor": metallic,
            "roughnessFactor": roughness,
        },
    }


materials = [
    pbr("orange_plastic", [0.96, 0.35, 0.05, 1.0], 0.0, 0.45),
    pbr("black_rubber", [0.06, 0.06, 0.07, 1.0], 0.0, 0.85),
    pbr("metal", [0.62, 0.64, 0.67, 1.0], 0.9, 0.35),
    pbr("white_plastic", [0.92, 0.92, 0.94, 1.0], 0.0, 0.4),
]

wheel_meshes = set(front_parts + rear_parts)
nodes = [{"name": "bike", "children": []}]
for i in range(len(parts)):
    if i in wheel_meshes:
        continue
    nodes[0]["children"].append(len(nodes))
    nodes.append({"name": f"bike_mesh_{i}", "mesh": i})
for name, group, z in (("wheel_front", front_parts, FRONT_Z), ("wheel_rear", rear_parts, REAR_Z)):
    index = len(nodes)
    nodes[0]["children"].append(index)
    nodes.append({"name": name, "translation": [0.0, AXLE_Y, z], "children": []})
    for mesh in group:
        nodes[index]["children"].append(len(nodes))
        nodes.append({"name": f"{name}_mesh_{mesh}", "mesh": mesh})

gltf = {
    "asset": {"version": "2.0", "generator": "make_placeholder_bike.py"},
    "scene": 0,
    "scenes": [{"nodes": [0]}],
    "nodes": nodes,
    "meshes": meshes,
    "materials": materials,
    "accessors": accessors,
    "bufferViews": views,
    "buffers": [{"byteLength": len(buffer)}],
}
encoded = json.dumps(gltf, separators=(",", ":")).encode()
encoded += b" " * ((-len(encoded)) % 4)
buffer.extend(b"\0" * ((-len(buffer)) % 4))
glb = (
    struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(encoded) + 8 + len(buffer))
    + struct.pack("<II", len(encoded), 0x4E4F534A)
    + encoded
    + struct.pack("<II", len(buffer), 0x004E4942)
    + bytes(buffer)
)

p = argparse.ArgumentParser()
p.add_argument("output", type=Path)
args = p.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
(args.output / "bike.glb").write_bytes(glb)
print(
    json.dumps(
        {
            "wrote": str(args.output / "bike.glb"),
            "bytes": len(glb),
            "meshes": len(meshes),
            "wheels": [
                {"node": "wheel_front", "axle": [0, AXLE_Y, FRONT_Z], "radius": FRONT_R},
                {"node": "wheel_rear", "axle": [0, AXLE_Y, REAR_Z], "radius": REAR_R},
            ],
        },
        indent=2,
    )
)
