use serde_json::{Value, json};

/// Lossless scene-centric join of all sensor data.
///
/// Each scene record contains the full data from every sensor,
/// keyed by signal source. No fields are dropped.
pub fn join_scenes(
    scenes: &Value,
    scene_stats: Option<&Value>,
    scene_audio: Option<&Value>,
    layout: Option<&Value>,
    track: Option<&Value>,
    beats: Option<&Value>,
    transcript: Option<&Value>,
    blur: Option<&Value>,
    complexity: Option<&Value>,
    spectral: Option<&Value>,
    audio_class: Option<&Value>,
    signalstats: Option<&Value>,
    gibberish_text: Option<&Value>,
    scene_description: Option<&Value>,
    _signals_dir: &str,
) -> Result<Vec<Value>, String> {
    let scene_arr = scenes
        .get("scenes")
        .and_then(|s| s.as_array())
        .ok_or("scenes.json missing 'scenes' array")?;

    // Pre-extract beat timestamps for per-scene slicing
    let beat_times: Vec<i64> = beats
        .and_then(|b| b.get("beats"))
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("time_ms").and_then(|t| t.as_i64()))
                .collect()
        })
        .unwrap_or_default();

    let global_bpm = beats.and_then(|b| b.get("bpm")).and_then(|b| b.as_f64());

    // Pre-extract transcript utterances
    let utterances: Vec<&Value> = transcript
        .and_then(|t| t.get("utterances"))
        .and_then(|u| u.as_array())
        .map(|arr| arr.iter().collect())
        .unwrap_or_default();

    let mut result = Vec::with_capacity(scene_arr.len());

    for scene in scene_arr {
        let idx = scene.get("idx").and_then(|v| v.as_i64()).unwrap_or(0);
        let start_ms = scene.get("start_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let end_ms = scene.get("end_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let duration_ms = end_ms - start_ms;

        let mut record = json!({
            "idx": idx,
            "start_ms": start_ms,
            "end_ms": end_ms,
            "duration_ms": duration_ms,
        });

        // Stats: merge full object, rewrite asset paths
        if let Some(stat) = find_by_scene_idx(scene_stats, idx) {
            let mut stat_obj = stat.clone();
            if let Some(obj) = stat_obj.as_object_mut() {
                obj.remove("scene_idx");
                obj.remove("start_ms");
                obj.remove("end_ms");

                // Rewrite keyframe path to Observation-relative.
                if let Some(kf) = obj.get("keyframe").and_then(|v| v.as_str()) {
                    let fname = std::path::Path::new(kf)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    obj.insert(
                        "keyframe".into(),
                        json!(format!("assets/keyframes/{fname}")),
                    );
                }
            }
            record["stats"] = stat_obj;
        }

        // Audio: merge full object
        if let Some(audio) = find_by_scene_idx(scene_audio, idx) {
            let mut audio_obj = audio.clone();
            if let Some(obj) = audio_obj.as_object_mut() {
                obj.remove("scene_idx");
            }
            record["audio"] = audio_obj;
        }

        // Layout: merge full object (bars, content_region, split_diagnostics preserved)
        if let Some(layout_item) = find_by_scene_idx(layout, idx) {
            let mut layout_obj = layout_item.clone();
            if let Some(obj) = layout_obj.as_object_mut() {
                obj.remove("scene_idx");
            }
            record["layout"] = layout_obj;
        }

        // Track: merge full object (segments array preserved)
        if let Some(track_item) = find_by_scene_idx(track, idx) {
            let mut track_obj = track_item.clone();
            if let Some(obj) = track_obj.as_object_mut() {
                obj.remove("scene_idx");
            }
            record["track"] = track_obj;
        }

        // Blur: sharpness measurements
        if let Some(blur_item) = find_by_scene_idx(blur, idx) {
            let mut obj = blur_item.clone();
            if let Some(o) = obj.as_object_mut() {
                o.remove("scene_idx");
            }
            record["blur"] = obj;
        }

        // Complexity: visual complexity measurements
        if let Some(comp_item) = find_by_scene_idx(complexity, idx) {
            let mut obj = comp_item.clone();
            if let Some(o) = obj.as_object_mut() {
                o.remove("scene_idx");
            }
            record["complexity"] = obj;
        }

        // Spectral: audio spectral features
        if let Some(spec_item) = find_by_scene_idx(spectral, idx) {
            let mut obj = spec_item.clone();
            if let Some(o) = obj.as_object_mut() {
                o.remove("scene_idx");
            }
            record["spectral"] = obj;
        }

        // Audio class: speech/music/silence
        if let Some(ac_item) = find_by_scene_idx(audio_class, idx) {
            let mut obj = ac_item.clone();
            if let Some(o) = obj.as_object_mut() {
                o.remove("scene_idx");
            }
            record["audio_class"] = obj;
        }

        // Signalstats: frame-level signal quality
        if let Some(ss_item) = find_by_scene_idx(signalstats, idx) {
            let mut obj = ss_item.clone();
            if let Some(o) = obj.as_object_mut() {
                o.remove("scene_idx");
            }
            record["signalstats"] = obj;
        }

        // VLM: gibberish text detection
        if let Some(gt_scene) = find_vlm_scene(gibberish_text, idx) {
            record["gibberish_text"] = gt_scene;
        }

        // VLM: scene description
        if let Some(sd_scene) = find_vlm_scene(scene_description, idx) {
            record["scene_description"] = sd_scene;
        }

        // Beats: slice timestamps into this scene, compute count + density
        let scene_beats: Vec<i64> = beat_times
            .iter()
            .filter(|&&t| t >= start_ms && t < end_ms)
            .copied()
            .collect();
        let beat_count = scene_beats.len() as f64;
        let dur_sec = duration_ms as f64 / 1000.0;
        let beat_density = if dur_sec > 0.0 {
            beat_count / dur_sec
        } else {
            0.0
        };
        let beat_density = (beat_density * 100.0).round() / 100.0;

        record["beats"] = json!({
            "count": beat_count as i64,
            "density": beat_density,
            "timestamps_ms": scene_beats,
            "bpm": global_bpm,
        });

        // Deltas: expose inter-scene measurements for downstream consumers.
        if idx > 0 {
            if let (Some(curr_stat), Some(prev_stat)) = (
                find_by_scene_idx(scene_stats, idx),
                find_by_scene_idx(scene_stats, idx - 1),
            ) {
                let mut deltas = serde_json::Map::new();
                if let (Some(curr), Some(prev)) = (
                    curr_stat.get("avg_brightness").and_then(|v| v.as_f64()),
                    prev_stat.get("avg_brightness").and_then(|v| v.as_f64()),
                ) {
                    deltas.insert("exposure".into(), json!((curr - prev).abs()));
                }
                if let (Some(curr), Some(prev)) = (
                    curr_stat.get("saturation").and_then(|v| v.as_f64()),
                    prev_stat.get("saturation").and_then(|v| v.as_f64()),
                ) {
                    deltas.insert("saturation".into(), json!((curr - prev).abs()));
                }
                if let (Some(curr), Some(prev)) = (
                    curr_stat.get("color_temp_k").and_then(|v| v.as_f64()),
                    prev_stat.get("color_temp_k").and_then(|v| v.as_f64()),
                ) {
                    deltas.insert("color_temp".into(), json!((curr - prev).abs()));
                }
                if !deltas.is_empty() {
                    record["deltas"] = Value::Object(deltas);
                }
            }
        }

        // Transcript: utterances whose midpoint falls in this scene
        let scene_utterances: Vec<Value> = utterances
            .iter()
            .filter_map(|utt| {
                let u_start = utt.get("start_ms").and_then(|v| v.as_i64())?;
                let u_end = utt.get("end_ms").and_then(|v| v.as_i64())?;
                let mid = (u_start + u_end) / 2;
                if mid >= start_ms && mid < end_ms {
                    let mut u = json!({
                        "start_ms": u_start,
                        "end_ms": u_end,
                        "text": utt.get("text").and_then(|t| t.as_str()).unwrap_or(""),
                    });
                    if let Some(spk) = utt.get("speaker").and_then(|s| s.as_str()) {
                        u["speaker"] = json!(spk);
                    }
                    Some(u)
                } else {
                    None
                }
            })
            .collect();

        if !scene_utterances.is_empty() {
            record["transcript"] = json!(scene_utterances);
        }

        result.push(record);
    }

    Ok(result)
}

