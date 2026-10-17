use serde_json::Value;
use std::fmt::Write;

pub fn generate(dashboard: &Value, video_name: &str) -> String {
    let identity = dashboard.get("video_identity").unwrap_or(&Value::Null);
    let stats = dashboard.get("global_stats").unwrap_or(&Value::Null);
    let dur_sec = identity
        .get("duration_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as f64
        / 1000.0;
    let scene_count = identity
        .get("scene_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let resolution = identity
        .get("resolution")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let fps = identity.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let mut out = String::with_capacity(2048);
    let _ = writeln!(out, "# {video_name}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Duration {dur_sec:.1}s · {scene_count} scenes · {resolution} · {fps:.0}fps"
    );
    let _ = writeln!(out);

    render_at_a_glance(&mut out, stats);

    out
}

fn render_at_a_glance(out: &mut String, stats: &Value) {
    let _ = writeln!(out, "## At a glance");
    let _ = writeln!(out);

    if let Some(b) = stats.get("brightness").filter(|v| !v.is_null()) {
        let mean = b.get("mean").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let std = b.get("std").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let _ = writeln!(
            out,
            "- **Brightness**: avg {mean:.1} (std {std:.1}, 0–255 scale)"
        );
    }

    if let Some(m) = stats.get("motion").filter(|v| !v.is_null()) {
        if let Some(obj) = m.as_object() {
            let mut parts: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("{} {}", v.as_u64().unwrap_or(0), k))
                .collect();
            parts.sort();
            let _ = writeln!(out, "- **Motion**: {}", parts.join(", "));
        }
    }

    if let Some(p) = stats.get("pacing").filter(|v| !v.is_null()) {
        let median = p
            .get("median_scene_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let short = p.get("shortest_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let long = p.get("longest_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let _ = writeln!(
            out,
            "- **Pacing**: median scene {:.1}s (shortest {:.1}s, longest {:.1}s)",
            median as f64 / 1000.0,
            short as f64 / 1000.0,
            long as f64 / 1000.0,
        );
    }

    if let Some(b) = stats.get("blur").filter(|v| !v.is_null()) {
        let sharp = b.get("sharpness").and_then(|v| v.as_str()).unwrap_or("?");
        let lap = b
            .get("laplacian_mean")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let _ = writeln!(out, "- **Sharpness**: {sharp} (Laplacian mean {lap:.1})");
    }

    if let Some(a) = stats.get("audio").filter(|v| !v.is_null()) {
        let lufs = a.get("lufs").and_then(|v| v.as_f64());
        let peak = a.get("true_peak_dbfs").and_then(|v| v.as_f64());
        match (lufs, peak) {
            (Some(l), Some(p)) => {
                let _ = writeln!(out, "- **Audio**: {l:.1} LUFS, true peak {p:.1} dBFS");
            }
            (Some(l), None) => {
                let _ = writeln!(out, "- **Audio**: {l:.1} LUFS");
            }
            _ => {}
        }
    }

    if let Some(s) = stats.get("avsync").filter(|v| !v.is_null()) {
        let has_audio = s.get("has_audio").and_then(|v| v.as_bool()).unwrap_or(true);
        if has_audio {
            let offset = s.get("offset_ms").and_then(|v| v.as_i64()).unwrap_or(0);
            let corr = s.get("correlation").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let _ = writeln!(
                out,
                "- **A/V sync**: offset {offset:+}ms (correlation {corr:.2})"
            );
        }
    }

    if let Some(b) = stats.get("beats").filter(|v| !v.is_null()) {
        let bpm = b.get("bpm").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let density = b.get("density").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let _ = writeln!(out, "- **Beats**: {bpm:.0} BPM (density {density:.2})");
    }

    if let Some(f) = stats.get("flicker").filter(|v| !v.is_null()) {
        let flashes = f.get("flash_count").and_then(|v| v.as_i64()).unwrap_or(0);
        let compliant = f
            .get("bt1702_compliant")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let suffix = if compliant {
            "BT.1702 compliant"
        } else {
            "**BT.1702 non-compliant**"
        };
        let _ = writeln!(out, "- **Flicker**: {flashes} flashes ({suffix})");
    }

    if let Some(fr) = stats.get("framerate").filter(|v| !v.is_null()) {
        let container = fr
            .get("container_fps")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let content = fr
            .get("content_fps")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let dup = fr
            .get("duplicate_ratio")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let interlace = fr
            .get("interlace")
            .and_then(|v| v.as_str())
            .unwrap_or("progressive");
        let _ = writeln!(
            out,
            "- **Framerate**: container {container:.2}, content {content:.2}, duplicates {:.0}%, {interlace}",
            dup * 100.0
        );
    }

    if let Some(c) = stats.get("complexity").filter(|v| !v.is_null()) {
        let class = c.get("class").and_then(|v| v.as_str()).unwrap_or("?");
        let edge = c
            .get("edge_density")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let _ = writeln!(out, "- **Complexity**: {class} (edge density {edge:.3})");
    }

    if let Some(l) = stats.get("layout").filter(|v| !v.is_null()) {
        if let Some(t) = l.get("type").and_then(|v| v.as_str()) {
            if !t.is_empty() && t != "fullframe" {
                let _ = writeln!(out, "- **Layout**: {t}");
            }
        }
    }

    if let Some(c) = stats.get("color").filter(|v| !v.is_null()) {
        let palette: Vec<&str> = c
            .get("palette")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect())
            .unwrap_or_default();
        let sat = c.get("saturation").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if !palette.is_empty() {
            let _ = writeln!(
                out,
                "- **Color**: palette {} (saturation {sat:.2})",
                palette.join(" ")
            );
        }
    }
}
