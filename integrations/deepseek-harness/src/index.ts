import type { Context } from '@deepseek-ai/cordis'
import { Config, resolveConfig } from './config.js'
import type { Config as PluginConfig } from './config.js'
import { registerVideoPerceptionSkills } from './skill.js'
import { registerVideoPerceptionTools } from './tools.js'

export { Config }
export type { PluginConfig }

export const name = 'video-perception'
export const inject = ['tools', 'subprocess', 'skills']

export function apply(ctx: Context, config: PluginConfig = {}): void {
  const resolved = resolveConfig(config)
  registerVideoPerceptionTools(ctx, resolved)
  registerVideoPerceptionSkills(ctx)
}

export default { name, inject, Config, apply }
