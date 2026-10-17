use crate::util::extract_json_block;
use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Serialize)]
pub struct Audio {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    pub sample_rate: u64,
    pub channels: u64,
    pub duration_ms: u64,
}

pub fn execute(video: &str) -> Result<(), String> {
    eprintln!("loudnorm: {video}");

    // Check audio stream
    let stream_out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=sample_rate,channels",
            "-of",
            "json",
            video,
        ])
        .output()
        .map_err(|e| format!("ffprobe failed: {e}"))?;

    let streams: Value = serde_json::from_slice(&stream_out.stdout)
        .map_err(|e| format!("Cannot parse ffprobe output: {e}"))?;

    let audio_stream = streams["streams"]
        .as_array()
        .and_then(|s| s.first())
        .ok_or("No audio stream found")?;

    // Get duration
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
        .map_err(|e| format!("ffprobe failed: {e}"))?;

    let dur_s: f64 = String::from_utf8_lossy(&dur_out.stdout)
        .trim()
        .parse()
        .map_err(|_| "Could not parse duration")?;

    // Run loudnorm
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            video,
            "-af",
            "loudnorm=print_format=json",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg failed: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Find JSON block with input_i
    let json_str =
        extract_json_block(&stderr).ok_or("Could not parse loudnorm JSON from ffmpeg output")?;

    let loud: Value =
        serde_json::from_str(&json_str).map_err(|e| format!("Invalid loudnorm JSON: {e}"))?;

    let parse_f64 = |key: &str| -> f64 {
        loud[key]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0)
    };

    let result = Audio {
        input_i: parse_f64("input_i"),
        input_tp: parse_f64("input_tp"),
        input_lra: parse_f64("input_lra"),
        input_thresh: parse_f64("input_thresh"),
        sample_rate: audio_stream["sample_rate"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        channels: audio_stream["channels"].as_u64().unwrap_or(0),
        duration_ms: (dur_s * 1000.0).round() as u64,
    };

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(())
}
