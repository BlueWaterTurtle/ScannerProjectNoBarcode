use std::{path::Path, process::Command};

use anyhow::{Context, Result};
use regex::Regex;

// ──────────────────────────────────────────────────────────────────────────────
// Scan mode
// ──────────────────────────────────────────────────────────────────────────────

/// How to extract an identifier from a scanned image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanMode {
    /// Try barcode first; fall back to OCR if no barcode is found.
    Auto,
    /// Tesseract OCR only — searches text for a PO-number pattern.
    OcrOnly,
    /// ZXing barcode / QR-code only.
    BarcodeOnly,
}

impl ScanMode {
    pub const ALL: [ScanMode; 3] = [ScanMode::Auto, ScanMode::OcrOnly, ScanMode::BarcodeOnly];

    pub fn label(self) -> &'static str {
        match self {
            ScanMode::Auto => "Auto (barcode → OCR)",
            ScanMode::OcrOnly => "OCR only",
            ScanMode::BarcodeOnly => "Barcode only",
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tesseract (OCR)
// ──────────────────────────────────────────────────────────────────────────────

/// Return the name / path of the `tesseract` executable, or an error if it
/// cannot be found on PATH.
pub fn find_tesseract() -> Result<String> {
    // `tesseract --version` exits 0 and writes to stderr on most installations.
    let probe = Command::new("tesseract").arg("--version").output();
    match probe {
        Ok(out) if out.status.success() || !out.stderr.is_empty() => {
            return Ok("tesseract".to_string());
        }
        _ => {}
    }

    // Fall back to `which` / `where`.
    #[cfg(unix)]
    let locator = "which";
    #[cfg(windows)]
    let locator = "where";
    #[cfg(not(any(unix, windows)))]
    let locator = "which";

    let out = Command::new(locator)
        .arg("tesseract")
        .output()
        .context("Could not run 'which tesseract'")?;

    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !path.is_empty() {
        Ok(path)
    } else {
        anyhow::bail!("Tesseract not found on PATH — please install Tesseract OCR.")
    }
}

/// Run Tesseract on `image_path` and return the extracted text.
pub fn run_ocr(tesseract_cmd: &str, image_path: &Path) -> Result<String> {
    let output = Command::new(tesseract_cmd)
        .arg(image_path)
        .arg("stdout")
        .output()
        .with_context(|| format!("Failed to launch Tesseract for {image_path:?}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Tesseract non-zero exit: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// ──────────────────────────────────────────────────────────────────────────────
// Barcode / QR (rxing)
// ──────────────────────────────────────────────────────────────────────────────

/// Attempt to decode a barcode or QR code from `image_path`.
/// Returns `None` if no barcode is found (not an error).
pub fn read_barcode(image_path: &Path) -> Result<Option<String>> {
    let path_str = image_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Image path contains non-UTF-8 characters"))?;

    match rxing::helpers::detect_in_file(path_str, None) {
        Ok(result) => Ok(Some(result.getText().to_string())),
        Err(_) => Ok(None), // "not found" and other errors are treated as "no barcode"
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PO-number extraction
// ──────────────────────────────────────────────────────────────────────────────

/// Extract the first PO-number match (`[A-Z]*PO\d+`) from arbitrary text.
pub fn extract_po_number(text: &str) -> Option<String> {
    let re = Regex::new(r"[A-Z]*PO\d+").expect("regex is valid");
    re.find(text).map(|m| m.as_str().to_string())
}

// ──────────────────────────────────────────────────────────────────────────────
// Combined pipeline
// ──────────────────────────────────────────────────────────────────────────────

/// Run the full extraction pipeline for one image and return the best
/// identifier found (PO number if the pattern matches, otherwise the raw
/// barcode value), or `None` if nothing was found.
pub fn process_image(
    image_path: &Path,
    mode: ScanMode,
    tesseract_cmd: &str,
) -> Result<Option<String>> {
    match mode {
        ScanMode::BarcodeOnly => {
            let raw = read_barcode(image_path)?;
            Ok(raw.map(|t| extract_po_number(&t).unwrap_or(t)))
        }

        ScanMode::OcrOnly => {
            let text = run_ocr(tesseract_cmd, image_path)?;
            Ok(extract_po_number(&text))
        }

        ScanMode::Auto => {
            // Try barcode first.
            if let Ok(Some(raw)) = read_barcode(image_path) {
                let id = extract_po_number(&raw).unwrap_or(raw);
                return Ok(Some(id));
            }
            // Fall back to OCR.
            let text = run_ocr(tesseract_cmd, image_path)?;
            Ok(extract_po_number(&text))
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Unit tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn po_standard() {
        assert_eq!(
            extract_po_number("Invoice PO12345 here"),
            Some("PO12345".into())
        );
    }

    #[test]
    fn po_prefixed() {
        assert_eq!(
            extract_po_number("ref PPO98765"),
            Some("PPO98765".into())
        );
    }

    #[test]
    fn po_missing() {
        assert_eq!(extract_po_number("nothing here"), None);
    }

    #[test]
    fn scan_mode_labels_are_unique() {
        let labels: Vec<_> = ScanMode::ALL.iter().map(|m| m.label()).collect();
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(labels.len(), unique.len());
    }
}
