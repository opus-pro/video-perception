"""Audio analysis and beat-grid generation."""

from __future__ import annotations

import datetime as dt
import importlib
import json
import math
import os
from dataclasses import asdict, dataclass
from html import escape
from pathlib import Path
from typing import Any
from urllib.parse import quote

import librosa
import numpy as np

from .errors import AutobeatError


@dataclass(frozen=True)
class Span:
    """A timeline span between adjacent anchors."""

    i: int
    start: float
    end: float
    dur: float


@dataclass(frozen=True)
class LooseDensityConfig:
    """Adaptive loose-grid density decision."""

    effective_stride: int
    median_beat_sec: float
    estimated_span_sec: float
    adapted: bool


@dataclass(frozen=True)
class LoosePhaseConfig:
    """Phase alignment decision for loose anchors."""

    phase_offset: int
    strategy: str
    score: float


@dataclass(frozen=True)
class PhaseReferenceConfig:
    """Beat index used as local phase reference for beat numbering."""

    start_index: int
    start_sec: float
    method: str


# ─────────────────────────────────────────────────────────────────────────────
# Multi-strategy constants
# ─────────────────────────────────────────────────────────────────────────────

VALID_INTENTS: frozenset[str] = frozenset({"auto", "vlog", "montage", "transition", "energy"})

#: Strategy preference order per intent (highest priority first).
INTENT_PREFS: dict[str, list[str]] = {
    "auto":       ["beat_loose", "beat_tight", "energy", "onset_cluster"],
    "vlog":       ["beat_loose", "energy", "onset_cluster", "beat_tight"],
    "montage":    ["beat_tight", "beat_loose", "onset_cluster", "energy"],
    "transition": ["energy", "onset_cluster", "beat_loose", "beat_tight"],
    "energy":     ["energy", "onset_cluster", "beat_loose", "beat_tight"],
}

STRATEGY_USE_WHEN: dict[str, str] = {
    "beat_loose":    "bar-level beat cuts (~1/bar); good for vlog natural pacing",
    "beat_tight":    "per-beat cuts; good for montage, fast transitions",
    "energy":        "energy peak cuts; good for ambient, cinematic, non-rhythmic tracks",
    "onset_cluster": "transient burst cuts; good for percussive or irregular-rhythm tracks",
}


@dataclass
class StrategyResult:
    """Result from a single grid strategy."""

    strategy: str
    anchors: np.ndarray
    spans: list[Any]
    quality_score: float
    analysis_meta: dict[str, Any]
    error: str | None = None

    @property
    def quality_tier(self) -> str:
        if self.quality_score >= 0.65:
            return "high"
        if self.quality_score >= 0.35:
            return "medium"
        return "low"


def _closeness(x: float, center: float, width: float) -> float:
    if width <= 0:
        return 0.0
    return float(np.clip(1.0 - abs(x - center) / width, 0.0, 1.0))


def _to_float(value: Any) -> float:
    return float(np.asarray(value).item())


def _compute_confidence(
    *,
    flatness: float,
    perc_ratio: float,
    interval_cv: float,
    peak_prom: float,
) -> float:
    """Compute confidence score in [0, 1] from heuristic metrics."""
    conf = 1.0
    conf *= np.clip(1.2 - 2.0 * flatness, 0.0, 1.0)
    conf *= np.clip((perc_ratio - 0.05) / 0.35, 0.0, 1.0)
    conf *= np.clip(1.0 - (interval_cv / 0.20), 0.0, 1.0)
    # peak_prom at half-weight (sqrt): funk/soul/jazz tracks have energy at
    # sub/super-harmonic BPMs that compress the tempogram peak ratio without
    # indicating unclear rhythm. A stable interval_cv should outweigh this.
    raw_prom = np.clip((peak_prom - 1.02) / 0.50, 0.0, 1.0)
    conf *= float(np.sqrt(raw_prom))
    return float(np.clip(conf, 0.0, 1.0))


def _make_preview_waveform(y: np.ndarray, max_points: int) -> list[float]:
    if max_points < 100:
        raise AutobeatError("`preview_points` must be >= 100.")
    if y.size == 0:
        return []

    point_count = int(min(max_points, y.size))
    indices = np.linspace(0, y.size - 1, num=point_count, dtype=np.int64)
    sampled = y[indices].astype(np.float64)
    peak = float(np.max(np.abs(sampled)))
    if peak > 1e-12:
        sampled = sampled / peak
    return [float(v) for v in sampled.tolist()]


def _normalize_unit(x: np.ndarray) -> np.ndarray:
    arr = np.asarray(x, dtype=np.float64)
    if arr.size == 0:
        return arr
    arr = arr - float(np.min(arr))
    peak = float(np.max(arr))
    if peak > 1e-12:
        arr = arr / peak
    return arr


def _compute_low_band_envelope(
    *,
    y_perc: np.ndarray,
    sr: int,
    hop_length: int,
    n_fft: int = 2048,
    fmin: float = 40.0,
    fmax: float = 180.0,
) -> np.ndarray:
    """Compute normalized low-frequency energy envelope for kick/downbeat bias."""
    spec = np.abs(librosa.stft(y_perc, n_fft=n_fft, hop_length=hop_length))
    freqs = librosa.fft_frequencies(sr=sr, n_fft=n_fft)
    mask = (freqs >= fmin) & (freqs <= fmax)
    if not np.any(mask):
        return np.zeros(spec.shape[1], dtype=np.float64)
    low_env = np.mean(spec[mask, :], axis=0)
    return _normalize_unit(low_env)


def _build_phase_energy_profile(
    *,
    beat_frames: np.ndarray,
    onset_env: np.ndarray,
    y_perc: np.ndarray,
    sr: int,
    hop_length: int,
    stride: int,
    start_index: int = 0,
) -> list[dict[str, float | int]]:
    if stride <= 0:
        return []
    onset_norm = _normalize_unit(onset_env)
    low_env = _compute_low_band_envelope(
        y_perc=y_perc,
        sr=sr,
        hop_length=hop_length,
    )

    profile: list[dict[str, float | int]] = []
    beat_count = int(beat_frames.size)
    if beat_count == 0:
        return []
    start_idx = int(np.clip(start_index, 0, max(0, beat_count - 1)))

    for phase in range(stride):
        beat_idx = np.arange(start_idx + phase, beat_count, stride, dtype=np.int64)
        if beat_idx.size == 0:
            profile.append(
                {
                    "beat": int(phase + 1),
                    "sample_count": 0,
                    "low_energy": 0.0,
                    "onset_energy": 0.0,
                    "score": 0.0,
                }
            )
            continue
        indices = beat_frames[beat_idx]
        safe_indices = np.minimum(indices, low_env.size - 1)
        low_vals = low_env[safe_indices]
        onset_vals = onset_norm[np.minimum(indices, onset_norm.size - 1)]
        score = float(0.75 * np.mean(low_vals) + 0.25 * np.mean(onset_vals))
        profile.append(
            {
                "beat": int(phase + 1),
                "sample_count": int(beat_idx.size),
                "low_energy": float(np.mean(low_vals)),
                "onset_energy": float(np.mean(onset_vals)),
                "score": score,
            }
        )
    return profile


def _rank_phase_beats(profile: list[dict[str, float | int]]) -> list[int]:
    ordered = sorted(profile, key=lambda item: float(item["score"]), reverse=True)
    return [int(item["beat"]) for item in ordered]


def _build_anchor_candidates(
    *,
    beat_times: np.ndarray,
    stride: int,
    duration: float,
    offset_sec: float,
    phase_start_index: int = 0,
) -> dict[str, list[float]]:
    if stride <= 0:
        return {}
    beat_count = int(beat_times.size)
    if beat_count == 0:
        return {}
    start_idx = int(np.clip(phase_start_index, 0, max(0, beat_count - 1)))
    candidates: dict[str, list[float]] = {}
    for local_phase in range(stride):
        global_phase = int((start_idx + local_phase) % stride)
        anchors = beat_times[global_phase::stride] + float(offset_sec)
        anchors = anchors[(anchors >= 0.0) & (anchors <= duration)]
        if anchors.size == 0:
            continue
        anchors = np.unique(np.round(anchors, 6))
        candidates[str(local_phase + 1)] = [float(v) for v in anchors.tolist()]
    return candidates


