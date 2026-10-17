use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ── Types ───────────────────────────────────────────────────────────

enum Layer {
    Overview,
    Window,
    Frame,
    Cross,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Json,
    Text,
    Stats,
    Spans,
}

impl Format {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "json" => Ok(Format::Json),
            "text" => Ok(Format::Text),
            "stats" => Ok(Format::Stats),
            "spans" => Ok(Format::Spans),
            _ => Err(format!(
                "unknown format '{s}', expected: json, text, stats, spans"
            )),
        }
    }
}

fn detect_layer(
    at: Option<&str>,
    frame: Option<u64>,
    window: Option<&str>,
    field: Option<&str>,
    text: Option<&str>,
) -> Layer {
    if at.is_some() {
        return Layer::Cross;
    }
    if frame.is_some() {
        return Layer::Frame;
    }
    if window.is_some() || field.is_some() || text.is_some() {
        return Layer::Window;
    }
    Layer::Overview
}

// ── Entry point ─────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn execute(
    workspace_arg: &str,
    window: Option<&str>,
    field: Option<&str>,
    gte: Option<f64>,
    lte: Option<f64>,
    text: Option<&str>,
    limit: usize,
    frame: Option<u64>,
    track: Option<&str>,
    at: Option<&str>,
    fmt_str: &str,
) -> Result<(), String> {
    let fmt = Format::parse(fmt_str)?;
    let ws = resolve_workspace(workspace_arg)?;
    let manifest_path = Path::new(&ws).join("manifest.json");
    let manifest_body = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("read {}: {error}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_str(&manifest_body)
        .map_err(|error| format!("parse {}: {error}", manifest_path.display()))?;
    super::manifest::validate_contract(&manifest)?;
    let layer = detect_layer(at, frame, window, field, text);

    match layer {
        Layer::Overview => execute_overview(&ws, fmt),
        Layer::Window => execute_window(&ws, window, field, gte, lte, text, track, limit, fmt),
        Layer::Frame => execute_frame(&ws, frame.unwrap(), track, fmt),
        Layer::Cross => execute_at(&ws, at.unwrap(), fmt),
    }
}

// ── Workspace auto-detection ────────────────────────────────────────

fn resolve_workspace(arg: &str) -> Result<String, String> {
    let p = Path::new(arg);
    // Resolve by the stable contract boundary first; data files are checked
    // only after manifest validation.
    if p.join("manifest.json").exists() {
        return Ok(arg.to_string());
    }
    // Default "obs" — check ./obs/
    if arg == "obs" {
        let obs = Path::new("./obs");
        if obs.join("manifest.json").exists() {
            return Ok("obs".to_string());
        }
        // Maybe we're inside the Observation
        if Path::new("./manifest.json").exists() {
            return Ok(".".to_string());
        }
    }
    Err(format!(
        "Observation not found at '{arg}'. Run `video-perception observe` first, or pass the path."
    ))
}

// ── Parsers ─────────────────────────────────────────────────────────

fn parse_timestamp(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix('s') {
        if rest.ends_with('m') {
            let num = rest
                .strip_suffix('m')
                .ok_or_else(|| format!("invalid timestamp: {s}"))?;
            num.parse::<f64>()
                .map(|v| v as u64)
                .map_err(|_| format!("invalid timestamp: {s}"))
        } else {
            rest.parse::<f64>()
                .map(|v| (v * 1000.0) as u64)
                .map_err(|_| format!("invalid timestamp: {s}"))
        }
    } else {
        s.parse::<f64>()
            .map(|v| v as u64)
            .map_err(|_| format!("invalid timestamp: {s}"))
    }
}

/// Parse --window value: "scene:3", "10s-20s", "10000-20000"
fn parse_window(s: &str, ws: &Path) -> Result<(u64, u64), String> {
    // scene:N → look up from signals/scenes.json
    if let Some(idx_str) = s.strip_prefix("scene:") {
        let idx: i64 = idx_str
            .parse()
            .map_err(|_| format!("invalid scene index: {idx_str}"))?;
        let scenes = load_raw_scenes(ws)?;
        return find_scene_window(&scenes, idx);
    }
    // Try "Xs-Ys" format (split on "s-")
    if let Some(pos) = s.find("s-") {
        let start = parse_timestamp(&s[..=pos])?;
        let end = parse_timestamp(&s[pos + 2..])?;
        return Ok((start, end));
    }
    // Plain "START-END" in ms
    let parts: Vec<&str> = s.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid window '{s}', expected: scene:N, 10s-20s, or 10000-20000"
        ));
    }
    let start = parse_timestamp(parts[0])?;
    let end = parse_timestamp(parts[1])?;
    Ok((start, end))
}

