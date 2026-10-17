"""Tests for the multi-preset (fused) inference path in video-perception-vlm.

Covers prompt fusion shape, response splitting under partial / total failure,
and an end-to-end run with a monkey-patched VLM client so no real backend is
required.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from vlm_engine import main as vlm_main
from vlm_engine.main import (
    ensure_inspection_succeeded,
    fuse_prompts,
    run_inspection_fused,
    split_fused_response,
)


def _make_prompt(name: str, system: str = "", prompt: str = "", version: str = "1.0") -> dict:
    return {
        "name": name,
        "system": system or f"system-for-{name}",
        "prompt": prompt or f"prompt-for-{name}",
        "version": version,
        "model": None,
    }


# ── fuse_prompts ────────────────────────────────────────────────


def test_fuse_prompts_includes_all_task_names_and_rubrics():
    prompts = [
        _make_prompt("gibberish-text"),
        _make_prompt("scene-description"),
        _make_prompt("object-inventory"),
    ]
    fused = fuse_prompts(prompts)

    assert fused["task_names"] == ["gibberish-text", "scene-description", "object-inventory"]
    assert fused["version"] == "fused-1.0"
    assert fused["name"].startswith("fused:")

    for p in prompts:
        assert f"## Task: {p['name']}" in fused["system"], "each task's system section must be present"
        assert p["system"] in fused["system"]
        assert f"## {p['name']}" in fused["prompt"]
        assert p["prompt"] in fused["prompt"]

    assert "Return ONLY the JSON object" in fused["prompt"]


def test_fuse_prompts_rejects_fewer_than_two():
    with pytest.raises(ValueError):
        fuse_prompts([_make_prompt("only-one")])


def test_fuse_prompts_records_source_versions():
    prompts = [_make_prompt("a", version="1.2"), _make_prompt("b", version="3.4")]
    fused = fuse_prompts(prompts)
    assert fused["source_versions"] == {"a": "1.2", "b": "3.4"}


# ── split_fused_response ────────────────────────────────────────


def test_split_fused_response_happy_path():
    response = {
        "gibberish-text": {"dominant_text_present": False},
        "scene-description": {"contains_people": True, "people_count": 2},
        "object-inventory": {"objects": ["person"]},
    }
    out = split_fused_response(response, list(response.keys()))
    assert out == response


def test_split_fused_response_marks_missing_tasks_with_error():
    response = {"gibberish-text": {"dominant_text_present": False}}
    out = split_fused_response(response, ["gibberish-text", "scene-description"])
    assert out["gibberish-text"] == {"dominant_text_present": False}
    assert "error" in out["scene-description"]
    assert "missing" in out["scene-description"]["error"]


def test_split_fused_response_propagates_full_error_to_all_tasks():
    response = {"error": "ollama timeout"}
    out = split_fused_response(response, ["a", "b", "c"])
    assert out == {
        "a": {"error": "ollama timeout"},
        "b": {"error": "ollama timeout"},
        "c": {"error": "ollama timeout"},
    }


def test_split_fused_response_ignores_non_dict_values():
    response = {"gibberish-text": "not a dict", "scene-description": {"contains_people": False}}
    out = split_fused_response(response, ["gibberish-text", "scene-description"])
    assert "error" in out["gibberish-text"]
    assert out["scene-description"] == {"contains_people": False}


# ── run_inspection_fused ────────────────────────────────────────


def test_run_inspection_fused_emits_one_output_per_task(tmp_path, monkeypatch):
    scenes_path = tmp_path / "scenes.json"
    scenes_path.write_text(json.dumps({"scenes": [{"idx": 0}, {"idx": 1}]}))

    keyframe_dir = tmp_path / "stats"
    keyframe_dir.mkdir()
    (keyframe_dir / "scene_00_key.png").write_bytes(b"fake-png-0")
    (keyframe_dir / "scene_01_key.png").write_bytes(b"fake-png-1")

    calls: list[str] = []

    def fake_query(image_path, system, user, model, backend):
        calls.append(image_path)
        return {
            "gibberish-text": {"dominant_text_present": False, "fully_legible": True},
            "scene-description": {"contains_people": True, "people_count": 1},
        }

    monkeypatch.setattr(vlm_main, "query_vlm", fake_query)

    prompts = [_make_prompt("gibberish-text"), _make_prompt("scene-description")]
    out = run_inspection_fused(prompts, str(scenes_path), str(keyframe_dir), "test-model")

    assert len(calls) == 2, "exactly one VLM call per scene (fused), not per-task × per-scene"

    assert set(out.keys()) == {"gibberish-text", "scene-description", "_fusion"}

    for name in ("gibberish-text", "scene-description"):
        assert out[name]["task"] == name
        assert out[name]["model"] == "test-model"
        assert out[name]["fusion_call"] is True
        assert len(out[name]["scenes"]) == 2
        assert [s["scene_idx"] for s in out[name]["scenes"]] == [0, 1]

    assert out["_fusion"]["task_names"] == ["gibberish-text", "scene-description"]
    assert out["_fusion"]["scene_count"] == 2


def test_run_inspection_fused_handles_missing_keyframes(tmp_path, monkeypatch):
    scenes_path = tmp_path / "scenes.json"
    scenes_path.write_text(json.dumps({"scenes": [{"idx": 0}, {"idx": 1}]}))
    keyframe_dir = tmp_path / "stats"
    keyframe_dir.mkdir()
    (keyframe_dir / "scene_00_key.png").write_bytes(b"fake")

    monkeypatch.setattr(
        vlm_main,
        "query_vlm",
        lambda *_args, **_kw: {"a": {"ok": True}, "b": {"ok": True}},
    )

    prompts = [_make_prompt("a"), _make_prompt("b")]
    out = run_inspection_fused(prompts, str(scenes_path), str(keyframe_dir), "m")

    assert out["a"]["scenes"][0]["ok"] is True
    assert out["a"]["scenes"][1]["error"] == "no keyframe"
    assert out["b"]["scenes"][1]["error"] == "no keyframe"


def test_run_inspection_fused_handles_provider_error(tmp_path, monkeypatch):
    scenes_path = tmp_path / "scenes.json"
    scenes_path.write_text(json.dumps({"scenes": [{"idx": 0}]}))
    keyframe_dir = tmp_path / "stats"
    keyframe_dir.mkdir()
    (keyframe_dir / "scene_00_key.png").write_bytes(b"fake")

    monkeypatch.setattr(
        vlm_main,
        "query_vlm",
        lambda *_args, **_kw: {"error": "backend exploded"},
    )

    prompts = [_make_prompt("a"), _make_prompt("b")]
    out = run_inspection_fused(prompts, str(scenes_path), str(keyframe_dir), "m")

    assert out["a"]["scenes"][0]["error"] == "backend exploded"
    assert out["b"]["scenes"][0]["error"] == "backend exploded"
    with pytest.raises(RuntimeError, match="VLM inspection failed"):
        ensure_inspection_succeeded(out)


def test_run_inspection_fused_empty_scenes_returns_empty_per_task(tmp_path):
    scenes_path = tmp_path / "scenes.json"
    scenes_path.write_text(json.dumps({"scenes": []}))
    keyframe_dir = tmp_path / "stats"
    keyframe_dir.mkdir()

    prompts = [_make_prompt("a"), _make_prompt("b")]
    out = run_inspection_fused(prompts, str(scenes_path), str(keyframe_dir), "m")

    assert out == {
        "a": {"task": "a", "model": "m", "backend": "ollama", "scenes": []},
        "b": {"task": "b", "model": "m", "backend": "ollama", "scenes": []},
    }
