use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ── Diff mode ───────────────────────────────────────────────

#[derive(Serialize)]
struct DiffReport {
    mode: String,
    a: String,
    b: String,
    scene_diffs: Vec<SceneDiff>,
    global_diffs: Vec<FieldDelta>,
    summary: DiffSummary,
}

#[derive(Serialize)]
struct SceneDiff {
    scene_idx: i64,
    deltas: Vec<FieldDelta>,
}

#[derive(Serialize, Clone)]
struct FieldDelta {
    path: String,
    a: Value,
    b: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pct_change: Option<f64>,
}

#[derive(Serialize)]
struct DiffSummary {
    scenes_a: usize,
    scenes_b: usize,
    total_fields_compared: usize,
    top_changes: Vec<FieldDelta>,
}

pub fn execute_diff(a_path: &str, b_path: &str) -> Result<(), String> {
    validate_observation(a_path)?;
    validate_observation(b_path)?;
    let scenes_a = load_scenes(a_path)?;
    let scenes_b = load_scenes(b_path)?;

    let mut scene_diffs = Vec::new();
    let mut all_deltas: Vec<FieldDelta> = Vec::new();

    let max_scenes = scenes_a.len().max(scenes_b.len());
    for i in 0..max_scenes {
        let sa = scenes_a.get(i);
        let sb = scenes_b.get(i);

        if let (Some(a), Some(b)) = (sa, sb) {
            let idx = a.get("idx").and_then(|v| v.as_i64()).unwrap_or(i as i64);
            let deltas = diff_objects(a, b, "");
            all_deltas.extend(deltas.clone());
            if !deltas.is_empty() {
                scene_diffs.push(SceneDiff {
                    scene_idx: idx,
                    deltas,
                });
            }
        }
    }

    // Global diffs (framerate, flicker — loaded from signal dirs)
    let global_diffs = diff_global_signals(a_path, b_path);
    all_deltas.extend(global_diffs.clone());

    let total = all_deltas.len();

    // Top changes by magnitude
    let mut sorted = all_deltas
        .iter()
        .filter(|d| d.pct_change.is_some())
        .cloned()
        .collect::<Vec<_>>();
    sorted.sort_by(|a, b| {
        b.pct_change
            .unwrap_or(0.0)
            .abs()
            .partial_cmp(&a.pct_change.unwrap_or(0.0).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_changes: Vec<FieldDelta> = sorted.into_iter().take(10).collect();

    let report = DiffReport {
        mode: "diff".into(),
        a: a_path.into(),
        b: b_path.into(),
        scene_diffs,
        global_diffs,
        summary: DiffSummary {
            scenes_a: scenes_a.len(),
            scenes_b: scenes_b.len(),
            total_fields_compared: total,
            top_changes,
        },
    };

    // Print summary to stderr, full JSON to stdout
    eprintln!("compare diff: {} vs {}", a_path, b_path);
    eprintln!("  scenes: {} vs {}", scenes_a.len(), scenes_b.len());
    eprintln!("  fields compared: {}", total);

    if !report.summary.top_changes.is_empty() {
        eprintln!("\n  Top changes:");
        for d in &report.summary.top_changes {
            let pct = d.pct_change.unwrap_or(0.0);
            let sign = if pct > 0.0 { "+" } else { "" };
            eprintln!(
                "    {:<40} {}{:.1}%  ({} → {})",
                d.path, sign, pct, d.a, d.b
            );
        }
    }

    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(())
}

fn validate_observation(path: &str) -> Result<(), String> {
    let manifest_path = Path::new(path).join("manifest.json");
    let body = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("read {}: {error}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_str(&body)
        .map_err(|error| format!("parse {}: {error}", manifest_path.display()))?;
    crate::workspace::manifest::validate_contract(&manifest)
}

// ── Helpers ─────────────────────────────────────────────────

fn load_scenes(path: &str) -> Result<Vec<Value>, String> {
    // Try Observation scenes.jsonl first, then the raw signal deck.
    let ws_path = Path::new(path).join("scenes.jsonl");
    let jsonl_path = if ws_path.exists() {
        ws_path
    } else {
        Path::new(path).to_path_buf()
    };

    if jsonl_path
        .extension()
        .map(|e| e == "jsonl")
        .unwrap_or(false)
        || jsonl_path.join("scenes.jsonl").exists()
    {
        let target = if jsonl_path.is_dir() {
            jsonl_path.join("scenes.jsonl")
        } else {
            jsonl_path
        };
        let file =
            fs::File::open(&target).map_err(|e| format!("open {}: {e}", target.display()))?;
        let reader = BufReader::new(file);
        let mut scenes = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| format!("read: {e}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&line).map_err(|e| format!("parse: {e}"))?;
            scenes.push(v);
        }
        return Ok(scenes);
    }

    Err(format!("No scenes.jsonl found in {path}"))
}

fn diff_objects(a: &Value, b: &Value, prefix: &str) -> Vec<FieldDelta> {
    let mut deltas = Vec::new();

    let a_obj = match a.as_object() {
        Some(o) => o,
        None => return deltas,
    };
    let b_obj = match b.as_object() {
        Some(o) => o,
        None => return deltas,
    };

    // Merge keys from both
    let mut all_keys: Vec<&String> = a_obj.keys().collect();
    for k in b_obj.keys() {
        if !a_obj.contains_key(k) {
            all_keys.push(k);
        }
    }
    all_keys.sort();

    for key in all_keys {
        // Skip index/timing fields
        if matches!(key.as_str(), "idx" | "start_ms" | "end_ms" | "duration_ms") {
            continue;
        }

        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let va = a_obj.get(key).unwrap_or(&Value::Null);
        let vb = b_obj.get(key).unwrap_or(&Value::Null);

        if va == vb {
            continue;
        }

        // Recurse into objects
        if va.is_object() && vb.is_object() {
            deltas.extend(diff_objects(va, vb, &path));
            continue;
        }

        // Numeric delta
        if let (Some(na), Some(nb)) = (va.as_f64(), vb.as_f64()) {
            let delta = nb - na;
            let pct = if na.abs() > 0.001 {
                (delta / na) * 100.0
            } else {
                0.0
            };
            deltas.push(FieldDelta {
                path,
                a: va.clone(),
                b: vb.clone(),
                delta: Some((delta * 100.0).round() / 100.0),
                pct_change: Some((pct * 10.0).round() / 10.0),
            });
        } else {
            // String/other changes
            deltas.push(FieldDelta {
                path,
                a: va.clone(),
                b: vb.clone(),
                delta: None,
                pct_change: None,
            });
        }
    }

    deltas
}

fn diff_global_signals(a_path: &str, b_path: &str) -> Vec<FieldDelta> {
    let mut deltas = Vec::new();

    // Compare global signal files (framerate, flicker)
    for filename in ["framerate.json", "flicker.json"] {
        let file_a = Path::new(a_path).join("signals").join(filename);
        let file_b = Path::new(b_path).join("signals").join(filename);

        // Also try a direct raw-signals path for internal callers.
        let file_a = if file_a.exists() {
            file_a
        } else {
            Path::new(a_path).join(filename)
        };
        let file_b = if file_b.exists() {
            file_b
        } else {
            Path::new(b_path).join(filename)
        };

        if let (Ok(ja), Ok(jb)) = (
            fs::read_to_string(&file_a).and_then(|s| {
                serde_json::from_str::<Value>(&s)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            }),
            fs::read_to_string(&file_b).and_then(|s| {
                serde_json::from_str::<Value>(&s)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            }),
        ) {
            let prefix = filename.trim_end_matches(".json");
            deltas.extend(diff_objects(&ja, &jb, prefix));
        }
    }

    deltas
}
