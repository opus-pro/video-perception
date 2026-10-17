# Video Perception

**Give any agent a practiced, local-first way to understand video.**

Preview the whole. Revisit what matters. Answer with evidence.
The host agent plans and interprets; local tools gather signals and images;
the host's vision or a user-selected visual provider supplies visual meaning.
This is a reusable observation method, not a promise of another model's
intelligence or a requirement to use a hosted gateway.

Video Perception discovers the video capabilities already available on a
computer, applies reusable perception practices, and returns structured,
timestamped evidence an agent can query. The user does not need to know that a
good answer may require metadata inspection, scene-aware sampling, motion and
audio signals, transcript, OCR, or selective semantic vision.

It is a protocol-first agent plugin backed by a small CLI runtime. The plugin
teaches agents how to perceive video; the runtime makes capability discovery,
provider selection, execution, provenance, and Observation storage consistent
across Claude, Codex, DeepSeek Harness, and other shell-capable agents.

## What changes after installation

Without Video Perception, an agent commonly invents a one-off FFmpeg command,
samples a few uniform frames, and asks a vision model to guess the rest. With
Video Perception, the agent can:

1. inspect the source before choosing a sampling method;
2. use deterministic local evidence before spending model tokens;
3. find information-dense intervals using scenes, motion, signal change, and audio;
4. add local ASR or semantic vision only when the task needs it;
5. use an explicitly selected user-owned API when local evidence is insufficient;
6. return timestamps and exact provider/model provenance with the answer.

Video Perception does not install models, hide provider failures, or require a
central model registry. A new model behind an existing OpenAI-compatible,
Ollama-compatible, or command protocol is normally a local manifest change—not
a change to this repository.

## Install

Each path installs the same `video-perception` executable for macOS (Apple
Silicon or Intel) or Linux x86_64. FFmpeg and ffprobe must be on `PATH`;
ImageMagick enables contact sheets.

```bash
# Release binary, verified against the release checksums
curl -fsSL https://raw.githubusercontent.com/opus-pro/video-perception/main/install.sh | sh

# Homebrew
brew install opus-pro/tap/video-perception

# npm; or run without installing: npx video-perception mcp
npm install -g video-perception
```

Keep an installed release current with `video-perception update`. Other
platforms build from source as in the quick start below.

## Quick start

Prerequisites: Rust, FFmpeg/ffprobe, and ImageMagick.

```bash
git clone https://github.com/opus-pro/video-perception.git
cd video-perception
cargo install --path cli

video-perception capabilities
video-perception doctor
video-perception catalog list --json
video-perception extension list --json
```

Run a deterministic local scan:

```bash
video-perception observe video.mp4 \
  --capabilities probe,scenes,stats,layout,track,framerate,signalstats,avsync \
  --out observation

video-perception observation query observation
video-perception observation query observation --at 00:00:12.500
video-perception observation query observation --text "title card"
```

The agent plugin also includes source-aware strategies:

```bash
video-perception observe video.mp4 \
  --strategy plugin/strategies/quick-scan.yaml \
  --out observation
```

Watch a file, camera, OBS virtual camera, RTSP feed, or another FFmpeg-readable
source:

```bash
video-perception stream input.mp4 \
  --capabilities keyframes,signalstats \
  --out stream-session
```

File inputs replay in real time by default. Use `--speed 0` for unpaced
evaluation. The catalog reports which capabilities genuinely support streaming.

## A bounded second look

The source build includes `sample` for agent-driven visual investigation:

```bash
# Choose the window after probing the real input duration.
video-perception sample video.mp4 --end 60 --fps 0.2 --sheet --out evidence/overview
video-perception sample video.mp4 --start 12 --end 14 --fps 8 --sheet \
  --parent evidence/overview/sample.json --out evidence/detail
```

Each round produces timestamped JPEGs and `sample.json`, plus an optional
labelled contact sheet. The agent must open images before citing visual
observations. It can instead use an explicitly selected visual provider when
the host lacks vision. No models or dependencies are installed by `sample`.
Local extraction does not make a cloud agent's subsequent image viewing offline.

