use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use egui::{
    Color32, ColorImage, Context, FontId, RichText, ScrollArea, TextureHandle, TextureOptions,
    Vec2,
};
use log::{error, info, warn};
use notify::RecommendedWatcher;

use crate::{
    events::AppEvent,
    file_ops::{ensure_dir, move_file},
    processor::{find_tesseract, process_image, ScanMode},
    watcher::{start_watcher, wait_for_file_access},
};

#[cfg(windows)]
use crate::scanner_hw::scan_to_file;

// ──────────────────────────────────────────────────────────────────────────────
// Constants
// ──────────────────────────────────────────────────────────────────────────────

const MAX_LOG_LINES: usize = 200;

// ──────────────────────────────────────────────────────────────────────────────
// App state
// ──────────────────────────────────────────────────────────────────────────────

pub struct ScannerApp {
    // ── Settings (user-editable) ──────────────────────────────────────────────
    root_directory: String,
    mode: ScanMode,
    tesseract_cmd: String,
    tesseract_ok: bool,

    // ── Derived directory paths ───────────────────────────────────────────────
    waves_dir: PathBuf,
    finished_dir: PathBuf,
    error_dir: PathBuf,

    // ── UI state ──────────────────────────────────────────────────────────────
    preview_texture: Option<TextureHandle>,
    preview_path: Option<PathBuf>,
    last_id: Option<String>,
    log: VecDeque<String>,
    is_scanning: bool,
    status: String,

    // ── Background channel ────────────────────────────────────────────────────
    event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
    /// Holds the watcher alive for the app's lifetime.
    _watcher: Option<RecommendedWatcher>,
}

impl ScannerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let root_directory = default_root();
        let (waves_dir, finished_dir, error_dir) = derive_dirs(&root_directory);

        let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

        let (tesseract_cmd, tesseract_ok) = match find_tesseract() {
            Ok(cmd) => {
                info!("Tesseract found: {cmd}");
                (cmd, true)
            }
            Err(e) => {
                warn!("Tesseract not found at startup: {e}");
                ("tesseract".to_string(), false)
            }
        };

        let watcher = init_dirs_and_watcher(
            &waves_dir,
            &finished_dir,
            &error_dir,
            event_tx.clone(),
            cc.egui_ctx.clone(),
        );

        let mut app = Self {
            root_directory,
            mode: ScanMode::Auto,
            tesseract_cmd,
            tesseract_ok,
            waves_dir,
            finished_dir,
            error_dir,
            preview_texture: None,
            preview_path: None,
            last_id: None,
            log: VecDeque::with_capacity(MAX_LOG_LINES),
            is_scanning: false,
            status: "Ready".to_string(),
            event_tx,
            event_rx,
            _watcher: watcher,
        };

        app.push_log("Application started.");
        app
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

