# Third-party components

Original Shuffle Room code is MIT licensed. External playback/download tools
are separate programs invoked with arguments and IPC, not copied into this
repository or release bundle:

- mpv: https://mpv.io/ — exact licensing depends on build configuration (GPL or
  LGPL plus dependencies). Obtain binaries and corresponding notices/source
  information from the build provider linked by mpv.
- FFmpeg/ffprobe: https://ffmpeg.org/ — LGPL/GPL configuration and enabled codec
  libraries vary. See https://ffmpeg.org/legal.html and the selected provider.
- yt-dlp (optional): https://github.com/yt-dlp/yt-dlp — source and packaged
  executables have their own notices; follow the official release information.
- Deno (optional YouTube JS runtime): https://deno.com/ — use official packages.

Do not add these runtime binaries to a redistribution without auditing the exact
build's licenses, source availability and required notices. Users may put their
own installed runtime files in `bin/` for personal use.

The Rust binary includes third-party crates listed exactly in `Cargo.lock`.
`scripts/collect-notices.py` gathers the selected target's crate metadata and
license/notice files, including bundled font notices. The Windows package
contains `third-party-notices/` and `dependencies.json`. Preserve those files
when redistributing the executable. These notices do not relicense those crates
under the project's MIT license. Run the script against the same locked sources
whenever rebuilding a release.

Fixtures are synthetic signals and FFmpeg test patterns generated locally by
the provided script. No user videos or downloaded YouTube media are included.
