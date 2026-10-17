//! `track` sensor — per-scene motion intensity.
//!
//! Backed by `video-perception-motion-snn` (a LIF spiking neural network over event-rate
//! input). Replaces the prior magick-diff-based implementation which silently
//! produced all-zero motion_intensity on most inputs.
//!
//! Output schema is unchanged so the dashboard's `s.track.motion_intensity`
//! contract and any downstream consumers of `track.json` keep working.
//! `video-perception-motion-snn` adds extra `snn_*` fields per scene (additive).
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Serialize, Deserialize)]
pub struct MotionSegment {
    pub from_pct: u32,
    pub to_pct: u32,
    pub intensity: f64,
}

#[derive(Serialize, Deserialize)]
pub struct SceneMotion {
    pub scene_idx: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub motion_intensity: f64,
    pub motion_class: String,
    pub dominant_direction: String,
    pub segments: Vec<MotionSegment>,
    // SNN-specific fields (additive — older consumers ignore unknown keys).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snn_intensity_mean: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snn_intensity_max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snn_intensity_std: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snn_spike_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snn_active_ratio: Option<f64>,
}

pub fn execute(video: &str, scenes_path: &str, _out_dir: &str) -> Result<(), String> {
    eprintln!("track: dispatching to video-perception-motion-snn (scenes={scenes_path})");

    let output = Command::new("video-perception-motion-snn")
        .args([
            "--input", video,
            "--scenes", scenes_path,
            "--emit", "legacy",
        ])
        .output()
        .map_err(|e| format!(
            "Failed to invoke video-perception-motion-snn: {e}.\n\
             Install it: `uv tool install --python 3.11 tools/motion-snn` (or `make install-tools`)."
        ))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "video-perception-motion-snn failed (exit {:?}):\n{}",
            output.status.code(),
            stderr.trim()
        ));
    }

    // Forward tool stderr (per-frame progress) so users see what's happening.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim());
    }

    // Validate the JSON shape before forwarding to stdout.
    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("video-perception-motion-snn output not UTF-8: {e}"))?;
    let scenes: Vec<SceneMotion> = serde_json::from_str(&stdout).map_err(|e| {
        format!("video-perception-motion-snn returned invalid SceneMotion JSON: {e}")
    })?;

    println!(
        "{}",
        serde_json::to_string_pretty(&scenes).map_err(|e| e.to_string())?
    );
    Ok(())
}
