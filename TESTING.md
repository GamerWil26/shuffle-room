# Reproducible verification

## What was actually run

Environment: Debian 13 x86-64; Rust 1.100.0; FFmpeg/ffprobe 7.1.5;
mpv 0.40.0 (FFmpeg libraries 7.1.5); MinGW GCC 14.2.

| Platform / mode | Result |
|---|---|
| Linux native build | Passed |
| Linux unit tests | 6 passed |
| Linux and Windows-target Clippy, warnings as errors | Passed |
| Linux headless integration | Passed; real mpv decoder, controller, IPC, filter and PCM audio output |
| Linux graphical desktop | Actual video rendered; shuffled advance, pause, URL validation and controller exit observed |
| Linux native folder chooser | Click attempted; this minimal desktop lacks an XDG portal, so no picker opened. Folder invocation was verified instead. |
| Linux speaker output | Not listened to / not validated; desktop used null audio and headless tests captured PCM |
| Windows x64 | Release executable cross-compiled and PE/DLL imports inspected; **not run on Windows or Wine** |
| macOS | Architecture supported; **not compiled or run** |
| YouTube | Parsing, shortcut save, cached synthetic media through analysis/mpv, and invalid-link fallback tested; **no live YouTube download tested** |
| Native drag/drop | Handler implemented; **OS drag interaction not manually verified**. Browser raw-link drops depend on toolkit support; `.url` file drops and paste are the intended routes. |

No original/user videos were used. No sound-card loopback or physical room
measurement is claimed. The desktop and Xvfb runs used the same source with an
mpv wrapper adding `--ao=null`; the numerical tests used mpv's `--ao=pcm` output,
not an approximation of the filter performed by a separate FFmpeg render.

## Run the suite

Install Rust, Python 3, FFmpeg/ffprobe and mpv >= 0.40. From the project root:

```sh
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --locked
python3 scripts/generate-fixtures.py
python3 scripts/verify.py
```

Windows equivalents use `python` and
`python scripts/verify.py --player target/debug/shuffle-room.exe`.
All scripts accept explicit helper paths; see `--help`. Fixture generation needs
libx264, libvpx-vp9, AAC, FLAC, libopus, MPEG-4 and libmp3lame **encoders**, even
though ordinary playback only needs the applicable decoders. The fixture script
writes its own output directory; do not point it at personal media.

`verify.py` asserts two shuffled passes, unique entries per pass, no adjacent
repeats, automatic EOF advance, quarantine, analysis cache reuse, unchanged input
hashes, silence behavior, gain/peak behavior, single-file looping, empty/all-bad
folder idle, rescan discovery, saved URL playback using a synthetic cache, and
invalid-URL fallback. It records JSON event logs, a report and post-filter WAVs.
The unsupported-video fixture is an AVI with its video FourCC changed to ZZZZ
while retaining valid MP3 audio. It reaches mpv and proves audio-only fallback
is skipped when video cannot initialize; the corrupt MP4 is rejected earlier.
The test video and cache directories are disposable. The tests never contact
YouTube. Null values in loudness JSON represent non-finite/silent measurements.

One manual equivalent:

```sh
target/debug/shuffle-room fixtures --headless --seed 42 --max-clips 16 \
  --capture-dir evidence/capture --log evidence/playback.jsonl --timeout 120
target/debug/shuffle-room --analyze fixtures/01-quiet.mp4
ffmpeg -i evidence/capture/0004-01-quiet.mp4.wav \
  -af loudnorm=I=-18:TP=-2:LRA=11:print_format=json -f null -
```

The capture index depends on shuffle seed. `--analyze` prints the actual filter
string for a local file. `--headless` normally uses null audio/video; adding
`--capture-dir` replaces null audio with mpv PCM capture. Neither plays speakers.

## Measured results

The machine-readable evidence is [evidence/report.json](evidence/report.json).
Measurements below use the first pass, before FFmpeg normalization and after
the real mpv filter/output path, at room volume 100%.

| Synthetic clip | Input LUFS | Output LUFS | Output true peak |
|---|---:|---:|---:|
| Quiet H.264/AAC MP4 | −36.69 | −18.69 | −17.08 dBTP |
| Loud H.264/FLAC MKV | −6.05 | −18.05 | −17.46 dBTP |
| Medium VP9/Opus WebM | −19.75 | −18.05 | −17.57 dBTP |
| Medium MPEG-4/MP3 AVI | −18.76 | −18.06 | −17.23 dBTP |
| Sparse transient MKV | −24.05 | −21.05 | −2.00 dBTP |
| Silent AAC MP4 | −∞ | −∞ | −∞ |
| Near-silent FLAC MKV | Below measurement gate | Below measurement gate | −89.21 dBTP |

The four normal tone clips span **30.64 LU** before and **0.64 LU** after.
The quietest clip stops at the +18 dB boost limit. The transient clip remains
below −18 LUFS because its measured peak constrains gain. Exact digital silence
remains silent. These simple tones test the engineering path; dialogue, music,
multichannel downmixes, very short clips and real perceived loudness still need
listening validation. Absolute peak equality across different decoder/resampler
builds is not promised.

## Windows acceptance checklist (remaining)

1. On clean Windows 10/11 x64 install supported media helpers and extract the
   preview ZIP. Confirm `mpv --version`, `ffmpeg -version`, `ffprobe -version`.
2. Generate fixtures; launch the executable with a folder containing spaces and
   Unicode characters. Repeat with **Choose folder…** and Cancel.
3. Observe actual video and speakers across at least two shuffled passes. Confirm
   the quiet/loud clips sound comparable, and silent/no-audio clips do not stall.
4. Pause for over 20 seconds; resume. Test Skip while preparing a long clip,
   volume, fullscreen, bad files, rescan, and closing video/controller windows.
5. Run `verify.py` against the Windows executable. Check the report, PCM peaks
   and Windows overlapped named-pipe IPC with real mpv.
6. Install current yt-dlp plus supported JS/EJS dependencies. Using an authorized
   test video, paste/Add link; drop a `.url`; confirm shortcut creation, cache,
   measured playback, unavailable-link skipping and offline replay of cache.
7. Check Windows Defender/SmartScreen behavior and sign/package the executable
   as appropriate before calling this a tested production release.

For macOS, build natively first, repeat the above desktop and integration tests,
and add an app bundle/signing/notarization process if distributing a `.app`.
The manual GitHub Actions workflow can perform native compilation and unit
checks on all three OSes after publication. It has not been run for this task
and does not replace GUI, audio or network smoke tests.

## Known limits

First-play analysis/download introduces gaps. There is no seamless transition,
persistent analysis database, cache quota, recursive folder traversal, live
YouTube playback, subtitle selector or alternate-track selector. Fresh mpv
processes recreate the video window per clip. Hardware/audio-driver behavior on
Windows/macOS, OS drag events and network extraction remain outside the tested
claims. The frozen package lockfile and included notices support reproduction;
the external helper builds must also be recorded when comparing results.