fn load_raw_scenes(ws: &Path) -> Result<Value, String> {
    let path = ws.join("signals/scenes.json");
    let content =
        fs::read_to_string(&path).map_err(|e| format!("read signals/scenes.json: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse signals/scenes.json: {e}"))
}

fn find_scene_window(scenes: &Value, idx: i64) -> Result<(u64, u64), String> {
    let arr = scenes
        .get("scenes")
        .and_then(|s| s.as_array())
        .ok_or("scenes.json missing 'scenes' array")?;
    for scene in arr {
        if scene.get("idx").and_then(|v| v.as_i64()) == Some(idx) {
            let start = scene.get("start_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let end = scene.get("end_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            return Ok((start, end));
        }
    }
    Err(format!("scene {idx} not found in signals/scenes.json"))
}

/// Parse "track_name.field_name" → (Some("track_name"), Some("field_name"))
fn parse_field_path(field: Option<&str>) -> (Option<&str>, Option<&str>) {
    match field {
        Some(f) if f.contains('.') => {
            let (t, n) = f.split_once('.').unwrap();
            (Some(t), Some(n))
        }
        Some(f) => (None, Some(f)),
        None => (None, None),
    }
}

fn resolve_track_list<'a>(
    available: &'a [String],
    track_filter: Option<&str>,
    field_track: Option<&str>,
) -> Result<Vec<&'a str>, String> {
    if let (Some(ft), Some(tf)) = (field_track, track_filter) {
        if ft != tf {
            return Err(format!(
                "--field implies track '{ft}' but --track is '{tf}' — use one or the other"
            ));
        }
    }
    let target = field_track.or(track_filter);
    if let Some(name) = target {
        if !available.iter().any(|t| t == name) {
            return Err(format!(
                "track '{name}' not found. available: {}",
                available.join(", ")
            ));
        }
        Ok(vec![available.iter().find(|t| t.as_str() == name).unwrap()])
    } else {
        Ok(available.iter().map(|s| s.as_str()).collect())
    }
}

// ── L0: Dashboard overview ──────────────────────────────────────────

fn execute_overview(workspace_dir: &str, fmt: Format) -> Result<(), String> {
    let ws = Path::new(workspace_dir);
    let content = fs::read_to_string(ws.join("dashboard.json"))
        .map_err(|e| format!("open dashboard.json: {e}"))?;
    let dashboard: Value =
        serde_json::from_str(&content).map_err(|e| format!("parse dashboard.json: {e}"))?;

    match fmt {
        Format::Text => print_overview_text(&dashboard),
        _ => println!("{}", serde_json::to_string_pretty(&dashboard).unwrap()),
    }
    Ok(())
}

