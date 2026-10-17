//! Incremental media observation runtime.
//!
//! Stream mode intentionally has its own scheduler and contract. Batch
//! Observation planning depends on a complete asset; an open-ended stream does
//! not have stable duration or final scene boundaries. Both modes still consume
//! the same explicit SensePlan and logical capability catalog.

use crate::plan::SensePlan;
use crate::registry::{self, StreamingProfile};
use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const EVENT_CONTRACT: &str = "video-perception.stream-event/v1";
const SESSION_CONTRACT: &str = "video-perception.stream-session/v1";
const SESSION_VERSION: &str = "1.0";
static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackpressurePolicy {
    Block,
    DropNewest,
}

pub struct StreamOptions<'a> {
    pub source: &'a str,
    pub out: &'a str,
    pub plan: &'a SensePlan,
    pub speed: f64,
    pub sample_fps: f64,
    pub event_buffer: usize,
    pub backpressure: BackpressurePolicy,
    pub provider_overrides: &'a [String],
    pub finalize_observation: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderRef {
    id: String,
    class: &'static str,
    locality: &'static str,
}

#[derive(Debug, Serialize)]
struct StreamEvent<'a> {
    contract: &'static str,
    session_id: &'a str,
    seq: u64,
    observed_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_time_ms: Option<u64>,
    capability: &'a str,
    kind: &'a str,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<&'a ProviderRef>,
    payload: &'a Value,
}

#[derive(Debug)]
struct DraftEvent {
    capability: String,
    kind: &'static str,
    status: &'static str,
    media_time_ms: Option<u64>,
    provider: Option<ProviderRef>,
    payload: Value,
}

enum WorkerMessage {
    Event(DraftEvent),
    Finished {
        capability: String,
        provider: ProviderRef,
        result: Result<Value, String>,
        dropped_events: u64,
    },
}

#[derive(Debug, Serialize)]
struct SessionSource<'a> {
    uri: &'a str,
    kind: &'static str,
    pacing: &'static str,
    speed: f64,
}

#[derive(Debug, Serialize)]
struct SessionFiles<'a> {
    events: &'static str,
    tracks: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct SessionCapability {
    id: String,
    required: bool,
    strategy: &'static str,
    provider: ProviderRef,
    status: &'static str,
    dropped_events: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionManifest<'a> {
    contract: &'static str,
    version: &'static str,
    session_id: &'a str,
    state: &'static str,
    source: SessionSource<'a>,
    sense_plan: &'a SensePlan,
    started_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    ended_at_unix_ms: Option<u64>,
    event_count: u64,
    backpressure: BackpressurePolicy,
    event_buffer: usize,
    capabilities: &'a [SessionCapability],
    files: SessionFiles<'a>,
}

#[derive(Clone)]
struct EventEmitter {
    sender: SyncSender<WorkerMessage>,
    policy: BackpressurePolicy,
    dropped: Arc<AtomicU64>,
}

impl EventEmitter {
    fn event(&self, event: DraftEvent) -> bool {
        match self.policy {
            BackpressurePolicy::Block => self.sender.send(WorkerMessage::Event(event)).is_ok(),
            BackpressurePolicy::DropNewest => {
                match self.sender.try_send(WorkerMessage::Event(event)) {
                    Ok(()) => true,
                    Err(TrySendError::Full(_)) => {
                        self.dropped.fetch_add(1, Ordering::Relaxed);
                        true
                    }
                    Err(TrySendError::Disconnected(_)) => false,
                }
            }
        }
    }

    fn finish(&self, capability: String, provider: ProviderRef, result: Result<Value, String>) {
        let _ = self.sender.send(WorkerMessage::Finished {
            capability,
            provider,
            result,
            dropped_events: self.dropped.load(Ordering::Relaxed),
        });
    }
}

