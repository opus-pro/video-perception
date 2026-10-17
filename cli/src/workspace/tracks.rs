use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Per-frame signal data from the signalstats track.
#[derive(Deserialize)]
#[allow(dead_code)]
struct SignalFrame {
    f: u64,
    t: u64,
    y: f64,
    yn: f64,
    yx: f64,
    sa: f64,
}

/// Load a signalstats track JSONL and aggregate per scene.
///
/// Produces the same per-scene format as the original signalstats sensor,
/// but computed at Observation generation time from raw per-frame data.
/// This decouples measurement (capability) from Observation aggregation.
pub fn aggregate_signalstats(track_path: &Path, scenes: &Value, fps: f64) -> Result<Value, String> {
    let frames = load_track(track_path)?;
    let scene_arr = scenes
        .get("scenes")
        .and_then(|s| s.as_array())
        .ok_or("scenes.json missing 'scenes' array")?;

    let mut results: Vec<Value> = Vec::new();

    for scene in scene_arr {
        let idx = scene.get("idx").and_then(|v| v.as_u64()).unwrap_or(0);
        let start_ms = scene.get("start_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        let end_ms = scene.get("end_ms").and_then(|v| v.as_u64()).unwrap_or(0);

        let start_frame = ((start_ms as f64 / 1000.0) * fps).round() as u64;
        let end_frame = ((end_ms as f64 / 1000.0) * fps).round() as u64;

        let scene_frames: Vec<&SignalFrame> = frames
            .iter()
            .filter(|f| f.f >= start_frame && f.f < end_frame)
            .collect();

        let n = scene_frames.len();
        if n == 0 {
            continue;
        }

        let yavgs: Vec<f64> = scene_frames.iter().map(|f| f.y).collect();
        let yavg_mean = mean(&yavgs);
        let yavg_std = std_dev(&yavgs, yavg_mean);

        let yranges: Vec<f64> = scene_frames
            .iter()
            .map(|f| (f.yx - f.yn).max(0.0))
            .collect();

        let mut diffs: Vec<f64> = Vec::new();
        for i in 1..scene_frames.len() {
            diffs.push((scene_frames[i].y - scene_frames[i - 1].y).abs());
        }
        if diffs.is_empty() {
            diffs.push(0.0);
        }

        let nf = n as f64;
        let nd = diffs.len() as f64;
        let dark = scene_frames.iter().filter(|f| f.y < 15.0).count();
        let bright = scene_frames.iter().filter(|f| f.y > 240.0).count();
        let low_var = yranges.iter().filter(|&&r| r < 20.0).count();
        let static_f = diffs.iter().filter(|&&d| d < 0.5).count();

        results.push(json!({
            "scene_idx": idx,
            "frame_count": n,
            "yavg_mean": r2(yavg_mean),
            "yavg_std": r2(yavg_std),
            "yavg_min": r2(yavgs.iter().copied().fold(f64::MAX, f64::min)),
            "yavg_max": r2(yavgs.iter().copied().fold(f64::MIN, f64::max)),
            "yrange_mean": r2(mean(&yranges)),
            "yrange_min": r2(yranges.iter().copied().fold(f64::MAX, f64::min)),
            "satavg_mean": r2(mean(&scene_frames.iter().map(|f| f.sa).collect::<Vec<_>>())),
            "frame_diff_mean": r2(mean(&diffs)),
            "frame_diff_max": r2(diffs.iter().copied().fold(f64::MIN, f64::max)),
            "frame_diff_std": r2(std_dev(&diffs, mean(&diffs))),
            "dark_frame_ratio": r4(dark as f64 / nf),
            "bright_frame_ratio": r4(bright as f64 / nf),
            "low_variance_ratio": r4(low_var as f64 / nf),
            "static_frame_ratio": r4(static_f as f64 / nd),
        }));
    }

    Ok(Value::Array(results))
}

/// Compute global stats from raw track for the dashboard.
/// Uses ALL frames — more accurate than averaging scene averages.
pub fn global_from_track(track_path: &Path) -> Result<Value, String> {
    let frames = load_track(track_path)?;
    if frames.is_empty() {
        return Ok(json!(null));
    }

    let yavgs: Vec<f64> = frames.iter().map(|f| f.y).collect();
    let mean_val = mean(&yavgs);
    let std_val = std_dev(&yavgs, mean_val);

    let nf = frames.len() as f64;
    let dark = frames.iter().filter(|f| f.y < 15.0).count();
    let bright = frames.iter().filter(|f| f.y > 240.0).count();

    let mut diffs: Vec<f64> = Vec::new();
    for i in 1..frames.len() {
        diffs.push((frames[i].y - frames[i - 1].y).abs());
    }
    let static_f = diffs.iter().filter(|&&d| d < 0.5).count();

    Ok(json!({
        "brightness_mean": r1(mean_val),
        "brightness_std": r1(std_val),
        "total_frames": frames.len(),
        "dark_frame_ratio": r4(dark as f64 / nf),
        "bright_frame_ratio": r4(bright as f64 / nf),
        "static_frame_ratio": r4(if diffs.is_empty() { 0.0 } else { static_f as f64 / diffs.len() as f64 }),
    }))
}

fn load_track(path: &Path) -> Result<Vec<SignalFrame>, String> {
    let file = fs::File::open(path).map_err(|e| format!("open track {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    let mut frames = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let f: SignalFrame =
            serde_json::from_str(&line).map_err(|e| format!("parse track frame: {e}"))?;
        frames.push(f);
    }
    Ok(frames)
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn std_dev(v: &[f64], m: f64) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn r4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}
