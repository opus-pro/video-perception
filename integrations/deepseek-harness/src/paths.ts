import { randomUUID } from 'node:crypto'
import { isAbsolute, relative, resolve } from 'node:path'

export function withinWorkspace(workspace: string, input: string, label: string): string {
  const root = resolve(workspace)
  const target = resolve(root, input)
  const rel = relative(root, target)
  if (rel === '..' || rel.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) || isAbsolute(rel)) {
    throw new Error(`video-perception: ${label} must stay inside the session workspace`)
  }
  return target
}

export function newObservationPath(workspace: string, outputDirectory: string): string {
  const base = withinWorkspace(workspace, outputDirectory, 'outputDirectory')
  return resolve(base, `observation-${Date.now()}-${randomUUID().slice(0, 8)}`)
}
