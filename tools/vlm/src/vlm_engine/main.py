#!/usr/bin/env python3
"""video-perception-vlm — VLM inference engine for Video Perception.

Runs built-in capability prompt packs against keyframes via a configurable
user-managed model backend (Ollama or Gemini).
Called internally by the Video Perception executor for registered capabilities.

Usage:
    video-perception-vlm --preset gibberish-text --scenes scenes.json --keyframe-dir stats/
    video-perception-vlm --preset gibberish-text,scene-description --scenes ... --keyframe-dir ...
    video-perception-vlm --list-presets
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

from vlm_engine.providers import ModelBackend, get_provider

DEFAULT_MODEL = "gemini-2.5-flash"

PRESETS_DIR = Path(__file__).parent / "presets"

# ── Prompt Loader ──────────────────────────────────────────────

def _parse_frontmatter(text: str) -> tuple[dict, str]:
    """Parse YAML frontmatter from a prompt.md file. Returns (meta, body)."""
    if not text.startswith("---"):
        return {}, text
    _, fm, body = text.split("---", 2)
    meta = {}
    for line in fm.strip().splitlines():
        if ":" in line:
            key, val = line.split(":", 1)
            meta[key.strip()] = val.strip().strip('"').strip("'")
    return meta, body.strip()


def load_prompt(path: Path) -> dict:
    """Load a registered capability prompt.md file.

    Returns dict with keys: system, prompt, version, name.
    """
    if not path.exists():
        raise FileNotFoundError(f"No prompt file: {path}")

    text = path.read_text()
    meta, body = _parse_frontmatter(text)

    sections = {"system": [], "prompt": []}
    current = None
    for line in body.splitlines():
        heading = line.strip().lower()
        if heading == "## system":
            current = "system"
            continue
        elif heading == "## prompt":
            current = "prompt"
            continue
        if current in sections:
            sections[current].append(line)

    return {
        "system": "\n".join(sections["system"]).strip(),
        "prompt": "\n".join(sections["prompt"]).strip(),
        "version": meta.get("version", "0.0"),
        "name": meta.get("task", path.stem),
        "model": meta.get("model", None),
    }


def fuse_prompts(prompts: list[dict]) -> dict:
    """Combine N preset prompts into a single fused prompt for one-call inference.

    The fused prompt asks the model to return a top-level JSON object with one
    key per task. Each task's capability contract is preserved under a section
    header so the model has the same context it would have in a single-task
    call.
    """
    if len(prompts) < 2:
        raise ValueError(f"fuse_prompts requires >=2 prompts, got {len(prompts)}")

    names = [p["name"] for p in prompts]

    system_parts = [
        "You are a video frame analyst handling multiple tasks in a single response.",
        "",
        "Each task below has its own observation contract. Apply each task independently to the same image.",
    ]
    for p in prompts:
        if p["system"]:
            system_parts.append("")
            system_parts.append(f"## Task: {p['name']}")
            system_parts.append(p["system"])

    user_parts = [
        "For the image, return a single JSON object with one top-level key per task. Each key's value must conform to its task's output contract.",
    ]
    for p in prompts:
        user_parts.append("")
        user_parts.append(f"## {p['name']}")
        user_parts.append(p["prompt"])

    user_parts.append("")
    user_parts.append("Required output shape:")
    user_parts.append("{")
    user_parts.extend(f'  "{n}": {{...task-specific fields...}}{"," if i < len(names) - 1 else ""}'
                     for i, n in enumerate(names))
    user_parts.append("}")
    user_parts.append("")
    user_parts.append("Return ONLY the JSON object — no commentary, no markdown fences.")

    return {
        "system": "\n".join(system_parts),
        "prompt": "\n".join(user_parts),
        "version": "fused-1.0",
        "name": "fused:" + ",".join(names),
        "task_names": names,
        "source_versions": {p["name"]: p["version"] for p in prompts},
        "model": None,
    }


def split_fused_response(response: dict, task_names: list[str]) -> dict[str, dict]:
    """Split a fused VLM response into per-task dicts.

    Handles three cases:
    - VLM returned the expected shape: one key per task.
    - VLM returned partial keys: missing tasks get an error entry.
    - VLM returned an error wrapper (no task keys at all): every task inherits the error.
    """
    if "error" in response and not any(n in response for n in task_names):
        return {n: {"error": response["error"]} for n in task_names}

    out: dict[str, dict] = {}
    for name in task_names:
        if name in response and isinstance(response[name], dict):
            out[name] = response[name]
        else:
            out[name] = {"error": f"task '{name}' missing in fused response"}
    return out


def failed_scenes(output: dict) -> list[str]:
    """Return human-readable failures from single or fused inspection output."""
    tasks = (
        [value for key, value in output.items() if key != "_fusion"]
        if "_fusion" in output
        else [output]
    )
    failures: list[str] = []
    for task in tasks:
        task_name = task.get("task", "unknown")
        for scene in task.get("scenes", []):
            if "error" in scene:
                failures.append(
                    f"{task_name} scene {scene.get('scene_idx', '?')}: {scene['error']}"
                )
    return failures


def ensure_inspection_succeeded(output: dict) -> None:
    """Prevent provider errors from being reported as successful capabilities."""
    failures = failed_scenes(output)
    if failures:
        preview = "; ".join(failures[:3])
        suffix = f" (+{len(failures) - 3} more)" if len(failures) > 3 else ""
        raise RuntimeError(f"VLM inspection failed: {preview}{suffix}")


# ── VLM Client ─────────────────────────────────────────────────

def query_vlm(
    image_path: str,
    system_prompt: str,
    user_prompt: str,
    model: str,
    backend: ModelBackend | None = None,
) -> dict:
    """Send an image to the configured VLM backend and get structured JSON back."""
    backend = backend or get_provider()
    try:
        return backend.query(image_path, system_prompt, user_prompt, model)
    except Exception as e:
        return {"error": str(e)}


def find_keyframe(keyframe_dir: str, scene_idx: int) -> str | None:
    """Find keyframe file for a scene. Tries common naming patterns."""
    for pattern in [
        f"scene_{scene_idx:02d}_key.png",
        f"frame_{scene_idx:02d}.png",
        f"scene_{scene_idx:03d}_key.png",
        f"frame_{scene_idx:03d}.png",
    ]:
        path = Path(keyframe_dir) / pattern
        if path.exists():
            return str(path)
    return None


# ── Core Runner ─────────────────────────────────────────────────

def run_inspection(prompt: dict, scenes_path: str, keyframe_dir: str, model: str) -> dict:
    """Run a prompt pack against all scenes. Returns structured JSON output."""
    with open(scenes_path) as f:
        scenes_data = json.load(f)
    scenes = scenes_data.get("scenes", [])
    backend = get_provider()

    if not scenes:
        return {
            "task": prompt["name"],
            "model": model,
            "backend": backend.name,
            "sensor_class": "probabilistic",
            "prompt_version": prompt["version"],
            "created_at": datetime.now(timezone.utc).isoformat(),
            "scenes": [],
        }

    print(f"vlm: {prompt['name']} model={model} scenes={len(scenes)}", file=sys.stderr)

    results = []
    for scene in scenes:
        idx = scene["idx"]
        keyframe = find_keyframe(keyframe_dir, idx)

        if not keyframe:
            print(f"  scene {idx}: no keyframe, skipping", file=sys.stderr)
            results.append({"scene_idx": idx, "error": "no keyframe"})
            continue

        print(f"  scene {idx}: {Path(keyframe).name} ...", end="", file=sys.stderr, flush=True)
        response = query_vlm(keyframe, prompt["system"], prompt["prompt"], model, backend)
        response["scene_idx"] = idx

        if "error" not in response:
            print(" ok", file=sys.stderr)
        else:
            print(f" error: {response['error'][:60]}", file=sys.stderr)

        results.append(response)

    return {
        "task": prompt["name"],
        "model": model,
        "backend": backend.name,
        "sensor_class": "probabilistic",
        "prompt_version": prompt["version"],
        "created_at": datetime.now(timezone.utc).isoformat(),
        "scenes": results,
    }


def run_inspection_fused(prompts: list[dict], scenes_path: str, keyframe_dir: str, model: str) -> dict:
    """Run N preset prompts against all scenes via a single fused VLM call per scene.

    Returns a dict keyed by task name; each value has the same shape as a
    single-task run_inspection result, so consumers can treat them
    interchangeably.
    """
    fused = fuse_prompts(prompts)
    task_names = fused["task_names"]
    created_at = datetime.now(timezone.utc).isoformat()

    with open(scenes_path) as f:
        scenes_data = json.load(f)
    scenes = scenes_data.get("scenes", [])
    backend = get_provider()

    if not scenes:
        return {
            n: {"task": n, "model": model, "backend": backend.name, "scenes": []}
            for n in task_names
        }

    print(f"vlm[fused]: {','.join(task_names)} model={model} scenes={len(scenes)}", file=sys.stderr)

    per_task_scenes: dict[str, list[dict]] = {n: [] for n in task_names}

    for scene in scenes:
        idx = scene["idx"]
        keyframe = find_keyframe(keyframe_dir, idx)

        if not keyframe:
            print(f"  scene {idx}: no keyframe, skipping", file=sys.stderr)
            for n in task_names:
                per_task_scenes[n].append({"scene_idx": idx, "error": "no keyframe"})
            continue

        print(f"  scene {idx}: {Path(keyframe).name} ...", end="", file=sys.stderr, flush=True)
        raw = query_vlm(keyframe, fused["system"], fused["prompt"], model, backend)
        split = split_fused_response(raw, task_names)

        any_error = any("error" in v for v in split.values())
        if not any_error:
            print(" ok", file=sys.stderr)
        else:
            errors = [n for n, v in split.items() if "error" in v]
            print(f" partial: {','.join(errors)} failed", file=sys.stderr)

        for n in task_names:
            entry = dict(split[n])
            entry["scene_idx"] = idx
            per_task_scenes[n].append(entry)

    out: dict[str, dict] = {}
    for p in prompts:
        n = p["name"]
        out[n] = {
            "task": n,
            "model": model,
            "backend": backend.name,
            "sensor_class": "probabilistic",
            "prompt_version": p["version"],
            "created_at": created_at,
            "fusion_call": True,
            "scenes": per_task_scenes[n],
        }
    out["_fusion"] = {
        "task_names": task_names,
        "fused_prompt_version": fused["version"],
        "scene_count": len(scenes),
        "model": model,
        "backend": backend.name,
        "created_at": created_at,
    }
    return out


def _load_preset_by_name(name: str) -> dict:
    path = PRESETS_DIR / name / "prompt.md"
    if not path.exists():
        available = [d.name for d in PRESETS_DIR.iterdir() if (d / "prompt.md").exists()]
        raise SystemExit(f"Unknown preset: {name}. Available: {', '.join(available)}")
    return load_prompt(path)


# ── CLI ─────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="VLM inference engine for video-perception",
        epilog="Presets are registered, reusable capability prompt packs. "
               "Pass multiple presets comma-separated (e.g. a,b) to fuse them into a single VLM call per scene."
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--preset", help="Built-in preset name, or comma-separated list to fuse")
    group.add_argument("--list-presets", action="store_true", help="List built-in presets")

    parser.add_argument("--scenes", help="Path to scenes.json")
    parser.add_argument("--keyframe-dir", help="Directory with keyframe PNGs")
    parser.add_argument("--model", default=None, help=f"Override model (default: {DEFAULT_MODEL})")
    args = parser.parse_args()

    if args.list_presets:
        for d in sorted(PRESETS_DIR.iterdir()):
            if (d / "prompt.md").exists():
                p = load_prompt(d / "prompt.md")
                print(f"  {d.name:<20} v{p['version']}  model={p.get('model') or DEFAULT_MODEL}", file=sys.stderr)
        return

    if not args.preset:
        parser.error("--preset is required")

    if not args.scenes or not args.keyframe_dir:
        parser.error("--scenes and --keyframe-dir are required")

    backend = os.environ.get("VIDEO_PERCEPTION_VLM_BACKEND", "ollama")
    backend_default = {
        "ollama": "gemma4",
        "gemini": "gemini-2.5-flash",
    }.get(backend, DEFAULT_MODEL)

    if args.preset and "," in args.preset:
        names = [n.strip() for n in args.preset.split(",") if n.strip()]
        if len(names) < 2:
            parser.error("--preset list needs at least 2 presets when comma-separated")
        prompts = [_load_preset_by_name(n) for n in names]
        model = args.model or os.environ.get("VIDEO_PERCEPTION_VLM_MODEL") or os.environ.get("OLLAMA_MODEL") or backend_default
        output = run_inspection_fused(prompts, args.scenes, args.keyframe_dir, model)
        ensure_inspection_succeeded(output)
        print(json.dumps(output, indent=2))
        return

    prompt = _load_preset_by_name(args.preset)

    model = args.model or prompt.get("model") or os.environ.get("VIDEO_PERCEPTION_VLM_MODEL") or os.environ.get("OLLAMA_MODEL") or backend_default
    output = run_inspection(prompt, args.scenes, args.keyframe_dir, model)
    ensure_inspection_succeeded(output)
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
