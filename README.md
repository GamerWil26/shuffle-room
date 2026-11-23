# Shuffle Room

A standalone Rust desktop player for a folder of mixed videos. Pick a folder,
press nothing else, and let it run. Every pass is freshly shuffled. Clip loudness
is measured before playback so a loud clip does not follow a quiet one at its
original level. Source videos are read only.

**Status: working Linux prototype with a Windows x64 cross-compiled preview.**
The Windows executable has not yet been run on Windows. macOS has not been built
or run. See [TESTING.md](TESTING.md) for exact evidence and release acceptance steps.

## Windows quick start

1. Extract `shuffle-room-windows-x64-preview.zip` into its own directory.
2. Install **mpv 0.40 or newer**, **FFmpeg 7.1 or newer**, and **ffprobe** from
   builds linked by [mpv](https://mpv.io/installation/) and
   [FFmpeg](https://ffmpeg.org/download.html#build-windows). FFmpeg distributions
   normally include ffprobe. Choose Windows x64 builds. Keep each provider's
   required DLLs alongside its executables.
3. Either put their executable directories on `PATH`, or put their complete
   runtime files in `bin/` beside `shuffle-room.exe`.
4. Run `Start Shuffle Room.cmd` (or `shuffle-room.exe`). Click **Choose folder…**.
   Playback begins after the first clip has been measured. Keep the controller
   open; video uses a separate mpv window. The console is retained for diagnostics.

No Rust installation is needed to use the Windows executable. The package is
unsigned and contains **no mpv/FFmpeg/yt-dlp/Deno binaries**. It is not a fully
self-contained installer. Use a maintained Windows 10/11 x64 machine with working
OpenGL drivers. An actual Windows smoke test is still required.

```powershell
.\shuffle-room.exe "D:\Room videos"
# Explicit dependency locations are also supported:
.\shuffle-room.exe "D:\Room videos" --mpv "C:\mpv\mpv.exe" --ffmpeg "C:\ffmpeg\bin\ffmpeg.exe" --ffprobe "C:\ffmpeg\bin\ffprobe.exe"
```

**Pause/Resume**, **Next/skip**, **Full-screen video**, and **Room volume** are in
the controller. Room volume ranges from mute to the balanced reference level;
it cannot boost past 100%. Closing the video window leaves the controller idle;
Next resumes. Closing the controller also closes its player process.

## YouTube links

Paste a single YouTube video link into the link box and click **Add link**. Or
drag a Windows `.url` InternetShortcut file into the controller. A new
`YouTube-VIDEO_ID.url` is saved in the selected folder, without overwriting an
existing different file. Adding a link rescans the folder and restarts the queue.
You can also place `.url` files in the folder yourself or use:

```powershell
.\shuffle-room.exe "D:\Room videos" --add-url "https://www.youtube.com/watch?v=VIDEO_ID"
```

Replace `VIDEO_ID` with a real 11-character video ID. A shortcut looks like:

```ini
[InternetShortcut]
URL=https://www.youtube.com/watch?v=VIDEO_ID
```

Install current [yt-dlp](https://github.com/yt-dlp/yt-dlp#installation) for links.
Current YouTube extraction also needs yt-dlp's EJS support and a supported
JavaScript runtime such as [Deno](https://deno.com/). Follow yt-dlp's current
installation instructions; Windows release executables include components that
a minimal Python installation may omit. `--yt-dlp PATH` overrides discovery.

The first play downloads to the app cache, then measures that complete file and
uses the **same mpv playback path** as local video. This allows consistent
cross-clip loudness from the start of playback. It is not live streaming and may
take several minutes before a new link plays. Subsequent plays reuse the cached
download. Requests select up to 1080p where available, reject live videos and
videos over two hours, impose a 2 GiB per-download format limit, and time out
after five minutes. Extraction or analysis failure skips the link.

Only individual YouTube HTTP(S) links are accepted. Playlist parameters and
timestamps are removed. Private, unavailable, region-restricted or sign-in-only
videos may fail; no browser cookies, credentials, DRM workarounds or other-site
URLs are used. Download only videos you have permission to use. YouTube changes
can require updating yt-dlp and its runtime.

Drag support is for native **files**. Some browsers/OS combinations deliver a
dragged address as text or a virtual object that the window toolkit does not
expose; paste the link or save/drop a `.url` file in that case. Native browser-link
dragging is not claimed to work on every platform, and the OS drop interaction
has not yet been manually verified on Windows.

Cache defaults: `%LOCALAPPDATA%\ShuffleRoom\Cache` on Windows,
`~/Library/Caches/shuffle-room` on macOS, and `$XDG_CACHE_HOME/shuffle-room` or
`~/.cache/shuffle-room` on Linux. Override with `--cache-dir PATH`. To reclaim
space or redownload a changed video, quit the app and remove that video's ID
subfolder. Cache has no automatic total-size eviction in this version. URL
shortcut files remain small; downloaded media lives separately.

## Loudness behavior

Before a file's first playback in this session, FFmpeg measures integrated
BS.1770/EBU R128 loudness and true peak after downmixing the first audio track to
stereo. A constant clip gain is calculated:

```text
gain dB = min(-18 LUFS - measured LUFS, -2 dBTP - measured true peak, +18 dB)
```

This preserves within-clip dynamics. It avoids a continually adapting volume
filter that pumps up quiet passages. Peak-heavy material may remain below the
loudness target; safety takes priority over forcing everything to exactly the
same number. Material at or below −55 LUFS and unmeasurable silence gets no
positive boost. Video without an audio track plays normally. There is no noise
generator and no automatic makeup gain.

mpv applies the measured gain using its FFmpeg audio filter bridge, resamples to
192 kHz, passes through a −1 dB safety limiter with makeup disabled, then outputs
48 kHz stereo. The measured −2 dBTP cap gives additional headroom. Audio
passthrough and user mpv configs/scripts are disabled so they cannot bypass this
chain. The first audio and video tracks are selected consistently. This version
does not offer alternate audio tracks or subtitles.

Analysis results are cached in memory by path, size and modification time, so
subsequent passes do not repeat analysis. Restarting the app repeats measurement.
The initial measurement can delay a clip; this version does not prefetch the
entire next pass. The controls remain responsive and Skip cancels preparation.

## Shuffle, rescan and errors

- The folder scan is shallow; subdirectories and symbolic links are ignored.
- Each pass is a random permutation, not alphabetical playback. A pass boundary
  avoids the previous clip when at least two candidates remain. One valid file
  loops by necessity. Random permutations can coincidentally be similar.
- At pass end the folder is rescanned for additions/removals. **Rescan / retry**
  interrupts the current clip, starts a fresh pass, and clears quarantine.
- Analysis/decoding failures quarantine the item for this session. If all items
  are bad or the folder is empty, the app goes idle instead of spinning. Add files
  and click Rescan. A modified quarantined file needs that manual retry.
- FFprobe has a 15-second timeout; analysis and downloading each have a
  five-minute timeout. Playback with no progress for 20 seconds is skipped,
  except while paused. Errors are visible in Activity and optional JSON logs.
- An mpv process/window is created for each clip. Short transition gaps and
  window recreation are expected; this is not a gapless playback system.

## Formats

Container extensions scanned: MP4/M4V, MOV, MKV, WebM, AVI, MPG/MPEG, TS/MTS/M2TS,
WMV, FLV, OGV, plus `.url` YouTube shortcuts. Extensions are only candidates;
success depends on codecs enabled in **both** the chosen FFmpeg and mpv builds.

Verified synthetic combinations on Linux with FFmpeg 7.1.5 and mpv 0.40.0:

| Container | Video | Audio |
|---|---|---|
| MP4 | H.264 | AAC, including silence |
| MKV | H.264 | FLAC |
| WebM | VP9 | Opus |
| AVI | MPEG-4 Part 2 | MP3 |
| MOV | H.264 | no audio |

HEVC, AV1, ProRes, unusual camera formats, HDR handling and DRM-protected media
are **not validated** here. No promise of every codec. Inspect local builds with
`ffmpeg -decoders` and `mpv --vd=help`; ffprobe must recognize the file, FFmpeg must
analyze its audio, and mpv must decode its selected tracks.

## Build from source

Rust stable 1.100 was used; the lockfile is included. Install Rust using
[rustup](https://rustup.rs/). No C codec bindings or codec implementation are in
this project. Playback talks to the external mpv process through JSON IPC:
Unix-domain sockets on Linux/macOS, overlapped named pipes on Windows. The UI
uses egui/eframe and rfd.

```sh
cargo build --release --locked
cargo test --locked
```

Windows: use the x64 MSVC Rust toolchain and Visual Studio C++ Build Tools.
Run `powershell -File scripts/package-windows.ps1` to build and package. The
provided binary was instead cross-compiled on Linux using Rust's
`x86_64-pc-windows-gnu` target and MinGW GCC 14; no Wine/native Windows execution
was performed.

Linux (Debian/Ubuntu): install a C compiler, pkg-config, OpenGL/EGL and
X11/Wayland development dependencies, `libxkbcommon-dev`, mpv, ffmpeg, and an XDG
Desktop Portal implementation such as `xdg-desktop-portal-gtk`. rfd's Linux folder
chooser requires a desktop D-Bus session and portal; command-line folder
invocation works without the picker. On a minimal desktop with no portal the
chooser may return no selection.

macOS: install Xcode Command Line Tools, Rust, mpv and ffmpeg (for example from
Homebrew), then `cargo run --release -- /path/to/videos`. This architecture
supports macOS, but no macOS binary or notarized `.app` is supplied or tested.

See [TESTING.md](TESTING.md) for fixture generation, actual mpv PCM measurements,
headless assertions, and platform smoke tests. See
[THIRD_PARTY.md](THIRD_PARTY.md) for dependency notices and redistribution notes.
