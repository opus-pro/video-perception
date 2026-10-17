use serde_json::Value;
use std::fs;
use std::path::Path;

/// Generate index.html for an existing Observation.
/// Two modes: Analysis (scene-by-scene breakdown) + Files (Finder-style explorer).
/// Data is embedded at generation time. Charts rendered client-side from raw signal data.
///
/// Note: This is a self-contained offline viewer for local use only.
/// All data is embedded at build time from the user's own sensor output.
/// The innerHTML usage is safe because all dynamic content is escaped via the esc() helper
/// and there is no external/untrusted input — the viewer runs offline from local files.
pub fn generate(workspace_dir: &Path, manifest: &Value, scenes_jsonl: &str) -> Result<(), String> {
    let asset = manifest.get("asset").unwrap_or(&Value::Null);
    let video = asset
        .get("uri")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let video_name = Path::new(video)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let duration_ms = asset
        .get("duration_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let scene_count = manifest
        .get("scene_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let width = asset
        .pointer("/resolution/width")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let height = asset
        .pointer("/resolution/height")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let resolution = format!("{width}x{height}");
    let fps = asset.get("fps").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let dur_sec = duration_ms as f64 / 1000.0;

    // Build keyframe list
    let keyframes_dir = workspace_dir.join("assets/keyframes");
    let mut keyframe_files: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&keyframes_dir) {
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        files.sort();
        keyframe_files = files;
    }

    // Load keyframes track manifest
    let keyframes_track_json =
        fs::read_to_string(workspace_dir.join("tracks/keyframes.track.jsonl"))
            .map(|content| {
                let items: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
                format!("[{}]", items.join(","))
            })
            .unwrap_or("[]".into());

    // Build sensor config from signals/ directory
    let sensor_config_json = build_sensor_config(workspace_dir);

    // Build file tree as JSON
    let file_tree_json = build_file_tree_json(workspace_dir);

    // Data arrays
    let scenes_array = jsonl_to_array(scenes_jsonl);

    let keyframes_json = serde_json::to_string(&keyframe_files).unwrap_or("[]".into());

    // Build HTML
    let mut html = String::with_capacity(128 * 1024);
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n");
    let video_name = esc_html(&video_name);
    html.push_str(&format!("<title>{video_name} — video-perception</title>\n"));
    html.push_str("<style>\n");
    html.push_str(STYLE);
    html.push_str("\n</style>\n</head>\n<body>\n");

    // Top bar
    html.push_str("<div class=\"topbar\">\n");
    html.push_str(&format!("<div class=\"topbar-left\"><span class=\"logo\">video-perception</span> <span class=\"vname\">{video_name}</span></div>\n"));
    html.push_str(&format!(
        "<div class=\"topbar-stats ib-anchor\">\
        <span>{dur_sec:.1}s</span><span>{scene_count} scenes</span><span>{resolution}</span><span>{fps:.0}fps</span>\
        <div class=\"info-bubble\" style=\"top:36px;left:0\">\
        <strong>Video metadata</strong> from probe sensor<br>\
        <strong>{dur_sec:.1}s</strong> \u{2014} total duration from container format<br>\
        <strong>{scene_count} scenes</strong> \u{2014} cut points detected by scenes sensor (ffmpeg scene filter + merging). This is one sensor's output, not the foundation for others.<br>\
        <strong>{resolution}</strong> \u{2014} frame dimensions (width x height)<br>\
        <strong>{fps:.0}fps</strong> \u{2014} source video frame rate\
        </div></div>\n"
    ));
    html.push_str("<div class=\"topbar-right\">\n");
    html.push_str(
        "<span class=\"info-hint\" id=\"infoHint\">Hover over any section to see details</span>\n",
    );
    html.push_str("<button class=\"mode-btn\" id=\"btnInfo\" onclick=\"toggleInfo()\" title=\"Toggle signal descriptions\">\u{2139}</button>\n");
    html.push_str("<button class=\"mode-btn active\" id=\"btnAnalysis\" onclick=\"setMode('analysis')\">Timeline</button>\n");
    html.push_str(
        "<button class=\"mode-btn\" id=\"btnFiles\" onclick=\"setMode('files')\">Files</button>\n",
    );
    html.push_str("</div></div>\n");
    html.push_str(&build_capability_status_html(manifest));

    // ═══ ANALYSIS MODE — Time-centric viewer ═══
    html.push_str("<div id=\"modeAnalysis\" class=\"mode\">\n");

    // Dashboard summary (aggregate measurements only)
    html.push_str("<div class=\"ib-anchor\">\n");
    html.push_str("<div id=\"dashboardPanel\" class=\"dashboard-panel\"></div>\n");
    html.push_str("<div class=\"info-bubble\" style=\"top:4px;right:20px\">\
    <strong>Dashboard</strong> \u{2014} global stats computed from raw sensor data across all frames.<br><br>\
    <strong>Brightness</strong>: weighted average \u{00b1} std of Y channel across all frames. Source: signalstats track or stats fallback.<br>\
    <strong>Loudness</strong>: integrated loudness per EBU R128. \u{2212}24 to \u{2212}14 LUFS = broadcast range. Source: loudnorm sensor.<br>\
    <strong>AV Sync</strong>: estimated audio/video offset via cross-correlation. Source: avsync sensor.<br>\
    <strong>Pacing</strong>: shortest\u{2013}longest scene duration range. Source: scenes sensor.<br>\
    <strong>Motion</strong>: distribution of motion classes across scenes. Source: track sensor.<br>\
    Interpretation, product thresholds, and verdicts are intentionally left to the consuming agent service.\
    </div></div>\n");

    // Sensor config panel (sampling rates, which sensors ran)
    html.push_str("<div class=\"ib-anchor\">\n");
    html.push_str("<div id=\"sensorConfigPanel\" class=\"sensor-config-panel\"></div>\n");
    html.push_str("<div class=\"info-bubble\" style=\"top:4px;left:20px\">\
    <strong>Sensor inventory</strong> \u{2014} every sensor that ran on this video, with its output type and sampling config.<br><br>\
    <strong>Signals</strong> (global): one JSON file per sensor. Contains aggregated or event-based data (e.g., scene cuts, beats, loudness).<br>\
    <strong>Tracks</strong> (time-series): per-frame or fixed-interval JSONL files. Each line has a <code>t</code> (ms) timestamp. These are the raw measurements.<br>\
    <strong>Keyframes</strong> (frames): PNG images extracted at 2fps. Scene-independent. Foundation for visual analysis sensors.<br><br>\
    The number after each name is its file size or frame count. fps = sampling rate. ms = interval between samples.\
    </div></div>\n");

    // Layer navigation
    html.push_str("<div class=\"zoom-bar ib-anchor\" id=\"zoomBar\">\n");
    html.push_str("<div class=\"layer-btns\">\n");
    html.push_str("<button class=\"layer-btn active\" id=\"btnL0\" onclick=\"setLayer('overview')\">Overview</button>\n");
    html.push_str("<button class=\"layer-btn\" id=\"btnL1\" onclick=\"drillTo('5s', selectedMs>=0?selectedMs:(viewStart+viewEnd)/2)\">5s Detail</button>\n");
    html.push_str("<button class=\"layer-btn\" id=\"btnL2\" onclick=\"drillTo('1s', selectedMs>=0?selectedMs:(viewStart+viewEnd)/2)\">1s Detail</button>\n");
    html.push_str("<button class=\"layer-btn\" id=\"btnL3\" onclick=\"drillTo('frame', selectedMs>=0?selectedMs:(viewStart+viewEnd)/2)\">Per-frame</button>\n");
    html.push_str("</div>\n");
    html.push_str("<button class=\"mode-btn\" id=\"btnBack\" onclick=\"layerBack()\" style=\"display:none\">\u{2190} Back</button>\n");
    html.push_str("<span class=\"zoom-info\" id=\"zoomInfo\"></span>\n");
    html.push_str("<div class=\"info-bubble\" style=\"top:36px;left:0\">\
    <strong>Zoom levels</strong> \u{2014} 4 levels of temporal resolution:<br><br>\
    <strong>Overview</strong>: full video. Tracks bucketed into 5s windows (mean/min/max). Best for spotting overall patterns.<br>\
    <strong>5s Detail</strong>: 15s window. Tracks bucketed into 5s windows. Click a point or use \u{2193} arrow to zoom in.<br>\
    <strong>1s Detail</strong>: 5s window. Tracks bucketed into 1s windows. Individual keyframes visible.<br>\
    <strong>Per-frame</strong>: 1.5s window. Every raw frame value plotted. Maximum detail.<br><br>\
    <strong>Navigation:</strong><br>\
    \u{2022} <strong>Double-click</strong> timeline to zoom into that point<br>\
    \u{2022} <strong>Swipe left/right</strong> (trackpad) to pan in time<br>\
    \u{2022} <strong>\u{2190}\u{2192}</strong> arrow keys to pan, <strong>\u{2191}\u{2193}</strong> to zoom in/out<br>\
    \u{2022} <strong>Esc</strong> to reset to full overview<br>\
    \u{2022} <strong>Click</strong> a keyframe thumbnail to jump to that moment\
    </div>\n");
    html.push_str("</div>\n");

    // Timeline lanes — Chart.js canvases
    html.push_str("<div class=\"timeline\" id=\"timeline\">\n");

    // Keyframes observation strip (scene-independent, fixed-interval)
    html.push_str("<div class=\"tl-lane tl-lane-keyframes\" id=\"laneKeyframes\"><div class=\"tl-label\">Keyframes<br><span class=\"tl-range\" id=\"rangeKeyframes\">2fps</span><div class=\"info-bubble\"><strong>Keyframes</strong><br>Fixed-interval frame extraction at 2fps (500ms). Scene-independent. Used by blur, layout, complexity sensors.</div></div><div class=\"tl-chart\" id=\"keyframeStrip\"></div></div>\n");

    let lane_ids = [
        "Luma", "Sat", "Range", "Diff", "Motion", "Sharp", "Static", "Audio", "Color", "Scenes",
    ];
    let lane_labels = [
        "Luminance",
        "Saturation",
        "Contrast",
        "Frame Diff",
        "Motion",
        "Sharpness",
        "Static Ratio",
        "Audio",
        "Color",
        "Scenes",
    ];
    let lane_info = [
        "Per-frame Y channel mean brightness. <span class=\"ib-unit\">0\u{2013}255 IRE</span>. Source: ffmpeg signalstats YAVG.",
        "Per-frame color saturation. <span class=\"ib-unit\">0\u{2013}128</span>. Source: ffmpeg signalstats SATAVG.",
        "Per-frame dynamic range (YMAX \u{2212} YMIN). <span class=\"ib-unit\">0\u{2013}255</span>. High = high contrast.",
        "Absolute luminance difference between consecutive frames. Spikes = cuts or flashes.",
        "Per-scene motion intensity from optical flow analysis. <span class=\"ib-unit\">0\u{2013}30</span>. Source: track sensor.",
        "Per-scene Laplacian variance of keyframe. <span class=\"ib-unit\">0\u{2013}8000</span>. Low (&lt;50) = blurry. Source: blur sensor.",
        "Ratio of frames with near-zero inter-frame diff. <span class=\"ib-unit\">0\u{2013}1</span>. High (&gt;0.75) = freeze frame. Source: signalstats.",
        "Per-scene integrated loudness. <span class=\"ib-unit\">LUFS (EBU R128)</span>. \u{2212}24 to \u{2212}14 = broadcast range. Source: scene-audio sensor.",
        "Dominant colors per frame sampled at 0.5s intervals. Top 5 colors via ImageMagick k-means quantization. Scene-independent.",
        "Scene boundaries detected by ffmpeg scene filter with short-scene merging. One sensor among 22\u{2014}not the foundation for other signals.",
    ];
    for i in 0..lane_ids.len() {
        let lid = lane_ids[i];
        let ll = lane_labels[i];
        let info = lane_info[i];
        if lid == "Color" || lid == "Scenes" {
            html.push_str(&format!("<div class=\"tl-lane\" id=\"lane{lid}\"><div class=\"tl-label\">{ll}<div class=\"info-bubble\"><strong>{ll}</strong><br>{info}</div></div><div class=\"tl-chart\"><svg></svg></div></div>\n"));
        } else {
            html.push_str(&format!("<div class=\"tl-lane\" id=\"lane{lid}\"><div class=\"tl-label\">{ll}<br><span class=\"tl-range\" id=\"range{lid}\"></span><div class=\"info-bubble\"><strong>{ll}</strong><br>{info}</div></div><div class=\"tl-chart\"><canvas id=\"chart{lid}\"></canvas></div></div>\n"));
        }
    }
    html.push_str("<div class=\"tl-brush\" id=\"brush\" style=\"display:none\"></div>\n");
    html.push_str("<div class=\"info-bubble tl-info-bubble\" id=\"timelineInfoBubble\">\
    <strong>Timeline interaction</strong><br><br>\
    \u{2022} <strong>Hover</strong> \u{2014} readout bar shows values at cursor position<br>\
    \u{2022} <strong>Click</strong> \u{2014} select a point, detail panel below shows frame-level data + video player syncs to that moment<br>\
    \u{2022} <strong>Double-click</strong> \u{2014} zoom into that point (next detail level)<br>\
    \u{2022} <strong>Swipe left/right</strong> (trackpad) \u{2014} pan through time<br>\
    \u{2022} <strong>\u{2190}\u{2192} arrows</strong> \u{2014} pan, <strong>\u{2191}\u{2193} arrows</strong> \u{2014} zoom in/out<br>\
    \u{2022} <strong>Esc</strong> \u{2014} reset to full overview<br><br>\
    Each row is a different signal. The shaded band shows min/max range, the line shows the mean. At deeper zoom levels, individual data points become visible.\
    </div>\n");
    html.push_str("</div>\n"); // timeline

    // Readout bar
    html.push_str(
        "<div class=\"tl-readout ib-anchor\" id=\"readout\">Hover over the timeline to inspect\
    <div class=\"info-bubble\" style=\"bottom:28px;left:16px\">\
    <strong>Readout bar</strong> \u{2014} shows values at the mouse cursor position.<br>\
    Displays: timestamp, frame-level luminance/saturation/diff, and which scene the cursor is in.\
    </div></div>\n",
    );

    // Detail panel — video player + signal detail
    html.push_str("<div class=\"detail-panel ib-anchor\" id=\"detailPanel\">\n");
    html.push_str("<div class=\"info-bubble\" style=\"top:4px;left:50%;transform:translateX(-50%)\">\
    <strong>Detail panel</strong> \u{2014} appears when you click on the timeline.<br><br>\
    <strong>Left</strong>: video player synced to the clicked timestamp. Scrub the timeline to update.<br>\
    <strong>Center</strong>: gauge bars showing frame-level and scene-level signal values at this moment. Red bars indicate values outside typical ranges.<br>\
    <strong>Right</strong>: raw JSON of the scene record containing all joined sensor data. This is the full data the agent sees.\
    </div>\n");
    // A render does not imply copying source media. Show a clear placeholder
    // unless --include-media placed the asset in this Observation.
    let video_filename = Path::new(video)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    if workspace_dir
        .join("assets")
        .join(video_filename.as_ref())
        .is_file()
    {
        html.push_str(&format!("<div class=\"detail-left\" id=\"detailVideo\"><video id=\"videoPlayer\" src=\"assets/{}\" controls muted playsinline style=\"max-width:100%;max-height:100%;border-radius:4px\"></video></div>\n",
            esc_html(&video_filename)));
    } else {
        html.push_str("<div class=\"detail-left\" id=\"detailVideo\"><div class=\"dossier-empty\">Source media was not copied. Re-render with <code>--include-media</code> to enable playback.</div></div>\n");
    }
    html.push_str("<div class=\"detail-center\" id=\"detailSignals\"><div class=\"dossier-empty\">Click on the timeline to inspect a point</div></div>\n");
    html.push_str("<div class=\"detail-right\" id=\"detailRaw\"></div>\n");
    html.push_str("</div>\n");

    html.push_str("</div>\n"); // modeAnalysis

    // ═══ FILES MODE ═══
    html.push_str("<div id=\"modeFiles\" class=\"mode\" style=\"display:none\">\n");
    html.push_str("<div class=\"finder\">\n");
    html.push_str("<div class=\"finder-tree\" id=\"finderTree\"></div>\n");
    html.push_str("<div class=\"finder-list\" id=\"finderList\"></div>\n");
    html.push_str("<div class=\"finder-preview\" id=\"finderPreview\"><div class=\"finder-empty\">Select a file to preview</div></div>\n");
    html.push_str("</div>\n</div>\n");

    // Load track data for time-series rendering
    let track_json = fs::read_to_string(workspace_dir.join("tracks/signalstats.track.jsonl"))
        .map(|content| {
            let items: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
            format!("[{}]", items.join(","))
        })
        .unwrap_or("[]".into());

    let color_track_json = fs::read_to_string(workspace_dir.join("tracks/color.track.jsonl"))
        .map(|content| {
            let items: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
            format!("[{}]", items.join(","))
        })
        .unwrap_or("[]".into());

    // Chart.js library (embedded for offline use)
    html.push_str("<script>\n");
    html.push_str(include_str!("chartjs.min.js"));
    html.push_str("\n</script>\n");

    // Embedded data. Every blob carries text from untrusted media (transcript
    // utterances, file names), so it is escaped for the script context first.
    html.push_str("<script>\n");
    let dashboard_json =
        fs::read_to_string(workspace_dir.join("dashboard.json")).unwrap_or("null".into());
    for (name, json) in [
        ("DASHBOARD", dashboard_json.as_str()),
        ("TRACK", track_json.as_str()),
        ("COLOR_TRACK", color_track_json.as_str()),
        ("SCENES", scenes_array.as_str()),
        ("KEYFRAMES", keyframes_json.as_str()),
        ("KEYFRAMES_TRACK", keyframes_track_json.as_str()),
        ("SENSOR_CONFIG", sensor_config_json.as_str()),
        ("FILE_TREE", file_tree_json.as_str()),
    ] {
        html.push_str(&format!("const {name}={};\n", script_json(json)));
    }
    html.push_str(SCRIPT);
    html.push_str("\n</script>\n</body>\n</html>\n");

    fs::write(workspace_dir.join("index.html"), &html)
        .map_err(|e| format!("write index.html: {e}"))?;

    eprintln!("  \u{2713} index.html generated");
    Ok(())
}

fn jsonl_to_array(jsonl: &str) -> String {
    let items: Vec<&str> = jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    format!("[{}]", items.join(","))
}

/// Make JSON text safe to inline inside a `<script>` element. `<` is the
/// same string value as `<`, and `<`, `>`, `&` cannot occur outside strings in
/// valid JSON, so the data is unchanged while `</script>` and comment openers
/// can no longer appear in the page source. U+2028 and U+2029 are escaped
/// because they terminate a JavaScript line.
fn script_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for ch in json.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(ch),
        }
    }
    out
}

