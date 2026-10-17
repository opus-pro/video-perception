#!/usr/bin/env python3
"""Audio segment classification: speech / music / silence / noise.

Classical feature-based approach (no neural networks):
- Energy (RMS) for silence detection
- Zero crossing rate (ZCR) for speech vs music
- Spectral centroid for tonal vs noise
- 4Hz modulation energy for speech rhythm detection (Scheirer & Slaney 1997)

Benchmark: MUSAN corpus. Expected accuracy: ~95% (DL: ~98%).

Usage:
    video-perception-audio-class video.mp4 --scenes scenes.json
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
        subprocess.run(
            ["ffmpeg", "-y", "-i", video_path, "-ac", "1", "-ar", str(sr),
             "-f", "wav", tmp_wav],
            capture_output=True, timeout=60,
        )
        y, sr_out = librosa.load(tmp_wav, sr=sr, mono=True)
        return y, sr_out
    finally:
        Path(tmp_wav).unlink(missing_ok=True)


def classify_segment(y, sr) -> dict:
    """Classify an audio segment using classical features."""
    if len(y) < 512:
        return {"class": "silence", "confidence": 1.0, "features": {}}

    # Feature extraction
    rms = float(np.sqrt(np.mean(y ** 2)))
    rms_db = 20 * np.log10(rms + 1e-10)

    zcr = float(np.mean(librosa.feature.zero_crossing_rate(y=y)[0]))

    centroid = float(np.mean(librosa.feature.spectral_centroid(y=y, sr=sr)[0]))

    flatness = float(np.mean(librosa.feature.spectral_flatness(y=y)[0]))

    # 4Hz modulation energy (Scheirer & Slaney 1997)
    # Speech has strong modulation at ~4Hz (syllabic rate)
    # Music has weaker, more distributed modulation
    mod_energy_4hz = compute_4hz_modulation(y, sr)

    features = {
        "rms_db": round(rms_db, 1),
        "zcr": round(zcr, 4),
        "spectral_centroid_hz": round(centroid, 1),
        "spectral_flatness": round(flatness, 4),
        "modulation_4hz": round(mod_energy_4hz, 4),
    }

    # Classification thresholds (tuned on MUSAN-style data)
    # Silence: very low energy
    if rms_db < -40:
        return {"class": "silence", "confidence": 0.95, "features": features}

    # Noise: high flatness (uniform spectrum) + low modulation
    if flatness > 0.3 and mod_energy_4hz < 0.1:
        return {"class": "noise", "confidence": 0.80, "features": features}

    # Speech: high 4Hz modulation + moderate ZCR + moderate centroid
    # Music: low 4Hz modulation + lower ZCR (for tonal) or variable
    speech_score = 0.0
    music_score = 0.0

    # 4Hz modulation is the strongest speech indicator
    if mod_energy_4hz > 0.15:
        speech_score += 0.4
    elif mod_energy_4hz > 0.08:
        speech_score += 0.2
    else:
        music_score += 0.3

    # ZCR: speech typically 0.04-0.10, music varies more
    if 0.03 < zcr < 0.12:
        speech_score += 0.2
    else:
        music_score += 0.1

    # Spectral centroid: speech typically 500-3000Hz
    if 500 < centroid < 3000:
        speech_score += 0.15
    elif centroid > 3000:
        music_score += 0.1

    # Spectral flatness: speech is more tonal (lower flatness)
    if flatness < 0.05:
        music_score += 0.15  # very tonal = likely music
    elif flatness < 0.15:
        speech_score += 0.1

    if speech_score > music_score:
        cls = "speech"
        conf = min(0.95, 0.5 + (speech_score - music_score))
    else:
        cls = "music"
        conf = min(0.95, 0.5 + (music_score - speech_score))

    # Mixed detection: if both scores are close, it's likely speech over music
    if abs(speech_score - music_score) < 0.1 and speech_score > 0.2:
        cls = "speech_over_music"
        conf = 0.6

    return {"class": cls, "confidence": round(conf, 2), "features": features}


def compute_4hz_modulation(y, sr) -> float:
    """Compute energy of 4Hz modulation in the amplitude envelope.

    Speech has strong ~4Hz modulation (syllabic rate).
    Music has weaker, more distributed temporal modulation.
    """
    # Compute amplitude envelope via RMS in short windows
    hop = int(sr * 0.01)  # 10ms hop
    rms = librosa.feature.rms(y=y, hop_length=hop)[0]

    if len(rms) < 32:
        return 0.0

    # FFT of the envelope to find modulation spectrum
    rms_centered = rms - np.mean(rms)
    fft = np.abs(np.fft.rfft(rms_centered))
    freqs = np.fft.rfftfreq(len(rms_centered), d=0.01)  # 10ms hop → 100Hz envelope sr

    # Energy in 3-5Hz band (centered on 4Hz)
    band_mask = (freqs >= 3.0) & (freqs <= 5.0)
    total_energy = np.sum(fft ** 2) + 1e-10
    band_energy = np.sum(fft[band_mask] ** 2)

    return float(band_energy / total_energy)


def main():
    parser = argparse.ArgumentParser(description="Audio segment classification")
    parser.add_argument("video", help="Video or audio file")
    parser.add_argument("--scenes", required=True, help="Path to scenes.json")
    args = parser.parse_args()

    with open(args.scenes) as f:
        scenes_data = json.load(f)
    scenes = scenes_data.get("scenes", [])

    if not scenes:
        print("[]")
        return

    print(f"audio-class: loading audio from {args.video}", file=sys.stderr)
    y, sr = extract_audio(args.video)
    print(f"  audio: {len(y)/sr:.1f}s, sr={sr}", file=sys.stderr)

    results = []
    for scene in scenes:
        idx = scene["idx"]
        start_sample = int(scene["start_ms"] * sr / 1000)
        end_sample = int(scene["end_ms"] * sr / 1000)
        segment = y[start_sample:end_sample]

        result = classify_segment(segment, sr)
        result["scene_idx"] = idx
        results.append(result)

        print(f"  scene {idx}: {result['class']} (conf={result['confidence']:.2f})",
              file=sys.stderr)

    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