def _estimate_genre_guess(
    *,
    tempo_bpm: float,
    perc_ratio: float,
    flatness: float,
    interval_cv: float,
    phase_profile: list[dict[str, float | int]],
) -> dict[str, Any]:
    """Rule-based genre tendency estimate (heuristic, non-ML)."""
    groove_tempo = tempo_bpm
    if groove_tempo > 140.0:
        groove_tempo = groove_tempo / 2.0
    elif groove_tempo < 70.0:
        groove_tempo = groove_tempo * 2.0

    score_map = {int(item["beat"]): float(item["score"]) for item in phase_profile}
    one = score_map.get(1, 0.0)
    two = score_map.get(2, 0.0)
    three = score_map.get(3, 0.0)
    four = score_map.get(4, 0.0)
    backbeat = ((two + four) - (one + three)) / 2.0

    speech_noise = np.clip(flatness * 1.1 + max(0.0, 0.22 - perc_ratio) * 2.8 + interval_cv * 1.8, 0.0, 1.0)
    funk = np.clip(
        0.45 * _closeness(groove_tempo, 102.0, 34.0)
        + 0.35 * np.clip(backbeat + 0.25, 0.0, 1.0)
        + 0.20 * np.clip((perc_ratio - 0.10) / 0.45, 0.0, 1.0),
        0.0,
        1.0,
    )
    house_edm = np.clip(
        0.50 * _closeness(groove_tempo, 124.0, 20.0)
        + 0.30 * np.clip((perc_ratio - 0.18) / 0.45, 0.0, 1.0)
        + 0.20 * np.clip(1.0 - interval_cv / 0.20, 0.0, 1.0),
        0.0,
        1.0,
    )
    hiphop = np.clip(
        0.45 * _closeness(groove_tempo, 86.0, 22.0)
        + 0.35 * np.clip((one + three) / 2.0, 0.0, 1.0)
        + 0.20 * np.clip((perc_ratio - 0.12) / 0.40, 0.0, 1.0),
        0.0,
        1.0,
    )
    pop_rock = np.clip(
        0.45 * _closeness(groove_tempo, 112.0, 36.0)
        + 0.35 * np.clip(backbeat + 0.20, 0.0, 1.0)
        + 0.20 * np.clip(1.0 - flatness * 1.7, 0.0, 1.0),
        0.0,
        1.0,
    )

    candidates = [
        ("speech-or-noise", float(speech_noise)),
        ("funk-groove", float(funk)),
        ("house-edm", float(house_edm)),
        ("hiphop", float(hiphop)),
        ("pop-rock", float(pop_rock)),
    ]
    candidates.sort(key=lambda item: item[1], reverse=True)
    top_name, top_score = candidates[0]

    return {
        "method": "heuristic-v1",
        "tempo_for_genre_bpm": float(groove_tempo),
        "primary": top_name,
        "confidence": float(np.clip(top_score, 0.0, 1.0)),
        "candidates": [
            {"label": label, "score": float(score)}
            for label, score in candidates[:3]
        ],
    }


def _build_preview_audio_src(preview_path: Path, audio_path: Path) -> str:
    """Build an HTML-safe audio src value for local preview playback."""
    preview_dir = preview_path.parent.resolve()
    audio_abs = audio_path.resolve()
    try:
        rel = Path(os.path.relpath(audio_abs, start=preview_dir))
        if not rel.parts or rel.parts[0] != "..":
            return quote(rel.as_posix(), safe="/-_.~")
    except Exception:
        pass
    return audio_abs.as_uri()


def _select_loose_density(
    *,
    beat_times: np.ndarray,
    beats_per_bar: int,
    adaptive_density: bool,
    min_span_sec: float,
    max_span_sec: float,
    target_span_sec: float,
    max_stride: int = 32,
) -> LooseDensityConfig:
    if beat_times.size < 2:
        return LooseDensityConfig(
            effective_stride=beats_per_bar,
            median_beat_sec=0.0,
            estimated_span_sec=0.0,
            adapted=False,
        )

    beat_interval = float(np.median(np.diff(beat_times)))
    if beat_interval <= 0:
        return LooseDensityConfig(
            effective_stride=beats_per_bar,
            median_beat_sec=0.0,
            estimated_span_sec=0.0,
            adapted=False,
        )

    if not adaptive_density:
        return LooseDensityConfig(
            effective_stride=beats_per_bar,
            median_beat_sec=beat_interval,
            estimated_span_sec=beat_interval * beats_per_bar,
            adapted=False,
        )

    # Normalize invalid target values into the configured bound range.
    target = float(np.clip(target_span_sec, min_span_sec, max_span_sec))
    span_range = max(max_span_sec - min_span_sec, 1e-6)

    best_stride = beats_per_bar
    best_score = float("inf")

    for stride in range(1, max_stride + 1):
        span_sec = beat_interval * stride
        bounded = float(np.clip(span_sec, min_span_sec, max_span_sec))
        bound_penalty = abs(span_sec - bounded) / span_range
        target_penalty = abs(span_sec - target) / max(target, 1e-6)

        # Prefer musically meaningful strides around beats_per_bar.
        if stride == beats_per_bar:
            music_penalty = 0.0
        elif stride % beats_per_bar == 0:
            music_penalty = 0.10
        elif beats_per_bar % stride == 0:
            music_penalty = 0.15
        else:
            music_penalty = 0.35

        score = bound_penalty * 2.5 + target_penalty + music_penalty
        if score < best_score:
            best_score = score
            best_stride = stride

    return LooseDensityConfig(
        effective_stride=best_stride,
        median_beat_sec=beat_interval,
        estimated_span_sec=beat_interval * best_stride,
        adapted=(best_stride != beats_per_bar),
    )


def _detect_phase_reference(
    *,
    beat_frames: np.ndarray,
    beat_times: np.ndarray,
    onset_env: np.ndarray,
    y_perc: np.ndarray,
    sr: int,
    hop_length: int,
    stride: int,
) -> PhaseReferenceConfig:
    beat_count = int(beat_frames.size)
    if beat_count == 0:
        return PhaseReferenceConfig(start_index=0, start_sec=0.0, method="start")
    if beat_count < max(8, stride * 2):
        return PhaseReferenceConfig(start_index=0, start_sec=float(beat_times[0]), method="start")

    onset_norm = _normalize_unit(onset_env)
    low_env = _compute_low_band_envelope(
        y_perc=y_perc,
        sr=sr,
        hop_length=hop_length,
    )
    safe_frames = np.minimum(beat_frames, low_env.size - 1)
    beat_energy = 0.75 * low_env[safe_frames] + 0.25 * onset_norm[np.minimum(beat_frames, onset_norm.size - 1)]

    window = int(np.clip(stride * 4, 8, 32))
    kernel = np.ones(window, dtype=np.float64) / float(window)
    smooth = np.convolve(beat_energy.astype(np.float64), kernel, mode="same")

    threshold = float(np.quantile(smooth, 0.90) * 0.95)
    candidates = np.where(smooth >= threshold)[0]
    if candidates.size == 0:
        return PhaseReferenceConfig(start_index=0, start_sec=float(beat_times[0]), method="start")

    min_idx = 0
    if beat_count > window:
        min_idx = max(0, window // 2 - stride)
    valid = candidates[candidates >= min_idx]
    idx = int(valid[0] if valid.size > 0 else candidates[0])

    return PhaseReferenceConfig(
        start_index=idx,
        start_sec=float(beat_times[idx]),
        method="auto-energy",
    )


def _select_loose_phase(
    *,
    beat_frames: np.ndarray,
    onset_env: np.ndarray,
    y_perc: np.ndarray,
    sr: int,
    hop_length: int,
    stride: int,
    auto_phase: bool,
    manual_anchor_beat: int | None = None,
    phase_profile: list[dict[str, float | int]] | None = None,
    phase_start_index: int = 0,
) -> LoosePhaseConfig:
    if stride <= 1 or beat_frames.size == 0:
        return LoosePhaseConfig(phase_offset=0, strategy="fixed-zero", score=0.0)
    if phase_profile is None:
        phase_profile = _build_phase_energy_profile(
            beat_frames=beat_frames,
            onset_env=onset_env,
            y_perc=y_perc,
            sr=sr,
            hop_length=hop_length,
            stride=stride,
            start_index=phase_start_index,
        )

    if manual_anchor_beat is not None:
        local_phase = int((manual_anchor_beat - 1) % stride)
        global_phase = int((phase_start_index + local_phase) % stride)
        score = float(phase_profile[local_phase]["score"]) if phase_profile else 0.0
        return LoosePhaseConfig(phase_offset=global_phase, strategy="manual-anchor-beat", score=score)
    if not auto_phase:
        global_phase = int(phase_start_index % stride)
        score = float(phase_profile[0]["score"]) if phase_profile else 0.0
        return LoosePhaseConfig(phase_offset=global_phase, strategy="fixed-zero", score=score)

    if not phase_profile:
        return LoosePhaseConfig(phase_offset=int(phase_start_index % stride), strategy="fixed-zero", score=0.0)
    best_item = max(phase_profile, key=lambda item: float(item["score"]))
    local_phase = int(best_item["beat"]) - 1
    best_phase = int((phase_start_index + local_phase) % stride)
    best_score = float(best_item["score"])

    if not math.isfinite(best_score):
        return LoosePhaseConfig(
            phase_offset=int(phase_start_index % stride),
            strategy="fixed-zero",
            score=0.0,
        )
    return LoosePhaseConfig(
        phase_offset=int(best_phase),
        strategy="auto-lowfreq",
        score=float(best_score),
    )


def write_preview_html(
    *,
    preview_path: Path,
    audio_path: Path,
    duration: float,
    tempo_bpm: float,
    confidence: float,
    anchors: np.ndarray,
    waveform: list[float],
    preview_audio_data_url: str | None = None,
    preview_audio_path: Path | None = None,
    anchor_candidates: dict[str, list[float]] | None = None,
    phase_energy_profile: list[dict[str, float | int]] | None = None,
    selected_anchor_beat: int = 1,
    recommended_anchor_beat: int = 1,
    preview_payload: dict[str, Any] | None = None,
    alternative: dict[str, Any] | None = None,
) -> None:
    """Write interactive preview html with waveform + phase analysis."""
    preview_path.parent.mkdir(parents=True, exist_ok=True)

    waveform_json = json.dumps(waveform, ensure_ascii=False)
    default_anchors = [float(v) for v in anchors.tolist()]
    anchors_json = json.dumps(default_anchors, ensure_ascii=False)
    candidate_map = anchor_candidates or {"1": default_anchors}
    if not candidate_map:
        candidate_map = {"1": default_anchors}
    candidate_keys = sorted((int(k) for k in candidate_map.keys()))
    if selected_anchor_beat not in candidate_keys:
        selected_anchor_beat = candidate_keys[0]
    phase_profile = phase_energy_profile or []
    has_phase = bool(phase_profile)
    has_multi_beat = len(candidate_keys) > 1
    candidates_json = json.dumps(candidate_map, ensure_ascii=False)
    phase_json = json.dumps(phase_profile, ensure_ascii=False)
    if preview_audio_data_url is not None:
        audio_src = preview_audio_data_url
    else:
        preview_audio = preview_audio_path if preview_audio_path is not None else audio_path
        audio_src = _build_preview_audio_src(preview_path=preview_path, audio_path=preview_audio)
    payload_pretty = ""
    if preview_payload is not None:
        payload_pretty = json.dumps(preview_payload, ensure_ascii=False, indent=2)
    generated_at = dt.datetime.now().isoformat(timespec="seconds")
    title = f"autobeat preview - {audio_path.name}"

    # Alternative strategy anchors for overlay display
    alt_anchors: list[float] = []
    alt_strategy_name = ""
    if alternative is not None:
        alt_grid = alternative.get("grid") or {}
        alt_anchors = [float(v) for v in (alt_grid.get("anchors_sec") or [])]
        alt_strategy_name = str(alternative.get("strategy", ""))
    alt_anchors_json = json.dumps(alt_anchors, ensure_ascii=False)

    html = f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{escape(title)}</title>
  <style>
    body {{
      margin: 16px;
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      color: #111827;
      background: #f9fafb;
    }}
    .panel {{
      background: white;
      border: 1px solid #e5e7eb;
      border-radius: 10px;
      padding: 12px;
      max-width: 1280px;
    }}
    .meta {{
      font-size: 14px;
      line-height: 1.5;
      color: #374151;
      margin-bottom: 10px;
      white-space: pre-wrap;
    }}
    canvas {{
      width: 100%;
      border: 1px solid #e5e7eb;
      border-radius: 8px;
      background: #ffffff;
      display: block;
    }}
    #wave {{
      height: 360px;
      transition: border-color 60ms linear, box-shadow 200ms ease-out;
    }}
    #wave.beat-active {{
      border-color: #10b981;
      box-shadow: 0 0 0 3px rgba(16, 185, 129, 0.22);
    }}
    #phase-energy {{
      height: 180px;
      margin-top: 12px;
    }}
    .legend {{
      margin-top: 8px;
      font-size: 12px;
      color: #4b5563;
    }}
    .controls {{
      display: flex;
      align-items: center;
      gap: 10px;
      margin: 10px 0 8px;
      flex-wrap: wrap;
    }}
    .time {{
      font-size: 13px;
      color: #111827;
      font-variant-numeric: tabular-nums;
    }}
    .beat-indicator {{
      width: 20px;
      height: 20px;
      border-radius: 999px;
      background: #d1d5db;
      flex-shrink: 0;
      transition: background-color 80ms linear, transform 80ms linear, box-shadow 160ms ease-out;
    }}
    .beat-indicator.active {{
      background: #10b981;
      transform: scale(1.35);
      box-shadow: 0 0 0 10px rgba(16, 185, 129, 0.18);
    }}
    details {{
      margin-top: 10px;
      border-top: 1px solid #e5e7eb;
      padding-top: 10px;
    }}
    summary {{
      cursor: pointer;
      font-size: 13px;
      color: #111827;
      user-select: none;
    }}
    pre {{
      margin-top: 8px;
      max-height: 320px;
      overflow: auto;
      font-size: 12px;
      line-height: 1.45;
      background: #f9fafb;
      border: 1px solid #e5e7eb;
      border-radius: 8px;
      padding: 10px;
      white-space: pre;
    }}
    .beat-controls {{
      display: flex;
      align-items: center;
      gap: 10px;
      margin: 8px 0 10px;
      flex-wrap: wrap;
    }}
    .beat-controls label {{
      font-size: 13px;
      color: #111827;
    }}
    .beat-controls select {{
      border: 1px solid #d1d5db;
      border-radius: 6px;
      padding: 4px 8px;
      background: #ffffff;
      color: #111827;
    }}
  </style>
