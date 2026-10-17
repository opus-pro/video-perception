# Agent-driven observation loop

Status: source-build feature; not a claim about an already-published binary.

The plugin teaches a host agent to discover tools, plan an initial pass, gather
local evidence, perceive selected images/audio, investigate uncertainty, and
summarize with citations. `sample` is the visual evidence primitive. It does
not call an LLM, run a planner, automatically choose a provider, or install tools.
Existing `observe` and `stream` contracts are unchanged.

## Who uses it

- A user asks an agent what a video says: overview plus relevant ASR, then
  targeted visual checks where the summary needs them.
- An editor asks about a brief glitch: coarse context, then dense frames around
  a candidate transition, with explicit uncertainty about unsampled moments.
- An agent verifies its own edit: inspect the rendered output before claiming
  success. Sensor gathers evidence; rendering and final acceptance remain upstream.

## Commands

```bash
video-perception capabilities --json
video-perception doctor --json
video-perception observe video.mp4 --capabilities probe,scenes --out structure

# Select a window appropriate to the probed asset. These rates are examples.
video-perception sample video.mp4 --end 60 --fps 0.2 --sheet --out evidence/overview
video-perception sample video.mp4 --start 12 --end 14 --fps 8 \
  --max-frames 24 --max-size 1280 --sheet \
  --parent evidence/overview/sample.json --out evidence/detail

# Crop coordinates refer to display-rotated pixels before scaling.
video-perception sample video.mp4 --start 12 --end 12.1 --fps 1 \
  --crop 10,10,200,100 --max-size 960 \
  --parent evidence/detail/sample.json --out evidence/text
```

`sample` accepts regular local video files, not network sources or playlists.
FFmpeg is restricted to local file protocol and supported video containers.
Outputs are new directories; an existing directory is rejected, never replaced.
Use the same unchanged canonical source for linked rounds. Identity checks use
path, size, and modification time, not a cryptographic content identity.

## Evidence contract

`video-perception.sample/v1` records the source, requested window/rate/size/crop,
local tool provenance, selected frames, parent manifest, cumulative frame count,
warnings, and wall time. Each frame has a round-local ID, relative JPEG path,
and decoded PTS in seconds, normalized so the first decoded video frame is zero.
It does not calculate timestamps from frame number divided by nominal FPS.
The `[start, end)` window is on this normalized video timeline. Align separately
before joining audio or Observation tracks with another time origin.

The labelled sheet follows manifest order. Labels link back to individual
frames for higher-resolution inspection. Cite both the sample path and frame
ID (e.g. `evidence/detail/sample.json`, `r2:f3`, `12.250s`). IDs alone are not
globally unique. `sampled` means images were extracted, not that an LLM viewed
them or the entire requested interval was verified. `partial` flags decoder
damage; an empty window or failed decode returns nonzero without a manifest.
Logs and partial output remain for diagnosis. A sheet failure does not erase
otherwise usable frames.

## Budgets and limitations

- Defaults: 48 frames per round, 240 cumulative selected frames, 960-pixel
  maximum edge, 60 seconds processing time. At most 20 linked rounds.
- Requested duration × FPS must fit the per-round and cumulative budgets before
  decoding. A child can lower but cannot raise its parent's total budget.
- Sampling chooses existing source frames without duplication. It may return
  fewer frames than requested, especially for low/VFR frame rates or short input.
- Decoding currently starts at the beginning to preserve a single PTS origin.
  Late windows in long videos can hit the time budget; this is not optimized seeking.
- Frame budgets do not measure image tokens, bytes, model cost, or model viewing
  history. These remain caller-owned. Do not equate extracted frames with VLM calls.
- Resizing a whole sheet can hide small text; open individual frames or crop.
- No automatic OCR, ASR, semantic provider invocation, quality guarantee, or
  autonomous retry is added by `sample`. Use existing capabilities as needed.

## Verification and next evaluation

Generated-fixture integration tests cover VFR timestamps, linked rounds,
budgets, immutable output, invalid requests, empty windows, and blocked remote
playlists. These establish tool behavior, not model intelligence.

Evaluate the same host model with and without the plugin on intact, licensed
videos. Keep tasks, model settings, and budgets fixed. Measure event recall,
timestamp error, supported vs unsupported claims, frames actually viewed,
wall time, and model usage. Repeat runs before making comparative claims.
