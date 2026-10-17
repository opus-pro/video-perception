use crate::commands::scenes::Scenes;
use crate::util::extract_json_block;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::process::Command;

#[derive(Serialize)]
pub struct SceneAudio {
    pub scene_idx: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
}

pub fn execute(video: &str, scenes_path: &str) -> Result<(), String> {
    let scenes: Scenes = serde_json::from_str(
        &fs::read_to_string(scenes_path).map_err(|e| format!("Cannot read {scenes_path}: {e}"))?,
    )
    .map_err(|e| format!("Invalid scenes JSON: {e}"))?;

    eprintln!("scene-audio: {} scenes", scenes.scene_count);

    let mut result: Vec<SceneAudio> = Vec::new();
    for scene in &scenes.scenes {
        let start_s = format!("{:.3}", scene.start_ms as f64 / 1000.0);
        let duration_ms = scene.end_ms.saturating_sub(scene.start_ms);
        let dur_s = format!("{:.3}", duration_ms as f64 / 1000.0);

        let output = Command::new("ffmpeg")
            .args([
                "-ss",
                &start_s,
                "-t",
                &dur_s,
                "-i",
                video,
                "-vn",
                "-sn",
                "-af",
                "loudnorm=print_format=json",
                "-f",
                "null",
                "-",
            ])
            .output()
            .map_err(|e| format!("ffmpeg failed for scene {}: {e}", scene.idx))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let json_str = match extract_json_block(&stderr) {
            Some(v) => v,
            None => {
                eprintln!("  scene {}: skipped (no loudnorm payload)", scene.idx);
                continue;
            }
        };

        let loud: Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Invalid loudnorm JSON for scene {}: {e}", scene.idx))?;

        let parse_f64 = |key: &str| -> f64 {
            loud[key]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0)
        };

        let row = SceneAudio {
            scene_idx: scene.idx,
            start_ms: scene.start_ms,
            end_ms: scene.end_ms,
            input_i: parse_f64("input_i"),
            input_tp: parse_f64("input_tp"),
            input_lra: parse_f64("input_lra"),
            input_thresh: parse_f64("input_thresh"),
        };

        eprintln!(
            "  scene {}: I={:.1} LUFS TP={:.1} LRA={:.1}",
            row.scene_idx, row.input_i, row.input_tp, row.input_lra
        );
        result.push(row);
    }

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(())
}