See [the observation loop](docs/observation-loop.md) for limits and citations.

## The extension model

Video Perception has three decentralized extension contracts:

| Extension | Purpose | Typical author |
|---|---|---|
| Provider | Implements one or more capability cells using a local tool, native API, hosted model, or user-owned service | Tool/model integrator |
| Strategy | Chooses and configures capabilities for gathering video evidence | Video practitioner |
| Knowledge pack | Teaches an agent how to interpret evidence for a source type or workflow | User, team, or community |

Extensions can live in:

- paths listed in `VIDEO_PERCEPTION_EXTENSION_PATH`;
- the user directory reported by `video-perception extension paths`;
- `.video-perception/extensions/` in the current project.

They do not need to be merged into this repository. Validate one manifest or a
complete pack without executing provider code:

```bash
video-perception extension validate examples/extensions/openai-compatible-provider.yaml --json
video-perception extension validate plugin/packs/foundation --json
```

See [extension authoring](docs/extensions.md) and the example manifests under
[`examples/extensions/`](examples/extensions/).

## Capability layers

| Layer | Examples | Default | Media locality |
|---|---|---|---|
| Built-in deterministic | probe, scenes, stats, layout, track, framerate, flicker, signalstats, avsync | Yes | Local |
| System native | Apple Speech through a separately installed provider host | Optional | Local |
| User-managed model | whisper.cpp, Ollama, or a discovered local Provider | Optional | Local |
| User-configured remote | Gemini or an OpenAI-compatible BYOK Provider | Explicit only | Remote |

`capabilities` explains device fit; `doctor` verifies installed tools and live
services; `setup --profile basic|mac-native|local-model` prints a read-only
upgrade plan. Credentials are referenced by environment-variable name and are
never read or printed during discovery.

Model-backed capabilities are filled by role, not by vendor.
[Model roles](docs/model-roles.md) defines `asr`, `image-vlm`, and
`video-vlm`, the problems each role is known to have, and the order in which an
agent should use them. The core ships no vendor default.

## Agent plugin

The plugin ships three Skills:

- `perceive-video` applies the bundled foundation playbook automatically to
  normal requests such as “tell me what happens in this video”;
- `add-video-provider` teaches an agent to connect an existing local tool,
  model, or user-owned API without changing core;
- `create-perception-pack` turns reusable techniques, tutorials, or user
  knowledge into a strategy plus knowledge pack.

- Agent plugin for Claude Code, Codex, ChatGPT, and any Agent Plugins 1.0
  client: [`plugin/`](plugin/), with install steps in its README. The
  repository doubles as a Claude Code and Codex marketplace.
- DeepSeek Harness adapter: [`integrations/deepseek-harness/`](integrations/deepseek-harness/)

The adapters contain no second video-processing engine. They delegate to the
same `video-perception` CLI and versioned contracts.

## MCP server

Hosts that give an agent tools but no shell can run the same surface over MCP:

```bash
video-perception mcp
```

The server speaks protocol revision 2026-07-28 on stdio and still answers the
`initialize` handshake of older clients. Its tools mirror the CLI: discover
with `capabilities`, `doctor`, `catalog_list`, and `extension_list`; gather
evidence with `observe` and `sample`; read it with `query` and
`observation_diff`. Observation and sample directories are the handles that
tools return and accept, so the CLI and the MCP server read the same evidence.
The plugin's `.mcp.json` starts it for hosts that load MCP servers from a
plugin. See the [CLI reference](docs/cli-reference.md#serve-as-an-mcp-server).

## Boundary

Video Perception owns reusable video-perception practice, capability execution,
dependency resolution, evidence contracts, provenance, storage, and provider
selection. Model runtimes own model downloads, credentials, serving, and
hardware. The consuming agent owns the user's intent, private/company
knowledge, business rules, actions, and final answer.

A remote route exists only as a provider manifest the user installs. This
repository ships none.

See [architecture](ARCHITECTURE.md), [CLI reference](docs/cli-reference.md),
[provider contract](docs/provider-contract.md), and
[contribution guide](CONTRIBUTING.md).

## License

[MIT](LICENSE)