fn print_overview_text(d: &Value) {
    if let Some(vid) = d.get("video_identity") {
        let video = vid.get("video").and_then(|v| v.as_str()).unwrap_or("?");
        let dur = vid.get("duration_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let res = vid
            .get("resolution")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let fps = vid.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let scenes = vid.get("scene_count").and_then(|v| v.as_i64()).unwrap_or(0);
        println!("  video: {video}");
        println!("  {dur}ms | {res} @ {fps}fps | {scenes} scenes");
    }

    if let Some(stats) = d.get("global_stats") {
        // Brightness
        if let Some(br) = stats.get("brightness").filter(|v| !v.is_null()) {
            let mean = br.get("mean").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let std = br.get("std").and_then(|v| v.as_f64()).unwrap_or(0.0);
            println!("  brightness: {mean:.1} +/- {std:.1} IRE");
        }
        // Motion
        if let Some(motion) = stats.get("motion").filter(|v| !v.is_null()) {
            if let Some(obj) = motion.as_object() {
                let parts: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.as_i64().unwrap_or(0)))
                    .collect();
                println!("  motion: {}", parts.join(" "));
            }
        }
        // Pacing
        if let Some(pacing) = stats.get("pacing").filter(|v| !v.is_null()) {
            let med = pacing
                .get("median_scene_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let short = pacing
                .get("shortest_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let long = pacing
                .get("longest_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            println!("  pacing: median {med}ms ({short}–{long}ms)");
        }
        // Audio
        if let Some(audio) = stats.get("audio").filter(|v| !v.is_null()) {
            let lufs = audio.get("lufs").and_then(|v| v.as_f64());
            let peak = audio.get("true_peak_dbfs").and_then(|v| v.as_f64());
            if let (Some(l), Some(p)) = (lufs, peak) {
                println!("  audio: {l:.1} LUFS, peak {p:.1} dBFS");
            }
        }
        // AV sync
        if let Some(avsync) = stats.get("avsync").filter(|v| !v.is_null()) {
            if avsync
                .get("has_audio")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let offset = avsync
                    .get("offset_ms")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                println!("  avsync: {offset}ms offset");
            }
        }
        // Blur
        if let Some(blur) = stats.get("blur").filter(|v| !v.is_null()) {
            let lap = blur
                .get("laplacian_mean")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let class = blur.get("sharpness").and_then(|v| v.as_str()).unwrap_or("");
            if lap > 0.0 {
                println!("  sharpness: {lap:.0} laplacian ({class})");
            }
        }
        // Complexity
        if let Some(cx) = stats.get("complexity").filter(|v| !v.is_null()) {
            let edge = cx
                .get("edge_density")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let class = cx.get("class").and_then(|v| v.as_str()).unwrap_or("");
            if edge > 0.0 {
                println!("  complexity: edges={edge:.3} ({class})");
            }
        }
        // Framerate
        if let Some(fr) = stats.get("framerate").filter(|v| !v.is_null()) {
            let container = fr
                .get("container_fps")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let content = fr
                .get("content_fps")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let dups = fr
                .get("duplicate_ratio")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if container != content || dups > 0.0 {
                println!(
                    "  framerate: {container}fps container, {content}fps content, {:.1}% dups",
                    dups * 100.0
                );
            }
        }
        // Flicker
        if let Some(fl) = stats.get("flicker").filter(|v| !v.is_null()) {
            let flashes = fl.get("flash_count").and_then(|v| v.as_i64()).unwrap_or(0);
            let compliant = fl
                .get("bt1702_compliant")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if flashes > 0 || !compliant {
                println!(
                    "  flicker: {} flashes{}",
                    flashes,
                    if !compliant {
                        " (BT.1702 non-compliant)"
                    } else {
                        ""
                    }
                );
            }
        }
        // Color
        if let Some(color) = stats.get("color").filter(|v| !v.is_null()) {
            if let Some(palette) = color.get("palette").and_then(|v| v.as_array()) {
                let colors: Vec<&str> = palette.iter().filter_map(|c| c.as_str()).take(5).collect();
                if !colors.is_empty() {
                    println!("  palette: {}", colors.join(" "));
                }
            }
        }
        // Beats
        if let Some(beats) = stats.get("beats").filter(|v| !v.is_null()) {
            let bpm = beats.get("bpm").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let density = beats.get("density").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if bpm > 0.0 {
                println!("  beats: {bpm:.0} BPM, density {density:.2}");
            }
        }
        // Layout
        if let Some(layout) = stats.get("layout").filter(|v| !v.is_null()) {
            let lt = layout.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if !lt.is_empty() {
                println!("  layout: {lt}");
            }
        }
    }
}

// ── L1: Time-window queries ─────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn execute_window(
    workspace_dir: &str,
    window: Option<&str>,
    field: Option<&str>,
    gte: Option<f64>,
    lte: Option<f64>,
    text: Option<&str>,
    track_filter: Option<&str>,
    limit: usize,
    fmt: Format,
) -> Result<(), String> {
    let ws = Path::new(workspace_dir);

    if let Some(query) = text {
        let time_window = resolve_window(ws, window)?;
        return search_transcript(ws, query, time_window, limit, fmt);
    }

    let tracks_dir = ws.join("tracks");
    if !tracks_dir.exists() {
        return Err("no tracks/ directory in Observation".into());
    }

    let (start_ms, end_ms) = resolve_window(ws, window)?;
    let available = list_tracks(&tracks_dir)?;
    if available.is_empty() {
        return Err("no track files found".into());
    }

    let (field_track, field_name) = parse_field_path(field);
    let effective_tracks = resolve_track_list(&available, track_filter, field_track)?;

    // Collect frames from all tracks
    let mut all_results = serde_json::Map::new();
    let mut total_frames = 0usize;

    for name in &effective_tracks {
        let path = tracks_dir.join(format!("{name}.track.jsonl"));
        let frames = load_track_generic(&path)?;

        let windowed: Vec<&Value> = frames
            .iter()
            .filter(|f| {
                let t = f.get("t").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
                t >= start_ms && t < end_ms
            })
            .collect();

        let filtered: Vec<&Value> = if let Some(fname) = field_name {
            windowed
                .into_iter()
                .filter(|f| {
                    if let Some(val) = f.get(fname).and_then(|v| v.as_f64()) {
                        if let Some(lo) = gte {
                            if val < lo {
                                return false;
                            }
                        }
                        if let Some(hi) = lte {
                            if val > hi {
                                return false;
                            }
                        }
                        true
                    } else {
                        false
                    }
                })
                .collect()
        } else {
            windowed
        };

        total_frames += filtered.len();

        if filtered.is_empty() {
            continue;
        }

        match fmt {
            Format::Stats | Format::Text => {
                let agg = aggregate_frames(&filtered, field_name);
                all_results.insert(name.to_string(), agg);
            }
            Format::Spans => {
                let spans = compute_spans(&filtered, field_name);
                if !spans.is_empty() {
                    all_results.insert(name.to_string(), Value::Array(spans));
                }
            }
            Format::Json => {
                let limited: Vec<Value> = filtered.into_iter().take(limit).cloned().collect();
                if !limited.is_empty() {
                    all_results.insert(name.to_string(), Value::Array(limited));
                }
            }
        }
    }

    if all_results.is_empty() {
        let desc = window.unwrap_or("full video");
        return Err(format!("no track data in {desc}"));
    }

    let desc = window.unwrap_or("full video");

    match fmt {
        Format::Text | Format::Stats => {
            println!("  {desc}: {total_frames} frames");
            for (name, agg) in &all_results {
                println!("    {name}: {agg}");
            }
            eprintln!("{total_frames} frames");
        }
        Format::Spans => {
            // Flatten spans from all tracks into output
            println!(
                "{}",
                serde_json::to_string_pretty(&Value::Object(all_results)).unwrap()
            );
            eprintln!("{total_frames} matching frames");
        }
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&Value::Object(all_results)).unwrap()
            );
            eprintln!("{total_frames} frames");
        }
    }

    Ok(())
}

