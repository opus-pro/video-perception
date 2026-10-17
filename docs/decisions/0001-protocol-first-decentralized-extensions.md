# Decision 0001: Protocol-first decentralized extensions

- Status: accepted
- Date: 2026-09-06

## Context

Video Perception began as a way for an Agent to discover and use video-related
capabilities already present on a user's computer. A centralized provider and
model list would make every new model depend on a core release and would turn
the project into a model catalog rather than a local perception contract.

At the same time, a file convention without a runtime would leave discovery,
fallback, privacy, output normalization, and provenance inconsistent across
agents.

## Decision

Ship a small CLI runtime plus Agent Skills and versioned file contracts.
Providers, strategies, and knowledge packs are discovered from explicit, user,
and project roots. They are usable without a central registry or contribution
to this repository.

The CLI validates manifests, resolves capabilities, executes explicitly
selected providers, and writes standard Observations. Skills deliver general
video-perception practice and teach Agents how to author extensions. User and
company knowledge remains in optional packs.

Model identifiers remain open strings. Provider packages advertise only the
capability cells they implement; they are never required to maintain a complete
matrix.

## Consequences

- Existing protocol-compatible models can be adopted through configuration.
- Third-party command providers execute only after explicit selection.
- Duplicate extension ids fail closed rather than overriding silently.
- Core maintainers own contracts and baseline practices, not every model entry.
- A curated registry can be added later without becoming a runtime dependency.
