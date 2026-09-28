"""Correlate Skate 2 layer-page channels with extracted surface normals."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy
from PIL import Image


def _sample_vertices(
    pixels: numpy.ndarray,
    uvs: numpy.ndarray,
) -> numpy.ndarray:
    height, width, _channels = pixels.shape
    coordinates = numpy.abs(uvs)
    xs = numpy.rint(coordinates[:, 0] * (width - 1)).astype(numpy.int32)
    ys = numpy.rint((1.0 - coordinates[:, 1]) * (height - 1)).astype(
        numpy.int32
    )
    xs = numpy.clip(xs, 0, width - 1)
    ys = numpy.clip(ys, 0, height - 1)
    samples = []
    for x, y in zip(xs, ys, strict=True):
        x0 = max(0, x - 2)
        x1 = min(width, x + 3)
        y0 = max(0, y - 2)
        y1 = min(height, y + 3)
        samples.append(pixels[y0:y1, x0:x1].max(axis=(0, 1)))
    return numpy.asarray(samples, dtype=numpy.float32)


def analyze(cache_root: Path, limit: int) -> dict[str, object]:
    manifest = json.loads(
        (cache_root / "manifest.json").read_text(encoding="utf-8")
    )
    aliases = json.loads(
        (cache_root / "lightmap_aliases.json").read_text(encoding="utf-8")
    )["aliases"]
    confusion = numpy.zeros((3, 3), dtype=numpy.int64)
    weighted = numpy.zeros((3, 3), dtype=numpy.float64)
    accepted = 0
    image_cache: dict[str, numpy.ndarray] = {}

    for model in manifest["models"]:
        if accepted >= limit:
            break
        npz_path = (
            cache_root
            / "models"
            / f"{str(model['asset_id'])[2:]}.npz"
        )
        if not npz_path.is_file():
            continue
        with numpy.load(npz_path) as arrays:
            for mesh in model["meshes"]:
                if accepted >= limit:
                    break
                index = int(mesh["index"])
                normal_key = f"normals_{index}"
                uv_key = f"lightmap_uvs_{index}"
                if normal_key not in arrays or uv_key not in arrays:
                    continue
                values = mesh.get("retail_parameters", {}).get(
                    "lightmap", []
                )
                if not values or not values[0]:
                    continue
                alias = aliases.get(str(values[0]))
                if alias is None:
                    continue
                image_name = str(alias["png"])
                pixels = image_cache.get(image_name)
                if pixels is None:
                    pixels = numpy.asarray(
                        Image.open(cache_root / image_name).convert("RGB"),
                        dtype=numpy.float32,
                    )
                    image_cache[image_name] = pixels
                samples = _sample_vertices(pixels, arrays[uv_key])
                mean_sample = samples.mean(axis=0)
                if float(mean_sample.max()) <= 4.0:
                    continue
                mean_normal = numpy.abs(arrays[normal_key]).mean(axis=0)
                normal_axis = int(numpy.argmax(mean_normal))
                channel = int(numpy.argmax(mean_sample))
                confusion[normal_axis, channel] += 1
                weighted[normal_axis] += mean_sample
                accepted += 1

    return {
        "accepted_meshes": accepted,
        "confusion_normal_axis_by_layer_channel": confusion.tolist(),
        "mean_layer_rgb_by_normal_axis": (
            weighted
            / numpy.maximum(confusion.sum(axis=1), 1)[:, numpy.newaxis]
        ).round(3).tolist(),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cache_root", type=Path)
    parser.add_argument("--limit", type=int, default=5000)
    args = parser.parse_args()
    print(
        json.dumps(
            analyze(args.cache_root.resolve(), max(1, args.limit)),
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