fn build_file_tree_json(workspace_dir: &Path) -> String {
    let tree = walk_dir_json(workspace_dir, "observation", 0);
    serde_json::to_string(&tree).unwrap_or("[]".into())
}

fn walk_dir_json(dir: &Path, name: &str, depth: usize) -> Value {
    if depth > 4 {
        return serde_json::json!({"name": name, "type": "dir", "children": []});
    }

    let mut children = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in &entries {
            let fname = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                children.push(walk_dir_json(&entry.path(), &fname, depth + 1));
            } else {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let ext = Path::new(&fname)
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                let previewable = matches!(ext.as_str(), "json" | "jsonl" | "md" | "svg" | "txt");
                let content = if previewable && size < 500_000 {
                    fs::read_to_string(entry.path()).ok()
                } else {
                    None
                };

                let mut node = serde_json::json!({
                    "name": fname,
                    "type": "file",
                    "size": size,
                    "ext": ext,
                });
                if let Some(c) = content {
                    node.as_object_mut()
                        .unwrap()
                        .insert("content".into(), Value::String(c));
                }
                children.push(node);
            }
        }
    }

    serde_json::json!({
        "name": name,
        "type": "dir",
        "children": children,
    })
}

// ═══════════════════════════════════════════════════════════════
// CSS
// ═══════════════════════════════════════════════════════════════

