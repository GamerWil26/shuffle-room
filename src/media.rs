use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub const TARGET: f64 = -18.0;
pub const PEAK_CEILING: f64 = -2.0;

pub fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000); // no helper console windows
    }
    c
}

pub fn executable(name: &str) -> PathBuf {
    let filename = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    if let Ok(exe) = std::env::current_exe() {
        for dir in [
            exe.parent().unwrap().join("bin"),
            exe.parent().unwrap().to_path_buf(),
        ] {
            let p = dir.join(&filename);
            if p.is_file() {
                return p;
            }
        }
    }
    filename.into()
}

// Drain both pipes concurrently: a damaged decoder can otherwise fill stderr and deadlock.
pub fn run(mut cmd: Command, cancel: &AtomicBool, timeout: Duration) -> Result<(String, String)> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start media helper; check dependencies")?;
    fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
        thread::spawn(move || {
            let mut saved = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                saved.extend_from_slice(&buf[..n]);
                if saved.len() > 2_000_000 {
                    saved.drain(..1_000_000);
                }
            }
            saved
        })
    }
    let out = drain(child.stdout.take().unwrap());
    let err = drain(child.stderr.take().unwrap());
    let start = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            bail!(
                "Analysis cancelled or exceeded {} seconds",
                timeout.as_secs()
            );
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(25));
    };
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    if !status.success() {
        bail!(
            "Media helper failed: {}",
            stderr
                .chars()
                .rev()
                .take(700)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
    }
    Ok((stdout, stderr))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Level {
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    pub gain_db: f64,
    pub has_audio: bool,
    pub note: String,
}

impl Level {
    pub fn from_measurement(i: f64, tp: f64) -> Self {
        let valid = i.is_finite() && tp.is_finite();
        let gain = if valid && i > -55.0 {
            (TARGET - i).min(PEAK_CEILING - tp).min(18.0)
        } else if tp.is_finite() {
            0.0f64.min(PEAK_CEILING - tp)
        } else {
            0.0
        };
        Self {
            integrated_lufs: i.is_finite().then_some(i),
            true_peak_dbtp: tp.is_finite().then_some(tp),
            gain_db: gain,
            has_audio: true,
            note: if !valid || i <= -55.0 {
                "Silence / near silence: no boost"
            } else if gain < TARGET - i - 0.05 {
                "Peak or boost limit protects headroom"
            } else {
                "Matched toward -18 LUFS"
            }
            .into(),
        }
    }

    pub fn graph(&self) -> String {
        // Resample before limiting for intersample headroom; disable alimiter auto makeup.
        format!("aformat=channel_layouts=stereo,volume={:.6}dB,aresample=192000,alimiter=limit=0.891251:level=false:latency=true,aresample=48000", self.gain_db)
    }
}

pub fn analyze(
    path: &Path,
    ffmpeg: &Path,
    ffprobe: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<Level> {
    let mut probe = command(ffprobe);
    probe
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(path);
    let (output, _) = run(probe, &cancel, Duration::from_secs(15))?;
    let v: Value = serde_json::from_str(&output)?;
    let streams = v["streams"].as_array().context("No media streams")?;
    if !streams
        .iter()
        .any(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
    {
        bail!("No video stream");
    }
    if !streams.iter().any(|s| s["codec_type"] == "audio") {
        return Ok(Level {
            integrated_lufs: None,
            true_peak_dbtp: None,
            gain_db: 0.0,
            has_audio: false,
            note: "Video without audio".into(),
        });
    }
    let mut cmd = command(ffmpeg);
    cmd.args(["-hide_banner", "-nostdin", "-v", "info", "-xerror", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-af",
            "aformat=channel_layouts=stereo,loudnorm=I=-18:TP=-2:LRA=11:print_format=json",
            "-f",
            "null",
            "-",
        ]);
    let (_, stderr) = run(cmd, &cancel, Duration::from_secs(300))?;
    let start = stderr
        .rfind('{')
        .context("FFmpeg did not report loudness")?;
    let end = stderr.rfind('}').context("Incomplete loudness report")?;
    let m: Value = serde_json::from_str(&stderr[start..=end])?;
    let number =
        |key: &str| -> Result<f64> { Ok(m[key].as_str().context("Missing measurement")?.parse()?) };
    Ok(Level::from_measurement(
        number("input_i")?,
        number("input_tp")?,
    ))
}

pub fn scan(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(folder).context("Cannot read video folder")? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if [
            "mp4", "m4v", "mov", "mkv", "webm", "avi", "mpg", "mpeg", "ts", "mts", "m2ts", "wmv",
            "flv", "ogv", "url",
        ]
        .contains(&ext.as_str())
        {
            result.push(path);
        }
    }
    result.sort();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gain_limits_and_silence() {
        assert_eq!(Level::from_measurement(-30., -20.).gain_db, 12.);
        assert_eq!(Level::from_measurement(-10., -1.).gain_db, -8.);
        assert_eq!(Level::from_measurement(-40., -35.).gain_db, 18.);
        assert_eq!(Level::from_measurement(-30., -3.).gain_db, 1.);
        assert_eq!(Level::from_measurement(-60., -50.).gain_db, 0.);
        assert_eq!(
            Level::from_measurement(f64::NEG_INFINITY, f64::NEG_INFINITY).gain_db,
            0.
        );
    }
    #[test]
    fn scan_is_shallow_and_case_insensitive() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("A.MP4"), "fake").unwrap();
        std::fs::write(d.path().join("ignore.txt"), "fake").unwrap();
        std::fs::create_dir(d.path().join("dir.mkv")).unwrap();
        assert_eq!(scan(d.path()).unwrap().len(), 1);
    }
}
