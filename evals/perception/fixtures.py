#!/usr/bin/env python3
"""Build the synthetic evaluation videos and their ground truth.

Every event in these videos (cuts, on-screen text, speech, silence, black
frames, a loud spike) is placed by this script at a known time, so answers can
be graded objectively. Nothing is downloaded and no media is committed.

Requirements: macOS `say` (speech), FFmpeg, and ImageMagick (`magick`).

    python3 evals/perception/fixtures.py            # writes evals/perception/fixtures/
    python3 evals/perception/fixtures.py --out DIR
"""

from __future__ import annotations

import argparse
import json
import math
import shutil
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

HERE = Path(__file__).resolve().parent
RATE = 24000  # mono 16-bit PCM for the speech track
FONT = "/System/Library/Fonts/Supplemental/Arial.ttf"
FONT_BOLD = "/System/Library/Fonts/Supplemental/Arial Bold.ttf"
VOICE = "Samantha"
SPEECH_GAIN = 0.35
SPIKE_GAIN = 0.98


def run(cmd: list[str]) -> None:
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


# ---------------------------------------------------------------- audio


def speak(text: str, work: Path, index: int) -> list[int]:
    """Render one sentence with macOS `say`; return 16-bit samples at RATE."""
    path = work / f"say-{index:04d}.wav"
    run(["say", "-v", VOICE, "-o", str(path), f"--data-format=LEI16@{RATE}", text])
    with wave.open(str(path), "rb") as handle:
        frames = handle.readframes(handle.getnframes())
    samples = list(struct.unpack(f"<{len(frames) // 2}h", frames))
    # Trim leading/trailing near-silence so placement offsets equal speech onsets.
    threshold = 400
    start = next((i for i, s in enumerate(samples) if abs(s) > threshold), 0)
    end = len(samples) - next((i for i, s in enumerate(reversed(samples)) if abs(s) > threshold), 0)
    return samples[start:end]


class Track:
    def __init__(self, seconds: float) -> None:
        self.buf = [0.0] * int(seconds * RATE)

    def place(self, at: float, samples: list[int], gain: float) -> float:
        offset = int(at * RATE)
        for i, s in enumerate(samples):
            j = offset + i
            if j < len(self.buf):
                self.buf[j] += gain * s / 32768.0
        return at + len(samples) / RATE

    def tone(self, at: float, seconds: float, hz: float, gain: float) -> None:
        offset = int(at * RATE)
        for i in range(int(seconds * RATE)):
            self.buf[offset + i] += gain * math.sin(2 * math.pi * hz * i / RATE)

    def write(self, path: Path) -> None:
        pcm = b"".join(struct.pack("<h", int(max(-1.0, min(1.0, v)) * 32767)) for v in self.buf)
        with wave.open(str(path), "wb") as handle:
            handle.setnchannels(1)
            handle.setsampwidth(2)
            handle.setframerate(RATE)
            handle.writeframes(pcm)


def lay_speech(track: Track, lines: list[tuple[float, str]], work: Path) -> list[dict]:
    spans = []
    for index, (at, text) in enumerate(lines):
        end = track.place(at, speak(text, work, index), SPEECH_GAIN)
        spans.append({"start": round(at, 3), "end": round(end, 3), "text": text})
    for a, b in zip(spans, spans[1:]):
        if b["start"] < a["end"]:
            sys.exit(f"speech overlaps: {a['text']!r} ends after {b['text']!r} starts")
    return spans


def silences(spans: list[dict], duration: float, longer_than: float) -> list[list[float]]:
    gaps, cursor = [], 0.0
    for span in spans:
        if span["start"] - cursor > longer_than:
            gaps.append([round(cursor, 3), round(span["start"], 3)])
        cursor = span["end"]
    if duration - cursor > longer_than:
        gaps.append([round(cursor, 3), round(duration, 3)])
    return gaps


# ---------------------------------------------------------------- video


def slide(path: Path, size: str, bg: str, title: str, body: str = "", extra: str = "") -> None:
    cmd = ["magick", "-size", size, f"xc:{bg}", "-fill", "white", "-gravity", "center",
           "-font", FONT_BOLD, "-pointsize", "64", "-annotate", "+0-60", title]
    if body:
        cmd += ["-font", FONT, "-pointsize", "34", "-annotate", "+0+40", body]
    if extra:
        cmd += ["-fill", "#fde047", "-font", FONT_BOLD, "-pointsize", "48", "-annotate", "+0+150", extra]
    run(cmd + [str(path)])


