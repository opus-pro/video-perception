# Add a Video Perception provider

Use this when the user asks to connect an existing local model, video CLI, or
user-owned API. First call `video_perception_extensions` to inspect decentralized
project and user manifests.

Prefer a data-only `openai-compatible` or `ollama-compatible` manifest when the
service already speaks one of those protocols. Use a command provider with
`video-perception.provider-stdio/v1` only for a genuinely different runtime.
Declare only the capability cells the provider implements.

Start from the bundled `openai-compatible-provider.yaml` or
`command-provider.yaml` resource in this Skill package.

Never put credential values in a manifest; reference environment-variable
names. Never install code, download a model, start a container, or select a
remote-media route without explicit user approval. Validate project manifests
with `video_perception_validate_extension` before use.
