from __future__ import annotations

import json
from pathlib import Path, PurePosixPath
from typing import Callable

from .binary import FormatError
from .timeline import resolve_display_list


IDENTITY_MATRIX = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]


def compose_matrix(parent: list[float], local: list[float]) -> list[float]:
    pa, pb, pc, pd, ptx, pty = parent
    la, lb, lc, ld, ltx, lty = local
    return [
        pa * la + pc * lb,
        pb * la + pd * lb,
        pa * lc + pc * ld,
        pb * lc + pd * ld,
        pa * ltx + pc * lty + ptx,
        pb * ltx + pd * lty + pty,
    ]


def transform_point(matrix: list[float], point: list[float]) -> list[float]:
    return [
        matrix[0] * point[0] + matrix[2] * point[1] + matrix[4],
        matrix[1] * point[0] + matrix[3] * point[1] + matrix[5],
    ]


class AssetCache:
    def __init__(self, root: Path):
        self.root = Path(root).resolve()
        self.manifest = json.loads(
            (self.root / "manifest.json").read_text(encoding="utf-8")
        )
        if self.manifest.get("format") != "skate3-native-ui-asset-cache":
            raise FormatError(f"{self.root}: not a native UI asset cache")
        self.bundles = {
            self.normalize_bundle(record["name"]): record
            for record in self.manifest["bundles"]
        }
        self._loaded: dict[str, dict] = {}
        self.font_aliases: dict[str, dict] = {}

    def add_font_alias(self, source: str, target: str, provenance: dict) -> None:
        self.font_aliases[source] = {
            "target": target,
            "provenance": provenance,
        }

    @staticmethod
    def normalize_bundle(value: str) -> str:
        normalized = value.replace("\\", "/").lower()
        if normalized.startswith("source/"):
            normalized = "data/fe/" + normalized
        return str(PurePosixPath(normalized))

    def load_bundle(self, name: str) -> dict:
        key = self.normalize_bundle(name)
        if key in self._loaded:
            return self._loaded[key]
        record = self.bundles.get(key)
        if not record or "timeline" not in record:
            raise FormatError(f"APT bundle is absent from cache: {name}")
        apt = json.loads(
            (self.root / record["timeline"]["metadata"]).read_text(encoding="utf-8")
        )
        geo = {"shapes": []}
        if "geometry" in record:
            geo = json.loads(
                (self.root / record["geometry"]["metadata"]).read_text(
                    encoding="utf-8"
                )
            )
        textures = {"textures": []}
        if "textures" in record:
            textures = json.loads(
                (self.root / record["textures"]["manifest"]).read_text(
                    encoding="utf-8"
                )
            )
        value = {
            "key": key,
            "record": record,
            "apt": apt,
            "characters": {item["id"]: item for item in apt["characters"]},
            "imports": {
                item["character_id"]: item for item in apt.get("imports", [])
            },
            "exports": {},
            "shapes": {item["id"]: item for item in geo["shapes"]},
            "textures": textures,
        }
        for item in apt.get("exports", []):
            value["exports"].setdefault(item["name"], item["character_id"])
        self._loaded[key] = value
        return value

    def resolve_character(self, bundle_name: str, character_id: int) -> tuple[dict, dict]:
        bundle = self.load_bundle(bundle_name)
        character = bundle["characters"].get(character_id)
        if character:
            return bundle, character
        imported = bundle["imports"].get(character_id)
        if not imported:
            raise FormatError(
                f"{bundle['key']}: character {character_id} is neither local nor imported"
            )
        imported_bundle = self.load_bundle(imported["file"])
        exported_id = imported_bundle["exports"].get(imported["name"])
        if exported_id is None:
            raise FormatError(
                f"{imported_bundle['key']}: export {imported['name']!r} is absent"
            )
        character = imported_bundle["characters"].get(exported_id)
        if not character:
            raise FormatError(
                f"{imported_bundle['key']}: exported character {exported_id} is absent"
            )
        return imported_bundle, character

    def font_asset(self, family: str) -> dict | None:
        requested_family = family
        alias = self.font_aliases.get(family)
        if alias:
            family = alias["target"]
        matches = []
        for record in self.manifest["bundles"]:
            font = record.get("font")
            if font and font["family"] == family:
                matches.append(record)
        if len(matches) != 1:
            return None
        record = matches[0]
        metadata = json.loads(
            (self.root / record["font"]["metadata"]).read_text(encoding="utf-8")
        )
        texture = json.loads(
            (self.root / record["textures"]["manifest"]).read_text(encoding="utf-8")
        )["textures"][0]
        result = {
            "bundle": record["name"],
            "metadata": record["font"]["metadata"],
            "texture": (
                Path(record["textures"]["manifest"]).parent
                / texture["rgba_file"]
            ).as_posix(),
            "preview": (
                Path(record["textures"]["manifest"]).parent
                / texture["preview_file"]
            ).as_posix(),
            "definition": metadata,
        }
        if alias:
            result["requested_family"] = requested_family
            result["alias"] = alias
        return result

    def texture_for_id(self, bundle: dict, texture_id: int) -> dict:
        matches = []
        for item in bundle["textures"]["textures"]:
            basename = item["name"].replace("\\", "/").rsplit("/", 1)[-1]
            stem = basename.rsplit(".", 1)[0]
            if stem == str(texture_id):
                matches.append(item)
        if len(matches) != 1:
            raise FormatError(
                f"{bundle['key']}: expected one texture resource {texture_id}, "
                f"found {len(matches)}"
            )
        item = matches[0]
        manifest_dir = Path(bundle["record"]["textures"]["manifest"]).parent
        return {
            "resource_name": item["name"],
            "width": item["width"],
            "height": item["height"],
            "preview": (manifest_dir / item["preview_file"]).as_posix(),
            "rgba": (manifest_dir / item["rgba_file"]).as_posix(),
        }