const STYLE: &str = r##"
@import url('https://fonts.googleapis.com/css2?family=Inconsolata:wght@400;500;600;700&family=Inter:wght@400;500;600;700;900&display=swap');
*{margin:0;padding:0;box-sizing:border-box}
:root{--neon:#c8cc6e;--neon-bright:#e2e57a;--neon-dim:#3a3c1a;--red:#d4453e;--red-bg:rgba(212,69,62,0.08);--bg:#080808;--surface:#111;--border:rgba(60,60,60,0.6);--border-hard:#2a2a2a;--text:#d4d4d4;--text-sec:#999;--text-dim:#6a6a6a;--mono:'Inconsolata',monospace;--sans:'Inter',system-ui,sans-serif}
body{font-family:var(--sans);background:var(--bg);color:var(--text);font-size:14px;line-height:1.5;overflow-x:hidden;overflow-y:auto;min-height:100vh}
.topbar{display:flex;align-items:center;justify-content:space-between;height:48px;padding:0 20px;background:var(--bg);border-bottom:1px solid var(--border);flex-shrink:0;position:sticky;top:0;z-index:300}
.topbar-left{display:flex;align-items:center;gap:12px}
.logo{font-family:var(--mono);font-weight:500;font-size:13px;color:var(--text-sec);letter-spacing:1px}
.vname{color:var(--text-sec);font-size:13px;max-width:300px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.topbar-stats{display:flex;gap:20px;font-family:var(--mono);font-size:12px;color:var(--text-sec)}
.topbar-stats span{white-space:nowrap}
.topbar-right{display:flex;align-items:center;gap:8px}
#searchBox{background:var(--surface);border:1px solid var(--border);border-radius:4px;padding:6px 12px;color:var(--text-sec);font-family:var(--mono);font-size:12px;width:220px;outline:none}
#searchBox:focus{border-color:var(--neon)}
.mode-btn{background:var(--surface);border:1px solid var(--border);color:var(--text-sec);padding:6px 16px;border-radius:4px;cursor:pointer;font-family:var(--mono);font-size:12px;font-weight:500}
.mode-btn:hover{background:#3a3a3a}
.mode-btn.active{background:var(--surface);border-color:var(--text-sec);color:var(--text)}
.observation-status{display:flex;align-items:flex-start;gap:14px;padding:10px 20px;background:rgba(200,204,110,0.06);border-bottom:1px solid var(--neon-dim);font-size:12px}
.observation-status.partial{background:var(--red-bg);border-bottom-color:rgba(212,69,62,0.35)}
.status-title{font-family:var(--mono);font-weight:700;color:var(--neon-bright);white-space:nowrap}
.observation-status.partial .status-title{color:#ef7770}
.status-body{display:flex;flex-direction:column;gap:3px;min-width:0}
.status-summary{font-family:var(--mono);color:var(--text)}
.status-detail{color:var(--text-sec);overflow-wrap:anywhere}
.status-detail code{font-family:var(--mono);color:inherit}
.status-note{color:var(--text-dim)}
.dashboard-panel{padding:10px 16px;background:var(--bg);border-bottom:1px solid var(--border)}
.dash-stats{display:flex;gap:12px;flex-wrap:wrap}
.dash-stat{background:var(--surface);border:1px solid var(--border);border-radius:4px;padding:6px 10px;font-size:12px}
.dash-label{font-family:var(--mono);font-size:11px;color:var(--text-dim);text-transform:uppercase;letter-spacing:0.5px;display:block;margin-bottom:2px}
.dash-val{font-family:var(--mono);color:var(--text);font-weight:600}
.zoom-bar{display:flex;align-items:center;gap:12px;padding:4px 16px;background:var(--bg);border-bottom:1px solid var(--border);min-height:32px}
.zoom-info{font-family:var(--mono);font-size:11px;color:var(--text-sec);margin-left:auto}
.layer-btns{display:flex;gap:2px}
.layer-btn{background:var(--surface);border:1px solid var(--border);color:var(--text-dim);padding:4px 14px;cursor:pointer;font-family:var(--mono);font-size:11px;font-weight:500;border-radius:0}
.layer-btn:first-child{border-radius:4px 0 0 4px}
.layer-btn:last-child{border-radius:0 4px 4px 0}
.layer-btn:hover{background:#2a2a2a;color:var(--text-sec)}
.layer-btn.active{background:var(--surface);border-color:var(--neon-dim);color:var(--neon);font-weight:600}
.layer-btn.disabled{opacity:0.3;cursor:default}
.timeline{background:var(--bg);position:relative;cursor:crosshair}
.tl-lane{display:grid;grid-template-columns:90px 1fr;align-items:stretch;border-bottom:1px solid var(--border);height:80px}
.tl-label{font-family:var(--mono);font-size:11px;color:var(--text-dim);text-transform:uppercase;letter-spacing:0.5px;padding:6px 10px;display:flex;flex-direction:column;justify-content:center;border-right:1px solid var(--border);background:var(--bg);line-height:1.4}
.tl-range{font-size:10px;color:var(--text-dim);letter-spacing:0;text-transform:none}
.tl-chart{overflow:hidden;background:var(--surface);position:relative}
.tl-chart svg{width:100%;height:100%;display:block}
.tl-chart svg polyline{vector-effect:non-scaling-stroke}
.tl-chart svg .area{opacity:0.12}
.tl-chart svg .scene-marker{stroke:rgba(80,80,80,0.4);stroke-width:1;stroke-dasharray:3,4;vector-effect:non-scaling-stroke}
.tl-chart svg .scene-block{opacity:0.7}
.tl-axis{display:grid;grid-template-columns:90px 1fr;height:22px;border-bottom:1px solid var(--border)}
.tl-brush{position:absolute;top:0;height:100%;background:rgba(200,204,110,0.12);border-left:1px solid var(--neon);border-right:1px solid var(--neon);pointer-events:none;z-index:10}
.tl-readout{font-family:var(--mono);font-size:11px;color:var(--text-sec);padding:6px 16px;min-height:24px;background:var(--bg);border-bottom:1px solid var(--border)}
.tl-lane-keyframes{height:80px}
.tl-lane-keyframes .tl-chart{display:flex;align-items:stretch;overflow:hidden;gap:0}
.kf-thumb{height:100%;object-fit:contain;background:#000;image-rendering:auto;border-right:1px solid rgba(30,30,30,0.8);flex-shrink:0;cursor:pointer;transition:opacity 0.15s}
.kf-thumb:hover{opacity:0.8}
.sensor-config-panel{display:flex;gap:16px;padding:8px 20px;background:var(--bg);border-bottom:1px solid var(--border);overflow-x:auto;flex-shrink:0}
.sensor-chip{font-family:var(--mono);font-size:11px;padding:4px 10px;border-radius:4px;background:var(--surface);color:var(--text-sec);border:1px solid var(--border);white-space:nowrap;display:flex;align-items:center;gap:6px}
.sensor-chip .sc-dot{width:6px;height:6px;border-radius:50%;background:var(--neon);flex-shrink:0}
.sensor-chip .sc-dim{color:var(--text-sec);opacity:0.7}
.info-hint{display:none;font-family:var(--mono);font-size:11px;color:var(--neon);letter-spacing:0.3px;animation:info-fade 2s ease-in-out infinite}
.info-active .info-hint{display:inline}
@keyframes info-fade{0%,100%{opacity:0.6}50%{opacity:1}}
.info-active #btnInfo{background:var(--neon);border-color:var(--neon);color:#080808;font-weight:700}
.ib-anchor{position:relative}
.info-active .ib-anchor,.info-active .tl-label{outline:none;border-image:none;background-image:repeating-linear-gradient(90deg,var(--neon) 0 4px,transparent 4px 8px),repeating-linear-gradient(0deg,var(--neon) 0 4px,transparent 4px 8px),repeating-linear-gradient(90deg,var(--neon) 0 4px,transparent 4px 8px),repeating-linear-gradient(0deg,var(--neon) 0 4px,transparent 4px 8px);background-size:100% 1px,1px 100%,100% 1px,1px 100%;background-position:0 0,100% 0,0 100%,0 0;background-repeat:no-repeat;animation:info-march 0.6s linear infinite}
@keyframes info-march{to{background-position:8px 0,100% 8px,calc(100% - 8px) 100%,0 calc(100% - 8px)}}
.info-bubble{display:none;position:absolute;z-index:200;background:#141810;border:1px solid var(--neon);border-radius:8px;padding:10px 14px;font-family:var(--sans);font-size:12px;color:#ccc;line-height:1.65;max-width:360px;pointer-events:none;box-shadow:0 6px 24px rgba(0,0,0,0.7);text-transform:none;letter-spacing:0}
.info-active .ib-anchor:hover>.info-bubble,.info-active .tl-label:hover>.info-bubble{display:block}
.info-bubble strong{color:var(--neon-bright);font-weight:600}
.info-bubble em{color:#888;font-style:italic}
.info-bubble code{background:rgba(200,204,110,0.1);border:1px solid rgba(200,204,110,0.2);border-radius:3px;padding:0 4px;font-size:11px;font-family:var(--mono);color:var(--neon)}
.info-bubble .ib-unit{color:#aaa}
.tl-info-bubble{display:none;position:absolute;top:50%;left:50%;transform:translate(-50%,-50%);max-width:400px}
.info-active .timeline:hover .tl-info-bubble{display:block}
.detail-panel{display:grid;grid-template-columns:220px 1fr 280px;height:220px;overflow:hidden;background:var(--bg)}
.detail-left{border-right:1px solid var(--border);padding:8px;overflow:hidden;display:flex;align-items:center;justify-content:center;background:#000}
.detail-center{overflow-y:auto;padding:12px 16px}
.detail-right{border-left:1px solid var(--border);overflow-y:auto;padding:12px;font-family:var(--mono);font-size:10px}
.col-nav{overflow-y:auto;border-right:1px solid var(--border);background:var(--bg)}
.scene-item{display:grid;grid-template-columns:56px 1fr;gap:10px;padding:8px 10px;cursor:pointer;border-bottom:1px solid var(--border);align-items:center}
.scene-item:hover{background:var(--surface)}
.scene-item.active{background:var(--surface);border-left:2px solid var(--text-sec)}
.scene-thumb{width:56px;height:32px;object-fit:cover;border-radius:2px;background:#222}
.scene-meta{overflow:hidden}
.scene-id{font-family:var(--mono);font-weight:600;font-size:12px;color:var(--text-sec)}
.scene-time{font-family:var(--mono);font-size:10px;color:var(--text-dim)}
.scene-chips{display:flex;flex-wrap:wrap;gap:3px;margin-top:3px}
.col-dossier{overflow-y:auto;padding:20px 24px;background:var(--bg)}
.dossier-empty,.insp-empty,.finder-empty{color:var(--text-dim);font-family:var(--mono);font-size:13px;padding:40px 20px;text-align:center}
.dossier-header{display:flex;gap:16px;margin-bottom:16px}
.dossier-keyframe{width:200px;border-radius:4px;background:#222;border:1px solid var(--border)}
.dossier-info{flex:1}
.dossier-title{font-family:var(--mono);font-size:16px;font-weight:600;margin-bottom:2px;color:var(--text)}
.dossier-subtitle{font-family:var(--mono);font-size:12px;color:var(--text-dim);margin-bottom:6px}
.dossier-tags{display:flex;gap:6px;margin-top:6px}
.dtag{font-family:var(--mono);font-size:10px;padding:2px 8px;border-radius:4px;background:var(--surface);color:var(--text-sec);border:1px solid var(--border);font-weight:500}
.signal-group{margin-top:20px}
.signal-group h3{font-family:var(--mono);font-size:10px;text-transform:uppercase;letter-spacing:1.4px;color:var(--text-dim);margin-bottom:8px;padding-bottom:6px;border-bottom:1px solid var(--border);font-weight:400}
.signal-row{display:grid;grid-template-columns:140px 1fr;gap:8px;padding:4px 0;font-size:13px;border-bottom:1px solid rgba(65,65,65,0.3)}
.signal-field{color:var(--text-dim);font-family:var(--mono);font-size:12px}
.signal-value{color:var(--text);font-family:var(--mono);font-size:12px}
.gauge-panel{margin:16px 0;border:1px solid var(--border);border-radius:4px;background:var(--bg);padding:12px 14px}
.gauge-header{font-family:var(--mono);font-size:10px;letter-spacing:1.4px;padding:4px 0 8px;border-bottom:1px solid var(--border);margin-bottom:8px;font-weight:500;color:var(--text-dim)}
.gauge-header.warn{color:var(--red)}
.gauge-sec{font-family:var(--mono);font-size:9px;letter-spacing:1.4px;color:var(--text-dim);margin:10px 0 4px;padding-top:6px;border-top:1px solid var(--border);font-weight:500}
.gauge-sec:first-child{border-top:none;margin-top:0}
.gauge{display:grid;grid-template-columns:100px 1fr 72px;align-items:center;gap:8px;padding:3px 0;font-family:var(--mono);font-size:12px}
.gauge.warn .g-label{color:var(--red)}
.gauge.warn .g-fill{background:var(--red)}
.gauge.warn .g-val{color:var(--red)}
.g-label{color:var(--text-dim);font-size:10px;letter-spacing:0.3px;text-align:right;font-weight:400}
.g-track{position:relative;height:3px;background:#1a1a1a;border-radius:0;overflow:hidden}
.g-fill{position:absolute;top:0;left:0;height:100%;background:var(--text-dim);transition:width 0.3s}
.g-zone-lo{position:absolute;top:0;left:0;height:100%;background:var(--red-bg);pointer-events:none}
.g-zone-hi{position:absolute;top:0;height:100%;background:var(--red-bg);pointer-events:none}
.g-val{color:var(--text-sec);text-align:right;font-size:11px;font-weight:400}
.g-unit{color:var(--text-dim);margin-left:2px;font-size:9px}
.content-line{font-size:13px;color:var(--text-sec);padding:4px 0;border-bottom:1px solid rgba(65,65,65,0.3)}
.content-line.flagged{color:var(--red)}
.flag-inline{font-family:var(--mono);font-size:10px;color:var(--red);background:var(--red-bg);padding:1px 6px;border-radius:4px;border:1px solid rgba(239,68,68,0.3);font-weight:500}
.sensor-badge{font-family:var(--mono);font-size:9px;padding:2px 6px;border-radius:4px;font-weight:600;letter-spacing:0.5px;vertical-align:middle}
.sensor-badge.vlm{background:var(--neon-dim);color:var(--neon);border:1px solid var(--neon-dim)}
.insp-transcript{font-size:13px;color:var(--text-sec);padding:6px 10px;background:var(--surface);border-radius:4px;margin-bottom:4px;border:1px solid var(--border)}
.transcript-time{font-family:var(--mono);font-size:10px;color:var(--text-dim);margin-right:8px}
.tag{font-family:var(--mono);font-size:9px;padding:2px 8px;border-radius:4px;background:var(--surface);color:var(--text-sec);white-space:nowrap;border:1px solid var(--border);font-weight:500}
.col-inspector{overflow-y:auto;border-left:1px solid var(--border);background:var(--bg);padding:16px}
.insp-section{margin-bottom:20px}
.insp-section h4{font-family:var(--mono);font-size:11px;text-transform:uppercase;letter-spacing:1.4px;color:var(--text-dim);margin-bottom:8px;font-weight:600}
.finder{display:grid;grid-template-columns:220px 280px 1fr;height:calc(100vh - 48px);overflow:hidden}
.finder-tree{overflow-y:auto;border-right:1px solid var(--border);background:var(--bg);padding:8px}
.finder-list{overflow-y:auto;border-right:1px solid var(--border);background:var(--surface);padding:4px}
.finder-preview{overflow-y:auto;background:var(--bg);padding:20px}
.tree-item{padding:4px 10px;cursor:pointer;border-radius:4px;font-family:var(--mono);font-size:12px;color:var(--text-sec);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.tree-item:hover{background:var(--surface)}
.tree-item.active{background:var(--surface);color:var(--text)}
.file-item{display:flex;justify-content:space-between;padding:6px 10px;cursor:pointer;border-radius:4px;font-family:var(--mono);font-size:12px}
.file-item:hover{background:#3a3a3a}
.file-item.active{background:var(--surface);color:var(--text)}
.file-name{color:var(--text-sec);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.file-size{color:var(--text-dim);font-size:11px;flex-shrink:0;margin-left:8px}
.preview-header{font-family:var(--mono);font-size:13px;font-weight:600;color:var(--text);padding:0 0 10px;margin-bottom:10px;border-bottom:1px solid var(--border);display:flex;align-items:center;justify-content:space-between}
.preview-size{font-weight:400;font-size:11px;color:var(--text-dim)}
.preview-img{max-width:100%;border-radius:4px;margin-top:4px;border:1px solid var(--border)}
.preview-code{font-family:var(--mono);font-size:12px;color:var(--text-sec);white-space:pre-wrap;word-break:break-all;max-height:calc(100vh - 120px);overflow-y:auto;background:var(--surface);padding:16px;border-radius:4px;border:1px solid var(--border);line-height:1.6;tab-size:2}
.preview-svg{width:100%;background:var(--surface);border-radius:4px;padding:8px;border:1px solid var(--border)}
.preview-svg svg{width:100%;height:auto}
.preview-md{max-height:calc(100vh - 120px);overflow-y:auto;padding:4px}
.hl-key{color:#8a8d5e}
.hl-str{color:#6b9e7a}
.hl-num{color:var(--text-sec)}
.hl-kw{color:var(--text-dim)}
.md-h1{font-family:var(--sans);font-size:20px;font-weight:700;color:var(--text);margin:16px 0 8px;padding-bottom:6px;border-bottom:1px solid var(--border)}
.md-h2{font-family:var(--sans);font-size:16px;font-weight:600;color:var(--text-sec);margin:12px 0 6px}
.md-h3{font-family:var(--sans);font-size:14px;font-weight:600;color:var(--text-sec);margin:10px 0 4px}
.md-p{color:var(--text-sec);margin:4px 0}
.md-li{color:var(--text-sec);padding-left:14px;margin:3px 0}
.md-code{font-family:var(--mono);font-size:12px;color:var(--text-sec);background:var(--surface);border:1px solid var(--border);border-radius:4px;padding:10px;margin:8px 0;overflow-x:auto}
.md-inline{font-family:var(--mono);font-size:12px;background:var(--surface);border:1px solid var(--border);border-radius:4px;padding:1px 5px;color:var(--text)}
::-webkit-scrollbar{width:6px;height:6px}
::-webkit-scrollbar-track{background:transparent}
::-webkit-scrollbar-thumb{background:var(--border-hard);border-radius:0}
::-webkit-scrollbar-thumb:hover{background:var(--text-dim)}
"##;

// ═══════════════════════════════════════════════════════════════
// JavaScript — self-contained offline viewer for local sensor data.
// All dynamic content is escaped via esc() before DOM insertion.
// No external/untrusted input; runs offline from local files only.
// ═══════════════════════════════════════════════════════════════

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn build_capability_status_html(manifest: &Value) -> String {
    let summary = super::summarize_capability_status(manifest);
    let issue_count =
        summary.optional_failed.len() + summary.required_failed.len() + summary.incomplete.len();
    let is_partial = issue_count > 0;
    let title = if is_partial {
        "Partial observation"
    } else {
        "Complete observation"
    };
    let class = if is_partial {
        "observation-status partial"
    } else {
        "observation-status"
    };
    let mut counts = vec![format!("{} succeeded", summary.succeeded)];
    if !summary.optional_failed.is_empty() {
        counts.push(format!("{} optional failed", summary.optional_failed.len()));
    }
    if !summary.required_failed.is_empty() {
        counts.push(format!("{} required failed", summary.required_failed.len()));
    }
    if !summary.incomplete.is_empty() {
        counts.push(format!("{} incomplete", summary.incomplete.len()));
    }

    let mut html = format!(
        "<div class=\"{class}\"><strong class=\"status-title\">{title}</strong><div class=\"status-body\"><span class=\"status-summary\">{}</span>",
        counts.join(" · ")
    );
    for issue in summary
        .optional_failed
        .iter()
        .chain(summary.required_failed.iter())
        .chain(summary.incomplete.iter())
    {
        let diagnostic = issue.error.as_deref().unwrap_or("no diagnostic recorded");
        html.push_str(&format!(
            "<span class=\"status-detail\"><code>{}</code> ({}): {}</span>",
            esc_html(&issue.id),
            esc_html(&issue.status),
            esc_html(diagnostic)
        ));
    }
    if is_partial {
        html.push_str("<span class=\"status-note\">Available evidence remains usable. Empty Viewer lanes may belong to capabilities that were not requested or did not complete.</span>");
    }
    html.push_str("</div></div>\n");
    html
}

/// Build sensor config JSON from signals/ directory — what ran, sampling rates, file sizes.
fn build_sensor_config(workspace_dir: &Path) -> String {
    let mut sensors = Vec::new();

    // Check signals/ for sensor outputs
    let signals_dir = workspace_dir.join("signals");
    if let Ok(entries) = fs::read_dir(&signals_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                let id = name.trim_end_matches(".json");
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                sensors.push(serde_json::json!({
                    "id": id,
                    "type": "signal",
                    "file": name,
                    "size": size,
                }));
            }
        }
    }

    // Check tracks/ for track outputs
    let tracks_dir = workspace_dir.join("tracks");
    if let Ok(entries) = fs::read_dir(&tracks_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".track.jsonl") {
                let id = name.trim_end_matches(".track.jsonl");
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                // Count lines for frame count
                let frames = fs::read_to_string(entry.path())
                    .map(|c| c.lines().filter(|l| !l.trim().is_empty()).count())
                    .unwrap_or(0);
                let mut info = serde_json::json!({
                    "id": id,
                    "type": "track",
                    "file": name,
                    "size": size,
                    "frames": frames,
                });
                // Infer sampling rate from first two entries
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
                    if let (Some(a), Some(b)) = (lines.next(), lines.next()) {
                        if let (Ok(va), Ok(vb)) = (
                            serde_json::from_str::<serde_json::Value>(a),
                            serde_json::from_str::<serde_json::Value>(b),
                        ) {
                            let ta = va.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let tb = vb.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let interval_ms = tb - ta;
                            if interval_ms > 0.0 {
                                info["interval_ms"] = serde_json::json!(interval_ms);
                                info["fps"] =
                                    serde_json::json!((1000.0 / interval_ms * 10.0).round() / 10.0);
                            }
                        }
                    }
                }
                sensors.push(info);
            }
        }
    }

    // Check tracks/keyframes/ for keyframe PNGs
    let kf_dir = tracks_dir.join("keyframes");
    if kf_dir.exists() {
        if let Ok(entries) = fs::read_dir(&kf_dir) {
            let count = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".png"))
                .count();
            if count > 0 {
                sensors.push(serde_json::json!({
                    "id": "keyframes",
                    "type": "frames",
                    "count": count,
                    "fps": 2,
                    "interval_ms": 500,
                    "dir": "tracks/keyframes/",
                }));
            }
        }
    }

    serde_json::to_string(&sensors).unwrap_or("[]".into())
}

const SCRIPT: &str = include_str!("html_viewer.js");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn script_json_neutralizes_script_breakout_and_keeps_the_value() {
        let value = json!({
            "text": "</script><script>alert(1)</script> & <!-- \u{2028}",
            "n": 1
        });
        let raw = serde_json::to_string(&value).unwrap();
        let safe = script_json(&raw);
        assert!(!safe.contains("</script>"));
        assert!(!safe.contains('<') && !safe.contains('>') && !safe.contains('&'));
        assert!(!safe.contains('\u{2028}'));
        assert_eq!(serde_json::from_str::<Value>(&safe).unwrap(), value);
    }

    #[test]
    fn esc_html_neutralizes_a_crafted_file_name() {
        let escaped = esc_html("clip\" onload=\"x\"<img src=x>.mp4");
        assert!(!escaped.contains('<') && !escaped.contains('"'));
        assert!(escaped.contains("&lt;img") && escaped.contains("&quot;"));
    }

    #[test]
    fn capability_status_banner_explains_partial_observations_and_escapes_errors() {
        let html = build_capability_status_html(&json!({
            "capabilities": [
                {"id": "probe", "required": true, "status": "succeeded"},
                {
                    "id": "transcript<script>",
                    "required": false,
                    "status": "failed",
                    "error": "provider <offline>"
                }
            ]
        }));

        assert!(html.contains("Partial observation"));
        assert!(html.contains("1 succeeded · 1 optional failed"));
        assert!(html.contains("transcript&lt;script&gt;"));
        assert!(html.contains("provider &lt;offline&gt;"));
        assert!(!html.contains("transcript<script>"));
    }

    #[test]
    fn capability_status_banner_marks_complete_observations() {
        let html = build_capability_status_html(&json!({
            "capabilities": [
                {"id": "probe", "required": true, "status": "succeeded"},
                {"id": "scenes", "required": true, "status": "succeeded"}
            ]
        }));

        assert!(html.contains("Complete observation"));
        assert!(html.contains("2 succeeded"));
        assert!(!html.contains("class=\"observation-status partial\""));
    }
}
