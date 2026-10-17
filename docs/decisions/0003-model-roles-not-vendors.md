# Decision 0003: Route model work by role, not by vendor

- Status: accepted
- Date: 2026-10-18

## Context

Decision 0001 removed the central model list: providers are decentralized and
model ids are open strings. That left a gap in guidance. The Skills and
strategies still had to tell an agent *when* a model call is worth making and
*what* its answer is worth, and the only vocabulary available was capability
ids and provider names.

Capability ids are too fine for that guidance: `scene-description` and
`gibberish-text` share every failure mode of reading a single frame. Provider
names are too specific: a rule about one vendor's video API is wrong the month
a different model is configured. Meanwhile the project wants to open its
contracts to any agent harness without shipping or recommending a vendor.

A third class of model, the one that judges, plans, or decides, kept being
proposed as a Video Perception concern because "the agent needs a model to
interpret the evidence".

## Decision

Define a small set of model roles as a normative reference,
[docs/model-roles.md](../model-roles.md):

- `measure`: deterministic local evidence, listed so ordering rules can name
  it;
- `asr`: audio to timed utterances;
- `image-vlm`: one frame, crop, or sheet to an observation;
- `video-vlm`: a bounded segment to an observation over a time range.

Each role carries its known problems and the provenance a provider must
record. Selection rules are written against roles: measure first, narrowest
role that answers, local before remote, models report and measurements
decide, bound every call, escalate on a named gap, record the route.

Decision and judgment models are explicitly out of scope. The consuming agent
owns them; Video Perception hands them evidence, not media.

`video-vlm` is defined before it is implemented. The capability id
`video-description` is reserved; no built-in or vendor route is added by this
decision.

## Alternatives considered

- **Keep guidance at the capability level.** Rejected: the same caveats would
  be repeated per capability and drift apart.
- **Name recommended vendors per capability.** Rejected: it recreates the
  central model list that Decision 0001 removed and makes the public contract
  endorse specific products.
- **Add a decision role with a built-in planner.** Rejected: it moves consumer
  knowledge and thresholds into the sensing layer, which the project boundary
  forbids, and it contradicts Decision 0002.

## Consequences

- Skills, strategies, and knowledge packs refer to roles when they explain
  cost, privacy, or reliability. A model change does not invalidate them.
- Provider authors know which provenance fields their role requires before
  writing a manifest.
- Adding `video-description` to the core catalog is a follow-up with a defined
  contract and no vendor dependency.
- The project can state publicly that it binds no model vendor without leaving
  agents without guidance.
