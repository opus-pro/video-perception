pub mod dashboard;
pub mod grep;
pub mod html;
pub mod join;
pub mod manifest;
pub mod report;
pub mod tracks;

use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Default, PartialEq)]
struct CapabilityStatusSummary {
    succeeded: usize,
    optional_failed: Vec<CapabilityIssue>,
    required_failed: Vec<CapabilityIssue>,
    incomplete: Vec<CapabilityIssue>,
}

#[derive(Debug, PartialEq)]
struct CapabilityIssue {
    id: String,
    status: String,
    error: Option<String>,
}

fn summarize_capability_status(manifest: &Value) -> CapabilityStatusSummary {
    let mut summary = CapabilityStatusSummary::default();

    for capability in manifest
        .get("capabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let status = capability
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if status == "succeeded" {
            summary.succeeded += 1;
            continue;
        }

        let issue = CapabilityIssue {
            id: capability
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            status: status.to_string(),
            error: capability
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        match (status, capability.get("required").and_then(Value::as_bool)) {
            ("failed", Some(true)) => summary.required_failed.push(issue),
            ("failed", _) => summary.optional_failed.push(issue),
            _ => summary.incomplete.push(issue),
        }
    }

    summary
}

fn print_capability_status(manifest: &Value) {
    let summary = summarize_capability_status(manifest);
    let mut counts = vec![
        format!("{} succeeded", summary.succeeded),
        format!("{} optional failed", summary.optional_failed.len()),
        format!("{} required failed", summary.required_failed.len()),
    ];
    if !summary.incomplete.is_empty() {
        counts.push(format!("{} incomplete", summary.incomplete.len()));
    }
    eprintln!("  capabilities: {}", counts.join(", "));

    for (label, issue) in summary
        .optional_failed
        .iter()
        .map(|issue| ("optional failed", issue))
        .chain(
            summary
                .required_failed
                .iter()
                .map(|issue| ("required failed", issue)),
        )
        .chain(
            summary
                .incomplete
                .iter()
                .map(|issue| (issue.status.as_str(), issue)),
        )
    {
        let diagnostic = issue.error.as_deref().unwrap_or("no diagnostic recorded");
        eprintln!("  {label}: {} — {diagnostic}", issue.id);
    }

    if !summary.optional_failed.is_empty() || !summary.incomplete.is_empty() {
        eprintln!(
            "  partial observation: usable evidence was saved; inspect manifest.json before relying on missing tracks"
        );
    }
}

/// Execute an explicit SensePlan and materialize its reusable Observation.
pub fn observe(
    video: &str,
    observation_dir: &str,
    sense_plan: &crate::plan::SensePlan,
) -> Result<(), String> {
    let ws = Path::new(observation_dir);
    let signals_dir = ws.join("signals");
    let signals_str = signals_dir.to_string_lossy().to_string();

    // Run requested capabilities into observation/signals/.
    crate::run::execute_plan(video, &signals_str, sense_plan, true)?;

    // Build the stable Observation views on top.
    generate(&signals_str, observation_dir)?;
    let manifest = load_json(ws, "manifest.json")?;
    print_capability_status(&manifest);
    Ok(())
}

/// Materialize an Observation from executor output.
fn generate(signals_dir: &str, observation_dir: &str) -> Result<(), String> {
    let signals = Path::new(signals_dir);
    let ws = Path::new(observation_dir);

    // Create the Observation structure.
    let signals_out = ws.join("signals");
    let assets_dir = ws.join("assets");
    for d in [&signals_out, &assets_dir] {
        fs::create_dir_all(d).map_err(|e| format!("mkdir {}: {e}", d.display()))?;
    }

    // Load sensor files (required + optional)
    let metadata = load_json(signals, "metadata.json")?;
    let scenes = load_json(signals, "scenes.json")?;
    let scene_stats = load_json(signals, "scene_stats.json").ok();
    let audio = load_json(signals, "audio.json").ok();
    let scene_audio = load_json(signals, "scene_audio.json").ok();
    let layout = load_json(signals, "layout.json").ok();
    let track = load_json(signals, "track.json").ok();
    let beats = load_json(signals, "beats.json").ok();
    let transcript = load_json(signals, "transcript.json").ok();
    let blur = load_json(signals, "blur.json").ok();
    let complexity = load_json(signals, "complexity.json").ok();
    let spectral = load_json(signals, "spectral.json").ok();
    let audio_class = load_json(signals, "audio_class.json").ok();
    // Prefer raw track over pre-aggregated scene data for signalstats.
    // Track-based aggregation is more accurate (computed here from raw frames)
    // and decouples measurement from scene detection.
    let signalstats_track = signals.join("tracks/signalstats.track.jsonl");
    let fps = metadata.get("fps").and_then(|v| v.as_f64()).unwrap_or(24.0);
    let signalstats = if signalstats_track.exists() {
        match tracks::aggregate_signalstats(&signalstats_track, &scenes, fps) {
            Ok(v) => {
                eprintln!(
                    "  signalstats: aggregated from raw track ({} frames)",
                    signalstats_track.metadata().map(|m| m.len()).unwrap_or(0) / 60
                );
                Some(v)
            }
            Err(e) => {
                eprintln!("  signalstats track error: {e}, falling back to pre-aggregated");
                load_json(signals, "signalstats.json").ok()
            }
        }
    } else {
        load_json(signals, "signalstats.json").ok()
    };
    let avsync = load_json(signals, "avsync.json").ok();
    let flicker = load_json(signals, "flicker.json").ok();
    let gibberish_text = load_json(signals, "gibberish_text.json").ok();
    let scene_description = load_json(signals, "scene_description.json").ok();
    let execution = load_json(signals, "execution.json")?;

    // Copy raw signal files verbatim into Observation/signals/.
    copy_signals(signals, &signals_out);

    // Copy tracks if present (files + subdirectories like keyframes/)
    let tracks_in = signals.join("tracks");
    if tracks_in.exists() {
        let tracks_out = ws.join("tracks");
        fs::create_dir_all(&tracks_out).ok();
        copy_tracks_recursive(&tracks_in, &tracks_out);
    }

    // Generate scenes.jsonl — lossless scene-centric join
    let joined = join::join_scenes(
        &scenes,
        scene_stats.as_ref(),
        scene_audio.as_ref(),
        layout.as_ref(),
        track.as_ref(),
        beats.as_ref(),
        transcript.as_ref(),
        blur.as_ref(),
        complexity.as_ref(),
        spectral.as_ref(),
        audio_class.as_ref(),
        signalstats.as_ref(),
        gibberish_text.as_ref(),
        scene_description.as_ref(),
        signals_dir,
    )?;
    let scenes_jsonl: String = joined
        .iter()
        .map(|s| serde_json::to_string(s).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(ws.join("scenes.jsonl"), format!("{scenes_jsonl}\n"))
        .map_err(|e| format!("write scenes.jsonl: {e}"))?;

    // Generate transcript.jsonl
    if let Some(ref t) = transcript {
        let transcript_jsonl = join::transcript_to_jsonl(t, &scenes)?;
        fs::write(ws.join("transcript.jsonl"), &transcript_jsonl)
            .map_err(|e| format!("write transcript.jsonl: {e}"))?;
    }

    // Copy keyframes into Observation/assets/.
    copy_assets(signals, &assets_dir);

    // Generate dashboard.json — L0 progressive layer with aggregate measurements.
    // When raw track exists, use it for accurate global stats (all frames, not scene averages)
    let track_globals = if signalstats_track.exists() {
        tracks::global_from_track(&signalstats_track).ok()
    } else {
        None
    };
    let framerate = load_json(signals, "framerate.json").ok();
    let dashboard_val = dashboard::generate(
        &metadata,
        &scenes,
        &joined,
        audio.as_ref(),
        avsync.as_ref(),
        flicker.as_ref(),
        track_globals.as_ref(),
        blur.as_ref(),
        complexity.as_ref(),
        framerate.as_ref(),
        beats.as_ref(),
        layout.as_ref(),
        scene_stats.as_ref(),
    );
    fs::write(
        ws.join("dashboard.json"),
        serde_json::to_string_pretty(&dashboard_val).unwrap(),
    )
    .map_err(|e| format!("write dashboard.json: {e}"))?;

    // Generate manifest
    let scene_count = scenes
        .get("scenes")
        .and_then(|s| s.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let manifest = manifest::generate(
        &metadata,
        signals,
        &execution,
        transcript.is_some(),
        joined.len(),
    );
    fs::write(
        ws.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .map_err(|e| format!("write manifest.json: {e}"))?;

    eprintln!(
        "✓ observation: {observation_dir}/ ({scene_count} scenes, {} joined records)",
        joined.len()
    );

    Ok(())
}

/// Generate human-readable artifacts for an existing Observation.
pub fn render(observation_dir: &str, include_media: bool) -> Result<(), String> {
    let observation = Path::new(observation_dir);
    let manifest_value = load_json(observation, "manifest.json")?;
    manifest::validate_contract(&manifest_value)?;
    let dashboard = load_json(observation, "dashboard.json")?;
    let scenes_jsonl = fs::read_to_string(observation.join("scenes.jsonl"))
        .map_err(|error| format!("read scenes.jsonl: {error}"))?;
    let metadata = load_json(&observation.join("signals"), "metadata.json")?;
    let scenes = load_json(&observation.join("signals"), "scenes.json")?;

    let video_path = metadata
        .get("video")
        .and_then(|value| value.as_str())
        .unwrap_or("video");
    let video_name = Path::new(video_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(video_path);
    let report_md = report::generate(&dashboard, video_name);
    fs::write(observation.join("report.md"), report_md)
        .map_err(|error| format!("write report.md: {error}"))?;

    let readme = manifest::generate_readme(&metadata, &scenes, &manifest_value);
    fs::write(observation.join("README.md"), readme)
        .map_err(|error| format!("write README.md: {error}"))?;

    let copied_media = if include_media {
        let source = Path::new(video_path);
        if !source.is_file() {
            return Err(format!(
                "cannot include media because source asset is unavailable: {}",
                source.display()
            ));
        }
        let assets = observation.join("assets");
        fs::create_dir_all(&assets).map_err(|error| format!("create assets directory: {error}"))?;
        let destination = assets.join(source.file_name().unwrap_or_default());
        fs::copy(source, &destination).map_err(|error| format!("copy source media: {error}"))?;
        Some(destination)
    } else {
        None
    };

    html::generate(observation, &manifest_value, &scenes_jsonl)?;
    eprintln!("✓ rendered observation: {observation_dir}/");
    eprintln!("  viewer: {}", observation.join("index.html").display());
    if let Some(media) = copied_media {
        eprintln!("  media: copied to {}", media.display());
    } else {
        eprintln!(
            "  media: not copied (privacy default); re-run with --include-media to enable playback"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarizes_success_optional_failure_and_incomplete_capabilities() {
        let summary = summarize_capability_status(&json!({
            "capabilities": [
                {"id": "probe", "required": true, "status": "succeeded"},
                {"id": "scenes", "required": true, "status": "succeeded"},
                {
                    "id": "transcript",
                    "required": false,
                    "status": "failed",
                    "error": "provider unavailable"
                },
                {
                    "id": "scene_description",
                    "required": false,
                    "status": "skipped",
                    "error": "dependency unavailable"
                }
            ]
        }));

        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.optional_failed.len(), 1);
        assert_eq!(summary.optional_failed[0].id, "transcript");
        assert_eq!(summary.required_failed.len(), 0);
        assert_eq!(summary.incomplete.len(), 1);
        assert_eq!(summary.incomplete[0].status, "skipped");
    }
}

fn load_json(dir: &Path, name: &str) -> Result<Value, String> {
    let path = dir.join(name);
    let content = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Copy raw capability JSON files into Observation/signals/.
/// Derives the file list from the registry so new sensors are auto-included.
fn copy_tracks_recursive(src: &Path, dest: &Path) {
    if let Ok(entries) = fs::read_dir(src) {
        for entry in entries.flatten() {
            let file_type = entry
                .file_type()
                .unwrap_or_else(|_| entry.metadata().unwrap().file_type());
            let dst = dest.join(entry.file_name());
            if file_type.is_dir() {
                fs::create_dir_all(&dst).ok();
                copy_tracks_recursive(&entry.path(), &dst);
            } else {
                fs::copy(entry.path(), &dst).ok();
            }
        }
    }
}

fn copy_signals(signals: &Path, signals_out: &Path) {
    // Skip if source and destination are the same directory (observe writes
    // capabilities directly into Observation/signals/, then materializes views which
    // would copy each file to itself — on macOS fs::copy truncates to 0 bytes).
    if let (Ok(a), Ok(b)) = (fs::canonicalize(signals), fs::canonicalize(signals_out)) {
        if a == b {
            return;
        }
    }
    for sensor in crate::registry::REGISTRY {
        if let Some(file) = sensor.output_file {
            let src = signals.join(file);
            if src.exists() {
                fs::copy(&src, signals_out.join(file)).ok();
            }
        }
    }
    // Always copy the combined deck
    let deck = signals.join("deck.json");
    if deck.exists() {
        fs::copy(&deck, signals_out.join("deck.json")).ok();
    }
}

fn copy_assets(signals: &Path, assets_dir: &Path) {
    let keyframes_dir = assets_dir.join("keyframes");
    fs::create_dir_all(&keyframes_dir).ok();

    // Copy keyframes from stats/
    if let Ok(entries) = fs::read_dir(signals.join("stats")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "png").unwrap_or(false) {
                let dest = keyframes_dir.join(entry.file_name());
                fs::copy(&path, &dest).ok();
            }
        }
    }
}
