import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import type { Context } from '@deepseek-ai/cordis'
import type {} from '@deepseek-ai/dsh-skill'

const SKILLS = [
  {
    name: 'perceive-video',
    description: 'Understand video with practiced local-first perception instead of an ad hoc frame-sampling workflow.',
    asset: 'perceive-video.md',
  },
  {
    name: 'add-video-provider',
    description: 'Connect an existing local model, CLI, or user-owned API through the Video Perception provider contract.',
    asset: 'add-provider.md',
  },
  {
    name: 'create-perception-pack',
    description: 'Turn reusable video-reading techniques or user knowledge into a strategy and knowledge pack.',
    asset: 'create-perception-pack.md',
  },
] as const

export function registerVideoPerceptionSkills(ctx: Context): void {
  for (const skill of SKILLS) {
    const skillUrl = new URL(`../assets/${skill.asset}`, import.meta.url)
    ctx.effect(() => ctx.skills.register({
      name: skill.name,
      description: skill.description,
      source: 'bundled',
      provider: 'video-perception-dsh-plugin',
      invocation: { modelInvocable: true, userInvocable: true },
      content: readFileSync(skillUrl, 'utf8'),
      path: fileURLToPath(skillUrl),
      resourceBase: {
        kind: 'directory',
        path: fileURLToPath(new URL('../assets/', import.meta.url)),
      },
    }))
  }
}