impl ScannerApp {
    fn push_log(&mut self, msg: &str) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let (hh, mm, ss) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
        let line = format!("[{hh:02}:{mm:02}:{ss:02}] {msg}");
        if self.log.len() >= MAX_LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    fn load_preview(&mut self, path: &Path, ctx: &Context) {
        match image::open(path) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let color_image =
                    ColorImage::from_rgba_unmultiplied([w as _, h as _], &rgba.into_raw());
                self.preview_texture =
                    Some(ctx.load_texture("preview", color_image, TextureOptions::LINEAR));
                self.preview_path = Some(path.to_path_buf());
            }
            Err(e) => {
                error!("Could not load preview for {path:?}: {e}");
                self.push_log(&format!("Could not load preview: {e}"));
            }
        }
    }

    fn apply_settings(&mut self, ctx: &Context) {
        let (waves_dir, finished_dir, error_dir) = derive_dirs(&self.root_directory);
        self.waves_dir = waves_dir.clone();
        self.finished_dir = finished_dir.clone();
        self.error_dir = error_dir.clone();

        self._watcher = init_dirs_and_watcher(
            &waves_dir,
            &finished_dir,
            &error_dir,
            self.event_tx.clone(),
            ctx.clone(),
        );
        self.push_log(&format!("Settings applied — watching {waves_dir:?}"));
        self.status = format!("Watching {}", waves_dir.display());
    }

    fn trigger_hardware_scan(&mut self, ctx: &Context) {
        #[cfg(windows)]
        {
            self.is_scanning = true;
            self.status = "Scanning…".to_string();

            let tx = self.event_tx.clone();
            let ctx_clone = ctx.clone();
            let out_dir = self.waves_dir.clone();

            thread::spawn(move || {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let path = out_dir.join(format!("scan_{ts}.png"));

                match scan_to_file(&path) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::HardwareScanReady(path));
                    }
                    Err(e) => {
                        let msg = format!("Scan failed: {e}");
                        error!("{msg}");
                        let _ = tx.send(AppEvent::Log(msg));
                    }
                }
                ctx_clone.request_repaint();
            });
        }

        #[cfg(not(windows))]
        {
            let _ = ctx;
            self.push_log("Hardware scanning requires Windows + WIA.");
            self.status = "Not available on this platform.".to_string();
        }
    }

    fn spawn_process(&self, path: PathBuf, ctx: &Context) {
        let mode = self.mode;
        let tess = self.tesseract_cmd.clone();
        let finished_dir = self.finished_dir.clone();
        let error_dir = self.error_dir.clone();
        let tx = self.event_tx.clone();
        let ctx_clone = ctx.clone();

        thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            if !wait_for_file_access(&path, 10) {
                let msg = format!("Could not access file after retries: {path:?}");
                error!("{msg}");
                let _ = tx.send(AppEvent::Log(msg));
                ctx_clone.request_repaint();
                return;
            }

            let po_number = match process_image(&path, mode, &tess) {
                Ok(id) => id,
                Err(e) => {
                    let msg = format!("Processing error for {path:?}: {e}");
                    error!("{msg}");
                    let _ = tx.send(AppEvent::Log(msg));
                    ctx_clone.request_repaint();
                    None
                }
            };

            let dest_dir = if po_number.is_some() { &finished_dir } else { &error_dir };
            let destination = move_file(&path, dest_dir, po_number.as_deref())
                .map_err(|e| e.to_string());

            let _ = tx.send(AppEvent::ProcessingDone { source: path, po_number, destination });
            ctx_clone.request_repaint();
        });
    }

    fn drain_events(&mut self, ctx: &Context) {
        while let Ok(evt) = self.event_rx.try_recv() {
            match evt {
                AppEvent::FileDetected(path) => {
                    self.push_log(&format!("File detected: {}", path.display()));
                    self.load_preview(&path, ctx);
                    self.status = format!("Processing {}…", path.display());
                    self.spawn_process(path, ctx);
                }

                AppEvent::ProcessingDone { source, po_number, destination } => {
                    match &destination {
                        Ok(dest) => {
                            let id = po_number.clone().unwrap_or_else(|| "(no ID)".to_string());
                            let msg = format!(
                                "✓ {} → {} [{}]",
                                source.file_name().unwrap_or_default().to_string_lossy(),
                                dest.file_name().unwrap_or_default().to_string_lossy(),
                                id,
                            );
                            info!("{msg}");
                            self.push_log(&msg);
                            self.last_id = po_number;
                            self.status = format!(
                                "Last ID: {}",
                                self.last_id.as_deref().unwrap_or("—")
                            );
                        }
                        Err(e) => {
                            let msg = format!("✗ Move failed for {}: {e}", source.display());
                            error!("{msg}");
                            self.push_log(&msg);
                            self.status = "Error — see log.".to_string();
                        }
                    }
                }

                AppEvent::HardwareScanReady(path) => {
                    self.is_scanning = false;
                    self.push_log(&format!("Hardware scan saved: {}", path.display()));
                    self.load_preview(&path, ctx);
                    self.status = "Scan complete — processing…".to_string();
                    self.spawn_process(path, ctx);
                }

                AppEvent::Log(msg) => {
                    self.push_log(&msg);
                }
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// eframe::App implementation (eframe 0.34 API)
// ──────────────────────────────────────────────────────────────────────────────

impl eframe::App for ScannerApp {
    /// Non-rendering logic — drains background events and updates state.
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.drain_events(ctx);
    }

    /// Renders the full application UI.
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // ── Top bar ───────────────────────────────────────────────────────────
        egui::Panel::top("top_bar").show_inside(ui, |ui: &mut egui::Ui| {
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.heading("PO Scanner");
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui: &mut egui::Ui| {
                        if ui.button("✕  Quit").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    },
                );
            });
        });

        // ── Bottom log ────────────────────────────────────────────────────────
        egui::Panel::bottom("log_panel")
            .resizable(true)
            .min_size(120.0)
            .default_size(160.0)
            .show_inside(ui, |ui: &mut egui::Ui| {
                ui.heading("Activity Log");
                ui.add_space(4.0);
                ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui: &mut egui::Ui| {
                        for line in &self.log {
                            ui.label(RichText::new(line).font(FontId::monospace(12.0)));
                        }
                    });
            });

        // ── Left settings panel ───────────────────────────────────────────────
        egui::Panel::left("settings_panel")
            .resizable(true)
            .default_size(240.0)
            .min_size(200.0)
            .show_inside(ui, |ui: &mut egui::Ui| {
                ScrollArea::vertical().show(ui, |ui: &mut egui::Ui| {
                    self.draw_settings(ui, frame);
                });
            });

        // ── Central image preview ─────────────────────────────────────────────
        egui::CentralPanel::default().show_inside(ui, |ui: &mut egui::Ui| {
            self.draw_preview(ui);
        });
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Panel drawing helpers
// ──────────────────────────────────────────────────────────────────────────────

