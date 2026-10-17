/// Sensor registry — single source of truth for all sensor metadata.
///
/// Drives: `video-perception catalog list`, SensePlan dependency resolution,
/// execution planning, and Observation generation.
use std::collections::HashSet;

// ── Types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Rust,
    Python(&'static str), // tool name on PATH, e.g. "video-perception-spectral"
    Vlm(&'static str),    // task name for video-perception-vlm, e.g. "gibberish-text"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Scene,
}

pub struct SensorDef {
    pub id: &'static str,
    pub detects: &'static str,
    pub scope: Scope,
    pub deps: &'static [&'static str],
    pub backend: Backend,
    // Execution metadata
    pub output_file: Option<&'static str>, // e.g. "blur.json"; None if the sensor writes only to assets
    pub deck_key: Option<&'static str>,    // key in deck.json, None if not in deck
    pub required: bool,                    // true = fail pipeline on error, false = best-effort
    pub sub_dir: Option<&'static str>,     // working subdir (e.g. "stats", "layout")
    pub needs_scenes: bool,                // needs --scenes arg
    pub needs_keyframe_dir: bool,          // needs --keyframe-dir (from stats/)
    pub needs_out_dir: bool,               // needs --out-dir (sub_dir)
    pub conditional_on: Option<&'static str>, // only run if this sensor succeeded
}

/// How a logical capability can participate in an incremental session.
///
/// `strategy` describes the correct execution shape even when the current
/// runtime has not shipped that adapter yet. `implemented` is the honest
/// readiness bit consumed by `catalog` and the stream planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingProfile {
    pub strategy: &'static str,
    pub implemented: bool,
    pub providers: &'static [&'static str],
    pub default_window_ms: Option<u64>,
    pub expected_first_event_ms: Option<u64>,
    pub parallelism: &'static str,
    pub notes: &'static str,
}

// ── Registry ───────────────────────────────────────────────────

