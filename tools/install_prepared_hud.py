"""Install already extracted original HUD caches into an existing asset root.

Copies only runtime manifests and referenced RGBA files, byte for byte. No game,
disc extraction, map conversion or renderer is started. Both caches are checked
before writing; existing differing files require an explicit --replace.
"""
from pathlib import Path
import argparse
import hashlib
import json
import shutil


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def contained(root, relative):
    path = (root / relative).resolve()
    if not path.is_relative_to(root.resolve()) or path == root.resolve():
        raise ValueError(f"HUD path escapes cache: {relative}")
    return path


def runtime_files(root, marker=False):
    manifest = "hud.json" if marker else "runtime/trickdisplay.json"
    data = json.loads((root / manifest).read_text(encoding="utf-8"))
    if data.get("version") != 1:
        raise ValueError(f"Unsupported HUD version: {root}")
    textures = []
    if marker:
        if data.get("canvas") != [1280, 720] or not data.get("meshes"):
            raise ValueError("Invalid session-marker HUD")
        textures = [(t["file"], t["width"], t["height"], t.get("output_sha256"))
                    for t in data["textures"]]
    else:
        if data.get("format") != "skate3-scoring-hud" or data.get("unresolved_fonts"):
            raise ValueError("Invalid scoring HUD or unresolved original fonts")
        for shape in data["shapes"].values():
            for primitive in shape:
                t = primitive["texture"]
                textures.append((t["rgba"], t["width"], t["height"], None))
        for font in data["fonts"].values():
            size = font["definition"]["textures"][0]
            textures.append((font["texture"], size["width"], size["height"], None))
    files = {manifest: digest(root / manifest)}
    for relative, width, height, expected in textures:
        path = contained(root, relative)
        if width <= 0 or height <= 0 or path.stat().st_size != width * height * 4:
            raise ValueError(f"Invalid RGBA size: {path}")
        actual = digest(path)
        if expected and expected != actual:
            raise ValueError(f"Original HUD texture hash mismatch: {path}")
        files[relative] = actual
    return files


def install(assets, scoring_cache, marker_cache, replace=False):
    plan = []
    for source, name, marker in [(scoring_cache, "hud", False),
                                  (marker_cache, "session-marker", True)]:
        if source is None:continue
        destination = assets / "private" / name
        for relative, sha256 in runtime_files(source, marker).items():
            target = contained(destination, relative)
            if target.exists() and digest(target) != sha256 and not replace:
                raise ValueError(f"Different installed HUD file (use --replace): {target}")
            plan.append((contained(source, relative), target, sha256))
    for source, target, sha256 in plan:
        if target.exists() and digest(target) == sha256:
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        if digest(target) != sha256:
            raise ValueError(f"Installed HUD hash mismatch: {target}")
    return {str(target.relative_to(assets.resolve())): sha for _, target, sha in plan}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--scoring-cache", type=Path, required=True)
    parser.add_argument("--marker-cache", type=Path, required=True)
    parser.add_argument("--replace", action="store_true")
    args = parser.parse_args()
    files = install(args.assets, args.scoring_cache, args.marker_cache, args.replace)
    print(json.dumps({"assets": str(args.assets.resolve()), "files": files}, indent=2))


if __name__ == "__main__":
    main()