impl ScannerApp {
    fn draw_settings(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        ui.heading("Settings");
        ui.add_space(8.0);

        ui.label("Root directory:");
        ui.text_edit_singleline(&mut self.root_directory)
            .on_hover_text(
                "Houses 'waves', 'wavesfinished', and 'wavesfinished/UncapturedPO'",
            );

        ui.add_space(4.0);
        ui.label("Tesseract path:");
        ui.horizontal(|ui: &mut egui::Ui| {
            ui.text_edit_singleline(&mut self.tesseract_cmd);
            let (hint, colour) = if self.tesseract_ok {
                ("✓", Color32::GREEN)
            } else {
                ("✗", Color32::RED)
            };
            ui.label(RichText::new(hint).color(colour));
        });

        ui.add_space(4.0);
        ui.label("Scan mode:");
        egui::ComboBox::from_id_salt("scan_mode")
            .selected_text(self.mode.label())
            .show_ui(ui, |ui: &mut egui::Ui| {
                for m in ScanMode::ALL {
                    ui.selectable_value(&mut self.mode, m, m.label());
                }
            });

        ui.add_space(8.0);
        if ui.button("Apply settings").clicked() {
            self.apply_settings(&ctx);
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // Scan button — enabled only on Windows.
        #[cfg(windows)]
        let scan_enabled = !self.is_scanning;
        #[cfg(not(windows))]
        let scan_enabled = false;

        let label = if self.is_scanning { "⏳ Scanning…" } else { "📷 Scan Now" };

        if ui
            .add_enabled(
                scan_enabled,
                egui::Button::new(RichText::new(label).size(16.0))
                    .min_size(Vec2::new(ui.available_width(), 40.0)),
            )
            .clicked()
        {
            self.trigger_hardware_scan(&ctx);
        }

        #[cfg(not(windows))]
        ui.label(
            RichText::new("Hardware scan: Windows + WIA only")
                .color(Color32::GRAY)
                .italics(),
        );

        if !self.status.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(&self.status).color(Color32::LIGHT_BLUE));
        }

        if let Some(id) = self.last_id.clone() {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label("Last extracted ID:");
            ui.label(
                RichText::new(&id)
                    .font(FontId::proportional(18.0))
                    .color(Color32::YELLOW)
                    .strong(),
            );
        }
    }

    fn draw_preview(&self, ui: &mut egui::Ui) {
        ui.heading("Image Preview");
        ui.add_space(8.0);

        if let Some(tex) = &self.preview_texture {
            let available = ui.available_size();
            let [tw, th] = [tex.size()[0] as f32, tex.size()[1] as f32];
            let scale = (available.x / tw).min((available.y - 40.0) / th).min(1.0);
            let display_size = Vec2::new(tw * scale, th * scale);

            ui.centered_and_justified(|ui: &mut egui::Ui| {
                ui.image((tex.id(), display_size));
            });

            if let Some(path) = &self.preview_path {
                ui.label(
                    RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                        .color(Color32::GRAY),
                );
            }
        } else {
            ui.centered_and_justified(|ui: &mut egui::Ui| {
                ui.label(
                    RichText::new(
                        "No image loaded.\n\
                         Drop a PNG into the 'waves' folder\n\
                         or press Scan Now.",
                    )
                    .color(Color32::GRAY)
                    .size(14.0),
                );
            });
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Module-level helpers
// ──────────────────────────────────────────────────────────────────────────────

fn default_root() -> String {
    PathBuf::from(std::path::MAIN_SEPARATOR.to_string())
        .join("renamescans")
        .to_string_lossy()
        .into_owned()
}

fn derive_dirs(root: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = PathBuf::from(root);
    let waves = root.join("waves");
    let finished = root.join("wavesfinished");
    let error = finished.join("UncapturedPO");
    (waves, finished, error)
}

fn init_dirs_and_watcher(
    waves_dir: &Path,
    finished_dir: &Path,
    error_dir: &Path,
    tx: mpsc::Sender<AppEvent>,
    ctx: Context,
) -> Option<RecommendedWatcher> {
    for dir in [waves_dir, finished_dir, error_dir] {
        if let Err(e) = ensure_dir(dir) {
            error!("Could not create {dir:?}: {e}");
        }
    }

    match start_watcher(waves_dir.to_path_buf(), tx, ctx) {
        Ok(w) => Some(w),
        Err(e) => {
            error!("Could not start watcher: {e}");
            None
        }
    }
}
