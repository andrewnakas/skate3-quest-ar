from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path

from .apt import inspect_apt
from .bitmap_font import parse_bitmap_font
from .big import BigArchive, BigEntry
from .binary import FormatError
from .geo import parse_geo
from .language import pair_language_tables
from .rw4 import extract_textures


CORE_ARCHIVES = ("fedata.big", "fetexture.big", "miscboot.big")
DYNAMIC_ARCHIVE = "fedynamic.big"
DEFAULT_PREFIXES = (
    "data/fe/source/screens/",
    "data/fe/source/controls/",
)
UI_EXTENSIONS = {
    ".apt",
    ".bmpfont",
    ".const",
    ".geo",
    ".rx2",
    ".rps3",
    ".jpg",
    ".png",
}
SUPPORT_PATHS = {
    "data/fe/languages/english/language_english_global_skate3ng.bin",
    "data/fe/languages/labels/language_labels_global_skate3ng.bin",
}
SUPPORT_PREFIXES = ("data/fe/fonts/",)
CACHE_VERSION = 3


@dataclass(frozen=True)
class SelectedEntry:
    archive_name: str
    archive: BigArchive
    entry: BigEntry


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _write_json(path: Path, value: object, force: bool) -> None:
    if path.exists() and not force:
        raise FileExistsError(f"refusing to overwrite {path}; pass --force")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def find_big_directory(game_root: Path) -> Path:
    root = Path(game_root).resolve()
    candidates = (root / "data" / "big", root / "big", root)
    for candidate in candidates:
        if (candidate / "fedata.big").is_file():
            return candidate
    raise FileNotFoundError(
        f"could not find data/big/fedata.big beneath the supplied game root: {root}"
    )


def is_selected_ui_path(path: str, prefixes: tuple[str, ...]) -> bool:
    normalized = path.replace("\\", "/").lower()
    if normalized in SUPPORT_PATHS:
        return True
    if any(normalized.startswith(prefix) for prefix in SUPPORT_PREFIXES):
        return Path(normalized).suffix in UI_EXTENSIONS
    if Path(normalized).suffix not in UI_EXTENSIONS:
        return False
    return any(normalized.startswith(prefix.lower()) for prefix in prefixes)


def cache_is_current(
    game_root: Path,
    output: Path,
    *,
    include_dynamic: bool,
    prefixes: tuple[str, ...],
    decode_png: bool,
) -> bool:
    manifest_path = Path(output).resolve() / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if (
            manifest.get("format") != "skate3-native-ui-asset-cache"
            or manifest.get("version") != CACHE_VERSION
            or manifest.get("prefixes") != list(prefixes)
        ):
            return False
        if decode_png and manifest.get("summary", {}).get("textures_decoded", 0) == 0:
            return False
        big_directory = find_big_directory(game_root)
        expected = list(CORE_ARCHIVES)
        if include_dynamic:
            expected.append(DYNAMIC_ARCHIVE)
        records = {record["name"]: record for record in manifest.get("archives", [])}
        if set(records) != set(expected):
            return False
        for name in expected:
            source = big_directory / name
            record = records[name]
            if (
                not source.is_file()
                or source.stat().st_size != record.get("bytes")
                or _sha256(source.read_bytes()) != record.get("sha256")
            ):
                return False
        return True
    except (FileNotFoundError, KeyError, OSError, ValueError, json.JSONDecodeError):
        return False


def _safe_output_path(root: Path, archive_path: str) -> Path:
    relative = BigArchive._safe_relative(archive_path)
    target = (root / relative).resolve()
    if root != target and root not in target.parents:
        raise FormatError(f"archive path escapes output: {archive_path!r}")
    return target


