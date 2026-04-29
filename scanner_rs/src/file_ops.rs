use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use log::info;
use uuid::Uuid;

/// Create `dir` (and all parents) if it does not already exist.
pub fn ensure_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory {dir:?}"))?;
        info!("Created directory: {dir:?}");
    }
    Ok(())
}

/// Move `src` to `dest_dir`.
///
/// * If `po_number` is `Some`, the destination file is named
///   `<po>_<6-hex-uuid><ext>` (ensures uniqueness).
/// * If `po_number` is `None`, the original filename is kept.
pub fn move_file(src: &Path, dest_dir: &Path, po_number: Option<&str>) -> Result<PathBuf> {
    ensure_dir(dest_dir)?;

    let dest = match po_number {
        Some(po) => {
            let ext = src
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            // Use the first 6 hex chars of a UUID v4 to create a unique suffix.
            let suffix = &Uuid::new_v4().simple().to_string()[..6];
            dest_dir.join(format!("{po}_{suffix}{ext}"))
        }
        None => dest_dir.join(
            src.file_name()
                .expect("source path always has a file name"),
        ),
    };

    fs::rename(src, &dest)
        .with_context(|| format!("Failed to move {src:?} → {dest:?}"))?;
    Ok(dest)
}
