"""Package final Blue Water validation with an offline screenshot comparison."""
import argparse
import base64
from datetime import datetime
from hashlib import sha256
import json
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "artifacts" / "blue-water"
PAIRS = {
    "environment-full-release-repeat": "Complete environment: above water (repeat)",
    "environment-submerged-full-release": "Complete environment: submerged",
    "sky-full-release": "Atmosphere: clouds disabled",
    "cloud-full-release": "Clouds: original atmosphere and ocean",
}
EXTRA = ("environment-full-release", "ocean-underwater-full-release", "ocean-submerged-full-release", "environment-runtime-final")
LOGS = ("blue-water-environment-release-build-final.log", "blue-water-final-library-tests.log",
        "blue-water-final-client-tests.log", "blue-water-final-perf-schema-tests.log",
        "blue-water-final-optics-tests.log")


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def viewer(reports):
    cases = []
    for name, title in PAIRS.items():
        report = reports[name]
        measurements = sorted(report["measurements"], key=lambda item: item["backend"])
        cases.append({"title": title, "camera": report["exact_screenshot_camera"],
            "pictures": ["data:image/png;base64," + base64.b64encode(
                Path(item["screenshot"]).read_bytes()).decode("ascii") for item in measurements],
            "measurements": measurements})
    return """<!doctype html><html lang="en"><meta charset="utf-8">
<title>Blue Water integration comparison</title><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
body{font:16px/1.5 system-ui;background:#101925;color:#e6edf8;max-width:1200px;margin:24px auto;padding:0 16px}
h1{font-size:26px}p{color:#bac9da}select{font:inherit;padding:8px;background:#22354e;color:inherit}
.stage{position:relative;aspect-ratio:16/9;margin-top:18px;border:1px solid #52677f;overflow:hidden;background:#000}
.stage img{position:absolute;width:100%;height:100%;object-fit:contain;top:0;left:0}
#after{clip-path:inset(0 50% 0 0)}.badge{position:absolute;top:8px;padding:4px 8px;background:#000b}
input{width:100%;margin:12px 0}table{border-collapse:collapse;width:100%}
td,th{border-bottom:1px solid #30435c;padding:8px;text-align:right}td:first-child,th:first-child{text-align:left}
code{font-size:13px;overflow-wrap:anywhere}
</style><h1>Blue Water integration comparison</h1>
<p>Experimental promoted-water, atmosphere and cloud paths in costadelsol. Each pair uses one full-release binary and exactly matching camera metadata.</p>
<label for="case">Comparison </label><select id="case"></select>
<div class="stage"><img id="before" alt="Backend 0"><img id="after" alt="Backend 1">
<span class="badge" style="left:8px">Backend 1</span><span class="badge" style="right:8px">Backend 0</span></div>
<label for="split">Reveal backend 1</label><input id="split" type="range" min="0" max="100" value="50">
<table><thead><tr><th>Measurement</th><th>Backend 0</th><th>Backend 1</th></tr></thead><tbody id="metrics"></tbody></table>
<p><small>FPS is a 20-second average. Timings are medians of 180 CSV frames. GPU timestamps are asynchronous; sub-intervals overlap World and must not be summed. One run per mode; background CPU load was not controlled. Memory observations are system-wide.</small></p>
<p><small>Underwater backend 1 retains geometry hidden by legacy absorption culling and is substantially slower here. Authored/secondary water and baked map lighting remain host paths. Object-shadow, wake, hull and shore inputs without matching JKA data remain neutral. Complete standalone-renderer image parity is not claimed.</small></p>
<p>Exact camera: <code id="camera"></code></p><script>const cases=""" + json.dumps(cases).replace("</", r"<\/") + """;
const select=document.getElementById('case'),slider=document.getElementById('split');
cases.forEach((item,index)=>{const o=document.createElement('option');o.value=index;o.textContent=item.title;select.appendChild(o)});
const fields=[['Average FPS',null],['Legacy cloud pass (ms)','gpu_latest_clouds_ms'],['CPU encode (ms)','cpu_encode_ms'],['GPU available interval (ms)','gpu_latest_available_ms'],
['Ocean simulation (ms)','gpu_latest_ocean_sim_ms'],['Ocean surface (ms)','gpu_latest_ocean_surface_ms'],
['Ocean optics (ms)','gpu_latest_ocean_optics_ms'],['Original underwater (ms)','gpu_latest_blue_water_underwater_ms'],
['Sky/cloud preparation (ms)','gpu_latest_blue_water_sky_prepare_ms'],['Sky display (ms)','gpu_latest_blue_water_sky_draw_ms'],
['Cloud image and dome (ms)','gpu_latest_blue_water_clouds_ms'],['Aerial composite (ms)','gpu_latest_blue_water_aerial_ms'],
['Water triangles','ocean_triangles']];
function update(){const item=cases[Number(select.value)];
document.getElementById('before').src=item.pictures[0];document.getElementById('after').src=item.pictures[1];
document.getElementById('camera').textContent=JSON.stringify(item.camera);const body=document.getElementById('metrics');body.replaceChildren();
fields.forEach(([label,key])=>{const row=document.createElement('tr');
[label,...item.measurements.map(m=>{const v=key?m.metrics[key]?.p50:m.fps_avg_20_seconds;return v==null?'inactive':v.toLocaleString(undefined,{maximumFractionDigits:key==='ocean_triangles'?0:3})})].forEach(v=>{const c=document.createElement('td');c.textContent=v;row.appendChild(c)});body.appendChild(row)})}
select.addEventListener('change',update);slider.addEventListener('input',()=>{document.getElementById('after').style.clipPath='inset(0 '+(100-Number(slider.value))+'% 0 0)'});
update();</script></html>"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-checkpoint", type=Path, required=True)
    source = parser.parse_args().source_checkpoint.resolve()
    if source.parent != OUT.resolve() or source.suffix != ".zip":
        parser.error("Source checkpoint must be a ZIP in artifacts/blue-water")
    binary_hash = digest(ROOT / "target/release/DinurdoJK.exe")
    reports, files = {}, {}
    for name in (*PAIRS, *EXTRA):
        root = ROOT / "target/blue-water-ab" / name
        manifest = json.loads((root / "results.json").read_text(encoding="utf-8-sig"))
        runs = manifest if isinstance(manifest, list) else [manifest]
        if any(r["exit_code"] or r["gpu_error"] or not r["camera_reached"] for r in runs):
            raise ValueError("Failed run: " + name)
        if name in PAIRS:
            report = json.loads((root / "summary.json").read_text(encoding="utf-8"))
            if report["build_profile"] != "full-release" or report["executable_sha256"] != binary_hash:
                raise ValueError("Release mismatch: " + name)
            if {m["backend"] for m in report["measurements"]} != {0, 1}:
                raise ValueError("Incomplete pair: " + name)
            reports[name] = report
        if name == "environment-runtime-final":
            log = Path(runs[0]["stdout"]).read_text(encoding="utf-8")
            for label in ("restored_jka", "restored_blue_water", "blue_water_msaa4", "blue_water_ldr",
                          "blue_water_hdr", "blue_water_submerged", "blue_water_waterline", "blue_water_return_above"):
                if "label=" + label + " " not in log:
                    raise ValueError("Missing runtime step: " + label)
        for pattern in ("*.log", "*.json", "base/screenshots/*.png", "base/renderperf/*.csv",
                        "base/renderperf/*.json", "base/blue_water_ab.cfg", "base/blue_water_validation.cfg"):
            for path in root.glob(pattern):
                files["runs/" + name + "/" + path.relative_to(root).as_posix()] = path
    for name in LOGS:
        files["logs/" + name] = ROOT / "target" / name
    report_path = OUT / "comparison.html"
    report_path.write_text(viewer(reports), encoding="utf-8")
    files["comparison.html"] = report_path
    provenance = {"source_checkpoint": source.name, "source_sha256": digest(source),
        "release_executable_sha256": binary_hash,
        "files": {name: digest(path) for name, path in sorted(files.items())}}
    archive = OUT / ("environment-validation-" + datetime.now().strftime("%Y%m%d-%H%M%S") + ".zip")
    with ZipFile(archive, "w", ZIP_DEFLATED, compresslevel=6) as bundle:
        for name, path in sorted(files.items()):
            bundle.write(path, name)
        bundle.writestr("provenance.json", json.dumps(provenance, indent=2) + "\n")
    archive_hash = digest(archive)
    archive.with_suffix(".zip.sha256").write_text(archive_hash + "  " + archive.name + "\n", encoding="ascii")
    latest = {k: provenance[k] for k in ("source_checkpoint", "source_sha256", "release_executable_sha256")}
    latest.update({"validation_archive": archive.name, "validation_sha256": archive_hash,
                   "comparison_report": report_path.name})
    (OUT / "latest.json").write_text(json.dumps(latest, indent=2) + "\n", encoding="utf-8")
    (OUT / "LATEST.md").write_text("# Blue Water checkpoint\n\n"
        + f"- [Offline A/B viewer]({report_path.name})\n"
        + f"- [Cumulative source checkpoint]({source.name})\n"
        + f"- [Captures, timings and logs]({archive.name})\n"
        + f"- Release executable SHA-256: {binary_hash}\n"
        + f"- Source SHA-256: {latest['source_sha256']}\n"
        + f"- Validation SHA-256: {archive_hash}\n", encoding="utf-8")
    with ZipFile(archive) as bundle:
        for name, expected in provenance["files"].items():
            if sha256(bundle.read(name)).hexdigest() != expected:
                raise ValueError("Archived file mismatch: " + name)
    print(json.dumps(latest, indent=2))


if __name__ == "__main__":
    main()