</head>
<body>
  <div class="panel">
    <div class="meta">audio: {escape(str(audio_path))}
duration_sec: {duration:.3f}
tempo_bpm: {tempo_bpm:.2f}
confidence: {confidence:.3f}
anchors: {len(anchors)}
generated_at: {escape(generated_at)}</div>
    <div class="controls">
      <audio id="audio" controls preload="metadata" src="{escape(audio_src)}"></audio>
      <span class="time" id="time">0.00 / {duration:.2f}s</span>
    </div>
    <div class="beat-controls">
      {"<label for='anchor-beat-select'>Anchor beat</label><select id='anchor-beat-select'></select>" if has_multi_beat else "<select id='anchor-beat-select' style='display:none'></select>"}
      <span id="beat-indicator" class="beat-indicator" title="Beat pulse"></span>
      <span class="time" id="beat-hint"></span>
    </div>
    <canvas id="wave"></canvas>
    {"<canvas id='phase-energy'></canvas>" if has_phase else ""}
    <div class="legend">Wave: Blue=waveform, Red=anchors, Green=playhead.{"  Phase chart: Teal=low-band, Orange=onset, Purple=score." if has_phase else ""}  Click waveform to seek.</div>
    <details>
      <summary>View Data Structure (JSON)</summary>
      <pre id="payload-json">{escape(payload_pretty)}</pre>
    </details>
  </div>
  <script>
    const waveform = {waveform_json};
    const anchors = {anchors_json};
    const anchorCandidates = {candidates_json};
    const phaseProfile = {phase_json};
    const initialSelectedBeat = {int(selected_anchor_beat)};
    const recommendedBeat = {int(recommended_anchor_beat)};
    const duration = {duration:.9f};
    const altAnchors = {alt_anchors_json};
    const altStrategyName = {json.dumps(alt_strategy_name)};
    const canvas = document.getElementById("wave");
    const phaseCanvas = document.getElementById("phase-energy");
    const audio = document.getElementById("audio");
    const timeLabel = document.getElementById("time");
    const beatSelect = document.getElementById("anchor-beat-select");
    const beatIndicator = document.getElementById("beat-indicator");
    const beatHint = document.getElementById("beat-hint");
    const ctx = canvas.getContext("2d");
    const phaseCtx = phaseCanvas ? phaseCanvas.getContext("2d") : null;
    const staticLayer = document.createElement("canvas");
    const staticCtx = staticLayer.getContext("2d");
    let activeBeatKey = String(initialSelectedBeat);
    let width = 1;
    let height = 1;
    let dpr = 1;
    let phaseWidth = 1;
    let phaseHeight = 1;
    let beatEventCursor = 0;
    let lastPlayheadSec = 0;
    let indicatorActiveUntilMs = 0;

    function indexAtOrAfter(values, timeSec) {{
      let lo = 0;
      let hi = values.length;
      while (lo < hi) {{
        const mid = (lo + hi) >> 1;
        if (values[mid] < timeSec) {{
          lo = mid + 1;
        }} else {{
          hi = mid;
        }}
      }}
      return lo;
    }}

    function resizeCanvas() {{
      dpr = window.devicePixelRatio || 1;
      const rect = canvas.getBoundingClientRect();
      width = Math.max(1, Math.floor(rect.width));
      height = Math.max(1, Math.floor(rect.height));
      canvas.width = Math.floor(width * dpr);
      canvas.height = Math.floor(height * dpr);
      staticLayer.width = Math.floor(width * dpr);
      staticLayer.height = Math.floor(height * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      staticCtx.setTransform(dpr, 0, 0, dpr, 0, 0);

      if (phaseCanvas) {{
        const phaseRect = phaseCanvas.getBoundingClientRect();
        phaseWidth = Math.max(1, Math.floor(phaseRect.width));
        phaseHeight = Math.max(1, Math.floor(phaseRect.height));
        phaseCanvas.width = Math.floor(phaseWidth * dpr);
        phaseCanvas.height = Math.floor(phaseHeight * dpr);
        phaseCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
      }}
    }}

    function sortedBeatKeys() {{
      return Object.keys(anchorCandidates).sort((a, b) => Number(a) - Number(b));
    }}

    function getActiveAnchors() {{
      return anchorCandidates[activeBeatKey] || anchors;
    }}

    function resetBeatCursor(timeSec) {{
      const a = getActiveAnchors();
      beatEventCursor = indexAtOrAfter(a, Math.max(0, timeSec - 0.02));
    }}

    function triggerIndicatorPulse() {{
      indicatorActiveUntilMs = performance.now() + 160;
    }}

    function updateIndicatorState() {{
      const active = performance.now() <= indicatorActiveUntilMs;
      beatIndicator.classList.toggle("active", active);
      canvas.classList.toggle("beat-active", active);
    }}

    function processBeatEvents(currentSec, prevSec) {{
      const a = getActiveAnchors();
      if (a.length === 0) return;
      if (currentSec + 0.2 < prevSec || currentSec - prevSec > 0.9) {{
        resetBeatCursor(currentSec);
        return;
      }}
      while (beatEventCursor < a.length && a[beatEventCursor] <= currentSec + 0.02) {{
        if (a[beatEventCursor] >= prevSec - 0.03) triggerIndicatorPulse();
        beatEventCursor += 1;
      }}
    }}

    function clamp01(v) {{
      if (!Number.isFinite(v)) return 0;
      return Math.max(0, Math.min(1, v));
    }}

    function updateBeatHint() {{
      const keys = sortedBeatKeys();
      if (keys.length <= 1) {{
        beatHint.textContent = "";
        return;
      }}
      const selected = Number(activeBeatKey);
      const hasRecommended = keys.includes(String(recommendedBeat));
      if (hasRecommended) {{
        beatHint.textContent = `Beat ${{selected}} selected, beat ${{recommendedBeat}} recommended.`;
      }} else {{
        beatHint.textContent = `Beat ${{selected}} selected.`;
      }}
    }}

    function populateBeatSelect() {{
      const keys = sortedBeatKeys();
      beatSelect.innerHTML = "";
      if (keys.length === 0) {{
        const option = document.createElement("option");
        option.value = "1";
        option.textContent = "Beat 1";
        beatSelect.appendChild(option);
        activeBeatKey = "1";
        beatSelect.disabled = true;
        updateBeatHint();
        return;
      }}

      for (const key of keys) {{
        const n = Number(key);
        const option = document.createElement("option");
        option.value = key;
        option.textContent = n === recommendedBeat ? `Beat ${{key}} (recommended)` : `Beat ${{key}}`;
        beatSelect.appendChild(option);
      }}
      if (!keys.includes(activeBeatKey)) {{
        activeBeatKey = keys[0];
      }}
      beatSelect.value = activeBeatKey;
      beatSelect.disabled = keys.length <= 1;
      updateBeatHint();
    }}

    function drawStatic() {{
      staticCtx.clearRect(0, 0, width, height);
      staticCtx.fillStyle = "#ffffff";
      staticCtx.fillRect(0, 0, width, height);

      staticCtx.strokeStyle = "#e5e7eb";
      staticCtx.lineWidth = 1;
      staticCtx.beginPath();
      staticCtx.moveTo(0, height / 2);
      staticCtx.lineTo(width, height / 2);
      staticCtx.stroke();

      if (waveform.length > 1) {{
        staticCtx.strokeStyle = "#2563eb";
        staticCtx.lineWidth = 1.5;
        staticCtx.beginPath();
        for (let i = 0; i < waveform.length; i += 1) {{
          const x = (i / (waveform.length - 1)) * width;
          const y = height / 2 - waveform[i] * (height * 0.42);
          if (i === 0) {{
            staticCtx.moveTo(x, y);
          }} else {{
            staticCtx.lineTo(x, y);
          }}
        }}
        staticCtx.stroke();
      }}

      if (duration > 0) {{
        staticCtx.strokeStyle = "rgba(220, 38, 38, 0.72)";
        staticCtx.lineWidth = 1;
        const activeAnchors = getActiveAnchors();
        for (let i = 0; i < activeAnchors.length; i += 1) {{
          const t = activeAnchors[i];
          if (t < 0 || t > duration) continue;
          const x = (t / duration) * width;
          staticCtx.beginPath();
          staticCtx.moveTo(x, 0);
          staticCtx.lineTo(x, height);
          staticCtx.stroke();
        }}

      }}
    }}

    function drawPhaseChart() {{
      if (!phaseCtx) return;
      phaseCtx.clearRect(0, 0, phaseWidth, phaseHeight);
      phaseCtx.fillStyle = "#ffffff";
      phaseCtx.fillRect(0, 0, phaseWidth, phaseHeight);

      if (phaseProfile.length === 0) return;

      const padLeft = 28;
      const padRight = 12;
      const padTop = 14;
      const padBottom = 24;
      const chartW = Math.max(1, phaseWidth - padLeft - padRight);
      const chartH = Math.max(1, phaseHeight - padTop - padBottom);

      phaseCtx.strokeStyle = "#e5e7eb";
      phaseCtx.lineWidth = 1;
      phaseCtx.beginPath();
      phaseCtx.moveTo(padLeft, padTop + chartH);
      phaseCtx.lineTo(padLeft + chartW, padTop + chartH);
      phaseCtx.stroke();

      const sorted = [...phaseProfile].sort((a, b) => Number(a.beat) - Number(b.beat));
      const n = sorted.length;
      const groupW = chartW / Math.max(n, 1);
      const barW = Math.max(5, groupW * 0.16);
      const gap = Math.max(3, groupW * 0.05);

      for (let i = 0; i < n; i += 1) {{
        const item = sorted[i];
        const beat = Number(item.beat);
        const low = clamp01(Number(item.low_energy));
        const onset = clamp01(Number(item.onset_energy));
        const score = clamp01(Number(item.score));
        const groupX = padLeft + i * groupW;
        const xLow = groupX + groupW * 0.14;
        const xOnset = xLow + barW + gap;
        const xScore = xOnset + barW + gap;

        const lowH = chartH * low;
        const onsetH = chartH * onset;
        const scoreH = chartH * score;

        phaseCtx.fillStyle = "rgba(20, 184, 166, 0.85)";
        phaseCtx.fillRect(xLow, padTop + chartH - lowH, barW, lowH);

        phaseCtx.fillStyle = "rgba(245, 158, 11, 0.85)";
        phaseCtx.fillRect(xOnset, padTop + chartH - onsetH, barW, onsetH);

        phaseCtx.fillStyle = "rgba(99, 102, 241, 0.85)";
        phaseCtx.fillRect(xScore, padTop + chartH - scoreH, barW, scoreH);

        if (beat === recommendedBeat) {{
          phaseCtx.setLineDash([4, 3]);
          phaseCtx.strokeStyle = "rgba(99, 102, 241, 0.9)";
          phaseCtx.lineWidth = 1.5;
          phaseCtx.strokeRect(groupX + 2, padTop + 2, groupW - 4, chartH - 4);
          phaseCtx.setLineDash([]);
        }}
        if (beat === Number(activeBeatKey)) {{
          phaseCtx.strokeStyle = "rgba(16, 185, 129, 0.95)";
          phaseCtx.lineWidth = 2;
          phaseCtx.strokeRect(groupX + 4, padTop + 4, groupW - 8, chartH - 8);
        }}

        phaseCtx.fillStyle = "#111827";
        phaseCtx.font = "12px sans-serif";
        phaseCtx.textAlign = "center";
        phaseCtx.fillText(String(beat), groupX + groupW * 0.5, padTop + chartH + 16);
      }}
      phaseCtx.textAlign = "start";
    }}

    function draw(playheadSec) {{
      ctx.clearRect(0, 0, width, height);
      ctx.drawImage(staticLayer, 0, 0, width, height);
      if (duration > 0 && Number.isFinite(playheadSec)) {{
        const clamped = Math.max(0, Math.min(duration, playheadSec));
        const activeAnchors = getActiveAnchors();
        if (activeAnchors.length > 0) {{
          const insert = indexAtOrAfter(activeAnchors, clamped);
          let idx = Math.min(insert, activeAnchors.length - 1);
          if (insert > 0 && Math.abs(activeAnchors[insert - 1] - clamped) < Math.abs(activeAnchors[idx] - clamped)) {{
            idx = insert - 1;
          }}
          const nearest = activeAnchors[idx];
          const dist = Math.abs(nearest - clamped);
          if (dist <= 0.08) {{
            const bx = (nearest / duration) * width;
            ctx.strokeStyle = "rgba(245, 158, 11, 0.95)";
            ctx.lineWidth = 3;
            ctx.beginPath();
            ctx.moveTo(bx, 0);
            ctx.lineTo(bx, height);
            ctx.stroke();
          }}
        }}
        const x = (clamped / duration) * width;
        ctx.strokeStyle = "rgba(16, 185, 129, 0.95)";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, height);
        ctx.stroke();
      }}
    }}

    function updateTime() {{
      const current = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
      const total = Number.isFinite(duration) ? duration : 0;
      timeLabel.textContent = `${{current.toFixed(2)}} / ${{total.toFixed(2)}}s`;
    }}

    function frame() {{
      const current = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
      processBeatEvents(current, lastPlayheadSec);
      draw(current);
      updateTime();
      updateIndicatorState();
      lastPlayheadSec = current;
      if (!audio.paused && !audio.ended) {{
        window.requestAnimationFrame(frame);
      }}
    }}

    function redrawAll() {{
      resizeCanvas();
      drawStatic();
      drawPhaseChart();
      draw(audio.currentTime || 0);
      updateTime();
      updateBeatHint();
      updateIndicatorState();
    }}

    audio.addEventListener("play", () => {{
      lastPlayheadSec = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
      resetBeatCursor(lastPlayheadSec);
      window.requestAnimationFrame(frame);
    }});
    audio.addEventListener("pause", () => {{
      lastPlayheadSec = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
      draw(audio.currentTime);
      updateTime();
      updateIndicatorState();
    }});
    audio.addEventListener("timeupdate", () => {{
      lastPlayheadSec = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
      draw(audio.currentTime);
      updateTime();
    }});
    audio.addEventListener("seeking", () => {{
      lastPlayheadSec = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
      resetBeatCursor(lastPlayheadSec);
      draw(audio.currentTime);
      updateTime();
      updateIndicatorState();
    }});
    audio.addEventListener("loadedmetadata", () => {{
      lastPlayheadSec = Number.isFinite(audio.currentTime) ? audio.currentTime : 0;
      resetBeatCursor(lastPlayheadSec);
      redrawAll();
    }});
    beatSelect.addEventListener("change", () => {{
      activeBeatKey = beatSelect.value;
      resetBeatCursor(audio.currentTime || 0);
      drawStatic();
      drawPhaseChart();
      draw(audio.currentTime || 0);
      updateBeatHint();
    }});

    canvas.addEventListener("click", (event) => {{
      if (!(duration > 0)) return;
      const rect = canvas.getBoundingClientRect();
      const ratio = Math.max(0, Math.min(1, (event.clientX - rect.left) / Math.max(rect.width, 1)));
      audio.currentTime = ratio * duration;
      lastPlayheadSec = audio.currentTime;
      resetBeatCursor(lastPlayheadSec);
      draw(audio.currentTime);
      updateTime();
      updateIndicatorState();
    }});

    window.addEventListener("resize", redrawAll);
    populateBeatSelect();
    redrawAll();
  </script>
