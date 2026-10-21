# With and without Video Perception

This evaluation asks one question: does the same agent, on the same machine,
answer everyday video questions better after Video Perception is installed?

Each task is something a person would actually type, such as "when do I first
mention the Lumina Pro plan?" or "does this have any black screens?". Prompts
never mention FFmpeg, frame sampling, transcripts, or any other method. The
agent has to know what to do.

## What stays the same, what changes

Both conditions run on one machine with the same tools on `PATH`: FFmpeg,
ImageMagick, and whisper.cpp with a local model. Each run starts in an empty
folder that holds only the task's video, and every agent runs headless with the
user's own configuration removed.

| Condition | Video Perception skill | Video Perception MCP server | `video-perception` binary |
|---|---|---|---|
| baseline | no | no | not on `PATH` |
| plugin | yes | yes | on `PATH` |

Isolation per agent, checked with `run.py --probe`:

- **Claude Code** loads project settings only, uses strict MCP, excludes every
  `CLAUDE.md`, and disables memory. The plugin condition adds `--plugin-dir`.
- **Codex** runs with a temporary `HOME` and `CODEX_HOME` that hold only the
  login, so user-level skills do not load. Account-level apps are turned off
  with `apps = false`, because an app can upload the video to a remote service
  and answer from there. The plugin condition adds the skills and the MCP
  server to that temporary home.
- **DeepSeek Harness** runs with a temporary `DSH_HOME`. The plugin condition
  installs the adapter in `integrations/deepseek-harness`.

## Fixtures and ground truth

`fixtures.py` builds two videos from a script, so every event happens at a
known time and nothing is downloaded or committed:

- `tutorial.mp4`, 92 seconds: slides and moving footage, narration, an
  on-screen discount code, a long pause, a black gap, and a loud beep.
- `lecture.mp4`, 12 minutes: 18 slides with continuous narration, one spoken
  password, and a room number that is on screen for four seconds.

The script writes `ground-truth.json` next to the videos. Its timings were
checked against FFmpeg's `blackdetect`, `silencedetect`, and `volumedetect`,
not against Video Perception.

## Tasks

| Task | What the person wants | Passes when |
|---|---|---|
| `tutorial-mention` | Jump to where a product is mentioned | time inside the sentence, 2 s early or 1 s late allowed |
| `tutorial-dead-air` | Cut the long pause | start and end within 1.5 s |
| `tutorial-code` | Read a code shown on screen | exact code, start within 2 s |
| `tutorial-black` | Find blank screens before upload | the one black gap, ends within 1 s, nothing extra |
| `tutorial-spike` | Catch a sound that hurts | time within 1 s |
| `tutorial-cuts` | Count hard cuts | within 1 of the true count |
| `lecture-password` | Find one sentence in 12 minutes | right words, time inside the sentence, 3 s early or 1 s late allowed |
| `lecture-room` | Catch 4 s of text in 12 minutes | right room, start within 3 s |

## Run it

The agents run headless with shell access, so they can run any command your
account can. Run the evaluation in a virtual machine or a throwaway user
account. The shell is not narrowed to a few tools on purpose: the baseline is
meant to show what an agent does on its own, including installing or
downloading things. Each run gets only the environment variables listed in
`ENV_KEEP` in `run.py`.

macOS is required for the fixtures because narration uses `say`.

```sh
python3 evals/perception/fixtures.py
export WHISPER_CPP_BIN="$(command -v whisper-cli)"
export WHISPER_CPP_MODEL="$HOME/.local/share/whisper/ggml-base.bin"

python3 evals/perception/run.py --agent claude --probe
python3 evals/perception/run.py --agent claude --model sonnet --repeat 2 --parallel 4
python3 evals/perception/run.py --agent codex --repeat 2 --parallel 4
DEEPSEEK_API_KEY=... python3 evals/perception/run.py --agent dsh --repeat 2

python3 evals/perception/grade.py evals/perception/results/raw/*.jsonl \
  --out evals/perception/results/summary.md --graded evals/perception/results/graded.jsonl
```

`run.py` writes one JSON line per run to `results/raw/`. `grade.py` reads the
final `ANSWER:` line of each run, grades it against the ground truth, and
writes the summary plus a per-run file with each answer, the reason it passed
or failed, and the shell commands the agent ran. Local paths are replaced with
`~` and `<tmp>` before anything is written.

## Results, October 2026

Runs: Claude Code with Sonnet and with Haiku, and Codex with its default model,
16 runs per condition each. The full table is in
[`results/summary.md`](results/summary.md), and every run's answer, grade and
shell commands are in [`results/graded.jsonl`](results/graded.jsonl).

| Agent | Right answers, without and with | Spoken or long-video questions, median time | Quick checks, median time |
|---|---|---|---|
| Claude Code, Sonnet | 16/16 and 14/16 | 109 s and 28 s | 15 s and 24 s |
| Claude Code, Haiku | 16/16 and 15/16 | 412 s and 19 s | 23 s and 31 s |
| Codex | 14/16 and 16/16 | 164 s and 64 s | 32 s and 60 s |

What we learned:

- On a machine where FFmpeg and whisper.cpp are already installed, all three
  agents usually find the answer without the plugin. Without it they improvise:
  Claude Code built a Python environment and downloaded a speech model, and
  Codex guessed two spoken-word answers from the slides.
- The plugin helps most when the answer is spoken or sits in a long video. It
  costs a few seconds on quick checks such as black frames or a loud spike.
- The plugin made counting cuts worse. Its scene detector reports false
  boundaries on static slides and counts text overlays as cuts, and agents
  trusted that count.

Known measurement noise:

- A few runs in both conditions last about 12 minutes because the agent started
  a `find /` search for the video's absolute path. Claude Code's headless mode
  moves it to the background and waits for it before exiting. Medians are
  barely affected.
- The first Codex run had account-level apps enabled. In three baseline runs
  Codex uploaded the video to a remote video service and answered from there.
  Those runs were discarded and all 32 Codex runs were repeated with
  `apps = false`.

DeepSeek Harness runs need a `DEEPSEEK_API_KEY` and are not included yet.
