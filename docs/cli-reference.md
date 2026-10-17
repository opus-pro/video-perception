# CLI reference

## Discover and plan

```bash
video-perception capabilities [--json]
video-perception doctor [--json]
video-perception setup --profile basic|mac-native|local-model [--json]
video-perception catalog list [--json]
video-perception catalog search QUERY [--json]
video-perception extension paths [--json]
video-perception extension list [--json]
video-perception extension validate PATH [--json]
```

`capabilities` is static and read-only. `doctor` checks installed executables and live local services. `setup` only prints an advisory plan; it never downloads a model or edits the shell.

## Serve as an MCP server

```bash
video-perception mcp
```

Speaks MCP on stdin and stdout for hosts that give an agent tools but no shell.
The server implements protocol revision 2026-07-28 and still answers the
`initialize` handshake of older clients. Tools: `capabilities`, `doctor`,
`catalog_list`, `extension_list`, `observe`, `query`, `sample`, and
`observation_diff`. Each tool runs this executable as a child process with the
same arguments the CLI documents, so output contracts and provider rules do
not fork.

`observe` and `sample` return a directory path; pass it to `query`,
`observation_diff`, or as `sample`'s `parent`. When no `out` is given, the
directory is created under `$VIDEO_PERCEPTION_HOME/evidence` or the platform
data directory. `sample` can attach the contact sheet or individual frames as
image content for hosts that cannot read local files. The server makes no
network calls of its own; a remote provider runs only when a SensePlan or
strategy names it. See [decision 0004](decisions/0004-mcp-server-as-thin-cli-adapter.md).

## Sample a bounded visual window (source build)

```bash
video-perception sample VIDEO --start 0 --end 60 --fps 0.2 \
  --max-frames 48 --max-size 960 --sheet --out evidence/overview
video-perception sample VIDEO --start 12 --end 14 --fps 8 \
  --parent evidence/overview/sample.json --out evidence/detail
```

Local regular videos only. Optional `--crop x,y,width,height` is applied before
scaling. New output directories contain JPEGs and `sample.json`; `--sheet`
requests an optional ImageMagick montage. Default cumulative frame budget is
240 and processing timeout is 60 seconds. Selected frames carry normalized
decoded video timestamps, not nominal frame-rate estimates. This command does
not invoke a model. See [observation-loop.md](observation-loop.md).

## Observe a bounded video

```bash
video-perception observe VIDEO \
  --capabilities probe,scenes,stats \
  --out observation

video-perception observe VIDEO --plan sense-plan.json --out observation
video-perception observe VIDEO --strategy strategy.yaml --out observation
```

Exactly one of `--capabilities`, `--plan`, and `--strategy` is required. A
strategy may be a manifest path or a discovered namespaced id. Technical
dependencies are added automatically and recorded in `execution.json`.
Existing output directories are not overwritten.

A minimal SensePlan:

```json
{
  "contract": "video-perception.sense-plan/v1",
  "capabilities": [
    { "id": "probe", "required": true },
    { "id": "scenes", "required": true },
    { "id": "scene-description", "required": false }
  ],
  "model_backend": {
    "provider": "ollama",
    "model": "gemma4"
  }
}
```

Built-in model backends are `ollama` and `gemini`. A discovered
OpenAI-compatible or Ollama-compatible Provider id is also accepted, and its
model is an open string. Transcript providers are `auto`, `apple-speech`,
`whisper-cpp`, and explicitly selected command providers; public defaults are
local-only.

## Extensions

`extension paths`, `list`, and `validate` are read-only. They never run a
provider, read credential values, download a model, or make a network request.
Extensions are discovered from explicit, user, and project roots, so they do
not require a core repository contribution.

A selected command provider receives one
`video-perception.provider-request/v1` JSON value on stdin. See
[extension authoring](extensions.md).

## Query and compare

```bash
video-perception observation query observation
video-perception observation query observation --at 12.5
video-perception observation query observation --text "slide title"
video-perception compare observation-a observation-b
```

The query command returns machine-readable evidence from an existing Observation; it does not call a model.

## Stream

```bash
video-perception stream SOURCE \
  --capabilities keyframes,signalstats \
  --out stream-session
```

`SOURCE` can be a file or any FFmpeg-readable live input. File inputs replay at real time; `--speed 0` disables pacing for evaluation. Events are emitted as JSONL while the session manifest and event log are written under `--out`.

Run `video-perception catalog list --json` before choosing stream capabilities. A capability is accepted only when its streaming adapter reports `implemented: true`.

## Exit behavior

- required capability failure: non-zero exit;
- optional capability failure: Observation remains valid and records the error;
- provider response containing scene errors: capability failure, never false success;
- invalid contract, unknown capability, or existing output path: non-zero exit before execution.
