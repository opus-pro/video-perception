# video-perception-vlm

Model-backed capability runner for Video Perception. It executes registered, consumer-agnostic prompt packs and records backend/model provenance.

```bash
video-perception-vlm --preset scene-description \
  --scenes observation/signals/scenes.json \
  --keyframe-dir observation/assets/keyframes

video-perception-vlm --preset gibberish-text,scene-description \
  --scenes observation/signals/scenes.json \
  --keyframe-dir observation/assets/keyframes
```

List registered prompt packs with `video-perception-vlm --list-presets`.

## Model backends

Ollama is the local default:

```bash
VIDEO_PERCEPTION_VLM_BACKEND=ollama OLLAMA_MODEL=gemma4 video-perception-vlm ...
```

Gemini is an explicit remote route owned by the user:

```bash
VIDEO_PERCEPTION_VLM_BACKEND=gemini \
VIDEO_PERCEPTION_VLM_MODEL=gemini-2.5-flash \
GEMINI_API_KEY=your_key \
video-perception-vlm ...
```

The CLI exits non-zero when a provider fails, a keyframe is missing, or any scene result contains an error. This prevents a failed model call from appearing as a successful Video Perception capability.

## Tests

```bash
python -m pytest tests -v
```

Tests mock inference; no network or model is required.
