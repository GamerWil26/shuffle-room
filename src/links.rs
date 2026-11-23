use crate::media;
use anyhow::{bail, Context, Result};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

/// Accept individual YouTube videos only, never playlists, arbitrary hosts,
/// credentials, local URLs, or commands embedded in an InternetShortcut.
pub fn video_id(text: &str) -> Result<String> {
    let url = url::Url::parse(text.trim()).context("Paste a full YouTube video URL")?;
    anyhow::ensure!(
        matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "Use a normal YouTube HTTP(S) URL"
    );
    let host = url.host_str().unwrap_or("");
    let id = if host == "youtu.be" {
        url.path().trim_matches('/').to_owned()
    } else if [
        "youtube.com",
        "www.youtube.com",
        "m.youtube.com",
        "music.youtube.com",
    ]
    .contains(&host)
    {
        if url.path() == "/watch" {
            url.query_pairs()
                .find(|(k, _)| k == "v")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        } else {
            let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
            if parts.len() == 2 && ["shorts", "embed", "live"].contains(&parts[0]) {
                parts[1].into()
            } else {
                String::new()
            }
        }
    } else {
        bail!("Only YouTube video links are supported");
    };
    anyhow::ensure!(
        id.len() == 11
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "The link needs an individual 11-character YouTube video ID"
    );
    Ok(id)
}
pub fn canonical(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

pub fn read_shortcut(path: &Path) -> Result<String> {
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 16_384,
        "URL shortcut is too large"
    );
    let bytes = std::fs::read(path)?;
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        String::from_utf16(
            &bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )?
    } else {
        String::from_utf8(bytes)?
            .trim_start_matches('\u{feff}')
            .to_owned()
    };
    let url = text
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("URL")
                .then_some(value.trim())
        })
        .unwrap_or(text.trim());
    Ok(canonical(&video_id(url)?))
}

pub fn save(folder: &Path, text: &str) -> Result<PathBuf> {
    let id = video_id(text)?;
    let path = folder.join(format!("YouTube-{id}.url"));
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            write!(file, "[InternetShortcut]\r\nURL={}\r\n", canonical(&id))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::ensure!(
                read_shortcut(&path)? == canonical(&id),
                "A different shortcut already has this filename; it was not changed"
            );
        }
        Err(e) => return Err(e.into()),
    }
    Ok(path)
}

pub fn cache_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(p).join("ShuffleRoom").join("Cache");
        }
    }
    if let Some(p) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(p).join("shuffle-room");
    }
    if let Some(p) = std::env::var_os("HOME") {
        return PathBuf::from(p).join(if cfg!(target_os = "macos") {
            "Library/Caches/shuffle-room"
        } else {
            ".cache/shuffle-room"
        });
    }
    std::env::temp_dir().join("shuffle-room-cache")
}

pub fn resolve(
    path: &Path,
    yt_dlp: &Path,
    ffmpeg: &Path,
    cache: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("url"))
    {
        return Ok(path.to_path_buf());
    }
    let url = read_shortcut(path)?;
    let id = video_id(&url)?;
    let saved = cache.join(&id);
    if saved.is_dir() {
        let files = media::scan(&saved)?;
        if let Some(file) = files
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e != "url"))
        {
            return Ok(file);
        }
    }
    std::fs::create_dir_all(cache).context("Cannot create YouTube cache directory")?;
    let staging = tempfile::Builder::new()
        .prefix("download-")
        .tempdir_in(cache)?;
    let mut cmd = media::command(yt_dlp);
    cmd.args([
        "--ignore-config",
        "--no-playlist",
        "--no-progress",
        "--no-warnings",
        "--no-mtime",
        "--socket-timeout",
        "15",
        "--retries",
        "1",
        "--fragment-retries",
        "1",
        "--max-filesize",
        "2G",
        "--match-filter",
        "!is_live & duration <= 7200",
        "--format",
        "bv*[height<=1080]+ba/b[height<=1080]/b",
        "--merge-output-format",
        "mkv",
        "--print",
        "after_move:filepath",
        "--output",
    ])
    .arg(staging.path().join("media.%(ext)s"));
    if ffmpeg.is_absolute() {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }
    cmd.arg("--").arg(url);
    let (stdout, _) = media::run(cmd, cancel, Duration::from_secs(300)).context(
        "YouTube download failed (install/update yt-dlp; inaccessible or live videos are skipped)",
    )?;
    let downloaded = stdout
        .lines()
        .rev()
        .find(|s| !s.trim().is_empty())
        .context("YouTube video was unavailable or rejected by the duration/size limits")?;
    let downloaded = PathBuf::from(downloaded).canonicalize()?;
    anyhow::ensure!(
        downloaded.starts_with(staging.path().canonicalize()?),
        "Downloader returned a path outside its cache staging directory"
    );
    anyhow::ensure!(downloaded.is_file(), "Downloader did not produce a video");
    // Only a complete download is made visible. A cancelled job never poisons cache.
    std::fs::create_dir_all(&saved)?;
    let destination = saved.join(downloaded.file_name().unwrap());
    std::fs::rename(downloaded, &destination)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_and_normalizes_video_links() {
        for s in [
            "https://youtu.be/abcdefgh_12?t=3",
            "https://www.youtube.com/watch?v=abcdefgh_12&list=x",
            "https://youtube.com/shorts/abcdefgh_12",
            "https://m.youtube.com/embed/abcdefgh_12",
        ] {
            assert_eq!(video_id(s).unwrap(), "abcdefgh_12");
        }
        for s in [
            "file:///etc/passwd",
            "https://youtube.com.evil.test/watch?v=abcdefgh_12",
            "https://youtube.com/playlist?list=abc",
            "https://youtube.com/watch?v=bad",
            "https://evil@youtube.com/watch?v=abcdefgh_12",
            "--exec rm",
        ] {
            assert!(video_id(s).is_err(), "{s}");
        }
    }
    #[test]
    fn shortcut_roundtrip_preserves_existing_files() {
        let d = tempfile::tempdir().unwrap();
        let p = save(d.path(), "https://youtu.be/abcdefgh_12").unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(read_shortcut(&p).unwrap(), canonical("abcdefgh_12"));
        save(d.path(), "https://youtube.com/watch?v=abcdefgh_12").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
        std::fs::write(&p, "do not overwrite").unwrap();
        assert!(save(d.path(), "https://youtu.be/abcdefgh_12").is_err());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "do not overwrite");
    }
}
