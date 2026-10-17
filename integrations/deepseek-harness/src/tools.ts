import { readFile } from 'node:fs/promises'
import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import type { JsonValue, ToolRunContext } from '@deepseek-ai/dsh-tools'
import type { ResolvedConfig } from './config.js'
import { newObservationPath, withinWorkspace } from './paths.js'
import { assertSucceeded, VideoPerceptionRunner, parseJson } from './runner.js'

const JSON_OUTPUT_SCHEMA = { type: 'json' } as const

export function registerVideoPerceptionTools(ctx: Context, config: ResolvedConfig): void {
  const runner = new VideoPerceptionRunner(ctx, config)

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_capabilities',
    description: 'Discover the video sensing capabilities and provider paths already available on this machine. Read-only; installs nothing.',
    parameters: {},
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: 30_000,
    async execute(_args, exec) {
      return runJson(runner, ['capabilities', '--json'], workspaceOf(exec), exec.signal)
    },
    presentCall: () => ({ card: 'generic', title: 'Discover video capabilities', kind: 'read' }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_doctor',
    description: 'Verify Video Perception core tools and live local provider services. Read-only; does not repair or install.',
    parameters: {},
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: 30_000,
    async execute(_args, exec) {
      return runJson(runner, ['doctor', '--json'], workspaceOf(exec), exec.signal)
    },
    presentCall: () => ({ card: 'generic', title: 'Check video sensing setup', kind: 'read' }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_catalog',
    description: 'List the stable Video Perception capability ids, dependencies, output contracts, and streaming support. Read-only.',
    parameters: {},
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: 30_000,
    async execute(_args, exec) {
      return runJson(runner, ['catalog', 'list', '--json'], workspaceOf(exec), exec.signal)
    },
    presentCall: () => ({ card: 'generic', title: 'List video perception capabilities', kind: 'read' }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_setup',
    description: 'Return a read-only setup plan for a basic, Mac-native, or local-model video sensing route. Changes nothing.',
    parameters: {
      profile: { type: 'string', required: true, enum: ['basic', 'mac-native', 'local-model'] },
    },
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: 30_000,
    async execute(args, exec) {
      return runJson(runner, ['setup', '--profile', args.profile, '--json'], workspaceOf(exec), exec.signal)
    },
    presentCall: args => ({ card: 'generic', title: `Plan ${args.profile} sensing`, kind: 'read' }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_extensions',
    description: 'List decentralized provider, strategy, and knowledge-pack manifests visible to this workspace. Read-only; executes no provider code.',
    parameters: {},
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: 30_000,
    async execute(_args, exec) {
      return runJson(runner, ['extension', 'list', '--json'], workspaceOf(exec), exec.signal)
    },
    presentCall: () => ({ card: 'generic', title: 'Discover video extensions', kind: 'read' }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_validate_extension',
    description: 'Validate a provider, strategy, or knowledge pack inside the workspace without executing provider code.',
    parameters: {
      path: { type: 'string', required: true, description: 'Manifest or extension directory inside the session workspace.' },
    },
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: 30_000,
    async execute(args, exec) {
      const workspace = workspaceOf(exec)
      const path = withinWorkspace(workspace, args.path, 'extension path')
      return runJson(runner, ['extension', 'validate', path, '--json'], workspace, exec.signal)
    },
    presentCall: args => ({ card: 'generic', title: 'Validate video extension', kind: 'read', rawInput: args.path }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_observe',
    description: 'Perceive one workspace video into a reusable Observation using exactly one capability list, strategy, or SensePlan. No dependency is installed automatically.',
    parameters: {
      source: { type: 'string', required: true, description: 'Video path inside the session workspace.' },
      capabilities: {
        type: 'array',
        items: { type: 'string' },
        description: 'Video Perception catalog capability ids, for example probe, scenes, stats, transcript, or scene-description.',
      },
      strategy: {
        type: 'string',
        description: 'Discovered strategy id, or a strategy manifest path inside the session workspace.',
      },
      plan: {
        type: 'string',
        description: 'SensePlan JSON path inside the session workspace. Use this to select a custom provider.',
      },
    },
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: config.toolTimeoutMs,
    async execute(args, exec) {
      const workspace = workspaceOf(exec)
      const source = withinWorkspace(workspace, args.source, 'source')
      const output = newObservationPath(workspace, config.outputDirectory)
      const requestCount = [args.capabilities, args.strategy, args.plan]
        .filter(value => value !== undefined).length
      if (requestCount !== 1) {
        throw new Error('video-perception: provide exactly one of capabilities, strategy, or plan')
      }
      const argv = ['observe', source]
      if (args.capabilities !== undefined) {
        const capabilities = args.capabilities.map(String)
        if (capabilities.length === 0) throw new Error('video-perception: capabilities must not be empty')
        argv.push('--capabilities', capabilities.join(','))
      } else if (args.strategy !== undefined) {
        const strategy = args.strategy.includes('/') || args.strategy.startsWith('.')
          ? withinWorkspace(workspace, args.strategy, 'strategy')
          : args.strategy
        argv.push('--strategy', strategy)
      } else if (args.plan !== undefined) {
        argv.push('--plan', withinWorkspace(workspace, args.plan, 'plan'))
      }
      argv.push('--out', output)
      const result = await runner.run(argv, workspace, exec.signal)
      assertSucceeded('video-perception observe', result)
      const manifest = parseJson('Video Perception manifest', await readFile(`${output}/manifest.json`, 'utf8'))
      return toJsonValue({ observation: output, manifest })
    },
    presentCall: args => ({ card: 'generic', title: 'Observe video', kind: 'execute', rawInput: args.source }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_sample',
    description: 'Gather a bounded local visual sample for overview or a closer look. Returns timestamped image paths, not semantic claims. Open the images with host vision or use an explicitly selected visual provider.',
    parameters: {
      source: { type: 'string', required: true, description: 'Local video inside the workspace.' },
      start: { type: 'number', description: 'Start seconds, default 0.' },
      end: { type: 'number', required: true, description: 'Exclusive end seconds; window must fit the frame budget.' },
      fps: { type: 'number', description: 'Maximum sampling rate, default 0.2.' },
      max_frames: { type: 'integer', description: 'Per-round frame budget, default 48.' },
      max_size: { type: 'integer', description: 'Maximum image edge, default 960 pixels.' },
      crop: { type: 'string', description: 'Optional x,y,width,height pixel crop.' },
      parent: { type: 'string', description: 'Previous sample.json inside the workspace.' },
      sheet: { type: 'boolean', description: 'Create an optional timestamp-labelled contact sheet.' },
    },
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: config.toolTimeoutMs,
    async execute(args, exec) {
      const workspace = workspaceOf(exec)
      const source = withinWorkspace(workspace, args.source, 'source')
      const output = newObservationPath(workspace, config.outputDirectory)
      const argv = ['sample', source, '--out', output, '--end', String(args.end), '--timeout-seconds', String(Math.max(1, Math.min(60, Math.floor(config.toolTimeoutMs / 1000) - 2)))]
      if (args.start !== undefined) argv.push('--start', String(args.start))
      if (args.fps !== undefined) argv.push('--fps', String(args.fps))
      if (args.max_frames !== undefined) argv.push('--max-frames', String(args.max_frames))
      if (args.max_size !== undefined) argv.push('--max-size', String(args.max_size))
      if (args.crop !== undefined) argv.push('--crop', args.crop)
      if (args.parent !== undefined) argv.push('--parent', withinWorkspace(workspace, args.parent, 'parent'))
      if (args.sheet) argv.push('--sheet')
      const sample = await runJson(runner, argv, workspace, exec.signal)
      return toJsonValue({ directory: output, sample })
    },
    presentCall: args => ({ card: 'generic', title: 'Gather video frames', kind: 'execute', rawInput: args.source }),
  })))

  ctx.effect(() => ctx.tools.register(defineTool({
    name: 'video_perception_query',
    description: 'Query structured evidence from a previous Video Perception Observation without reprocessing the video.',
    parameters: {
      observation: { type: 'string', required: true, description: 'Observation directory inside the session workspace.' },
      at: { type: 'string', description: 'Timestamp such as 12.5s or 00:00:12.500.' },
      text: { type: 'string', description: 'Case-insensitive transcript text search.' },
      track: { type: 'string', description: 'Optional evidence track.' },
      limit: { type: 'integer', description: 'Maximum results, default 50.' },
    },
    output: { schema: JSON_OUTPUT_SCHEMA, render: renderJson },
    timeoutMs: 30_000,
    async execute(args, exec) {
      const workspace = workspaceOf(exec)
      const observation = withinWorkspace(workspace, args.observation, 'observation')
      const argv = ['observation', 'query', observation, '--fmt', 'json', '--limit', String(args.limit ?? 50)]
      if (args.at !== undefined) argv.push('--at', args.at)
      if (args.text !== undefined) argv.push('--text', args.text)
      if (args.track !== undefined) argv.push('--track', args.track)
      return runJson(runner, argv, workspace, exec.signal)
    },
    presentCall: args => ({ card: 'generic', title: 'Query video evidence', kind: 'read', rawInput: args.observation }),
  })))
}

async function runJson(
  runner: VideoPerceptionRunner,
  argv: readonly string[],
  workspace: string,
  signal?: AbortSignal,
): Promise<JsonValue> {
  const result = await runner.run(argv, workspace, signal)
  assertSucceeded(`video-perception ${argv[0] ?? ''}`, result)
  return toJsonValue(parseJson('Video Perception', result.stdout))
}

function workspaceOf(exec: ToolRunContext): string {
  return exec.agent?.session.header.cwd ?? process.cwd()
}

function renderJson(_args: unknown, value: JsonValue) {
  return [{ type: 'text' as const, text: JSON.stringify(value, null, 2) }]
}

function toJsonValue(value: unknown): JsonValue {
  return JSON.parse(JSON.stringify(value)) as JsonValue
}