fn resolve_window(ws: &Path, window: Option<&str>) -> Result<(u64, u64), String> {
    match window {
        Some(w) => parse_window(w, ws),
        None => Ok((0, u64::MAX)),
    }
}

fn aggregate_frames(frames: &[&Value], field: Option<&str>) -> Value {
    let n = frames.len();
    if n == 0 {
        return json!({"frames": 0});
    }

    let t_min = frames
        .iter()
        .filter_map(|f| f.get("t").and_then(|v| v.as_u64()))
        .min()
        .unwrap_or(0);
    let t_max = frames
        .iter()
        .filter_map(|f| f.get("t").and_then(|v| v.as_u64()))
        .max()
        .unwrap_or(0);

    if let Some(fname) = field {
        let vals: Vec<f64> = frames
            .iter()
            .filter_map(|f| f.get(fname).and_then(|v| v.as_f64()))
            .collect();
        if vals.is_empty() {
            return json!({"frames": n, "start_ms": t_min, "end_ms": t_max});
        }
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        let min = vals.iter().copied().fold(f64::MAX, f64::min);
        let max = vals.iter().copied().fold(f64::MIN, f64::max);
        let std = if vals.len() > 1 {
            (vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64).sqrt()
        } else {
            0.0
        };

        json!({
            "frames": n, "start_ms": t_min, "end_ms": t_max,
            "mean": r2(mean), "std": r2(std), "min": r2(min), "max": r2(max),
        })
    } else {
        json!({"frames": n, "start_ms": t_min, "end_ms": t_max})
    }
}

