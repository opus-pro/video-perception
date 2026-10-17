import { rm } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../', import.meta.url))
const output = fileURLToPath(new URL('../lib/', import.meta.url))
const tsc = fileURLToPath(new URL('../node_modules/typescript/bin/tsc', import.meta.url))

await rm(output, { recursive: true, force: true })
const result = spawnSync(process.execPath, [tsc, '-p', 'tsconfig.json'], {
  cwd: root,
  stdio: 'inherit',
})
process.exit(result.status ?? 1)
