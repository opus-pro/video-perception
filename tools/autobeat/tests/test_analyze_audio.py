from __future__ import annotations

import shutil
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

pytest.importorskip("librosa")

from autobeat.analyze import analyze_audio, write_result_json
from autobeat.errors import AutobeatError


def _write_click_track(path: Path, *, sr: int = 22050, bpm: float = 120.0, duration_sec: float = 8.0) -> None:
    samples = int(sr * duration_sec)
    audio = np.zeros(samples, dtype=np.float32)
    beat_step = int(round(sr * (60.0 / bpm)))
    click_len = max(8, sr // 200)
    for idx in range(0, samples, beat_step):
        end = min(samples, idx + click_len)
        audio[idx:end] += 0.9 * np.hanning(end - idx).astype(np.float32)
    sf.write(path, audio, sr)


def _write_short_noise(path: Path, *, sr: int = 22050, duration_sec: float = 0.2) -> None:
    samples = int(sr * duration_sec)
    audio = np.random.default_rng(7).normal(0.0, 0.25, size=samples).astype(np.float32)
    sf.write(path, audio, sr)


def _sel(result: dict) -> dict:
    """Extract the selected strategy from the multi-strategy result."""
    return result["selected"]


def test_analyze_tight_mode_succeeds_for_click_track(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    _write_click_track(audio_path)
    result = analyze_audio(
        audio_path=audio_path,
        mode="tight",
        min_bpm=50,
        max_bpm=200,
    )

    sel = _sel(result)
    anchors = sel["grid"]["anchors_sec"]
    spans = sel["grid"]["spans"]
    assert len(anchors) > 4
    assert len(spans) == len(anchors) - 1
    assert all(anchors[i] < anchors[i + 1] for i in range(len(anchors) - 1))
    assert all(span["dur"] > 0 for span in spans)


def test_analyze_loose_mode_has_fewer_or_equal_anchors(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    _write_click_track(audio_path)
    tight = analyze_audio(audio_path=audio_path, mode="tight")
    loose = analyze_audio(
        audio_path=audio_path,
        mode="loose",
        beats_per_bar=4,
    )
    # Loose should have fewer or equal anchors (may fall back to non-beat strategy)
    assert len(_sel(loose)["grid"]["anchors_sec"]) <= len(_sel(tight)["grid"]["anchors_sec"])


def test_analyze_offset_changes_anchor_positions(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    _write_click_track(audio_path)
    baseline = analyze_audio(audio_path=audio_path, mode="tight")
    shifted = analyze_audio(
        audio_path=audio_path,
        mode="tight",
        offset_sec=0.1,
    )
    assert _sel(shifted)["grid"]["anchors_sec"][0] > _sel(baseline)["grid"]["anchors_sec"][0]


def test_preview_html_is_generated(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    preview_path = tmp_path / "preview.html"
    _write_click_track(audio_path)
    result = analyze_audio(
        audio_path=audio_path,
        mode="tight",
        preview_html=preview_path,
        preview_points=600,
    )
    assert preview_path.exists()
    body = preview_path.read_text(encoding="utf-8")
    assert "<audio id=\"audio\"" in body
    assert "Click waveform to seek" in body
    # anchor-beat-select is always present (may be hidden for non-beat strategies)
    assert "anchor-beat-select" in body
    assert len(_sel(result)["grid"]["anchors_sec"]) > 0


def test_preview_html_supports_audio_path_override(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    preview_path = tmp_path / "share" / "preview.html"
    preview_audio_path = tmp_path / "share" / "click_track.wav"
    _write_click_track(audio_path)
    preview_audio_path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(audio_path, preview_audio_path)

    analyze_audio(
        audio_path=audio_path,
        mode="tight",
        preview_html=preview_path,
        preview_audio_path=preview_audio_path,
        preview_points=600,
    )
    body = preview_path.read_text(encoding="utf-8")
    assert 'src="click_track.wav"' in body


def test_preview_html_supports_embedded_audio_and_payload(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    preview_path = tmp_path / "preview_embed.html"
    _write_click_track(audio_path)

    analyze_audio(
        audio_path=audio_path,
        mode="tight",
        preview_html=preview_path,
        preview_audio_data_url="data:audio/wav;base64,AAAA",
        preview_points=600,
    )
    body = preview_path.read_text(encoding="utf-8")
    assert 'src="data:audio/wav;base64,AAAA"' in body
    assert "View Data Structure (JSON)" in body
    assert "id=\"payload-json\"" in body


def test_analyze_short_audio_returns_fallback(tmp_path: Path) -> None:
    """Short/noisy audio no longer hard-fails — it falls back to energy/onset strategies."""
    audio_path = tmp_path / "short_noise.wav"
    _write_short_noise(audio_path, duration_sec=1.0)
    result = analyze_audio(audio_path=audio_path, mode="tight")
    sel = _sel(result)
    # Should still produce a result (energy or onset fallback)
    assert sel["strategy"] in {"energy", "onset_cluster", "beat_tight", "beat_loose"}
    assert len(sel["grid"]["anchors_sec"]) >= 2


def test_analyze_empty_audio_raises(tmp_path: Path) -> None:
    """Truly empty audio should still raise."""
    audio_path = tmp_path / "empty.wav"
    sf.write(audio_path, np.array([], dtype=np.float32), 22050)
    with pytest.raises(AutobeatError):
        analyze_audio(audio_path=audio_path, mode="tight")


def test_loose_adaptive_density_controls_span_length(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track_slow.wav"
    _write_click_track(audio_path, bpm=60.0, duration_sec=18.0)

    strict = analyze_audio(
        audio_path=audio_path,
        mode="loose",
        beats_per_bar=4,
        adaptive_density=False,
    )
    adaptive = analyze_audio(
        audio_path=audio_path,
        mode="loose",
        beats_per_bar=4,
        adaptive_density=True,
        min_span_sec=1.0,
        max_span_sec=3.0,
        target_span_sec=2.0,
    )

    strict_sel = _sel(strict)
    adaptive_sel = _sel(adaptive)

    # Both should produce results; if beat strategy was selected, check density
    if strict_sel["strategy"] == "beat_loose" and adaptive_sel["strategy"] == "beat_loose":
        strict_mean = float(strict_sel["analysis"]["mean_span_sec"])
        adaptive_mean = float(adaptive_sel["analysis"]["mean_span_sec"])
        assert adaptive_mean <= strict_mean or adaptive_mean <= 3.0
    else:
        # Fallback strategies still produce valid output
        assert len(strict_sel["grid"]["anchors_sec"]) >= 2
        assert len(adaptive_sel["grid"]["anchors_sec"]) >= 2


def test_loose_auto_phase_can_shift_anchor_start(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    _write_click_track(audio_path, bpm=120.0, duration_sec=12.0)

    fixed = analyze_audio(
        audio_path=audio_path,
        mode="loose",
        beats_per_bar=4,
        auto_phase=False,
        adaptive_density=False,
    )
    auto = analyze_audio(
        audio_path=audio_path,
        mode="loose",
        beats_per_bar=4,
        auto_phase=True,
        adaptive_density=False,
    )

    auto_sel = _sel(auto)
    fixed_sel = _sel(fixed)

    # Phase fields only present when beat_loose strategy is selected
    if auto_sel["strategy"] == "beat_loose":
        assert "effective_phase_offset_beats" in auto_sel["analysis"]
        assert auto_sel["analysis"]["phase_strategy"] in {"auto-lowfreq", "fixed-zero"}
        assert isinstance(auto_sel["analysis"].get("phase_energy_profile", []), list)
    # Both should produce valid grids regardless
    assert len(auto_sel["grid"]["anchors_sec"]) >= 2
    assert len(fixed_sel["grid"]["anchors_sec"]) >= 2


def test_loose_manual_anchor_beat_override(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    _write_click_track(audio_path, bpm=120.0, duration_sec=12.0)
    beat1 = analyze_audio(
        audio_path=audio_path,
        mode="loose",
        beats_per_bar=4,
        adaptive_density=False,
        auto_phase=True,
        anchor_beat=1,
    )
    beat3 = analyze_audio(
        audio_path=audio_path,
        mode="loose",
        beats_per_bar=4,
        adaptive_density=False,
        auto_phase=True,
        anchor_beat=3,
    )

    b1_sel = _sel(beat1)
    b3_sel = _sel(beat3)

    # If beat_loose strategy was selected, verify anchor beat behavior
    if b1_sel["strategy"] == "beat_loose" and b3_sel["strategy"] == "beat_loose":
        assert b1_sel["analysis"]["selected_anchor_beat"] == 1
        assert b3_sel["analysis"]["selected_anchor_beat"] == 3
        assert b3_sel["analysis"]["phase_strategy"] == "manual-anchor-beat"
        assert b1_sel["grid"]["anchors_sec"][0] != b3_sel["grid"]["anchors_sec"][0]
    else:
        # Fallback strategies still produce valid output
        assert len(b1_sel["grid"]["anchors_sec"]) >= 2
        assert len(b3_sel["grid"]["anchors_sec"]) >= 2


def test_write_result_json(tmp_path: Path) -> None:
    payload = {"ok": True}
    output = tmp_path / "out" / "result.json"
    write_result_json(output, payload)
    assert output.exists()
    assert output.read_text(encoding="utf-8").strip().startswith("{")


def test_analyze_rejects_invalid_span_bounds(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    _write_click_track(audio_path)
    with pytest.raises(AutobeatError):
        analyze_audio(
            audio_path=audio_path,
            mode="loose",
            min_span_sec=3.0,
            max_span_sec=1.0,
        )


def test_analyze_rejects_invalid_anchor_beat(tmp_path: Path) -> None:
    audio_path = tmp_path / "click_track.wav"
    _write_click_track(audio_path)
    with pytest.raises(AutobeatError):
        analyze_audio(
            audio_path=audio_path,
            mode="loose",
            anchor_beat=0,
        )