/// Group contiguous matching frames into time spans.
fn compute_spans(frames: &[&Value], field: Option<&str>) -> Vec<Value> {
    if frames.is_empty() {
        return vec![];
    }

    // Get sorted timestamps
    let mut timed: Vec<(u64, f64)> = frames
        .iter()
        .filter_map(|f| {
            let t = f.get("t").and_then(|v| v.as_u64())?;
            let val = field
                .and_then(|fname| f.get(fname).and_then(|v| v.as_f64()))
                .unwrap_or(0.0);
            Some((t, val))
        })
        .collect();
    timed.sort_by_key(|(t, _)| *t);

    if timed.is_empty() {
        return vec![];
    }

    // Compute median inter-frame gap
    let gaps: Vec<u64> = timed
        .windows(2)
        .map(|w| w[1].0.saturating_sub(w[0].0))
        .collect();
    let median_gap = if gaps.is_empty() {
        500
    } else {
        let mut sorted = gaps.clone();
        sorted.sort();
        sorted[sorted.len() / 2]
    };
    let gap_threshold = (median_gap * 2).max(100); // at least 100ms

    // Walk and group
    let mut spans = Vec::new();
    let mut span_start = timed[0].0;
    let mut span_vals: Vec<f64> = vec![timed[0].1];
    let mut prev_t = timed[0].0;

    for &(t, val) in &timed[1..] {
        if t.saturating_sub(prev_t) > gap_threshold {
            // Close current span
            spans.push(make_span(span_start, prev_t, &span_vals));
            span_start = t;
            span_vals.clear();
        }
        span_vals.push(val);
        prev_t = t;
    }
    // Close final span
    spans.push(make_span(span_start, prev_t, &span_vals));

    spans
}

fn make_span(start: u64, end: u64, vals: &[f64]) -> Value {
    let n = vals.len();
    if n == 0 {
        return json!({"start_ms": start, "end_ms": end, "frames": 0});
    }

    let mean = vals.iter().sum::<f64>() / n as f64;
    let min = vals.iter().copied().fold(f64::MAX, f64::min);
    let max = vals.iter().copied().fold(f64::MIN, f64::max);

    json!({
        "start_ms": start, "end_ms": end, "frames": n,
        "mean": r2(mean), "min": r2(min), "max": r2(max),
    })
}

fn search_transcript(
    ws: &Path,
    query: &str,
    time_window: (u64, u64),
    limit: usize,
    fmt: Format,
) -> Result<(), String> {
    let transcript_path = ws.join("transcript.jsonl");
    if !transcript_path.exists() {
        return Err("no transcript available".into());
    }

    let file =
        fs::File::open(&transcript_path).map_err(|e| format!("open transcript.jsonl: {e}"))?;
    let reader = BufReader::new(file);
    let query_lower = query.to_lowercase();
    let (win_start, win_end) = time_window;

    let mut results = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read line: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Value>(&line) {
            let text = entry.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let start = entry.get("start_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            if start < win_start || start >= win_end {
                continue;
            }
            if text.to_lowercase().contains(&query_lower) {
                results.push(line.clone());
                if results.len() >= limit {
                    break;
                }
            }
        }
    }

    if fmt == Format::Text {
        for r in &results {
            if let Ok(v) = serde_json::from_str::<Value>(r) {
                let start = v.get("start_ms").and_then(|s| s.as_i64()).unwrap_or(0);
                let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
                let short = if text.len() > 60 { &text[..60] } else { text };
                println!("  {start:>8}ms | {short}");
            }
        }
    } else {
        for r in &results {
            println!("{r}");
        }
    }
    eprintln!("{} results", results.len());
    Ok(())
}

// ── L2: Frame lookup ────────────────────────────────────────────────

fn execute_frame(
    workspace_dir: &str,
    frame_num: u64,
    track_name: Option<&str>,
    fmt: Format,
) -> Result<(), String> {
    let ws = Path::new(workspace_dir);
    let tracks_dir = ws.join("tracks");
    if !tracks_dir.exists() {
        return Err("no tracks/ directory in Observation".into());
    }

    let available = list_tracks(&tracks_dir)?;
    let track_names: Vec<&str> = if let Some(name) = track_name {
        if !available.iter().any(|t| t == name) {
            return Err(format!(
                "track '{name}' not found. available: {}",
                available.join(", ")
            ));
        }
        vec![name]
    } else {
        available.iter().map(|s| s.as_str()).collect()
    };

    let mut result = serde_json::Map::new();
    for name in &track_names {
        let path = tracks_dir.join(format!("{name}.track.jsonl"));
        if let Ok(val) = query_track_by_frame(&path, frame_num) {
            result.insert(name.to_string(), val);
        }
    }
    if result.is_empty() {
        return Err(format!("frame {frame_num} not found in any track"));
    }

    if fmt == Format::Text {
        println!("  frame {frame_num}:");
        for (name, val) in &result {
            println!("    {name}: {val}");
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Object(result)).unwrap()
        );
    }
    Ok(())
}

