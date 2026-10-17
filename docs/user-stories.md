# User stories

## An agent needs to understand a video

The user asks, “What happened in this video?” The agent automatically applies
the `perceive-video` Skill, inspects the source, selects a local strategy,
creates an Observation, queries relevant timestamps, and reports both findings
and provider provenance. The user does not need to prompt for FFmpeg, scene
detection, sampling density, ASR, or a particular model.

## A user wants to use what is already on the machine

The user runs `video-perception capabilities` and sees deterministic local
capabilities, operating-system integrations, user-managed models, and installed
extensions separately. Video Perception recommends a profile without installing
anything. `doctor` explains which live prerequisites are ready and how missing
capability paths could be added.

## A user connects an existing tool or paid API

The user already has a transcript CLI, a local DeepSeek-VL endpoint, or an API
key for an OpenAI-compatible vision service. The agent uses the
`add-video-provider` Skill to write and validate a local Provider manifest. The
credential remains in the user's environment, the model name is not constrained
by a central list, and the selected Observation records locality, provider, and
model.

## A practitioner teaches the agent a better video method

The user provides an editing tutorial, a sampling recipe, or a set of review
principles. The agent uses `create-perception-pack` to separate executable
evidence collection into a Strategy and interpretation guidance into a
Knowledge Pack. The pack can stay private, live in one project, or be shared
from an independent repository without changing Video Perception core.

## A developer builds a video workflow

An editing, review, retrieval, or generation agent combines general perception
practice from the plugin with its own private knowledge and decision rules. It
stores or indexes the Observation rather than coupling its business profile to
the sensing runtime.

## A user needs semantic detail

The user can select Ollama for local VLM observations or explicitly select Gemini with a personal API key. The Observation records which route ran. Provider failure is visible and never converted to empty “success.”

## An agent watches a live source

The agent passes an FFmpeg-readable OBS, camera, RTSP, or network source to `video-perception stream`. It consumes incremental events and checks the catalog before asking for a capability whose streaming adapter is not implemented.

## A DeepSeek Harness user installs the adapter

Harness gains native Video Perception tools for discovery, diagnosis, observation, and query. The adapter calls the same public CLI and contracts; it does not carry a forked sensing engine or install dependencies behind the user's back.
