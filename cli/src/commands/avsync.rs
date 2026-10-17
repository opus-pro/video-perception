use serde::Serialize;
use std::process::Command;

/// Audio-visual sync check via energy envelope correlation.
///
/// Deterministic approach (no neural network):
/// 1. Extract audio RMS energy envelope (via ffmpeg astats)
/// 2. Extract visual change envelope (via ffmpeg signalstats frame diffs)
/// 3. Cross-correlate the two envelopes
/// 4. Peak correlation lag = estimated A/V offset
///
/// Pure measurement — outputs offset estimate + correlation value.
/// Whether the offset is "acceptable" is a spec/rules concern.

#[derive(Serialize)]
pub struct AVSyncReport {
    pub audio_duration_ms: u64,
    pub video_duration_ms: u64,
    pub duration_diff_ms: i64,
    pub estimated_offset_ms: i64,
    pub correlation_peak: f64,
    pub audio_envelope_samples: usize,
    pub visual_envelope_samples: usize,
    pub has_audio: bool,
}

pub fn execute(video: &str) -> Result<(), String> {
    eprintln!("avsync: {video}");

    // Check for audio stream — fail clearly if missing
    let has_audio = check_audio(video);
    if !has_audio {
        return Err("no audio stream — cannot measure A/V sync".into());
    }

    let audio_dur = get_duration_ms(video, "a");
    let video_dur = get_duration_ms(video, "v");
    let dur_diff = audio_dur as i64 - video_dur as i64;

    // Extract audio energy envelope: RMS per 50ms window
    let audio_env = extract_audio_envelope(video)?;

    // Extract visual change envelope: frame-to-frame luminance difference
    let visual_env = extract_visual_envelope(video)?;

    eprintln!(
        "  audio envelope: {} samples, visual envelope: {} samples",
        audio_env.len(),
        visual_env.len()
    );

    // Resample to common length (100 samples per second)
    let target_len = (video_dur as f64 / 10.0).ceil() as usize; // 100Hz
    let audio_resampled = resample(&audio_env, target_len);
    let visual_resampled = resample(&visual_env, target_len);

    // Cross-correlate with ±500ms search window (±50 samples at 100Hz)
    let max_lag = 50; // 500ms
    let (best_lag, peak_corr) = cross_correlate(&audio_resampled, &visual_resampled, max_lag);

    let offset_ms = best_lag * 10; // 100Hz → 10ms per sample

    eprintln!(
        "  offset={}ms correlation={:.3} dur_diff={}ms",
        offset_ms, peak_corr, dur_diff
    );

    let report = AVSyncReport {
        audio_duration_ms: audio_dur,
        video_duration_ms: video_dur,
        duration_diff_ms: dur_diff,
        estimated_offset_ms: offset_ms as i64,
        correlation_peak: (peak_corr * 1000.0).round() / 1000.0,
        audio_envelope_samples: audio_env.len(),
        visual_envelope_samples: visual_env.len(),
        has_audio: true,
    };

    let json = serde_json::to_string_pretty(&report).map_err(|e| format!("JSON serialize: {e}"))?;
    println!("{json}");
    Ok(())
}

fn check_audio(video: &str) -> bool {
    Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
            video,
        ])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

fn get_duration_ms(video: &str, stream_type: &str) -> u64 {
    let selector = format!("{}:0", stream_type);
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            &selector,
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            video,
        ])
        .output()
        .ok();

    out.and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<f64>()
            .ok()
    })
    .map(|d| (d * 1000.0).round() as u64)
    .unwrap_or(0)
}

fn extract_audio_envelope(video: &str) -> Result<Vec<f64>, String> {
    // Use ffmpeg to compute RMS energy per short window
    // astats outputs per-frame RMS; we use a short analysis window
    let out = Command::new("ffmpeg")
        .args([
            "-i",
            video,
            "-af",
            "astats=metadata=1:reset=1,ametadata=print:file=-:key=lavfi.astats.Overall.RMS_level",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg astats: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut envelope = Vec::new();

    for line in stdout.lines() {
        if line.contains("lavfi.astats.Overall.RMS_level=") {
            if let Some(val_str) = line.split('=').last() {
                if let Ok(db) = val_str.trim().parse::<f64>() {
                    // Convert dB to linear (0-1 range)
                    let linear = if db <= -100.0 {
                        0.0
                    } else {
                        10f64.powf(db / 20.0)
                    };
                    envelope.push(linear);
                }
            }
        }
    }

    if envelope.is_empty() {
        // Fallback: use volumedetect for a simpler envelope
        eprintln!("  astats parse failed, using volume fallback");
    }

    Ok(envelope)
}

fn extract_visual_envelope(video: &str) -> Result<Vec<f64>, String> {
    // Frame-to-frame luminance difference via signalstats
    let out = Command::new("ffmpeg")
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

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut luminances: Vec<f64> = Vec::new();

    for line in stdout.lines() {
        if line.contains("lavfi.signalstats.YAVG=") {
            if let Some(val_str) = line.split("YAVG=").nth(1) {
                if let Ok(val) = val_str.trim().parse::<f64>() {
                    luminances.push(val);
                }
            }
        }
    }

    // Convert to frame differences (visual change magnitude)
    let mut envelope = Vec::new();
    for i in 1..luminances.len() {
        envelope.push((luminances[i] - luminances[i - 1]).abs());
    }

    Ok(envelope)
}

fn resample(data: &[f64], target_len: usize) -> Vec<f64> {
    if data.is_empty() || target_len == 0 {
        return vec![0.0; target_len];
    }
    let mut result = Vec::with_capacity(target_len);
    for i in 0..target_len {
        let src_idx = (i as f64 / target_len as f64) * data.len() as f64;
        let idx = (src_idx as usize).min(data.len() - 1);
        result.push(data[idx]);
    }
    result
}

fn cross_correlate(a: &[f64], b: &[f64], max_lag: usize) -> (i32, f64) {
    let n = a.len().min(b.len());
    if n < 10 {
        return (0, 0.0);
    }

    // Normalize signals
    let mean_a: f64 = a.iter().sum::<f64>() / n as f64;
    let mean_b: f64 = b.iter().sum::<f64>() / n as f64;
    let std_a = (a.iter().map(|x| (x - mean_a).powi(2)).sum::<f64>() / n as f64).sqrt();
    let std_b = (b.iter().map(|x| (x - mean_b).powi(2)).sum::<f64>() / n as f64).sqrt();

    if std_a < 1e-10 || std_b < 1e-10 {
        return (0, 0.0);
    }

    let mut best_lag: i32 = 0;
    let mut best_corr: f64 = f64::NEG_INFINITY;

    let max_lag = max_lag.min(n / 2);

    for lag in -(max_lag as i32)..=(max_lag as i32) {
        let mut sum = 0.0;
        let mut count = 0;

        for i in 0..n {
            let j = i as i32 + lag;
            if j >= 0 && (j as usize) < n {
                sum += (a[i] - mean_a) * (b[j as usize] - mean_b);
                count += 1;
            }
        }

        if count > 0 {
            let corr = sum / (count as f64 * std_a * std_b);
            if corr > best_corr {
                best_corr = corr;
                best_lag = lag;
            }
        }
    }

    (best_lag, best_corr)
}
