mod engine;
mod links;
mod media;
mod player;
mod shuffle;
#[cfg(windows)]
mod windows_pipe;

use anyhow::{Context, Result};
use clap::Parser;
use eframe::egui;
use engine::{Config, Control, Event};
use std::{
    collections::VecDeque,
    fs::File,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::AtomicBool,
        mpsc::{Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    version,
    about = "Hands-free shuffled video playback with measured clip loudness"
)]
struct Args {
    /// Start immediately with this folder (non-recursive).
    folder: Option<PathBuf>,
    #[arg(long)]
    mpv: Option<PathBuf>,
    #[arg(long)]
    ffmpeg: Option<PathBuf>,
    #[arg(long)]
    ffprobe: Option<PathBuf>,
    /// Optional helper for YouTube URL shortcuts.
    #[arg(long)]
    yt_dlp: Option<PathBuf>,
    /// Download cache for YouTube clips; originals remain unchanged.
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Save a YouTube InternetShortcut into the selected folder, then start.
    #[arg(long)]
    add_url: Option<String>,
    /// Run the same controller without the controls window. Null audio/video output.
    #[arg(long)]
    headless: bool,
    /// Stop after this many completed clips; useful for verification.
    #[arg(long)]
    max_clips: Option<u64>,
    /// Deterministic shuffle for reproducible tests; ordinary playback uses OS randomness.
    #[arg(long)]
    seed: Option<u64>,
    /// Save newline-delimited playback events.
    #[arg(long)]
    log: Option<PathBuf>,
    /// Use mpv's PCM audio output in place of speakers, one WAV per clip.
    #[arg(long)]
    capture_dir: Option<PathBuf>,
    /// Analyze one file and print the exact playback filter without opening a GUI.
    #[arg(long)]
    analyze: Option<PathBuf>,
    /// Bound headless verification runtime (seconds).
    #[arg(long, default_value_t = 120)]
    timeout: u64,
    /// Headless verification: trigger a manual rescan after N seconds.
    #[arg(long)]
    rescan_after: Option<u64>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config {
        mpv: args.mpv.unwrap_or_else(|| media::executable("mpv")),
        ffmpeg: args.ffmpeg.unwrap_or_else(|| media::executable("ffmpeg")),
        ffprobe: args.ffprobe.unwrap_or_else(|| media::executable("ffprobe")),
        yt_dlp: args.yt_dlp.unwrap_or_else(|| media::executable("yt-dlp")),
        cache_dir: args.cache_dir.unwrap_or_else(links::cache_dir),
        headless: args.headless,
        capture: args.capture_dir,
        seed: args.seed,
        max_clips: args.max_clips,
    };
    if let Some(path) = args.analyze {
        let level = media::analyze(
            &path,
            &config.ffmpeg,
            &config.ffprobe,
            Arc::new(AtomicBool::new(false)),
        )?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"path":path,"filter":level.graph(),"level":level})
            )?
        );
        return Ok(());
    }
    if let Some(d) = &config.capture {
        std::fs::create_dir_all(d)?;
    }
    let log = args.log.map(File::create).transpose()?;
    let (tx, rx, join) = engine::spawn(config);
    let folder = args
        .folder
        .map(std::fs::canonicalize)
        .transpose()
        .context("Cannot open folder")?;
    if let Some(url) = args.add_url {
        links::save(
            folder.as_deref().context("--add-url requires a folder")?,
            &url,
        )?;
    }
    if let Some(folder) = &folder {
        tx.send(Control::Folder(folder.clone()))?;
    }
    if args.headless {
        anyhow::ensure!(folder.is_some(), "--headless requires a folder");
        let start = Instant::now();
        let mut rescanned = false;
        let mut log = log;
        let mut completed = 0;
        loop {
            if start.elapsed().as_secs() >= args.timeout {
                let _ = tx.send(Control::Quit);
                break;
            }
            if !rescanned
                && args
                    .rescan_after
                    .is_some_and(|s| start.elapsed().as_secs() >= s)
            {
                tx.send(Control::Rescan)?;
                rescanned = true;
            }
            if let Ok(event) = rx.recv_timeout(Duration::from_millis(100)) {
                record(&event, &mut log);
                println!("{}", serde_json::to_string(&event)?);
                if event.kind == "finished" {
                    completed += 1;
                }
                if event.kind == "stopped" {
                    break;
                }
                if event.kind == "idle" && (args.rescan_after.is_none() || rescanned) {
                    let _ = tx.send(Control::Quit);
                }
            }
            if join.is_finished() {
                break;
            }
        }
        let _ = tx.send(Control::Quit);
        let _ = join.join();
        anyhow::ensure!(
            completed > 0,
            "No clips completed; inspect the event log and dependency paths"
        );
        if let Some(n) = args.max_clips {
            anyhow::ensure!(
                completed >= n,
                "Completed {completed}/{n} clips before stopping"
            );
        }
    } else {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([650., 480.])
                .with_min_inner_size([500., 380.]),
            ..Default::default()
        };
        eframe::run_native(
            "Shuffle Room",
            options,
            Box::new(move |cc| {
                cc.egui_ctx.set_visuals(egui::Visuals::dark());
                Ok(Box::new(App {
                    tx,
                    rx,
                    join: Some(join),
                    folder,
                    log,
                    status: "Choose a folder to start".into(),
                    current: String::new(),
                    level: None,
                    history: VecDeque::new(),
                    paused: false,
                    fullscreen: false,
                    volume: 100,
                    pass: 0,
                    url_input: String::new(),
                    link_status: String::new(),
                }))
            }),
        )
        .map_err(|e| anyhow::anyhow!("Could not open desktop window: {e}"))?;
    }
    Ok(())
}

