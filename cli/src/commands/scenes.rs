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
    /// Visual changes too weak to call a cut: a new slide with the same layout,
    /// or text appearing on an unchanged shot. Pixels alone cannot tell these
    /// apart, so they are reported for the caller to inspect, not counted.
    #[serde(default)]
    pub weak_changes: Vec<WeakChange>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WeakChange {
    pub ms: u64,
    pub score: f64,
}

struct FrameSignal {
    ms: u64,
    score: f64,
}

/// scdet scores run from 0 to 100. Peaks at or above this floor are cuts.
/// Below it a peak may be a low-contrast slide change or text appearing on an
/// unchanged shot, which pixels cannot tell apart, so it becomes a weak change.
const SCDET_MIN_SCORE: f64 = 5.0;

/// Peaks below this are encoder noise, such as keyframes in static footage.
const SCDET_WEAK_SCORE: f64 = 0.4;

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
    let (scdet_cuts, mut weak_changes) = if !signals.is_empty() {
        let (cuts, weak) = detect_scdet_peaks(&signals, fps);
        eprintln!(
            "  signal 2: {} scdet cuts, {} weak changes",
            cuts.len(),
            weak.len()
        );
        (cuts, weak)
    } else {
        (Vec::new(), Vec::new())
    };

    // ── Signal 3: black frame boundaries ────────────────────────
    let black_cuts = detect_black_boundaries(video, duration_ms, frame_ms)?;
    eprintln!("  signal 3: {} black-frame boundaries", black_cuts.len());

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

    // A weak change already confirmed by another signal is a cut, not a weak change.
    weak_changes.retain(|weak| {
        !filtered
            .iter()
            .any(|&cut| cut.abs_diff(weak.ms) < frame_ms * 2)
    });

    if filtered.is_empty() {
        return emit_single_scene(video, duration_ms, merge_ms, weak_changes);
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
        weak_changes,
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

/// Adaptive peak detection on scdet scores. Returns cuts and weak changes.
fn detect_scdet_peaks(signals: &[FrameSignal], fps: f64) -> (Vec<u64>, Vec<WeakChange>) {
    let n = signals.len();
    if n < 10 {
        return (Vec::new(), Vec::new());
    }

    let scores: Vec<f64> = signals.iter().map(|s| s.score).collect();
    let nms_window = (3.0 * fps) as usize;
    let roll_window = (5.0 * fps) as usize;
    let k = 5.0;
    let min_score = SCDET_MIN_SCORE;

    let thresholds = rolling_adaptive_threshold(&scores, roll_window, k);

    let mut cuts: Vec<u64> = Vec::new();
    let mut weak: Vec<WeakChange> = Vec::new();
    for i in 3..n.saturating_sub(3) {
        if scores[i] < SCDET_WEAK_SCORE {
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

        if scores[i] >= min_score {
            cuts.push(signals[i].ms);
        } else {
            weak.push(WeakChange {
                ms: signals[i].ms,
                score: (scores[i] * 1000.0).round() / 1000.0,
            });
        }
    }

    (cuts, weak)
}

/// Signal 3: entry and exit points of black segments, measured from pixel
/// luminance with FFmpeg's blackdetect. A low scdet score means "no change",
/// not "black", so it cannot be used here: static slides would read as black
/// and every encoder keyframe inside them would become a false cut.
fn detect_black_boundaries(
    video: &str,
    duration_ms: u64,
    frame_ms: u64,
) -> Result<Vec<u64>, String> {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostats",
            "-i",
            video,
            "-vf",
            "blackdetect=d=0.2:pix_th=0.10",
            "-an",
            "-f",
            "null",
            "-",
        ])
        .output()
        .map_err(|e| format!("ffmpeg blackdetect failed: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(black_boundaries(&stderr, duration_ms, frame_ms))
}

/// Parse blackdetect output into cut points, skipping a black segment's edge
/// when it touches the start or end of the video.
fn black_boundaries(blackdetect_log: &str, duration_ms: u64, frame_ms: u64) -> Vec<u64> {
    let edge = frame_ms * 5;
    let mut cuts = Vec::new();
    for line in blackdetect_log.lines() {
        let (Some(start), Some(end)) = (
            field_seconds(line, "black_start:"),
            field_seconds(line, "black_end:"),
        ) else {
            continue;
        };
        let (start_ms, end_ms) = (
            (start * 1000.0).round() as u64,
            (end * 1000.0).round() as u64,
        );
        if start_ms > edge {
            cuts.push(start_ms);
        }
        if end_ms + edge < duration_ms {
            cuts.push(end_ms);
        }
    }
    cuts
}

fn field_seconds(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
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

        // Static footage: almost every frame scores 0, so there is no local
        // distribution to adapt to. Leave the decision to the absolute floors.
        if local.len() < 5 {
            thresholds[i] = 0.0;
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

fn emit_single_scene(
    video: &str,
    duration_ms: u64,
    merge_ms: u64,
    weak_changes: Vec<WeakChange>,
) -> Result<(), String> {
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
        weak_changes,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_signals(score: f64, spike_at: usize, spike: f64) -> Vec<FrameSignal> {
        (0..300)
            .map(|i| FrameSignal {
                ms: i as u64 * 33,
                score: if i == spike_at { spike } else { score },
            })
            .collect()
    }

    #[test]
    fn text_change_on_a_static_slide_is_a_weak_change_not_a_cut() {
        let signals = flat_signals(0.02, 150, 0.5);
        let (cuts, weak) = detect_scdet_peaks(&signals, 30.0);
        assert!(cuts.is_empty());
        assert_eq!(
            weak,
            vec![WeakChange {
                ms: 150 * 33,
                score: 0.5
            }]
        );
    }

    #[test]
    fn changes_on_perfectly_static_footage_are_still_seen() {
        let cut = detect_scdet_peaks(&flat_signals(0.0, 150, 5.5), 30.0);
        assert_eq!(cut, (vec![150 * 33], Vec::new()));
        let weak = detect_scdet_peaks(&flat_signals(0.0, 150, 0.48), 30.0);
        assert_eq!(
            weak,
            (
                Vec::new(),
                vec![WeakChange {
                    ms: 150 * 33,
                    score: 0.48
                }]
            )
        );
    }

    #[test]
    fn encoder_noise_is_neither_a_cut_nor_a_weak_change() {
        let signals = flat_signals(0.02, 150, 0.2);
        assert_eq!(detect_scdet_peaks(&signals, 30.0), (Vec::new(), Vec::new()));
    }

    #[test]
    fn real_cut_still_detected_by_scdet() {
        let signals = flat_signals(0.02, 150, 30.0);
        assert_eq!(
            detect_scdet_peaks(&signals, 30.0),
            (vec![150 * 33], Vec::new())
        );
    }

    #[test]
    fn black_boundaries_come_from_blackdetect_and_skip_video_edges() {
        let log = "\
[blackdetect @ 0x1] black_start:0 black_end:0.5 black_duration:0.5
[blackdetect @ 0x1] black_start:70 black_end:73 black_duration:3
[blackdetect @ 0x1] black_start:91.8 black_end:92 black_duration:0.2";
        assert_eq!(
            black_boundaries(log, 92_000, 33),
            vec![500, 70_000, 73_000, 91_800]
        );
    }

    #[test]
    fn static_footage_without_black_yields_no_black_boundaries() {
        assert!(black_boundaries("frame=2760 fps=0.0 q=-0.0", 92_000, 33).is_empty());
    }
}
