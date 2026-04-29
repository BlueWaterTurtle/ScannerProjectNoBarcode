use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use log::{error, info, warn};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use regex::Regex;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// Monitor a directory for PNG files, extract PO numbers via OCR, and
/// rename/move the files accordingly.
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Root directory that houses `waves`, `wavesfinished`, and
    /// `wavesfinished/UncapturedPO`. Defaults to `<drive-root>/renamescans`.
    #[arg(long, default_value_os_t = default_root())]
    root_directory: PathBuf,
}

/// Returns `<drive-root>/renamescans` as the default root directory.
fn default_root() -> PathBuf {
    // On Unix `/renamescans`; on Windows `\renamescans`.
    PathBuf::from(std::path::MAIN_SEPARATOR.to_string()).join("renamescans")
}

// ---------------------------------------------------------------------------
// Tesseract helpers
// ---------------------------------------------------------------------------

/// Verify that the `tesseract` executable is on PATH and return its path.
fn find_tesseract() -> Result<String> {
    let output = Command::new("tesseract").arg("--version").output();
    match output {
        Ok(out) if out.status.success() || !out.stderr.is_empty() => {
            // `tesseract --version` writes to stderr on some versions.
            Ok("tesseract".to_string())
        }
        _ => {
            // Fall back to which/where
            #[cfg(unix)]
            let which_cmd = "which";
            #[cfg(windows)]
            let which_cmd = "where";

            let result = Command::new(which_cmd).arg("tesseract").output();
            match result {
                Ok(out) if out.status.success() => {
                    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if path.is_empty() {
                        anyhow::bail!("Tesseract not found on PATH. Please install Tesseract OCR.");
                    }
                    Ok(path)
                }
                _ => anyhow::bail!("Tesseract not found on PATH. Please install Tesseract OCR."),
            }
        }
    }
}

