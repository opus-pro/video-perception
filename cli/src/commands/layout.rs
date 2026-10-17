use crate::commands::scenes::Scenes;
use crate::util::run_magick;
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
struct Bars {
    top: u32,
    bottom: u32,
    left: u32,
    right: u32,
}

#[derive(Serialize)]
struct Region {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Serialize)]
struct SplitDiagnostics {
    axis: String,
    luma_delta: f64,
    color_distance: f64,
}

#[derive(Serialize)]
struct SceneLayout {
    scene_idx: usize,
    start_ms: u64,
    end_ms: u64,
    layout: String,
    bars: Bars,
    content_region: Region,
    #[serde(skip_serializing_if = "Option::is_none")]
    split_diagnostics: Option<SplitDiagnostics>,
}

pub fn execute(
    video: &str,
    scenes_path: &str,
    out_dir: &str,
    keyframe_dir: Option<&str>,
) -> Result<(), String> {
    let scenes: Scenes = serde_json::from_str(
        &fs::read_to_string(scenes_path).map_err(|e| format!("Cannot read {scenes_path}: {e}"))?,
    )
    .map_err(|e| format!("Invalid scenes JSON: {e}"))?;

    fs::create_dir_all(out_dir).map_err(|e| format!("Cannot create {out_dir}: {e}"))?;

    eprintln!("layout: {} scenes (parallel)", scenes.scene_count);

    // Process scenes in parallel using rayon (bounded by CPU cores)
    let mut results: Vec<SceneLayout> = scenes
        .scenes
        .par_iter()
        .filter_map(
            |scene| match process_scene(video, scene, out_dir, keyframe_dir) {
                Ok(Some(layout)) => Some(layout),
                Ok(None) => None,
                Err(e) => {
                    eprintln!("  scene {}: error: {e}", scene.idx);
                    None
                }
            },
        )
        .collect();

    // Sort by scene_idx for deterministic output
    results.sort_by_key(|r| r.scene_idx);

    println!("{}", serde_json::to_string_pretty(&results).unwrap());
    Ok(())
}

fn process_scene(
    video: &str,
    scene: &crate::commands::scenes::Scene,
    out_dir: &str,
    keyframe_dir: Option<&str>,
) -> Result<Option<SceneLayout>, String> {
    let mid_ms = (scene.start_ms + scene.end_ms) / 2;
    let mid_sec = format!("{:.3}", mid_ms as f64 / 1000.0);

    // Reuse keyframe from stats sensor if available
    let keyframe_path = if let Some(kdir) = keyframe_dir {
        let existing = format!("{kdir}/scene_{:02}_key.png", scene.idx);
        if Path::new(&existing).exists() {
            existing
        } else {
            extract_keyframe(video, &mid_sec, out_dir, scene.idx)?
        }
    } else {
        extract_keyframe(video, &mid_sec, out_dir, scene.idx)?
    };

    // Get image dimensions (1 magick call)
    let dim_raw = run_magick(&["identify", "-format", "%w %h", &keyframe_path])?;
    let dims: Vec<u32> = dim_raw
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    let (img_w, img_h) = match dims.as_slice() {
        [w, h, ..] => (*w, *h),
        _ => {
            eprintln!("  scene {}: could not read dimensions", scene.idx);
            return Ok(None);
        }
    };

    // Get full brightness profiles in 2 magick calls (replaces hundreds of per-row calls)
    let row_profile = brightness_profile(&keyframe_path, img_w, img_h, true)?;
    let col_profile = brightness_profile(&keyframe_path, img_w, img_h, false)?;

    let top_b = row_profile.first().copied().unwrap_or(128.0);
    let bottom_b = row_profile.last().copied().unwrap_or(128.0);
    let left_b = col_profile.first().copied().unwrap_or(128.0);
    let right_b = col_profile.last().copied().unwrap_or(128.0);

    let mut bars = Bars {
        top: 0,
        bottom: 0,
        left: 0,
        right: 0,
    };
    let mut layout = String::from("standard");
    let mut split_diagnostics: Option<SplitDiagnostics> = None;

    // Bar detection (pure Rust, no subprocess)
    if top_b < 15.0 && bottom_b < 15.0 {
        let max_scan = (img_h / 4).min(200) as usize;
        bars.top = count_dark_from_start(&row_profile, max_scan, 15.0);
        bars.bottom = count_dark_from_end(&row_profile, max_scan, 15.0);
        if bars.top > 0 || bars.bottom > 0 {
            layout = String::from("letterbox");
        }
    } else if left_b < 15.0 && right_b < 15.0 {
        let max_scan = (img_w / 4).min(200) as usize;
        bars.left = count_dark_from_start(&col_profile, max_scan, 15.0);
        bars.right = count_dark_from_end(&col_profile, max_scan, 15.0);
        if bars.left > 0 || bars.right > 0 {
            layout = String::from("pillarbox");
        }
    }

    // Split detection from profiles (no extra magick calls)
    if layout == "standard" {
        let mid_col = col_profile.len() / 2;
        let mid_row = row_profile.len() / 2;

        let left_mean = avg(&col_profile[..mid_col]);
        let right_mean = avg(&col_profile[mid_col..]);
        let top_mean = avg(&row_profile[..mid_row]);
        let bottom_mean = avg(&row_profile[mid_row..]);

        if (left_mean - right_mean).abs() > 40.0 {
            // Only make RGB calls when split is detected (2 magick calls)
            let (lr, lg, lb) = sample_region_rgb(&keyframe_path, "50%x100%+0+0", None)?;
            let (rr, rg, rb) = sample_region_rgb(&keyframe_path, "50%x100%+0+0", Some("East"))?;
            layout = String::from("split_v");
            split_diagnostics = Some(SplitDiagnostics {
                axis: String::from("vertical"),
                luma_delta: round2((left_mean - right_mean).abs()),
                color_distance: round2(chroma_distance((lr, lg, lb), (rr, rg, rb))),
            });
        } else if (top_mean - bottom_mean).abs() > 40.0 {
            let (tr, tg, tb) = sample_region_rgb(&keyframe_path, "100%x50%+0+0", None)?;
            let (br, bg, bb) = sample_region_rgb(&keyframe_path, "100%x50%+0+0", Some("South"))?;
            layout = String::from("split_h");
            split_diagnostics = Some(SplitDiagnostics {
                axis: String::from("horizontal"),
                luma_delta: round2((top_mean - bottom_mean).abs()),
                color_distance: round2(chroma_distance((tr, tg, tb), (br, bg, bb))),
            });
        }
    }

    let content_region = Region {
        x: bars.left,
        y: bars.top,
        w: img_w.saturating_sub(bars.left + bars.right),
        h: img_h.saturating_sub(bars.top + bars.bottom),
    };

    eprintln!(
        "  scene {}: {} (bars t={} b={} l={} r={})",
        scene.idx, layout, bars.top, bars.bottom, bars.left, bars.right
    );

    Ok(Some(SceneLayout {
        scene_idx: scene.idx,
        start_ms: scene.start_ms,
        end_ms: scene.end_ms,
        layout,
        bars,
        content_region,
        split_diagnostics,
    }))
}

