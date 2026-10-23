# Decision 0005: A capability contract, not a procedure

- Status: accepted
- Date: 2026-10-23

## Context

The perception Skill had grown into a standard operating procedure: read a
playbook first, then run `capabilities`, `catalog list`, `doctor`, and
`extension list`, then plan, then observe. It told the agent how to do video
work.

A with/without evaluation (`evals/perception`, October 2026) measured what that
procedure is worth. Claude Code (Sonnet and Haiku), Codex, and DeepSeek Harness
answered eight everyday questions with and without the plugin on the same
machine. Two findings decided this record:

- Agents already know the methods. Without the plugin they sampled frames,
  built contact sheets, ran `silencedetect`, `blackdetect`, and loudness
  filters, and reached for whisper on their own. Even the smallest model did.
  The procedure added steps, not knowledge: with the plugin, quick checks such
  as "any black screens?" got slower on Claude Code and Codex.
- Agents do not know this machine. Without the plugin, 35 to 47 percent of the
  commands on spoken and long-video questions went to finding out what was
  installed: searching the disk for a speech model, listing Python packages,
  downloading a model, or writing Swift against Apple's speech framework. With
  the plugin those questions were 3 to 20 times faster.

What the agent lacks is an index of this machine's video capabilities and a
stable way to call them, not instructions.

## Decision

Video Perception is a contract with three parts:

1. **Capability declaration.** Every provider, built-in or user-supplied, is a
   `video-perception.provider/v1` manifest. An optional `detect` block lists
   read-only checks that decide whether it can run here: the platform, a
   command on PATH (optionally overridden by an environment variable), a file
   found through an environment variable or a list of globs, or a top-level
   Python package located with `importlib.util.find_spec`, which never imports
   it. `capabilities` evaluates every manifest and reports the result, with the
   resolved paths, as the machine's index.
2. **Output format.** Whatever runs underneath, results are timestamped,
   attributed Observations that can be queried again.
3. **Safety boundary.** Detection never downloads, installs, or runs provider
   code. Installing software, downloading weights, and sending media to a
   remote service remain user-authorized actions.

The detection rules that used to be code (whisper.cpp, the Apple provider
host, Ollama) are now entries in a built-in catalog shipped inside the binary,
evaluated by the same engine as user manifests. A new runtime type,
`external`, describes a tool the agent calls itself and that Video Perception
never runs, such as Tesseract or a short Swift script against Apple's Vision
framework. Its manifest must say how in `usage`. The built-in catalog carries a
default list per platform; users and projects add entries the same way.

The Skill shrinks to the contract: read the index, use what is available, do
quick checks directly, and open a method pack only when the task is hard.
Methods remain available as optional knowledge packs; nothing requires reading
them first.

When an agent finds or writes a capability the index lacks, it can record it as
a user manifest, so the next session finds it in the index instead of searching
again.

## Alternatives considered

- **Keep the procedure and make it adaptive per model.** Rejected: the plugin
  cannot reliably tell which model it is talking to, and the evaluation showed
  the procedure itself was the overhead. Progressive disclosure adapts on its
  own: a strong model stops at the index, a weaker one opens a pack.
- **Let the agent probe the machine itself from hints in the Skill.** Rejected:
  that is the 35 to 47 percent of commands we measured. The CLI evaluates the
  same checks in under 0.1 s.
- **Hard-code more providers in Rust.** Rejected: every new tool would need a
  release, and user knowledge about their own machine would have no place to
  live.

## Consequences

- `capabilities --json` gains a `tools` list: built-in `external` entries and
  every user or project provider, each with `status`, `available`, the
  resolved paths, and `usage`.
- whisper.cpp works without `WHISPER_CPP_BIN` and `WHISPER_CPP_MODEL` when the
  binary is on PATH and a model sits in a usual folder; the variables still
  take precedence.
- A Python check spawns `python3 -I` once per check. Dotted module names are
  rejected because `find_spec` would import the parent package.
- The provider manifest schema gains optional `detect`, `usage`, and `notes`;
  existing manifests remain valid.
- The DeepSeek Harness adapter keeps its step-by-step Skill for now. In the
  evaluation it was the one agent the plugin made faster on every kind of
  question, so it changes only after a rerun shows the contract version does
  not slow it down.
