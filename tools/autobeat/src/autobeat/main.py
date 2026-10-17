"""Unified autobeat CLI with noun-verb subcommands."""

from __future__ import annotations

import argparse
import base64
import json
import mimetypes
import os
import shutil
import sys
from pathlib import Path
from typing import Sequence

from .analyze import analyze_audio, build_slim_result, generate_preview_from_result, write_result_json
from .errors import AutobeatError, TrackLayoutError
from .track_layout import resolve_layout


def _add_analyze_arguments(analyze_parser: argparse.ArgumentParser) -> None:
    analyze_parser.add_argument("input", type=Path, help="Input audio path")
    analyze_parser.add_argument("-o", "--output", type=Path, default=Path("timeline.json"), help="Output JSON path")
    analyze_parser.add_argument(
        "--mode",
        choices=["tight", "loose"],
        default="loose",
        help="tight: every beat, loose: one per bar",
    )
    analyze_parser.add_argument("--beats-per-bar", type=int, default=4, help="Used in loose mode")
    analyze_parser.add_argument(
        "--offset",
        type=float,
        default=0.0,
        help="Seconds. Positive shifts later, negative shifts earlier.",
    )
    analyze_parser.add_argument("--sr", type=int, default=22050, help="Analysis sample rate")
    analyze_parser.add_argument("--hop-length", type=int, default=512, help="Hop length for onset/tempogram")
    analyze_parser.add_argument("--min-bpm", type=float, default=60.0, help="Minimum accepted BPM")
    analyze_parser.add_argument("--max-bpm", type=float, default=180.0, help="Maximum accepted BPM")
    analyze_parser.add_argument("--min-confidence", type=float, default=0.65, help="Minimum accepted confidence score")
    analyze_parser.add_argument(
        "--adaptive-density",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Adapt loose-mode stride to keep spans in a practical density range.",
    )
    analyze_parser.add_argument(
        "--auto-phase",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Auto-align loose anchor phase (bar offset) using low-frequency emphasis.",
    )
    analyze_parser.add_argument(
        "--anchor-beat",
        type=int,
        default=None,
        help="Manual loose anchor beat index (1-based). Overrides auto phase when set.",
    )
    analyze_parser.add_argument(
        "--min-span-sec",
        type=float,
        default=1.0,
        help="Preferred lower bound for span duration in seconds (used by adaptive loose mode).",
    )
    analyze_parser.add_argument(
        "--max-span-sec",
        type=float,
        default=3.0,
        help="Preferred upper bound for span duration in seconds (used by adaptive loose mode).",
    )
    analyze_parser.add_argument(
        "--target-span-sec",
        type=float,
        default=2.0,
        help="Target span duration in seconds (used by adaptive loose mode).",
    )
    analyze_parser.add_argument(
        "--intent",
        choices=["auto", "vlog", "montage", "transition", "energy"],
        default="auto",
        help=(
            "Video editing intent: auto=CLI picks best strategy; "
            "vlog=natural bar-level cuts; montage=per-beat fast cuts; "
            "transition=energy/structure anchors; energy=force energy grid."
        ),
    )
    analyze_parser.add_argument(
        "--engine",
        choices=["librosa", "madmom"],
        default="librosa",
        help="Downbeat engine for loose mode. madmom is optional and falls back to librosa if needed.",
    )
    analyze_parser.add_argument(
        "--preview",
        "--preview-html",
        type=Path,
        default=None,
        help="Optional HTML preview path. Renders interactive waveform/beat preview.",
    )
    analyze_parser.add_argument(
        "--preview-embed-audio",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Embed audio directly in preview HTML as data URL. Default: enabled.",
    )
    analyze_parser.add_argument(
        "--preview-embed-max-mb",
        type=float,
        default=20.0,
        help="Max audio size to embed into preview HTML. Larger files fallback to copied sidecar audio.",
    )
    analyze_parser.add_argument(
        "--preview-copy-audio",
        action="store_true",
        help="When not embedding, copy input audio next to preview HTML and reference it via relative path.",
    )
    analyze_parser.add_argument(
        "--preview-points",
        type=int,
        default=2400,
        help="Waveform point count used in preview rendering.",
    )
    analyze_parser.add_argument(
        "--audio-path-mode",
        choices=["absolute", "relative", "basename", "omit"],
        default="absolute",
        help="How `audio.path` is written in JSON output.",
    )
    analyze_parser.add_argument(
        "--print-analysis",
        action="store_true",
        help="Print analysis summary (phase energy + genre guess) to stdout.",
    )
    analyze_parser.add_argument(
        "--verbose",
        action="store_true",
        help=(
            "Write full technical JSON (includes analysis detail, phase metrics, alternative strategy, etc.). "
            "Default writes slim LLM-friendly output."
        ),
    )


