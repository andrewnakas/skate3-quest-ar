"""Re-tint named materials in a GLB, in place, without touching geometry.

A downloaded model arrives in its own livery. This rewrites baseColorFactor (and
optionally roughness/metallic) for materials matched by name, so a bike can be
put in a different colour without going back through Blender.

    python tools/recolour_glb.py sdk/examples/freestyle-mx/bike.glb --list
    python tools/recolour_glb.py sdk/examples/freestyle-mx/bike.glb \
        --set "Orange Plastics=0.03,0.03,0.035,rough=0.35" \
        --set "Orange Metal=0.05,0.05,0.055,rough=0.3"

Matching is case-insensitive substring, so "orange" hits both. Values are linear
sRGB 0..1, the same space glTF baseColorFactor uses.
"""
import argparse, json, struct
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("glb", type=Path)
p.add_argument("--list", action="store_true", help="Print materials and stop")
p.add_argument(
    "--set",
    action="append",
    default=[],
    metavar="NAME=R,G,B[,rough=X][,metal=Y]",
    help="Re-tint every material whose name contains NAME",
)
args = p.parse_args()

data = args.glb.read_bytes()
if struct.unpack_from("<I", data, 0)[0] != 0x46546C67:
    raise SystemExit("Not a binary glTF (.glb)")
json_len = struct.unpack_from("<I", data, 12)[0]
header, gltf_json, rest = data[:12], data[20 : 20 + json_len], data[20 + json_len :]
gltf = json.loads(gltf_json)
materials = gltf.get("materials", [])

if args.list or not args.set:
    for i, m in enumerate(materials):
        pbr = m.get("pbrMetallicRoughness", {})
        colour = [round(v, 3) for v in pbr.get("baseColorFactor", [1, 1, 1, 1])]
        print(
            f"{i:2d} {m.get('name', ''):24s} rgba {colour}"
            f" metal {pbr.get('metallicFactor', '-')} rough {pbr.get('roughnessFactor', '-')}"
        )
    raise SystemExit(0)

changed = []
for rule in args.set:
    name, _, spec = rule.partition("=")
    if not spec:
        raise SystemExit(f"Malformed --set {rule!r}; expected NAME=R,G,B")
    rgb, rough, metal = [], None, None
    for token in spec.split(","):
        token = token.strip()
        if token.startswith("rough="):
            rough = float(token[6:])
        elif token.startswith("metal="):
            metal = float(token[6:])
        else:
            rgb.append(float(token))
    if len(rgb) != 3 or not all(0.0 <= c <= 1.0 for c in rgb):
        raise SystemExit(f"Need three 0..1 colour components in {rule!r}")
    hits = [m for m in materials if name.strip().lower() in m.get("name", "").lower()]
    if not hits:
        raise SystemExit(f"No material matches {name!r}; run with --list")
    for m in hits:
        pbr = m.setdefault("pbrMetallicRoughness", {})
        # Preserve whatever alpha the material had; only the tint is being changed.
        alpha = pbr.get("baseColorFactor", [1, 1, 1, 1])[3]
        pbr["baseColorFactor"] = rgb + [alpha]
        if rough is not None:
            pbr["roughnessFactor"] = rough
        if metal is not None:
            pbr["metallicFactor"] = metal
        changed.append(m.get("name", "?"))

encoded = json.dumps(gltf, separators=(",", ":")).encode()
encoded += b" " * ((-len(encoded)) % 4)
out = (
    struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(encoded) + len(rest))
    + struct.pack("<II", len(encoded), 0x4E4F534A)
    + encoded
    + rest
)
args.glb.write_bytes(out)
print(json.dumps({"recoloured": changed, "bytes": len(out)}, indent=2))