/// Generate transcript.jsonl — utterances with scene_idx alignment.
pub fn transcript_to_jsonl(transcript: &Value, scenes: &Value) -> Result<String, String> {
    let utterances = transcript
        .get("utterances")
        .and_then(|u| u.as_array())
        .ok_or("transcript missing 'utterances' array")?;

    let scene_arr = scenes
        .get("scenes")
        .and_then(|s| s.as_array())
        .ok_or("scenes missing 'scenes' array")?;

    let mut lines = Vec::new();
    for utt in utterances {
        let start = utt.get("start_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let end = utt.get("end_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let mid = (start + end) / 2;

        let scene_idx = scene_arr
            .iter()
            .filter_map(|s| {
                let s_start = s.get("start_ms").and_then(|v| v.as_i64())?;
                let s_end = s.get("end_ms").and_then(|v| v.as_i64())?;
                let idx = s.get("idx").and_then(|v| v.as_i64())?;
                if mid >= s_start && mid < s_end {
                    Some(idx)
                } else {
                    None
                }
            })
            .next()
            .unwrap_or(0);

        let text = utt.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let speaker = utt.get("speaker").and_then(|s| s.as_str());

        let mut line = json!({
            "scene_idx": scene_idx,
            "start_ms": start,
            "end_ms": end,
            "text": text,
        });
        if let Some(spk) = speaker {
            line["speaker"] = json!(spk);
        }
        lines.push(serde_json::to_string(&line).unwrap());
    }
    Ok(lines.join("\n") + "\n")
}

fn find_by_scene_idx<'a>(data: Option<&'a Value>, idx: i64) -> Option<&'a Value> {
    let arr = data?.as_array()?;
    arr.iter().find(|item| {
        item.get("scene_idx")
            .and_then(|v| v.as_i64())
            .map(|i| i == idx)
            .unwrap_or(false)
    })
}

/// Extract a scene from VLM sensor output (format: {scenes: [{scene_idx, ...}]}).
fn find_vlm_scene(data: Option<&Value>, idx: i64) -> Option<Value> {
    let scenes = data?.get("scenes")?.as_array()?;
    scenes
        .iter()
        .find(|s| {
            s.get("scene_idx")
                .and_then(|v| v.as_i64())
                .map(|i| i == idx)
                .unwrap_or(false)
        })
        .map(|s| {
            let mut obj = s.clone();
            if let Some(o) = obj.as_object_mut() {
                o.remove("scene_idx");
            }
            obj
        })
}
