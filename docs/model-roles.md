# Model roles

Video Perception routes model work by *role*, not by vendor. A role names what
a model must do and what input it receives. A provider manifest names which
model fills a role on this machine. This page is the normative reference for
the roles, the problems each role is known to have, and the rules an agent
follows when it chooses one.

[Provider contract](provider-contract.md) defines how a provider is declared.
[Extension authoring](extensions.md) shows the manifests. Decision
[0003](decisions/0003-model-roles-not-vendors.md) records why roles exist.

## Why roles

- A capability id such as `transcript` or `scene-description` is stable. The
  model behind it changes every few months.
- The failure modes of a role are stable too. A video-reading model from any
  vendor is slow to feed and imprecise about time. A speech model from any
  vendor reports estimated timing unless it aligns words.
- A rule written against a role survives a model change. A rule written
  against a vendor name does not, and it turns this project into a model
  catalog.

The core therefore ships no vendor default. Built-in routes exist only where a
local tool already implements the role.

## The roles

| Role | Input | Output | Capabilities today | Routes today |
|---|---|---|---|---|
| `measure` | decoded frames and audio | numbers with timestamps | every deterministic cell: `probe`, `scenes`, `stats`, `signalstats`, `track`, `loudnorm`, `beats`, `avsync`, and the rest | built-in Rust/FFmpeg, local Python tools |
| `asr` | the audio track | utterances with timing | `transcript` | `apple-speech` (native host), `whisper-cpp`, command providers |
| `image-vlm` | one frame, a crop, or a contact sheet | text or a structured observation per image | `scene-description`, `gibberish-text`, the host viewing `sample` output | `ollama`, `gemini`, OpenAI-compatible providers, command providers, the host's own vision |
| `video-vlm` | a bounded video segment with its audio | an observation over a time range | reserved id `video-description` (not in the core catalog yet) | command providers only |

