#!/usr/bin/env python3
"""Run the perception tasks with and without Video Perception.

Each run gets a fresh folder holding only the task's video, the same prompt,
and the same machine tools (FFmpeg, whisper.cpp, ImageMagick stay on PATH in
both conditions). The only difference between conditions is the plugin:

  baseline  the agent with no Video Perception skill, MCP server, or binary
  plugin    the same agent with the Video Perception skills, MCP server, and binary

Agents run headless and isolated from the user's own configuration:

  claude  Claude Code: project settings only, strict MCP, no CLAUDE.md, no memory
  codex   Codex CLI: a temporary CODEX_HOME holding only the login
  dsh     DeepSeek Harness: a temporary DSH_HOME; needs DEEPSEEK_API_KEY

    python3 evals/perception/run.py --agent claude --probe
    python3 evals/perception/run.py --agent claude --repeat 2 --parallel 4
"""

from __future__ import annotations

import argparse
import concurrent.futures as futures
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PLUGIN = ROOT / "plugin"
DSH_PACKAGE = ROOT / "integrations" / "deepseek-harness"
# Only what the agents and local tools need; unrelated credentials stay out of the runs.
ENV_KEEP = {"HOME", "USER", "LOGNAME", "PATH", "SHELL", "TMPDIR", "LANG", "TERM",
            "WHISPER_CPP_BIN", "WHISPER_CPP_MODEL", "DEEPSEEK_API_KEY", "CODEX_HOME",
            # Network settings, not credentials: machines behind a TLS-inspecting proxy need these to reach model APIs.
            "NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE", "SSL_CERT_DIR", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE",
            "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "no_proxy", "all_proxy"}
PROBE = ("Answer in three lines and nothing else: (1) every skill name you can use, "
         "(2) every plugin name, (3) every MCP server and its tools.")


def find_binary(explicit: str | None) -> Path:
    for candidate in (explicit, ROOT / "cli/target/release/video-perception", shutil.which("video-perception")):
        if candidate and Path(candidate).is_file():
            return Path(candidate).resolve()
    sys.exit("video-perception binary not found; build it with `cargo build --release` in cli/")


def base_env(binary: Path, condition: str) -> dict:
    """Same environment for both conditions, except where the binary is reachable."""
    env = {k: v for k, v in os.environ.items() if k in ENV_KEEP or k.startswith("LC_")}
    paths = [p for p in env.get("PATH", "").split(os.pathsep)
             if p and not (Path(p) / "video-perception").exists()]
    if condition == "plugin":
        paths.insert(0, str(binary.parent))
    env["PATH"] = os.pathsep.join(paths)
    return env


def prompt_for(task: dict, suffix: str) -> str:
    keys = ", ".join(f'"{k}" ({v})' for k, v in task["keys"].items())
    return f"{task['prompt']}\n\n{suffix.format(keys=keys)}"


# ---------------------------------------------------------------- agents


def run_claude(ws: Path, prompt: str, condition: str, env: dict, model: str | None, timeout: int, home: Path) -> dict:
    settings = {
        "claudeMdExcludes": [str(Path.home() / ".claude" / "CLAUDE.md"), "**/CLAUDE.md", "**/CLAUDE.local.md"],
        "autoMemoryEnabled": False,
    }
    cmd = ["claude", "-p", prompt, "--output-format", "stream-json", "--verbose",
           "--setting-sources", "project", "--strict-mcp-config", "--settings", json.dumps(settings),
           "--permission-mode", "acceptEdits", "--allowedTools", "Bash", "Read", "Write", "Edit", "Glob", "Grep"]
    if model:
        cmd += ["--model", model]
    if condition == "plugin":
        mcp = home / "mcp.json"
        mcp.write_text(json.dumps({"mcpServers": {"video-perception": {"command": "video-perception", "args": ["mcp"]}}}))
        cmd += ["--plugin-dir", str(PLUGIN), "--mcp-config", str(mcp), "--allowedTools", "mcp__video-perception"]
    raw, code, seconds = execute(cmd, ws, env, timeout)
    final, commands, tools, usage = "", [], [], {}
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "assistant":
            for block in event.get("message", {}).get("content", []):
                if block.get("type") == "tool_use":
                    tools.append(block.get("name"))
                    if block.get("name") == "Bash":
                        commands.append(block.get("input", {}).get("command", ""))
        elif event.get("type") == "result":
            final = event.get("result") or ""
            usage = {"cost_usd": event.get("total_cost_usd"), "turns": event.get("num_turns"),
                     "model_usage": list((event.get("modelUsage") or {}).keys())}
    return {"exit": code, "seconds": seconds, "final": final, "commands": commands, "tools": tools,
            "usage": usage, "raw": raw}


def run_codex(ws: Path, prompt: str, condition: str, env: dict, model: str | None, timeout: int, home: Path) -> dict:
    # A temporary HOME keeps user-level skills (~/.agents/skills) out of both conditions.
    fake_home = home / "home"
    codex_home = fake_home / ".codex"
    codex_home.mkdir(parents=True)
    auth = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")) / "auth.json"
    if not auth.is_file():
        sys.exit(f"codex login not found at {auth}")
    shutil.copy2(auth, codex_home / "auth.json")
    # Account-level apps (connectors) can upload media to remote services; keep them out of both conditions.
    config = ["[features]", "apps = false", ""]
    if condition == "plugin":
        shutil.copytree(PLUGIN / "skills", codex_home / "skills")
        shutil.copytree(PLUGIN / "packs", codex_home / "packs")
        command = shutil.which("video-perception", path=env["PATH"])
        config += ["[mcp_servers.video-perception]", f"command = {json.dumps(command)}", 'args = ["mcp"]']
    (codex_home / "config.toml").write_text("\n".join(config) + "\n")
    env = dict(env, HOME=str(fake_home), CODEX_HOME=str(codex_home))
    cmd = ["codex", "exec", "--skip-git-repo-check", "--json", "-C", str(ws), "--sandbox", "workspace-write"]
    if model:
        cmd += ["--model", model]
    raw, code, seconds = execute(cmd + [prompt], ws, env, timeout)
    final, commands, tools, usage = "", [], [], {}
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item") or {}
        if event.get("type") == "item.completed":
            kind = item.get("type")
            if kind == "agent_message":
                final = item.get("text") or final
            elif kind == "command_execution":
                tools.append("shell")
                commands.append(item.get("command", ""))
            elif kind:
                tools.append(kind if kind != "mcp_tool_call" else f"mcp:{item.get('server')}:{item.get('tool')}")
        elif event.get("type") == "turn.completed":
            usage = event.get("usage") or usage
    return {"exit": code, "seconds": seconds, "final": final, "commands": commands, "tools": tools,
            "usage": usage, "raw": raw}


def run_dsh(ws: Path, prompt: str, condition: str, env: dict, model: str | None, timeout: int, home: Path) -> dict:
    if not env.get("DEEPSEEK_API_KEY"):
        sys.exit("dsh runs need DEEPSEEK_API_KEY in the environment")
    dsh_home = home / "dsh"
    dsh_home.mkdir()
    env = dict(env, DSH_HOME=str(dsh_home))
    if condition == "plugin":
        packed = sorted((home / "pack").glob("*.tgz")) if (home / "pack").exists() else []
        if not packed:
            (home / "pack").mkdir(exist_ok=True)
            subprocess.run(["npm", "pack", "--pack-destination", str(home / "pack")], cwd=DSH_PACKAGE,
                           check=True, capture_output=True)
            packed = sorted((home / "pack").glob("*.tgz"))
        version = json.loads((DSH_PACKAGE / "package.json").read_text())
        dsh_version = subprocess.run(["dsh", "--version"], capture_output=True, text=True).stdout.strip()
        # DSH refuses plugin versions it has not seen for this DSH build; allow ours in this throwaway home.
        subprocess.run(["dsh", "plugin", "--profile", "headless", "allow-version",
                        f"{version['name']}@{version['version']}", "--dsh-version", dsh_version, "--accept-risk"],
                       env=env, check=True, capture_output=True)
        subprocess.run(["dsh", "plugin", "--profile", "headless", "add", str(packed[0])],
                       env=env, check=True, capture_output=True)
    raw, code, seconds = execute(["dsh", "headless", prompt], ws, env, timeout)
    return {"exit": code, "seconds": seconds, "final": raw.strip(), "commands": [], "tools": [],
            "usage": {}, "raw": raw}


AGENTS = {"claude": run_claude, "codex": run_codex, "dsh": run_dsh}


def execute(cmd: list[str], cwd: Path, env: dict, timeout: int) -> tuple[str, int, float]:
    started = time.monotonic()
    try:
        done = subprocess.run(cmd, cwd=cwd, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                              text=True, timeout=timeout)
        out, code = done.stdout + ("\n" + done.stderr if done.returncode else ""), done.returncode
    except subprocess.TimeoutExpired as err:
        out = (err.stdout or b"").decode() if isinstance(err.stdout, bytes) else (err.stdout or "")
        code = -9
    return out, code, round(time.monotonic() - started, 1)


# ---------------------------------------------------------------- driver


def one_run(args, binary: Path, fixtures: Path, task: dict | None, prompt: str, condition: str, attempt: int) -> dict:
    with tempfile.TemporaryDirectory(prefix="vp-eval-") as tmp:
        home = Path(tmp)
        ws = home / "work"
        ws.mkdir()
        if task:
            shutil.copy2(fixtures / task["video"], ws / task["video"])
        env = base_env(binary, condition)
        result = AGENTS[args.agent](ws, prompt, condition, env, args.model, args.timeout, home)
    result.update({
        "agent": args.agent, "model": args.model or "default", "condition": condition,
        "task": task["id"] if task else "probe", "attempt": attempt,
        "used_video_perception": any("video-perception" in c for c in result["commands"])
        or any("video-perception" in str(t) or "video_perception" in str(t) for t in result["tools"]),
    })
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--agent", choices=sorted(AGENTS), required=True)
    parser.add_argument("--condition", choices=["baseline", "plugin", "both"], default="both")
    parser.add_argument("--task", action="append", help="task id (repeatable); default: all")
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--parallel", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=900, help="seconds per run")
    parser.add_argument("--model")
    parser.add_argument("--bin", help="path to the video-perception binary")
    parser.add_argument("--fixtures", type=Path, default=HERE / "fixtures")
    parser.add_argument("--out", type=Path, default=HERE / "results" / "raw")
    parser.add_argument("--probe", action="store_true", help="print what each condition can see and exit")
    args = parser.parse_args()

    binary = find_binary(args.bin)
    conditions = ["baseline", "plugin"] if args.condition == "both" else [args.condition]
    spec = json.loads((HERE / "tasks.json").read_text())

    if args.probe:
        failed = False
        for condition in conditions:
            result = one_run(args, binary, args.fixtures, None, PROBE, condition, 0)
            print(f"== {args.agent} / {condition} (exit {result['exit']})\n{result['final'].strip()}\n")
            failed = failed or result["exit"] != 0
        sys.exit(1 if failed else 0)

    if not (args.fixtures / "ground-truth.json").is_file():
        sys.exit(f"fixtures missing in {args.fixtures}; run fixtures.py first")
    tasks = [t for t in spec["tasks"] if not args.task or t["id"] in args.task]
    jobs = [(t, c, a) for a in range(1, args.repeat + 1) for t in tasks for c in conditions]
    args.out.mkdir(parents=True, exist_ok=True)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    log = args.out / f"{args.agent}-{stamp}.jsonl"
    with futures.ThreadPoolExecutor(max_workers=args.parallel) as pool, log.open("a") as sink:
        pending = {pool.submit(one_run, args, binary, args.fixtures, t, prompt_for(t, spec["suffix"]), c, a): (t, c, a)
                   for t, c, a in jobs}
        for done in futures.as_completed(pending):
            task, condition, attempt = pending[done]
            result = done.result()
            sink.write(json.dumps(result) + "\n")
            sink.flush()
            print(f"{task['id']:<20} {condition:<8} #{attempt} exit={result['exit']} "
                  f"{result['seconds']:>6}s vp={result['used_video_perception']}", flush=True)
    print(f"wrote {log}")


if __name__ == "__main__":
    main()