</body>
</html>
"""
    preview_path.write_text(html, encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────────────
# Strategy helpers
# ─────────────────────────────────────────────────────────────────────────────


def _build_spans(anchors: np.ndarray) -> list[Span]:
    return [
        Span(i=i, start=float(anchors[i]), end=float(anchors[i + 1]), dur=float(anchors[i + 1] - anchors[i]))
        for i in range(len(anchors) - 1)
    ]


def _compute_span_fit_quality(spans: list[Any], min_span_sec: float, max_span_sec: float) -> float:
    if not spans:
        return 0.0
    in_range = sum(1 for s in spans if min_span_sec <= s.dur <= max_span_sec)
    return float(in_range / len(spans))


def _run_energy_strategy(
    *,
    onset_env: np.ndarray,
    sr: int,
    hop_length: int,
    duration: float,
    offset_sec: float,
    min_span_sec: float,
    max_span_sec: float,
    target_span_sec: float,
) -> StrategyResult:
    """Anchor at smoothed onset-energy peaks. Good for ambient/cinematic tracks."""
    fps = sr / hop_length
    min_dist = max(1, int(min_span_sec * fps))
    target_frames = max(3, int(target_span_sec * fps))

    kernel = np.ones(target_frames, dtype=np.float64) / target_frames
    smoothed = np.convolve(onset_env.astype(np.float64), kernel, mode="same")

    delta = float(np.std(smoothed) * 0.3)
    peaks = librosa.util.peak_pick(
        smoothed,
        pre_max=max(1, min_dist // 2),
        post_max=max(1, min_dist // 2),
        pre_avg=min_dist,
        post_avg=min_dist,
        delta=delta,
        wait=min_dist,
    )

    note: str
    if len(peaks) >= 2:
        anchor_times = librosa.frames_to_time(peaks, sr=sr, hop_length=hop_length).astype(float)
        anchor_times = anchor_times + offset_sec
        anchor_times = anchor_times[(anchor_times >= 0.0) & (anchor_times <= duration)]
        anchor_times = np.unique(np.round(anchor_times, 6))
        note = f"found {len(peaks)} energy peaks"
    else:
        note = "too few energy peaks; used even spacing"

    if len(peaks) < 2 or anchor_times.size < 2:
        n = max(2, int(round(duration / target_span_sec)))
        anchor_times = np.linspace(0.0, duration, n + 1)[:-1]
        anchor_times = np.clip(anchor_times, 0.0, duration)

    spans = _build_spans(anchor_times)
    quality = _compute_span_fit_quality(spans, min_span_sec, max_span_sec)

    return StrategyResult(
        strategy="energy",
        anchors=anchor_times,
        spans=spans,
        quality_score=quality,
        analysis_meta={
            "strategy": "energy",
            "peak_count": int(anchor_times.size),
            "strategy_note": note,
            "span_count": len(spans),
            "mean_span_sec": float(np.mean([s.dur for s in spans])) if spans else 0.0,
        },
    )


def _run_onset_cluster_strategy(
    *,
    onset_env: np.ndarray,
    sr: int,
    hop_length: int,
    duration: float,
    offset_sec: float,
    min_span_sec: float,
    max_span_sec: float,
    target_span_sec: float,
) -> StrategyResult:
    """Anchor at onset transient bursts. Good for percussive or irregular-rhythm tracks."""
    fps = sr / hop_length
    min_dist = max(1, int(min_span_sec * fps))
    smooth_k = max(3, int(min_span_sec * fps // 3))

    kernel = np.ones(smooth_k, dtype=np.float64) / smooth_k
    smoothed = np.convolve(onset_env.astype(np.float64), kernel, mode="same")

    delta = float(np.std(smoothed) * 0.5)
    peaks = librosa.util.peak_pick(
        smoothed,
        pre_max=max(1, smooth_k),
        post_max=max(1, smooth_k),
        pre_avg=min_dist,
        post_avg=min_dist,
        delta=delta,
        wait=min_dist,
    )

    note: str
    quality: float
    if len(peaks) >= 2:
        peak_strengths = onset_env[np.minimum(peaks, len(onset_env) - 1)]
        norm_strength = float(np.mean(peak_strengths) / (float(np.max(onset_env)) + 1e-12))
        quality = float(np.clip(norm_strength * 1.5, 0.0, 1.0))
        anchor_times = librosa.frames_to_time(peaks, sr=sr, hop_length=hop_length).astype(float)
        anchor_times = anchor_times + offset_sec
        anchor_times = anchor_times[(anchor_times >= 0.0) & (anchor_times <= duration)]
        anchor_times = np.unique(np.round(anchor_times, 6))
        note = f"found {anchor_times.size} onset cluster peaks"
    else:
        quality = 0.1
        note = "too few onset clusters; used even spacing"

    if len(peaks) < 2 or anchor_times.size < 2:
        n = max(2, int(round(duration / target_span_sec)))
        anchor_times = np.linspace(0.0, duration, n + 1)[:-1]
        anchor_times = np.clip(anchor_times, 0.0, duration)
        quality = 0.1

    spans = _build_spans(anchor_times)

    return StrategyResult(
        strategy="onset_cluster",
        anchors=anchor_times,
        spans=spans,
        quality_score=quality,
        analysis_meta={
            "strategy": "onset_cluster",
            "peak_count": int(anchor_times.size),
            "strategy_note": note,
            "span_count": len(spans),
            "mean_span_sec": float(np.mean([s.dur for s in spans])) if spans else 0.0,
        },
    )


def _select_by_intent(
    candidates: dict[str, StrategyResult],
    intent: str,
) -> tuple[StrategyResult, StrategyResult | None]:
    """Select primary and alternative strategy based on intent preference order."""
    prefs = INTENT_PREFS.get(intent, INTENT_PREFS["auto"])
    ordered = [candidates[s] for s in prefs if s in candidates]

    if not ordered:
        raise AutobeatError("No strategy produced usable results.")

    # Follow intent order, but skip strategies that are essentially broken (< 0.15)
    _VIABILITY = 0.15
    selected = next((r for r in ordered if r.quality_score >= _VIABILITY), None)
    if selected is None:
        selected = max(ordered, key=lambda r: r.quality_score)

    alternatives = [r for r in ordered if r is not selected]
    best_alt = max(alternatives, key=lambda r: r.quality_score) if alternatives else None
    return selected, best_alt


def _make_agent_hint(
    selected: StrategyResult,
    alternative: StrategyResult | None,
    intent: str,
) -> str:
    desc = STRATEGY_USE_WHEN.get(selected.strategy, selected.strategy)
    parts = [f"Selected '{selected.strategy}' ({desc}). Quality: {selected.quality_tier} ({selected.quality_score:.2f})."]

    if selected.quality_tier == "low":
        if alternative and alternative.quality_tier in ("high", "medium"):
            alt_intent = "energy" if alternative.strategy in ("energy", "onset_cluster") else alternative.strategy
            parts.append(
                f"Quality is low. Re-run with --intent {alt_intent} for better results "
                f"('{alternative.strategy}' quality: {alternative.quality_score:.2f})."
            )
        else:
            parts.append("Quality is low. Track may lack clear rhythmic or energy structure.")
    elif selected.quality_tier == "medium" and alternative and alternative.quality_score > selected.quality_score + 0.15:
        alt_intent = "energy" if alternative.strategy in ("energy", "onset_cluster") else alternative.strategy
        parts.append(
            f"Alternative '{alternative.strategy}' has higher quality ({alternative.quality_score:.2f}). "
            f"Try --intent {alt_intent} if results seem off."
        )

    return " ".join(parts)


def _beat_analysis_meta(
    *,
    tempo: float,
    confidence: float,
    mode: str,
    beats_per_bar: int,
    effective_beats: int,
    offset_sec: float,
    engine: str,
    engine_used: str,
    adaptive_density: bool,
    auto_phase: bool,
    anchor_beat: int | None,
    phase_ref: PhaseReferenceConfig,
    phase_cfg: LoosePhaseConfig,
    effective_stride: int,
    phase_profile: list[dict[str, float | int]],
    phase_ranking: list[int],
    recommended_anchor_beat: int,
    selected_anchor_beat: int,
    density_cfg: LooseDensityConfig,
    intervals: np.ndarray,
    mean_span_sec: float,
    median_span_sec: float,
    flatness: float,
    perc_ratio: float,
    interval_cv: float,
    peak_prom: float,
    notes: list[str],
    genre_guess: dict[str, Any],
    min_span_sec: float,
    max_span_sec: float,
    target_span_sec: float,
) -> dict[str, Any]:
    return {
        "strategy": f"beat_{mode}",
        "tempo_bpm": float(tempo),
        "confidence": float(confidence),
        "mode": mode,
        "beats_per_bar": int(beats_per_bar),
        "effective_beats_per_anchor": int(effective_beats),
        "offset_sec": float(offset_sec),
        "engine_requested": engine,
        "engine_used": engine_used,
        "adaptive_density": bool(adaptive_density),
        "auto_phase": bool(auto_phase),
        "anchor_beat_override": int(anchor_beat) if anchor_beat is not None else None,
        "phase_reference_method": phase_ref.method,
        "phase_reference_start_beat_index": int(phase_ref.start_index),
        "phase_reference_start_sec": float(phase_ref.start_sec),
        "effective_phase_offset_beats": int(phase_cfg.phase_offset),
        "effective_phase_offset_from_reference": int(
            (phase_cfg.phase_offset - phase_ref.start_index) % max(effective_stride, 1)
        ),
        "phase_strategy": phase_cfg.strategy,
        "phase_selection_score": float(phase_cfg.score),
        "selected_anchor_beat": int(selected_anchor_beat),
        "recommended_anchor_beat": int(recommended_anchor_beat),
        "phase_ranking": [int(v) for v in phase_ranking],
        "phase_energy_profile": phase_profile,
        "min_span_sec": float(min_span_sec),
        "max_span_sec": float(max_span_sec),
        "target_span_sec": float(target_span_sec),
        "median_beat_sec": float(density_cfg.median_beat_sec),
        "estimated_span_sec": float(density_cfg.estimated_span_sec),
        "mean_span_sec": float(mean_span_sec),
        "median_span_sec": float(median_span_sec),
        "flatness": float(flatness),
        "percussive_ratio": float(perc_ratio),
        "interval_cv": float(interval_cv),
        "tempogram_peak_prom": float(peak_prom),
        "genre_guess": genre_guess,
        "notes": notes,
    }


def _detect_downbeats_madmom(audio_path: Path, beats_per_bar: int) -> np.ndarray:
    try:
        madmom = importlib.import_module("madmom")
    except Exception as exc:
        raise AutobeatError(
            "madmom is not installed. Install with `uv sync --extra downbeat`, or use --engine librosa."
        ) from exc

    try:
        rnn = madmom.features.downbeats.RNNDownBeatProcessor()
        activations = rnn(str(audio_path))
        tracker = madmom.features.downbeats.DBNDownBeatTrackingProcessor(
            beats_per_bar=[beats_per_bar],
            fps=100,
        )
        downbeat_events = np.asarray(tracker(activations), dtype=float)
    except Exception as exc:
        raise AutobeatError(f"madmom downbeat detection failed: {exc}") from exc

    if downbeat_events.size == 0 or downbeat_events.ndim != 2 or downbeat_events.shape[1] < 2:
        raise AutobeatError("madmom returned no usable downbeat events.")

    beat_positions = np.rint(downbeat_events[:, 1]).astype(int)
    anchors = downbeat_events[beat_positions == 1, 0].astype(float)
    if anchors.size < 2:
        raise AutobeatError("madmom returned too few downbeats for loose mode.")
    return anchors


def analyze_audio(
    *,
    audio_path: Path,
    mode: str = "loose",
    beats_per_bar: int = 4,
    offset_sec: float = 0.0,
    sr: int = 22050,
    hop_length: int = 512,
    min_bpm: float = 60.0,
    max_bpm: float = 180.0,
    min_confidence: float = 0.65,  # kept for compat; no longer a hard gate
    adaptive_density: bool = True,
    auto_phase: bool = True,
    anchor_beat: int | None = None,
    min_span_sec: float = 1.0,
    max_span_sec: float = 3.0,
    target_span_sec: float = 2.0,
    engine: str = "librosa",
    preview_html: Path | None = None,
    preview_audio_data_url: str | None = None,
    preview_audio_path: Path | None = None,
    preview_points: int = 2400,
    intent: str = "auto",
) -> dict[str, Any]:
    """Analyze audio and return multi-strategy grid payload.

    Always returns a result — never hard-fails on low confidence or BPM
    out-of-range. Beat strategies are skipped gracefully and energy/onset
    strategies provide a fallback. The ``selected`` key contains the best
    strategy for the given ``intent``; ``alternative`` suggests a different
    one if relevant.
    """
    # ── parameter validation ───────────────────────────────────────────────
    if mode not in {"tight", "loose"}:
        raise AutobeatError("`mode` must be one of: tight, loose.")
    if engine not in {"librosa", "madmom"}:
        raise AutobeatError("`engine` must be one of: librosa, madmom.")
    if beats_per_bar <= 0:
        raise AutobeatError("`beats_per_bar` must be > 0.")
    if sr <= 0:
        raise AutobeatError("`sr` must be > 0.")
    if hop_length <= 0:
        raise AutobeatError("`hop_length` must be > 0.")
    if min_bpm <= 0 or max_bpm <= 0 or min_bpm >= max_bpm:
        raise AutobeatError("Invalid BPM bounds. Require 0 < min_bpm < max_bpm.")
    if min_span_sec <= 0 or max_span_sec <= 0:
        raise AutobeatError("`min_span_sec` and `max_span_sec` must be > 0.")
    if min_span_sec > max_span_sec:
        raise AutobeatError("`min_span_sec` must be <= `max_span_sec`.")
    if not math.isfinite(target_span_sec) or target_span_sec <= 0:
        raise AutobeatError("`target_span_sec` must be a positive number.")
    if anchor_beat is not None and anchor_beat <= 0:
        raise AutobeatError("`anchor_beat` must be >= 1 when provided.")
    if intent not in VALID_INTENTS:
        raise AutobeatError(f"`intent` must be one of: {sorted(VALID_INTENTS)}.")

    # ── load audio (hard fail: nothing useful without audio) ───────────────
    try:
        y, sr = librosa.load(str(audio_path), sr=sr, mono=True)
    except Exception as exc:
        raise AutobeatError(f"Failed to load audio: {exc}") from exc
    if y.size == 0:
        raise AutobeatError("Audio is empty after loading.")
    duration = float(librosa.get_duration(y=y, sr=sr))
    if duration <= 0:
        raise AutobeatError("Invalid audio duration.")

    # ── shared feature extraction ──────────────────────────────────────────
    flatness = _to_float(np.mean(librosa.feature.spectral_flatness(y=y)))
    y_harm, y_perc = librosa.effects.hpss(y)
    harm_energy = _to_float(np.mean(y_harm**2))
    perc_energy = _to_float(np.mean(y_perc**2))
    perc_ratio = float(perc_energy / (harm_energy + perc_energy + 1e-12))

    onset_env = librosa.onset.onset_strength(y=y, sr=sr, hop_length=hop_length)
    tempo, beat_frames = librosa.beat.beat_track(
        onset_envelope=onset_env, sr=sr, hop_length=hop_length,
        start_bpm=120.0, tightness=100,
    )
    tempo = _to_float(tempo)
    beat_times = librosa.frames_to_time(beat_frames, sr=sr, hop_length=hop_length).astype(float)

    beat_ok = beat_times.size >= 4
    intervals = np.diff(beat_times) if beat_ok else np.array([], dtype=float)
    interval_cv = float(np.std(intervals) / (np.mean(intervals) + 1e-12)) if beat_ok else 1.0
    tempogram = librosa.feature.tempogram(onset_envelope=onset_env, sr=sr, hop_length=hop_length)
    tg_mean = np.mean(tempogram, axis=1)
    peak_prom = float(
        (np.sort(tg_mean)[-1] + 1e-9) / (np.sort(tg_mean)[-2] + 1e-9)
    ) if tg_mean.size >= 2 else 1.0
    confidence = _compute_confidence(
        flatness=flatness, perc_ratio=perc_ratio,
        interval_cv=interval_cv, peak_prom=peak_prom,
    )
    bpm_ok = min_bpm <= tempo <= max_bpm

    # ── beat strategies (graceful, no hard fail) ───────────────────────────
    notes: list[str] = []
    engine_used = "librosa"
    beat_candidates: dict[str, StrategyResult] = {}

    # Always-defined defaults for metadata helpers
    density_cfg = LooseDensityConfig(
        effective_stride=beats_per_bar,
        median_beat_sec=float(np.median(intervals)) if beat_ok else 0.0,
        estimated_span_sec=(float(np.median(intervals)) if beat_ok else 0.0) * beats_per_bar,
        adapted=False,
    )
    phase_cfg = LoosePhaseConfig(phase_offset=0, strategy="fixed-zero", score=0.0)
    phase_ref = PhaseReferenceConfig(
        start_index=0, start_sec=float(beat_times[0]) if beat_ok else 0.0, method="start"
    )
    phase_profile: list[dict[str, float | int]] = []
    phase_ranking: list[int] = []
    recommended_anchor_beat = 1
    effective_stride = 1
    selected_anchor_beat = 1

    if beat_ok and bpm_ok:
        if mode == "tight":
            if engine == "madmom":
                notes.append("engine=madmom is only applied to loose downbeat detection.")
            _t = beat_times + float(offset_sec)
            _t = np.unique(np.round(_t[(_t >= 0.0) & (_t <= duration)], 6))
            if _t.size >= 2:
                _sp = _build_spans(_t)
                _meta = _beat_analysis_meta(
                    tempo=tempo, confidence=confidence, mode="tight",
                    beats_per_bar=beats_per_bar, effective_beats=1,
                    offset_sec=offset_sec, engine=engine, engine_used=engine_used,
                    adaptive_density=adaptive_density, auto_phase=auto_phase,
                    anchor_beat=anchor_beat, phase_ref=phase_ref, phase_cfg=phase_cfg,
                    effective_stride=1, phase_profile=[], phase_ranking=[],
                    recommended_anchor_beat=1, selected_anchor_beat=1,
                    density_cfg=density_cfg, intervals=intervals,
                    mean_span_sec=float(np.mean([s.dur for s in _sp])),
                    median_span_sec=float(np.median([s.dur for s in _sp])),
                    flatness=flatness, perc_ratio=perc_ratio, interval_cv=interval_cv,
                    peak_prom=peak_prom, notes=list(notes),
                    genre_guess=_estimate_genre_guess(
                        tempo_bpm=float(tempo), perc_ratio=float(perc_ratio),
                        flatness=float(flatness), interval_cv=float(interval_cv),
                        phase_profile=[],
                    ),
                    min_span_sec=min_span_sec, max_span_sec=max_span_sec,
                    target_span_sec=target_span_sec,
                )
                beat_candidates["beat_tight"] = StrategyResult(
                    strategy="beat_tight", anchors=_t, spans=_sp,
                    quality_score=confidence, analysis_meta=_meta,
                )
        else:  # loose
            density_cfg = _select_loose_density(
                beat_times=beat_times, beats_per_bar=beats_per_bar,
                adaptive_density=adaptive_density,
                min_span_sec=min_span_sec, max_span_sec=max_span_sec,
                target_span_sec=target_span_sec,
            )
            effective_stride = density_cfg.effective_stride
            if density_cfg.adapted:
                notes.append(
                    f"adaptive density: loose stride adjusted {beats_per_bar} -> "
                    f"{effective_stride} beats (estimated span {density_cfg.estimated_span_sec:.2f}s)"
                )
            phase_ref = _detect_phase_reference(
                beat_frames=beat_frames, beat_times=beat_times, onset_env=onset_env,
                y_perc=y_perc, sr=sr, hop_length=hop_length, stride=effective_stride,
            )
            if phase_ref.start_index > 0:
                notes.append(
                    f"phase reference shifted to stable section: beat_idx={phase_ref.start_index}, "
                    f"t={phase_ref.start_sec:.2f}s ({phase_ref.method})"
                )
            phase_profile = _build_phase_energy_profile(
                beat_frames=beat_frames, onset_env=onset_env, y_perc=y_perc,
                sr=sr, hop_length=hop_length, stride=effective_stride,
                start_index=phase_ref.start_index,
            )
            phase_ranking = _rank_phase_beats(phase_profile)
            recommended_anchor_beat = phase_ranking[0] if phase_ranking else 1
            if anchor_beat is not None and anchor_beat > effective_stride:
                notes.append(
                    f"anchor_beat {anchor_beat} out of range for stride {effective_stride}; ignored."
                )
                anchor_beat = None
            phase_cfg = _select_loose_phase(
                beat_frames=beat_frames, onset_env=onset_env, y_perc=y_perc,
                sr=sr, hop_length=hop_length, stride=effective_stride,
                auto_phase=auto_phase, manual_anchor_beat=anchor_beat,
                phase_profile=phase_profile, phase_start_index=phase_ref.start_index,
            )
            if phase_cfg.strategy == "manual-anchor-beat":
                local_beat = int(((phase_cfg.phase_offset - phase_ref.start_index) % effective_stride) + 1)
                notes.append(
                    f"manual anchor beat: {anchor_beat} "
                    f"(local beat={local_beat}, global phase={phase_cfg.phase_offset})"
                )
            elif phase_cfg.phase_offset != int(phase_ref.start_index % max(effective_stride, 1)):
                notes.append(
                    f"loose phase aligned: global phase={phase_cfg.phase_offset} "
                    f"(strategy={phase_cfg.strategy}, score={phase_cfg.score:.3f})"
                )
            if engine == "madmom":
                if effective_stride == beats_per_bar:
                    try:
                        _loose_raw = _detect_downbeats_madmom(
                            audio_path=audio_path, beats_per_bar=beats_per_bar
                        )
                        engine_used = "madmom"
                    except AutobeatError as exc:
                        _loose_raw = beat_times[phase_cfg.phase_offset::effective_stride]
                        engine_used = "librosa-fallback"
                        notes.append(f"madmom fallback: {exc}")
                else:
                    _loose_raw = beat_times[phase_cfg.phase_offset::effective_stride]
                    notes.append("madmom skipped: adaptive loose stride differs from beats_per_bar.")
            else:
                _loose_raw = beat_times[phase_cfg.phase_offset::effective_stride]

            selected_anchor_beat = int(
                ((phase_cfg.phase_offset - phase_ref.start_index) % max(effective_stride, 1)) + 1
            )
            genre_guess_loose = _estimate_genre_guess(
                tempo_bpm=float(tempo), perc_ratio=float(perc_ratio),
                flatness=float(flatness), interval_cv=float(interval_cv),
                phase_profile=phase_profile,
            )

            # beat_loose candidate
            _la = _loose_raw + float(offset_sec)
            _la = np.unique(np.round(_la[(_la >= 0.0) & (_la <= duration)], 6))
            if _la.size >= 2:
                _lsp = _build_spans(_la)
                _ldurs = np.array([s.dur for s in _lsp], dtype=float)
                beat_candidates["beat_loose"] = StrategyResult(
                    strategy="beat_loose", anchors=_la, spans=_lsp,
                    quality_score=confidence,
                    analysis_meta=_beat_analysis_meta(
                        tempo=tempo, confidence=confidence, mode="loose",
                        beats_per_bar=beats_per_bar, effective_beats=int(effective_stride),
                        offset_sec=offset_sec, engine=engine, engine_used=engine_used,
                        adaptive_density=adaptive_density, auto_phase=auto_phase,
                        anchor_beat=anchor_beat, phase_ref=phase_ref, phase_cfg=phase_cfg,
                        effective_stride=effective_stride, phase_profile=phase_profile,
                        phase_ranking=phase_ranking,
                        recommended_anchor_beat=recommended_anchor_beat,
                        selected_anchor_beat=selected_anchor_beat,
                        density_cfg=density_cfg, intervals=intervals,
                        mean_span_sec=float(np.mean(_ldurs)) if _ldurs.size else 0.0,
                        median_span_sec=float(np.median(_ldurs)) if _ldurs.size else 0.0,
                        flatness=flatness, perc_ratio=perc_ratio,
                        interval_cv=interval_cv, peak_prom=peak_prom,
                        notes=list(notes), genre_guess=genre_guess_loose,
                        min_span_sec=min_span_sec, max_span_sec=max_span_sec,
                        target_span_sec=target_span_sec,
                    ),
                )

            # beat_tight candidate (bonus; always built in loose mode)
            _ta = beat_times + float(offset_sec)
            _ta = np.unique(np.round(_ta[(_ta >= 0.0) & (_ta <= duration)], 6))
            if _ta.size >= 2:
                _tsp = _build_spans(_ta)
                beat_candidates["beat_tight"] = StrategyResult(
                    strategy="beat_tight", anchors=_ta, spans=_tsp,
                    quality_score=confidence,
                    analysis_meta=_beat_analysis_meta(
                        tempo=tempo, confidence=confidence, mode="tight",
                        beats_per_bar=beats_per_bar, effective_beats=1,
                        offset_sec=offset_sec, engine=engine, engine_used="librosa",
                        adaptive_density=adaptive_density, auto_phase=auto_phase,
                        anchor_beat=None, phase_ref=phase_ref, phase_cfg=phase_cfg,
                        effective_stride=1, phase_profile=[], phase_ranking=[],
                        recommended_anchor_beat=1, selected_anchor_beat=1,
                        density_cfg=density_cfg, intervals=intervals,
                        mean_span_sec=float(np.mean([s.dur for s in _tsp])),
                        median_span_sec=float(np.median([s.dur for s in _tsp])),
                        flatness=flatness, perc_ratio=perc_ratio,
                        interval_cv=interval_cv, peak_prom=peak_prom, notes=[],
                        genre_guess=_estimate_genre_guess(
                            tempo_bpm=float(tempo), perc_ratio=float(perc_ratio),
                            flatness=float(flatness), interval_cv=float(interval_cv),
                            phase_profile=[],
                        ),
                        min_span_sec=min_span_sec, max_span_sec=max_span_sec,
                        target_span_sec=target_span_sec,
                    ),
                )
    else:
        reason = (
            f"tempo {tempo:.1f} BPM outside [{min_bpm:.1f}, {max_bpm:.1f}]"
            if not bpm_ok else "too few beats detected"
        )
        notes.append(f"Beat strategies unavailable: {reason}. Falling back to energy/onset grid.")

    # ── energy / onset strategies (always run) ─────────────────────────────
    energy_result = _run_energy_strategy(
        onset_env=onset_env, sr=sr, hop_length=hop_length, duration=duration,
        offset_sec=float(offset_sec), min_span_sec=min_span_sec,
        max_span_sec=max_span_sec, target_span_sec=target_span_sec,
    )
    onset_result = _run_onset_cluster_strategy(
        onset_env=onset_env, sr=sr, hop_length=hop_length, duration=duration,
        offset_sec=float(offset_sec), min_span_sec=min_span_sec,
        max_span_sec=max_span_sec, target_span_sec=target_span_sec,
    )

    # ── policy selection ───────────────────────────────────────────────────
    all_candidates: dict[str, StrategyResult] = {
        **beat_candidates,
        "energy": energy_result,
        "onset_cluster": onset_result,
    }
    selected, alternative = _select_by_intent(all_candidates, intent)

    # ── preview metadata (beat strategies only get rich phase data) ────────
    preview_phase_profile: list[dict[str, float | int]] = []
    preview_anchor_candidates: dict[str, list[float]] = {
        "1": [float(v) for v in selected.anchors.tolist()]
    }
    preview_selected_beat = 1
    preview_recommended_beat = 1
    if selected.strategy == "beat_loose" and beat_ok and bpm_ok:
        preview_phase_profile = phase_profile
        _pac = _build_anchor_candidates(
            beat_times=beat_times, stride=effective_stride, duration=duration,
            offset_sec=float(offset_sec), phase_start_index=phase_ref.start_index,
        )
        if _pac:
            preview_anchor_candidates = _pac
        _avail = sorted(int(k) for k in preview_anchor_candidates)
        preview_selected_beat = (
            selected_anchor_beat if selected_anchor_beat in _avail else (_avail[0] if _avail else 1)
        )
        preview_recommended_beat = (
            recommended_anchor_beat if recommended_anchor_beat in _avail else (_avail[0] if _avail else 1)
        )

    # ── build result ───────────────────────────────────────────────────────
    anchors_sel = selected.anchors
    spans_sel = selected.spans
    result: dict[str, Any] = {
        "audio": {
            "path": str(audio_path),
            "duration": duration,
            "sr": int(sr),
        },
        "selected": {
            "strategy": selected.strategy,
            "intent": intent,
            "grid": {
                "anchors_sec": [float(v) for v in anchors_sel.tolist()],
                "spans": [asdict(s) for s in spans_sel],
            },
            "analysis": selected.analysis_meta,
            "quality": {
                "score": float(selected.quality_score),
                "tier": selected.quality_tier,
                "agent_hint": _make_agent_hint(selected, alternative, intent),
            },
        },
        "alternative": {
            "strategy": alternative.strategy,
            "use_when": STRATEGY_USE_WHEN.get(alternative.strategy, ""),
            "quality_score": float(alternative.quality_score),
            "grid": {
                "anchors_sec": [float(v) for v in alternative.anchors.tolist()],
                "spans": [asdict(s) for s in alternative.spans],
            },
        } if alternative is not None else None,
        # backward-compat aliases (resolve-layout reads result["grid"])
        "grid": {
            "anchors_sec": [float(v) for v in anchors_sel.tolist()],
            "spans": [asdict(s) for s in spans_sel],
        },
        "analysis": selected.analysis_meta,
        "segments": [
            {
                "t0": 0.0,
                "t1": duration,
                "strategy": selected.strategy,
                "anchor_count": int(anchors_sel.size),
                "intent": intent,
            }
        ],
    }

    if preview_html is not None:
        waveform = _make_preview_waveform(y=y, max_points=preview_points)
        write_preview_html(
            preview_path=preview_html,
            audio_path=audio_path,
            duration=duration,
            tempo_bpm=float(tempo) if beat_ok else 0.0,
            confidence=float(confidence),
            anchors=anchors_sel,
            waveform=waveform,
            preview_audio_data_url=preview_audio_data_url,
            preview_audio_path=preview_audio_path,
            anchor_candidates=preview_anchor_candidates,
            phase_energy_profile=preview_phase_profile,
            selected_anchor_beat=preview_selected_beat,
            recommended_anchor_beat=preview_recommended_beat,
            preview_payload=result,
            alternative=result.get("alternative"),
        )

    return result


def write_result_json(output_path: Path, payload: dict[str, Any]) -> None:
    """Write result payload to json."""
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")


_STRATEGY_LABELS: dict[str, str] = {
    "beat_loose": "bar-level cuts",
    "beat_tight": "per-beat cuts",
    "energy": "energy cuts",
    "onset_cluster": "onset cuts",
}


def build_slim_result(full_result: dict[str, Any]) -> dict[str, Any]:
    """Return a compact, LLM-friendly result from the full analysis result.

    Strips all internal technical detail (phase metrics, confidence factors,
    engine info, etc.). Keeps only what an LLM needs for video cut decisions:
    a readable summary, actionable hint, audio metadata, and the grid.

    The ``grid`` key is preserved for ``autobeat resolve-layout`` compatibility.
    Pass ``--verbose`` to ``autobeat analyze`` to write the full result instead.
    """
    sel = full_result.get("selected") or {}
    analysis = sel.get("analysis") or full_result.get("analysis") or {}
    grid = sel.get("grid") or full_result.get("grid") or {}
    quality = sel.get("quality") or {}
    audio = full_result.get("audio") or {}

    strategy = str(sel.get("strategy") or full_result.get("strategy") or "")
    quality_tier = str(quality.get("tier") or full_result.get("quality") or "")
    hint = str(quality.get("agent_hint") or full_result.get("hint") or "")

    anchors: list[float] = grid.get("anchors_sec") or []
    spans: list[Any] = grid.get("spans") or []
    duration = float(audio.get("duration") or audio.get("duration_sec") or 0)

    tempo_raw = analysis.get("tempo_bpm") or audio.get("tempo_bpm")
    tempo_bpm = float(tempo_raw) if tempo_raw and float(tempo_raw) > 0 else None

    label = _STRATEGY_LABELS.get(strategy, strategy)
    n = len(anchors)
    dur_min = int(duration // 60)
    dur_sec_rem = int(duration % 60)
    if tempo_bpm:
        summary = f"{tempo_bpm:.0f} BPM · {n} {label} · {dur_min}:{dur_sec_rem:02d} total · quality: {quality_tier}"
    else:
        summary = f"{n} {label} · {dur_min}:{dur_sec_rem:02d} total · quality: {quality_tier}"

    audio_slim: dict[str, Any] = {
        "path": str(audio.get("path") or ""),
        "duration_sec": round(duration, 3),
    }
    if tempo_bpm:
        audio_slim["tempo_bpm"] = round(tempo_bpm, 1)

    return {
        "summary": summary,
        "hint": hint,
        "audio": audio_slim,
        "strategy": strategy,
        "quality": quality_tier,
        "grid": {
            "anchors_sec": anchors,
            "spans": spans,
        },
    }


def generate_preview_from_result(
    *,
    result: dict[str, Any],
    audio_path: Path,
    preview_path: Path,
    preview_audio_data_url: str | None = None,
    preview_audio_path: Path | None = None,
    preview_points: int = 2400,
) -> None:
    """Generate preview HTML from an existing analysis result without re-running analysis.

    Only loads the audio file to extract waveform data. All beat/analysis data is
    read from ``result`` (the parsed JSON from a previous ``analyze_audio`` call).
    """
    try:
        y, sr = librosa.load(str(audio_path), sr=22050, mono=True)
    except Exception as exc:
        raise AutobeatError(f"Failed to load audio for preview: {exc}") from exc

    waveform = _make_preview_waveform(y=y, max_points=preview_points)

    # Support full result (selected.analysis), verbose, and slim format
    sel = result.get("selected") or {}
    analysis = sel.get("analysis") or result.get("analysis") or {}
    grid = sel.get("grid") or result.get("grid") or {}

    anchors_list = grid.get("anchors_sec") or []
    anchors = np.array(anchors_list, dtype=float)

    audio_meta = result.get("audio") or {}
    duration = float(audio_meta.get("duration") or audio_meta.get("duration_sec") or librosa.get_duration(y=y, sr=sr))
    tempo_bpm = float(analysis.get("tempo_bpm") or audio_meta.get("tempo_bpm") or 0.0)
    confidence = float(analysis.get("confidence", 0.0))

    phase_profile = analysis.get("phase_energy_profile") or []
    selected_anchor_beat = int(analysis.get("selected_anchor_beat", 1))
    recommended_anchor_beat = int(analysis.get("recommended_anchor_beat", 1))

    # alt strategy: present in full result but not slim
    alternative = result.get("alternative") or (
        {"strategy": sel.get("alternative", {}).get("strategy", "")} if sel.get("alternative") else None
    )

    write_preview_html(
        preview_path=preview_path,
        audio_path=audio_path,
        duration=duration,
        tempo_bpm=tempo_bpm,
        confidence=confidence,
        anchors=anchors,
        waveform=waveform,
        preview_audio_data_url=preview_audio_data_url,
        preview_audio_path=preview_audio_path,
        anchor_candidates=None,  # not stored in JSON; switcher shows selected set only
        phase_energy_profile=phase_profile,
        selected_anchor_beat=selected_anchor_beat,
        recommended_anchor_beat=recommended_anchor_beat,
        preview_payload=result,
        alternative=alternative,
    )
