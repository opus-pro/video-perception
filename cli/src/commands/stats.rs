use crate::commands::scenes::Scenes;
use crate::util::run_magick;
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::process::Command;

#[derive(Serialize, Clone)]
pub struct ColorEntry {
    pub color: String,
    pub ratio: f64,
}

#[derive(Serialize)]
pub struct SceneStats {
    pub scene_idx: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub avg_brightness: f64,
    pub min_brightness: f64,
    pub max_brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub dominant_colors: Vec<String>,
    pub color_palette: Vec<ColorEntry>,
    pub keyframe: String,
}

pub fn execute(video: &str, scenes_path: &str, out_dir: &str) -> Result<(), String> {
    let scenes: Scenes = serde_json::from_str(
        &fs::read_to_string(scenes_path).map_err(|e| format!("Cannot read {scenes_path}: {e}"))?,
    )
    .map_err(|e| format!("Invalid scenes JSON: {e}"))?;

    fs::create_dir_all(out_dir).map_err(|e| format!("Cannot create {out_dir}: {e}"))?;

    eprintln!("stats: {} scenes (parallel)", scenes.scene_count);

    let video_str = video.to_string();
    let out_dir_str = out_dir.to_string();

    let mut results: Vec<SceneStats> = scenes
        .scenes
        .par_iter()
        .filter_map(|scene| {
            let mid_ms = (scene.start_ms + scene.end_ms) / 2;
            let mid_sec = format!("{:.3}", mid_ms as f64 / 1000.0);
            let keyframe_path = format!("{out_dir_str}/scene_{:02}_key.png", scene.idx);

            // Extract keyframe
            let out = Command::new("ffmpeg")
                .args([
                    "-y",
                    "-ss",
                    &mid_sec,
                    "-i",
                    video_str.as_str(),
                    "-vframes",
                    "1",
                    "-q:v",
                    "2",
                    &keyframe_path,
                ])
                .output()
                .ok()?;

            if !out.status.success() {
                eprintln!("  scene {}: keyframe extraction failed", scene.idx);
                return None;
            }

            // Brightness stats via magick (grayscale)
            let identify = run_magick(&[
                &keyframe_path,
                "-colorspace",
                "Gray",
                "-format",
                "%[fx:minima*255] %[fx:maxima*255] %[fx:mean*255]",
                "info:",
            ])
            .ok()?;

            let vals: Vec<f64> = identify
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            let (min_b, max_b, avg_b) = match vals.as_slice() {
                [a, b, c, ..] => (*a, *b, *c),
                _ => (0.0, 255.0, 128.0),
            };
            let contrast = if max_b > 0.0 {
                (max_b - min_b) / 255.0
            } else {
                0.0
            };

            // Saturation via HSL
            let sat_raw = run_magick(&[
                &keyframe_path,
                "-colorspace",
                "HSL",
                "-channel",
                "G",
                "-format",
                "%[fx:mean]",
                "info:",
            ])
            .ok()?;
            let saturation: f64 = sat_raw.trim().parse().unwrap_or(0.0);

            // Dominant colors with pixel counts (no -unique-colors, to preserve counts)
            let color_raw = run_magick(&[
                &keyframe_path,
                "-resize",
                "50x50!",
                "-colors",
                "5",
                "-format",
                "%c",
                "histogram:info:-",
            ])
            .ok()?;

            let mut color_entries: Vec<(String, u64)> = Vec::new();
            for line in color_raw.lines() {
                let trimmed = line.trim();
                let count: u64 = trimmed
                    .split(':')
                    .next()
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);

                if let Some(pos) = trimmed.find('#') {
                    if pos + 7 <= trimmed.len() {
                        let hex = format!("#{}", &trimmed[pos + 1..pos + 7].to_lowercase());
                        color_entries.push((hex, count));
                    }
                }
            }

            color_entries.sort_by(|a, b| b.1.cmp(&a.1));
            color_entries.truncate(5);

            let total_pixels: u64 = color_entries.iter().map(|(_, c)| c).sum();
            let color_palette: Vec<ColorEntry> = color_entries
                .iter()
                .map(|(color, count)| {
                    let ratio = if total_pixels > 0 {
                        (*count as f64 / total_pixels as f64 * 100.0).round() / 100.0
                    } else {
                        0.0
                    };
                    ColorEntry {
                        color: color.clone(),
                        ratio,
                    }
                })
                .collect();

            let dominant_colors: Vec<String> =
                color_palette.iter().map(|e| e.color.clone()).collect();

            let round = |v: f64, d: u32| {
                let m = 10f64.powi(d as i32);
                (v * m).round() / m
            };

            eprintln!(
                "  scene {}: brightness={:.0} contrast={:.2} sat={:.2} colors={}",
                scene.idx,
                avg_b,
                contrast,
                saturation,
                color_palette.len()
            );

            Some(SceneStats {
                scene_idx: scene.idx,
                start_ms: scene.start_ms,
                end_ms: scene.end_ms,
                avg_brightness: round(avg_b, 1),
                min_brightness: round(min_b, 1),
                max_brightness: round(max_b, 1),
                contrast: round(contrast, 3),
                saturation: round(saturation, 3),
                dominant_colors,
                color_palette,
                keyframe: keyframe_path,
            })
        })
        .collect();

    results.sort_by_key(|r| r.scene_idx);
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
    Ok(())
}

