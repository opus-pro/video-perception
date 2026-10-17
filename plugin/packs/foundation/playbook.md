# Foundation video perception playbook

Use this playbook when the user asks a substantive question about a video. The
goal is not to run every capability. It is to gather enough timestamped evidence
to answer reliably while preserving privacy, cost, and provenance.

## Begin with the asset, not a guessed extraction command

Inspect duration, resolution, frame rate, audio tracks, and scene structure
before choosing a sampling rate. Long, static, speech-heavy, and fast-moving
videos need different evidence plans.

## Prefer coarse-to-fine perception

1. Run a deterministic structural scan.
2. Use scene boundaries, motion, signal change, and audio activity to locate
   information-dense intervals.
3. Add transcript when spoken meaning matters.
4. Add semantic vision only for questions that deterministic evidence and
   transcript cannot answer.
5. Revisit a narrow timestamp window when confidence is low instead of sending
   the entire video to a larger model.

## Match the technique to the source

- Talking head or lecture: transcript first; sample slides and visible changes.
- Screen recording: prioritize UI transitions and text changes, including low-motion frames.
- Sports or action: increase temporal density around motion peaks and scene changes.
- Vlog or edited narrative: preserve scene order and context on both sides of a cut.
- Silent visual material: rely on scene, motion, layout, and semantic vision rather than ASR.

## Preserve evidence quality

- Keep timestamps attached to every claim.
- Distinguish aligned timestamps from estimates.
- Read the Observation manifest before trusting optional outputs.
- Report which provider and model produced semantic evidence.
- Never describe a missing provider result as successful empty evidence.

## Close the observation loop

Before a second pass, name the missing evidence: a sentence from the audio,
an unreadable label, an uncertain object, or a short transition. Change only
the dimensions needed to investigate it:

- A broad summary benefits from a sparse overview plus relevant speech.
- Small text needs larger individual frames or a bounded crop, not merely a
  denser low-resolution sheet.
- A brief visual event needs more temporal samples in a narrow interval,
  retaining context before and after the event.
- Motion or scene-change scores suggest places to inspect; they do not prove
  semantic importance. Static slides and UI states may still matter.

Use `sample --parent` for linked rounds and read their manifests. Stop when
the question has enough cited evidence, the budget is exhausted, or another
pass would repeat the same failed operation. Report remaining uncertainty.
For OCR/ASR or another optional provider, test a small representative window
before scanning a long video; repeated errors or garbage are a reason to
stop or use an already-authorized alternative, not a reason to scale up.

Keep a short caller-owned evidence note with the question, sample paths,
frames actually viewed, relevant tool/provider outputs, and unresolved gaps.
This is an observable action log, not hidden reasoning. Separate measurements,
model observations, and final interpretation. Quote the actual frame PTS,
not a nominal frame-rate calculation. A contact sheet's blank padding is not
evidence of black video frames. Sparse sampling cannot establish absence of
brief events, and a truncated input cannot support claims about its missing
tail. Reopen a larger frame before relying on details lost in a montage.

## Choose a model role, not a vendor

Model-backed evidence comes from three roles: `asr` reads the audio track,
`image-vlm` reads one frame, crop, or sheet, and `video-vlm` reads a bounded
segment. Pick the narrowest role that can answer the question, and expect the
problems that belong to the role rather than to a brand:

- `asr`: only `timing: aligned` supports exact quotation or cutting; music,
  noise, and silence produce invented text, so check `audio-class` first.
- `image-vlm`: a frame has no time context and shows only what was sampled;
  small text needs a larger frame, not a denser sheet.
- `video-vlm`: the segment leaves the machine, cost follows duration, the
  provider resamples silently, and its timestamps and durations are hints to
  verify with `sample`, not evidence.

Any time, duration, count, or dimension stated by a model is checked against
`probe`, `scenes`, or decoded PTS before it is cited. Bound every model call
with a window, a frame budget, or a segment length. Judgment and final answers
are the calling agent's work; no role decides.

## Respect the user's resources

- Prefer capabilities already available on the machine.
- Do not install a model or executable without approval.
- Do not send media to a remote provider without an explicit remote route.
- Escalate model quality only when the task benefits from it.
