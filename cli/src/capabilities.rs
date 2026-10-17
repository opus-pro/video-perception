//! Device capability discovery and setup-profile planning.
//!
//! This module is deliberately read-only. It may inspect local executables,
//! environment variables, memory, and disk space, but it never downloads a
//! model, changes configuration, or probes a remote endpoint. `doctor` remains
//! the command that verifies live services.

use crate::{extensions, util};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

const CONTRACT: &str = "video-perception.capabilities/v1";
const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
struct DeviceReport {
    os: String,
    os_version: Option<String>,
    architecture: String,
    apple_silicon: bool,
    memory_gib: Option<f64>,
    available_disk_gib: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ProviderClass {
    SystemNative,
    UserManagedLocal,
}

#[derive(Debug, Clone, Serialize)]
struct Requirement {
    id: String,
    met: bool,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderCapability {
    id: String,
    ready: bool,
    verification: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderReport {
    id: String,
    class: ProviderClass,
    locality: String,
    status: String,
    capabilities: Vec<ProviderCapability>,
    requirements: Vec<Requirement>,
    privacy: String,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ModelRecommendation {
    id: String,
    provider: String,
    purpose: String,
    install_state: String,
    estimated_download_gib: f64,
    minimum_memory_gib: f64,
    recommended_memory_gib: f64,
    docker_required: bool,
    device_fit: String,
    fit_reason: String,
    install_command: String,
    caveat: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProfileReport {
    id: String,
    label: String,
    description: String,
    privacy: String,
    transcript_provider: String,
    semantic_provider: Option<String>,
    core_ready: bool,
    full_profile_ready: bool,
    readiness_scope: String,
    blockers: Vec<String>,
    environment: BTreeMap<String, String>,
    next_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Recommendation {
    profile: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize)]
struct RoutingReport {
    deterministic_video: String,
    transcript_provider: String,
    transcript_policy: String,
    semantic_provider: String,
    remote_media_possible: bool,
    source: String,
    overrides: BTreeMap<String, String>,
    live_verification: String,
}

#[derive(Debug, Clone, Serialize)]
struct CapabilityReport {
    contract: &'static str,
    device: DeviceReport,
    routing: RoutingReport,
    providers: Vec<ProviderReport>,
    local_models: Vec<ModelRecommendation>,
    profiles: Vec<ProfileReport>,
    extensions: Value,
    recommendation: Recommendation,
    guarantees: Vec<String>,
}

trait Probe {
    fn os(&self) -> String;
    fn os_version(&self) -> Option<String>;
    fn architecture(&self) -> String;
    fn memory_bytes(&self) -> Option<u64>;
    fn available_disk_bytes(&self) -> Option<u64>;
    fn command_exists(&self, name: &str) -> bool;
    fn file_exists(&self, path: &str) -> bool;
    fn env(&self, name: &str) -> Option<String>;
}

struct SystemProbe;

impl Probe for SystemProbe {
    fn os(&self) -> String {
        std::env::consts::OS.to_string()
    }

    fn os_version(&self) -> Option<String> {
        let (command, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
            ("sw_vers", &["-productVersion"])
        } else {
            ("uname", &["-r"])
        };
        command_output(command, args)
    }

    fn architecture(&self) -> String {
        std::env::consts::ARCH.to_string()
    }

    fn memory_bytes(&self) -> Option<u64> {
        if cfg!(target_os = "macos") {
            if let Some(memory) =
                command_output("sysctl", &["-n", "hw.memsize"]).and_then(|value| value.parse().ok())
            {
                return Some(memory);
            }
            return command_output("system_profiler", &["SPHardwareDataType"])
                .and_then(|output| parse_system_profiler_memory(&output));
        }

        std::fs::read_to_string("/proc/meminfo")
            .ok()?
            .lines()
            .find_map(|line| {
                let value = line.strip_prefix("MemTotal:")?.split_whitespace().next()?;
                value.parse::<u64>().ok().map(|kib| kib * 1024)
            })
    }

    fn available_disk_bytes(&self) -> Option<u64> {
        let output = Command::new("df").args(["-Pk", "."]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let available_kib = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .next_back()?
            .split_whitespace()
            .nth(3)?
            .parse::<u64>()
            .ok()?;
        Some(available_kib * 1024)
    }

    fn command_exists(&self, name: &str) -> bool {
        if name.contains('/') {
            Path::new(name).is_file()
        } else {
            util::which(name)
        }
    }

    fn file_exists(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn env(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
}

pub fn execute(json: bool) -> Result<(), String> {
    let report = build_report(&SystemProbe);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| format!("JSON: {error}"))?
        );
    } else {
        print_report(&report);
    }
    Ok(())
}

pub fn setup(profile: Option<&str>, json: bool) -> Result<(), String> {
    let report = build_report(&SystemProbe);
    let profile_id = profile.unwrap_or(&report.recommendation.profile);
    let selected = report
        .profiles
        .iter()
        .find(|candidate| candidate.id == profile_id)
        .ok_or_else(|| {
            let names = report
                .profiles
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("Unknown setup profile: {profile_id}. Use: {names}")
        })?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "contract": "video-perception.setup-plan/v1",
                "device": &report.device,
                "current_routing": &report.routing,
                "profile": selected,
                "mutates_system": false,
                "live_service_verification": "Run `video-perception doctor --json` after applying the plan.",
            }))
            .map_err(|error| format!("JSON: {error}"))?
        );
    } else {
        print_setup(selected, &report.routing);
    }
    Ok(())
}