def _add_resolve_layout_arguments(resolve_parser: argparse.ArgumentParser) -> None:
    resolve_parser.add_argument("--grid", type=Path, required=True, help="Grid JSON path")
    resolve_parser.add_argument("--track", type=Path, required=True, help="Track JSON path")
    resolve_parser.add_argument(
        "-o",
        "--output",
        type=Path,
        default=Path("resolved_track.json"),
        help="Resolved output JSON",
    )


def _add_preview_arguments(preview_parser: argparse.ArgumentParser) -> None:
    preview_parser.add_argument("--grid", type=Path, required=True, help="Grid JSON from a previous analyze run")
    preview_parser.add_argument(
        "--audio",
        type=Path,
        default=None,
        help="Audio file path. Defaults to the path stored in the grid JSON (audio.path).",
    )
    preview_parser.add_argument("-o", "--output", type=Path, default=Path("preview.html"), help="Output HTML path")
    preview_parser.add_argument(
        "--preview-embed-audio",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Embed audio directly in preview HTML as data URL. Default: enabled.",
    )
    preview_parser.add_argument(
        "--preview-embed-max-mb",
        type=float,
        default=20.0,
        help="Max audio size to embed. Larger files fallback to copied sidecar audio.",
    )
    preview_parser.add_argument(
        "--preview-copy-audio",
        action="store_true",
        help="When not embedding, copy audio next to preview HTML and reference it via relative path.",
    )
    preview_parser.add_argument(
        "--preview-points",
        type=int,
        default=2400,
        help="Waveform point count used in preview rendering.",
    )


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="autobeat",
        description="Beat-grid extraction and span-track layout CLI",
    )
    parser.add_argument(
        "--version",
        action="version",
        version="autobeat 0.1.0",
    )
    subparsers = parser.add_subparsers(dest="command_group", required=True)

    # Preferred simple commands
    analyze_parser = subparsers.add_parser("analyze", help="Analyze audio and output beat-grid JSON")
    _add_analyze_arguments(analyze_parser)

    preview_parser = subparsers.add_parser("preview", help="Generate preview HTML from an existing grid JSON (no re-analysis)")
    _add_preview_arguments(preview_parser)

    resolve_layout_parser = subparsers.add_parser("resolve-layout", help="Resolve track JSON against beat-grid JSON")
    _add_resolve_layout_arguments(resolve_layout_parser)
    return parser


def _normalize_legacy_args(argv: Sequence[str]) -> list[str]:
    """Normalize shorthand/compat syntax into canonical noun-verb commands."""
    if not argv:
        return list(argv)
    first = argv[0]
    if first == "grid" and len(argv) >= 2 and argv[1] == "analyze":
        return ["analyze", *argv[2:]]
    if first == "layout" and len(argv) >= 2 and argv[1] == "resolve":
        return ["resolve-layout", *argv[2:]]
    known = {"analyze", "preview", "resolve-layout", "-h", "--help", "--version"}
    if first in known:
        return list(argv)
    return ["analyze", *argv]


