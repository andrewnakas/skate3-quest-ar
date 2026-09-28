"""Incrementally extract the owned session-marker HUD into a private overlay.

Uses the existing UI toolkit's bounds-checked APT/GEO/RX2/font readers. No game
process, captures, font substitution or redrawn artwork is involved.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import sys
import importlib

# Presentation calibration against the supplied retail HUD captures. These are
# host compositor adjustments, not recovered APT constants. Keep the original
# geometry, RGB artwork and foreground font untouched.
PANEL_OPACITY = 0.75
SHADOW_COVERAGE_GAMMA = 2.2


def compile_hud(cache_root: Path, output: Path, toolkit="vendor.skate3_ui") -> None:
    scene = importlib.import_module(toolkit + ".scene_graph")
    AssetCache, SceneFlattener, transform_point = scene.AssetCache, scene.SceneFlattener, scene.transform_point
    measure_bitmap_text = importlib.import_module(toolkit + ".bitmap_font").measure_bitmap_text
    playback_frame = importlib.import_module(toolkit + ".timeline").playback_frame
    cache = AssetCache(cache_root)
    bundle = "data/fe/source/screens/hud2/hudphonelist"
    labels = json.loads((cache_root / "metadata/languages/english_global.json").read_text())
    language = {e["label"]: e["value"] for e in labels["entries"] if "label" in e}

    def state(path, owner, character):
        if path == "/mSkatePark": return {"visible": False}
        if character["type_name"] in ("sprite", "animation"):
            count = character["movie"]["frame_count"]
            if owner["key"] == bundle:
                label = {0: "hudintro", 17: "maximized", 16: "3"}.get(character["id"])
                if label: return {"frame": playback_frame(character, label, play=True)}
            if count <= 1: return {"frame": 0}
        if character["type_name"] == "text" and "/text2/" in path:
            return {"text": language["ID_PHONELIST_OBJECTDROPPER"]}
        return {}

    flat = SceneFlattener(cache, state).flatten(bundle, 0)
    if flat["unresolved"]: raise ValueError(flat["unresolved"])
    output.mkdir(parents=True, exist_ok=True)
    textures = []
    texture_ids = {}

    def texture(path, width=None, height=None, *, shadow=False):
        path = str(path)
        key = (path, shadow)
        if key in texture_ids: return texture_ids[key]
        src = cache_root / path
        data = src.read_bytes()
        if width is None or height is None or len(data) != width * height * 4:
            raise ValueError(f"Invalid source RGBA texture: {path}")
        if shadow:
            # Preserve the packaged blur profile. Compensate its coverage for
            # linear-light black blending: 1-(1-a)^gamma. No blur/redrawn text.
            pixels = bytearray(data)
            for i in range(3, len(pixels), 4):
                pixels[i] = round(255 * (1 - (1 - pixels[i] / 255) ** SHADOW_COVERAGE_GAMMA))
            data = bytes(pixels)
        index = len(textures)
        name = f"texture-{index}.rgba"
        (output / name).write_bytes(data)
        texture_ids[key] = index
        textures.append({"file": name, "width": width, "height": height,
            "source": path, "sha256": hashlib.sha256(src.read_bytes()).hexdigest(),
            "output_sha256": hashlib.sha256(data).hexdigest(),
            "shadow_coverage_gamma": SHADOW_COVERAGE_GAMMA if shadow else None})
        return index

    buttons_dir = Path("assets/data/fe/source/images/buttons/xbox360/buttons")
    if not (cache_root/buttons_dir/'manifest.json').is_file():
        manifest = json.loads((cache_root/'manifest.json').read_text())
        errors = manifest.get('errors', [])
        raise RuntimeError('Session-marker Xbox button textures could not be extracted. '
                           + ('; '.join(errors) if errors else 'The owned UI archives do not contain the required Xbox button texture bank.'))
    buttons = json.loads((cache_root / buttons_dir / "manifest.json").read_text())["textures"]
    meshes = []
    for primitive in flat["primitives"]:
        tex = primitive.get("texture")
        index = texture(tex["rgba"], tex["width"], tex["height"]) if tex else None
        role = "art"
        primitive["color"][3] *= PANEL_OPACITY if "mButtonRender" not in primitive["path"] else 1
        if "mButtonRender" in primitive["path"]:
            role = ("return" if "/mButton0/" in primitive["path"] else
                    "place" if "/mButton1/" in primitive["path"] else "dropper")
            name = {"return": "button_DPad_Up_hud.Texture",
                    "place": "button_DPad_Down_hud.Texture", "dropper": "button_B_hud.Texture"}[role]
            item = next(t for t in buttons if t["name"] == name)
            index = texture(buttons_dir / item["rgba_file"], item["width"], item["height"])
            # RenderButton 825DF618 replaces the placeholder: full texture size,
            # with (dimension - 48)/2 offset before +/- dimension/2.
            corners = [(-24, -24, 0, 0), (item["width"]-24, -24, 1, 0),
                       (item["width"]-24, item["height"]-24, 1, 1),
                       (-24, item["height"]-24, 0, 1)]
            vertices = [{"position": transform_point(primitive["matrix"], [x, y]), "uv": [u, v]}
                        for x, y, u, v in corners]
            primitive["triangles"] = [[vertices[i] for i in indices] for indices in ((0,1,2),(0,2,3))]
            primitive["color"] = [1, 1, 1, primitive["color"][3]]
            if role == "dropper": primitive["color"][3] *= 0.3
        meshes.append({"texture": index, "role": role, "color": primitive["color"],
            "vertices": [dict(position=v["position"], uv=v.get("uv", [0, 0]))
                         for tri in primitive["triangles"] for v in tri],
            "order": primitive["draw_order"], "source": primitive["path"]})
    def emit_text(text, value, font, tint, offset, role):
        definition = font["definition"]
        metrics = measure_bitmap_text(definition, value, text["font_height"])
        size = definition["textures"][0]
        index = texture(font["texture"], size["width"], size["height"], shadow=offset == 0)
        atlas = textures[index]
        glyphs = {g["glyph_index"]: g for g in definition["glyphs"]}
        vertices = []
        scale = metrics["scale"]
        for used in metrics["glyphs"]:
            g = glyphs[used["glyph_index"]]
            x = text["bounds"][0] + 2 + used["left"] + offset
            y = text["bounds"][1] + 2 + text["font_height"] - g["y_offset"] * scale
            w, h = g["width"] * scale, g["height"] * scale
            a, b, c, d = g["atlas_bounds"]
            corners = [(x,y,a,b), (x+w,y,c,b), (x+w,y+h,c,d), (x,y+h,a,d)]
            for corner in (0,1,2,0,2,3):
                px,py,u,v = corners[corner]
                vertices.append({"position": transform_point(text["matrix"], [px,py]),
                    "uv": [u/atlas["width"],v/atlas["height"]]})
        meshes.append({"texture": index, "role": role, "color": tint,
            "vertices": vertices, "order": text["draw_order"] + offset * 0.1,
            "source": text["path"], "font_pass": "shadow" if offset == 0 else "foreground"})

    for text in flat["text"]:
        value = language.get(text["value"], text["value"])
        if value.startswith("ID_"): raise ValueError(f"Missing retail text {value}")
        role = ("return" if "/text0/" in text["path"] else
                "place" if "/text1/" in text["path"] else "dropper")
        argb = int(text["color_argb"].lstrip("#"), 16)
        color = [((argb >> n) & 255)/255 for n in (16,8,0)] + [text["alpha"]*((argb>>24)&255)/255]
        if role == "dropper": color[3] *= 0.3
        # AllocateString 825D6B68 / SkateAptString::Render 82CA1FD8:
        # Futura Shadow is black, then futuraheavy in the text color at x+1.
        passes = [(text["font_asset"], [0, 0, 0, color[3]], 0),
                  (cache.font_asset("Futura Std Medium"), color, 1)]
        for font, tint, offset in passes:
            if font is None: raise ValueError("Missing original futuraheavy font")
            emit_text(text, value, font, tint, offset, role)

    manifest = {"version": 1, "canvas": [1280,720], "textures": textures,
        "meshes": sorted(meshes, key=lambda m:m["order"]),
        "source_manifest_sha256": hashlib.sha256((cache_root/"manifest.json").read_bytes()).hexdigest(),
        "timelines": {"hudintro": [1,14], "hudoutro": [15,30], "maximized": [27,49], "3": [9,18]},
        "presentation": {"panel_opacity": PANEL_OPACITY, "shadow_coverage_gamma": SHADOW_COVERAGE_GAMMA},
        "notes": "Authored three-row display list; native RenderButton dimensions and dual font passes. Panel opacity and shadow coverage compensate the host compositor against reference screenshots; they are not recovered native constants. Object Dropper is unavailable; its 0.3 opacity is a host presentation choice."}
    (output/"hud.json").write_text(json.dumps(manifest, indent=2)+"\n")



def prepare(game: Path, output: Path, toolkit="vendor.skate3_ui"):
    extract_project = importlib.import_module(toolkit + ".project").extract_project
    cache = output / "source-cache"
    extract_project(game, cache, include_dynamic=True, prefixes=(
        "data/fe/source/screens/hud2/hudphonelist", "data/fe/source/controls/button_item2",
        "data/fe/source/images/buttons/xbox360"), update=True)
    compile_hud(cache, output, toolkit)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game", type=Path, required=True)
    parser.add_argument("--ui-toolkit", type=Path, help="Optional external toolkit; defaults to the vendored extractor")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.resolve().is_relative_to(args.game.resolve()):
        parser.error("Output must be outside the owned game source")
    if args.ui_toolkit:
        sys.path.insert(0, str(args.ui_toolkit.resolve()))
    prepare(args.game, args.output, "skate3_ui_extract" if args.ui_toolkit else "vendor.skate3_ui")
    print(f"Private original session-marker HUD: {args.output / 'hud.json'}")


if __name__ == "__main__": main()
