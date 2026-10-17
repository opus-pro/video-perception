use crate::commands::scenes::Scenes;
use serde::Serialize;
use std::fs;
use std::process::Command;

/// Per-scene frame-level signal statistics.
///
/// Pure measurement — no thresholds, no classification, no judgment.
/// Outputs statistical summaries of per-frame luminance, saturation,
/// and inter-frame difference for each scene. Product interpretation is done
/// by the consuming agent service.

#[derive(Serialize)]
pub struct SceneSignalStats {
    pub scene_idx: usize,
    pub frame_count: usize,

    // Luminance (YAVG) distribution across frames in this scene
    pub yavg_mean: f64,
    pub yavg_std: f64,
    pub yavg_min: f64,
    pub yavg_max: f64,

    // Luminance range (YMAX - YMIN per frame, measures contrast/variance)
    pub yrange_mean: f64,
    pub yrange_min: f64,

    // Saturation
    pub satavg_mean: f64,

    // Inter-frame difference (temporal change)
    pub frame_diff_mean: f64,
    pub frame_diff_max: f64,
    pub frame_diff_std: f64,

    // Distribution ratios (fraction of frames in each bucket)
    pub dark_frame_ratio: f64,   // yavg < 15
    pub bright_frame_ratio: f64, // yavg > 240
    pub low_variance_ratio: f64, // yrange < 20 (flat/pure color)
    pub static_frame_ratio: f64, // frame_diff < 0.5 (freeze)
}

/// Per-frame signal data for track output (L3 raw layer).
#[derive(Serialize)]
struct SignalFrame {
    f: u64,  // frame number
    t: u64,  // timestamp ms
    y: f64,  // YAVG (average luminance)
    yn: f64, // YMIN
    yx: f64, // YMAX
    sa: f64, // SATAVG
}