pub fn execute(options: StreamOptions<'_>) -> Result<(), String> {
    validate_options(&options)?;
    let source_kind = source_kind(options.source)?;
    if options.finalize_observation && source_kind != "file" {
        return Err("--finalize-observation currently requires a local file source".into());
    }
    let provider_overrides = parse_provider_overrides(options.provider_overrides)?;
    let requested = resolve_requested_capabilities(options.plan, &provider_overrides)?;
    if source_kind == "network" && requested.len() > 1 {
        return Err(
            "network streams currently support one capability per session; use a relay/tee or run separate sessions"
                .into(),
        );
    }

    let out_dir = PathBuf::from(options.out);
    prepare_output_dir(&out_dir)?;
    let tracks_dir = out_dir.join("tracks");
    fs::create_dir_all(&tracks_dir).map_err(|error| format!("create tracks directory: {error}"))?;

    let started_at = now_ms();
    let session_id = format!("stream-{started_at}-{}", std::process::id());
    CANCELLED.store(false, Ordering::Relaxed);
    install_cancel_handler()?;

    let mut capability_states = requested
        .iter()
        .map(|item| SessionCapability {
            id: item.id.clone(),
            required: item.required,
            strategy: item.profile.strategy,
            provider: item.provider.clone(),
            status: "running",
            dropped_events: 0,
            summary: None,
            error: None,
        })
        .collect::<Vec<_>>();

    write_manifest(
        &out_dir,
        &session_id,
        "running",
        source_kind,
        &options,
        started_at,
        None,
        0,
        &capability_states,
        None,
    )?;

    let events_path = out_dir.join("events.jsonl");
    let events_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&events_path)
        .map_err(|error| format!("create {}: {error}", events_path.display()))?;
    let mut event_writer = BufWriter::new(events_file);
    let stdout = std::io::stdout();
    let mut stdout_lock = stdout.lock();
    let mut seq = 0_u64;

    let started_payload = json!({
        "source": options.source,
        "source_kind": source_kind,
        "capabilities": requested.iter().map(|item| json!({
            "id": item.id,
            "strategy": item.profile.strategy,
            "provider": item.provider,
        })).collect::<Vec<_>>(),
        "sample_fps": options.sample_fps,
        "backpressure": options.backpressure,
    });
    write_event(
        &mut event_writer,
        &mut stdout_lock,
        &session_id,
        &mut seq,
        DraftEvent {
            capability: "session".to_string(),
            kind: "session.started",
            status: "final",
            media_time_ms: None,
            provider: None,
            payload: started_payload,
        },
    )?;

    let (sender, receiver) = mpsc::sync_channel(options.event_buffer);
    let mut handles = Vec::new();
    let backpressure = options.backpressure;
    for item in requested {
        let worker_sender = sender.clone();
        let worker_tracks = tracks_dir.clone();
        let source = options.source.to_string();
        let sample_fps = options.sample_fps;
        let speed = options.speed;
        let source_is_file = source_kind == "file";
        handles.push(thread::spawn(move || {
            let dropped = Arc::new(AtomicU64::new(0));
            let emitter = EventEmitter {
                sender: worker_sender,
                policy: backpressure,
                dropped,
            };
            let result = match item.id.as_str() {
                "keyframes" => run_keyframes(
                    &source,
                    &worker_tracks,
                    sample_fps,
                    speed,
                    source_is_file,
                    &item.provider,
                    &emitter,
                    &CANCELLED,
                ),
                "signalstats" => run_signalstats(
                    &source,
                    &worker_tracks,
                    sample_fps,
                    speed,
                    source_is_file,
                    &item.provider,
                    &emitter,
                    &CANCELLED,
                ),
                _ => Err(format!("no streaming worker for {}", item.id)),
            };
            emitter.finish(item.id, item.provider, result);
        }));
    }
    drop(sender);

    collect_worker_events(
        &receiver,
        &session_id,
        &mut seq,
        &mut event_writer,
        &mut stdout_lock,
        &mut capability_states,
    )?;
    for handle in handles {
        handle
            .join()
            .map_err(|_| "stream worker panicked".to_string())?;
    }

    let required_failed = capability_states
        .iter()
        .any(|capability| capability.required && capability.status == "failed");
    let any_failed = capability_states
        .iter()
        .any(|capability| capability.status == "failed");
    let was_cancelled = CANCELLED.load(Ordering::Relaxed);

    let mut observation_path = None;
    if options.finalize_observation && !required_failed && !was_cancelled {
        let finalizing_payload = json!({"output": "observation"});
        write_event(
            &mut event_writer,
            &mut stdout_lock,
            &session_id,
            &mut seq,
            DraftEvent {
                capability: "session".to_string(),
                kind: "session.finalizing",
                status: "provisional",
                media_time_ms: None,
                provider: None,
                payload: finalizing_payload,
            },
        )?;
        let target = out_dir.join("observation");
        if let Err(error) = crate::workspace::observe(
            options.source,
            target
                .to_str()
                .ok_or_else(|| "observation path is not valid UTF-8".to_string())?,
            options.plan,
        ) {
            let failure_payload = json!({"error": error});
            write_event(
                &mut event_writer,
                &mut stdout_lock,
                &session_id,
                &mut seq,
                DraftEvent {
                    capability: "session".to_string(),
                    kind: "session.closed",
                    status: "error",
                    media_time_ms: None,
                    provider: None,
                    payload: failure_payload,
                },
            )?;
            write_manifest(
                &out_dir,
                &session_id,
                "failed",
                source_kind,
                &options,
                started_at,
                Some(now_ms()),
                seq,
                &capability_states,
                None,
            )?;
            return Err(format!(
                "finalize Observation failed; stream session saved at {}",
                out_dir.display()
            ));
        }
        observation_path = Some("observation");
    }

    let state = if was_cancelled {
        "cancelled"
    } else if required_failed {
        "failed"
    } else {
        "closed"
    };
    let outcome = if was_cancelled {
        "cancelled"
    } else if required_failed {
        "failed"
    } else if any_failed {
        "partial"
    } else {
        "succeeded"
    };
    let closed_payload = json!({
        "outcome": outcome,
        "events": seq + 1,
        "observation": observation_path,
    });
    write_event(
        &mut event_writer,
        &mut stdout_lock,
        &session_id,
        &mut seq,
        DraftEvent {
            capability: "session".to_string(),
            kind: "session.closed",
            status: if required_failed { "error" } else { "final" },
            media_time_ms: None,
            provider: None,
            payload: closed_payload,
        },
    )?;

    write_manifest(
        &out_dir,
        &session_id,
        state,
        source_kind,
        &options,
        started_at,
        Some(now_ms()),
        seq,
        &capability_states,
        observation_path,
    )?;

    if was_cancelled {
        Err(format!(
            "stream cancelled; session saved at {}",
            out_dir.display()
        ))
    } else if required_failed {
        Err(format!(
            "required streaming capability failed; session saved at {}",
            out_dir.display()
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone)]
struct RequestedCapability {
    id: String,
    required: bool,
    profile: StreamingProfile,
    provider: ProviderRef,
}

fn validate_options(options: &StreamOptions<'_>) -> Result<(), String> {
    if !options.speed.is_finite() || options.speed < 0.0 {
        return Err("--speed must be a finite number greater than or equal to 0".into());
    }
    if !options.sample_fps.is_finite() || options.sample_fps <= 0.0 || options.sample_fps > 30.0 {
        return Err("--sample-fps must be a finite number in (0, 30]".into());
    }
    if options.event_buffer == 0 {
        return Err("--event-buffer must be greater than 0".into());
    }
    Ok(())
}

fn source_kind(source: &str) -> Result<&'static str, String> {
    if Path::new(source).is_file() {
        return Ok("file");
    }
    if [
        "http://", "https://", "rtmp://", "rtsp://", "srt://", "udp://",
    ]
    .iter()
    .any(|prefix| source.starts_with(prefix))
    {
        return Ok("network");
    }
    Err(format!(
        "stream source does not exist and is not a supported network URL: {source}"
    ))
}

