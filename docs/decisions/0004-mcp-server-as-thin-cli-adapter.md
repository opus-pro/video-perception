# Decision 0004: Serve MCP from the same binary as a thin adapter over the CLI

- Status: accepted
- Date: 2026-10-18

## Context

The plugin and Skills assume a shell-capable agent. Many hosts give an agent
tools but no shell: desktop chat clients, IDE assistants with restricted
terminals, and non-coding agents. Those hosts speak MCP. A weaker model also
handles a typed tool schema more reliably than a CLI it must assemble from
prose. Later work on interactive views needs MCP as the carrier as well.

MCP revision 2026-07-28 removed sessions and the `initialize` handshake. A
server that implements it must answer `server/discover`, tag results with
`resultType`, and attach cache hints to list results, while older clients still
open with `initialize`.

## Decision

Add `video-perception mcp`, a stdio MCP server built into the existing binary
with the `rmcp` crate. The server is a thin adapter: every tool runs this same
executable as a child process with the arguments the CLI documents, captures
its output, and returns it as structured content. No internal function is
exposed directly and no second processing engine exists.

Observation and sample directories are the handles. A tool returns a directory
path; later tools accept it. The CLI reads the same directories, so an agent
can mix both surfaces and a human can inspect the evidence.

The server implements revision 2026-07-28 and remains dual-era: `rmcp`
answers `server/discover`, strips `resultType` for peers that negotiated an
older revision, and defaults cache hints. Business failures (a missing file, a
failed capability) return `isError` tool results; malformed requests return
JSON-RPC errors. Cancelling a request kills the child process.

The server is stdio only. It makes no network calls of its own; remote
providers run only when a SensePlan or strategy names them, exactly as on the
CLI.

## Alternatives considered

- **A separate Node or Python MCP server.** Rejected: it adds a runtime to
  install, a second place where provider rules could drift, and a release that
  can lag the CLI.
- **Refactor the CLI into a library and call it in-process.** Rejected for
  now: the commands print their contracts to stdout and stderr and own
  process-level budgets; wrapping them as a child process keeps one contract
  and one test surface. A library split can come later without changing the
  tool schema.
- **A hosted remote MCP server.** Out of scope: it needs upload, storage,
  billing, and authentication decisions that the local-first product has not
  made.

## Consequences

- Agents without a shell get the full discover, observe, sample, query loop
  with typed parameters and image content for frames and sheets.
- Each tool call costs one extra process spawn; the CLI's own budgets and
  timeouts apply unchanged.
- The tool list is part of the public contract and is pinned by a test.
- `tokio`, `rmcp`, and `base64` join the build. The binary stays single-file.
- `plugin/.mcp.json` declares the server for hosts that load MCP from a
  plugin; the executable must already be installed.