fn build_report(probe: &dyn Probe) -> CapabilityReport {
    let device = device_report(probe);
    let routing = routing_report(probe);
    let core_ready = ["ffmpeg", "ffprobe", "magick"]
        .iter()
        .all(|tool| probe.command_exists(tool));
    let providers = provider_reports(probe);
    let local_models = model_recommendations(probe, &device);
    let profiles = profile_reports(probe, core_ready, &providers);
    let extensions = extensions::discovery_value();

    let has_local_transcript = capability_ready(&providers, "apple-speech", "transcript")
        || capability_ready(&providers, "whisper-cpp", "transcript");
    let has_local_semantics = capability_ready(&providers, "ollama", "semantic-visual-observation");
    let recommendation = if has_local_transcript || has_local_semantics {
        Recommendation {
            profile: "local-model".to_string(),
            reason:
                "At least one user-managed local model path is present; verify it live before use."
                    .to_string(),
        }
    } else {
        Recommendation {
            profile: "basic".to_string(),
            reason: "Start with deterministic local sensing. Add a local transcript or vision provider only when the task needs it.".to_string(),
        }
    };

    CapabilityReport {
        contract: CONTRACT,
        device,
        routing,
        providers,
        local_models,
        profiles,
        extensions,
        recommendation,
        guarantees: vec![
            "Discovery performs no downloads or configuration writes.".to_string(),
            "Discovery performs no remote network request; doctor verifies live services.".to_string(),
            "A planned macOS framework capability is never reported as available until its provider host is installed.".to_string(),
            "Consumer profiles, business rules, and final judgments remain outside Video Perception.".to_string(),
        ],
    }
}

fn routing_report(probe: &dyn Probe) -> RoutingReport {
    let mut overrides = BTreeMap::new();
    for key in [
        "VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER",
        "VIDEO_PERCEPTION_TRANSCRIPT_POLICY",
        "VIDEO_PERCEPTION_VLM_BACKEND",
    ] {
        if let Some(value) = probe.env(key) {
            overrides.insert(key.to_string(), value);
        }
    }

    let transcript_provider = overrides
        .get("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER")
        .cloned()
        .unwrap_or_else(|| "auto".to_string());
    let transcript_policy = overrides
        .get("VIDEO_PERCEPTION_TRANSCRIPT_POLICY")
        .cloned()
        .unwrap_or_else(|| "local-only".to_string());
    let semantic_provider = overrides
        .get("VIDEO_PERCEPTION_VLM_BACKEND")
        .cloned()
        .unwrap_or_else(|| "ollama".to_string());
    let remote_media_possible = route_can_use_remote_media(
        &transcript_provider,
        &transcript_policy,
        Some(&semantic_provider),
    );

    RoutingReport {
        deterministic_video: "local".to_string(),
        transcript_provider,
        transcript_policy,
        semantic_provider,
        remote_media_possible,
        source: if overrides.is_empty() {
            "built-in-defaults"
        } else {
            "environment-overrides"
        }
        .to_string(),
        overrides,
        live_verification: "not-probed; run video-perception doctor".to_string(),
    }
}