/// Fixed-interval color sampling — independent of scene boundaries.
/// Extracts frames at 0.5s intervals, computes dominant colors for each.
/// Outputs tracks/color.track.jsonl.
pub fn generate_color_track(video: &str, track_dir: &str) -> Result<(), String> {
    fs::create_dir_all(track_dir).map_err(|e| format!("mkdir: {e}"))?;

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
        .map_err(|e| format!("ffprobe: {e}"))?;
    let duration_sec: f64 = String::from_utf8_lossy(&dur_out.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0);
    if duration_sec <= 0.0 {
        return Err("Cannot determine video duration".into());
    }

    let interval = 0.5; // seconds
    let n_samples = (duration_sec / interval).ceil() as usize;
    eprintln!(
        "  color track: {} samples at {interval}s intervals",
        n_samples
    );

    // Extract all frames at once using fps filter (much faster than individual seeks)
    let tmp_dir = format!("{track_dir}/_color_frames");
    fs::create_dir_all(&tmp_dir).ok();

    let fps_filter = format!("fps={}", 1.0 / interval);
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            video,
            "-vf",
            &fps_filter,
            "-q:v",
            "4",
            &format!("{tmp_dir}/f_%04d.png"),
        ])
        .output()
        .map_err(|e| format!("ffmpeg frame extract: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "ffmpeg frame extract failed: {}",
            stderr.lines().last().unwrap_or("")
        ));
    }

    // Process each frame for dominant colors (parallel)
    let mut entries: Vec<(usize, String)> = (0..n_samples).into_par_iter().filter_map(|i| {
        let frame_path = format!("{tmp_dir}/f_{:04}.png", i + 1);
        if !std::path::Path::new(&frame_path).exists() { return None; }

        // Dominant colors (5 colors from 50x50 quantization)
        let color_raw = run_magick(&[
            &frame_path, "-resize", "50x50!", "-colors", "5",
            "-format", "%c", "histogram:info:-",
        ]).ok()?;

        let mut colors: Vec<(String, u64)> = Vec::new();
        for line in color_raw.lines() {
            let trimmed = line.trim();
            let count: u64 = trimmed.split(':').next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            if let Some(pos) = trimmed.find('#') {
                if pos + 7 <= trimmed.len() {
                    colors.push((format!("#{}", &trimmed[pos+1..pos+7].to_lowercase()), count));
                }
            }
        }
        colors.sort_by(|a, b| b.1.cmp(&a.1));
        colors.truncate(5);
        let total: u64 = colors.iter().map(|(_, c)| *c).sum();

        // Saturation
        let sat_raw = run_magick(&[
            &frame_path, "-colorspace", "HSL", "-channel", "G",
            "-format", "%[fx:mean]", "info:",
        ]).ok()?;
        let sat: f64 = sat_raw.trim().parse().unwrap_or(0.0);

        let t_ms = (i as f64 * interval * 1000.0).round() as u64;
        let entry = serde_json::json!({
            "t": t_ms,
            "colors": colors.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>(),
            "ratios": colors.iter().map(|(_, count)| {
                if total > 0 { (*count as f64 / total as f64 * 100.0).round() / 100.0 } else { 0.0 }
            }).collect::<Vec<f64>>(),
            "sat": (sat * 1000.0).round() / 1000.0,
        });

        Some((i, serde_json::to_string(&entry).unwrap()))
    }).collect();

    entries.sort_by_key(|(i, _)| *i);

    let jsonl = entries
        .iter()
        .map(|(_, s)| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let track_path = format!("{track_dir}/color.track.jsonl");
    fs::write(&track_path, format!("{jsonl}\n")).map_err(|e| format!("write color track: {e}"))?;
    eprintln!("  color track: {} samples → {track_path}", entries.len());

    // Cleanup temp frames
    fs::remove_dir_all(&tmp_dir).ok();

    Ok(())
}
