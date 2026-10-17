use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

#[derive(Serialize)]
pub struct Beat {
    pub time_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
}

#[derive(Serialize)]
pub struct Beats {
    pub bpm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_signature: Option<String>,
    pub beats: Vec<Beat>,
}

pub fn execute(video: &str) -> Result<(), String> {
    let runner = resolve_runner()?;
    // The output file lives in a directory only this user can reach; it is
    // removed with the directory when `scratch` drops.
    let scratch = crate::temp::PrivateDir::create("beats")?;
    let output_path = scratch.file("beats.json");

    let mut command = match &runner {
        Runner::Path(binary) => {
            let mut c = Command::new(binary);
            c.args(["analyze", video, "-o"]);
            c.arg(&output_path);
            c.args(["--mode", "tight", "--audio-path-mode", "omit", "--verbose"]);
            c
        }
        Runner::UvProject(project) => {
            let mut c = Command::new("uv");
            // uv keeps its cache and the project environment under this
            // user's cache directory, never in a shared temp directory.
            if let Some(cache) = per_user_cache() {
                c.env("UV_CACHE_DIR", cache.join("uv-cache"));
                c.env("UV_PROJECT_ENVIRONMENT", cache.join("autobeat-venv"));
            }
            c.args(["run", "--project"]);
            c.arg(project);
            c.args(["autobeat", "analyze", video, "-o"]);
            c.arg(&output_path);
            c.args(["--mode", "tight", "--audio-path-mode", "omit", "--verbose"]);
            c
        }
    };

    let output = command
        .output()
        .map_err(|e| format!("autobeat analyze failed: {e}"))?;

    // autobeat prints logs and hints to stdout/stderr.
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        eprint!("{stdout}");
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }

    if !output.status.success() {
        return Err(format!("autobeat exited with {}", output.status));
    }

    let payload = std::fs::read(&output_path).map_err(|e| {
        format!(
            "Could not read autobeat output {}: {e}",
            output_path.display()
        )
    })?;
    let parsed: Value = serde_json::from_slice(&payload)
        .map_err(|e| format!("Invalid autobeat JSON output: {e}"))?;

    let anchors = parsed
        .get("selected")
        .and_then(|s| s.get("grid"))
        .and_then(|g| g.get("anchors_sec"))
        .and_then(Value::as_array)
        .or_else(|| {
            parsed
                .get("grid")
                .and_then(|g| g.get("anchors_sec"))
                .and_then(Value::as_array)
        })
        .ok_or("autobeat JSON missing grid.anchors_sec")?;

    let bpm = parsed
        .get("grid")
        .and_then(|_| {
            parsed
                .get("selected")
                .and_then(|s| s.get("analysis"))
                .and_then(|a| a.get("tempo_bpm"))
                .and_then(positive_f64)
        })
        .or_else(|| {
            parsed
                .get("analysis")
                .and_then(|a| a.get("tempo_bpm"))
                .and_then(positive_f64)
        })
        .or_else(|| {
            parsed
                .get("audio")
                .and_then(|a| a.get("tempo_bpm"))
                .and_then(positive_f64)
        })
        .or_else(|| estimate_bpm(anchors))
        .ok_or("autobeat JSON missing tempo_bpm and could not infer bpm from anchors")?;

    let beats = anchors
        .iter()
        .filter_map(|v| v.as_f64())
        .map(|sec| Beat {
            time_ms: (sec * 1000.0).round().max(0.0) as u64,
            strength: None,
        })
        .collect::<Vec<_>>();

    if beats.is_empty() {
        return Err("autobeat produced no beat anchors".to_string());
    }

    let result = Beats {
        bpm,
        time_signature: None,
        beats,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&result)
            .map_err(|e| format!("Could not encode beats JSON: {e}"))?
    );

    Ok(())
}

fn tool_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

enum Runner {
    Path(String),
    UvProject(PathBuf),
}

fn resolve_runner() -> Result<Runner, String> {
    for candidate in project_candidates() {
        let local_binary = candidate.join(".venv/bin/autobeat");
        if local_binary.is_file() {
            return Ok(Runner::Path(local_binary.to_string_lossy().to_string()));
        }
    }

    if tool_exists("autobeat") {
        return Ok(Runner::Path("autobeat".to_string()));
    }

    if tool_exists("uv") {
        for candidate in project_candidates() {
            if candidate.join("pyproject.toml").is_file() {
                return Ok(Runner::UvProject(candidate));
            }
        }
    }

    Err(
        "autobeat is unavailable. Install `autobeat` in PATH or set AUTOBEAT_PROJECT to a local autobeat repo."
            .to_string(),
    )
}

/// Only an explicit `AUTOBEAT_PROJECT` names a project to run through uv. The
/// current directory is never searched: a checkout that happens to contain a
/// `.venv/bin/autobeat` must not become the executable this command runs.
fn project_candidates() -> Vec<PathBuf> {
    std::env::var("AUTOBEAT_PROJECT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect()
}

fn per_user_cache() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("video-perception"))
}

fn positive_f64(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| *v > 0.0)
}

fn estimate_bpm(anchors: &[Value]) -> Option<f64> {
    let mut intervals = anchors
        .windows(2)
        .filter_map(|pair| {
            match (
                pair.first().and_then(Value::as_f64),
                pair.get(1).and_then(Value::as_f64),
            ) {
                (Some(a), Some(b)) if b > a => Some(b - a),
                _ => None,
            }
        })
        .collect::<Vec<_>>();

    if intervals.is_empty() {
        return None;
    }

    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = intervals[intervals.len() / 2];
    if median <= 0.0 {
        return None;
    }

    Some(60.0 / median)
}
