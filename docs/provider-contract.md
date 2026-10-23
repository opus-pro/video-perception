# Provider contract

Providers supply technical implementations for Video Perception capabilities.
A Provider Package may expose one capability or many; it is not required to
fill a vendor-wide capability matrix. Consumers continue to request stable
capability ids rather than model-specific product names.

A provider fills a model role. [Model roles](model-roles.md) defines the
roles, the problems each one is known to have, and the provenance each one
must record.

## Built-in routes

| Provider | Class | Locality | Capabilities |
|---|---|---|---|
| built-in Rust/FFmpeg | built-in local | local | deterministic video and audio evidence |
| `apple-speech` | system native | local | transcript |
| `whisper-cpp` | user-managed local | local | transcript |
| `ollama` | user-managed local | local | semantic visual evidence |
| `gemini` | user-configured remote | remote | semantic visual evidence |

This repository ships no remote route of its own. Any remote route is a
provider manifest the user installs, and an Observation has the same shape
whichever route produced it.

## Decentralized providers

`video-perception.provider/v1` manifests can be installed in any extension root
reported by:

```bash
video-perception extension paths --json
```

Supported runtime declarations are `openai-compatible`, `ollama-compatible`,
`command`, and `native`. Model ids remain open strings. An OpenAI-compatible
Provider can therefore use a newly released model without waiting for a core
release.

The manifest declares:

- a namespaced provider id and semantic version;
- local or remote execution and whether media can leave the machine;
- a runtime transport that contains no credential values;
- the exact capability cells and modes implemented;
- optional model defaults, schemas, and license information;
- an optional `detect` block of read-only checks that decides whether the
  provider can run on this machine;
- for an `external` runtime, `usage` that tells the agent how to call the tool
  itself.

`video-perception capabilities --json` evaluates every `detect` block,
built-in or user-supplied, and reports the result as the machine's index. See
[decision 0005](decisions/0005-capability-contract-not-sop.md).

See [extension authoring](extensions.md) for complete examples and the command
stdio protocol.

## Selection

Transcript selection is controlled by a SensePlan or:

```bash
export VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER=auto
export VIDEO_PERCEPTION_TRANSCRIPT_POLICY=local-only
```

`auto` tries eligible built-in local transcript providers. It does not
implicitly execute third-party provider code.

Semantic selection is controlled by `model_backend` or:

```bash
export VIDEO_PERCEPTION_VLM_BACKEND=ollama
export VIDEO_PERCEPTION_VLM_MODEL=gemma4
```

A custom OpenAI-compatible model backend is selected by its manifest id and an
arbitrary model string. Remote selection is an explicit media-transfer choice.
The user owns the credential environment variable named by the manifest;
discovery and validation do not read or print it.

## Output and provenance

Every provider-backed output must expose or allow the runtime to record:

- provider id and local/remote execution;
- resolved model when applicable;
- aligned, estimated, or unknown timing where relevant;
- attempts and terminal failure.

A provider exits non-zero when inference fails or a required response is
malformed. Returning an error object with exit code zero is forbidden because
it would make failed evidence appear successful.

## Security boundary

- Listing and validating manifests is read-only and never executes provider code.
- Command providers are invoked as argv without a shell.
- A command provider runs only after explicit SensePlan selection.
- Installing binaries, downloading weights, starting a container, and allowing
  remote media transfer remain user-authorized actions.
- Credential values must not appear in manifests, logs, Observations, fixtures,
  or Agent prompts.
