#!/usr/bin/env node
// video-perception launcher.
//
// Downloads the release binary that matches this machine the first time it
// runs, verifies it against the release checksums, caches it, and then runs
// it with the given arguments. Nothing is printed to stdout by the launcher
// itself, so `npx video-perception mcp` keeps stdout clean for MCP.
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { homedir, platform, arch, tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const pkg = JSON.parse(readFileSync(join(here, 'package.json'), 'utf8'))
const VERSION = pkg.version
const REPO = 'opus-pro/video-perception'
const BINARY = 'video-perception'
const BASE_URL =
  process.env.VIDEO_PERCEPTION_RELEASE_BASE_URL ||
  `https://github.com/${REPO}/releases/download/v${VERSION}`

function log(message) {
  process.stderr.write(`[video-perception] ${message}\n`)
}

function fail(message) {
  log(message)
  process.exit(1)
}

function isMusl() {
  try {
    return platform() === 'linux' && !process.report.getReport().header.glibcVersionRuntime
  } catch {
    return false
  }
}

export function assetName(version, os = platform(), cpu = arch(), musl = isMusl()) {
  const table = {
    'darwin:arm64': 'darwin_arm64',
    'darwin:x64': 'darwin_amd64',
    'linux:x64': musl ? 'linux_amd64_musl' : 'linux_amd64',
  }
  const suffix = table[`${os}:${cpu}`]
  if (!suffix) {
    return null
  }
  return `${BINARY}_${version}_${suffix}.tar.gz`
}

export function expectedChecksum(checksumsText, name) {
  for (const line of checksumsText.split('\n')) {
    const match = line.trim().match(/^([0-9a-f]{64})\s+\*?(.+)$/i)
    if (match && match[2] === name) {
      return match[1].toLowerCase()
    }
  }
  return null
}

function cacheDir() {
  if (process.env.VIDEO_PERCEPTION_NPM_CACHE) {
    return process.env.VIDEO_PERCEPTION_NPM_CACHE
  }
  const base = process.env.XDG_CACHE_HOME || join(homedir(), '.cache')
  return join(base, BINARY, 'npm', VERSION)
}

async function fetchBytes(url) {
  const response = await fetch(url, { redirect: 'follow' })
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText} for ${url}`)
  }
  return Buffer.from(await response.arrayBuffer())
}

async function install(target) {
  const name = assetName(VERSION)
  if (!name) {
    fail(
      `no published binary for ${platform()} ${arch()}; build from source: ` +
        `https://github.com/${REPO}#quick-start`,
    )
  }
  log(`downloading ${name} from ${BASE_URL}`)
  const [archive, checksums] = await Promise.all([
    fetchBytes(`${BASE_URL}/${name}`),
    fetchBytes(`${BASE_URL}/checksums.txt`),
  ])
  const expected = expectedChecksum(checksums.toString('utf8'), name)
  if (!expected) {
    fail(`checksums.txt has no entry for ${name}`)
  }
  const actual = createHash('sha256').update(archive).digest('hex')
  if (actual !== expected) {
    fail(`checksum mismatch for ${name}: expected ${expected}, got ${actual}`)
  }
  const staging = mkdtempSync(join(tmpdir(), 'video-perception-'))
  try {
    const archivePath = join(staging, name)
    writeFileSync(archivePath, archive)
    const tar = spawnSync('tar', ['-xzf', archivePath, '-C', staging], { stdio: ['ignore', 'ignore', 'inherit'] })
    if (tar.status !== 0) {
      fail(`could not extract ${name} (tar exited ${tar.status})`)
    }
    const extracted = join(staging, BINARY)
    if (!existsSync(extracted)) {
      fail(`${name} did not contain ${BINARY}`)
    }
    chmodSync(extracted, 0o755)
    mkdirSync(dirname(target), { recursive: true })
    renameSync(extracted, target)
  } finally {
    rmSync(staging, { recursive: true, force: true })
  }
  log(`installed ${BINARY} ${VERSION} to ${target}`)
}

async function main() {
  const target = join(cacheDir(), BINARY)
  if (!existsSync(target)) {
    try {
      await install(target)
    } catch (error) {
      fail(`install failed: ${error.message}`)
    }
  }
  const child = spawnSync(target, process.argv.slice(2), { stdio: 'inherit' })
  if (child.error) {
    fail(`could not run ${target}: ${child.error.message}`)
  }
  process.exit(child.status ?? 1)
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main()
}
