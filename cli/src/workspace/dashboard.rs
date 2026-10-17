use serde_json::{Value, json};

/// Generate dashboard.json — the L0 overview layer.
///
/// Aggregate measurements only. Consumers interpret these measurements and
/// may drill into tracks via `grep --window` or `grep --field`.
#[allow(clippy::too_many_arguments)]
pub fn generate(
    metadata: &Value,
    scenes: &Value,
    joined_scenes: &[Value],
    audio: Option<&Value>,
    avsync: Option<&Value>,
    flicker: Option<&Value>,
    track_globals: Option<&Value>,
    blur: Option<&Value>,
    complexity: Option<&Value>,
    framerate: Option<&Value>,
    beats: Option<&Value>,
    layout: Option<&Value>,
    scene_stats: Option<&Value>,
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
    let scene_count = scenes
        .get("scenes")
        .and_then(|s| s.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let global_stats = compute_global_stats(
        joined_scenes,
        audio,
        avsync,
        track_globals,
        blur,
        complexity,
        framerate,
        flicker,
        beats,
        layout,
        scene_stats,
    );
    json!({
        "version": "1.0",
        "video_identity": {
            "video": video,
            "duration_ms": duration_ms,
            "resolution": format!("{width}x{height}"),
            "fps": fps,
            "scene_count": scene_count,
        },
        "global_stats": global_stats,
    })
}

#[allow(clippy::too_many_arguments)]
fn compute_global_stats(
    joined_scenes: &[Value],
    audio: Option<&Value>,
    avsync: Option<&Value>,
    track_globals: Option<&Value>,
    blur: Option<&Value>,
    complexity: Option<&Value>,
    framerate: Option<&Value>,
    flicker: Option<&Value>,
    beats: Option<&Value>,
    layout: Option<&Value>,
    scene_stats: Option<&Value>,
) -> Value {
    // ── Brightness ──
    let brightness = if let Some(tg) = track_globals {
        let mean = tg
            .get("brightness_mean")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let std = tg
            .get("brightness_std")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        json!({"mean": r1(mean), "std": r1(std), "source": "track"})
    } else {
        let mut sum = 0.0_f64;
        let mut sq_sum = 0.0_f64;
        let mut total = 0_u64;
        for scene in joined_scenes {
            let fc = scene
                .get("signalstats")
                .and_then(|s| s.get("frame_count"))
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            if let Some(y) = scene
                .get("signalstats")
                .and_then(|s| s.get("yavg_mean"))
                .and_then(|v| v.as_f64())
            {
                sum += y * fc as f64;
                sq_sum += y * y * fc as f64;
                total += fc;
            }
        }
        if total > 0 {
            let mean = sum / total as f64;
            let var = (sq_sum / total as f64) - mean * mean;
            json!({"mean": r1(mean), "std": r1(if var > 0.0 { var.sqrt() } else { 0.0 }), "source": "scene_avg"})
        } else {
            json!(null)
        }
    };

    // ── Motion ──
    let mut motion_classes: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for scene in joined_scenes {
        if let Some(mc) = scene
            .get("track")
            .and_then(|t| t.get("motion_class"))
            .and_then(|v| v.as_str())
        {
            *motion_classes.entry(mc.to_string()).or_insert(0) += 1;
        }
    }
    let motion = if motion_classes.is_empty() {
        json!(null)
    } else {
        json!(motion_classes)
    };

    // ── Pacing ──
    let mut durations: Vec<i64> = joined_scenes
        .iter()
        .filter_map(|s| s.get("duration_ms").and_then(|v| v.as_i64()))
        .collect();
    durations.sort();
    let pacing = if durations.is_empty() {
        json!(null)
    } else {
        let median = durations[durations.len() / 2];
        json!({
            "median_scene_ms": median,
            "shortest_ms": durations.first().copied().unwrap_or(0),
            "longest_ms": durations.last().copied().unwrap_or(0),
        })
    };

    // ── Audio ──
    let audio_stats = audio.map(|a| {
        json!({
            "lufs": a.get("input_i").and_then(|v| v.as_f64()),
            "true_peak_dbfs": a.get("input_tp").and_then(|v| v.as_f64()),
            "loudness_range_lu": a.get("input_lra").and_then(|v| v.as_f64()),
        })
    });

    // ── AV sync ──
    let avsync_stats = avsync.map(|a| {
        json!({
            "offset_ms": a.get("estimated_offset_ms").and_then(|v| v.as_i64()),
            "correlation": a.get("correlation_peak").and_then(|v| v.as_f64()),
            "has_audio": a.get("has_audio").and_then(|v| v.as_bool()),
        })
    });

    // ── Blur (avg across scenes) ──
    let blur_stats = blur.and_then(|b| {
        let arr = b.as_array()?;
        if arr.is_empty() {
            return None;
        }
        let laps: Vec<f64> = arr
            .iter()
            .filter_map(|s| s.get("laplacian_variance").and_then(|v| v.as_f64()))
            .collect();
        if laps.is_empty() {
            return None;
        }
        let mean = laps.iter().sum::<f64>() / laps.len() as f64;
        let classes: Vec<&str> = arr
            .iter()
            .filter_map(|s| s.get("sharpness_class").and_then(|v| v.as_str()))
            .collect();
        let sharpness = if classes.iter().all(|&c| c == "sharp") {
            "sharp"
        } else if classes.iter().any(|&c| c == "blurry") {
            "mixed"
        } else {
            "moderate"
        };
        Some(json!({"laplacian_mean": r1(mean), "sharpness": sharpness}))
    });

    // ── Complexity (avg across scenes) ──
    let complexity_stats = complexity.and_then(|c| {
        let arr = c.as_array()?;
        if arr.is_empty() {
            return None;
        }
        let edges: Vec<f64> = arr
            .iter()
            .filter_map(|s| s.get("edge_density").and_then(|v| v.as_f64()))
            .collect();
        if edges.is_empty() {
            return None;
        }
        let mean_edge = edges.iter().sum::<f64>() / edges.len() as f64;
        let class = arr
            .first()
            .and_then(|s| s.get("complexity_class").and_then(|v| v.as_str()))
            .unwrap_or("");
        Some(json!({"edge_density": r3(mean_edge), "class": class}))
    });

    // ── Framerate ──
    let framerate_stats = framerate.map(|fr| {
        let container = fr
            .get("container_fps")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let content = fr
            .get("content_fps_estimate")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let dup_ratio = fr
            .get("duplicate_ratio")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let interlace = fr
            .get("interlace_type")
            .and_then(|v| v.as_str())
            .unwrap_or("progressive");
        json!({
            "container_fps": container,
            "content_fps": content,
            "duplicate_ratio": r3(dup_ratio),
            "interlace": interlace,
        })
    });

    // ── Flicker ──
    let flicker_stats = flicker.map(|fl| {
        let flashes = fl.get("flash_count").and_then(|v| v.as_i64()).unwrap_or(0);
        let max_delta = fl
            .get("max_luminance_delta")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let compliant = fl
            .get("bt1702_compliant")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        json!({
            "flash_count": flashes,
            "max_delta": r1(max_delta),
            "bt1702_compliant": compliant,
        })
    });

    // ── Beats ──
    let beats_stats = beats.and_then(|b| {
        let arr = b.as_array()?;
        if arr.is_empty() {
            return None;
        }
        // Average BPM and density across scenes
        let bpms: Vec<f64> = arr
            .iter()
            .filter_map(|s| s.get("bpm").and_then(|v| v.as_f64()))
            .filter(|&v| v > 0.0)
            .collect();
        let densities: Vec<f64> = arr
            .iter()
            .filter_map(|s| s.get("density").and_then(|v| v.as_f64()))
            .collect();
        if bpms.is_empty() {
            return None;
        }
        let bpm = bpms.iter().sum::<f64>() / bpms.len() as f64;
        let density = if densities.is_empty() {
            0.0
        } else {
            densities.iter().sum::<f64>() / densities.len() as f64
        };
        Some(json!({"bpm": r1(bpm), "density": r2(density)}))
    });

    // ── Layout ──
    let layout_stats = layout.and_then(|l| {
        let arr = l.as_array()?;
        let first = arr.first()?;
        let lt = first.get("layout").and_then(|v| v.as_str()).unwrap_or("");
        Some(json!({"type": lt}))
    });

    // ── Color palette (from first scene stats) ──
    let color_stats = scene_stats.and_then(|ss| {
        let arr = ss.as_array()?;
        let first = arr.first()?;
        let palette: Vec<&str> = first
            .get("dominant_colors")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|c| c.as_str()).collect())
            .unwrap_or_default();
        if palette.is_empty() {
            return None;
        }
        let sat = first
            .get("saturation")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        Some(json!({"palette": palette, "saturation": r2(sat)}))
    });

    json!({
        "brightness": brightness,
        "motion": motion,
        "pacing": pacing,
        "audio": audio_stats,
        "avsync": avsync_stats,
        "blur": blur_stats,
        "complexity": complexity_stats,
        "framerate": framerate_stats,
        "flicker": flicker_stats,
        "beats": beats_stats,
        "layout": layout_stats,
        "color": color_stats,
    })
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}