/// Run Tesseract OCR on `image_path` and return the extracted text.
///
/// Calls: `tesseract <image_path> stdout`
fn run_ocr(tesseract_cmd: &str, image_path: &Path) -> Result<String> {
    let output = Command::new(tesseract_cmd)
        .arg(image_path)
        .arg("stdout")
        .output()
        .with_context(|| format!("Failed to launch Tesseract for {:?}", image_path))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Tesseract returned a non-zero exit code: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// ---------------------------------------------------------------------------
// PO-number extraction
// ---------------------------------------------------------------------------

/// Extract the first PO number (e.g. `PO12345` or `PPO12345`) from OCR text.
fn extract_po_number(ocr_text: &str) -> Option<String> {
    let re = Regex::new(r"[A-Z]*PO\d+").expect("PO regex is valid");
    re.find(ocr_text).map(|m| m.as_str().to_string())
}

// ---------------------------------------------------------------------------
// Directory utilities
// ---------------------------------------------------------------------------

/// Create `dir` (and all parents) if it does not already exist.
fn ensure_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory {:?}", dir))?;
        info!("Created directory: {:?}", dir);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// File-event handling
// ---------------------------------------------------------------------------

/// Wait until `file_path` can be opened for writing (i.e. is fully flushed by
/// the writer), retrying up to `max_tries` times with a 2-second pause.
fn wait_for_file_access(file_path: &Path, max_tries: u32) -> bool {
    for attempt in 1..=max_tries {
        match fs::OpenOptions::new().append(true).open(file_path) {
            Ok(_) => return true,
            Err(e) => {
                info!(
                    "Waiting for file access ({}/{}): {:?} — {}",
                    attempt, max_tries, file_path, e
                );
                thread::sleep(Duration::from_secs(2));
            }
        }
    }
    false
}

/// Move `src` to `dest_dir/<po>_<6-hex-uuid><ext>` when `unique_prefix` is
/// `Some`, or to `dest_dir/<filename>` when it is `None`.
fn move_file(src: &Path, dest_dir: &Path, unique_prefix: Option<&str>) -> Result<PathBuf> {
    ensure_dir(dest_dir)?;

    let dest = if let Some(prefix) = unique_prefix {
        let ext = src
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let suffix = &Uuid::new_v4().simple().to_string()[..6];
        dest_dir.join(format!("{prefix}_{suffix}{ext}"))
    } else {
        dest_dir.join(
            src.file_name()
                .expect("src always has a file name at this point"),
        )
    };

    fs::rename(src, &dest)
        .with_context(|| format!("Failed to move {:?} -> {:?}", src, dest))?;
    Ok(dest)
}

/// Core processing logic for a single newly-created file.
fn process_file(
    file_path: &Path,
    finished_dir: &Path,
    error_dir: &Path,
    tesseract_cmd: &str,
) {
    info!("New file detected: {:?}", file_path);

    // Give the writer an initial moment to finish.
    thread::sleep(Duration::from_secs(3));

    // Wait until we can open the file.
    if !wait_for_file_access(file_path, 10) {
        error!("Could not acquire file access after retries: {:?}", file_path);
        return;
    }

    // Only process PNG files.
    let ext = file_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());

    if ext.as_deref() != Some("png") {
        warn!("Unsupported file format, skipping: {:?}", file_path);
        return;
    }

    // Run OCR.
    let ocr_text = match run_ocr(tesseract_cmd, file_path) {
        Ok(text) => {
            info!("OCR raw output: {}", text);
            text
        }
        Err(e) => {
            error!("Error running OCR on {:?}: {}", file_path, e);
            if let Err(mv_err) = move_file(file_path, error_dir, None) {
                error!("Could not move {:?} to error dir: {}", file_path, mv_err);
            }
            return;
        }
    };

    // Extract PO number.
    match extract_po_number(&ocr_text) {
        Some(po) => {
            info!("Extracted PO Number: {}", po);
            match move_file(file_path, finished_dir, Some(&po)) {
                Ok(dest) => info!("File moved to: {:?}", dest),
                Err(e) => error!("Failed to move file: {}", e),
            }
        }
        None => {
            info!("PO Number could not be extracted.");
            match move_file(file_path, error_dir, None) {
                Ok(dest) => warn!("File with no PO number moved to: {:?}", dest),
                Err(e) => error!("Failed to move file to error dir: {}", e),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    // Initialise structured logging (respects RUST_LOG env var; defaults to INFO).
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    // Build directory paths.
    let root = &args.root_directory;
    let waves_dir = root.join("waves");
    let finished_dir = root.join("wavesfinished");
    let error_dir = finished_dir.join("UncapturedPO");

    // Ensure all directories exist.
    for dir in [root, &waves_dir, &finished_dir, &error_dir] {
        ensure_dir(dir)?;
    }

    // Verify Tesseract is available.
    let tesseract_cmd = find_tesseract().context(
        "Tesseract executable not found. Please install Tesseract OCR or add it to PATH.",
    )?;
    info!("Tesseract successfully located: {}", tesseract_cmd);

    // Set up the filesystem watcher.
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .context("Failed to create filesystem watcher")?;

    watcher
        .watch(&waves_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("Failed to watch directory {:?}", waves_dir))?;

    info!("Monitoring directory: {:?}", waves_dir);

    // Event loop — blocks until the watcher is dropped or the process exits.
    for result in rx {
        match result {
            Ok(event) => {
                if matches!(event.kind, EventKind::Create(_)) {
                    for path in event.paths {
                        let finished = finished_dir.clone();
                        let error = error_dir.clone();
                        let cmd = tesseract_cmd.clone();
                        // Process each file in its own thread so the watcher
                        // stays responsive while a slow OCR call is in flight.
                        thread::spawn(move || {
                            process_file(&path, &finished, &error, &cmd);
                        });
                    }
                }
            }
            Err(e) => {
                error!("Filesystem watcher error: {}", e);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_standard_po_number() {
        let text = "Invoice ref:\nPO12345\nThank you for your order.";
        assert_eq!(extract_po_number(text), Some("PO12345".to_string()));
    }

    #[test]
    fn extracts_prefixed_po_number() {
        let text = "Reference: PPO98765 — please quote on all correspondence.";
        assert_eq!(extract_po_number(text), Some("PPO98765".to_string()));
    }

    #[test]
    fn returns_none_when_no_po_present() {
        let text = "No purchase order information found on this document.";
        assert_eq!(extract_po_number(text), None);
    }

    #[test]
    fn default_root_contains_renamescans() {
        let root = default_root();
        assert!(root.to_string_lossy().contains("renamescans"));
    }
}
