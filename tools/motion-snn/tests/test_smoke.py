"""Smoke tests for video-perception-motion-snn — no real video required."""

from __future__ import annotations

import json
from unittest.mock import patch

from motion_snn.main import aggregate_per_scene, classify_motion


def test_classify_motion_thresholds():
    assert classify_motion(0.0) == "static"
    assert classify_motion(2.9) == "static"
    assert classify_motion(3.0) == "subtle"
    assert classify_motion(14.9) == "subtle"
    assert classify_motion(15.0) == "moderate"
    assert classify_motion(39.9) == "moderate"
    assert classify_motion(40.0) == "dynamic"
    assert classify_motion(79.9) == "dynamic"
    assert classify_motion(80.0) == "chaotic"


def test_aggregate_per_scene_legacy_shape(tmp_path):
    snn = {
        "per_frame": [
            {"frame": i, "pts": i * 0.04, "intensity": 0.5 if i < 10 else 0.1, "spike": i == 5}
            for i in range(20)
        ]
    }
    scenes_path = tmp_path / "scenes.json"
    scenes_path.write_text(json.dumps({"scenes": [
        {"idx": 0, "start_ms": 0, "end_ms": 400},
        {"idx": 1, "start_ms": 400, "end_ms": 800},
    ]}))

    out = aggregate_per_scene(snn, scenes_path)
    assert len(out) == 2
    assert {r["scene_idx"] for r in out} == {0, 1}
    # Legacy fields present
    for r in out:
        assert "motion_intensity" in r
        assert "motion_class" in r
        assert "dominant_direction" in r
        assert "segments" in r
    # Scene 0 has higher intensity than scene 1
    s0 = next(r for r in out if r["scene_idx"] == 0)
    s1 = next(r for r in out if r["scene_idx"] == 1)
    assert s0["motion_intensity"] > s1["motion_intensity"]
    # Spike counted in scene 0
    assert s0["snn_spike_count"] >= 1
