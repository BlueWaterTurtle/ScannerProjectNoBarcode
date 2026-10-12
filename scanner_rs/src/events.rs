use std::path::PathBuf;

/// Events sent from background threads to the GUI thread.
#[derive(Debug)]
pub enum AppEvent {
    /// A new PNG appeared in the watched `waves` directory.
    FileDetected(PathBuf),

    /// Processing (OCR / barcode) finished for a file.
    ProcessingDone {
        /// Original file path (already moved — for display only).
        source: PathBuf,
        /// Extracted PO number, if any was found.
        po_number: Option<String>,
        /// Where the file ended up, or an error string.
        destination: Result<PathBuf, String>,
    },

    /// A hardware scan completed and the image was saved to this path.
    /// Only ever sent on Windows; the variant is kept unconditional so the
    /// rest of the match arms remain exhaustive everywhere.
    #[allow(dead_code)]
    HardwareScanReady(PathBuf),

    /// A plain log line to append to the activity log.
    Log(String),
}