def extract_project(
    game_root: Path,
    output: Path,
    *,
    include_dynamic: bool = False,
    prefixes: tuple[str, ...] = DEFAULT_PREFIXES,
    decode_png: bool = True,
    force: bool = False,
    update: bool = False,
) -> dict:
    """Build a local, exact-asset UI cache from an installed Skate 3 copy."""
    game_root = Path(game_root).resolve()
    output = Path(output).resolve()
    if game_root == output or game_root in output.parents:
        raise ValueError("output must be outside the read-only game-data tree")
    if update and cache_is_current(
        game_root,
        output,
        include_dynamic=include_dynamic,
        prefixes=prefixes,
        decode_png=decode_png,
    ):
        manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
        manifest["cache_reused"] = True
        return manifest
    if update:
        force = True
    if output.exists() and any(output.iterdir()) and not force:
        raise FileExistsError(f"{output} is not empty; pass --force to update it")
    output.mkdir(parents=True, exist_ok=True)

    big_directory = find_big_directory(game_root)
    archive_names = list(CORE_ARCHIVES)
    if include_dynamic:
        archive_names.append(DYNAMIC_ARCHIVE)

    archives: list[tuple[str, BigArchive]] = []
    archive_records = []
    for name in archive_names:
        source = big_directory / name
        if not source.is_file():
            raise FileNotFoundError(f"required UI archive is missing: {source}")
        archive = BigArchive(source)
        archives.append((name, archive))
        archive_records.append(
            {
                "name": name,
                "source": str(source),
                "bytes": source.stat().st_size,
                "sha256": _sha256(source.read_bytes()),
                "entries": len(archive.entries),
            }
        )

    # Later archives are preferred. fedynamic.big is a superset when requested,
    # while fetexture.big supplies RX2 files absent from fedata.big.
    candidates: dict[str, list[SelectedEntry]] = {}
    for archive_name, archive in archives:
        for entry in archive.entries:
            if is_selected_ui_path(entry.path, prefixes):
                key = entry.path.replace("\\", "/").lower()
                candidates.setdefault(key, []).append(
                    SelectedEntry(archive_name, archive, entry)
                )

    selected = {key: choices[-1] for key, choices in candidates.items()}
    raw_root = output / "raw"
    file_records = []
    extracted: dict[str, Path] = {}
    for key, source in sorted(selected.items()):
        data = source.archive.read(source.entry)
        target = _safe_output_path(raw_root, source.entry.path)
        if target.exists() and not force:
            raise FileExistsError(f"refusing to overwrite {target}; pass --force")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        extracted[key] = target
        file_records.append(
            {
                "path": source.entry.path,
                "archive": source.archive_name,
                "bytes": len(data),
                "sha256": _sha256(data),
                "alternate_sources": [
                    choice.archive_name for choice in candidates[key][:-1]
                ],
            }
        )

    bundles: dict[str, dict[str, Path]] = {}
    display_names: dict[str, str] = {}
    for key, path in extracted.items():
        stem = str(Path(key).with_suffix("")).replace("\\", "/")
        display_names.setdefault(
            stem, str(Path(selected[key].entry.path).with_suffix("")).replace("\\", "/")
        )
        bundles.setdefault(stem, {})[path.suffix.lower()] = path

    bundle_records = []
    errors = []
    for key, parts in sorted(bundles.items()):
        display_name = display_names[key]
        record: dict = {
            "name": display_name,
            "files": {
                suffix.lstrip("."): path.relative_to(output).as_posix()
                for suffix, path in sorted(parts.items())
            },
        }
        metadata_base = _safe_output_path(output / "metadata", display_name)
        asset_base = _safe_output_path(output / "assets", display_name)

        if ".apt" in parts and ".const" in parts:
            try:
                apt_data = inspect_apt(parts[".apt"], parts[".const"])
                apt_target = metadata_base.with_suffix(".apt.json")
                _write_json(apt_target, apt_data, force)
                movie = (apt_data.get("root") or {}).get("movie", {})
                record["timeline"] = {
                    "metadata": apt_target.relative_to(output).as_posix(),
                    "frames": movie.get("frame_count", 0),
                    "milliseconds_per_frame": movie.get("milliseconds_per_frame", 0),
                    "characters": len(apt_data.get("characters", [])),
                }
            except (FormatError, OSError, ValueError) as error:
                message = f"{display_name}: APT: {error}"
                errors.append(message)
                record.setdefault("errors", []).append(message)

        if ".geo" in parts:
            try:
                geo_data = parse_geo(parts[".geo"])
                geo_target = metadata_base.with_suffix(".geo.json")
                _write_json(geo_target, geo_data, force)
                record["geometry"] = {
                    "metadata": geo_target.relative_to(output).as_posix(),
                    "shapes": len(geo_data.get("shapes", [])),
                }
            except (FormatError, OSError, ValueError) as error:
                message = f"{display_name}: GEO: {error}"
                errors.append(message)
                record.setdefault("errors", []).append(message)

        if ".bmpfont" in parts:
            try:
                font_data = parse_bitmap_font(parts[".bmpfont"])
                font_target = metadata_base.with_suffix(".font.json")
                _write_json(font_target, font_data, force)
                record["font"] = {
                    "metadata": font_target.relative_to(output).as_posix(),
                    "family": font_data["family"],
                    "glyphs": len(font_data["glyphs"]),
                    "characters": len(font_data["characters"]),
                }
            except (FormatError, OSError, ValueError) as error:
                message = f"{display_name}: bitmap font: {error}"
                errors.append(message)
                record.setdefault("errors", []).append(message)

        arena = parts.get(".rx2") or parts.get(".rps3")
        if arena:
            try:
                texture_manifest = extract_textures(
                    arena, asset_base, force=force, decode_png=decode_png
                )
                texture_target = asset_base / "manifest.json"
                _write_json(texture_target, texture_manifest, force)
                record["textures"] = {
                    "manifest": texture_target.relative_to(output).as_posix(),
                    "platform": texture_manifest["platform"],
                    "count": len(texture_manifest["textures"]),
                    "decoded": sum(
                        "preview_file" in texture
                        for texture in texture_manifest["textures"]
                    ),
                }
                for texture in texture_manifest["textures"]:
                    if "preview_error" in texture:
                        message = (
                            f"{display_name}: texture {texture['index']}: "
                            f"{texture['preview_error']}"
                        )
                        errors.append(message)
            except (FormatError, OSError, ValueError) as error:
                message = f"{display_name}: RW4: {error}"
                errors.append(message)
                record.setdefault("errors", []).append(message)

        bundle_records.append(record)

    labels_key = (
        "data/fe/languages/labels/language_labels_global_skate3ng.bin"
    )
    english_key = (
        "data/fe/languages/english/language_english_global_skate3ng.bin"
    )
    language_record = None
    if labels_key in extracted and english_key in extracted:
        try:
            language_data = pair_language_tables(
                extracted[labels_key], extracted[english_key]
            )
            language_target = output / "metadata" / "languages" / "english_global.json"
            _write_json(language_target, language_data, force)
            language_record = {
                "metadata": language_target.relative_to(output).as_posix(),
                "entries": language_data["pool_entries"],
                "duplicate_labels": len(language_data["duplicate_labels"]),
            }
        except (FormatError, OSError, ValueError) as error:
            message = f"English global language pair: {error}"
            errors.append(message)

    manifest = {
        "format": "skate3-native-ui-asset-cache",
        "version": CACHE_VERSION,
        "notice": (
            "Exact retail assets extracted locally from the user's installed game; "
            "do not redistribute this cache."
        ),
        "game_root": str(game_root),
        "output": str(output),
        "prefixes": list(prefixes),
        "archives": archive_records,
        "selected_files": file_records,
        "bundles": bundle_records,
        "languages": {"english_global": language_record}
        if language_record
        else {},
        "errors": errors,
        "summary": {
            "files": len(file_records),
            "bundles": len(bundle_records),
            "timelines": sum("timeline" in bundle for bundle in bundle_records),
            "geometry": sum("geometry" in bundle for bundle in bundle_records),
            "bitmap_fonts": sum("font" in bundle for bundle in bundle_records),
            "languages": int(language_record is not None),
            "texture_arenas": sum("textures" in bundle for bundle in bundle_records),
            "textures_decoded": sum(
                bundle.get("textures", {}).get("decoded", 0)
                for bundle in bundle_records
            ),
            "errors": len(errors),
        },
    }
    _write_json(output / "manifest.json", manifest, force)
    return manifest