fn parse_provider_overrides(values: &[String]) -> Result<HashMap<String, String>, String> {
    let mut result = HashMap::new();
    for value in values {
        let (capability, provider) = value
            .split_once('=')
            .ok_or_else(|| format!("invalid --provider {value}; expected CAPABILITY=PROVIDER"))?;
        if capability.is_empty() || provider.is_empty() {
            return Err(format!(
                "invalid --provider {value}; expected CAPABILITY=PROVIDER"
            ));
        }
        if result
            .insert(capability.to_string(), provider.to_string())
            .is_some()
        {
            return Err(format!("duplicate provider override for {capability}"));
        }
    }
    Ok(result)
}

fn resolve_requested_capabilities(
    plan: &SensePlan,
    overrides: &HashMap<String, String>,
) -> Result<Vec<RequestedCapability>, String> {
    for capability in overrides.keys() {
        if !plan.capabilities.iter().any(|item| &item.id == capability) {
            return Err(format!(
                "provider override targets unrequested capability: {capability}"
            ));
        }
    }

    plan.capabilities
        .iter()
        .map(|request| {
            let definition = registry::find(&request.id)
                .ok_or_else(|| format!("unknown capability: {}", request.id))?;
            let profile = registry::streaming_profile(&request.id);
            if !profile.implemented {
                return Err(format!(
                    "capability {} is not available in stream mode (strategy: {}). Run `video-perception catalog list --json` for readiness.",
                    request.id, profile.strategy
                ));
            }
            let requested_provider = overrides
                .get(&request.id)
                .map(String::as_str)
                .unwrap_or("auto");
            let provider_id = if requested_provider == "auto" {
                profile.providers.first().copied().ok_or_else(|| {
                    format!("capability {} has no streaming provider", request.id)
                })?
            } else if profile.providers.contains(&requested_provider) {
                requested_provider
            } else {
                return Err(format!(
                    "stream provider {requested_provider} cannot serve {}; available: {}",
                    request.id,
                    profile.providers.join(", ")
                ));
            };
            Ok(RequestedCapability {
                id: request.id.clone(),
                required: request.required.unwrap_or(definition.required),
                profile,
                provider: provider_ref(provider_id),
            })
        })
        .collect()
}

