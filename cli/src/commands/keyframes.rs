use std::fs;
use std::process::Command;

/// Extract keyframes at 2fps (every 500ms), independent of scene detection.
///
/// Outputs:
///   {out_dir}/tracks/keyframes/frame_NNNN.png   — PNG frames
///   {out_dir}/tracks/keyframes.track.jsonl       — manifest with timestamps
pub fn execute(video: &str, out_dir: &str) -> Result<(), String> {
    let kf_dir = format!("{out_dir}/tracks/keyframes");
    fs::create_dir_all(&kf_dir).map_err(|e| format!("mkdir: {e}"))?;

    // Get duration from ffprobe
    let dur_out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            video,
        ])
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    let duration_sec: f64 = String::from_utf8_lossy(&dur_out.stdout)
        .trim()
        .parse()
        .map_err(|_| "cannot parse video duration")?;

    if duration_sec <= 0.0 {
        return Err("video has zero duration".into());
    }

    let fps = 2.0; // 2 frames per second
    let n_frames = (duration_sec * fps).ceil() as usize;

    eprintln!("keyframes: extracting {n_frames} frames at {fps}fps from {duration_sec:.1}s video");

    // Extract all frames at once using fps filter
    let fps_filter = format!("fps={fps}");
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            video,
            "-vf",
            &fps_filter,
            "-q:v",
            "2",
            &format!("{kf_dir}/frame_%04d.png"),
        ])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "ffmpeg frame extraction failed: {}",
            stderr.lines().last().unwrap_or("")
        ));
    }

    // Build manifest from extracted files
    let interval_ms = (1000.0 / fps) as u64; // 500ms
    let mut manifest = String::new();
    let mut count = 0usize;

    for idx in 1..=n_frames {
        let filename = format!("frame_{idx:04}.png");
        let path = format!("{kf_dir}/{filename}");
        if !std::path::Path::new(&path).exists() {
            break;
        }
        let t_ms = (idx as u64 - 1) * interval_ms;
        let entry = serde_json::json!({
            "t": t_ms,
            "path": format!("keyframes/{filename}"),
            "idx": idx,
        });
        manifest.push_str(&serde_json::to_string(&entry).unwrap());
        manifest.push('\n');
        count += 1;
    }

    let manifest_path = format!("{out_dir}/tracks/keyframes.track.jsonl");
    fs::write(&manifest_path, &manifest).map_err(|e| format!("write manifest: {e}"))?;

    eprintln!("keyframes: {count} frames → {manifest_path}");

    // Output JSON summary to stdout (for deck.json / pipeline)
    println!(
        "{}",
        serde_json::json!({
            "frames": count,
            "fps": fps,
            "interval_ms": interval_ms,
            "manifest": "tracks/keyframes.track.jsonl",
        })
    );

    Ok(())
}
