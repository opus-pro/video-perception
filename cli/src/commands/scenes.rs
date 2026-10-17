use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Serialize, Deserialize, Clone)]
pub struct Scene {
    pub idx: usize,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Scenes {
    pub video: String,
    pub duration_ms: u64,
    pub scene_count: usize,
    pub merge_threshold_ms: u64,
    pub scenes: Vec<Scene>,
}

struct FrameSignal {
    ms: u64,
    score: f64,
}

pub fn execute(video: &str, threshold: f64, merge_ms: u64, do_merge: bool) -> Result<(), String> {
    let duration_ms = get_duration_ms(video)?;
    let fps = get_fps(video).unwrap_or(24.0);
    let frame_ms = (1000.0 / fps).round() as u64;

    eprintln!("scenes: {video} ({duration_ms}ms, threshold={threshold}, fps={fps:.1})");

    // ── Signal 1: select(scene) — histogram-based hard cuts ─────
    // This is the proven method for live-action content.
    let detect_threshold = threshold.min(0.15);
    let mut hard_cuts = detect_hard_cuts(video, detect_threshold, duration_ms, frame_ms)?;
    eprintln!(
        "  signal 1: {} hard cuts (select threshold={detect_threshold})",
        hard_cuts.len()
    );

    // ── Signal 2: scdet — per-frame scoring for soft/animated cuts ─
    // Catches same-palette transitions that select(scene) misses.
    let signals = extract_scdet_signals(video, duration_ms)?;
    let scdet_cuts = if !signals.is_empty() {
        let cuts = detect_scdet_peaks(&signals, fps);
        eprintln!("  signal 2: {} scdet adaptive peaks", cuts.len());
        cuts
    } else {
        Vec::new()
    };

    // ── Signal 3: black frame boundaries ────────────────────────
    let black_cuts = if !signals.is_empty() {
        let cuts = detect_black_regions(&signals, frame_ms);
        eprintln!("  signal 3: {} black-frame boundaries", cuts.len());
        cuts
    } else {
        Vec::new()
    };

    // ── Merge all signals ───────────────────────────────────────
    let mut all_cuts: Vec<u64> = Vec::new();
    all_cuts.append(&mut hard_cuts);
    all_cuts.extend_from_slice(&scdet_cuts);
    all_cuts.extend_from_slice(&black_cuts);
    all_cuts.sort();
    all_cuts.dedup();

    // Deduplicate within 2 frames
    let mut merged_cuts: Vec<u64> = Vec::new();
    for &cut in &all_cuts {
        if let Some(&prev) = merged_cuts.last() {
            if cut - prev < frame_ms * 2 {
                continue;
            }
        }
        merged_cuts.push(cut);
    }

    // Flash filter: remove cuts creating scenes < 3 frames on both sides
    let mut filtered: Vec<u64> = Vec::new();
    for i in 0..merged_cuts.len() {
        let ms = merged_cuts[i];
        let prev = if let Some(&p) = filtered.last() { p } else { 0 };
        let next = if i + 1 < merged_cuts.len() {
            merged_cuts[i + 1]
        } else {
            duration_ms
        };
        if ms - prev < 200 && next - ms < 200 {
            continue;
        }
        filtered.push(ms);
    }

    eprintln!(
        "  combined: {} cuts after merging + filtering",
        filtered.len()
    );

    if filtered.is_empty() {
        return emit_single_scene(video, duration_ms, merge_ms);
    }

    // ── Build scene list ────────────────────────────────────────
    let mut scenes: Vec<Scene> = Vec::new();
    let mut prev = 0u64;
    for &cut in &filtered {
        if cut > prev && cut < duration_ms {
            scenes.push(Scene {
                idx: scenes.len(),
                start_ms: prev,
                end_ms: cut,
            });
            prev = cut;
        }
    }
    if prev < duration_ms {
        scenes.push(Scene {
            idx: scenes.len(),
            start_ms: prev,
            end_ms: duration_ms,
        });
    }

    scenes.retain(|s| s.start_ms < s.end_ms);
    if scenes.is_empty() {
        scenes.push(Scene {
            idx: 0,
            start_ms: 0,
            end_ms: duration_ms,
        });
    }

    scenes[0].start_ms = 0;
    scenes.last_mut().unwrap().end_ms = duration_ms;
    for i in 0..scenes.len() - 1 {
        let end = scenes[i].end_ms;
        scenes[i + 1].start_ms = end;
    }

    // Short scene merge (optional)
    if do_merge && scenes.len() > 1 {
        let mut merged: Vec<Scene> = vec![scenes[0].clone()];
        for s in &scenes[1..] {
            if s.end_ms - s.start_ms < merge_ms {
                merged.last_mut().unwrap().end_ms = s.end_ms;
            } else {
                merged.push(s.clone());
            }
        }
        if merged.len() > 1 && merged[0].end_ms - merged[0].start_ms < merge_ms {
            merged[1].start_ms = merged[0].start_ms;
            merged.remove(0);
        }
        scenes = merged;
    }

    for (i, s) in scenes.iter_mut().enumerate() {
        s.idx = i;
    }

    eprintln!("  {} scenes detected", scenes.len());

    let result = Scenes {
        video: video.to_string(),
        duration_ms,
        scene_count: scenes.len(),
        merge_threshold_ms: merge_ms,
        scenes,
    };
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(())
}

/// Signal 1: Hard cut detection via select(scene) + showinfo.
/// Proven for live-action. Two-pass: recall-first candidates, then artifact filtering.
fn detect_hard_cuts(
    video: &str,
    threshold: f64,
    duration_ms: u64,
    frame_ms: u64,
) -> Result<Vec<u64>, String> {
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            video,
            "-vf",
            &format!("select='gt(scene,{threshold})',showinfo"),
            "-vsync",
            "vfr",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg select failed: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut candidates: Vec<(u64, f64)> = Vec::new();

    for line in stderr.lines() {
        if let Some(pts_pos) = line.find("pts_time:") {
            let rest = &line[pts_pos + 9..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            if let Ok(t) = rest[..end].parse::<f64>() {
                let ms = (t * 1000.0).round() as u64;
                if ms > 0 && ms < duration_ms {
                    let score = if let Some(score_pos) = line.find("scene_score=") {
                        let s = &line[score_pos + 12..];
                        let se = s
                            .find(|c: char| !c.is_ascii_digit() && c != '.')
                            .unwrap_or(s.len());
                        s[..se].parse::<f64>().unwrap_or(0.0)
                    } else {
                        threshold
                    };
                    if !candidates.iter().any(|(m, _)| *m == ms) {
                        candidates.push((ms, score));
                    }
                }
            }
        }
    }
    candidates.sort_by_key(|(ms, _)| *ms);

    // Artifact filtering
    let mut filtered: Vec<u64> = Vec::new();
    for i in 0..candidates.len() {
        let (ms, _score) = candidates[i];

        // Micro-cut removal
        if let Some(&prev_ms) = filtered.last() {
            if ms - prev_ms < frame_ms * 2 {
                let prev_score = candidates
                    .iter()
                    .find(|(m, _)| *m == prev_ms)
                    .map(|(_, s)| *s)
                    .unwrap_or(0.0);
                if _score > prev_score {
                    filtered.pop();
                    filtered.push(ms);
                }
                continue;
            }
        }

        // Flash detection
        if i + 1 < candidates.len() {
            let next_ms = candidates[i + 1].0;
            if next_ms - ms < frame_ms * 3 {
                let gap_before = if let Some(&prev_ms) = filtered.last() {
                    ms - prev_ms
                } else {
                    ms
                };
                if gap_before < 200 {
                    continue;
                }
            }
        }

        filtered.push(ms);
    }

    Ok(filtered)
}

/// Signal 2: Per-frame scdet scoring with adaptive peak detection.
/// Catches same-palette animated transitions that select(scene) misses.
fn extract_scdet_signals(video: &str, duration_ms: u64) -> Result<Vec<FrameSignal>, String> {
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            video,
            "-vf",
            "scale=320:-1,scdet=t=0",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg scdet failed: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut signals: Vec<FrameSignal> = Vec::new();

    for line in stderr.lines() {
        if let Some(score_pos) = line.find("scd.score:") {
            let rest = line[score_pos + 10..].trim_start();
            let end = rest.find(',').unwrap_or(rest.len());
            let score: f64 = rest[..end].trim().parse().unwrap_or(0.0);

            if let Some(time_pos) = line.find("scd.time:") {
                let rest = line[time_pos + 9..].trim_start();
                let end = rest
                    .find(|c: char| !c.is_ascii_digit() && c != '.')
                    .unwrap_or(rest.len());
                if let Ok(t) = rest[..end].parse::<f64>() {
                    let ms = (t * 1000.0).round() as u64;
                    if ms < duration_ms {
                        signals.push(FrameSignal { ms, score });
                    }
                }
            }
        }
    }

    signals.sort_by_key(|s| s.ms);
    Ok(signals)
}

/// Adaptive peak detection on scdet scores.
fn detect_scdet_peaks(signals: &[FrameSignal], fps: f64) -> Vec<u64> {
    let n = signals.len();
    if n < 10 {
        return Vec::new();
    }

    let scores: Vec<f64> = signals.iter().map(|s| s.score).collect();
    let nms_window = (3.0 * fps) as usize;
    let roll_window = (5.0 * fps) as usize;
    let k = 5.0;
    let min_score = 0.3;

    let thresholds = rolling_adaptive_threshold(&scores, roll_window, k);

    let mut cuts: Vec<u64> = Vec::new();
    for i in 3..n.saturating_sub(3) {
        if scores[i] < min_score {
            continue;
        }

        let lo = i.saturating_sub(nms_window);
        let hi = (i + nms_window + 1).min(n);
        let local_max = scores[lo..hi]
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        if (scores[i] - local_max).abs() > f64::EPSILON {
            continue;
        }
        if scores[i] <= thresholds[i] {
            continue;
        }

        cuts.push(signals[i].ms);
    }

    cuts
}

/// Detect black frame regions and return entry/exit points.
fn detect_black_regions(signals: &[FrameSignal], frame_ms: u64) -> Vec<u64> {
    let black_thresh = 0.01;
    let min_black_frames = 6;

    let mut cuts: Vec<u64> = Vec::new();
    let mut run_start: Option<usize> = None;

    for i in 0..signals.len() {
        let is_black = signals[i].score < black_thresh;
        match (is_black, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(start)) => {
                if i - start >= min_black_frames {
                    let entry_ms = signals[start].ms;
                    let exit_ms = signals[i].ms;
                    if entry_ms > frame_ms * 5 {
                        cuts.push(entry_ms);
                    }
                    if i < signals.len() - 5 {
                        cuts.push(exit_ms);
                    }
                }
                run_start = None;
            }
            _ => {}
        }
    }
    cuts
}

fn rolling_adaptive_threshold(scores: &[f64], window: usize, k: f64) -> Vec<f64> {
    let n = scores.len();
    let mut thresholds = vec![f64::MAX; n];

    for i in 0..n {
        let lo = i.saturating_sub(window);
        let hi = (i + window + 1).min(n);

        let mut local: Vec<f64> = scores[lo..hi]
            .iter()
            .copied()
            .filter(|&s| s > 0.01)
            .collect();

        if local.len() < 5 {
            thresholds[i] = f64::MAX;
            continue;
        }

        local.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = local[local.len() / 2];

        let mut deviations: Vec<f64> = local.iter().map(|&x| (x - median).abs()).collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mad = deviations[deviations.len() / 2].max(0.05);

        thresholds[i] = median + k * mad;
    }

    thresholds
}

fn emit_single_scene(video: &str, duration_ms: u64, merge_ms: u64) -> Result<(), String> {
    eprintln!("  no cuts detected — single scene");
    let result = Scenes {
        video: video.to_string(),
        duration_ms,
        scene_count: 1,
        merge_threshold_ms: merge_ms,
        scenes: vec![Scene {
            idx: 0,
            start_ms: 0,
            end_ms: duration_ms,
        }],
    };
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(())
}

fn get_duration_ms(video: &str) -> Result<u64, String> {
    let output = Command::new("ffprobe")
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

    let s = String::from_utf8_lossy(&output.stdout);
    let secs: f64 = s.trim().parse().map_err(|_| "Could not parse duration")?;
    Ok((secs * 1000.0).round() as u64)
}

fn get_fps(video: &str) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "stream=r_frame_rate",
            "-select_streams",
            "v:0",
            "-of",
            "csv=p=0",
            video,
        ])
        .output()
        .ok()?;

    let s = String::from_utf8_lossy(&output.stdout);
    let s = s.trim();
    if let Some(pos) = s.find('/') {
        let num: f64 = s[..pos].parse().ok()?;
        let den: f64 = s[pos + 1..].parse().ok()?;
        if den > 0.0 { Some(num / den) } else { None }
    } else {
        s.parse().ok()
    }
}
