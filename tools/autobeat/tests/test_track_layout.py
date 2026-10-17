from __future__ import annotations

import pytest

from autobeat.errors import TrackLayoutError
from autobeat.track_layout import resolve_layout


def _grid() -> dict:
    return {
        "grid": {
            "anchors_sec": [0.0, 0.5, 1.0, 1.5, 2.0, 2.5],
        }
    }


def test_resolve_magnetic_layout() -> None:
    track = {
        "track_mode": "magnetic",
        "clips": [
            {"id": "c1", "span_len": 2},
            {"id": "c2", "span_len": 1},
            {"id": "c3", "span_len": 2},
        ],
    }
    resolved = resolve_layout(_grid(), track)
    assert resolved["track_mode"] == "magnetic"
    assert [c["start_span_idx"] for c in resolved["clips"]] == [0, 2, 3]
    assert [c["end_span_idx"] for c in resolved["clips"]] == [2, 3, 5]
    assert resolved["timeline"]["total_span_len"] == 5
    assert resolved["timeline"]["total_sec"] == pytest.approx(2.5)


def test_resolve_absolute_layout_with_gaps() -> None:
    track = {
        "track_mode": "absolute",
        "clips": [
            {"id": "c1", "span_len": 1, "start_span_idx": 0},
            {"id": "c2", "span_len": 2, "start_span_idx": 3},
        ],
    }
    resolved = resolve_layout(_grid(), track)
    assert [c["start_span_idx"] for c in resolved["clips"]] == [0, 3]
    assert [c["end_span_idx"] for c in resolved["clips"]] == [1, 5]
    assert resolved["timeline"]["total_sec"] == pytest.approx(2.5)


def test_resolve_absolute_overlap_rejected_by_default() -> None:
    track = {
        "track_mode": "absolute",
        "clips": [
            {"id": "c1", "span_len": 2, "start_span_idx": 1},
            {"id": "c2", "span_len": 2, "start_span_idx": 2},
        ],
    }
    with pytest.raises(TrackLayoutError, match="overlapping clips"):
        resolve_layout(_grid(), track)


def test_resolve_absolute_overlap_allowed() -> None:
    track = {
        "track_mode": "absolute",
        "allow_overlap": True,
        "clips": [
            {"id": "c1", "span_len": 2, "start_span_idx": 1},
            {"id": "c2", "span_len": 2, "start_span_idx": 2},
        ],
    }
    resolved = resolve_layout(_grid(), track)
    assert len(resolved["clips"]) == 2


def test_resolve_rejects_out_of_range_clip() -> None:
    track = {
        "track_mode": "magnetic",
        "clips": [{"id": "c1", "span_len": 99}],
    }
    with pytest.raises(TrackLayoutError, match="exceeds span_count"):
        resolve_layout(_grid(), track)