def still(image: Path, seconds: float, fps: int, out: Path) -> None:
    run(["ffmpeg", "-y", "-loop", "1", "-framerate", str(fps), "-t", f"{seconds}", "-i", str(image),
         "-vf", "format=yuv420p", "-c:v", "libx264", "-preset", "veryfast", "-r", str(fps), str(out)])


def synthetic(source: str, seconds: float, fps: int, size: str, out: Path) -> None:
    run(["ffmpeg", "-y", "-f", "lavfi", "-i", f"{source}=size={size}:rate={fps}",
         "-t", f"{seconds}", "-vf", "format=yuv420p", "-c:v", "libx264", "-preset", "veryfast",
         "-r", str(fps), str(out)])


def assemble(segments: list[Path], audio: Path, out: Path, work: Path) -> None:
    listing = work / f"{out.stem}-segments.txt"
    listing.write_text("".join(f"file '{p}'\n" for p in segments))
    run(["ffmpeg", "-y", "-f", "concat", "-safe", "0", "-i", str(listing), "-i", str(audio),
         "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-b:a", "96k",
         "-shortest", "-movflags", "+faststart", str(out)])


# ---------------------------------------------------------------- fixtures


def tutorial(out_dir: Path, work: Path) -> dict:
    """A 92 s product walkthrough: slides, demo footage, Q&A dead air, a black gap, a loud beep."""
    size, fps, duration = "1280x720", 30, 92.0
    w = work / "tutorial"
    w.mkdir()
    slide(w / "welcome.png", size, "#1e3a8a", "Welcome to the Editor", "A quick walkthrough")
    slide(w / "pricing.png", size, "#166534", "Pricing", "Plans for every creator")
    slide(w / "pricing-code.png", size, "#166534", "Pricing", "Plans for every creator",
          "Discount code: KESTREL-42")
    slide(w / "questions.png", size, "#6b21a8", "Questions", "From the live chat")
    slide(w / "black.png", size, "black", " ")
    slide(w / "thanks.png", size, "#c2410c", "Thanks for watching", "See you next time")
    segs = []
    def add_still(name: str, seconds: float) -> None:
        p = w / f"seg{len(segs):02d}.mp4"
        still(w / f"{name}.png", seconds, fps, p)
        segs.append(p)
    def add_synth(source: str, seconds: float) -> None:
        p = w / f"seg{len(segs):02d}.mp4"
        synthetic(source, seconds, fps, size, p)
        segs.append(p)
    add_still("welcome", 12)        # 0-12
    add_synth("testsrc2", 12)       # 12-24
    add_still("pricing", 2)         # 24-26
    add_still("pricing-code", 7)    # 26-33 code visible
    add_still("pricing", 3)         # 33-36
    add_synth("mandelbrot", 14)     # 36-50
    add_still("questions", 20)      # 50-70
    add_still("black", 3)           # 70-73
    add_still("thanks", 19)         # 73-92

    lines = [
        (1.0, "Hi everyone, welcome to this quick walkthrough of our new editor."),
        (6.5, "Today I will show you the timeline, rendering, and pricing."),
        (12.5, "Here is the main workspace. You can drag clips onto the timeline."),
        (18.5, "Trimming works by dragging the edges of each clip."),
        (24.5, "Now let's talk about pricing. Early users get a discount."),
        (30.0, "The starter plan stays free for personal projects."),
        (36.5, "This next part shows rendering with the graphics card enabled."),
        (43.5, "The new Lumina Pro plan includes unlimited exports."),
        (50.0, "Let me answer a few questions from the chat."),
        (66.0, "Okay, that was the last question."),
        (70.5, "Thanks for watching this walkthrough."),
        (75.5, "If you have feedback, leave a comment below."),
        (80.5, "Our next video covers color grading."),
        (87.5, "Bye for now."),
    ]
    track = Track(duration)
    spans = lay_speech(track, lines, w)
    spike_at = 85.0
    track.tone(spike_at, 0.6, 1000.0, SPIKE_GAIN)
    wav = w / "audio.wav"
    track.write(wav)
    out = out_dir / "tutorial.mp4"
    assemble(segs, wav, out, w)

    dead_air = silences(spans, duration, 5.0)
    if len(dead_air) != 1:
        sys.exit(f"tutorial: expected exactly one silence > 5 s, got {dead_air}")
    mention = next(s for s in spans if "Lumina Pro" in s["text"])
    return {
        "video": out.name,
        "duration_s": duration,
        "cuts_s": [12.0, 24.0, 36.0, 50.0, 70.0, 73.0],
        "on_screen_text": [{"text": "KESTREL-42", "start": 26.0, "end": 33.0}],
        "mention": {"phrase": "Lumina Pro", "start": mention["start"], "end": mention["end"]},
        "dead_air": dead_air[0],
        "black": [70.0, 73.0],
        "loud_spike": {"start": spike_at, "end": spike_at + 0.6},
        "speech": spans,
    }