fn extract_keyframe(
    video: &str,
    mid_sec: &str,
    out_dir: &str,
    idx: usize,
) -> Result<String, String> {
    let path = format!("{out_dir}/layout_{idx:02}_key.png");
    let out = Command::new("ffmpeg")
        .args([
            "-y", "-ss", mid_sec, "-i", video, "-vframes", "1", "-q:v", "2", &path,
        ])
        .output()
        .map_err(|e| format!("ffmpeg failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("keyframe extraction failed for scene {idx}"));
    }
    Ok(path)
}

/// Get brightness profile: resize to 1-pixel strip, parse all values in one shot.
/// rows=true: 1xH (one value per row), rows=false: Wx1 (one value per column).
fn brightness_profile(path: &str, w: u32, h: u32, rows: bool) -> Result<Vec<f64>, String> {
    let resize = if rows {
        format!("1x{h}!")
    } else {
        format!("{w}x1!")
    };
    let raw = run_magick(&[
        path,
        "-colorspace",
        "Gray",
        "-filter",
        "Box", // exact arithmetic mean, no Lanczos blurring
        "-resize",
        &resize,
        "-depth",
        "8",
        "txt:-",
    ])?;
    parse_txt_brightness(&raw)
}

/// Parse ImageMagick txt:- output into brightness values.
/// Format: "0,0: (N)  #XX  gray(V)\n" or "0,0: (N,N,N)  #XXXXXX  srgb(...)"
fn parse_txt_brightness(raw: &str) -> Result<Vec<f64>, String> {
    let mut values = Vec::new();
    for line in raw.lines() {
        if line.starts_with('#') {
            continue;
        }
        // Try gray(N) format first
        if let Some(pos) = line.find("gray(") {
            let rest = &line[pos + 5..];
            if let Some(end) = rest.find(')') {
                if let Ok(v) = rest[..end].parse::<f64>() {
                    values.push(v);
                    continue;
                }
            }
        }
        // Fallback: parse hex value (#XX for 8-bit gray)
        if let Some(pos) = line.find('#') {
            let hex = &line[pos + 1..];
            if hex.len() >= 2 {
                if let Ok(v) = u8::from_str_radix(&hex[..2], 16) {
                    values.push(v as f64);
                }
            }
        }
    }
    Ok(values)
}

fn count_dark_from_start(profile: &[f64], max_scan: usize, threshold: f64) -> u32 {
    profile
        .iter()
        .take(max_scan)
        .take_while(|&&v| v < threshold)
        .count() as u32
}

fn count_dark_from_end(profile: &[f64], max_scan: usize, threshold: f64) -> u32 {
    profile
        .iter()
        .rev()
        .take(max_scan)
        .take_while(|&&v| v < threshold)
        .count() as u32
}

fn avg(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn sample_region_rgb(
    path: &str,
    crop: &str,
    gravity: Option<&str>,
) -> Result<(f64, f64, f64), String> {
    let mut args = vec![path];
    if let Some(g) = gravity {
        args.push("-gravity");
        args.push(g);
    }
    args.push("-crop");
    args.push(crop);
    args.push("-format");
    args.push("%[fx:mean.r*255] %[fx:mean.g*255] %[fx:mean.b*255]");
    args.push("info:");
    let raw = run_magick(&args)?;
    let v: Vec<f64> = raw
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    match v.as_slice() {
        [r, g, b, ..] => Ok((*r, *g, *b)),
        _ => Err(format!("Could not parse RGB: {raw}")),
    }
}

fn chroma_distance(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let a_sum = (a.0 + a.1 + a.2).max(1.0);
    let b_sum = (b.0 + b.1 + b.2).max(1.0);
    let a_n = (a.0 / a_sum, a.1 / a_sum, a.2 / a_sum);
    let b_n = (b.0 / b_sum, b.1 / b_sum, b.2 / b_sum);
    let dr = (a_n.0 - b_n.0) * 255.0;
    let dg = (a_n.1 - b_n.1) * 255.0;
    let db = (a_n.2 - b_n.2) * 255.0;
    (dr * dr + dg * dg + db * db).sqrt()
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
