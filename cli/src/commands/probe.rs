use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Serialize)]
pub struct Metadata {
    pub video: String,
    pub duration_ms: u64,
    pub width: u64,
    pub height: u64,
    pub fps: f64,
    pub codec: String,
    pub audio_tracks: u64,
    pub filesize_bytes: u64,
}

pub fn execute(video: &str) -> Result<(), String> {
    eprintln!("probe: {video}");

    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            video,
        ])
        .output()
        .map_err(|e| format!("ffprobe failed: {e}"))?;

    if !output.status.success() {
        return Err(format!("ffprobe exited with {}", output.status));
    }

    let info: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse ffprobe output: {e}"))?;

    let vs = info["streams"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["codec_type"] == "video"))
        .ok_or("No video stream found")?;

    let audio_count = info["streams"]
        .as_array()
        .map(|s| s.iter().filter(|s| s["codec_type"] == "audio").count() as u64)
        .unwrap_or(0);

    let fps = parse_frame_rate(vs["r_frame_rate"].as_str().unwrap_or("24/1"));

    let duration_s: f64 = info["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    let meta = Metadata {
        video: video.to_string(),
        duration_ms: (duration_s * 1000.0).round() as u64,
        width: vs["width"].as_u64().unwrap_or(0),
        height: vs["height"].as_u64().unwrap_or(0),
        fps: (fps * 1000.0).round() / 1000.0,
        codec: vs["codec_name"].as_str().unwrap_or("unknown").to_string(),
        audio_tracks: audio_count,
        filesize_bytes: info["format"]["size"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    };

    println!("{}", serde_json::to_string_pretty(&meta).unwrap());
    Ok(())
}

fn parse_frame_rate(s: &str) -> f64 {
    let parts: Vec<f64> = s.split('/').filter_map(|p| p.parse().ok()).collect();
    match parts.as_slice() {
        [num, den] if *den > 0.0 => num / den,
        [num] => *num,
        _ => 24.0,
    }
}
