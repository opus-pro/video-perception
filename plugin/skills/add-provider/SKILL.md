---
name: add-video-provider
description: "Connect an existing local video tool, hosted model, Ollama-compatible service, or user-owned API to Video Perception without changing the core repository."
---

# Add a Video Perception provider

Use this skill when the user asks to connect a model/API/tool, or when a video
task needs a capability that an already-installed service can provide. Do not
run this skill merely because an optional provider is unavailable.

## Safety and ownership

- Discover before changing anything: run `video-perception extension paths --json`
  and `video-perception extension list --json`.
- Never place credential values in a manifest. Reference an environment
  variable name or an operating-system credential facility.
- Creating declarative files is allowed within the user's requested scope.
  Installing binaries, downloading models, starting containers, or sending
  media remotely requires the user's explicit approval.
- Do not modify the Video Perception core repository for a user-specific
  provider. Put it under the project's `.video-perception/extensions/` or the
  user extension directory reported by `extension paths`.

## Choose the smallest adapter

1. For an OpenAI-compatible vision endpoint, use `runtime.type:
   openai-compatible`. A newly released model is normally only a manifest
   model-name change.
2. For an Ollama-compatible service, use `runtime.type: ollama-compatible`.
3. For another installed CLI or runtime, implement the language-independent
   `video-perception.provider-stdio/v1` command protocol.
4. For a tool the agent should call itself, such as an OCR command or a short
   script against an operating-system framework, use `runtime.type: external`
   and write `usage` that says how to call it. Video Perception never runs it.
5. Declare only the capability cells the provider really implements. A
   provider is never required to complete the full matrix.

Add a `detect` block so `video-perception capabilities` can tell whether the
provider works on this machine and report it in the index. Each check is a
`command` on PATH, a file through `env` or `file_globs`, or a top-level
`python_module`; all are read-only. See "External tool example" in
`docs/extensions.md`.

Start from `../../templates/openai-compatible-provider.yaml` or
`../../templates/command-provider.yaml`. In a repository checkout, the same
examples are under `examples/extensions/`. Use a lowercase namespaced id such
as `user.deepseek-vl` or `company.example.video-model`.

Validate without executing provider code:

```bash
video-perception extension validate <provider.yaml> --json
```

For a command provider, the configured executable receives the argument
`execute` and one `video-perception.provider-request/v1` JSON object on stdin.
It must write one capability-contract JSON value to stdout, diagnostics to
stderr, and exit non-zero on failure. Video Perception invokes it only when a
SensePlan explicitly selects the provider.

After validation, run `video-perception extension list --json`, create a narrow
test SensePlan, and inspect the resulting Observation provenance. Report the
provider locality, media-transfer declaration, model, and any failed attempts.
