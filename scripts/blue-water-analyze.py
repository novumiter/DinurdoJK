"""Validate a Blue Water A/B run and summarize its measured CSV frames."""
import argparse
import csv
import json
import math
from pathlib import Path
import re
import statistics
import struct


METRICS = (
    "frame_ms", "cpu_encode_ms", "gpu_latest_available_ms", "gpu_latest_clouds_ms",
    "gpu_latest_ocean_sim_ms", "gpu_latest_ocean_surface_ms",
    "gpu_latest_ocean_optics_ms", "gpu_latest_blue_water_underwater_ms",
    "ocean_original_surfaces", "ocean_triangles",
    "gpu_latest_blue_water_sky_prepare_ms", "gpu_latest_blue_water_sky_draw_ms",
    "gpu_latest_blue_water_clouds_ms", "gpu_latest_blue_water_aerial_ms",
)


def png_camera(path: Path) -> dict:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"Not a PNG: {path}")
    at = 8
    while at + 12 <= len(data):
        size = struct.unpack_from(">I", data, at)[0]
        kind = data[at + 4:at + 8]
        payload = data[at + 8:at + 8 + size]
        at += size + 12
        if kind == b"iTXt" and payload.startswith(b"DinurdoJK-Screenshot\0"):
            parts = payload.split(b"\0", 5)
            if len(parts) != 6 or any(parts[1:5]):
                raise ValueError(f"Unsupported screenshot metadata encoding: {path}")
            metadata = json.loads(parts[5])
            fields = ("camera_origin", "view_angles", "fov", "map_name", "third_person")
            camera = {key: metadata[key] for key in fields}
            if camera["camera_origin"] is None or camera["view_angles"] is None:
                raise ValueError(f"Missing exact camera metadata: {path}")
            return camera
    raise ValueError(f"Missing screenshot metadata: {path}")


def summarize(root: Path, require_full_release: bool) -> dict:
    runs = json.loads((root / "results.json").read_text(encoding="utf-8-sig"))
    runs = runs if isinstance(runs, list) else [runs]
    comparisons = {run.get("comparison", "ocean") for run in runs}
    if len(comparisons) != 1:
        raise ValueError("Comparison axes differ")
    comparison = comparisons.pop()
    if len({run["backend"] for run in runs}) != len(runs):
        raise ValueError("Duplicate backends in run manifest")
    if len({run["camera_at_sample"] for run in runs}) != 1:
        raise ValueError("A/B cameras differ")
    if len({run.get("executable_sha256") for run in runs}) != 1:
        raise ValueError("A/B executable hashes differ")
    if require_full_release and (len(runs) != 2 or any(
        run.get("build_profile") != "full-release" for run in runs
    )):
        raise ValueError("A full-release measurement requires both backends")
    measurements = []
    exact_camera = None
    for run in runs:
        if run["exit_code"] != 0 or run["gpu_error"] or not run["camera_reached"]:
            raise ValueError(f"Failed run: {run['label']}")
        label = run["label"]
        candidates = sorted((root / "base" / "renderperf").glob(f"*-{label}.csv"))
        if not candidates:
            raise ValueError(f"Missing CSV: {label}")
        source = candidates[-1]
        with source.open(encoding="utf-8-sig", newline="") as stream:
            rows = list(csv.DictReader(stream))
        if len(rows) != 180:
            raise ValueError(f"Expected 180 measured frames, got {len(rows)}: {source}")
        axes = ("ocean", "sky", "cloud") if comparison == "environment" else (comparison,)
        if any(int(float(row[axis + "_backend"])) != run["backend"] for row in rows for axis in axes):
            raise ValueError(f"CSV backend disagrees with manifest: {source}")
        for axis in ("ocean", "sky", "cloud"):
            key = axis + "_backend"
            if key in run and key in rows[0] and any(int(float(row[key])) != run[key] for row in rows):
                raise ValueError(f"Fixed backend {axis} disagrees with manifest: {source}")
        metrics = {}
        for name in METRICS:
            values = sorted(float(row[name]) for row in rows if row.get(name)
                            and math.isfinite(float(row[name])))
            if not values:
                metrics[name] = None
                continue
            at = (len(values) - 1) * 0.95
            lo, hi = math.floor(at), math.ceil(at)
            metrics[name] = {"p50": statistics.median(values),
                             "p95": values[lo] + (values[hi] - values[lo]) * (at - lo),
                             "samples": len(values)}
        log = Path(run["stdout"]).read_text(encoding="utf-8", errors="replace")
        shot = re.search(r"^WROTE (.+\.png)\s*$", log, re.MULTILINE)
        if not shot:
            raise ValueError(f"Missing screenshot: {label}")
        screenshot = Path(shot[1].strip())
        camera = png_camera(screenshot)
        if exact_camera is not None and camera != exact_camera:
            raise ValueError("A/B floating-point camera, angles, FOV or map differ")
        exact_camera = camera
        fps = re.search(r"\[JKA PERF SAMPLE\] label=" + re.escape(label)
                        + r" fps_avg=([0-9.]+)", log)
        if not fps:
            raise ValueError(f"Missing final performance sample: {label}")
        measurements.append({"backend": run["backend"], "csv": str(source), "screenshot": str(screenshot),
                             "fps_avg_20_seconds": float(fps[1]), "metrics": metrics,
                             "system_gpu_memory_before_mib": run["total_gpu_memory_before_mib"],
                             "system_gpu_memory_peak_mib": run["total_gpu_memory_peak_mib"]})
    return {"comparison": comparison, "camera": runs[0]["camera_at_sample"],
            "exact_screenshot_camera": exact_camera,
            "build_profile": runs[0].get("build_profile", "unspecified"),
            "executable_sha256": runs[0].get("executable_sha256"),
            "notes": ["GPU timestamps are asynchronous, not necessarily the CSV CPU frame.",
                      "GPU sub-intervals overlap World; do not sum them as independent frame time.",
                      "GPU memory is a coarse system-wide observation, not process allocation."],
            "measurements": measurements}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_root", type=Path)
    parser.add_argument("--require-full-release", action="store_true")
    args = parser.parse_args()
    report = summarize(args.run_root, args.require_full_release)
    result = json.dumps(report, indent=2, allow_nan=False) + "\n"
    (args.run_root / "summary.json").write_text(result, encoding="utf-8")
    print(result)
