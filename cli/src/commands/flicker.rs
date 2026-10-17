use crate::util::parse_fps;
use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
pub struct FlickerFrame {
    pub frame: u64,
    pub time_ms: u64,
    pub luminance_delta: f64,
}

#[derive(Serialize)]
pub struct FlickerAnalysis {
    pub total_frames_analyzed: u64,
    pub max_luminance_delta: f64,
    pub flash_frames: Vec<FlickerFrame>,
    pub flash_count: u64,
    pub max_flash_frequency_hz: f64,
    pub bt1702_compliant: bool,
}

pub fn execute(video: &str) -> Result<(), String> {
    eprintln!("flicker: {video}");

    // Get FPS for timestamp calculation
    let probe_out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate",
            "-of",
            "csv=p=0",
            video,
        ])
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;

    let fps_str = String::from_utf8_lossy(&probe_out.stdout);
    let fps = parse_fps(fps_str.trim());
    if fps <= 0.0 {
        return Err("Cannot determine FPS".into());
    }

    // Use ffmpeg signalstats + metadata print to get per-frame YAVG (average luminance)
    // metadata=print:file=- sends metadata to stdout
    let stats_out = Command::new("ffmpeg")
        .args([
            "-i",
            video,
            "-vf",
            "signalstats=stat=tout+vrep+brng,metadata=print:file=-",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg signalstats: {e}"))?;

    let stdout_log = String::from_utf8_lossy(&stats_out.stdout);

    let mut luminances: Vec<f64> = Vec::new();
    for line in stdout_log.lines() {
        if line.contains("lavfi.signalstats.YAVG=") {
            if let Some(after) = line.split("YAVG=").nth(1) {
                if let Ok(val) = after.trim().parse::<f64>() {
                    luminances.push(val);
                }
            }
        }
    }

    let total_frames = luminances.len() as u64;
    if total_frames == 0 {
        return Err(
            "flicker: parsed 0 frame luminances — ffmpeg signalstats produced no YAVG data".into(),
        );
    }
    eprintln!("  parsed {} frame luminances", total_frames);

    // Detect flashes: opposing luminance transitions exceeding threshold
    // ITU-R BT.1702: a flash is a pair of opposing transitions >20 cd/m²
    // We approximate using YAVG units (0-255 scale, ~20 cd/m² ≈ 20 units)
    let flash_threshold = 20.0;
    let mut flash_frames: Vec<FlickerFrame> = Vec::new();
    let mut max_delta: f64 = 0.0;

    for i in 1..luminances.len() {
        let delta = (luminances[i] - luminances[i - 1]).abs();
        if delta > max_delta {
            max_delta = delta;
        }

        if delta > flash_threshold {
            let time_ms = ((i as f64 / fps) * 1000.0).round() as u64;
            flash_frames.push(FlickerFrame {
                frame: i as u64,
                time_ms,
                luminance_delta: (delta * 10.0).round() / 10.0,
            });
        }
    }

    // BT.1702: no more than 3 flashes per second
    let flash_count = flash_frames.len() as u64;

    // Find max flash frequency in any 1-second window
    let mut max_freq: f64 = 0.0;
    if flash_frames.len() > 1 {
        let window_ms: u64 = 1000;
        for i in 0..flash_frames.len() {
            let window_start = flash_frames[i].time_ms;
            let count_in_window = flash_frames[i..]
                .iter()
                .take_while(|f| f.time_ms < window_start + window_ms)
                .count() as f64;
            if count_in_window > max_freq {
                max_freq = count_in_window;
            }
        }
    }

    let bt1702_compliant = max_freq <= 3.0;

    let result = FlickerAnalysis {
        total_frames_analyzed: total_frames,
        max_luminance_delta: (max_delta * 10.0).round() / 10.0,
        flash_frames,
        flash_count,
        max_flash_frequency_hz: max_freq,
        bt1702_compliant,
    };

    eprintln!(
        "  max_delta={:.1} flashes={} max_freq={:.0}Hz compliant={}",
        max_delta, flash_count, max_freq, bt1702_compliant
    );

    let json = serde_json::to_string_pretty(&result).map_err(|e| format!("JSON serialize: {e}"))?;
    println!("{json}");
    Ok(())
}
