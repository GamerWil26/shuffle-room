#!/usr/bin/env python3
"""Small synthetic media only. Requires an FFmpeg build with the named encoders."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

p = argparse.ArgumentParser()
p.add_argument("--out", default="fixtures")
p.add_argument("--ffmpeg", default="ffmpeg")
a = p.parse_args()
out = Path(a.out)
out.mkdir(parents=True, exist_ok=True)

def make(name, audio, video="libx264", acodec="aac", extra=()):
    cmd = [a.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
           "testsrc2=size=320x180:rate=24:duration=4"]
    if audio:
        cmd += ["-f", "lavfi", "-i", audio, "-map", "0:v:0", "-map", "1:a:0"]
    cmd += ["-t", "4", "-c:v", video, "-threads", "2", "-pix_fmt", "yuv420p"]
    if video == "libx264": cmd += ["-preset", "ultrafast", "-crf", "30"]
    if audio: cmd += ["-c:a", acodec, "-ar", "48000", "-ac", "2"]
    cmd += list(extra) + [str(out / name)]
    subprocess.run(cmd, check=True)

# FFmpeg sine defaults to amplitude 0.125. Differences are intentional, not
# inferred from nominal codec gain: verify.py measures decoded inputs/outputs.
make("01-quiet.mp4", "sine=frequency=440:sample_rate=48000:duration=4,volume=0.18")
make("02-loud.mkv", "sine=frequency=660:sample_rate=48000:duration=4,volume=6", acodec="flac")
make("03-medium.webm", "sine=frequency=880:sample_rate=48000:duration=4,volume=1.2", "libvpx-vp9", "libopus", ["-b:v", "120k"])
make("04-medium.avi", "sine=frequency=330:sample_rate=48000:duration=4,volume=1.5", "mpeg4", "libmp3lame", ["-q:v", "8"])
make("05-silent.mp4", "anullsrc=r=48000:cl=stereo")
make("06-no-audio.mov", None)
make("07-near-silent.mkv", "sine=frequency=440:sample_rate=48000:duration=4,volume=0.0005", acodec="flac")
# Sparse transient forces the peak cap rather than the loudness target.
make("08-transient.mkv", "aevalsrc=if(lt(mod(t\\,1)\\,0.005)\\,0.8*sin(2*PI*1000*t)\\,0.001*sin(2*PI*440*t)):s=48000:d=4", acodec="flac")
(out / "09-corrupt.mp4").write_bytes(b"synthetic deliberately invalid video\n")
avi = (out / "04-medium.avi").read_bytes()
assert avi.count(b"FMP4") == 2, "Expected AVI stream and bitmap format tags"
(out / "10-unsupported.avi").write_bytes(avi.replace(b"FMP4", b"ZZZZ"))
manifest = {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(out.iterdir()) if f.is_file() and f.name != "manifest.json"}
(out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(f"Generated {len(manifest)} fixtures in {out}")
