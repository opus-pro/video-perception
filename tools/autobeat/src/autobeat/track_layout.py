"""Resolve span-based track layout to timeline seconds."""

from __future__ import annotations

from typing import Any

from .errors import TrackLayoutError


def _validate_anchors(anchors: list[float]) -> None:
    if len(anchors) < 2:
        raise TrackLayoutError("grid.anchors_sec must contain at least 2 anchor points.")
    for i in range(1, len(anchors)):
        if anchors[i] <= anchors[i - 1]:
            raise TrackLayoutError("grid.anchors_sec must be strictly increasing.")


def _has_overlap(a_start: int, a_end: int, b_start: int, b_end: int) -> bool:
    return max(a_start, b_start) < min(a_end, b_end)


def _resolve_clip(
    *,
    clip_id: str,
    span_len: int,
    start_span_idx: int,
    anchors: list[float],
    span_count: int,
) -> dict[str, Any]:
    if span_len <= 0:
        raise TrackLayoutError(f"clip={clip_id!r}: span_len must be positive.")
    if start_span_idx < 0:
        raise TrackLayoutError(f"clip={clip_id!r}: start_span_idx must be >= 0.")

    end_span_idx = start_span_idx + span_len
    if end_span_idx > span_count:
        raise TrackLayoutError(
            f"clip={clip_id!r}: end_span_idx={end_span_idx} exceeds span_count={span_count}."
        )

    start_sec = float(anchors[start_span_idx])
    end_sec = float(anchors[end_span_idx])
    return {
        "id": clip_id,
        "span_len": int(span_len),
        "start_span_idx": int(start_span_idx),
        "end_span_idx": int(end_span_idx),
        "start_sec": start_sec,
        "end_sec": end_sec,
        "dur_sec": float(end_sec - start_sec),
    }

def resolve_layout(grid_payload: dict[str, Any], track_payload: dict[str, Any]) -> dict[str, Any]:
    """Resolve track payload against beat-grid anchors."""
    if "grid" in grid_payload:
        anchors_raw = grid_payload.get("grid", {}).get("anchors_sec")
    else:
        anchors_raw = grid_payload.get("anchors_sec")
    if not isinstance(anchors_raw, list) or not anchors_raw:
        raise TrackLayoutError("Missing anchors_sec in grid payload.")

    anchors = [float(v) for v in anchors_raw]
    _validate_anchors(anchors)
    span_count = len(anchors) - 1

    track_mode = track_payload.get("track_mode", "magnetic")
    if track_mode not in {"magnetic", "absolute"}:
        raise TrackLayoutError("track_mode must be one of: magnetic, absolute.")

    allow_overlap = bool(track_payload.get("allow_overlap", False))
    clips_raw = track_payload.get("clips")
    if not isinstance(clips_raw, list) or len(clips_raw) == 0:
        raise TrackLayoutError("track_payload.clips must be a non-empty array.")

    resolved: list[dict[str, Any]] = []
    if track_mode == "magnetic":
        cursor = 0
        for i, clip in enumerate(clips_raw):
            clip_id = str(clip.get("id", f"clip_{i}"))
            span_len = int(clip.get("span_len", 0))
            resolved_clip = _resolve_clip(
                clip_id=clip_id,
                span_len=span_len,
                start_span_idx=cursor,
                anchors=anchors,
                span_count=span_count,
            )
            resolved.append(resolved_clip)
            cursor = int(resolved_clip["end_span_idx"])
    else:
        for i, clip in enumerate(clips_raw):
            clip_id = str(clip.get("id", f"clip_{i}"))
            span_len = int(clip.get("span_len", 0))
            if "start_span_idx" not in clip:
                raise TrackLayoutError(
                    f"clip={clip_id!r}: start_span_idx is required when track_mode=absolute."
                )
            start_span_idx = int(clip.get("start_span_idx"))
            resolved_clip = _resolve_clip(
                clip_id=clip_id,
                span_len=span_len,
                start_span_idx=start_span_idx,
                anchors=anchors,
                span_count=span_count,
            )
            resolved.append(resolved_clip)

        if not allow_overlap:
            for i in range(len(resolved)):
                for j in range(i + 1, len(resolved)):
                    a = resolved[i]
                    b = resolved[j]
                    if _has_overlap(
                        int(a["start_span_idx"]),
                        int(a["end_span_idx"]),
                        int(b["start_span_idx"]),
                        int(b["end_span_idx"]),
                    ):
                        raise TrackLayoutError(
                            (
                                "Absolute track has overlapping clips while allow_overlap=false: "
                                f"{a['id']!r} overlaps {b['id']!r}."
                            )
                        )

    max_end_span_idx = max(int(c["end_span_idx"]) for c in resolved)
    total_sec = float(anchors[max_end_span_idx])
    return {
        "track_mode": track_mode,
        "allow_overlap": allow_overlap,
        "grid_ref": {
            "anchor_count": len(anchors),
            "span_count": span_count,
        },
        "clips": resolved,
        "timeline": {
            "total_span_len": max_end_span_idx,
            "total_sec": total_sec,
        },
    }
