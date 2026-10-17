---
name: create-perception-pack
description: "Turn reusable video-reading techniques, tutorials, or user workflow knowledge into a decentralized Video Perception strategy and knowledge pack."
---

# Create a perception pack

Use this skill when the user wants Video Perception to learn a recurring way of
reading video: a source type, editing method, review practice, tutorial, or
team-specific workflow.

Keep two layers separate:

- `strategy.yaml` chooses capabilities and technical parameters for gathering
  evidence;
- knowledge Markdown tells the consuming agent how to interpret that evidence
  for the user's task.

Do not encode publishing actions, private credentials, or final business
verdicts into the core capability contracts. Do not redistribute tutorial
media, copied documentation, model weights, or datasets without verifying their
licenses.

Create the pack in a project or user extension directory:

```text
my-pack/
├── pack.yaml
├── strategy.yaml
├── knowledge.md
├── examples/
└── evals/
```

Use `video-perception.strategy/v1` and
`video-perception.knowledge-pack/v1`. Give the pack a namespaced id. Reference
knowledge using paths contained within the pack; parent-directory traversal is
invalid.

Validate the complete directory:

```bash
video-perception extension validate ./my-pack --json
```

Test the strategy on at least one safe video and compare it with the bundled
quick scan. Record which extra evidence changed the answer, processing time,
sampled frames, provider locality, and optional failures. Keep the pack local
unless the user explicitly wants to publish it.
