/**
 * Signal schemas — deterministic sensor pipeline output.
 * Produced once per video, consumed by all agents.
 */
import { z } from "zod/v4";

// ── Primitives ──────────────────────────────────────────────

/** Millisecond timestamp */
const ms = z.number().int().nonnegative();

/** Time range in ms */
export const TimeRange = z.object({
  start_ms: ms,
  end_ms: ms,
});

/** Hex color string */
const hexColor = z.string().regex(/^#[0-9a-fA-F]{6}$/);

// ── ① Metadata ──────────────────────────────────────────────

export const Metadata = z.object({
  video: z.string(),
  duration_ms: ms,
  width: z.number().int().positive(),
  height: z.number().int().positive(),
  fps: z.number().positive(),
  codec: z.string(),
  audio_tracks: z.number().int().nonnegative(),
  filesize_bytes: z.number().int().nonnegative(),
});

export type Metadata = z.infer<typeof Metadata>;

// ── ② Scenes ────────────────────────────────────────────────

export const Scene = z.object({
  idx: z.number().int().nonnegative(),
  start_ms: ms,
  end_ms: ms,
});

/** A visual change too weak to call a cut, such as a new slide with the same
 * layout or text appearing on an unchanged shot. Reported for inspection. */
export const WeakChange = z.object({
  ms,
  score: z.number().nonnegative(),
});

export const Scenes = z.object({
  video: z.string(),
  duration_ms: ms,
  scene_count: z.number().int().positive(),
  merge_threshold_ms: ms,
  scenes: z.array(Scene),
  weak_changes: z.array(WeakChange).default([]),
});

export type Scene = z.infer<typeof Scene>;
export type Scenes = z.infer<typeof Scenes>;

// ── ④ Per-scene stats (signalstats + color) ─────────────────

export const SceneStats = z.object({
  scene_idx: z.number().int().nonnegative(),
  start_ms: ms,
  end_ms: ms,

  // Exposure
  avg_brightness: z.number().min(0).max(255),
  min_brightness: z.number().min(0).max(255),
  max_brightness: z.number().min(0).max(255),
  contrast: z.number().min(0).max(1),

  // Color
  saturation: z.number().min(0).max(1),
  dominant_colors: z.array(hexColor).max(5),
  color_temp_k: z.number().int().positive().optional(),

  // Files
  keyframe: z.string(),
});

export type SceneStats = z.infer<typeof SceneStats>;

// ── ⑤ Layout (per-scene frame layout detection) ─────────────

export const LayoutBars = z.object({
  top: z.number().int().nonnegative(),
  bottom: z.number().int().nonnegative(),
  left: z.number().int().nonnegative(),
  right: z.number().int().nonnegative(),
});

export const LayoutRegion = z.object({
  x: z.number().int().nonnegative(),
  y: z.number().int().nonnegative(),
  w: z.number().int().nonnegative(),
  h: z.number().int().nonnegative(),
});

export const SplitDiagnostics = z.object({
  axis: z.enum(["horizontal", "vertical"]),
  luma_delta: z.number().nonnegative(),
  color_distance: z.number().nonnegative(),
});

export const SceneLayout = z.object({
  scene_idx: z.number().int().nonnegative(),
  start_ms: ms,
  end_ms: ms,
  layout: z.enum(["standard", "letterbox", "pillarbox", "split_v", "split_h"]),
  bars: LayoutBars,
  content_region: LayoutRegion,
  split_diagnostics: SplitDiagnostics.optional(),
});

export type SceneLayout = z.infer<typeof SceneLayout>;
export type SplitDiagnostics = z.infer<typeof SplitDiagnostics>;

// ── ⑥ Audio ─────────────────────────────────────────────────

export const Audio = z.object({
  input_i: z.number(), // integrated loudness (LUFS)
  input_tp: z.number(), // true peak (dBFS)
  input_lra: z.number(), // loudness range (LU)
  input_thresh: z.number(),
  sample_rate: z.number().int().positive(),
  channels: z.number().int().positive(),
  duration_ms: ms,
});

export type Audio = z.infer<typeof Audio>;

// ── ⑦ Track (per-scene motion analysis) ─────────────────────

export const MotionSegment = z.object({
  from_pct: z.number().int().min(0).max(100),
  to_pct: z.number().int().min(0).max(100),
  intensity: z.number().nonnegative(),
});

export const SceneTrack = z.object({
  scene_idx: z.number().int().nonnegative(),
  start_ms: ms,
  end_ms: ms,
  motion_intensity: z.number().nonnegative(),
  motion_class: z.enum(["static", "subtle", "moderate", "dynamic", "chaotic"]),
  dominant_direction: z.enum([
    "none",
    "uniform",
    "left_to_right",
    "right_to_left",
    "top_to_bottom",
    "bottom_to_top",
  ]),
  segments: z.array(MotionSegment),
});

export type MotionSegment = z.infer<typeof MotionSegment>;
export type SceneTrack = z.infer<typeof SceneTrack>;

// ── ⑧ Scene audio (per-scene loudness features) ─────────────

export const SceneAudio = z.object({
  scene_idx: z.number().int().nonnegative(),
  start_ms: ms,
  end_ms: ms,
  input_i: z.number(), // integrated loudness (LUFS) per scene
  input_tp: z.number(), // true peak (dBFS)
  input_lra: z.number(), // loudness range (LU)
  input_thresh: z.number(),
});

export type SceneAudio = z.infer<typeof SceneAudio>;

// ── ⑨ Beats ─────────────────────────────────────────────────

export const Beat = z.object({
  time_ms: ms,
  strength: z.number().min(0).max(1).optional(),
});

export const Beats = z.object({
  bpm: z.number().positive(),
  time_signature: z.string().optional(), // e.g. "4/4"
  beats: z.array(Beat),
  downbeats: z.array(Beat).optional(),
});

export type Beat = z.infer<typeof Beat>;
export type Beats = z.infer<typeof Beats>;

// ── ⑩ Transcript ────────────────────────────────────────────

export const Word = z.object({
  text: z.string(),
  start_ms: ms,
  end_ms: ms,
  confidence: z.number().min(0).max(1).optional(),
});

export const Utterance = z.object({
  speaker: z.string().optional(),
  text: z.string(),
  start_ms: ms,
  end_ms: ms,
  words: z.array(Word).optional(),
});

export const TranscriptProviderAttempt = z.object({
  provider: z.string().min(1),
  outcome: z.enum(["unavailable", "failed", "selected"]),
  reason: z.string().optional(),
});

export const TranscriptProvenance = z.object({
  provider: z.string().min(1),
  locality: z.enum(["local", "remote"]),
  model: z.string().optional(),
  timing: z.enum(["aligned", "estimated", "unknown"]),
  attempts: z.array(TranscriptProviderAttempt),
});

export const Transcript = z.object({
  // Optional only for reading observations produced before provider routing.
  // New Sensor runs always write it.
  provenance: TranscriptProvenance.optional(),
  language: z.string(),
  utterances: z.array(Utterance),
});

export type Word = z.infer<typeof Word>;
export type Utterance = z.infer<typeof Utterance>;
export type TranscriptProviderAttempt = z.infer<typeof TranscriptProviderAttempt>;
export type TranscriptProvenance = z.infer<typeof TranscriptProvenance>;
export type Transcript = z.infer<typeof Transcript>;

// ── ⑪ Blur (per-scene sharpness analysis) ───────────────────

export const SceneBlur = z.object({
  scene_idx: z.number().int().nonnegative(),
  laplacian_variance: z.number().nonnegative(),
  fft_high_freq_ratio: z.number().min(0).max(1),
  sharpness_class: z.enum(["blurry", "soft", "sharp"]),
});

export type SceneBlur = z.infer<typeof SceneBlur>;

// ── ⑫ Complexity (per-scene visual complexity) ──────────────

export const SceneComplexity = z.object({
  scene_idx: z.number().int().nonnegative(),
  edge_density: z.number().min(0).max(1),
  shannon_entropy: z.number().nonnegative(),
  jpeg_compression_ratio: z.number().nonnegative(),
  complexity_class: z.enum(["minimal", "low", "moderate", "high"]),
});

export type SceneComplexity = z.infer<typeof SceneComplexity>;

// ── ⑬ Framerate (global frame rate analysis) ────────────────

export const FramerateAnalysis = z.object({
  container_fps: z.number().nonnegative(),
  duplicate_frame_count: z.number().int().nonnegative(),
  total_frames: z.number().int().nonnegative(),
  duplicate_ratio: z.number().min(0).max(1),
  interlace_detected: z.boolean(),
  interlace_type: z.enum(["progressive", "top_field_first", "bottom_field_first"]),
  content_fps_estimate: z.number().nonnegative(),
  pulldown_detected: z.boolean(),
});

export type FramerateAnalysis = z.infer<typeof FramerateAnalysis>;

// ── ⑭ Flicker (luminance flash detection) ───────────────────

export const FlickerFrame = z.object({
  frame: z.number().int().nonnegative(),
  time_ms: ms,
  luminance_delta: z.number().nonnegative(),
});

export const FlickerAnalysis = z.object({
  total_frames_analyzed: z.number().int().nonnegative(),
  max_luminance_delta: z.number().nonnegative(),
  flash_frames: z.array(FlickerFrame),
  flash_count: z.number().int().nonnegative(),
  max_flash_frequency_hz: z.number().nonnegative(),
  bt1702_compliant: z.boolean(),
});

export type FlickerFrame = z.infer<typeof FlickerFrame>;
export type FlickerAnalysis = z.infer<typeof FlickerAnalysis>;

// ── ⑮ Signal Stats (per-scene frame-level measurements) ─────

export const SceneSignalStats = z.object({
  scene_idx: z.number().int().nonnegative(),
  frame_count: z.number().int().nonnegative(),

  yavg_mean: z.number(),
  yavg_std: z.number().nonnegative(),
  yavg_min: z.number(),
  yavg_max: z.number(),

  yrange_mean: z.number().nonnegative(),
  yrange_min: z.number().nonnegative(),

  satavg_mean: z.number().nonnegative(),

  frame_diff_mean: z.number().nonnegative(),
  frame_diff_max: z.number().nonnegative(),
  frame_diff_std: z.number().nonnegative(),

  dark_frame_ratio: z.number().min(0).max(1),
  bright_frame_ratio: z.number().min(0).max(1),
  low_variance_ratio: z.number().min(0).max(1),
  static_frame_ratio: z.number().min(0).max(1),
});

export type SceneSignalStats = z.infer<typeof SceneSignalStats>;

// ── ⑯ A/V Sync (audio-visual offset measurement) ───────────

export const AVSyncReport = z.object({
  audio_duration_ms: ms,
  video_duration_ms: ms,
  duration_diff_ms: z.number().int(),
  estimated_offset_ms: z.number().int(),
  correlation_peak: z.number(),
  audio_envelope_samples: z.number().int().nonnegative(),
  visual_envelope_samples: z.number().int().nonnegative(),
  has_audio: z.boolean(),
});

export type AVSyncReport = z.infer<typeof AVSyncReport>;

// ── ⑰ Spectral (per-scene audio spectral features) ─────────

export const SceneSpectral = z.object({
  scene_idx: z.number().int().nonnegative(),
  spectral_centroid_hz: z.number().nonnegative(),
  spectral_centroid_std: z.number().nonnegative(),
  spectral_bandwidth_hz: z.number().nonnegative(),
  spectral_rolloff_hz: z.number().nonnegative(),
  spectral_flatness: z.number().nonnegative(),
  zero_crossing_rate: z.number().nonnegative(),
  rms_energy: z.number().nonnegative(),
  rms_energy_db: z.number(),
  mfcc_means: z.array(z.number()).length(13),
  audio_texture: z.enum(["bright", "dark", "balanced", "noisy"]).optional(),
});

export type SceneSpectral = z.infer<typeof SceneSpectral>;

// ── ⑱ Audio Classification (per-scene speech/music/silence) ─

export const SceneAudioClass = z.object({
  scene_idx: z.number().int().nonnegative(),
  class: z.enum(["speech", "music", "silence", "noise", "speech_over_music"]),
  confidence: z.number().min(0).max(1),
  features: z.object({
    rms_db: z.number(),
    zcr: z.number().nonnegative(),
    spectral_centroid_hz: z.number().nonnegative(),
    spectral_flatness: z.number().nonnegative(),
    modulation_4hz: z.number().nonnegative(),
  }).optional(),
});

export type SceneAudioClass = z.infer<typeof SceneAudioClass>;

// ── Characters (face/body detection per scene) ──────────────

export const CharacterDetection = z.object({
  character_id: z.string(),
  scene_idx: z.number().int().nonnegative(),
  confidence: z.number().min(0).max(1),
  bbox: z.object({
    x: z.number(), y: z.number(), w: z.number(), h: z.number(),
  }).optional(),
  face_crop: z.string().optional(),       // path to cropped face image
});

export const Character = z.object({
  id: z.string(),
  label: z.string().optional(),           // e.g., "Person A", "Dog"
  scenes: z.array(z.number().int()),      // scene indices where detected
  face_crops: z.array(z.string()),        // paths to best face crops
  description: z.string().optional(),     // VLM-generated appearance description
});

export const Characters = z.object({
  version: z.literal("1.0"),
  detection_method: z.string(),           // "ffmpeg_face_detect" | "nanobanana2"
  total_detections: z.number().int(),
  characters: z.array(Character),
  detections: z.array(CharacterDetection),
});

export type CharacterDetection = z.infer<typeof CharacterDetection>;
export type Character = z.infer<typeof Character>;
export type Characters = z.infer<typeof Characters>;

// ── Observation manifest (stable consumer boundary) ─────────

export const CapabilityRequest = z.object({
  id: z.string().min(1),
  required: z.boolean().optional(),
  provider: z.string().min(1).optional(),
  policy: z.string().min(1).optional(),
  language: z.string().min(1).optional(),
  options: z.object({
    threshold: z.number().min(0).max(1).optional(),
    merge_ms: ms.optional(),
    merge: z.boolean().optional(),
  }).optional(),
});

export const ModelBackendConfig = z.object({
  provider: z.string().min(1),
  model: z.string().min(1).optional(),
}).refine(
  (backend) => ["ollama", "gemini"].includes(backend.provider) || backend.model !== undefined,
  "A custom model backend requires an explicit model",
);

export const SensePlan = z.object({
  contract: z.literal("video-perception.sense-plan/v1"),
  capabilities: z.array(CapabilityRequest).min(1),
  model_backend: ModelBackendConfig.optional(),
});

export const ObservationProvider = z.object({
  id: z.string().min(1),
  locality: z.enum(["local", "remote"]),
  model: z.string().optional(),
  timing: z.enum(["aligned", "estimated", "unknown"]).optional(),
});

export const ObservationCapability = z.object({
  id: z.string().min(1),
  scope: z.enum(["global", "scene"]),
  backend: z.enum(["rust", "python", "vlm"]),
  requested: z.boolean(),
  required: z.boolean(),
  status: z.enum(["succeeded", "failed", "skipped"]),
  output: z.string().startsWith("signals/").optional(),
  error: z.string().min(1).optional(),
  provider: ObservationProvider.optional(),
});

export const ObservationManifest = z.object({
  contract: z.literal("video-perception.observation/v1"),
  version: z.literal("1.0"),
  sense_plan: SensePlan,
  asset: z.object({
    uri: z.string(),
    duration_ms: ms,
    resolution: z.object({
      width: z.number().int().nonnegative(),
      height: z.number().int().nonnegative(),
    }),
    fps: z.number().nonnegative(),
  }),
  scene_count: z.number().int().nonnegative(),
  capabilities: z.array(ObservationCapability),
  files: z.object({
    dashboard: z.literal("dashboard.json"),
    scenes: z.literal("scenes.jsonl"),
    transcript: z.literal("transcript.jsonl").nullable(),
  }),
});

export type ObservationCapability = z.infer<typeof ObservationCapability>;
export type ObservationProvider = z.infer<typeof ObservationProvider>;
export type ObservationManifest = z.infer<typeof ObservationManifest>;
export type CapabilityRequest = z.infer<typeof CapabilityRequest>;
export type ModelBackendConfig = z.infer<typeof ModelBackendConfig>;
export type SensePlan = z.infer<typeof SensePlan>;

// ── Signal Deck (complete output of sensor pipeline) ────────

export const SignalDeck = z.object({
  version: z.literal("1.0"),
  video: z.string(),
  created_at: z.string().datetime(),

  metadata: Metadata,
  scenes: Scenes,
  scene_stats: z.array(SceneStats),
  audio: Audio.optional(),
  scene_audio: z.array(SceneAudio).optional(),
  layout: z.array(SceneLayout).optional(),
  track: z.array(SceneTrack).optional(),
  beats: Beats.optional(),
  transcript: Transcript.optional(),
  blur: z.array(SceneBlur).optional(),
  complexity: z.array(SceneComplexity).optional(),
  framerate: FramerateAnalysis.optional(),
  flicker: FlickerAnalysis.optional(),
  signalstats: z.array(SceneSignalStats).optional(),
  avsync: AVSyncReport.optional(),
  spectral: z.array(SceneSpectral).optional(),
  audio_class: z.array(SceneAudioClass).optional(),
  characters: Characters.optional(),
});

export type SignalDeck = z.infer<typeof SignalDeck>;