fn record(event: &Event, log: &mut Option<File>) {
    if let Some(log) = log {
        let _ = writeln!(log, "{}", serde_json::to_string(event).unwrap());
        let _ = log.flush();
    }
}

struct App {
    tx: Sender<Control>,
    rx: Receiver<Event>,
    join: Option<thread::JoinHandle<()>>,
    folder: Option<PathBuf>,
    log: Option<File>,
    status: String,
    current: String,
    level: Option<media::Level>,
    history: VecDeque<String>,
    paused: bool,
    fullscreen: bool,
    volume: u8,
    pass: u64,
    url_input: String,
    link_status: String,
}
impl App {
    fn add_link(&mut self, text: &str) {
        let result = self
            .folder
            .as_deref()
            .context("Choose a video folder first")
            .and_then(|folder| links::save(folder, text));
        match result {
            Ok(path) => {
                self.link_status = format!("Saved {}", path.file_name().unwrap().to_string_lossy());
                self.url_input.clear();
                let _ = self.tx.send(Control::Rescan);
            }
            Err(e) => self.link_status = e.to_string(),
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Native desktop file drops (including browser-created InternetShortcuts).
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            let result = if let Some(path) = file.path {
                links::read_shortcut(&path)
            } else if let Some(bytes) = file.bytes {
                std::str::from_utf8(&bytes)
                    .map(str::to_owned)
                    .map_err(anyhow::Error::from)
            } else {
                Ok(file.name)
            };
            match result {
                Ok(url) => self.add_link(&url),
                Err(e) => self.link_status = format!("Drop a YouTube .url shortcut: {e}"),
            }
        }
        for event in self.rx.try_iter() {
            record(&event, &mut self.log);
            self.status = event.message.clone();
            self.pass = event.pass;
            if let Some(p) = &event.path {
                self.current = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
            }
            self.level = event.level;
            self.history
                .push_front(format!("{} · {}", event.kind, event.message));
            self.history.truncate(30);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.);
            ui.heading("Shuffle Room");
            ui.label("Set the folder. Let the room play.");
            ui.add_space(16.);
            ui.horizontal(|ui| {
                if ui.button("Choose folder…").clicked() {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Choose your video folder")
                        .pick_folder()
                    {
                        self.folder = Some(folder.clone());
                        self.paused = false;
                        let _ = self.tx.send(Control::Folder(folder));
                    }
                }
                if ui
                    .add_enabled(self.folder.is_some(), egui::Button::new("Rescan / retry"))
                    .clicked()
                {
                    let _ = self.tx.send(Control::Rescan);
                }
                ui.label(format!("Shuffled pass {}", self.pass));
            });
            ui.label(
                self.folder
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "No folder selected".into()),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.url_input)
                        .hint_text("Paste a YouTube video link…")
                        .desired_width(390.),
                );
                if ui.button("Add link").clicked() {
                    let url = self.url_input.clone();
                    self.add_link(&url);
                }
            });
            ui.small("Or drop a .url shortcut here. Saved links join the shuffled folder.");
            if !self.link_status.is_empty() {
                ui.label(&self.link_status);
            }
            ui.add_space(12.);
            ui.separator();
            ui.add_space(10.);
            ui.heading(if self.current.is_empty() {
                "Ready when you are"
            } else {
                &self.current
            });
            ui.label(&self.status);
            if let Some(level) = &self.level {
                ui.label(format!(
                    "Clip adjustment {:+.1} dB  ·  {}",
                    level.gain_db, level.note
                ));
            }
            ui.add_space(12.);
            ui.horizontal(|ui| {
                if ui
                    .button(if self.paused {
                        "▶ Resume"
                    } else {
                        "⏸ Pause"
                    })
                    .clicked()
                {
                    self.paused = !self.paused;
                    let _ = self.tx.send(Control::Pause(self.paused));
                }
                if ui.button("Next / skip ⏭").clicked() {
                    let _ = self.tx.send(Control::Next);
                }
                if ui
                    .checkbox(&mut self.fullscreen, "Full-screen video")
                    .changed()
                {
                    let _ = self.tx.send(Control::Fullscreen(self.fullscreen));
                }
            });
            if ui
                .add(egui::Slider::new(&mut self.volume, 0..=100).text("Room volume"))
                .changed()
            {
                let _ = self.tx.send(Control::Volume(self.volume));
            }
            ui.small("Clip balancing stays active at every volume. Originals are never modified.");
            ui.add_space(14.);
            egui::CollapsingHeader::new("Activity / skipped clips").show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(130.)
                    .show(ui, |ui| {
                        for line in &self.history {
                            ui.small(line);
                        }
                    });
            });
        });
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.tx.send(Control::Quit);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
