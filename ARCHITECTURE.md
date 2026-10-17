# Video Perception architecture

Video Perception is an agent-facing perception protocol and local runtime. It
is not a model host, central provider catalog, video editor, or general agent
orchestrator.

```text
user video task
       |
       v
Agent Skill + foundation playbook
       |
       v
SensePlan / Strategy -> capability resolver -> provider adapters
                                                |
                                                v
                                   Observation or stream events
                                                |
                                                v
                              optional knowledge pack + Agent
```

## Four extension boundaries

1. **Capability** defines stable evidence semantics and an output schema.
2. **Provider** implements one or more capability cells through a local,
   native, command, or user-owned service runtime.
3. **Strategy** packages technical evidence-collection practice such as a
   quick structural scan or a screen-recording plan.
4. **Knowledge pack** teaches the consuming Agent how to interpret evidence for
   a source type or user workflow.

Providers answer “who produces this evidence?” Strategies answer “how should
the evidence be collected?” Knowledge answers “how should it be interpreted?”
They are separate contracts and are distributed independently.

## Ownership

Video Perception owns:

- general video-perception practice delivered through Agent Skills;
- capability contracts and technical dependency resolution;
- decentralized extension discovery and fail-closed validation;
- local execution and explicit provider selection;
- Observation, SensePlan, setup, and stream contracts;
- timestamp alignment, provenance, failure reporting, and artifact storage;
- query and comparison primitives for agents.

The consuming agent owns:

- the user's intent and final answer;
- private/company knowledge and business rules;
- editing, generation, publishing, and other actions;
- the decision to install code or allow remote media processing.

Model runtimes own credentials, model downloads, GPU allocation, serving, and
model-level retries. Video Perception adapts to them but does not install or
operate them.

## Provider layers

1. **Built-in deterministic:** Rust plus FFmpeg/ImageMagick. This is the default.
2. **System native:** operating-system capabilities exposed through a provider host.
3. **User-managed local:** whisper.cpp, Ollama, and discovered local providers.
4. **User-configured remote:** Gemini or a discovered BYOK provider explicitly selected by a SensePlan.

A Provider Package advertises only the capability cells it really implements.
The capability matrix is derived from manifests; contributors do not maintain a
complete vendor matrix. Model names are not centrally enumerated.

## Extension discovery and trust

The runtime reads manifests from explicit, user, and project paths. Discovery
and validation never execute provider code or read credential values. Duplicate
ids fail closed. Third-party command providers are invoked without a shell and
only when a SensePlan explicitly selects them.

The optional future registry is a discovery and trust layer, not a runtime
dependency. A local directory is a complete, supported distribution mechanism.

## Two execution modes

`observe` processes a bounded asset and writes an immutable
`video-perception.observation/v1` directory. `stream` consumes any
FFmpeg-readable source and appends `video-perception.stream-event/v1` records
plus a durable session manifest.

Streaming support is capability-specific. The catalog reports the intended
strategy and whether the adapter is implemented. Unsupported combinations fail
before media processing begins.

## Storage

An Observation is intentionally boring and agent-readable:

```text
observation/
  manifest.json
  deck.json
  execution.json
  signals/
  assets/
```

It records source metadata, requested capabilities, provider identity,
locality, model, outputs, and failures. It contains evidence, not a consumer's
final judgment.
