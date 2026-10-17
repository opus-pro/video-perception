use crate::registry::REGISTRY;
use serde_json::{Value, json};
use std::path::Path;

/// Build the stable index for one sensor observation.
///
/// The manifest describes what was measured and where each capability wrote
/// its raw output. It deliberately contains no consumer profile, rule, or
/// verdict. Those belong to the agent service consuming the observation.
pub fn generate(
    metadata: &Value,
    signals_dir: &Path,
    execution: &Value,
    has_transcript: bool,
    scene_count: usize,
) -> Value {
    let video = metadata
        .get("video")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let duration_ms = metadata
        .get("duration_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let width = metadata.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
    let height = metadata.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
    let fps = metadata.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let capabilities: Vec<Value> = execution
        .get("capabilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|execution_capability| {
            let id = execution_capability.get("id")?.as_str()?;
            let capability = REGISTRY.iter().find(|definition| definition.id == id)?;
            let output_file = capability.output_file;
            let mut observation = json!({
                "id": capability.id,
                "scope": capability.scope.to_string(),
                "backend": capability.backend.to_string(),
                "requested": execution_capability
                    .get("requested")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "required": execution_capability
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(capability.required),
                "status": execution_capability
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
            });
            if let Some(output_file) = output_file
                && signals_dir.join(output_file).is_file()
            {
                observation["output"] = json!(format!("signals/{output_file}"));
            }
            if let Some(error) = execution_capability.get("error").and_then(Value::as_str) {
                observation["error"] = json!(error);
            }
            if let Some(provider) = execution_capability.get("provider") {
                observation["provider"] = provider.clone();
            }
            if capability.id == "transcript"
                && let Some(output_file) = output_file
                && let Some(provider) = transcript_provider(&signals_dir.join(output_file))
            {
                observation["provider"] = provider;
            }
            Some(observation)
        })
        .collect();

    json!({
        "contract": "video-perception.observation/v1",
        "version": "1.0",
        "sense_plan": execution.get("plan").cloned().unwrap_or(Value::Null),
        "asset": {
            "uri": video,
            "duration_ms": duration_ms,
            "resolution": {
                "width": width,
                "height": height,
            },
            "fps": fps,
        },
        "scene_count": scene_count,
        "capabilities": capabilities,
        "files": {
            "dashboard": "dashboard.json",
            "scenes": "scenes.jsonl",
            "transcript": if has_transcript { Some("transcript.jsonl") } else { None },
        }
    })
}

pub fn validate_contract(manifest: &Value) -> Result<(), String> {
    match manifest.get("contract").and_then(Value::as_str) {
        Some("video-perception.observation/v1") => Ok(()),
        Some(contract) => Err(format!(
            "unsupported Observation contract: {contract} (expected video-perception.observation/v1)"
        )),
        None => Err("missing Observation contract in manifest.json".into()),
    }
}

fn transcript_provider(path: &Path) -> Option<Value> {
    let body = std::fs::read_to_string(path).ok()?;
    let transcript: Value = serde_json::from_str(&body).ok()?;
    let provenance = transcript.get("provenance")?;
    let id = provenance.get("provider")?.as_str()?;
    let locality = provenance.get("locality")?.as_str()?;
    let mut provider = json!({ "id": id, "locality": locality });
    if let Some(model) = provenance.get("model").and_then(Value::as_str) {
        provider["model"] = json!(model);
    }
    if let Some(timing) = provenance.get("timing").and_then(Value::as_str) {
        provider["timing"] = json!(timing);
    }
    Some(provider)
}

pub fn generate_readme(metadata: &Value, scenes: &Value, _manifest: &Value) -> String {
    let video = metadata
        .get("video")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let duration_ms = metadata
        .get("duration_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let scene_count = scenes
        .get("scenes")
        .and_then(|s| s.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let dur_sec = duration_ms as f64 / 1000.0;

    format!(
        r#"# Video Observation: {video}

Duration: {dur_sec:.1}s | Scenes: {scene_count}

## Quick Start

```bash
# Inspect the stable observation contract and capability inventory
cat manifest.json

# Read scene data (one JSON line per scene, all available signals merged)
cat scenes.jsonl

# Query measurements (L0 overview, L1 time-window, L2 raw frames)
video-perception observation query .                                      # L0
video-perception observation query . --window scene:0 --track signalstats # L1
video-perception observation query . --field signalstats.y --gte 100 --fmt spans
video-perception observation query . --frame 0 --track signalstats        # L2
video-perception observation query . --text "keyword"
```

## Structure

```
manifest.json             video-perception.observation/v1 capability index
dashboard.json            Aggregate measurements; no verdicts or thresholds
scenes.jsonl              Lossless scene-centric join of all signals
transcript.jsonl          Speech-to-text with scene alignment (when available)
signals/                  Raw capability outputs (full fidelity archive)
tracks/                   Raw time-series measurements (when available)
assets/
  keyframes/              Per-scene keyframe PNGs
```

## Consumer boundary

This observation contains measurements only. Product profiles, company
knowledge, thresholds, rules, sampling policy, and final judgments belong to
the consuming agent service. They are intentionally not registered here.

## Scene Record Format

Each line in `scenes.jsonl` contains data from the capabilities that ran:
```json
{{"idx":0,"start_ms":0,"end_ms":1050,"duration_ms":1050,"stats":{{...}},"audio":{{...}},"layout":{{...}},"track":{{...}},"beats":{{...}}}}
```
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn transcript_provider_is_summarized_in_manifest() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let signals = std::env::temp_dir().join(format!("video-perception-manifest-{unique}"));
        std::fs::create_dir_all(&signals).unwrap();
        std::fs::write(
            signals.join("transcript.json"),
            r#"{"provenance":{"provider":"apple-speech","locality":"local","model":"apple-speech","timing":"measured","attempts":[]},"language":"en","utterances":[]}"#,
        )
        .unwrap();

        let metadata = json!({
            "video": "video.mp4",
            "duration_ms": 1000,
            "width": 1920,
            "height": 1080,
            "fps": 30.0,
        });
        let execution = json!({
            "contract": "video-perception.execution-report/v1",
            "plan": {
                "contract": "video-perception.sense-plan/v1",
                "capabilities": [{"id": "transcript", "required": false}]
            },
            "capabilities": [{
                "id": "transcript",
                "requested": true,
                "required": false,
                "status": "succeeded",
                "output": "signals/transcript.json"
            }]
        });
        let manifest = generate(&metadata, &signals, &execution, true, 1);
        let transcript = manifest["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|capability| capability["id"] == "transcript")
            .unwrap();

        assert_eq!(transcript["provider"]["id"], "apple-speech");
        assert_eq!(transcript["provider"]["locality"], "local");
        assert_eq!(transcript["provider"]["timing"], "measured");
        assert_eq!(transcript["provider"]["model"], "apple-speech");
        assert_eq!(transcript["status"], "succeeded");
        assert_eq!(
            manifest["sense_plan"]["contract"],
            "video-perception.sense-plan/v1"
        );
        let _ = std::fs::remove_dir_all(signals);
    }

    #[test]
    fn validates_observation_contract() {
        assert!(validate_contract(&json!({"contract": "video-perception.observation/v1"})).is_ok());
        assert!(validate_contract(&json!({"contract": "other/v1"})).is_err());
        assert!(validate_contract(&json!({})).is_err());
    }
}
