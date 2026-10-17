# Extension authoring

Video Perception extensions are decentralized. They may be checked into a
project, installed for one user, or supplied through an explicit search path.
No extension needs to be merged into the core repository before an agent can
use it.

## Discovery

The CLI reads manifests recursively from these roots without executing provider
code:

1. `VIDEO_PERCEPTION_EXTENSION_PATH` entries;
2. `$VIDEO_PERCEPTION_HOME/extensions`, or the platform configuration directory;
3. `.video-perception/extensions/` in the current project.

```bash
video-perception extension paths --json
video-perception extension list --json
video-perception extension validate ./my-extension --json
```

Duplicate ids are errors rather than implicit overrides. Extension ids must be
lowercase and namespaced, for example `user.deepseek-vl` or
`community.adaptive-frames`. The core `video-perception.*` namespace is
reserved for stable contracts shipped by this project.

## Provider manifests

A Provider Package may implement one capability or many. It is not required to
fill the whole capability matrix. A broad API provider can share transport and
authentication across several capability adapters; a focused provider such as
an ASR CLI can expose only `transcript`.

Supported v1 runtime declarations are:

- `openai-compatible`: generic semantic vision through a user-selected model;
- `ollama-compatible`: generic local semantic vision;
- `command`: any executable implementing the stdio protocol;
- `native`: discovery metadata for an operating-system host integration.

Credential fields contain environment-variable names only. Discovery never
reads those variables or makes a network request.

### OpenAI-compatible example

```yaml
contract: video-perception.provider/v1
id: user.my-vision-api
version: 0.1.0
description: My user-owned vision endpoint
locality: remote
media_transfer: required
runtime:
  type: openai-compatible
  base_url: https://example.com/v1
  credential_env: MY_VISION_API_KEY
capabilities:
  - id: scene-description
    contract: video-perception.scene-description/v1
    modes: [video]
license: MIT
```

Select it from a SensePlan using `model_backend.provider`. The model name is an
open string so newly released models do not require a core update:

```json
{
  "contract": "video-perception.sense-plan/v1",
  "capabilities": [{ "id": "scene-description", "required": false }],
  "model_backend": {
    "provider": "user.my-vision-api",
    "model": "my-new-model"
  }
}
```

### Command provider protocol

A command provider declares an argv array and the protocol
`video-perception.provider-stdio/v1`. Video Perception appends `execute`, sends
one request on stdin, and expects one capability-contract JSON value on stdout:

```json
{
  "contract": "video-perception.provider-request/v1",
  "provider": "user.example-transcript",
  "capability": "transcript",
  "mode": "video",
  "request": {
    "video": "/absolute/path/video.mp4",
    "scenes": null,
    "keyframe_dir": null,
    "out_dir": null,
    "options": {
      "id": "transcript",
      "provider": "user.example-transcript",
      "policy": "local-only"
    }
  }
}
```

The provider writes diagnostics to stderr and exits non-zero when execution or
output validation fails. Video Perception invokes command providers only when a
SensePlan explicitly selects them. Merely listing or validating extensions is
always read-only.

## Strategy manifests

A strategy packages reusable technical perception practice. Its `plan` is a
normal `video-perception.sense-plan/v1`; its guidance helps an Agent decide when
the strategy is appropriate. Strategies collect evidence but do not publish,
edit, score, or make a business decision.

```yaml
contract: video-perception.strategy/v1
id: user.quick-lecture-read
version: 0.1.0
description: Read a lecture transcript first and inspect visual changes
plan:
  contract: video-perception.sense-plan/v1
  capabilities:
    - id: scenes
    - id: track
    - id: transcript
      required: false
      provider: auto
      policy: local-only
guidance:
  - Inspect slides around scene changes before escalating semantic vision.
```

Use a strategy by file path or discovered id:

```bash
video-perception observe lecture.mp4 --strategy ./strategy.yaml --out observation
video-perception observe lecture.mp4 --strategy user.quick-lecture-read --out observation
```

## Knowledge packs

Knowledge packs deliver interpretation guidance to an Agent without coupling
private or product-specific rules to the sensing runtime. A pack may reference
a strategy and one or more Markdown files inside its own directory.

```yaml
contract: video-perception.knowledge-pack/v1
id: user.my-editing-practice
version: 0.1.0
description: My reusable interview editing practice
strategy: user.quick-interview-read
knowledge:
  - principles.md
  - examples.md
```

Knowledge paths may not be absolute or escape the pack. Keep tutorial media,
copyrighted documentation, private data, model weights, and datasets out of a
shared pack unless their licenses explicitly permit redistribution.

## Publishing is optional

Local and project extensions are first-class. Authors who want to share an
extension can publish it from any Git repository or package manager. A future
curated index may provide discovery, conformance results, benchmarks, and trust
labels, but it is not required for installation or execution.
