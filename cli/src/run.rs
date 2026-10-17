use crate::plan::{CapabilityRequest, ModelBackendConfig, SensePlan};
use crate::provider::{ExecutionPolicy, Locality};
use crate::registry::{Backend, SensorDef};
use serde::Serialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionReport {
    pub contract: &'static str,
    pub plan: SensePlan,
    pub capabilities: Vec<CapabilityExecution>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityExecution {
    pub id: String,
    pub requested: bool,
    pub required: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ExecutionProvider>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionProvider {
    pub id: String,
    pub locality: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

pub fn execute_plan(
    video: &str,
    out_dir: &str,
    sense_plan: &SensePlan,
    quiet: bool,
) -> Result<ExecutionReport, String> {
    execute_inner(video, out_dir, sense_plan, quiet)
}

/// Result of one sensor's run, delivered from a worker thread back to the
/// scheduler. `stderr` is the child process's captured stderr; the scheduler
/// flushes it under a per-sensor header so logs don't interleave.
struct SensorOutcome {
    plan_idx: usize,
    result: Result<String, String>,
    stderr: String,
    provider: Option<ExecutionProvider>,
}

fn execute_inner(
    video: &str,
    out_dir: &str,
    sense_plan: &SensePlan,
    quiet: bool,
) -> Result<ExecutionReport, String> {
    sense_plan.validate()?;
    let plan = sense_plan.resolved_plan()?;

    fs::create_dir_all(out_dir).map_err(|e| format!("Cannot create {out_dir}: {e}"))?;

    // Create subdirectories for sensors that need them
    for sensor in &plan {
        if let Some(dir) = sensor.sub_dir {
            fs::create_dir_all(format!("{out_dir}/{dir}")).ok();
        }
    }

    let exe_path = std::env::current_exe().map_err(|e| format!("Cannot find self: {e}"))?;
    let exe = Arc::new(exe_path.to_string_lossy().to_string());
    let video = Arc::new(video.to_string());
    let out_dir_arc = Arc::new(out_dir.to_string());
    let scenes_path = Arc::new(format!("{out_dir}/scenes.json"));
    let stats_dir = Arc::new(format!("{out_dir}/stats"));
    let sense_plan_arc = Arc::new(sense_plan.clone());

    let n_workers = std::env::var("VIDEO_PERCEPTION_WORKERS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1)
        .min(plan.len().max(1));

    eprintln!(
        "\n\u{25b6} sensor pipeline: {video} \u{2192} {out_dir}/ ({} sensors, {n_workers} workers)\n",
        plan.len(),
    );

    // ── Dependency map ──
    // Worker scheduling is dep-graph driven: a sensor is "ready" once all of
    // its `deps` that are also in the plan have completed (regardless of
    // outcome — `conditional_on` handles the success gate). `remaining_deps`
    // tracks unsatisfied deps; `dependents` is the reverse map so a finished
    // sensor can decrement counters in O(degree).
    let in_plan: HashSet<&str> = plan.iter().map(|s| s.id).collect();
    let id_to_idx: HashMap<&str, usize> = plan.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    let mut remaining_deps: Vec<usize> = plan
        .iter()
        .map(|s| s.deps.iter().filter(|d| in_plan.contains(*d)).count())
        .collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); plan.len()];
    for (i, s) in plan.iter().enumerate() {
        for dep in s.deps.iter() {
            if let Some(&j) = id_to_idx.get(dep) {
                dependents[j].push(i);
            }
        }
    }

    // ── Worker pool ──
    // `shutdown` flips on the first required-sensor failure so workers stop
    // pulling new work mid-pipeline. Without it, dropping `work_tx` after a
    // fatal error still drains every buffered idx in the queue before the
    // joins return — wasteful and divergent from the old sequential behavior.
    let (work_tx, work_rx) = channel::<usize>();
    let (done_tx, done_rx) = channel::<SensorOutcome>();
    let work_rx = Arc::new(std::sync::Mutex::new(work_rx));
    let plan_arc: Arc<Vec<&'static SensorDef>> = Arc::new(plan.clone());
    let shutdown = Arc::new(AtomicBool::new(false));

    let mut workers = Vec::with_capacity(n_workers);
    for _ in 0..n_workers {
        let rx = Arc::clone(&work_rx);
        let tx = done_tx.clone();
        let plan = Arc::clone(&plan_arc);
        let video = Arc::clone(&video);
        let out_dir = Arc::clone(&out_dir_arc);
        let scenes_path = Arc::clone(&scenes_path);
        let stats_dir = Arc::clone(&stats_dir);
        let exe = Arc::clone(&exe);
        let shutdown = Arc::clone(&shutdown);
        let sense_plan = Arc::clone(&sense_plan_arc);
        workers.push(thread::spawn(move || {
            loop {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                let idx = {
                    let guard = rx.lock().expect("work_rx poisoned");
                    match guard.recv() {
                        Ok(i) => i,
                        Err(_) => break, // channel closed → shutdown
                    }
                };
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                let sensor = plan[idx];
                let (result, stderr, provider) = invoke_sensor_capturing(
                    sensor,
                    &video,
                    &out_dir,
                    &scenes_path,
                    &stats_dir,
                    &exe,
                    sense_plan.request_for(sensor.id),
                    sense_plan.model_backend.as_ref(),
                );
                if tx
                    .send(SensorOutcome {
                        plan_idx: idx,
                        result,
                        stderr,
                        provider,
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
    drop(done_tx);

    // ── Scheduling state ──
    let mut results: HashMap<&'static str, Option<String>> = HashMap::new();
    let mut succeeded: HashSet<&'static str> = HashSet::new();
    let mut scheduled: HashSet<usize> = HashSet::new();
    let mut executions: HashMap<&'static str, CapabilityExecution> = plan
        .iter()
        .map(|definition| {
            (
                definition.id,
                CapabilityExecution {
                    id: definition.id.to_string(),
                    requested: sense_plan.is_requested(definition.id),
                    required: sense_plan.is_required(definition),
                    status: "pending".into(),
                    output: definition.output_file.map(|file| format!("signals/{file}")),
                    error: None,
                    provider: None,
                },
            )
        })
        .collect();
    let mut completed = 0usize;
    let mut fatal_error: Option<String> = None;

    // Helper: try to dispatch sensor idx. Returns true if scheduled or skipped,
    // false if not yet ready (deps not done). Decides skip-vs-schedule based on
    // conditional_on at dispatch time.
    let try_dispatch = |idx: usize,
                        remaining_deps: &[usize],
                        succeeded: &HashSet<&'static str>,
                        scheduled: &mut HashSet<usize>,
                        results: &mut HashMap<&'static str, Option<String>>,
                        executions: &mut HashMap<&'static str, CapabilityExecution>,
                        completed: &mut usize,
                        work_tx: &Sender<usize>|
     -> bool {
        if scheduled.contains(&idx) {
            return true;
        }
        if remaining_deps[idx] > 0 {
            return false;
        }
        let sensor = plan_arc[idx];
        if let Some(cond) = sensor.conditional_on {
            if !succeeded.contains(cond) {
                // Either cond failed or was never in plan — skip this sensor.
                eprintln!("── ({}/{}) {} ──", idx + 1, plan_arc.len(), sensor.id);
                eprintln!("  skipped ({cond} not available)\n");
                results.insert(sensor.id, None);
                if let Some(execution) = executions.get_mut(sensor.id) {
                    execution.status = "skipped".into();
                    execution.error = Some(format!("dependency {cond} was unavailable"));
                }
                *completed += 1;
                scheduled.insert(idx);
                return true;
            }
        }
        scheduled.insert(idx);
        work_tx.send(idx).expect("worker channel closed");
        true
    };

    // Initial seed: dispatch every sensor with zero deps. Sensors whose deps
    // are all upstream skipped (e.g. their conditional_on was never going to
    // succeed) are picked up later by the unblock loop below.
    for idx in 0..plan_arc.len() {
        try_dispatch(
            idx,
            &remaining_deps,
            &succeeded,
            &mut scheduled,
            &mut results,
            &mut executions,
            &mut completed,
            &work_tx,
        );
    }

    while completed < plan_arc.len() {
        let outcome = match done_rx.recv() {
            Ok(o) => o,
            Err(_) => break, // all workers gone
        };
        completed += 1;
        let sensor = plan_arc[outcome.plan_idx];

        eprintln!(
            "── ({}/{}) {} ──",
            outcome.plan_idx + 1,
            plan_arc.len(),
            sensor.id
        );
        if !outcome.stderr.is_empty() {
            eprint!("{}", outcome.stderr);
        }

        match outcome.result {
            Ok(json) => {
                if let Some(file) = sensor.output_file {
                    fs::write(format!("{out_dir}/{file}"), &json).ok();
                }
                results.insert(sensor.id, Some(json));
                succeeded.insert(sensor.id);
                if let Some(execution) = executions.get_mut(sensor.id) {
                    execution.status = "succeeded".into();
                    execution.provider = outcome.provider;
                }
            }
            Err(e) => {
                let diagnostic = execution_error(&e, &outcome.stderr);
                if let Some(execution) = executions.get_mut(sensor.id) {
                    execution.status = "failed".into();
                    execution.error = Some(diagnostic.clone());
                }
                if sense_plan.is_required(sensor) {
                    fatal_error = Some(format!("{} failed: {diagnostic}", sensor.id));
                    shutdown.store(true, Ordering::Release);
                    break;
                }
                eprintln!("  skipped ({e})");
                results.insert(sensor.id, None);
            }
        }
        eprintln!();

        // Unblock dependents: a finished sensor (success OR best-effort fail)
        // satisfies its outgoing dep edge. The dispatch helper then re-checks
        // conditional_on against `succeeded`, so a failed prerequisite still
        // correctly skips downstream sensors that gated on it.
        for &dep_idx in &dependents[outcome.plan_idx] {
            remaining_deps[dep_idx] = remaining_deps[dep_idx].saturating_sub(1);
            try_dispatch(
                dep_idx,
                &remaining_deps,
                &succeeded,
                &mut scheduled,
                &mut results,
                &mut executions,
                &mut completed,
                &work_tx,
            );
        }
    }

    drop(work_tx);
    for w in workers {
        let _ = w.join();
    }

    let report = execution_report(sense_plan, &plan, &executions);
    write_execution_report(out_dir, &report)?;

    if let Some(err) = fatal_error {
        return Err(err);
    }

    // ── Post-pipeline ──
    if succeeded.contains("stats") {
        eprintln!("\u{2500}\u{2500} color track \u{2500}\u{2500}");
        let track_dir = format!("{out_dir}/tracks");
        fs::create_dir_all(&track_dir).ok();
        match crate::commands::stats::generate_color_track(&video, &track_dir) {
            Ok(()) => {}
            Err(e) => eprintln!("  color track skipped: {e}"),
        }
        eprintln!();
    }

    let deck = assemble_deck(&video, &plan, &results)?;
    fs::write(format!("{out_dir}/deck.json"), &deck).map_err(|e| format!("write: {e}"))?;

    eprintln!("\u{2713} signal deck: {out_dir}/deck.json\n");
    if !quiet {
        println!("{deck}");
    }
    Ok(report)
}

fn execution_error(summary: &str, stderr: &str) -> String {
    let details = stderr.trim();
    if details.is_empty() || details == summary {
        summary.to_string()
    } else {
        format!("{summary}: {details}")
    }
}

/// Variant of `invoke_sensor` used by the parallel scheduler. Captures the
/// child's stderr instead of streaming it, so the scheduler can flush it
/// under a per-sensor header without interleaving across workers.
fn invoke_sensor_capturing(
    sensor: &SensorDef,
    video: &str,
    out_dir: &str,
    scenes_path: &str,
    stats_dir: &str,
    exe: &str,
    request: Option<&CapabilityRequest>,
    model_backend: Option<&ModelBackendConfig>,
) -> (Result<String, String>, String, Option<ExecutionProvider>) {
    if let Some(provider_id) = request
        .and_then(|request| request.provider.as_deref())
        .filter(|provider| *provider != "auto")
    {
        match crate::extensions::find_provider(provider_id, sensor.id) {
            Ok(Some(provider)) => {
                let policy = request
                    .and_then(|request| request.policy.as_deref())
                    .unwrap_or("prefer-local");
                let policy = match ExecutionPolicy::parse(policy) {
                    Ok(policy) => policy,
                    Err(error) => return (Err(error), String::new(), None),
                };
                let locality = if provider.locality == "remote" {
                    Locality::Remote
                } else {
                    Locality::Local
                };
                if !policy.allows(locality) {
                    return (
                        Err(format!(
                            "provider {} is {} but the selected policy does not allow it",
                            provider.id, provider.locality
                        )),
                        String::new(),
                        None,
                    );
                }
                let model = provider
                    .capabilities
                    .iter()
                    .find(|capability| capability.id == sensor.id)
                    .and_then(|capability| capability.model.clone());
                let execution_provider = ExecutionProvider {
                    id: provider.id.clone(),
                    locality: provider.locality.clone(),
                    model,
                };
                let provider_request = json!({
                    "video": video,
                    "scenes": if sensor.needs_scenes { Some(scenes_path) } else { None },
                    "keyframe_dir": if sensor.needs_keyframe_dir { Some(stats_dir) } else { None },
                    "out_dir": if sensor.needs_out_dir { Some(out_dir) } else { None },
                    "options": request,
                });
                let result = crate::extensions::execute_command_provider(
                    &provider,
                    sensor.id,
                    provider_request,
                );
                return (result, String::new(), Some(execution_provider));
            }
            Ok(None) => {}
            Err(error) => return (Err(error), String::new(), None),
        }
    }

    if matches!(sensor.backend, Backend::Vlm(_))
        && let Some(config) = model_backend
        && !matches!(config.provider.as_str(), "ollama" | "gemini")
    {
        match crate::extensions::find_provider(&config.provider, sensor.id) {
            Ok(Some(provider)) if provider.runtime.kind == "command" => {
                let execution_provider = ExecutionProvider {
                    id: provider.id.clone(),
                    locality: provider.locality.clone(),
                    model: Some(config.model().to_string()),
                };
                let result = crate::extensions::execute_command_provider(
                    &provider,
                    sensor.id,
                    json!({
                        "video": video,
                        "scenes": scenes_path,
                        "keyframe_dir": stats_dir,
                        "model": config.model(),
                    }),
                );
                return (result, String::new(), Some(execution_provider));
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                return (
                    Err(format!(
                        "provider {} does not implement {}",
                        config.provider, sensor.id
                    )),
                    String::new(),
                    None,
                );
            }
            Err(error) => return (Err(error), String::new(), None),
        }
    }

    match sensor.backend {
        Backend::Rust => {
            let mut args: Vec<String> = vec!["__capability".into(), sensor.id.into(), video.into()];
            if sensor.needs_scenes {
                args.push("--scenes".into());
                args.push(scenes_path.into());
            }
            if sensor.needs_out_dir {
                let dir = sensor.sub_dir.unwrap_or(".");
                args.push("--out-dir".into());
                args.push(format!("{out_dir}/{dir}"));
            }
            if sensor.needs_keyframe_dir {
                args.push("--keyframe-dir".into());
                args.push(stats_dir.into());
            }
            if sensor.id == "keyframes" {
                args.push("--out-dir".into());
                args.push(out_dir.into());
            }
            if sensor.id == "signalstats" {
                let track_dir = format!("{out_dir}/tracks");
                fs::create_dir_all(&track_dir).ok();
                args.push("--track-dir".into());
                args.push(track_dir);
            }
            if sensor.id == "scenes" {
                if let Some(options) = request.and_then(|request| request.options.as_ref()) {
                    if let Some(threshold) = options.threshold {
                        args.push("--threshold".into());
                        args.push(threshold.to_string());
                    }
                    if let Some(merge_ms) = options.merge_ms {
                        args.push("--merge".into());
                        args.push(merge_ms.to_string());
                    }
                    if options.merge == Some(false) {
                        args.push("--no-merge".into());
                    }
                }
            }
            if sensor.id == "transcript" {
                if let Some(provider) = request.and_then(|request| request.provider.as_deref()) {
                    args.push("--provider".into());
                    args.push(provider.into());
                }
                if let Some(policy) = request.and_then(|request| request.policy.as_deref()) {
                    args.push("--policy".into());
                    args.push(policy.into());
                }
                if let Some(language) = request.and_then(|request| request.language.as_deref()) {
                    args.push("--lang".into());
                    args.push(language.into());
                }
            }
            let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let (result, stderr) = invoke_capturing(exe, &refs);
            (result, stderr, None)
        }
        Backend::Python(tool) => {
            let mut args: Vec<&str> = vec![video];
            if sensor.needs_scenes {
                args.extend_from_slice(&["--scenes", scenes_path]);
            }
            let (result, stderr) = invoke_python_capturing(tool, &args);
            (result, stderr, None)
        }
        Backend::Vlm(preset) => {
            let mut args: Vec<&str> = vec!["--preset", preset, "--scenes", scenes_path];
            if sensor.needs_keyframe_dir {
                args.extend_from_slice(&["--keyframe-dir", stats_dir]);
            }
            let (result, stderr) = invoke_model_backend_capturing(
                "video-perception-vlm",
                &args,
                model_backend,
                sensor.id,
            );
            let provider = model_backend_record(model_backend, sensor.id)
                .ok()
                .flatten();
            (result, stderr, provider)
        }
    }
}

fn assemble_deck(
    video: &str,
    plan: &[&SensorDef],
    results: &HashMap<&str, Option<String>>,
) -> Result<String, String> {
    let mut deck = serde_json::Map::new();
    deck.insert("version".into(), serde_json::Value::String("1.0".into()));
    deck.insert("video".into(), serde_json::Value::String(video.into()));
    deck.insert("created_at".into(), serde_json::Value::String(iso_now()));

    for sensor in plan {
        if let Some(key) = sensor.deck_key {
            if let Some(Some(json)) = results.get(sensor.id) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json.trim()) {
                    deck.insert(key.into(), val);
                }
            }
        }
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(deck))
        .map_err(|e| format!("deck JSON: {e}"))
}

/// Variant of `invoke` that captures stderr instead of streaming it. Used by
/// the parallel scheduler so per-sensor logs can be flushed atomically.
fn invoke_capturing(exe: &str, args: &[&str]) -> (Result<String, String>, String) {
    match Command::new(exe).args(args).output() {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            if !output.status.success() {
                return (
                    Err(format!("{} exited with {}", args[0], output.status)),
                    stderr,
                );
            }
            (
                Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
                stderr,
            )
        }
        Err(e) => (Err(format!("invoke {}: {e}", args[0])), String::new()),
    }
}

fn invoke_python_capturing(tool: &str, args: &[&str]) -> (Result<String, String>, String) {
    match Command::new(tool).args(args).output() {
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
            if o.status.success() {
                (Ok(String::from_utf8_lossy(&o.stdout).into_owned()), stderr)
            } else {
                (Err(format!("{tool} exited with {}", o.status)), stderr)
            }
        }
        Err(_) => (Err(format!("{tool} not found on PATH")), String::new()),
    }
}

fn invoke_model_backend_capturing(
    tool: &str,
    args: &[&str],
    config: Option<&ModelBackendConfig>,
    capability: &str,
) -> (Result<String, String>, String) {
    let mut command = Command::new(tool);
    command.args(args);
    if let Some(config) = config {
        command.env("VIDEO_PERCEPTION_VLM_BACKEND", &config.provider);
        command.env("VIDEO_PERCEPTION_VLM_MODEL", config.model());
        if !matches!(config.provider.as_str(), "ollama" | "gemini") {
            match crate::extensions::find_provider(&config.provider, capability) {
                Ok(Some(provider)) => match serde_json::to_string(&provider) {
                    Ok(encoded) => {
                        command.env("VIDEO_PERCEPTION_PROVIDER_CONFIG", encoded);
                    }
                    Err(error) => {
                        return (
                            Err(format!("encode provider {} config: {error}", provider.id)),
                            String::new(),
                        );
                    }
                },
                Ok(None) => {}
                Err(error) => return (Err(error), String::new()),
            }
        }
    }

    match command.output() {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            if output.status.success() {
                (
                    Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
                    stderr,
                )
            } else {
                (Err(format!("{tool} exited with {}", output.status)), stderr)
            }
        }
        Err(_) => (Err(format!("{tool} not found on PATH")), String::new()),
    }
}

fn model_backend_record(
    config: Option<&ModelBackendConfig>,
    capability: &str,
) -> Result<Option<ExecutionProvider>, String> {
    let provider_id = config
        .map(|config| config.provider.clone())
        .or_else(|| std::env::var("VIDEO_PERCEPTION_VLM_BACKEND").ok())
        .unwrap_or_else(|| "ollama".to_string());
    let model = config
        .map(|config| config.model().to_string())
        .or_else(|| std::env::var("VIDEO_PERCEPTION_VLM_MODEL").ok())
        .or_else(|| std::env::var("OLLAMA_MODEL").ok());
    let locality = match provider_id.as_str() {
        "ollama" => "local".to_string(),
        "gemini" => "remote".to_string(),
        _ => match crate::extensions::find_provider(&provider_id, capability)? {
            Some(provider) => provider.locality,
            None => return Ok(None),
        },
    };
    Ok(Some(ExecutionProvider {
        id: provider_id,
        locality,
        model,
    }))
}

fn execution_report(
    sense_plan: &SensePlan,
    plan: &[&SensorDef],
    executions: &HashMap<&str, CapabilityExecution>,
) -> ExecutionReport {
    ExecutionReport {
        contract: "video-perception.execution-report/v1",
        plan: sense_plan.clone(),
        capabilities: plan
            .iter()
            .filter_map(|definition| executions.get(definition.id).cloned())
            .collect(),
    }
}

fn write_execution_report(out_dir: &str, report: &ExecutionReport) -> Result<(), String> {
    let body = serde_json::to_string_pretty(report)
        .map_err(|error| format!("execution report JSON: {error}"))?;
    fs::write(format!("{out_dir}/execution.json"), body)
        .map_err(|error| format!("write execution report: {error}"))
}

fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Civil date from days since epoch (Howard Hinnant algorithm)
    let days = (secs / 86400) as i64;
    let time = secs % 86400;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

#[cfg(test)]
mod model_backend_tests {
    use super::*;

    #[test]
    fn configured_backend_is_forwarded_to_model_process() {
        let config = ModelBackendConfig {
            provider: "ollama".into(),
            model: Some("test-model".into()),
        };
        let (output, stderr) =
            invoke_model_backend_capturing("/usr/bin/env", &[], Some(&config), "scene-description");
        assert!(stderr.is_empty());
        let output = output.unwrap();
        assert!(
            output
                .lines()
                .any(|line| line == "VIDEO_PERCEPTION_VLM_BACKEND=ollama")
        );
        assert!(
            output
                .lines()
                .any(|line| line == "VIDEO_PERCEPTION_VLM_MODEL=test-model")
        );
    }
}
