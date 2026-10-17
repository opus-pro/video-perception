use crate::commands::scenes::Scenes;
use crate::util::run_magick;
use rayon::prelude::*;
use serde::Serialize;
use std::fs;
use std::process::Command;

#[derive(Serialize)]
pub struct ComplexityStats {
    pub scene_idx: usize,
    pub edge_density: f64,
    pub shannon_entropy: f64,
    pub jpeg_compression_ratio: f64,
    pub complexity_class: String,
}

fn classify_complexity(edge_density: f64, entropy: f64) -> &'static str {
    let score = edge_density * 0.5 + (entropy / 8.0) * 0.5;
    if score < 0.25 {
        "minimal"
    } else if score < 0.45 {
        "low"
    } else if score < 0.65 {
        "moderate"
    } else {
        "high"
    }
}

pub fn execute(video: &str, scenes_path: &str, keyframe_dir: &str) -> Result<(), String> {
    let scenes: Scenes = serde_json::from_str(
        &fs::read_to_string(scenes_path).map_err(|e| format!("Cannot read {scenes_path}: {e}"))?,
    )
    .map_err(|e| format!("Invalid scenes JSON: {e}"))?;

    eprintln!("complexity: {} scenes (parallel)", scenes.scene_count);

    let video_str = video.to_string();
    let kf_dir = keyframe_dir.to_string();

    let mut results: Vec<ComplexityStats> = scenes
        .scenes
        .par_iter()
        .filter_map(|scene| {
            let keyframe_path = format!("{kf_dir}/scene_{:02}_key.png", scene.idx);
            if !std::path::Path::new(&keyframe_path).exists() {
                let mid_sec = format!("{:.3}", (scene.start_ms + scene.end_ms) as f64 / 2000.0);
                Command::new("ffmpeg")
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
            }

            // Edge density: Canny edges → ratio of edge pixels to total pixels
            let edge_density = run_magick(&[
                &keyframe_path,
                "-colorspace",
                "Gray",
                "-canny",
                "0x1+10%+30%",
                "-format",
                "%[fx:mean]",
                "info:",
            ])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

            // Shannon entropy of grayscale histogram (0-8 bits)
            let entropy = run_magick(&[
                &keyframe_path,
                "-colorspace",
                "Gray",
                "-format",
                "%[entropy]",
                "info:",
            ])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

            // JPEG compression ratio: original PNG size vs JPEG quality 85 size
            let png_size = fs::metadata(&keyframe_path).map(|m| m.len()).unwrap_or(1) as f64;
            let jpeg_path = format!("{kf_dir}/scene_{:02}_complexity.jpg", scene.idx);
            let jpeg_size = Command::new("magick")
                .args([&keyframe_path, "-quality", "85", &jpeg_path])
                .output()
                .ok()
                .and_then(|_| fs::metadata(&jpeg_path).ok())
                .map(|m| m.len() as f64)
                .unwrap_or(png_size);

            let compression_ratio = if png_size > 0.0 {
                jpeg_size / png_size
            } else {
                1.0
            };

            // Clean up temp jpeg
            fs::remove_file(&jpeg_path).ok();

            let class = classify_complexity(edge_density, entropy).to_string();
            eprintln!(
                "  scene {}: edges={:.3} entropy={:.2} jpeg_ratio={:.3} → {}",
                scene.idx, edge_density, entropy, compression_ratio, class
            );

            Some(ComplexityStats {
                scene_idx: scene.idx,
                edge_density: (edge_density * 1000.0).round() / 1000.0,
                shannon_entropy: (entropy * 100.0).round() / 100.0,
                jpeg_compression_ratio: (compression_ratio * 1000.0).round() / 1000.0,
                complexity_class: class,
            })
        })
        .collect();

    results.sort_by_key(|s| s.scene_idx);

    let json =
        serde_json::to_string_pretty(&results).map_err(|e| format!("JSON serialize: {e}"))?;
    println!("{json}");
    Ok(())
}
