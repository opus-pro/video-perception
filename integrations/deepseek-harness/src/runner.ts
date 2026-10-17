import type { Context } from '@deepseek-ai/cordis'
import type { SubprocessHandle } from '@deepseek-ai/dsh-subprocess'
import type { ResolvedConfig } from './config.js'

export interface CommandResult {
  stdout: string
  stderr: string
  exitCode: number | null
  signal: NodeJS.Signals | null
}

export class VideoPerceptionRunner {
  constructor(
    private readonly ctx: Context,
    private readonly config: ResolvedConfig,
  ) {}

  async run(argv: readonly string[], cwd: string, signal?: AbortSignal): Promise<CommandResult> {
    let handle: SubprocessHandle
    handle = this.ctx.subprocess.spawn({
      argv: [this.config.command, ...argv],
      cwd,
      stdio: {
        stdin: 'ignore',
        stdout: { maxBytes: this.config.commandOutputMaxBytes },
        stderr: { maxBytes: this.config.commandStderrMaxBytes },
      },
      graceMs: this.config.subprocessGraceMs,
      ...(signal === undefined ? {} : { signal }),
    })
    const outcome = await handle.done
    const stdout = handle.collected.stdout?.readFrom(0)
    const stderr = handle.collected.stderr?.readFrom(0)
    if (stdout?.lossy) throw new Error('video-perception: command stdout exceeded the capture limit')
    if (stderr?.lossy) throw new Error('video-perception: command stderr exceeded the capture limit')
    return {
      stdout: stdout?.text ?? '',
      stderr: stderr?.text ?? '',
      exitCode: outcome.exitCode,
      signal: outcome.signal,
    }
  }
}

export function assertSucceeded(label: string, result: CommandResult): void {
  if (result.exitCode === 0) return
  const detail = result.stderr.trim() || result.stdout.trim() || `signal ${result.signal ?? 'unknown'}`
  throw new Error(`${label} failed${result.exitCode === null ? '' : ` (exit ${result.exitCode})`}: ${detail}`)
}

export function parseJson(label: string, text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    throw new Error(`${label} returned invalid JSON`)
  }
}
