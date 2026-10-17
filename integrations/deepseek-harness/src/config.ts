import z from '@deepseek-ai/schemastery'

export interface ResolvedConfig {
  command: string
  outputDirectory: string
  commandOutputMaxBytes: number
  commandStderrMaxBytes: number
  subprocessGraceMs: number
  toolTimeoutMs: number
}

export type Config = Partial<ResolvedConfig>

export const DEFAULT_CONFIG: ResolvedConfig = {
  command: 'video-perception',
  outputDirectory: '.video-perception/runs',
  commandOutputMaxBytes: 16 * 1024 * 1024,
  commandStderrMaxBytes: 2 * 1024 * 1024,
  subprocessGraceMs: 3_000,
  toolTimeoutMs: 15 * 60_000,
}

export const Config: z<Config> = z.object({
  command: z.string().default(DEFAULT_CONFIG.command),
  outputDirectory: z.string().default(DEFAULT_CONFIG.outputDirectory),
  commandOutputMaxBytes: z.number().default(DEFAULT_CONFIG.commandOutputMaxBytes),
  commandStderrMaxBytes: z.number().default(DEFAULT_CONFIG.commandStderrMaxBytes),
  subprocessGraceMs: z.number().default(DEFAULT_CONFIG.subprocessGraceMs),
  toolTimeoutMs: z.number().default(DEFAULT_CONFIG.toolTimeoutMs),
})

export function resolveConfig(config: Config = {}): ResolvedConfig {
  const resolved = { ...DEFAULT_CONFIG, ...config }
  if (resolved.command.trim() === '') throw new Error('video-perception: command must be non-empty')
  if (resolved.outputDirectory.trim() === '') throw new Error('video-perception: outputDirectory must be non-empty')
  for (const key of ['commandOutputMaxBytes', 'commandStderrMaxBytes', 'subprocessGraceMs', 'toolTimeoutMs'] as const) {
    if (!Number.isInteger(resolved[key]) || resolved[key] < 1) {
      throw new Error(`video-perception: ${key} must be a positive integer`)
    }
  }
  return resolved
}
