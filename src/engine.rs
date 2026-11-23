use crate::{
    links,
    media::{self, Level},
    player::Player,
    shuffle::ShuffleBag,
};
use rand::{rngs::StdRng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone)]
pub struct Config {
    pub mpv: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub yt_dlp: PathBuf,
    pub cache_dir: PathBuf,
    pub headless: bool,
    pub capture: Option<PathBuf>,
    pub seed: Option<u64>,
    pub max_clips: Option<u64>,
}

pub enum Control {
    Folder(PathBuf),
    Rescan,
    Next,
    Pause(bool),
    Volume(u8),
    Fullscreen(bool),
    Quit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub kind: String,
    pub message: String,
    pub path: Option<PathBuf>,
    pub pass: u64,
    pub level: Option<Level>,
}

struct Job {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<(PathBuf, Level), String>>,
    path: PathBuf,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
type Fingerprint = (u64, Option<SystemTime>);
fn fingerprint(p: &PathBuf) -> Option<Fingerprint> {
    std::fs::metadata(p)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

pub fn spawn(config: Config) -> (Sender<Control>, Receiver<Event>, thread::JoinHandle<()>) {
    let (tx, controls) = mpsc::channel();
    let (events, rx) = mpsc::channel();
    let join = thread::spawn(move || Engine::new(config, controls, events).run());
    (tx, rx, join)
}

struct Engine {
    config: Config,
    controls: Receiver<Control>,
    events: Sender<Event>,
    folder: Option<PathBuf>,
    bag: ShuffleBag,
    rng: StdRng,
    bad: HashSet<PathBuf>,
    cache: HashMap<PathBuf, (Fingerprint, PathBuf, Level)>,
    job: Option<Job>,
    player: Option<Player>,
    current: Option<PathBuf>,
    level: Option<Level>,
    paused: bool,
    volume: u8,
    fullscreen: bool,
    want_next: bool,
    running: bool,
    last_progress: Instant,
    last_time: f64,
    loaded: bool,
    video_seen: bool,
    completed: u64,
}

impl Engine {
    fn new(config: Config, controls: Receiver<Control>, events: Sender<Event>) -> Self {
        let rng = config
            .seed
            .map(StdRng::seed_from_u64)
            .unwrap_or_else(StdRng::from_entropy);
        Self {
            config,
            controls,
            events,
            folder: None,
            bag: ShuffleBag::default(),
            rng,
            bad: HashSet::new(),
            cache: HashMap::new(),
            job: None,
            player: None,
            current: None,
            level: None,
            paused: false,
            volume: 100,
            fullscreen: false,
            want_next: false,
            running: true,
            last_progress: Instant::now(),
            last_time: -1.0,
            loaded: false,
            video_seen: false,
            completed: 0,
        }
    }
    fn emit(&self, kind: &str, message: impl Into<String>) {
        let _ = self.events.send(Event {
            kind: kind.into(),
            message: message.into(),
            path: self.current.clone(),
            pass: self.bag.pass,
            level: self.level.clone(),
        });
    }
    fn stop(&mut self) {
        self.job = None;
        self.player = None;
        self.loaded = false;
        self.video_seen = false;
    }
    fn rescan(&mut self, clear_bad: bool) {
        let Some(folder) = &self.folder else {
            return;
        };
        if clear_bad {
            self.bad.clear();
        }
        match media::scan(folder) {
            Ok(files) => {
                self.cache.retain(|p, (f, media, _)| {
                    files.contains(p) && media.is_file() && fingerprint(p).as_ref() == Some(f)
                });
                let files: Vec<_> = files
                    .into_iter()
                    .filter(|p| !self.bad.contains(p))
                    .collect();
                self.emit(
                    "scan",
                    format!(
                        "{} candidate clips; {} quarantined",
                        files.len(),
                        self.bad.len()
                    ),
                );
                self.bag.refill(files, &mut self.rng);
            }
            Err(e) => {
                self.bag.pending.clear();
                self.emit("error", e.to_string());
            }
        }
    }
    fn next(&mut self) {
        self.want_next = false;
        if self.bag.pending.is_empty() {
            self.rescan(false);
        }
        while let Some(path) = self.bag.next() {
            if self.bad.contains(&path) {
                continue;
            }
            // A failed item can leave the last played clip as the only candidate.
            // Prefer a different playable candidate when one remains in this pass.
            if self.bag.last.as_ref() == Some(&path)
                && self
                    .bag
                    .pending
                    .iter()
                    .any(|p| !self.bad.contains(p) && p != &path)
            {
                self.bag.pending.insert(0, path);
                continue;
            }
            self.current = Some(path.clone());
            self.level = None;
            if let Some((fp, resolved, level)) = self.cache.get(&path) {
                if fingerprint(&path).as_ref() == Some(fp) && resolved.is_file() {
                    self.play(resolved.clone(), level.clone());
                    return;
                }
            }
            let (tx, rx) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let file = path.clone();
            let ffmpeg = self.config.ffmpeg.clone();
            let ffprobe = self.config.ffprobe.clone();
            let yt_dlp = self.config.yt_dlp.clone();
            let cache_dir = self.config.cache_dir.clone();
            thread::spawn(move || {
                let result = (|| {
                    let resolved = links::resolve(&file, &yt_dlp, &ffmpeg, &cache_dir, &flag)?;
                    let level = media::analyze(&resolved, &ffmpeg, &ffprobe, flag)?;
                    anyhow::Ok((resolved, level))
                })()
                .map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
            });
            self.emit(
                "analyzing",
                "Preparing and measuring clip (YouTube downloads on first play; Skip cancels)",
            );
            self.job = Some(Job { cancel, rx, path });
            return;
        }
        self.emit(
            "idle",
            "No playable clips. Choose a folder or use Rescan to retry.",
        );
    }
    fn play(&mut self, resolved: PathBuf, level: Level) {
        let path = self.current.as_ref().unwrap().clone();
        self.level = Some(level.clone());
        let capture = self.config.capture.as_ref().map(|d| {
            d.join(format!(
                "{:04}-{}.wav",
                self.completed,
                path.file_name().unwrap().to_string_lossy()
            ))
        });
        match Player::start(
            &self.config.mpv,
            &resolved,
            &level,
            self.config.headless,
            capture.as_deref(),
            self.volume,
            self.paused,
            self.fullscreen,
        ) {
            Ok(player) => {
                self.player = Some(player);
                self.last_progress = Instant::now();
                self.last_time = -1.0;
                self.loaded = false;
                self.video_seen = false;
                self.emit("loading", "Opening video");
            }
            Err(e) => {
                self.emit("error", format!("{e:#}"));
                self.bad.insert(path);
                self.want_next = true;
            }
        }
    }
    fn fail(&mut self, reason: &str) {
        self.emit("skipped", reason);
        if let Some(p) = &self.current {
            self.bad.insert(p.clone());
        }
        self.stop();
        self.want_next = true;
    }
    fn control(&mut self, c: Control) {
        match c {
            Control::Quit => self.running = false,
            Control::Folder(folder) => {
                self.stop();
                self.folder = Some(folder);
                self.bag = ShuffleBag::default();
                self.bad.clear();
                self.cache.clear();
                self.current = None;
                self.level = None;
                self.paused = false;
                self.rescan(true);
                self.want_next = true;
            }
            Control::Rescan => {
                self.stop();
                self.rescan(true);
                self.want_next = true;
            }
            Control::Next => {
                self.stop();
                self.want_next = true;
            }
            Control::Pause(pause) => {
                self.paused = pause;
                self.last_progress = Instant::now();
                if let Some(p) = &mut self.player {
                    let _ = p.send(json!(["set_property", "pause", pause]));
                }
                self.emit("pause", if pause { "Paused" } else { "Resumed" });
            }
            Control::Volume(volume) => {
                self.volume = volume.min(100);
                if let Some(p) = &mut self.player {
                    let _ = p.send(json!(["set_property", "volume", self.volume]));
                }
            }
            Control::Fullscreen(fullscreen) => {
                self.fullscreen = fullscreen;
                if let Some(p) = &mut self.player {
                    let _ = p.send(json!(["set_property", "fullscreen", fullscreen]));
                }
            }
        }
    }
    fn run(&mut self) {
        while self.running {
            loop {
                match self.controls.try_recv() {
                    Ok(c) => self.control(c),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.running = false;
                        break;
                    }
                }
            }
            if !self.running {
                break;
            }
            let result = self.job.as_ref().and_then(|job| job.rx.try_recv().ok());
            if let Some(result) = result {
                let job = self.job.take().unwrap();
                match result {
                    Ok((resolved, level)) => {
                        if let Some(fp) = fingerprint(&job.path) {
                            self.cache
                                .insert(job.path.clone(), (fp, resolved.clone(), level.clone()));
                        }
                        self.play(resolved, level);
                    }
                    Err(e) => self.fail(&e),
                }
            }
            let messages: Vec<_> = self
                .player
                .as_ref()
                .map(|p| p.events.try_iter().collect())
                .unwrap_or_default();
            for event in messages {
                if self.player.is_none() {
                    break;
                }
                match event["event"].as_str() {
                    Some("file-loaded") => {
                        self.loaded = true;
                        self.last_progress = Instant::now();
                    }
                    Some("property-change")
                        if event["name"] == "video-out-params"
                            && event["data"].is_object()
                            && !self.video_seen =>
                    {
                        self.video_seen = true;
                        if let Some(p) = self.current.clone() {
                            self.bag.played(p);
                        }
                        self.emit("playing", "Playing with measured loudness balancing");
                    }
                    Some("property-change") if event["name"] == "time-pos" => {
                        if let Some(t) = event["data"].as_f64() {
                            if t > 1.0 && self.loaded && !self.video_seen {
                                self.fail("mpv could not initialize video output; skipping audio-only fallback");
                                continue;
                            }
                            if (t - self.last_time).abs() > 0.01 {
                                self.last_progress = Instant::now();
                                self.last_time = t;
                            }
                        }
                    }
                    Some("end-file") => {
                        if event["reason"] == "eof" {
                            if !self.video_seen {
                                self.fail("mpv ended without decoding video");
                                continue;
                            }
                            self.completed += 1;
                            self.emit("finished", "Clip completed");
                            self.stop();
                            if self.config.max_clips.is_some_and(|n| self.completed >= n) {
                                self.running = false;
                            } else {
                                self.want_next = true;
                            }
                        } else if event["reason"] == "error" {
                            self.fail(&format!("mpv playback error: {}", event["file_error"]));
                        }
                    }
                    _ => {
                        if event
                            .get("error")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s != "success")
                        {
                            self.fail(&format!("mpv command failed: {}", event["error"]));
                        }
                    }
                }
            }
            if self.player.as_mut().is_some_and(|p| p.exited()) {
                // Closing the video window is intentional: leave the controller idle.
                self.stop();
                self.emit("idle", "Video window closed. Next or Rescan resumes.");
            }
            if self.player.is_some()
                && !self.paused
                && self.last_progress.elapsed() > Duration::from_secs(20)
            {
                self.fail("No playback progress for 20 seconds; skipping stalled clip");
            }
            if self.running && self.want_next && self.player.is_none() && self.job.is_none() {
                self.next();
            }
            thread::sleep(Duration::from_millis(25));
        }
        self.stop();
        self.emit("stopped", "Player stopped");
    }
}
