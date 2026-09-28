"""Summarize this engine's line-oriented Chrome trace JSON without loading all events.

Usage: python tools/analyse_performance_trace.py CAPTURE.json > summary.json
CPU durations are inclusive; totals across nested/parallel spans are not frame time.
"""
import argparse
from array import array
from collections import defaultdict
import json
import math


def stats(values):
    ordered = sorted(values)
    if not ordered:
        return {"count": 0}
    def percentile(p):
        return ordered[max(0, math.ceil(p * len(ordered)) - 1)]
    return {"count": len(ordered), "total_ms": sum(ordered),
            "mean_ms": sum(ordered) / len(ordered), "p50_ms": percentile(.5),
            "p95_ms": percentile(.95), "p99_ms": percentile(.99), "max_ms": ordered[-1]}


def analyse(path):
    spans = defaultdict(lambda: array("d"))
    frames = array("d")
    render_durations = defaultdict(lambda: array("d"))
    metadata = {}
    resources = {}
    active_begin = None
    active_end = None
    count = 0
    decoder = json.JSONDecoder()
    with open(path, encoding="utf-8") as stream:
        if stream.readline().strip() != '{"traceEvents":[':
            raise ValueError("Expected a native engine recorder file (one event per line)")
        for line in stream:
            event, _ = decoder.raw_decode(line.lstrip())
            count += 1
            name = event.get("name", "")
            if name in {"build", "loaded_map", "capture_complete", "adapter"}:
                metadata[name] = event.get("args", {})
            if event.get("ph") == "X":
                spans[(event.get("cat", ""), name)].append(event["dur"] / 1000)
                ts = event["ts"]
                active_begin = ts if active_begin is None else min(active_begin, ts)
                active_end = max(active_end or 0, ts + event["dur"])
            if name == "frame_interval_ms":
                frames.append(event["args"]["value"])
            if event.get("ph") == "C" and name.startswith("render/") and name.endswith(("/elapsed_gpu", "/elapsed_cpu")):
                render_durations[name].append(event["args"]["value"])
            if name in {"assets", "render_resources", "configuration", "game_configuration"}:
                resources.setdefault(name, {"first": event["args"], "last": event["args"], "samples": 0})
                resources[name]["last"] = event["args"]
                resources[name]["samples"] += 1
    complete = metadata.get("capture_complete", {})
    warnings = []
    if not complete:
        warnings.append("No completion marker: capture may be incomplete")
    if complete.get("reason") == "size_limit":
        warnings.append("Capture reached its byte limit before the requested duration")
    if complete.get("dropped_events", 0):
        warnings.append("Events were lost to the bounded writer queue")
    if any("Enable the debug feature" in name for _, name in spans):
        warnings.append("Release system names were unavailable; system attribution is invalid")
    if complete.get("min_span_us", 0):
        warnings.append("Short CPU slices are filtered; span totals/counts exclude them")
    if not metadata.get("build", {}).get("gpu_requested", False):
        warnings.append("CPU-only capture: no GPU execution durations were requested")
    elif not any(name.endswith("/elapsed_gpu") for name in render_durations):
        warnings.append("GPU diagnostics requested but no GPU duration samples found")
    results = [{"category": cat, "name": name, **stats(values)} for (cat, name), values in spans.items()]
    results.sort(key=lambda row: row["total_ms"], reverse=True)
    return {"schema": 1, "event_count": count, "metadata": metadata, "warnings": warnings,
            "active_cpu_span_window_ms": None if active_begin is None else (active_end-active_begin)/1000,
            "frame_interval": stats(frames), "resource_samples": resources,
            "cpu_spans_inclusive": results,
            "render_pass_duration_counters": {name: stats(values) for name, values in render_durations.items()},
            "interpretation": "Main-frame intervals are not display scanout times. CPU slices include waits; nested/parallel totals must not be summed into frame time. GPU support is not proof of GPU timing measurement."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture")
    options = parser.parse_args()
    print(json.dumps(analyse(options.capture), indent=2))
