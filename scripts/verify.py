#!/usr/bin/env python3
"""Reproducible real-mpv test, no network and no user videos. Python 3 stdlib."""
import argparse
import collections
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time

p = argparse.ArgumentParser()
p.add_argument("--player", default="target/debug/shuffle-room")
p.add_argument("--mpv", default="mpv")
p.add_argument("--ffmpeg", default="ffmpeg")
p.add_argument("--ffprobe", default="ffprobe")
p.add_argument("--fixtures", default="fixtures")
p.add_argument("--out", default="evidence")
a = p.parse_args()
out = Path(a.out).resolve(); out.mkdir(parents=True, exist_ok=True)
fixtures = Path(a.fixtures).resolve()
base = [str(Path(a.player).resolve()), "--mpv", a.mpv, "--ffmpeg", a.ffmpeg, "--ffprobe", a.ffprobe]

def digest():
    return {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in fixtures.iterdir() if f.is_file()}

def run(folder, name, *extra, expect=True):
    log = out / f"{name}.jsonl"
    command = base + [str(folder), "--headless", "--seed", "42", "--timeout", "120", "--log", str(log), *map(str, extra)]
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=140)
    assert (result.returncode == 0) == expect, (command, result.stderr, result.stdout)
    return [json.loads(line) for line in log.read_text().splitlines()]

def measure(file):
    cmd = [a.ffmpeg, "-hide_banner", "-nostdin", "-i", str(file), "-map", "0:a:0", "-af",
           "aformat=channel_layouts=stereo,loudnorm=I=-18:TP=-2:LRA=11:print_format=json", "-f", "null", "-"]
    result = subprocess.run(cmd, capture_output=True, text=True, check=True)
    text = result.stderr
    m = json.loads(text[text.rfind("{"):text.rfind("}")+1])
    return {"lufs": float(m["input_i"]), "peak_dbtp": float(m["input_tp"])}

before_hash = digest()
capture = out / "capture"; capture.mkdir(exist_ok=True)
events = run(fixtures, "playback", "--max-clips", 16, "--capture-dir", capture)
played = [e for e in events if e["kind"] == "playing"]
finished = [e for e in events if e["kind"] == "finished"]
assert len(finished) == 16 and len(played) == 16
passes = collections.defaultdict(list)
for e in played: passes[e["pass"]].append(Path(e["path"]).name)
assert len(passes) == 2
for order in passes.values(): assert len(order) == len(set(order)) == 8
assert list(passes.values())[0] != list(passes.values())[1]
assert all(x["path"] != y["path"] for x, y in zip(played, played[1:]))
assert len([e for e in events if e["kind"] == "skipped"]) == 2
assert len([e for e in events if e["kind"] == "analyzing"]) == 10, "second pass should reuse analysis"
rows = []
for index, event in enumerate(finished[:8]):
    name = Path(event["path"]).name
    if not event["level"]["has_audio"]: continue
    original = measure(fixtures / name)
    processed = measure(capture / f"{index:04}-{name}.wav")
    assert processed["peak_dbtp"] <= -1.0, (name, processed)
    if name[:2] in ("01", "02", "03", "04"):
        expected = original["lufs"] + event["level"]["gain_db"]
        assert abs(processed["lufs"] - expected) < 0.35, (name, processed, expected)
    if "silent" in name: assert event["level"]["gain_db"] == 0
    if name == "05-silent.mp4": assert not math.isfinite(processed["peak_dbtp"])
    rows.append({"clip": name, "before": original, "after": processed, "gain_db": event["level"]["gain_db"]})
assert digest() == before_hash, "Input files changed"
balanced = [r["after"]["lufs"] for r in rows if r["clip"][:2] in ("01", "02", "03", "04")]
assert max(balanced) - min(balanced) < 1.0

with tempfile.TemporaryDirectory(prefix="shuffle-verification-") as work:
    work = Path(work)
    single = work / "single"; single.mkdir(); shutil.copy(fixtures / "01-quiet.mp4", single)
    one = run(single, "single", "--max-clips", 2)
    assert len([e for e in one if e["kind"] == "finished"]) == 2
    empty = work / "empty"; empty.mkdir()
    idle = run(empty, "empty", expect=False)
    assert any(e["kind"] == "idle" for e in idle)
    corrupt = work / "corrupt"; corrupt.mkdir(); shutil.copy(fixtures / "09-corrupt.mp4", corrupt)
    bad = run(corrupt, "all-corrupt", expect=False)
    assert len([e for e in bad if e["kind"] == "skipped"]) == 1
    changing = work / "changing"; changing.mkdir(); shutil.copy(fixtures / "01-quiet.mp4", changing)
    def add():
        time.sleep(2)
        shutil.copy(fixtures / "02-loud.mkv", changing)
    t = threading.Thread(target=add); t.start()
    rescanned = run(changing, "rescan", "--max-clips", 3, "--rescan-after", 3)
    t.join()
    assert any(Path(e["path"]).name == "02-loud.mkv" for e in rescanned if e["kind"] == "playing")
    # Synthetic cache entry verifies .url resolution -> same analysis -> real mpv.
    linked = work / "linked"; linked.mkdir()
    cache = work / "cache"; (cache / "abcdefgh_12").mkdir(parents=True)
    shutil.copy(fixtures / "01-quiet.mp4", cache / "abcdefgh_12" / "media.mp4")
    url_events = run(linked, "url-cache", "--max-clips", 1, "--cache-dir", cache, "--add-url", "https://youtu.be/abcdefgh_12")
    assert (linked / "YouTube-abcdefgh_12.url").is_file()
    assert any(e["kind"] == "finished" and e["path"].endswith(".url") for e in url_events)
    unavailable = work / "unavailable"; unavailable.mkdir()
    (unavailable / "broken.url").write_text("[InternetShortcut]\nURL=https://not-youtube.invalid/watch?v=abcdefgh_12\n")
    shutil.copy(fixtures / "01-quiet.mp4", unavailable)
    fallback = run(unavailable, "url-fallback", "--max-clips", 2)
    assert any(e["kind"] == "skipped" for e in fallback)

def finite_json(obj):
    if isinstance(obj, float) and not math.isfinite(obj): return None
    if isinstance(obj, dict): return {k: finite_json(v) for k,v in obj.items()}
    if isinstance(obj, list): return [finite_json(v) for v in obj]
    return obj
report = {"passed": True, "passes": dict(passes), "loudness": rows,
          "checks": ["16 actual mpv completions", "shuffled permutations", "no adjacent repeats", "invalid-file quarantine", "analysis cache", "original hashes unchanged", "silence preserved", "peak ceiling", "single clip looping", "empty folder", "all corrupt idle", "manual rescan discovers added video", "YouTube URL synthetic cache playback", "invalid URL fallback"],
          "limits": ["headless Linux null video and actual mpv PCM output; no speaker hardware", "no live YouTube download; synthetic cached URL only", "no native Windows/macOS run"]}
(out / "report.json").write_text(json.dumps(finite_json(report), indent=2) + "\n")
print(json.dumps(finite_json(report), indent=2))
