use crate::commands::scenes::Scenes;
use crate::util::run_magick;
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::process::Command;

#[derive(Serialize)]
pub struct BlurStats {
    pub scene_idx: usize,
    pub laplacian_variance: f64,
    pub fft_high_freq_ratio: f64,
    pub sharpness_class: String,
}

/// Classify sharpness from Laplacian variance.
/// Thresholds from CERTH blur dataset literature.
fn classify_sharpness(lap_var: f64) -> &'static str {
    if lap_var < 50.0 {
        "blurry"
    } else if lap_var < 200.0 {
        "soft"
    } else {
        "sharp"
    }
}

pub fn execute(video: &str, scenes_path: &str, keyframe_dir: &str) -> Result<(), String> {
    let scenes: Scenes = serde_json::from_str(
        &fs::read_to_string(scenes_path).map_err(|e| format!("Cannot read {scenes_path}: {e}"))?,
    )
    .map_err(|e| format!("Invalid scenes JSON: {e}"))?;

    eprintln!("blur: {} scenes (parallel)", scenes.scene_count);

    let video_str = video.to_string();
    let kf_dir = keyframe_dir.to_string();

    let mut results: Vec<BlurStats> = scenes
        .scenes
        .par_iter()
        .filter_map(|scene| {
            // Try to reuse existing keyframe, else extract one
            let keyframe_path = format!("{kf_dir}/scene_{:02}_key.png", scene.idx);
            if !std::path::Path::new(&keyframe_path).exists() {
                let mid_sec = format!("{:.3}", (scene.start_ms + scene.end_ms) as f64 / 2000.0);
                let out = Command::new("ffmpeg")
                    .args([
                        "-y",
                        "-ss",
                        &mid_sec,
                        "-i",
                        &video_str,
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
            }

            // Laplacian variance: convert to grayscale, apply Laplacian edge detection,
            // then measure the variance of the result.
            // High variance = sharp edges = in focus. Low variance = blurry.
            let lap_var = match run_magick(&[
                &keyframe_path,
                "-colorspace",
                "Gray",
                "-define",
                "convolve:scale=1",
                "-morphology",
                "Convolve",
                "Laplacian:0",
                "-format",
                "%[fx:standard_deviation^2*255*255]",
                "info:",
            ]) {
                Ok(s) => s
                    .parse::<f64>()
                    .map_err(|e| {
                        eprintln!("  scene {}: magick laplacian parse error: {e}", scene.idx);
                    })
                    .ok()?,
                Err(e) => {
                    eprintln!("  scene {}: magick laplacian failed: {e}", scene.idx);
                    return None;
                }
            };

            // FFT high-frequency ratio: resize to tiny (DCT approximation),
            // measure how much detail survives downscaling.
            // Sharp images lose more detail = higher ratio of original to downscaled variance.
            let orig_std = match run_magick(&[
                &keyframe_path,
                "-colorspace",
                "Gray",
                "-format",
                "%[fx:standard_deviation*255]",
                "info:",
            ]) {
                Ok(s) => s.parse::<f64>().unwrap_or(1.0),
                Err(e) => {
                    eprintln!("  scene {}: magick orig_std failed: {e}", scene.idx);
                    return None;
                }
            };

            let small_std = match run_magick(&[
                &keyframe_path,
                "-colorspace",
                "Gray",
                "-resize",
                "64x64!",
                "-resize",
                "256x256!", // upscale back — blur from downscale
                "-format",
                "%[fx:standard_deviation*255]",
                "info:",
            ]) {
                Ok(s) => s.parse::<f64>().unwrap_or(1.0),
                Err(e) => {
                    eprintln!("  scene {}: magick small_std failed: {e}", scene.idx);
                    return None;
                }
            };

            let fft_ratio = if orig_std > 0.01 {
                ((orig_std - small_std) / orig_std).max(0.0)
            } else {
                0.0
            };

            let class = classify_sharpness(lap_var).to_string();

            eprintln!(
                "  scene {}: lap_var={:.1} fft={:.3} → {}",
                scene.idx, lap_var, fft_ratio, class
            );

            Some(BlurStats {
                scene_idx: scene.idx,
                laplacian_variance: (lap_var * 10.0).round() / 10.0,
                fft_high_freq_ratio: (fft_ratio * 1000.0).round() / 1000.0,
                sharpness_class: class,
            })
        })
        .collect();

    results.sort_by_key(|s| s.scene_idx);

    let json =
        serde_json::to_string_pretty(&results).map_err(|e| format!("JSON serialize: {e}"))?;
    println!("{json}");
    Ok(())
}