StateProvider = Callable[[str, dict, dict], dict]


class SceneFlattener:
    def __init__(
        self,
        cache: AssetCache,
        state_provider: StateProvider,
        *,
        include_zero_alpha: bool = False,
    ):
        self.cache = cache
        self.state_provider = state_provider
        self.include_zero_alpha = include_zero_alpha
        self.primitives: list[dict] = []
        self.text: list[dict] = []
        self.unresolved: list[dict] = []
        self.draw_order = 0

    def flatten(self, bundle_name: str, character_id: int) -> dict:
        self._visit(
            bundle_name,
            character_id,
            "/",
            IDENTITY_MATRIX,
            1.0,
            [],
        )
        return {
            "primitives": self.primitives,
            "text": self.text,
            "unresolved": self.unresolved,
        }

    def _visit(
        self,
        bundle_name: str,
        character_id: int,
        path: str,
        world_matrix: list[float],
        alpha: float,
        stack: list[tuple[str, int]],
    ) -> None:
        try:
            bundle, character = self.cache.resolve_character(bundle_name, character_id)
        except FormatError as error:
            self.unresolved.append({"path": path, "reason": str(error)})
            return
        key = (bundle["key"], character["id"])
        if key in stack:
            self.unresolved.append({"path": path, "reason": "recursive character graph"})
            return
        state = self.state_provider(path, bundle, character)
        if state.get("visible") is False:
            return
        alpha *= float(state.get("alpha", 1.0))
        if alpha <= 0 and not self.include_zero_alpha:
            return

        type_name = character["type_name"]
        if type_name == "shape":
            self._shape(bundle, character, path, world_matrix, alpha)
            return
        if type_name in ("text", "static_text"):
            self._text(bundle, character, path, world_matrix, alpha, state)
            return
        if type_name not in ("sprite", "animation"):
            return

        frame_count = character["movie"]["frame_count"]
        frame = state.get("frame")
        if frame is None:
            if frame_count == 1:
                frame = 0
            elif self.include_zero_alpha and alpha <= 0:
                # Retain a stable, non-rendering placeholder for authored
                # layers whose parent colour transform currently hides them.
                # Their frame cannot affect pixels until an extracted state
                # explicitly makes the layer visible.
                frame = 0
            else:
                self.unresolved.append(
                    {
                        "path": path,
                        "bundle": bundle["key"],
                        "character_id": character["id"],
                        "reason": "multi-frame character has no extracted runtime state",
                    }
                )
                return
        try:
            display = resolve_display_list(character, int(frame))
        except FormatError as error:
            self.unresolved.append({"path": path, "reason": str(error)})
            return
        self.unresolved.extend(
            {"path": path, **item} for item in display["unresolved"]
        )

        for item in display["objects"]:
            properties = item["properties"]
            child_id = properties.get("character_id")
            matrix = properties.get("matrix")
            if child_id is None or matrix is None:
                continue
            name = properties.get("name") or f"depth_{item['depth']}"
            child_path = path.rstrip("/") + "/" + name
            try:
                child_bundle, child_character = self.cache.resolve_character(
                    bundle["key"], child_id
                )
                child_state = self.state_provider(
                    child_path, child_bundle, child_character
                )
            except FormatError as error:
                self.unresolved.append({"path": child_path, "reason": str(error)})
                continue
            matrix = list(child_state.get("matrix", matrix))
            child_alpha = alpha
            color = properties.get("color_transform_argb")
            if color:
                child_alpha *= color[0] / 255.0
            child_alpha *= float(child_state.get("alpha", 1.0))
            if child_state.get("visible") is False or (
                child_alpha <= 0 and not self.include_zero_alpha
            ):
                continue
            self._visit(
                bundle["key"],
                child_id,
                child_path,
                compose_matrix(world_matrix, matrix),
                child_alpha,
                stack + [key],
            )

    def _shape(
        self,
        bundle: dict,
        character: dict,
        path: str,
        matrix: list[float],
        alpha: float,
    ) -> None:
        shape = bundle["shapes"].get(character["id"])
        if not shape:
            self.unresolved.append(
                {
                    "path": path,
                    "reason": f"shape {character['id']} has no GEO record",
                }
            )
            return
        for unit_index, unit in enumerate(shape["units"]):
            texture = None
            if unit["render_type_name"].startswith("texture"):
                try:
                    _, bitmap = self.cache.resolve_character(
                        bundle["key"], unit["texture_id"]
                    )
                    texture = self.cache.texture_for_id(
                        bundle, bitmap["bitmap"]["texture_id"]
                    )
                except (FormatError, KeyError) as error:
                    self.unresolved.append({"path": path, "reason": str(error)})
                    continue
            triangles = []
            for triangle in unit["triangles"]:
                vertices = []
                for point in triangle:
                    vertex = {"position": transform_point(matrix, point)}
                    if texture:
                        uv_point = transform_point(unit["uv_matrix"], point)
                        vertex["uv"] = [
                            uv_point[0] / texture["width"],
                            uv_point[1] / texture["height"],
                        ]
                    vertices.append(vertex)
                triangles.append(vertices)
            self.primitives.append(
                {
                    "draw_order": self.draw_order,
                    "path": path,
                    "bundle": bundle["key"],
                    "character_id": character["id"],
                    "unit": unit_index,
                    "render_type": unit["render_type_name"],
                    "matrix": matrix,
                    "color": unit["color"][:3] + [unit["color"][3] * alpha],
                    "texture": texture,
                    "triangles": triangles,
                }
            )
            self.draw_order += 1

    def _text(
        self,
        bundle: dict,
        character: dict,
        path: str,
        matrix: list[float],
        alpha: float,
        state: dict,
    ) -> None:
        if character["type_name"] != "text":
            self.unresolved.append(
                {"path": path, "reason": "static text flattening is not implemented"}
            )
            return
        definition = character["text"]
        font_character = bundle["characters"].get(definition["font_id"])
        if not font_character or "font" not in font_character:
            self.unresolved.append(
                {"path": path, "reason": "text font declaration is absent"}
            )
            return
        family = font_character["font"]["name"]
        font_asset = self.cache.font_asset(family)
        if not font_asset:
            self.unresolved.append(
                {
                    "path": path,
                    "reason": f"no exact packaged bitmap font for {family!r}",
                }
            )
        self.text.append(
            {
                "draw_order": self.draw_order,
                "path": path,
                "bundle": bundle["key"],
                "character_id": character["id"],
                "matrix": matrix,
                "bounds": character["bounds"],
                "font_family": family,
                "font_asset": font_asset,
                "font_height": definition["font_height"],
                "alignment": definition["alignment"],
                "multiline": definition["multiline"],
                "word_wrap": definition["word_wrap"],
                "color_argb": state.get("color_argb", definition["color_argb"]),
                "alpha": alpha,
                "value": state.get("text", definition["initial_text"]),
            }
        )
        self.draw_order += 1