fn route_can_use_remote_media(
    _transcript_provider: &str,
    _transcript_policy: &str,
    semantic_provider: Option<&str>,
) -> bool {
    match semantic_provider {
        Some("ollama") | None => false,
        Some("gemini") => true,
        Some(provider) => extensions::find_provider_by_id(provider)
            .ok()
            .flatten()
            .is_none_or(|manifest| manifest.locality == "remote"),
    }
}

fn device_report(probe: &dyn Probe) -> DeviceReport {
    let os = probe.os();
    let architecture = probe.architecture();
    DeviceReport {
        apple_silicon: os == "macos" && matches!(architecture.as_str(), "aarch64" | "arm64"),
        os,
        os_version: probe.os_version(),
        architecture,
        memory_gib: probe.memory_bytes().map(bytes_to_gib),
        available_disk_gib: probe.available_disk_bytes().map(bytes_to_gib),
    }
}

fn provider_reports(probe: &dyn Probe) -> Vec<ProviderReport> {
    let is_macos = probe.os() == "macos";
    let apple_bin = probe
        .env("VIDEO_PERCEPTION_APPLE_SPEECH_BIN")
        .unwrap_or_else(|| "video-perception-apple".to_string());
    let apple_ready = is_macos && probe.command_exists(&apple_bin);
    let apple_status = if apple_ready {
        "ready"
    } else if is_macos {
        "provider-host-not-installed"
    } else {
        "unsupported-on-this-os"
    };

    let whisper_bin = probe.env("WHISPER_CPP_BIN");
    let whisper_model = probe.env("WHISPER_CPP_MODEL");
    let whisper_bin_ready = whisper_bin
        .as_deref()
        .is_some_and(|path| probe.command_exists(path));
    let whisper_model_ready = whisper_model
        .as_deref()
        .is_some_and(|path| probe.file_exists(path));
    let whisper_ready = whisper_bin_ready && whisper_model_ready;

    let video_perception_vlm_ready = probe.command_exists("video-perception-vlm");
    let ollama_ready = probe.command_exists("ollama");

    vec![
        ProviderReport {
            id: "apple-speech".to_string(),
            class: ProviderClass::SystemNative,
            locality: "local".to_string(),
            status: apple_status.to_string(),
            capabilities: vec![ProviderCapability {
                id: "transcript".to_string(),
                ready: apple_ready,
                verification: "executable-present".to_string(),
            }],
            requirements: vec![
                requirement("macos", is_macos, "macOS host", None),
                requirement(
                    "apple-provider-host",
                    apple_ready,
                    &format!("{apple_bin} executable"),
                    Some(
                        "Install the signed Apple provider host or set VIDEO_PERCEPTION_APPLE_SPEECH_BIN",
                    ),
                ),
            ],
            privacy: "Media remains on this Mac.".to_string(),
            notes: vec![
                "Speech.framework is integrated only through the external provider host contract."
                    .to_string(),
                "Vision/OCR/saliency are planned capabilities and are not reported as available."
                    .to_string(),
            ],
        },
        ProviderReport {
            id: "whisper-cpp".to_string(),
            class: ProviderClass::UserManagedLocal,
            locality: "local".to_string(),
            status: if whisper_ready {
                "ready"
            } else {
                "needs-setup"
            }
            .to_string(),
            capabilities: vec![ProviderCapability {
                id: "transcript".to_string(),
                ready: whisper_ready,
                verification: "binary-and-model-files-present".to_string(),
            }],
            requirements: vec![
                requirement(
                    "whisper-binary",
                    whisper_bin_ready,
                    "WHISPER_CPP_BIN points to an executable",
                    Some("Install whisper.cpp and set WHISPER_CPP_BIN"),
                ),
                requirement(
                    "whisper-model",
                    whisper_model_ready,
                    "WHISPER_CPP_MODEL points to a model file",
                    Some("Download a whisper.cpp model and set WHISPER_CPP_MODEL"),
                ),
            ],
            privacy: "Media remains on this machine.".to_string(),
            notes: vec!["Uses a user-managed model and aligned timestamps.".to_string()],
        },
        ProviderReport {
            id: "ollama".to_string(),
            class: ProviderClass::UserManagedLocal,
            locality: "local".to_string(),
            status: if ollama_ready && video_perception_vlm_ready {
                "runtime-installed"
            } else {
                "needs-setup"
            }
            .to_string(),
            capabilities: vec![ProviderCapability {
                id: "semantic-visual-observation".to_string(),
                ready: ollama_ready && video_perception_vlm_ready,
                verification: "runtime-present; model availability requires doctor".to_string(),
            }],
            requirements: vec![
                requirement(
                    "ollama",
                    ollama_ready,
                    "Ollama runtime",
                    Some("brew install ollama"),
                ),
                requirement(
                    "video-perception-vlm",
                    video_perception_vlm_ready,
                    "Video Perception VLM adapter",
                    Some("Install the packaged video-perception-vlm adapter"),
                ),
            ],
            privacy: "Media remains on this machine.".to_string(),
            notes: vec![
                "The model process and storage are owned by the user-managed runtime.".to_string(),
                "Run doctor before relying on a pulled model or a live Ollama service.".to_string(),
            ],
        },
    ]
}