FILLER = [
    "Let's continue with the next idea in this section.",
    "This pattern shows up in many real systems.",
    "Take a moment to compare it with the previous example.",
    "Notice how the inputs change over time.",
    "We will come back to this point later in the course.",
    "The diagram on the slide summarizes the main steps.",
    "Many students find this part easier with practice.",
    "Keep the definitions from last week in mind.",
    "Here the order of operations matters a lot.",
    "Try to predict the result before we run it.",
    "That covers the basic case; now consider an edge case.",
    "Write down any questions and bring them to office hours.",
]


def lecture(out_dir: Path, work: Path) -> dict:
    """A 12 min lecture: 18 slides, continuous speech, one spoken password, one 4 s on-screen flash."""
    size, fps, duration, slide_s = "854x480", 15, 720.0, 40
    w = work / "lecture"
    w.mkdir()
    palette = ["#0f172a", "#1e293b", "#172554", "#14532d", "#3b0764", "#450a0a"]
    flash = {"text": "Room B214", "start": 431.0, "end": 435.0}
    segs = []
    for n in range(int(duration // slide_s)):
        start = n * slide_s
        bg = palette[n % len(palette)]
        base = w / f"slide{n:02d}.png"
        slide(base, size, bg, f"Lecture 4 - Slide {n + 1}", "Systems and signals")
        if start <= flash["start"] < start + slide_s:
            flashed = w / f"slide{n:02d}-flash.png"
            slide(flashed, size, bg, f"Lecture 4 - Slide {n + 1}", "Systems and signals",
                  "Exam review: " + flash["text"])
            parts = [(base, flash["start"] - start), (flashed, flash["end"] - flash["start"]),
                     (base, start + slide_s - flash["end"])]
        else:
            parts = [(base, slide_s)]
        for image, seconds in parts:
            p = w / f"seg{len(segs):03d}.mp4"
            still(image, seconds, fps, p)
            segs.append(p)

    secret = (557.0, "Remember, the exam password is heron seven.")
    lines, t, k = [], 1.0, 0
    while t < duration - 6:
        if secret[0] - 6.0 < t < secret[0] + 5.0:
            t = secret[0] + 5.0
            continue
        lines.append((t, FILLER[k % len(FILLER)]))
        k += 1
        t += 6.0
    lines.append(secret)
    lines.sort()
    track = Track(duration)
    spans = lay_speech(track, lines, w)
    wav = w / "audio.wav"
    track.write(wav)
    out = out_dir / "lecture.mp4"
    assemble(segs, wav, out, w)
    said = next(s for s in spans if "password" in s["text"])
    return {
        "video": out.name,
        "duration_s": duration,
        "on_screen_text": [flash],
        "password": {"answer": "heron seven", "start": said["start"], "end": said["end"]},
        "speech": spans,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=HERE / "fixtures")
    args = parser.parse_args()
    for tool in ("say", "ffmpeg", "magick"):
        if not shutil.which(tool):
            sys.exit(f"missing required tool: {tool}")
    args.out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="vp-fixtures-") as tmp:
        work = Path(tmp)
        truth = {"tutorial": tutorial(args.out, work), "lecture": lecture(args.out, work)}
    (args.out / "ground-truth.json").write_text(json.dumps(truth, indent=2) + "\n")
    print(f"wrote {args.out}/tutorial.mp4, lecture.mp4, ground-truth.json")


if __name__ == "__main__":
    main()
