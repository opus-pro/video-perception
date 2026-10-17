#!/usr/bin/env python3
"""Audio spectral analysis sensor.

Computes per-scene spectral features from audio:
- spectral centroid (brightness)
- spectral bandwidth
- spectral rolloff (energy concentration)
- spectral flatness (tonal vs noisy)
- MFCCs (13 coefficients, mean per scene)

Usage:
    video-perception-spectral video.mp4 --scenes scenes.json
    video-perception-spectral audio.wav --scenes scenes.json
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

import librosa
import numpy as np


def extract_audio(video_path: str, sr: int = 22050) -> tuple:
    """Extract audio from video, return (y, sr)."""
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as f:
        tmp_wav = f.name

    try:
        result = subprocess.run(
            ["ffmpeg", "-y", "-i", video_path, "-ac", "1", "-ar", str(sr),
             "-f", "wav", tmp_wav],
            capture_output=True, timeout=60,
        )
        if result.returncode != 0:
            raise RuntimeError(f"ffmpeg failed: {result.stderr[:200]}")

        y, sr_out = librosa.load(tmp_wav, sr=sr, mono=True)
        return y, sr_out
    finally:
        Path(tmp_wav).unlink(missing_ok=True)


def analyze_segment(y, sr, start_ms, end_ms) -> dict:
    """Compute spectral features for an audio segment."""
    start_sample = int(start_ms * sr / 1000)
    end_sample = int(end_ms * sr / 1000)
    segment = y[start_sample:end_sample]

    if len(segment) < 512:
        return {"error": "segment too short"}

    # Spectral features
    centroid = librosa.feature.spectral_centroid(y=segment, sr=sr)[0]
    bandwidth = librosa.feature.spectral_bandwidth(y=segment, sr=sr)[0]
    rolloff = librosa.feature.spectral_rolloff(y=segment, sr=sr)[0]
    flatness = librosa.feature.spectral_flatness(y=segment)[0]
    mfccs = librosa.feature.mfcc(y=segment, sr=sr, n_mfcc=13)

    # Zero crossing rate
    zcr = librosa.feature.zero_crossing_rate(y=segment)[0]

    # RMS energy
    rms = librosa.feature.rms(y=segment)[0]

    return {
        "spectral_centroid_hz": round(float(np.mean(centroid)), 1),
        "spectral_centroid_std": round(float(np.std(centroid)), 1),
        "spectral_bandwidth_hz": round(float(np.mean(bandwidth)), 1),
        "spectral_rolloff_hz": round(float(np.mean(rolloff)), 1),
        "spectral_flatness": round(float(np.mean(flatness)), 4),
        "zero_crossing_rate": round(float(np.mean(zcr)), 4),
        "rms_energy": round(float(np.mean(rms)), 4),
        "rms_energy_db": round(float(20 * np.log10(np.mean(rms) + 1e-10)), 1),
        "mfcc_means": [round(float(np.mean(mfccs[i])), 2) for i in range(13)],
    }


def main():
    parser = argparse.ArgumentParser(description="Audio spectral analysis sensor")
    parser.add_argument("video", help="Video or audio file")
    parser.add_argument("--scenes", required=True, help="Path to scenes.json")
    parser.add_argument("--sr", type=int, default=22050, help="Sample rate")
    args = parser.parse_args()

    # Load scenes
    with open(args.scenes) as f:
        scenes_data = json.load(f)
    scenes = scenes_data.get("scenes", [])

    if not scenes:
        print("[]")
        return

    # Extract audio
    print(f"spectral: loading audio from {args.video}", file=sys.stderr)
    y, sr = extract_audio(args.video, args.sr)
    duration_ms = len(y) / sr * 1000
    print(f"  audio: {duration_ms:.0f}ms, sr={sr}", file=sys.stderr)

    # Analyze per scene
    results = []
    for scene in scenes:
        idx = scene["idx"]
        start = scene["start_ms"]
        end = scene["end_ms"]

        features = analyze_segment(y, sr, start, end)
        features["scene_idx"] = idx

        if "error" not in features:
            # Classify audio texture from features
            centroid = features["spectral_centroid_hz"]
            flatness = features["spectral_flatness"]
            if flatness > 0.1:
                texture = "noisy"
            elif centroid > 3000:
                texture = "bright"
            elif centroid < 1000:
                texture = "dark"
            else:
                texture = "balanced"
            features["audio_texture"] = texture

            print(f"  scene {idx}: centroid={centroid:.0f}Hz flatness={flatness:.4f} → {texture}",
                  file=sys.stderr)

        results.append(features)

    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