fn provider_ref(id: &str) -> ProviderRef {
    match id {
        "apple-speech" => ProviderRef {
            id: id.to_string(),
            class: "system-native",
            locality: "local",
        },
        "ollama" | "whisper-cpp" => ProviderRef {
            id: id.to_string(),
            class: "user-managed-local",
            locality: "local",
        },
        "gemini" => ProviderRef {
            id: id.to_string(),
            class: "user-configured-remote",
            locality: "remote",
        },
        _ => ProviderRef {
            id: id.to_string(),
            class: "built-in-local",
            locality: "local",
        },
    }
}

fn prepare_output_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Err(format!(
            "stream output already exists: {}. Choose a new --out directory.",
            path.display()
        ));
    }
    fs::create_dir_all(path).map_err(|error| format!("create {}: {error}", path.display()))
}

#[cfg(unix)]
fn install_cancel_handler() -> Result<(), String> {
    extern "C" fn cancel_stream(_signal: libc::c_int) {
        CANCELLED.store(true, Ordering::Relaxed);
    }

    // SAFETY: sigaction is zero-initialized before every field used by libc is
    // assigned. `cancel_stream` has C signal-handler ABI and performs only an
    // atomic store, which is safe in this signal path.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = cancel_stream as *const () as usize;
    action.sa_flags = 0;
    // SAFETY: `action.sa_mask` is valid storage owned by this stack frame.
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
        return Err("install stream cancellation handler: sigemptyset() failed".into());
    }
    // SAFETY: `action` remains valid for each call and the handler is a static
    // function. SIGINT/SIGTERM are process-global and installed once before
    // workers start.
    let interrupt = unsafe { libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) };
    let terminate = unsafe { libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) };
    if interrupt != 0 || terminate != 0 {
        return Err("install stream cancellation handler: signal() failed".into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_cancel_handler() -> Result<(), String> {
    Ok(())
}

fn ffmpeg_input_args(command: &mut Command, source: &str, speed: f64, is_file: bool) {
    command.args(["-hide_banner", "-nostdin"]);
    if is_file && speed > 0.0 {
        command.args(["-readrate", &speed.to_string()]);
    }
    command.args(["-i", source]);
}

#[allow(clippy::too_many_arguments)]
fn run_signalstats(
    source: &str,
    tracks_dir: &Path,
    sample_fps: f64,
    speed: f64,
    is_file: bool,
    provider: &ProviderRef,
    emitter: &EventEmitter,
    cancelled: &AtomicBool,
) -> Result<Value, String> {
    let track_path = tracks_dir.join("signalstats.track.jsonl");
    let mut track = BufWriter::new(
        File::create(&track_path)
            .map_err(|error| format!("create {}: {error}", track_path.display()))?,
    );
    let filter = format!("fps={sample_fps},signalstats=stat=tout+vrep+brng,metadata=print");
    let mut command = Command::new("ffmpeg");
    ffmpeg_input_args(&mut command, source, speed, is_file);
    let mut child = command
        .args(["-loglevel", "info", "-vf", &filter, "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start ffmpeg signalstats: {error}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "ffmpeg signalstats stderr is unavailable".to_string())?;

    let mut current: Option<SignalFrame> = None;
    let mut count = 0_u64;
    let mut error_tail = Vec::<String>::new();
    for line in BufReader::new(stderr).lines() {
        let line = line.map_err(|error| format!("read ffmpeg signalstats output: {error}"))?;
        remember_tail(&mut error_tail, &line);
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
        if line.contains("frame:") && line.contains("pts_time:") {
            if let Some(frame) = current.take() {
                emit_signal_frame(frame, provider, emitter, &mut track)?;
                count += 1;
            }
            current = Some(SignalFrame::from_header(&line, count));
            continue;
        }
        if let Some(frame) = current.as_mut() {
            frame.read_metric(&line);
        }
    }
    if let Some(frame) = current.take() {
        emit_signal_frame(frame, provider, emitter, &mut track)?;
        count += 1;
    }
    track
        .flush()
        .map_err(|error| format!("flush signalstats track: {error}"))?;

    let status = child
        .wait()
        .map_err(|error| format!("wait for ffmpeg signalstats: {error}"))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("stream cancelled".into());
    }
    if !status.success() {
        return Err(format!(
            "ffmpeg signalstats failed: {}",
            error_tail
                .last()
                .map(String::as_str)
                .unwrap_or("unknown error")
        ));
    }
    Ok(json!({
        "frames": count,
        "sample_fps": sample_fps,
        "track": "tracks/signalstats.track.jsonl",
    }))
}

#[derive(Default)]
struct SignalFrame {
    frame: u64,
    media_time_ms: u64,
    yavg: f64,
    ymin: f64,
    ymax: f64,
    satavg: f64,
}

impl SignalFrame {
    fn from_header(line: &str, fallback_frame: u64) -> Self {
        Self {
            frame: value_after(line, "frame:")
                .and_then(|value| value.parse().ok())
                .unwrap_or(fallback_frame),
            media_time_ms: value_after(line, "pts_time:")
                .and_then(|value| value.parse::<f64>().ok())
                .map(seconds_to_ms)
                .unwrap_or(0),
            ..Self::default()
        }
    }

    fn read_metric(&mut self, line: &str) {
        if let Some(value) = metric(line, "lavfi.signalstats.YAVG=") {
            self.yavg = value;
        }
        if let Some(value) = metric(line, "lavfi.signalstats.YMIN=") {
            self.ymin = value;
        }
        if let Some(value) = metric(line, "lavfi.signalstats.YMAX=") {
            self.ymax = value;
        }
        if let Some(value) = metric(line, "lavfi.signalstats.SATAVG=") {
            self.satavg = value;
        }
    }
}

fn emit_signal_frame(
    frame: SignalFrame,
    provider: &ProviderRef,
    emitter: &EventEmitter,
    track: &mut BufWriter<File>,
) -> Result<(), String> {
    let payload = json!({
        "frame": frame.frame,
        "t": frame.media_time_ms,
        "y": round2(frame.yavg),
        "yn": round2(frame.ymin),
        "yx": round2(frame.ymax),
        "sa": round2(frame.satavg),
    });
    writeln!(
        track,
        "{}",
        serde_json::to_string(&payload)
            .map_err(|error| format!("serialize signal frame: {error}"))?
    )
    .map_err(|error| format!("write signalstats track: {error}"))?;
    if !emitter.event(DraftEvent {
        capability: "signalstats".to_string(),
        kind: "measurement",
        status: "final",
        media_time_ms: Some(frame.media_time_ms),
        provider: Some(provider.clone()),
        payload,
    }) {
        return Err("stream event receiver disconnected".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_keyframes(
    source: &str,
    tracks_dir: &Path,
    sample_fps: f64,
    speed: f64,
    is_file: bool,
    provider: &ProviderRef,
    emitter: &EventEmitter,
    cancelled: &AtomicBool,
) -> Result<Value, String> {
    let frame_dir = tracks_dir.join("keyframes");
    fs::create_dir_all(&frame_dir)
        .map_err(|error| format!("create {}: {error}", frame_dir.display()))?;
    let track_path = tracks_dir.join("keyframes.track.jsonl");
    let mut track = BufWriter::new(
        File::create(&track_path)
            .map_err(|error| format!("create {}: {error}", track_path.display()))?,
    );
    let output_pattern = frame_dir.join("frame_%06d.png");
    let output_pattern = output_pattern
        .to_str()
        .ok_or_else(|| "keyframe output path is not valid UTF-8".to_string())?;
    let filter = format!("fps={sample_fps},showinfo");

    let mut command = Command::new("ffmpeg");
    ffmpeg_input_args(&mut command, source, speed, is_file);
    let mut child = command
        .args([
            "-loglevel",
            "info",
            "-vf",
            &filter,
            "-fps_mode",
            "vfr",
            "-q:v",
            "2",
            output_pattern,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start ffmpeg keyframes: {error}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "ffmpeg keyframes stderr is unavailable".to_string())?;

    let mut count = 0_u64;
    let mut error_tail = Vec::<String>::new();
    for line in BufReader::new(stderr).lines() {
        let line = line.map_err(|error| format!("read ffmpeg keyframes output: {error}"))?;
        remember_tail(&mut error_tail, &line);
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
        if !line.contains("showinfo") || !line.contains(" pts_time:") {
            continue;
        }
        let frame_index = value_after(&line, " n:")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(count);
        let media_time_ms = value_after(&line, "pts_time:")
            .and_then(|value| value.parse::<f64>().ok())
            .map(seconds_to_ms)
            .unwrap_or_else(|| ((count as f64 / sample_fps) * 1000.0).round() as u64);
        let filename = format!("frame_{:06}.png", frame_index + 1);
        let absolute_path = frame_dir.join(&filename);
        let asset_ready = wait_for_file(&absolute_path, cancelled);
        let relative_path = format!("tracks/keyframes/{filename}");
        let payload = json!({
            "idx": frame_index + 1,
            "t": media_time_ms,
            "path": relative_path,
            "asset_ready": asset_ready,
        });
        writeln!(
            track,
            "{}",
            serde_json::to_string(&payload)
                .map_err(|error| format!("serialize keyframe: {error}"))?
        )
        .map_err(|error| format!("write keyframe track: {error}"))?;
        if !emitter.event(DraftEvent {
            capability: "keyframes".to_string(),
            kind: "asset",
            status: if asset_ready { "final" } else { "provisional" },
            media_time_ms: Some(media_time_ms),
            provider: Some(provider.clone()),
            payload,
        }) {
            return Err("stream event receiver disconnected".into());
        }
        count += 1;
    }
    track
        .flush()
        .map_err(|error| format!("flush keyframes track: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("wait for ffmpeg keyframes: {error}"))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("stream cancelled".into());
    }
    if !status.success() {
        return Err(format!(
            "ffmpeg keyframes failed: {}",
            error_tail
                .last()
                .map(String::as_str)
                .unwrap_or("unknown error")
        ));
    }
    Ok(json!({
        "frames": count,
        "sample_fps": sample_fps,
        "track": "tracks/keyframes.track.jsonl",
    }))
}

fn collect_worker_events(
    receiver: &Receiver<WorkerMessage>,
    session_id: &str,
    seq: &mut u64,
    event_writer: &mut BufWriter<File>,
    stdout: &mut impl Write,
    capability_states: &mut [SessionCapability],
) -> Result<(), String> {
    let mut remaining = capability_states.len();
    while remaining > 0 {
        let message = receiver.recv().map_err(|_| {
            "all stream workers disconnected before reporting completion".to_string()
        })?;
        match message {
            WorkerMessage::Event(event) => {
                write_event(event_writer, stdout, session_id, seq, event)?;
            }
            WorkerMessage::Finished {
                capability,
                provider,
                result,
                dropped_events,
            } => {
                let state = capability_states
                    .iter_mut()
                    .find(|item| item.id == capability)
                    .ok_or_else(|| format!("unknown finished stream capability: {capability}"))?;
                state.provider = provider.clone();
                state.dropped_events = dropped_events;
                let (kind, status, payload) = match result {
                    Ok(summary) => {
                        state.status = "succeeded";
                        state.summary = Some(summary.clone());
                        (
                            "capability.completed",
                            "final",
                            json!({"summary": summary, "dropped_events": dropped_events}),
                        )
                    }
                    Err(error) => {
                        state.status = "failed";
                        state.error = Some(error.clone());
                        (
                            "capability.failed",
                            "error",
                            json!({"error": error, "dropped_events": dropped_events}),
                        )
                    }
                };
                write_event(
                    event_writer,
                    stdout,
                    session_id,
                    seq,
                    DraftEvent {
                        capability,
                        kind,
                        status,
                        media_time_ms: None,
                        provider: Some(provider),
                        payload,
                    },
                )?;
                remaining -= 1;
            }
        }
    }
    Ok(())
}

fn write_event(
    event_writer: &mut BufWriter<File>,
    stdout: &mut impl Write,
    session_id: &str,
    seq: &mut u64,
    draft: DraftEvent,
) -> Result<(), String> {
    *seq += 1;
    let event = StreamEvent {
        contract: EVENT_CONTRACT,
        session_id,
        seq: *seq,
        observed_at_unix_ms: now_ms(),
        media_time_ms: draft.media_time_ms,
        capability: &draft.capability,
        kind: draft.kind,
        status: draft.status,
        provider: draft.provider.as_ref(),
        payload: &draft.payload,
    };
    let line = serde_json::to_string(&event)
        .map_err(|error| format!("serialize stream event: {error}"))?;
    writeln!(event_writer, "{line}").map_err(|error| format!("write events.jsonl: {error}"))?;
    event_writer
        .flush()
        .map_err(|error| format!("flush events.jsonl: {error}"))?;
    writeln!(stdout, "{line}").map_err(|error| format!("write stream event: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("flush stream event: {error}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_manifest(
    out_dir: &Path,
    session_id: &str,
    state: &'static str,
    source_kind: &'static str,
    options: &StreamOptions<'_>,
    started_at: u64,
    ended_at: Option<u64>,
    event_count: u64,
    capabilities: &[SessionCapability],
    observation: Option<&str>,
) -> Result<(), String> {
    let manifest = SessionManifest {
        contract: SESSION_CONTRACT,
        version: SESSION_VERSION,
        session_id,
        state,
        source: SessionSource {
            uri: options.source,
            kind: source_kind,
            pacing: if source_kind == "file" && options.speed > 0.0 {
                "readrate"
            } else if source_kind == "file" {
                "unpaced"
            } else {
                "source-paced"
            },
            speed: options.speed,
        },
        sense_plan: options.plan,
        started_at_unix_ms: started_at,
        ended_at_unix_ms: ended_at,
        event_count,
        backpressure: options.backpressure,
        event_buffer: options.event_buffer,
        capabilities,
        files: SessionFiles {
            events: "events.jsonl",
            tracks: "tracks/",
            observation,
        },
    };
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("serialize stream session: {error}"))?;
    fs::write(out_dir.join("session.json"), bytes)
        .map_err(|error| format!("write session.json: {error}"))
}

fn remember_tail(tail: &mut Vec<String>, line: &str) {
    const MAX_LINES: usize = 20;
    if tail.len() == MAX_LINES {
        tail.remove(0);
    }
    if !line.trim().is_empty() {
        tail.push(line.to_string());
    }
}

fn value_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.split_once(key)?.1.trim_start();
    rest.split_whitespace().next()
}

fn metric(line: &str, key: &str) -> Option<f64> {
    line.split_once(key)?.1.trim().parse().ok()
}

fn seconds_to_ms(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn wait_for_file(path: &Path, cancelled: &AtomicBool) -> bool {
    for _ in 0..200 {
        if path.is_file() {
            return true;
        }
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    path.is_file()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_overrides() {
        let values = vec!["keyframes=local-ffmpeg".to_string()];
        let parsed = parse_provider_overrides(&values).unwrap();
        assert_eq!(
            parsed.get("keyframes").map(String::as_str),
            Some("local-ffmpeg")
        );
    }

    #[test]
    fn rejects_duplicate_provider_overrides() {
        let values = vec![
            "keyframes=local-ffmpeg".to_string(),
            "keyframes=ollama".to_string(),
        ];
        assert!(parse_provider_overrides(&values).is_err());
    }

    #[test]
    fn parses_signalstats_header() {
        let frame = SignalFrame::from_header("frame:7 pts:21 pts_time:0.7", 0);
        assert_eq!(frame.frame, 7);
        assert_eq!(frame.media_time_ms, 700);
    }
}