pub fn execute(
    video: &str,
    scenes_path: Option<&str>,
    track_dir: Option<&str>,
) -> Result<(), String> {
    let scenes: Option<Scenes> = scenes_path
        .map(|p| {
            let content = fs::read_to_string(p)
                .map_err(|e| format!("read: {e}"))
                .ok()?;
            serde_json::from_str(&content)
                .map_err(|e| format!("parse: {e}"))
                .ok()
        })
        .flatten();

    if let Some(ref s) = scenes {
        eprintln!("signalstats: {video} ({} scenes)", s.scene_count);
    } else {
        eprintln!("signalstats: {video} (track-only, no scenes)");
    }

    // Extract per-frame metrics via ffmpeg signalstats
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
        .map_err(|e| format!("ffmpeg: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout);

    // Parse per-frame metrics
    let mut frames: Vec<FrameData> = Vec::new();
    let mut current = FrameData::default();
    let mut frame_num: u64 = 0;

    for line in stdout.lines() {
        if line.starts_with("frame:") {
            if frame_num > 0 {
                frames.push(current);
                current = FrameData::default();
            }
            frame_num += 1;
        }
        if let Some(v) = extract(line, "lavfi.signalstats.YAVG=") {
            current.yavg = v;
        }
        if let Some(v) = extract(line, "lavfi.signalstats.YMIN=") {
            current.ymin = v;
        }
        if let Some(v) = extract(line, "lavfi.signalstats.YMAX=") {
            current.ymax = v;
        }
        if let Some(v) = extract(line, "lavfi.signalstats.SATAVG=") {
            current.satavg = v;
        }
    }
    if frame_num > 0 {
        frames.push(current);
    }

    eprintln!("  parsed {} frames", frames.len());

    // Get FPS for frame→time mapping
    let fps = get_fps(video)?;

    // Write raw per-frame track if track_dir is specified (L3 raw layer)
    if let Some(dir) = track_dir {
        fs::create_dir_all(dir).map_err(|e| format!("mkdir tracks: {e}"))?;
        let track: Vec<SignalFrame> = frames
            .iter()
            .enumerate()
            .map(|(i, f)| SignalFrame {
                f: i as u64,
                t: ((i as f64 / fps) * 1000.0).round() as u64,
                y: r2(f.yavg),
                yn: r2(f.ymin),
                yx: r2(f.ymax),
                sa: r2(f.satavg),
            })
            .collect();

        let jsonl: String = track
            .iter()
            .map(|f| serde_json::to_string(f).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let track_path = format!("{dir}/signalstats.track.jsonl");
        fs::write(&track_path, format!("{jsonl}\n")).map_err(|e| format!("write track: {e}"))?;
        eprintln!("  track: {} frames → {track_path}", track.len());
    }

    // Compute per-scene statistics (only when scenes are available)
    let scene_list = scenes.as_ref().map(|s| &s.scenes);
    if scene_list.is_none() {
        if track_dir.is_some() {
            // Track was written but no scene aggregates possible — report clearly
            eprintln!("  signalstats: track written, no scenes for aggregation");
        }
        return Err("No scenes provided — cannot compute per-scene signalstats".into());
    }

    let mut results: Vec<SceneSignalStats> = Vec::new();

    for scene in scene_list.unwrap() {
        let start_frame = ((scene.start_ms as f64 / 1000.0) * fps).round() as usize;
        let end_frame = ((scene.end_ms as f64 / 1000.0) * fps).round() as usize;

        let scene_frames: Vec<&FrameData> = frames
            .iter()
            .enumerate()
            .filter(|(i, _)| *i >= start_frame && *i < end_frame.min(frames.len()))
            .map(|(_, f)| f)
            .collect();

        let n = scene_frames.len();
        if n == 0 {
            continue;
        }

        // Luminance stats
        let yavgs: Vec<f64> = scene_frames.iter().map(|f| f.yavg).collect();
        let yavg_mean = mean(&yavgs);
        let yavg_std = std_dev(&yavgs, yavg_mean);

        // Luminance range per frame (proxy for per-frame variance)
        let yranges: Vec<f64> = scene_frames
            .iter()
            .map(|f| (f.ymax - f.ymin).max(0.0))
            .collect();

        // Inter-frame differences
        let mut diffs: Vec<f64> = Vec::new();
        for i in 1..scene_frames.len() {
            diffs.push((scene_frames[i].yavg - scene_frames[i - 1].yavg).abs());
        }
        if diffs.is_empty() {
            diffs.push(0.0);
        }

        // Distribution ratios
        let dark = scene_frames.iter().filter(|f| f.yavg < 15.0).count();
        let bright = scene_frames.iter().filter(|f| f.yavg > 240.0).count();
        let low_var = yranges.iter().filter(|&&r| r < 20.0).count();
        let static_f = diffs.iter().filter(|&&d| d < 0.5).count();

        let nf = n as f64;
        let nd = diffs.len() as f64;

        let stat = SceneSignalStats {
            scene_idx: scene.idx,
            frame_count: n,
            yavg_mean: r2(yavg_mean),
            yavg_std: r2(yavg_std),
            yavg_min: r2(yavgs.iter().copied().fold(f64::MAX, f64::min)),
            yavg_max: r2(yavgs.iter().copied().fold(f64::MIN, f64::max)),
            yrange_mean: r2(mean(&yranges)),
            yrange_min: r2(yranges.iter().copied().fold(f64::MAX, f64::min)),
            satavg_mean: r2(mean(
                &scene_frames.iter().map(|f| f.satavg).collect::<Vec<_>>(),
            )),
            frame_diff_mean: r2(mean(&diffs)),
            frame_diff_max: r2(diffs.iter().copied().fold(f64::MIN, f64::max)),
            frame_diff_std: r2(std_dev(&diffs, mean(&diffs))),
            dark_frame_ratio: r4(dark as f64 / nf),
            bright_frame_ratio: r4(bright as f64 / nf),
            low_variance_ratio: r4(low_var as f64 / nf),
            static_frame_ratio: r4(static_f as f64 / nd),
        };

        eprintln!(
            "  scene {}: {}frames yavg={:.1}±{:.1} dark={:.1}% static={:.1}%",
            scene.idx,
            n,
            stat.yavg_mean,
            stat.yavg_std,
            stat.dark_frame_ratio * 100.0,
            stat.static_frame_ratio * 100.0
        );

        results.push(stat);
    }

    let json = serde_json::to_string_pretty(&results).map_err(|e| format!("JSON: {e}"))?;
    println!("{json}");
    Ok(())
}

#[derive(Default, Clone)]
struct FrameData {
    yavg: f64,
    ymin: f64,
    ymax: f64,
    satavg: f64,
}

fn extract(line: &str, key: &str) -> Option<f64> {
    line.split(key).nth(1)?.trim().parse().ok()
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

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn r4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

fn get_fps(video: &str) -> Result<f64, String> {
    let output = Command::new("ffprobe")
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
        .map_err(|e| format!("ffprobe fps: {e}"))?;
    if !output.status.success() {
        return Err("ffprobe fps: non-zero exit".into());
    }
    parse_fps_output(&String::from_utf8_lossy(&output.stdout))
}

fn parse_fps_output(output: &str) -> Result<f64, String> {
    // Some ffprobe builds emit CSV rows with a trailing comma.
    let s = output.trim().trim_end_matches(',').trim().to_string();
    if let Some((n, d)) = s.split_once('/') {
        let num: f64 = n
            .parse()
            .map_err(|_| format!("ffprobe fps: bad numerator '{n}'"))?;
        let den: f64 = d
            .parse()
            .map_err(|_| format!("ffprobe fps: bad denominator '{d}'"))?;
        if den == 0.0 {
            return Err("ffprobe fps: zero denominator".into());
        }
        Ok(num / den)
    } else {
        s.parse::<f64>()
            .map_err(|_| format!("ffprobe fps: unparseable '{s}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_fps_output;

    #[test]
    fn accepts_ffprobe_csv_trailing_comma() {
        assert_eq!(parse_fps_output("30/1,\n").unwrap(), 30.0);
    }
}
