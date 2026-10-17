# Video Perception for DeepSeek Harness

This package is a native Cordis/DeepSeek Harness adapter over the public
`video-perception` executable. Installing it gives Harness tools plus three
bundled Skills for practiced video perception, Provider authoring, and
Perception Pack authoring.

It is intentionally thin:

- video processing remains in the Video Perception CLI;
- the adapter uses Harness subprocess, tool, and skill services;
- it never installs FFmpeg or models;
- paths are restricted to the active Harness workspace;
- every result uses the same Video Perception contracts as other agents.

Install Video Perception first, then build/package the adapter:

```bash
npm ci
npm run check
npm pack
```

Maintainers can generate the `.tgz` artifact with the repository's **Dispatch DeepSeek Harness Plugin** workflow.

The adapter registers these tools:

- `video_perception_capabilities` and `video_perception_doctor` for discovery;
- `video_perception_extensions` and `video_perception_validate_extension` for
  decentralized Providers, Strategies, and Knowledge Packs;
- `video_perception_catalog`, `video_perception_observe`, and
  `video_perception_query` for planning, execution, and retrieval.

Normal prompts such as “tell me what happens in this video” activate the
bundled perception Skill. Provider and pack authoring are separate Skills so
users can extend their own machine without making the core repository a model
registry.
