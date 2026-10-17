# Video Perception agent plugin

This plugin gives a shell-capable agent practiced video perception over the
public `video-perception` CLI and contracts. It does not include a second
processing engine or install dependencies.

The agent learns to:

1. apply the bundled coarse-to-fine video perception playbook;
2. discover this machine's existing video capabilities;
3. request the smallest useful Strategy or SensePlan;
4. create and query provenance-bearing Observations;
5. author decentralized Provider and Knowledge Pack manifests.

```bash
video-perception capabilities --json
video-perception doctor --json
video-perception catalog list --json
video-perception extension list --json
```

Skills are `perceive-video`, `add-video-provider`, and
`create-perception-pack`. Bundled strategies and the foundation knowledge pack
live beside the Skills under this plugin directory.

## Install

The `video-perception` executable must already be on `PATH`; the plugin
installs nothing. Hosts that load MCP servers from a plugin start
`video-perception mcp` from `mcp.json` (Agent Plugins) or `.mcp.json` (Claude
Code).

**Claude Code.** The repository is a marketplace:

```bash
claude plugin marketplace add opus-pro/video-perception
/plugin install video-perception --marketplace opus-pro/video-perception
```

To work on the plugin, validate and load the directory directly:

```bash
claude plugin validate plugin
claude --plugin-dir /absolute/path/to/video-perception/plugin
```

**Codex and ChatGPT.** The same repository is a Codex marketplace. The
portable `plugin.json` carries the OpenAI presentation under
`extensions.com.openai`; `.codex-plugin/plugin.json` stays as the
compatibility fallback for older builds.

```bash
codex plugin marketplace add opus-pro/video-perception
```

Then install `video-perception` from the plugin browser.

**Any Agent Plugins 1.0 client.** Point the client at this directory.
`plugin.json`, `mcp.json`, and `skills/` are the portable components;
`.claude-plugin/` and `.codex-plugin/` are client overlays that other clients
ignore.

**DeepSeek Harness** uses the separate thin adapter under
`integrations/deepseek-harness`.
