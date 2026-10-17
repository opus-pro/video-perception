/// Shared utilities used across sensor commands and doctor.
use std::process::Command;

/// Run ImageMagick `magick` with given args. Returns trimmed stdout on success.
pub fn run_magick(args: &[&str]) -> Result<String, String> {
    let output = Command::new("magick")
        .args(args)
        .output()
        .map_err(|e| format!("magick: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!("magick exited with {}", output.status))
    }
}

/// Parse an ffprobe frame rate string like "30000/1001" or "29.97" into f64.
pub fn parse_fps(s: &str) -> f64 {
    if let Some((num, den)) = s.split_once('/') {
        let n: f64 = num.trim().parse().unwrap_or(0.0);
        let d: f64 = den.trim().parse().unwrap_or(1.0);
        if d > 0.0 { n / d } else { 0.0 }
    } else {
        s.trim().parse().unwrap_or(0.0)
    }
}

/// Extract a JSON object containing `"input_i"` from ffmpeg loudnorm stderr output.
pub fn extract_json_block(s: &str) -> Option<String> {
    let marker = "\"input_i\"";
    let pos = s.find(marker)?;
    let before = &s[..pos];
    let start = before.rfind('{')?;
    let after = &s[start..];
    let end = after.find('}')? + 1;
    Some(after[..end].to_string())
}

/// Check if a tool is on PATH.
pub fn which(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get version string for a tool (tries --version, -version, falls back to presence check).
pub fn get_version(name: &str) -> Option<String> {
    for flag in ["--version", "-version"] {
        if let Ok(out) = Command::new(name).arg(flag).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                let s = s.trim();
                if !s.is_empty() {
                    let line = s.lines().next().unwrap_or(s);
                    let short = if line.len() > 40 { &line[..40] } else { line };
                    return Some(short.to_string());
                }
            }
            let e = String::from_utf8_lossy(&out.stderr);
            if let Some(line) = e.lines().next() {
                if line.contains("version") || line.contains("Version") {
                    let short = if line.len() > 50 { &line[..50] } else { line };
                    return Some(short.to_string());
                }
            }
        }
    }
    if which(name) {
        Some("installed".into())
    } else {
        None
    }
}