def _run_grid_analyze(args: argparse.Namespace) -> int:
    if not args.input.exists() or not args.input.is_file():
        print(f"[autobeat] ERROR: Input file does not exist: {args.input}")
        return 2
    if args.preview_copy_audio and args.preview is None:
        print("[autobeat] ERROR: --preview-copy-audio requires --preview-html.")
        return 2
    if args.preview_embed_max_mb <= 0:
        print("[autobeat] ERROR: --preview-embed-max-mb must be > 0.")
        return 2

    input_abs = args.input.resolve()
    preview_audio_path: Path | None = None
    preview_audio_data_url: str | None = None
    preview_audio_fallback = False

    if args.preview is not None and args.preview_embed_audio:
        max_bytes = int(args.preview_embed_max_mb * 1024 * 1024)
        try:
            audio_size = input_abs.stat().st_size
            if audio_size <= max_bytes:
                preview_audio_data_url = _encode_audio_data_url(input_abs)
            else:
                preview_audio_path = _copy_preview_audio(input_abs=input_abs, preview_html=args.preview)
                preview_audio_fallback = True
        except Exception as exc:
            print(f"[autobeat] ERROR: Failed to prepare preview audio: {exc}")
            return 2
    elif args.preview_copy_audio and args.preview is not None:
        try:
            preview_audio_path = _copy_preview_audio(input_abs=input_abs, preview_html=args.preview)
        except Exception as exc:
            print(f"[autobeat] ERROR: Failed to copy preview audio: {exc}")
            return 2

    try:
        result = analyze_audio(
            audio_path=input_abs,
            mode=args.mode,
            beats_per_bar=args.beats_per_bar,
            offset_sec=args.offset,
            sr=args.sr,
            hop_length=args.hop_length,
            min_bpm=args.min_bpm,
            max_bpm=args.max_bpm,
            min_confidence=args.min_confidence,
            adaptive_density=args.adaptive_density,
            auto_phase=args.auto_phase,
            anchor_beat=args.anchor_beat,
            min_span_sec=args.min_span_sec,
            max_span_sec=args.max_span_sec,
            target_span_sec=args.target_span_sec,
            engine=args.engine,
            preview_html=args.preview,
            preview_audio_data_url=preview_audio_data_url,
            preview_audio_path=preview_audio_path,
            preview_points=args.preview_points,
            intent=args.intent,
        )
        json_audio_source = input_abs
        if preview_audio_path is not None and args.audio_path_mode in {"relative", "basename"}:
            json_audio_source = preview_audio_path.resolve()
        result["audio"]["path"] = _format_audio_path(
            input_path=json_audio_source,
            output_path=args.output,
            mode=args.audio_path_mode,
        )
        output_payload = result if args.verbose else build_slim_result(result)
        write_result_json(args.output, output_payload)
    except AutobeatError as exc:
        print(f"[autobeat] ERROR: {exc}")
        return 2
    except Exception as exc:
        print(f"[autobeat] ERROR: Unexpected failure: {exc}")
        return 2

    sel = result["selected"]
    alt = result.get("alternative")
    print(
        f"[autobeat] wrote: {args.output} "
        f"(strategy={sel['strategy']}, intent={sel['intent']}, "
        f"anchors={len(sel['grid']['anchors_sec'])}, spans={len(sel['grid']['spans'])}, "
        f"quality={sel['quality']['tier']})"
    )
    print(f"[autobeat] hint: {sel['quality']['agent_hint']}")
    if alt:
        print(f"[autobeat] alternative: {alt['strategy']} — {alt['use_when']}")
    if args.preview is not None:
        print(f"[autobeat] preview: {args.preview}")
        if preview_audio_data_url is not None:
            print("[autobeat] preview audio mode: embedded (data URL)")
        if preview_audio_fallback and preview_audio_path is not None:
            print("[autobeat] preview audio mode: sidecar (fallback from embed due size limit)")
            print(f"[autobeat] preview audio copied: {preview_audio_path}")
        elif args.preview_copy_audio and preview_audio_path is not None:
            print(f"[autobeat] preview audio copied: {preview_audio_path}")
    if args.print_analysis:
        analysis = sel["analysis"]
        print(
            f"[autobeat] analysis: tempo={analysis.get('tempo_bpm', 0):.2f} "
            f"confidence={analysis.get('confidence', 0):.3f} "
            f"selected_beat={analysis.get('selected_anchor_beat', 1)} "
            f"recommended_beat={analysis.get('recommended_anchor_beat', 1)} "
            f"genre={analysis.get('genre_guess', {}).get('primary', 'unknown')}"
        )
        phase_profile = analysis.get("phase_energy_profile", [])
        for item in phase_profile:
            print(
                "[autobeat] phase beat={beat} score={score:.3f} "
                "low={low:.3f} onset={onset:.3f} n={n}".format(
                    beat=item.get("beat"),
                    score=float(item.get("score", 0.0)),
                    low=float(item.get("low_energy", 0.0)),
                    onset=float(item.get("onset_energy", 0.0)),
                    n=int(item.get("sample_count", 0)),
                )
            )
    return 0


def _format_audio_path(*, input_path: Path, output_path: Path, mode: str) -> str:
    resolved_input = input_path.resolve()
    if mode == "absolute":
        return str(resolved_input)
    if mode == "relative":
        return os.path.relpath(resolved_input, start=output_path.parent.resolve())
    if mode == "basename":
        return resolved_input.name
    if mode == "omit":
        return ""
    return str(resolved_input)


def _copy_preview_audio(*, input_abs: Path, preview_html: Path) -> Path:
    preview_audio_path = preview_html.parent.resolve() / input_abs.name
    preview_audio_path.parent.mkdir(parents=True, exist_ok=True)
    if preview_audio_path != input_abs:
        shutil.copy2(input_abs, preview_audio_path)
    return preview_audio_path


def _encode_audio_data_url(audio_path: Path) -> str:
    mime_type, _ = mimetypes.guess_type(audio_path.name)
    if mime_type is None:
        mime_type = "audio/mpeg"
    raw = audio_path.read_bytes()
    encoded = base64.b64encode(raw).decode("ascii")
    return f"data:{mime_type};base64,{encoded}"


