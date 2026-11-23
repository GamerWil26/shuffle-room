Standalone Rust desktop player: shuffled automatic folder playback, whole-clip
loudness measurement, peak-limited gain, silent/no-audio handling, corrupt-file
quarantine, rescan and simple controls. YouTube links can be pasted or supplied
as `.url` shortcuts; optional yt-dlp downloads to a separate cache before the
same measured playback path.

Windows x64 **cross-compiled preview**. It has not been run on Windows or macOS.
Linux unit/integration tests and actual desktop video rendering passed. Measured
normal test clips reduced their loudness spread from 30.64 LU to 0.64 LU through
the real mpv PCM output path. No live YouTube extraction was tested.

Install mpv >= 0.40 plus FFmpeg/ffprobe separately. YouTube also needs current
yt-dlp and its supported JS/EJS runtime dependencies. No third-party media
binaries or personal videos are bundled. README and TESTING describe setup,
tested formats and remaining platform checks. The executable is unsigned.
