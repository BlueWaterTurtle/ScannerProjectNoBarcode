use std::path::Path;

use anyhow::Result;

/// Invoke the system scanner via WIA (Windows Image Acquisition) and save the
/// result as a PNG to `output_path`.
///
/// On non-Windows platforms this always returns an error — the GUI hides the
/// button in that case.
#[cfg(windows)]
pub fn scan_to_file(output_path: &Path) -> Result<()> {
    // The WIA COM automation model is easiest to drive from PowerShell.
    // `Transfer()` captures from the first available WIA scanner; `SaveFile()`
    // writes it in the format implied by the file extension (.png).
    let out = output_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Output path contains non-UTF-8 characters"))?
        .replace('"', "");          // sanitize — no double-quotes in path

    let script = format!(
        r#"
$mgr    = New-Object -ComObject WIA.DeviceManager
$infos  = @($mgr.DeviceInfos | Where-Object {{ $_.Type -eq 1 }})
if ($infos.Count -eq 0) {{ Write-Error 'No WIA scanner found'; exit 1 }}
$dev    = $infos[0].Connect()
$item   = $dev.Items.Item(1)
$image  = $item.Transfer()
$image.SaveFile("{out}")
"#
    );

    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ])
        .status()
        .map_err(|e| anyhow::anyhow!("Could not launch PowerShell: {e}"))?;

    anyhow::ensure!(status.success(), "WIA scan script exited with failure");
    anyhow::ensure!(
        output_path.exists(),
        "Scan appeared to succeed but output file was not created"
    );
    Ok(())
}

/// Stub for non-Windows platforms.
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn scan_to_file(_output_path: &Path) -> Result<()> {
    anyhow::bail!(
        "Hardware scanning via WIA is only available on Windows. \
         On other platforms, drop image files into the 'waves' folder."
    )
}