def _run_preview(args: argparse.Namespace) -> int:
    if not args.grid.exists() or not args.grid.is_file():
        print(f"[autobeat] ERROR: Grid JSON does not exist: {args.grid}")
        return 2
    if args.preview_copy_audio and args.output is None:
        print("[autobeat] ERROR: --preview-copy-audio requires -o / --output.")
        return 2
    if args.preview_embed_max_mb <= 0:
        print("[autobeat] ERROR: --preview-embed-max-mb must be > 0.")
        return 2

    try:
        result = json.loads(args.grid.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"[autobeat] ERROR: Failed to parse grid JSON: {exc}")
        return 2

    # Resolve audio path: explicit --audio wins, then fall back to JSON audio.path
    if args.audio is not None:
        audio_path = args.audio.resolve()
    else:
        stored = result.get("audio", {}).get("path", "")
        if not stored:
            print("[autobeat] ERROR: --audio not provided and audio.path is missing/empty in JSON.")
            return 2
        audio_path = Path(stored)

    if not audio_path.exists() or not audio_path.is_file():
        print(f"[autobeat] ERROR: Audio file does not exist: {audio_path}")
        return 2

    preview_audio_path: Path | None = None
    preview_audio_data_url: str | None = None
    preview_audio_fallback = False

    if args.preview_embed_audio:
        max_bytes = int(args.preview_embed_max_mb * 1024 * 1024)
        try:
            if audio_path.stat().st_size <= max_bytes:
                preview_audio_data_url = _encode_audio_data_url(audio_path)
            else:
                preview_audio_path = _copy_preview_audio(input_abs=audio_path, preview_html=args.output)
                preview_audio_fallback = True
        except Exception as exc:
            print(f"[autobeat] ERROR: Failed to prepare preview audio: {exc}")
            return 2
    elif args.preview_copy_audio:
        try:
            preview_audio_path = _copy_preview_audio(input_abs=audio_path, preview_html=args.output)
        except Exception as exc:
            print(f"[autobeat] ERROR: Failed to copy preview audio: {exc}")
            return 2

    try:
        generate_preview_from_result(
            result=result,
            audio_path=audio_path,
            preview_path=args.output,
            preview_audio_data_url=preview_audio_data_url,
            preview_audio_path=preview_audio_path,
            preview_points=args.preview_points,
        )
    except AutobeatError as exc:
        print(f"[autobeat] ERROR: {exc}")
        return 2
    except Exception as exc:
        print(f"[autobeat] ERROR: Unexpected failure: {exc}")
        return 2

    print(f"[autobeat] preview: {args.output}")
    if preview_audio_data_url is not None:
        print("[autobeat] preview audio mode: embedded (data URL)")
    elif preview_audio_fallback and preview_audio_path is not None:
        print("[autobeat] preview audio mode: sidecar (fallback from embed due to size limit)")
        print(f"[autobeat] preview audio copied: {preview_audio_path}")
    elif args.preview_copy_audio and preview_audio_path is not None:
        print(f"[autobeat] preview audio copied: {preview_audio_path}")
    return 0


def _run_layout_resolve(args: argparse.Namespace) -> int:
    if not args.grid.exists() or not args.grid.is_file():
        print(f"[autobeat-layout] ERROR: Grid file does not exist: {args.grid}")
        return 2
    if not args.track.exists() or not args.track.is_file():
        print(f"[autobeat-layout] ERROR: Track file does not exist: {args.track}")
        return 2

    try:
        grid_payload = json.loads(args.grid.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"[autobeat-layout] ERROR: Failed to parse grid JSON: {exc}")
        return 2

    try:
        track_payload = json.loads(args.track.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"[autobeat-layout] ERROR: Failed to parse track JSON: {exc}")
        return 2

    try:
        resolved = resolve_layout(grid_payload, track_payload)
    except TrackLayoutError as exc:
        print(f"[autobeat-layout] ERROR: {exc}")
        return 2
    except Exception as exc:
        print(f"[autobeat-layout] ERROR: Unexpected failure: {exc}")
        return 2

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(resolved, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"[autobeat-layout] wrote: {args.output} (clips={len(resolved['clips'])})")
    return 0


def run(argv: Sequence[str] | None = None) -> int:
    args_raw = list(sys.argv[1:] if argv is None else argv)
    parser = _build_parser()
    args = parser.parse_args(_normalize_legacy_args(args_raw))

    if args.command_group == "analyze":
        return _run_grid_analyze(args)
    if args.command_group == "preview":
        return _run_preview(args)
    if args.command_group == "resolve-layout":
        return _run_layout_resolve(args)

    parser.print_help()
    return 2


def main_entry() -> None:
    """Main console-script entrypoint."""
    raise SystemExit(run())


def layout_entry() -> None:
    """Legacy `autobeat-layout` alias."""
    raise SystemExit(run(["resolve-layout", *sys.argv[1:]]))


if __name__ == "__main__":
    main_entry()
