use crate::util::parse_fps;
use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
pub struct FramerateAnalysis {
    pub container_fps: f64,
    pub duplicate_frame_count: u64,
    pub total_frames: u64,
    pub duplicate_ratio: f64,
    pub interlace_detected: bool,
    pub interlace_type: String,
    pub content_fps_estimate: f64,
    pub pulldown_detected: bool,
}

pub fn execute(video: &str) -> Result<(), String> {
    eprintln!("framerate: {video}");

    // Get container FPS from ffprobe
    let probe_out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate,nb_frames",
            "-of",
            "csv=p=0",
            video,
        ])
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;

    let probe_str = String::from_utf8_lossy(&probe_out.stdout);
    let parts: Vec<&str> = probe_str.trim().split(',').collect();
    let container_fps = parse_fps(parts.first().copied().unwrap_or("0/1"));

    // Detect duplicate frames via mpdecimate
    // mpdecimate marks frames it would drop (duplicates)
    let decimate_out = Command::new("ffmpeg")
        .args([
            "-i",
            video,
            "-vf",
            "mpdecimate=hi=64*12:lo=64*5:frac=0.33",
            "-loglevel",
            "debug",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg mpdecimate: {e}"))?;

    let debug_log = String::from_utf8_lossy(&decimate_out.stderr);
    let mut total_frames: u64 = 0;
    let mut drop_frames: u64 = 0;

    for line in debug_log.lines() {
        if line.contains("Discarding frame") || line.contains("drop_count") {
            // Count "drop" frames logged by mpdecimate
            if line.contains("drop_count:") {
                if let Some(count_str) = line.split("drop_count:").nth(1) {
                    if let Ok(c) = count_str
                        .trim()
                        .split_whitespace()
                        .next()
                        .unwrap_or("0")
                        .parse::<u64>()
                    {
                        if c > 0 {
                            drop_frames += 1;
                        }
                    }
                }
            }
        }
        if line.contains("pts_time:") {
            total_frames += 1;
        }
    }

    // Simpler approach: count frames via ffprobe
    if total_frames == 0 {
        let count_out = Command::new("ffprobe")
            .args([
                "-v",
                "quiet",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=nb_read_frames",
                "-of",
                "csv=p=0",
                video,
            ])
            .output()
            .ok();
        if let Some(o) = count_out {
            total_frames = String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(0);
        }
    }

    let duplicate_ratio = if total_frames > 0 {
        drop_frames as f64 / total_frames as f64
    } else {
        0.0
    };

    // Estimate content FPS from duplicate ratio
    let content_fps = if duplicate_ratio > 0.15 {
        container_fps * (1.0 - duplicate_ratio)
    } else {
        container_fps
    };

    // Detect interlacing via idet
    let idet_out = Command::new("ffmpeg")
        .args([
            "-i",
            video,
            "-vf",
            "idet",
            "-frames:v",
            "500",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg idet: {e}"))?;

    let idet_log = String::from_utf8_lossy(&idet_out.stderr);
    let mut tff_count: u64 = 0;
    let mut bff_count: u64 = 0;
    let mut progressive_count: u64 = 0;

    for line in idet_log.lines() {
        if line.contains("Multi frame detection:") || line.contains("Repeated Fields:") {
            if let Some(after) = line.split("TFF:").nth(1) {
                tff_count += after
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
            }
            if let Some(after) = line.split("BFF:").nth(1) {
                bff_count += after
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
            }
            if let Some(after) = line.split("Progressive:").nth(1) {
                progressive_count += after
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }
    }

    let interlaced = tff_count + bff_count;
    let interlace_detected = interlaced > progressive_count && interlaced > 10;
    let interlace_type = if interlace_detected {
        if tff_count > bff_count {
            "top_field_first"
        } else {
            "bottom_field_first"
        }
    } else {
        "progressive"
    };

    // 3:2 pulldown detection: ~20% duplicate ratio at 29.97fps = 23.976 source
    let pulldown_detected = container_fps > 29.0
        && container_fps < 30.0
        && duplicate_ratio > 0.15
        && duplicate_ratio < 0.25;

    let result = FramerateAnalysis {
        container_fps: (container_fps * 1000.0).round() / 1000.0,
        duplicate_frame_count: drop_frames,
        total_frames,
        duplicate_ratio: (duplicate_ratio * 10000.0).round() / 10000.0,
        interlace_detected,
        interlace_type: interlace_type.to_string(),
        content_fps_estimate: (content_fps * 1000.0).round() / 1000.0,
        pulldown_detected,
    };

    eprintln!(
        "  container={:.3}fps content={:.3}fps dups={}/{} interlace={} pulldown={}",
        result.container_fps,
        result.content_fps_estimate,
        drop_frames,
        total_frames,
        interlace_type,
        if pulldown_detected { "3:2" } else { "none" }
    );

    let json = serde_json::to_string_pretty(&result).map_err(|e| format!("JSON serialize: {e}"))?;
    println!("{json}");
    Ok(())
}