// ── Cross-layer: --at timestamp ─────────────────────────────────────

fn execute_at(workspace_dir: &str, timestamp: &str, fmt: Format) -> Result<(), String> {
    let ws = Path::new(workspace_dir);
    let ts_ms = parse_timestamp(timestamp)?;

    let mut result = serde_json::Map::new();

    // L0: dashboard context
    let dashboard_path = ws.join("dashboard.json");
    if dashboard_path.exists() {
        let content =
            fs::read_to_string(&dashboard_path).map_err(|e| format!("read dashboard.json: {e}"))?;
        let dashboard: Value =
            serde_json::from_str(&content).map_err(|e| format!("parse dashboard.json: {e}"))?;

        let mut dash_slice = serde_json::Map::new();
        if let Some(vid) = dashboard.get("video_identity") {
            dash_slice.insert("video_identity".into(), vid.clone());
        }
        result.insert("dashboard".into(), Value::Object(dash_slice));
    }

    // Scene context from raw sensor output
    if let Ok(scenes_val) = load_raw_scenes(ws) {
        if let Some(arr) = scenes_val.get("scenes").and_then(|s| s.as_array()) {
            let scene = arr.iter().find(|s| {
                let start = s.get("start_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                let end = s.get("end_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                ts_ms >= start && ts_ms < end
            });
            if let Some(s) = scene {
                result.insert("scene".into(), s.clone());
            }
        }
    }

    // Track frames nearest this timestamp
    let tracks_dir = ws.join("tracks");
    if tracks_dir.exists() {
        if let Ok(available) = list_tracks(&tracks_dir) {
            let mut track_data = serde_json::Map::new();
            for name in &available {
                let path = tracks_dir.join(format!("{name}.track.jsonl"));
                if let Ok(items) = load_track_generic(&path) {
                    if let Some(frame) = find_nearest_frame(&items, ts_ms) {
                        track_data.insert(name.clone(), frame.clone());
                    }
                }
            }
            if !track_data.is_empty() {
                result.insert("tracks".into(), Value::Object(track_data));
            }
        }
    }

    if result.is_empty() {
        return Err(format!("no data at {timestamp} ({ts_ms}ms)"));
    }

    if fmt == Format::Text {
        println!("  at {timestamp} ({ts_ms}ms):");
        if let Some(scene) = result.get("scene") {
            let idx = scene.get("idx").and_then(|v| v.as_i64()).unwrap_or(0);
            let start = scene.get("start_ms").and_then(|v| v.as_i64()).unwrap_or(0);
            let end = scene.get("end_ms").and_then(|v| v.as_i64()).unwrap_or(0);
            println!("    scene {idx} ({start}–{end}ms)");
        }
        if let Some(tracks) = result.get("tracks").and_then(|v| v.as_object()) {
            for (name, val) in tracks {
                println!("    {name}: {val}");
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Object(result)).unwrap()
        );
    }
    Ok(())
}

// ── Shared helpers ──────────────────────────────────────────────────

fn list_tracks(tracks_dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let entries = fs::read_dir(tracks_dir).map_err(|e| format!("read tracks/: {e}"))?;
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if let Some(stem) = fname.strip_suffix(".track.jsonl") {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

fn load_track_generic(path: &Path) -> Result<Vec<Value>, String> {
    let file = fs::File::open(path).map_err(|e| format!("open track {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line).map_err(|e| format!("parse track: {e}"))?;
        items.push(v);
    }
    Ok(items)
}

fn query_track_by_frame(path: &Path, frame: u64) -> Result<Value, String> {
    let items = load_track_generic(path)?;
    for item in &items {
        if item.get("f").and_then(|v| v.as_u64()) == Some(frame) {
            return Ok(item.clone());
        }
    }
    for item in &items {
        if item.get("t").and_then(|v| v.as_u64()) == Some(frame) {
            return Ok(item.clone());
        }
    }
    Err(format!("frame {frame} not found"))
}

fn find_nearest_frame<'a>(items: &'a [Value], ts_ms: u64) -> Option<&'a Value> {
    if items.is_empty() {
        return None;
    }
    if items.first().and_then(|v| v.get("t")).is_some() {
        items.iter().min_by_key(|item| {
            let t = item.get("t").and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
            (t as i64 - ts_ms as i64).unsigned_abs()
        })
    } else {
        items.first()
    }
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp_seconds() {
        assert_eq!(parse_timestamp("12.5s").unwrap(), 12500);
        assert_eq!(parse_timestamp("0.5s").unwrap(), 500);
        assert_eq!(parse_timestamp("1s").unwrap(), 1000);
    }

    #[test]
    fn test_parse_timestamp_ms() {
        assert_eq!(parse_timestamp("12500ms").unwrap(), 12500);
        assert_eq!(parse_timestamp("500ms").unwrap(), 500);
    }

    #[test]
    fn test_parse_timestamp_plain() {
        assert_eq!(parse_timestamp("12500").unwrap(), 12500);
        assert_eq!(parse_timestamp("0").unwrap(), 0);
    }

    #[test]
    fn test_detect_layer_default_overview() {
        let layer = detect_layer(None, None, None, None, None);
        assert!(matches!(layer, Layer::Overview));
    }

    #[test]
    fn test_detect_layer_at_is_cross() {
        let layer = detect_layer(Some("12.5s"), None, None, None, None);
        assert!(matches!(layer, Layer::Cross));
    }

    #[test]
    fn test_detect_layer_frame() {
        let layer = detect_layer(None, Some(100), None, None, None);
        assert!(matches!(layer, Layer::Frame));
    }

    #[test]
    fn test_detect_layer_window() {
        let layer = detect_layer(None, None, Some("scene:3"), None, None);
        assert!(matches!(layer, Layer::Window));
    }

    #[test]
    fn test_detect_layer_field() {
        let layer = detect_layer(None, None, None, Some("signalstats.y"), None);
        assert!(matches!(layer, Layer::Window));
    }

    #[test]
    fn test_detect_layer_text() {
        let layer = detect_layer(None, None, None, None, Some("hello"));
        assert!(matches!(layer, Layer::Window));
    }

    #[test]
    fn test_parse_field_path_with_track() {
        let (t, f) = parse_field_path(Some("signalstats.y"));
        assert_eq!(t, Some("signalstats"));
        assert_eq!(f, Some("y"));
    }

    #[test]
    fn test_parse_field_path_bare() {
        let (t, f) = parse_field_path(Some("y"));
        assert_eq!(t, None);
        assert_eq!(f, Some("y"));
    }

    #[test]
    fn test_parse_field_path_none() {
        let (t, f) = parse_field_path(None);
        assert_eq!(t, None);
        assert_eq!(f, None);
    }

    #[test]
    fn test_format_parse() {
        assert!(matches!(Format::parse("json").unwrap(), Format::Json));
        assert!(matches!(Format::parse("text").unwrap(), Format::Text));
        assert!(matches!(Format::parse("stats").unwrap(), Format::Stats));
        assert!(matches!(Format::parse("spans").unwrap(), Format::Spans));
        assert!(Format::parse("bad").is_err());
    }

    #[test]
    fn test_compute_spans_single() {
        let frames: Vec<Value> = vec![
            json!({"t": 1000, "y": 50.0}),
            json!({"t": 1042, "y": 55.0}),
            json!({"t": 1083, "y": 52.0}),
        ];
        let refs: Vec<&Value> = frames.iter().collect();
        let spans = compute_spans(&refs, Some("y"));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0]["frames"].as_u64().unwrap(), 3);
        assert_eq!(spans[0]["start_ms"].as_u64().unwrap(), 1000);
    }

    #[test]
    fn test_compute_spans_gap() {
        let frames: Vec<Value> = vec![
            json!({"t": 1000, "y": 50.0}),
            json!({"t": 1042, "y": 55.0}),
            // gap > 2x median (~42ms), so threshold ~84ms
            json!({"t": 5000, "y": 52.0}),
            json!({"t": 5042, "y": 48.0}),
        ];
        let refs: Vec<&Value> = frames.iter().collect();
        let spans = compute_spans(&refs, Some("y"));
        assert_eq!(spans.len(), 2, "should split into 2 spans at the gap");
    }
}
