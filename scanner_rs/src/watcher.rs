use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use anyhow::Result;
use log::{error, info, warn};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::events::AppEvent;

/// Spawn a background thread that watches `waves_dir` for new PNG files and
/// sends [`AppEvent::FileDetected`] events over `tx`.
///
/// Returns the watcher handle — keep it alive for as long as you want events.
pub fn start_watcher(
    waves_dir: PathBuf,
    tx: mpsc::Sender<AppEvent>,
    egui_ctx: egui::Context,
) -> Result<RecommendedWatcher> {
    // The notify callback runs on an OS thread; it forwards raw events via an
    // internal channel so we can re-raise them as AppEvents from a dedicated
    // thread that also calls `request_repaint`.
    let (notify_tx, notify_rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = notify_tx.send(res);
        },
        Config::default(),
    )?;

    watcher.watch(&waves_dir, RecursiveMode::NonRecursive)?;
    info!("Watching directory: {waves_dir:?}");

    // Relay thread: translate notify events → AppEvents and wake the GUI.
    thread::spawn(move || {
        for result in notify_rx {
            match result {
                Ok(event) if matches!(event.kind, EventKind::Create(_)) => {
                    for path in event.paths {
                        if path
                            .extension()
                            .map(|e| e.to_string_lossy().to_lowercase() == "png")
                            .unwrap_or(false)
                        {
                            info!("Detected new PNG: {path:?}");
                            let _ = tx.send(AppEvent::FileDetected(path));
                            egui_ctx.request_repaint();
                        } else {
                            warn!("Ignoring non-PNG file: {path:?}");
                        }
                    }
                }
                Ok(_) => {} // other event kinds — ignore
                Err(e) => {
                    error!("Watcher error: {e}");
                    let _ = tx.send(AppEvent::Log(format!("Watcher error: {e}")));
                    egui_ctx.request_repaint();
                }
            }
        }
    });

    Ok(watcher)
}

/// Wait for `path` to become writable, retrying up to `max_tries` times with
/// a 2-second pause between attempts.
pub fn wait_for_file_access(path: &std::path::Path, max_tries: u32) -> bool {
    for attempt in 1..=max_tries {
        match std::fs::OpenOptions::new().append(true).open(path) {
            Ok(_) => return true,
            Err(e) => {
                info!("Waiting for file access ({attempt}/{max_tries}): {path:?} — {e}");
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
    false
}