`measure` is not a model. It is named so the ordering rule below can refer to
it. Decision is not a role; see [Decision models are out of scope](#decision-models-are-out-of-scope).

A provider may fill one role for one capability. It is never required to fill
every role, and a role never implies a vendor.

## Known problems by role

Treat these as properties of the role. A newer or larger model reduces some of
them; none of them disappears.

### Every model role

- **A model reports; it does not measure.** A duration, resolution, frame
  count, or timestamp stated by a model is an observation. It never replaces
  `probe`, `scenes`, or the decoded PTS in a `sample` manifest.
- **Output is not reproducible.** The same input can produce a different
  answer on the next call. Keep the exact input (frames, segment, prompt) in
  provenance so a claim can be re-checked.
- **A failure must look like a failure.** A provider exits non-zero when
  inference fails. An error object with exit code zero is forbidden, because
  it makes failed evidence look like successful empty evidence.
- **Remote means remote.** Media that reaches a provider with
  `locality: remote` has left the machine. A cloud host viewing a local frame
  is remote image inference even though the CLI made no network call.

### `asr`

- **Timing precision varies by provider.** `timing: aligned` means word-level
  alignment; `estimated` means utterance boundaries were inferred; `unknown`
  means nothing can be assumed. Only `aligned` timing supports exact quotation
  or cutting. Everything else is coarse navigation.
- **Language detection fails on short or mixed-language audio.** Pass the
  language when it is known.
- **No speaker diarization is part of the contract.** A transcript says what
  was said and when, not who said it.
- **Music, noise, and silence produce hallucinated text.** Repeated phrases
  and invented sentences are characteristic. Use `audio-class` and
  `scene-audio` to know what the audio contains before trusting a transcript.
- **Long audio is chunked.** Words at chunk boundaries are dropped or
  duplicated. Treat a sentence that spans a boundary as uncertain.

### `image-vlm`

- **A frame has no time context.** It cannot show motion, order, or cause.
  Give the model the timestamp and neighbors; use `sample --parent` windows
  for before-and-after context.
- **Quality is bounded by frame selection.** The model sees only what was
  sampled. Sparse sampling cannot establish absence of a brief event, and a
  motion peak is a place to look, not proof of importance.
- **Downscaling loses small text and detail.** A contact sheet is cheap and
  lossy. Reading a label needs a larger single frame or a bounded crop.
- **Cost is linear in frames.** Image token cost differs by provider and by
  resolution. A sheet is one image; a dense window is many.
- **Small local models read text, count objects, and judge spatial relations
  poorly.** Prefer them for coarse description; verify counts and text with
  a larger frame or a stronger route.
- **Faces, text, and screens are sensitive.** A remote `image-vlm` route
  needs the same explicit consent as any other media transfer.

### `video-vlm`

- **The segment leaves the machine.** Every known `video-vlm` route today is
  remote; manifests must declare `media_transfer: required`. Upload size and
  duration caps are provider-specific and change without notice.
- **Cost scales with duration, not with the question.** Billing is per
  second, per minute, or per sampled frame. A ten-second question asked about
  a ten-minute segment costs the ten minutes.
- **The provider resamples silently.** Context limits force the provider to
  decode at a low frame rate or drop frames. The model sees fewer frames than
  the segment contains, and reports nothing about which ones.
- **Time is imprecise.** Durations, absolute timestamps, and "at 1:23"-style
  references from a video model are frequently wrong: rounded, drifted, or
  anchored to the provider's internal sampling. Treat them as hints and verify
  with `sample` before citing.
- **The tail of a long segment is less reliable than the head.** Truncation
  and attention limits degrade late content first. Prefer short segments.
- **Latency is seconds to minutes.** This role never serves `streaming` mode.
- **There is no portable video input standard.** OpenAI-compatible chat
  endpoints do not define video content parts; each vendor's video API differs.
  This is why the core binds no vendor and why `video-vlm` is served by
  command providers.

### Local models, any role

- RAM, VRAM, and disk requirements are the user's resources. `capabilities`
  reports fit; `doctor` reports readiness; Video Perception installs nothing.
- Cold start can exceed the inference time for a short input.
- Quality tiers differ sharply between small and large local models. Record
  the exact model id in provenance so results can be compared.

## Selection rules

These rules are normative for the bundled Skills and strategies. A knowledge
pack may add rules; it may not remove these.

1. **Measure before you model.** Run deterministic evidence first. Use scene
   structure, motion, signal change, and audio activity to find the intervals
   worth a model call.
2. **Choose the narrowest role that can answer.** Speech questions go to
   `asr`. A located visual question goes to `image-vlm` on a bounded window.
   `video-vlm` is for change over time that a few frames cannot settle: what
   happens between two cuts, pacing, an action sequence.
3. **Local before remote; explicit before automatic.** `auto` never selects a
   remote route or executes third-party provider code. A remote route runs
   only when a SensePlan or the user names it.
4. **Models report, measurements decide.** Any time, duration, count, or
   dimension from a model is checked against `probe`, `scenes`, or `sample`
   PTS before it is cited.
5. **Bound every call.** Give a time window, a frame budget, or a segment
   length. Never send the whole asset by default.
6. **Escalate on a named gap.** Move to a stronger or wider role only when the
   current evidence names what is missing. Repeating a failed operation at a
   larger scale is not escalation.
7. **Record the route.** Every model-backed output carries role, provider id,
   resolved model, locality, and timing where relevant. The provenance rules
   in the [provider contract](provider-contract.md#output-and-provenance)
   apply to all roles.

## Decision models are out of scope

Video Perception does not route a decision, judgment, or planning model. The
consuming agent owns interpretation, business rules, thresholds, and the final
answer, as the [boundary](../README.md#boundary) states. Adding a decision
role here would pull consumer knowledge into the sensing layer.

What a decision model should receive from this project is evidence, not media:

- the Observation `manifest.json`, with each capability's status and error;
- measurements with timestamps;
- model observations labelled with their provenance;
- the remaining uncertainty, stated as what was not sampled or not answered.

Raw frames and segments go to a decision model only when the agent decides to
use that model in an `image-vlm` or `video-vlm` role, under the rules above.

## What a provider declares per role

A provider manifest declares capability cells, `locality`, `media_transfer`,
`modes`, and credential environment-variable names. Per role, its output must
also carry:

| Role | Required in output or provenance |
|---|---|
| `asr` | `timing` as `aligned`, `estimated`, or `unknown`; language used |
| `image-vlm` | the frame ids or timestamps the model actually received; resolution sent when downscaled |
| `video-vlm` | segment start and end sent; provider-side sampling if the API reports it; the model id |

A manifest that cannot meet the row for its role should implement a different
capability rather than omit the fields.

## Status of `video-vlm`

The role is defined; the catalog entry is not. The reserved capability id is
`video-description` with contract `video-perception.video-description/v1`:
input is one bounded segment `[start, end]` of a local asset; output is a list
of observations with time ranges relative to the asset timeline, plus a
`sampling` provenance block. Until the core registry lists the id,
`catalog list` does not show it and a SensePlan cannot request it. A command
provider written against this page can be validated with
`extension validate` today. Registering the id and adding a built-in request
shape is tracked as follow-up work; it does not add a vendor route.
