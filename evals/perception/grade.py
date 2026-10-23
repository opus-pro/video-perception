#!/usr/bin/env python3
"""Grade run logs against the fixtures' ground truth and write a summary.

    python3 evals/perception/grade.py results/raw/*.jsonl --out results/summary.md
"""

from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent


def parse_answer(text: str) -> dict | None:
    """Return the JSON object after the last `ANSWER:` marker, or None."""
    at = text.rfind("ANSWER:")
    if at < 0:
        return None
    rest = text[at + len("ANSWER:"):]
    start = rest.find("{")
    if start < 0:
        return None
    try:
        value, _ = json.JSONDecoder().raw_decode(rest[start:])
    except json.JSONDecodeError:
        return None
    return value if isinstance(value, dict) else None


def scrub(text: str) -> str:
    """Replace machine-specific paths so published logs carry no local details."""
    text = re.sub(r"/(?:private/)?var/folders/[^\"'\s]*?/T/vp-eval-[A-Za-z0-9_]+", "<tmp>", text)
    text = re.sub(r"/(?:private/)?tmp/vp-eval-[A-Za-z0-9_]+", "<tmp>", text)
    text = re.sub(r"/(?:private/)?var/folders/[^/\"'\s]+/[^/\"'\s]+/T", "<tmpdir>", text)
    text = text.replace(str(Path.home()), "~").replace("/System/Volumes/Data~", "~")
    # Names of unrelated apps and projects on the evaluation machine are not part of the result.
    text = re.sub(r"~/Library/Application Support/[^/\"']+", "~/Library/Application Support/<app>", text)
    return re.sub(r"~/code/[^/\"'\s]+", "~/<project>", text)


def num(value) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def near(value, target: float, tolerance: float) -> bool:
    v = num(value)
    return v is not None and abs(v - target) <= tolerance


def within(value, low: float, high: float) -> bool:
    """True when a spoken-moment answer falls inside the sentence span (with tolerance)."""
    v = num(value)
    return v is not None and low <= v <= high


def norm(text) -> str:
    return re.sub(r"[^a-z0-9]", "", str(text or "").lower())


def grade(task: str, answer: dict, truth: dict) -> tuple[bool, str]:
    t, l = truth["tutorial"], truth["lecture"]
    if task == "tutorial-mention":
        m = t["mention"]
        ok = within(answer.get("time_s"), m["start"] - 2.0, m["end"] + 1.0)
        return ok, f"time {answer.get('time_s')} vs sentence {m['start']}-{m['end']} (-2/+1)"
    if task == "tutorial-dead-air":
        s, e = t["dead_air"]
        ok = near(answer.get("start_s"), s, 1.5) and near(answer.get("end_s"), e, 1.5)
        return ok, f"{answer.get('start_s')}-{answer.get('end_s')} vs {s}-{e} ±1.5"
    if task == "tutorial-code":
        want = t["on_screen_text"][0]
        ok = norm(answer.get("code")) == norm(want["text"]) and near(answer.get("start_s"), want["start"], 2.0)
        return ok, f"{answer.get('code')} @ {answer.get('start_s')} vs {want['text']} @ {want['start']} ±2"
    if task == "tutorial-black":
        s, e = t["black"]
        segments = answer.get("segments") or []
        pairs = [seg for seg in segments if isinstance(seg, (list, tuple)) and len(seg) == 2]
        ok = (len(pairs) == 1 and len(segments) == 1
              and near(pairs[0][0], s, 1.0) and near(pairs[0][1], e, 1.0))
        return ok, f"{segments} vs [[{s}, {e}]] ±1, no extras"
    if task == "tutorial-spike":
        ok = near(answer.get("time_s"), t["loud_spike"]["start"], 1.0)
        return ok, f"time {answer.get('time_s')} vs {t['loud_spike']['start']} ±1"
    if task == "tutorial-cuts":
        want = len(t["cuts_s"])
        got = num(answer.get("cuts"))
        ok = got is not None and abs(got - want) <= 1
        return ok, f"{answer.get('cuts')} vs {want} ±1"
    if task == "lecture-password":
        said = norm(answer.get("password"))
        p = l["password"]
        ok = "heron" in said and ("seven" in said or "7" in said) and within(answer.get("time_s"), p["start"] - 3.0, p["end"] + 1.0)
        return ok, f"{answer.get('password')} @ {answer.get('time_s')} vs heron seven in sentence {p['start']}-{p['end']} (-3/+1)"
    if task == "lecture-room":
        want = l["on_screen_text"][0]
        ok = "b214" in norm(answer.get("room")) and near(answer.get("start_s"), want["start"], 3.0)
        return ok, f"{answer.get('room')} @ {answer.get('start_s')} vs B214 @ {want['start']} ±3"
    return False, "unknown task"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("logs", nargs="+", type=Path)
    parser.add_argument("--fixtures", type=Path, default=HERE / "fixtures")
    parser.add_argument("--out", type=Path, help="write a Markdown summary here")
    parser.add_argument("--graded", type=Path, help="write per-run grades as JSONL here")
    args = parser.parse_args()
    truth = json.loads((args.fixtures / "ground-truth.json").read_text())

    rows = []
    for log in args.logs:
        for line in log.read_text().splitlines():
            run = json.loads(line)
            answer = parse_answer(run.get("final", ""))
            ok, why = grade(run["task"], answer, truth) if answer is not None else (False, "no ANSWER line")
            rows.append({k: run.get(k) for k in ("agent", "model", "condition", "task", "attempt", "exit",
                                                  "seconds", "used_video_perception", "usage")}
                        | {"answer": answer, "pass": ok, "why": why, "commands": run.get("commands", [])})
    if not rows:
        sys.exit("no runs found")

    if args.graded:
        args.graded.write_text("".join(scrub(json.dumps(r)) + "\n" for r in rows))

    by_cell = defaultdict(list)
    for r in rows:
        by_cell[(r["agent"], r["model"], r["condition"])].append(r)
    tasks = sorted({r["task"] for r in rows})
    lines = ["| Agent | Condition | Passed | Median time | Used Video Perception |", "|---|---|---|---|---|"]
    for (agent, model, condition), runs in sorted(by_cell.items()):
        passed = sum(r["pass"] for r in runs)
        median = statistics.median(r["seconds"] for r in runs)
        used = sum(bool(r["used_video_perception"]) for r in runs)
        label = agent if model == "default" else f"{agent} ({model})"
        lines.append(f"| {label} | {condition} | {passed}/{len(runs)} | {median:.0f}s | {used}/{len(runs)} |")
    lines += ["", "| Task | " + " | ".join(f"{a}{'' if m == 'default' else ' ' + m} {c}" for a, m, c in sorted(by_cell)) + " |",
              "|---|" + "---|" * len(by_cell)]
    for task in tasks:
        cells = []
        for key in sorted(by_cell):
            runs = [r for r in by_cell[key] if r["task"] == task]
            cells.append(f"{sum(r['pass'] for r in runs)}/{len(runs)}" if runs else "-")
        lines.append(f"| {task} | " + " | ".join(cells) + " |")
    summary = "\n".join(lines) + "\n"
    print(summary)
    if args.out:
        args.out.write_text(summary)


if __name__ == "__main__":
    main()
