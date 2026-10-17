import { describe, expect, it } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import SkillRegistry from '@deepseek-ai/dsh-skill'
import LocalSubprocessRuntime from '@deepseek-ai/dsh-subprocess-local'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import VideoPerceptionPlugin from '../src/index.js'

describe('native DSH plugin composition', () => {
  it('registers thin Video Perception tools and three bundled skills', async () => {
    const ctx = new Context()
    const systemPrompt = await ctx.plugin(SystemPrompt)
    const tools = await ctx.plugin(ToolRuntime)
    const skills = await ctx.plugin(SkillRegistry)
    const subprocess = await ctx.plugin(LocalSubprocessRuntime)
    const sensor = await ctx.plugin(VideoPerceptionPlugin)
    let disposed = false
    try {
      expect(ctx.tools.schemas().map(schema => schema.name)).toEqual([
        'video_perception_capabilities',
        'video_perception_doctor',
        'video_perception_catalog',
        'video_perception_setup',
        'video_perception_extensions',
        'video_perception_validate_extension',
        'video_perception_observe',
        'video_perception_sample',
        'video_perception_query',
      ])
      expect(await ctx.skills.get('perceive-video')).toEqual(expect.objectContaining({
        provider: 'video-perception-dsh-plugin',
      }))
      expect((await ctx.skills.get('perceive-video'))?.content).toContain('use Video Perception')
      expect(await ctx.skills.get('add-video-provider')).toBeDefined()
      expect(await ctx.skills.get('create-perception-pack')).toBeDefined()
      await sensor.dispose()
      disposed = true
      expect(ctx.tools.schemas()).toEqual([])
      expect(await ctx.skills.get('perceive-video')).toBeUndefined()
      expect(await ctx.skills.get('add-video-provider')).toBeUndefined()
      expect(await ctx.skills.get('create-perception-pack')).toBeUndefined()
    } finally {
      if (!disposed) await sensor.dispose()
      await subprocess.dispose()
      await skills.dispose()
      await tools.dispose()
      await systemPrompt.dispose()
    }
  })
})