fn model_recommendations(probe: &dyn Probe, device: &DeviceReport) -> Vec<ModelRecommendation> {
    let whisper_installed = probe
        .env("WHISPER_CPP_MODEL")
        .as_deref()
        .is_some_and(|path| probe.file_exists(path));
    let ollama_runtime_installed = probe.command_exists("ollama");

    vec![
        model_recommendation(
            device,
            "whisper-base",
            "whisper-cpp",
            "Local timestamped speech transcription",
            if whisper_installed {
                "model-file-present"
            } else {
                "not-installed"
            },
            0.15,
            4.0,
            8.0,
            "Install whisper.cpp, download a model, and set WHISPER_CPP_BIN/WHISPER_CPP_MODEL",
            "The 0.15 GiB figure is an approximate download size for ggml-base; language and accuracy needs may justify a different model.",
        ),
        model_recommendation(
            device,
            "gemma4",
            "ollama",
            "Local semantic scene observations through the current default Ollama backend",
            if ollama_runtime_installed {
                "runtime-present-model-unverified"
            } else {
                "not-installed"
            },
            9.6,
            16.0,
            24.0,
            "Install Ollama and the video-perception-vlm adapter, then run `ollama pull gemma4`",
            "Disk and memory figures are planning estimates, not a benchmark guarantee. Confirm the resolved Ollama tag and size before downloading.",
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn model_recommendation(
    device: &DeviceReport,
    id: &str,
    provider: &str,
    purpose: &str,
    install_state: &str,
    download_gib: f64,
    minimum_memory_gib: f64,
    recommended_memory_gib: f64,
    install_command: &str,
    caveat: &str,
) -> ModelRecommendation {
    let disk_ok = device
        .available_disk_gib
        .is_none_or(|disk| disk >= download_gib + 2.0);
    let (device_fit, fit_reason) = match device.memory_gib {
        Some(_) if !disk_ok => (
            "not-recommended",
            format!(
                "Only {:.1} GiB free disk; allow at least {:.1} GiB including working space.",
                device.available_disk_gib.unwrap_or(0.0),
                download_gib + 2.0
            ),
        ),
        Some(memory) if memory >= recommended_memory_gib => (
            "recommended",
            format!(
                "{memory:.1} GiB memory meets the {recommended_memory_gib:.0} GiB planning recommendation."
            ),
        ),
        Some(memory) if memory >= minimum_memory_gib => (
            "possible",
            format!(
                "{memory:.1} GiB memory meets the planning minimum but not the {recommended_memory_gib:.0} GiB recommendation."
            ),
        ),
        Some(memory) => (
            "not-recommended",
            format!(
                "{memory:.1} GiB memory is below the {minimum_memory_gib:.0} GiB planning minimum."
            ),
        ),
        None if disk_ok => (
            "unknown",
            "Memory could not be detected; inspect the planning requirements before downloading."
                .to_string(),
        ),
        None => (
            "not-recommended",
            "Available disk is below the model plus working-space estimate.".to_string(),
        ),
    };

    ModelRecommendation {
        id: id.to_string(),
        provider: provider.to_string(),
        purpose: purpose.to_string(),
        install_state: install_state.to_string(),
        estimated_download_gib: download_gib,
        minimum_memory_gib,
        recommended_memory_gib,
        docker_required: false,
        device_fit: device_fit.to_string(),
        fit_reason,
        install_command: install_command.to_string(),
        caveat: caveat.to_string(),
    }
}

fn profile_reports(
    probe: &dyn Probe,
    core_ready: bool,
    providers: &[ProviderReport],
) -> Vec<ProfileReport> {
    let apple_ready = capability_ready(providers, "apple-speech", "transcript");
    let whisper_ready = capability_ready(providers, "whisper-cpp", "transcript");
    let ollama_ready = capability_ready(providers, "ollama", "semantic-visual-observation");
    let local_transcript = apple_ready || whisper_ready;

    vec![
        profile(
            "basic",
            "Basic local sensing",
            "Use deterministic local sensors without requiring a speech or vision model.",
            "Media remains on this machine.",
            "auto",
            None,
            core_ready,
            core_ready,
            blockers(core_ready, &[]),
            &[
                ("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER", "auto"),
                ("VIDEO_PERCEPTION_TRANSCRIPT_POLICY", "local-only"),
            ],
            vec!["Run `video-perception doctor` to verify FFmpeg and ImageMagick."],
        ),
        profile(
            "mac-native",
            "Mac native",
            "Use only the signed macOS provider host plus deterministic local sensors.",
            "No media leaves the Mac.",
            "apple-speech",
            None,
            core_ready,
            core_ready && apple_ready,
            blockers(
                core_ready,
                &[(apple_ready, "Apple Speech provider host is not ready")],
            ),
            &[
                ("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER", "apple-speech"),
                ("VIDEO_PERCEPTION_TRANSCRIPT_POLICY", "local-only"),
            ],
            vec!["Install/authorize the Apple provider host if listed as a blocker."],
        ),
        profile(
            "local-model",
            "Local models",
            "Use Apple Speech or whisper.cpp for transcript and Ollama for semantic observations.",
            "No media leaves this machine after required models are downloaded.",
            "auto",
            Some("ollama"),
            core_ready,
            core_ready && local_transcript && ollama_ready,
            blockers(
                core_ready,
                &[
                    (local_transcript, "No local transcript provider is ready"),
                    (
                        ollama_ready,
                        "Ollama and the Video Perception VLM adapter are not both ready",
                    ),
                ],
            ),
            &[
                ("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER", "auto"),
                ("VIDEO_PERCEPTION_TRANSCRIPT_POLICY", "local-only"),
                ("VIDEO_PERCEPTION_VLM_BACKEND", "ollama"),
            ],
            vec![
                "Review local_models device_fit before downloading a model.",
                "Run `video-perception doctor --json` to verify the Ollama model and service.",
            ],
        ),
    ]
    .into_iter()
    .map(|mut item| {
        if probe.os() != "macos" && item.id == "mac-native" {
            item.blockers
                .push("This profile requires macOS.".to_string());
            item.full_profile_ready = false;
        }
        item
    })
    .collect()
}

#[allow(clippy::too_many_arguments)]
fn profile(
    id: &str,
    label: &str,
    description: &str,
    privacy: &str,
    transcript_provider: &str,
    semantic_provider: Option<&str>,
    core_ready: bool,
    full_profile_ready: bool,
    blockers: Vec<String>,
    environment: &[(&str, &str)],
    next_steps: Vec<&str>,
) -> ProfileReport {
    ProfileReport {
        id: id.to_string(),
        label: label.to_string(),
        description: description.to_string(),
        privacy: privacy.to_string(),
        transcript_provider: transcript_provider.to_string(),
        semantic_provider: semantic_provider.map(str::to_string),
        core_ready,
        full_profile_ready,
        readiness_scope: "static-prerequisites-only; run doctor for live verification".to_string(),
        blockers,
        environment: environment
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
        next_steps: next_steps.into_iter().map(str::to_string).collect(),
    }
}

fn blockers(core_ready: bool, checks: &[(bool, &str)]) -> Vec<String> {
    let mut result = Vec::new();
    if !core_ready {
        result.push("Core tools ffmpeg, ffprobe, and magick are not all ready".to_string());
    }
    result.extend(
        checks
            .iter()
            .filter(|(ready, _)| !ready)
            .map(|(_, message)| (*message).to_string()),
    );
    result
}

fn capability_ready(providers: &[ProviderReport], provider_id: &str, capability_id: &str) -> bool {
    providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .and_then(|provider| {
            provider
                .capabilities
                .iter()
                .find(|capability| capability.id == capability_id)
        })
        .is_some_and(|capability| capability.ready)
}

fn requirement(id: &str, met: bool, detail: &str, action: Option<&str>) -> Requirement {
    Requirement {
        id: id.to_string(),
        met,
        detail: detail.to_string(),
        action: (!met).then(|| action.map(str::to_string)).flatten(),
    }
}

fn bytes_to_gib(bytes: u64) -> f64 {
    ((bytes as f64 / GIB as f64) * 10.0).round() / 10.0
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn parse_system_profiler_memory(output: &str) -> Option<u64> {
    let value = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Memory:"))?
        .trim();
    let mut parts = value.split_whitespace();
    let amount = parts.next()?.parse::<f64>().ok()?;
    let multiplier = match parts.next()? {
        "TB" => 1024.0 * GIB as f64,
        "GB" => GIB as f64,
        "MB" => (1024 * 1024) as f64,
        _ => return None,
    };
    Some((amount * multiplier).round() as u64)
}

fn print_report(report: &CapabilityReport) {
    eprintln!("Video Perception capability report\n");
    eprintln!(
        "Device: {} {}{}",
        report.device.os,
        report.device.architecture,
        report
            .device
            .memory_gib
            .map(|memory| format!(", {memory:.1} GiB memory"))
            .unwrap_or_default()
    );
    if let Some(disk) = report.device.available_disk_gib {
        eprintln!("Free disk: {disk:.1} GiB");
    }

    eprintln!("\nTask routes:");
    print_current_routes(&report.routing);

    eprintln!("\nExecution layers (static discovery; live services are not probed):");
    for provider in &report.providers {
        let ready = provider
            .capabilities
            .iter()
            .filter(|capability| capability.ready)
            .count();
        eprintln!(
            "  {:<14} {:<16} {:<24} {ready}/{} static paths",
            provider_class_label(&provider.class),
            provider.id,
            provider.status,
            provider.capabilities.len()
        );
    }

    let extension_count = report
        .extensions
        .get("extensions")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let extension_errors = report
        .extensions
        .get("errors")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    eprintln!("\nAgent extensions: {extension_count} discovered, {extension_errors} invalid");
    eprintln!("  Inspect: video-perception extension list --json");

    let missing = report
        .providers
        .iter()
        .flat_map(|provider| {
            provider.requirements.iter().filter_map(move |requirement| {
                (!requirement.met).then(|| {
                    format!(
                        "{}: {}",
                        provider.id,
                        requirement.action.as_deref().unwrap_or(&requirement.detail)
                    )
                })
            })
        })
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        eprintln!("\nMissing for additional routes:");
        for item in missing {
            eprintln!("  - {item}");
        }
    }

    eprintln!("\nLocal model fit:");
    for model in &report.local_models {
        eprintln!(
            "  {:<16} {:<15} {}",
            model.id, model.device_fit, model.fit_reason
        );
    }

    eprintln!(
        "\nRecommended profile: {} — {}",
        report.recommendation.profile, report.recommendation.reason
    );
    eprintln!(
        "Next: video-perception setup --profile {}",
        report.recommendation.profile
    );
    eprintln!("Then verify live services: video-perception doctor");
    eprintln!("JSON: video-perception capabilities --json");
}

fn print_setup(profile: &ProfileReport, current: &RoutingReport) {
    eprintln!("{} ({})\n", profile.label, profile.id);
    eprintln!("{}", profile.description);
    eprintln!("Privacy: {}", profile.privacy);
    eprintln!("\nCurrent task routes ({}):", current.source);
    print_current_routes(current);

    let planned_policy = profile
        .environment
        .get("VIDEO_PERCEPTION_TRANSCRIPT_POLICY")
        .map(String::as_str)
        .unwrap_or("prefer-local");
    let planned_semantic = profile.semantic_provider.as_deref();
    let remote_media_possible = route_can_use_remote_media(
        &profile.transcript_provider,
        planned_policy,
        planned_semantic,
    );
    eprintln!("\nPlanned task routes (advisory, not active yet):");
    eprintln!("  Video measurements  local deterministic sensors");
    eprintln!(
        "  Transcript          {}",
        transcript_route_label(&profile.transcript_provider, planned_policy)
    );
    eprintln!(
        "  Visual semantics    {}",
        planned_semantic.map_or("disabled".to_string(), semantic_route_label)
    );
    eprintln!(
        "  Remote media        {}",
        if remote_media_possible {
            "possible — explicit consent required before use"
        } else {
            "disabled by this routing plan"
        }
    );
    eprintln!("  Provider live       not checked — run video-perception doctor");

    eprintln!("\nStatic readiness:");
    eprintln!("  Core tools          {}", yes_no(profile.core_ready));
    eprintln!(
        "  Profile prereqs     {}",
        yes_no(profile.full_profile_ready)
    );

    if !profile.blockers.is_empty() {
        eprintln!("\nBlockers:");
        for blocker in &profile.blockers {
            eprintln!("  - {blocker}");
        }
    }

    eprintln!("\nEnvironment plan (apply only after accepting the privacy choice):");
    for (key, value) in &profile.environment {
        eprintln!("  export {key}={value}");
    }

    eprintln!("\nNext steps:");
    for step in &profile.next_steps {
        eprintln!("  - {step}");
    }
    eprintln!(
        "\nThis command is advisory and changed nothing on the system or in the current shell."
    );
}

fn print_current_routes(routing: &RoutingReport) {
    eprintln!("  Video measurements  local deterministic sensors");
    eprintln!(
        "  Transcript          {}",
        transcript_route_label(&routing.transcript_provider, &routing.transcript_policy)
    );
    eprintln!(
        "  Visual semantics    {}",
        semantic_route_label(&routing.semantic_provider)
    );
    eprintln!(
        "  Remote media        {}",
        if routing.remote_media_possible {
            "possible for model-backed tasks"
        } else {
            "disabled by current routing"
        }
    );
    eprintln!("  Provider live       {}", routing.live_verification);
}

fn transcript_route_label(provider: &str, policy: &str) -> String {
    match (provider, policy) {
        ("auto", "local-only") => "local providers only (auto selection)".to_string(),
        ("auto", _) => "local providers only (auto selection)".to_string(),
        ("apple-speech", _) => "macOS system-native provider".to_string(),
        ("whisper-cpp", _) => "user-managed local model".to_string(),
        (other, _) => format!("custom provider: {other}"),
    }
}

fn semantic_route_label(provider: &str) -> String {
    match provider {
        "ollama" => "user-managed local model (Ollama)".to_string(),
        "gemini" => "user-configured Gemini API".to_string(),
        other => format!("custom provider: {other}"),
    }
}

fn provider_class_label(class: &ProviderClass) -> &'static str {
    match class {
        ProviderClass::SystemNative => "System native",
        ProviderClass::UserManagedLocal => "Local model",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "ready" } else { "not ready" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    struct FakeProbe {
        os: String,
        arch: String,
        memory: Option<u64>,
        disk: Option<u64>,
        commands: HashSet<String>,
        files: HashSet<String>,
        env: HashMap<String, String>,
    }

    impl FakeProbe {
        fn mac(memory_gib: u64, disk_gib: u64) -> Self {
            Self {
                os: "macos".to_string(),
                arch: "aarch64".to_string(),
                memory: Some(memory_gib * GIB),
                disk: Some(disk_gib * GIB),
                commands: ["ffmpeg", "ffprobe", "magick", "curl"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                files: HashSet::new(),
                env: HashMap::new(),
            }
        }
    }

    impl Probe for FakeProbe {
        fn os(&self) -> String {
            self.os.clone()
        }
        fn os_version(&self) -> Option<String> {
            Some("15.0".to_string())
        }
        fn architecture(&self) -> String {
            self.arch.clone()
        }
        fn memory_bytes(&self) -> Option<u64> {
            self.memory
        }
        fn available_disk_bytes(&self) -> Option<u64> {
            self.disk
        }
        fn command_exists(&self, name: &str) -> bool {
            self.commands.contains(name)
        }
        fn file_exists(&self, path: &str) -> bool {
            self.files.contains(path)
        }
        fn env(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
    }

    #[test]
    fn clean_mac_does_not_claim_unshipped_apple_host() {
        let report = build_report(&FakeProbe::mac(16, 100));
        let apple = report
            .providers
            .iter()
            .find(|provider| provider.id == "apple-speech")
            .unwrap();
        assert_eq!(apple.status, "provider-host-not-installed");
        assert!(!apple.capabilities[0].ready);
        assert!(
            apple
                .notes
                .iter()
                .any(|note| note.contains("not reported as available"))
        );
    }

    #[test]
    fn local_model_fit_is_derived_from_device_resources() {
        let report = build_report(&FakeProbe::mac(16, 100));
        let whisper = report
            .local_models
            .iter()
            .find(|model| model.id == "whisper-base")
            .unwrap();
        let gemma = report
            .local_models
            .iter()
            .find(|model| model.id == "gemma4")
            .unwrap();
        assert_eq!(whisper.device_fit, "recommended");
        assert_eq!(gemma.device_fit, "possible");
        assert_eq!(gemma.install_state, "not-installed");
        assert!(!gemma.docker_required);
    }

    #[test]
    fn installed_local_stack_makes_local_model_profile_ready() {
        let mut probe = FakeProbe::mac(32, 100);
        probe.commands.extend(
            ["video-perception-apple", "ollama", "video-perception-vlm"]
                .into_iter()
                .map(str::to_string),
        );
        let report = build_report(&probe);
        let local_model = report
            .profiles
            .iter()
            .find(|profile| profile.id == "local-model")
            .unwrap();
        let gemma = report
            .local_models
            .iter()
            .find(|model| model.id == "gemma4")
            .unwrap();
        assert!(local_model.full_profile_ready);
        assert!(local_model.blockers.is_empty());
        assert_eq!(gemma.install_state, "runtime-present-model-unverified");
    }

    #[test]
    fn setup_profiles_do_not_encode_consumer_configuration() {
        let report = build_report(&FakeProbe::mac(24, 100));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("acme-consumer"));
        assert!(!json.contains("rubric"));
        assert!(!json.contains("threshold"));
    }

    #[test]
    fn parses_memory_when_sandbox_blocks_sysctl() {
        let output = "Hardware Overview:\n\n      Chip: Apple M4\n      Memory: 32 GB\n";
        assert_eq!(parse_system_profiler_memory(output), Some(32 * GIB));
    }

    #[test]
    fn routing_defaults_are_task_specific_and_local_only() {
        let report = build_report(&FakeProbe::mac(24, 100));

        assert_eq!(report.routing.deterministic_video, "local");
        assert_eq!(report.routing.transcript_provider, "auto");
        assert_eq!(report.routing.transcript_policy, "local-only");
        assert_eq!(report.routing.semantic_provider, "ollama");
        assert!(!report.routing.remote_media_possible);
        assert_eq!(report.routing.source, "built-in-defaults");
        assert!(report.routing.overrides.is_empty());
    }

    #[test]
    fn routing_environment_overrides_are_reported() {
        let mut probe = FakeProbe::mac(24, 100);
        probe.env.extend([
            (
                "VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER".to_string(),
                "auto".to_string(),
            ),
            (
                "VIDEO_PERCEPTION_TRANSCRIPT_POLICY".to_string(),
                "local-only".to_string(),
            ),
            (
                "VIDEO_PERCEPTION_VLM_BACKEND".to_string(),
                "ollama".to_string(),
            ),
        ]);
        let report = build_report(&probe);

        assert_eq!(report.routing.source, "environment-overrides");
        assert!(!report.routing.remote_media_possible);
        assert_eq!(report.routing.overrides.len(), 3);
    }
}
