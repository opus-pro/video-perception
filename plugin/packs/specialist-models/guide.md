# Specialist vision models

Video Perception's built-in capabilities measure signals, transcribe speech,
and sample frames for you to look at. Some questions need a trained model that
looks at every frame for one specific thing. This guide says which open model
fits which question, how to run it, and what to watch for. Video Perception
does not install or run these models; you do, with the user's approval.

Every entry below is written from the upstream project's own documentation.
None has been tested by this project yet. Check results against frames before
you rely on them.

## Rules for every model

- Ask the user before you install a package, pull a container, or download
  weights. Say the download size and whether a GPU is needed.
- Run the model on this machine. Do not send frames or audio to a hosted copy
  of the model unless the user chooses that.
- Treat model output as candidate evidence. Open the frames at the times it
  reports before you count, cut, or quote anything.
- Record what produced the evidence: model name, weights version, threshold,
  and the source times in seconds. Keep the output next to the Observation.
- Faces: detect and track faces only. Never try to identify who a person is.
- Licenses differ. Check the license column before using a model for
  commercial work.

## Which model for which question

| Question | Model | Why the built-in capabilities are not enough |
|---|---|---|
| Where are the shot boundaries in camera footage with fades and dissolves? | TransNetV2 | `scenes` uses frame-difference scores, and a slow fade or dissolve can stay below its thresholds. |
| Isolate one subject so I can track, crop, or measure it | SAM 2 | No built-in capability returns pixel masks. |
| In which frames does "a red car" or "the product box" appear? | Grounding DINO | Detection from a free-text description; no fixed class list. |
| Count or locate common objects (people, cars, phones) | YOLOX, or Ultralytics YOLO | Fast detection over many frames for a fixed set of classes. |
| Where are faces, and are they cropped by the frame edge? | A face detector | Layout and tracking signals do not know what a face is. |
| Who is speaking on screen at this moment? | TalkNet-ASD | Matches lip movement to the audio; a transcript says what was said, not who said it on screen. |

Prefer the cheapest route that answers. A transcript search or a few sampled
frames often settles a question without any of these models.

## TransNetV2: shot boundaries

- **Does:** scores every frame for the probability that a shot transition
  happens there. Handles hard cuts and gradual transitions.
- **Input:** all frames of the video, resized to 48x27 RGB. **Output:** a
  per-frame probability; frames above a threshold (0.5 by default) mark
  transitions.
- **Run:** the official repository (github.com/soCzech/TransNetV2) ships a
  TensorFlow model and a PyTorch reimplementation. The PyTorch weights must be
  converted from the TensorFlow weights with the repository's script, which
  needs an old TensorFlow release. Community repackagings with converted
  weights exist; this project has not reviewed them.
- **Hardware:** CPU is workable for short videos; a GPU is much faster.
- **License:** MIT.
- **Use with Video Perception:** run it on camera footage with gradual
  transitions. Compare its boundaries with `scenes` and look at frames where
  the two disagree. It works on frames reduced to 48x27, so it is not a fix for
  slide decks where only small text changes; for those, look at the frames
  around `weak_changes`.

## SAM 2: segmentation

- **Does:** returns a pixel mask for an object you point at (a point or a box
  prompt) and can propagate that mask through a video.
- **Run:** github.com/facebookresearch/sam2. Several checkpoint sizes exist;
  start with a small one.
- **Hardware:** a GPU is strongly recommended for video propagation.
- **License:** Apache 2.0 for the code and the model checkpoints.
- **Use with Video Perception:** take the subject's position from a sampled
  frame or from a detector box, then segment. Report the frame range where the
  mask is valid.

## Grounding DINO: open-vocabulary detection

- **Does:** finds boxes for objects described in free text ("a person holding
  a microphone").
- **Run:** github.com/IDEA-Research/GroundingDINO, or the
  `IDEA-Research/grounding-dino-base` checkpoint through the Hugging Face
  `transformers` zero-shot object detection API.
- **Hardware:** GPU preferred; CPU works for a handful of frames.
- **License:** Apache 2.0 for the code and the base checkpoint.
- **Use with Video Perception:** run it on frames from `sample`, not on every
  frame. Narrow the time range first with scenes or the transcript.

## Object detection: YOLOX or Ultralytics YOLO

- **Does:** detects objects from a fixed class list (for example the 80 COCO
  classes) quickly enough to run on many frames.
- **Run:** YOLOX from github.com/Megvii-BaseDetection/YOLOX, or the
  `ultralytics` package.
- **License:** YOLOX is Apache 2.0. Ultralytics is AGPL 3.0: using it inside a
  product you distribute or offer as a service brings AGPL obligations unless
  you hold an Ultralytics commercial license. Prefer YOLOX when that matters.

## Face detection

- **Does:** finds face boxes and landmarks per frame. Use it to check framing
  (faces cut off at the edge, faces too small) or to feed active speaker
  detection.
- **Run:** RetinaFace through the InsightFace library is a common choice.
- **License:** InsightFace code is MIT, but its pretrained models, including
  the ones its Python package downloads automatically, are for non-commercial
  research only. For commercial work, choose a detector whose weights allow
  it.
- **Use with Video Perception:** detection only. Do not run face recognition or
  try to name people.

## TalkNet-ASD: active speaker detection

- **Does:** for each tracked face, scores whether that person is speaking,
  using lip movement and the audio together.
- **Run:** github.com/TaoRuijie/TalkNet-ASD. It needs face tracks first, so it
  runs after a face detector.
- **Hardware:** GPU preferred.
- **License:** the code is MIT. The pretrained weights were trained on research
  datasets and their terms are not stated; treat them as research-only until
  confirmed. The face detector you pair with it brings its own license.
- **Use with Video Perception:** align its speaking segments with the
  transcript timestamps to say who said a line on screen.

## Keep what you set up

When one of these models works on this machine, offer to record it as an
`external` provider manifest so later sessions find it in the capability index
instead of searching. A `python_module` check works for models installed as
Python packages (for example `sam2`, `ultralytics`, or `insightface`). For a
model that runs only in a container, add a `command: docker` check so the
index at least knows a container runtime is present, and say in `usage` how to
start the container. A provider without a `detect` block is listed as
`unchecked`. See `../../skills/add-provider/SKILL.md`.