pub static REGISTRY: &[SensorDef] = &[
    SensorDef {
        id: "probe",
        detects: "video metadata (duration, resolution, codec, fps, audio tracks)",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("metadata.json"),
        deck_key: Some("metadata"),
        required: true,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "scenes",
        detects: "scene boundaries (cuts, transitions) with short-scene merging",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("scenes.json"),
        deck_key: Some("scenes"),
        required: true,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "keyframes",
        detects: "fixed-interval keyframes at 2fps for visual analysis (scene-independent)",
        scope: Scope::Global,
        deps: &["probe"],
        backend: Backend::Rust,
        output_file: Some("keyframes.json"),
        deck_key: Some("keyframes"),
        required: false,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "stats",
        detects: "per-scene visual statistics (brightness, contrast, saturation, colors) + keyframes",
        scope: Scope::Scene,
        deps: &["scenes"],
        backend: Backend::Rust,
        output_file: Some("scene_stats.json"),
        deck_key: Some("scene_stats"),
        required: true,
        sub_dir: Some("stats"),
        needs_scenes: true,
        needs_keyframe_dir: false,
        needs_out_dir: true,
        conditional_on: None,
    },
    SensorDef {
        id: "loudnorm",
        detects: "global audio loudness (integrated LUFS, true peak, loudness range)",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("audio.json"),
        deck_key: Some("audio"),
        required: false,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "scene-audio",
        detects: "per-scene audio loudness (LUFS, true peak per scene)",
        scope: Scope::Scene,
        deps: &["scenes", "loudnorm"],
        backend: Backend::Rust,
        output_file: Some("scene_audio.json"),
        deck_key: Some("scene_audio"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: Some("loudnorm"),
    },
    SensorDef {
        id: "layout",
        detects: "per-scene layout (split-screen, picture-in-picture, letterbox/pillarbox)",
        scope: Scope::Scene,
        deps: &["scenes", "stats"],
        backend: Backend::Rust,
        output_file: Some("layout.json"),
        deck_key: Some("layout"),
        required: false,
        sub_dir: Some("layout"),
        needs_scenes: true,
        needs_keyframe_dir: true,
        needs_out_dir: true,
        conditional_on: None,
    },
    SensorDef {
        id: "track",
        detects: "per-scene subject motion (intensity, class, dominant direction)",
        scope: Scope::Scene,
        deps: &["scenes"],
        backend: Backend::Rust,
        output_file: Some("track.json"),
        deck_key: Some("track"),
        required: false,
        sub_dir: Some("track"),
        needs_scenes: true,
        needs_keyframe_dir: false,
        needs_out_dir: true,
        conditional_on: None,
    },
    SensorDef {
        id: "beats",
        detects: "beat grid (BPM, beat timestamps, time signature) from audio",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("beats.json"),
        deck_key: Some("beats"),
        required: false,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "transcript",
        detects: "timestamped speech transcript (utterances with word-level timing)",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("transcript.json"),
        deck_key: Some("transcript"),
        required: false,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "blur",
        detects: "per-scene sharpness (Laplacian variance, FFT high-frequency ratio)",
        scope: Scope::Scene,
        deps: &["scenes", "stats"],
        backend: Backend::Rust,
        output_file: Some("blur.json"),
        deck_key: Some("blur"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: true,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "complexity",
        detects: "per-scene visual complexity (edge density, entropy, compression ratio)",
        scope: Scope::Scene,
        deps: &["scenes", "stats"],
        backend: Backend::Rust,
        output_file: Some("complexity.json"),
        deck_key: Some("complexity"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: true,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "framerate",
        detects: "frame rate analysis (actual fps, 3:2 pulldown, duplicate frames, interlace)",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("framerate.json"),
        deck_key: Some("framerate"),
        required: false,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "flicker",
        detects: "luminance flash detection (ITU-R BT.1702 compliance, flash rate)",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("flicker.json"),
        deck_key: Some("flicker"),
        required: false,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "signalstats",
        detects: "per-scene frame-level signal statistics (luminance, variance, inter-frame diff)",
        scope: Scope::Scene,
        deps: &["scenes"],
        backend: Backend::Rust,
        output_file: Some("signalstats.json"),
        deck_key: Some("signalstats"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "avsync",
        detects: "audio-visual sync offset (energy envelope cross-correlation)",
        scope: Scope::Global,
        deps: &[],
        backend: Backend::Rust,
        output_file: Some("avsync.json"),
        deck_key: Some("avsync"),
        required: false,
        sub_dir: None,
        needs_scenes: false,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "spectral",
        detects: "per-scene audio spectral features (centroid, bandwidth, MFCCs, texture)",
        scope: Scope::Scene,
        deps: &["scenes"],
        backend: Backend::Python("video-perception-spectral"),
        output_file: Some("spectral.json"),
        deck_key: Some("spectral"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "audio-class",
        detects: "per-scene audio classification (speech, music, silence, noise)",
        scope: Scope::Scene,
        deps: &["scenes"],
        backend: Backend::Python("video-perception-audio-class"),
        output_file: Some("audio_class.json"),
        deck_key: Some("audio_class"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: false,
        needs_out_dir: false,
        conditional_on: None,
    },
    // ── VLM sensors (default: local Ollama; VIDEO_PERCEPTION_VLM_BACKEND can override) ──
    SensorDef {
        id: "gibberish-text",
        detects: "garbled/unreadable text in video frames (AI rendering failures)",
        scope: Scope::Scene,
        deps: &["scenes", "stats"],
        backend: Backend::Vlm("gibberish-text"),
        output_file: Some("gibberish_text.json"),
        deck_key: Some("gibberish_text"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: true,
        needs_out_dir: false,
        conditional_on: None,
    },
    SensorDef {
        id: "scene-description",
        detects: "semantic content of a scene (people, objects, setting, action, mood)",
        scope: Scope::Scene,
        deps: &["scenes", "stats"],
        backend: Backend::Vlm("scene-description"),
        output_file: Some("scene_description.json"),
        deck_key: Some("scene_description"),
        required: false,
        sub_dir: None,
        needs_scenes: true,
        needs_keyframe_dir: true,
        needs_out_dir: false,
        conditional_on: None,
    },
];

// ── Lookups ────────────────────────────────────────────────────

pub fn find(id: &str) -> Option<&'static SensorDef> {
    REGISTRY.iter().find(|s| s.id == id)
}

/// Resolve a requested capability set into an execution plan.
/// Returns sensors in registry order (which is dependency order).
pub fn resolve_plan(capabilities: &[String]) -> Result<Vec<&'static SensorDef>, String> {
    for id in capabilities {
        if find(id).is_none() {
            return Err(format!(
                "Unknown capability: {id}. Run `video-perception catalog list`."
            ));
        }
    }
    let mut wanted: HashSet<&str> = capabilities.iter().map(String::as_str).collect();

    // Auto-add transitive deps
    loop {
        let mut added = false;
        for sensor in REGISTRY {
            if wanted.contains(sensor.id) {
                for dep in sensor.deps {
                    if !wanted.contains(dep) {
                        wanted.insert(dep);
                        added = true;
                    }
                }
            }
        }
        if !added {
            break;
        }
    }

    // Return in registry order (which is dependency-safe)
    let plan: Vec<&SensorDef> = REGISTRY.iter().filter(|s| wanted.contains(s.id)).collect();

    Ok(plan)
}

// ── Display ────────────────────────────────────────────────────

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::Rust => write!(f, "rust"),
            Backend::Python(_) => write!(f, "python"),
            Backend::Vlm(_) => write!(f, "vlm"),
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::Global => write!(f, "global"),
            Scope::Scene => write!(f, "scene"),
        }
    }
}

/// Streaming context for every registered capability.
///
/// Batch dependencies are deliberately not reused here: an Observation needs
/// complete metadata and stable scenes, while a stream may emit useful evidence
/// before either exists. The streaming runtime therefore validates only the
/// explicitly requested capabilities and their streaming adapters.
pub fn streaming_profile(id: &str) -> StreamingProfile {
    match id {
        "keyframes" => StreamingProfile {
            strategy: "native",
            implemented: true,
            providers: &["local-ffmpeg"],
            default_window_ms: Some(500),
            expected_first_event_ms: Some(750),
            parallelism: "one worker per session",
            notes: "Emits a durable frame asset as soon as each sampled frame is available.",
        },
        "signalstats" => StreamingProfile {
            strategy: "native",
            implemented: true,
            providers: &["local-ffmpeg"],
            default_window_ms: Some(500),
            expected_first_event_ms: Some(750),
            parallelism: "one worker per session",
            notes: "Emits frame-level luminance and saturation measurements incrementally.",
        },
        "transcript" => StreamingProfile {
            strategy: "windowed",
            implemented: false,
            providers: &["apple-speech", "whisper-cpp"],
            default_window_ms: Some(4_000),
            expected_first_event_ms: None,
            parallelism: "provider-dependent",
            notes: "Requires provider-specific partial/final transcript reconciliation.",
        },
        "scenes" | "stats" | "layout" | "track" | "scene-audio" | "blur" | "complexity"
        | "spectral" | "audio-class" => StreamingProfile {
            strategy: "windowed",
            implemented: false,
            providers: &["local-ffmpeg"],
            default_window_ms: Some(4_000),
            expected_first_event_ms: None,
            parallelism: "planned",
            notes: "Needs provisional windows and later reconciliation before it is stream-safe.",
        },
        "gibberish-text" | "scene-description" => StreamingProfile {
            strategy: "windowed",
            implemented: false,
            providers: &["ollama", "gemini"],
            default_window_ms: Some(4_000),
            expected_first_event_ms: None,
            parallelism: "provider-dependent",
            notes: "The event contract is ready; stateful local or user-configured model sessions are not shipped yet.",
        },
        "flicker" => StreamingProfile {
            strategy: "native",
            implemented: false,
            providers: &["local-ffmpeg"],
            default_window_ms: Some(1_000),
            expected_first_event_ms: None,
            parallelism: "planned",
            notes: "The current implementation aggregates the complete asset before returning.",
        },
        "probe" | "loudnorm" | "beats" | "framerate" | "avsync" => StreamingProfile {
            strategy: "finalize-only",
            implemented: false,
            providers: &["local-ffmpeg"],
            default_window_ms: None,
            expected_first_event_ms: None,
            parallelism: "not applicable",
            notes: "The stable result depends on end-of-stream or complete-asset context.",
        },
        _ => StreamingProfile {
            strategy: "unsupported",
            implemented: false,
            providers: &[],
            default_window_ms: None,
            expected_first_event_ms: None,
            parallelism: "not applicable",
            notes: "No streaming execution shape is registered.",
        },
    }
}
