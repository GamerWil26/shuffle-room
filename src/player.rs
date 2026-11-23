use crate::media::{command, Level};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
type Pipe = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Pipe = crate::windows_pipe::Pipe;

pub struct Player {
    child: Child,
    pipe: Pipe,
    pub events: Receiver<Value>,
    _socket_dir: tempfile::TempDir,
}

impl Player {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        mpv: &Path,
        path: &Path,
        level: &Level,
        headless: bool,
        capture: Option<&Path>,
        volume: u8,
        paused: bool,
        fullscreen: bool,
    ) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        #[cfg(unix)]
        let socket: PathBuf = dir.path().join("mpv.sock");
        #[cfg(windows)]
        let socket: PathBuf = PathBuf::from(format!(
            r"\\.\pipe\shuffle-room-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut cmd = command(mpv);
        cmd.args([
            "--no-config",
            "--load-scripts=no",
            "--idle=yes",
            "--keep-open=no",
            "--terminal=no",
            "--osc=no",
            "--input-default-bindings=no",
            "--input-vo-keyboard=no",
            "--audio-file-auto=no",
            "--sub-auto=no",
            "--audio-channels=stereo",
            "--audio-samplerate=48000",
            "--audio-spdif=",
            "--aid=1",
            "--vid=1",
            "--sid=no",
            "--volume-max=100",
            "--focus-on=never",
            "--title=Shuffle Room — video",
            "--autofit=960x640",
        ])
        .arg(format!("--input-ipc-server={}", socket.display()))
        .arg(format!("--volume={volume}"))
        .arg(format!("--pause={}", if paused { "yes" } else { "no" }))
        .arg(format!(
            "--fullscreen={}",
            if fullscreen { "yes" } else { "no" }
        ));
        if level.has_audio {
            cmd.arg(format!("--af=lavfi=[{}]", level.graph()));
        }
        if headless || capture.is_some() {
            cmd.arg("--vo=null");
        } else {
            cmd.arg("--force-window=yes");
        }
        if let Some(capture) = capture {
            cmd.arg("--ao=pcm")
                .arg(format!("--ao-pcm-file={}", capture.display()));
        } else if headless {
            cmd.arg("--ao=null");
        }
        // mpv never receives an unanalysed clip. The filter is configured before loadfile.
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Could not start mpv; install it or place it in bin beside this app")?;
        let start = Instant::now();
        let pipe = loop {
            #[cfg(unix)]
            let connection = Pipe::connect(&socket);
            #[cfg(windows)]
            let connection = Pipe::connect(&socket);
            match connection {
                Ok(pipe) => break pipe,
                Err(_) => {
                    if child.try_wait()?.is_some() {
                        bail!("mpv exited before IPC connected; verify its build and dependencies");
                    }
                    if start.elapsed() > Duration::from_secs(8) {
                        let _ = child.kill();
                        let _ = child.wait();
                        bail!("mpv IPC connection timed out");
                    }
                    thread::sleep(Duration::from_millis(25));
                }
            }
        };
        let reader = pipe.try_clone()?;
        let (tx, events) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        let mut p = Self {
            child,
            pipe,
            events,
            _socket_dir: dir,
        };
        p.send(json!(["observe_property", 1, "time-pos"]))?;
        p.send(json!(["observe_property", 2, "video-out-params"]))?;
        p.send(json!([
            "loadfile",
            path.to_str().context("Filename is not valid Unicode")?,
            "replace"
        ]))?;
        Ok(p)
    }
    pub fn send(&mut self, cmd: Value) -> Result<()> {
        writeln!(self.pipe, "{}", json!({"command":cmd}))?;
        self.pipe.flush()?;
        Ok(())
    }
    pub fn exited(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // This also releases IPC reader threads and finalizes PCM captures.
        let _ = self.send(json!(["quit"]));
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(300) {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
