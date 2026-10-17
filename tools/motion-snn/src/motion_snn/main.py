"""video-perception-motion-snn CLI.

Runs a lightweight LIF spiking neural network over event-rate input derived
from per-frame pixel differences. Emits a per-frame motion-intensity signal
plus per-scene aggregates that match the video-perception signal contract.

Replaces the prior `track_motion.py` script which returned all-zero
motion_intensity on most inputs (silent failure of the magick-based diff).

CLI:
    video-perception-motion-snn --input video.mp4 --scenes scenes.json --out track.json
    video-perception-motion-snn --input video.mp4 --out motion_snn.json --emit per-frame
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

import numpy as np


def probe_video(video: Path) -> dict:
    out = subprocess.check_output([
        "ffprobe", "-v", "error", "-select_streams", "v:0",
        "-show_entries", "stream=width,height,r_frame_rate,duration",
        "-of", "json", str(video),
    ])
    s = json.loads(out)["streams"][0]
    num, den = s["r_frame_rate"].split("/")
    fps = int(num) / int(den) if int(den) else 0.0
    return {
        "width": int(s["width"]),
        "height": int(s["height"]),
        "fps": fps,
        "duration_s": float(s.get("duration", 0.0)),
    }


def iter_frames(video: Path, width: int, height: int, downsample: int = 4):
    """Decode video as gray uint8 frames at native rate; yield (idx, frame).

    `downsample` reduces spatial resolution via an in-decoder scale filter.
    """
    w = width // downsample if downsample > 1 else width
    h = height // downsample if downsample > 1 else height
    vf = f"scale={w}:{h}" if downsample > 1 else None

    cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(video)]
    if vf:
        cmd += ["-vf", vf]
    cmd += ["-f", "rawvideo", "-pix_fmt", "gray", "pipe:1"]
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE)
    assert proc.stdout is not None
    frame_size = w * h
    idx = 0
    while True:
        buf = proc.stdout.read(frame_size)
        if len(buf) < frame_size:
            break
        yield idx, np.frombuffer(buf, dtype=np.uint8).reshape((h, w))
        idx += 1
    proc.wait()


def run_snn(
    video: Path,
    pixel_theta: int = 30,
    membrane_decay: float = 0.7,
    v_th: float = 4.0,
    refractory: int = 2,
    downsample: int = 4,
) -> dict:
    """Run the LIF-SNN over the video, return per-frame motion-intensity signal."""
    meta = probe_video(video)
    period = 1.0 / meta["fps"] if meta["fps"] else 1.0 / 30.0
    pixel_count = max(1, (meta["width"] // downsample) * (meta["height"] // downsample))

    per_frame: list[dict] = []
    prev: np.ndarray | None = None
    v = 0.0
    refr = 0

    t0 = time.perf_counter()
    for idx, frame in iter_frames(video, meta["width"], meta["height"], downsample):
        pts = idx * period
        if prev is None:
            prev = frame.astype(np.int16)
            per_frame.append({
                "frame": idx, "pts": round(pts, 4),
                "n_events": 0, "membrane": 0.0, "intensity": 0.0, "spike": False,
            })
            continue
        cur = frame.astype(np.int16)
        diff = cur - prev
        n_events = int(np.sum(np.abs(diff) > pixel_theta))
        prev = cur

        event_rate = n_events / pixel_count

        spike = False
        if refr > 0:
            refr -= 1
            v = 0.0
        else:
            v = membrane_decay * v + event_rate * 10.0
            if v > v_th:
                spike = True
                refr = refractory

        intensity = min(1.0, v / v_th) if not spike else 1.0
        per_frame.append({
            "frame": idx, "pts": round(pts, 4),
            "n_events": n_events, "membrane": round(v, 4),
            "intensity": round(intensity, 4), "spike": spike,
        })
        if spike:
            v = 0.0

    elapsed = time.perf_counter() - t0

    return {
        "version": "1",
        "video": str(video),
        "fps": meta["fps"],
        "width": meta["width"],
        "height": meta["height"],
        "duration_s": meta["duration_s"],
        "frame_count": len(per_frame),
        "elapsed_s": round(elapsed, 3),
        "rtf": round(elapsed / max(meta["duration_s"], 1e-6), 4),
        "params": {
            "pixel_theta": pixel_theta,
            "membrane_decay": membrane_decay,
            "v_th": v_th,
            "refractory": refractory,
            "downsample": downsample,
            "pixel_count_after_downsample": pixel_count,
        },
        "per_frame": per_frame,
    }


def classify_motion(intensity_0_30: float) -> str:
    """Match thresholds the legacy track sensor used (0-30 scale)."""
    if intensity_0_30 < 3.0:
        return "static"
    if intensity_0_30 < 15.0:
        return "subtle"
    if intensity_0_30 < 40.0:
        return "moderate"
    if intensity_0_30 < 80.0:
        return "dynamic"
    return "chaotic"


def aggregate_per_scene(snn: dict, scenes_path: Path) -> list[dict]:
    """Join per-frame intensities to scene boundaries.

    Emits the legacy `track` schema (scene_idx, start_ms, end_ms,
    motion_intensity on a 0-30 scale, motion_class, dominant_direction,
    segments) so the dashboard and downstream consumers keep working.
    """
    scenes = json.loads(scenes_path.read_text())["scenes"]
    per_frame = snn["per_frame"]
    out: list[dict] = []
    for sc in scenes:
        start_s = sc["start_ms"] / 1000.0
        end_s = sc["end_ms"] / 1000.0
        rows = [r for r in per_frame if start_s <= r["pts"] < end_s]
        if not rows:
            continue
        intensity = np.array([r["intensity"] for r in rows])
        spikes = sum(1 for r in rows if r["spike"])
        dur_s = max(end_s - start_s, 1e-6)

        # Map normalized [0,1] SNN intensity to the legacy 0-30 scale.
        intensity_scaled = round(float(intensity.mean()) * 30.0, 2)

        # Segments at 0-33%, 33-66%, 66-100% of the scene.
        segments = []
        n = len(rows)
        if n >= 3:
            for i, (lo_pct, hi_pct) in enumerate([(0, 33), (33, 66), (66, 100)]):
                lo = int(n * lo_pct / 100)
                hi = max(lo + 1, int(n * hi_pct / 100))
                seg_mean = float(intensity[lo:hi].mean()) * 30.0 if hi > lo else 0.0
                segments.append({
                    "from_pct": lo_pct,
                    "to_pct": hi_pct,
                    "intensity": round(seg_mean, 2),
                })

        out.append({
            "scene_idx": sc["idx"],
            "start_ms": sc["start_ms"],
            "end_ms": sc["end_ms"],
            "motion_intensity": intensity_scaled,
            "motion_class": classify_motion(intensity_scaled),
            # SNN doesn't compute direction; the legacy field is preserved for schema compat.
            "dominant_direction": "none",
            "segments": segments,
            # Extended SNN-specific fields (additive — won't break existing consumers).
            "snn_intensity_mean": round(float(intensity.mean()), 4),
            "snn_intensity_max": round(float(intensity.max()), 4),
            "snn_intensity_std": round(float(intensity.std()), 4),
            "snn_spike_count": spikes,
            "snn_active_ratio": round(float((intensity > 0.2).mean()), 4),
        })
    return out


def main(argv: list[str] | None = None) -> None:
    ap = argparse.ArgumentParser(prog="video-perception-motion-snn", description=__doc__)
    ap.add_argument("--input", required=True, type=Path, help="video file")
    ap.add_argument("--scenes", type=Path, help="scenes.json for per-scene aggregation (legacy track schema)")
    ap.add_argument("--out", type=Path, help="output JSON (defaults to stdout)")
    ap.add_argument("--emit", choices=["legacy", "per-frame", "both"], default="legacy",
                    help="legacy = track.json shape (default); per-frame = SNN per-frame only; both = full motion_snn.json")
    ap.add_argument("--pixel-theta", type=int, default=30)
    ap.add_argument("--membrane-decay", type=float, default=0.7)
    ap.add_argument("--v-th", type=float, default=4.0)
    ap.add_argument("--refractory", type=int, default=2)
    ap.add_argument("--downsample", type=int, default=4,
                    help="spatial downsample factor (4 = 1/16 pixels, sufficient for motion intensity)")
    args = ap.parse_args(argv)

    snn = run_snn(
        args.input,
        pixel_theta=args.pixel_theta,
        membrane_decay=args.membrane_decay,
        v_th=args.v_th,
        refractory=args.refractory,
        downsample=args.downsample,
    )

    if args.emit == "per-frame":
        payload = snn
    elif args.scenes and args.scenes.exists():
        per_scene = aggregate_per_scene(snn, args.scenes)
        if args.emit == "both":
            payload = {**snn, "per_scene": per_scene}
        else:
            payload = per_scene  # legacy track.json shape: array of SceneMotion
    else:
        # No scenes provided → can't emit legacy. Fall back to full SNN dump.
        payload = snn

    text = json.dumps(payload, indent=2)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text + "\n")
    else:
        print(text)

    print(
        f"video-perception-motion-snn: {snn['frame_count']} frames, {snn['elapsed_s']}s, rtf={snn['rtf']}x",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
