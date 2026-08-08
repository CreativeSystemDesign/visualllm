#!/usr/bin/env node

/**
 * Verify a locally downloaded release artifact directory and latest.json.
 *
 * Usage: node tools/verify-release.js <artifact-dir> [manifest-path]
 *
 * This intentionally checks the relationships that can be established without
 * GitHub credentials: every manifest URL resolves to a local payload, its
 * adjacent Tauri signature exists and is the value recorded in the manifest,
 * platform keys use explicit architectures, macOS is not advertised for the
 * unsigned/manual-update preview, and published checksum files match payloads.
 */

const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')

function fail(message) {
  console.error(`release verification failed: ${message}`)
  process.exitCode = 1
}

function filesUnder(root) {
  const found = []
  function visit(current) {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name)
      if (entry.isDirectory()) visit(full)
      else found.push(full)
    }
  }
  visit(root)
  return found
}

function byBaseName(files) {
  const index = new Map()
  for (const file of files) {
    const name = path.basename(file)
    const prior = index.get(name)
    if (prior && !/^SHA256SUMS(?:\.txt)?$/i.test(name)) {
      throw new Error(`duplicate artifact basename: ${name}`)
    }
    if (prior) continue
    index.set(name, file)
  }
  return index
}

function sha256(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
}

function verifyChecksums(files, index) {
  for (const file of files.filter((item) => /^SHA256SUMS(?:\.txt)?$/i.test(path.basename(item)))) {
    const lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
    for (const line of lines) {
      const match = line.trim().match(/^([a-f0-9]{64})\s+[* ]?(.+)$/i)
      if (!match) continue
      const payloadName = path.basename(match[2])
      const payload = index.get(payloadName)
      if (!payload) throw new Error(`${path.basename(file)} references missing ${payloadName}`)
      if (sha256(payload) !== match[1].toLowerCase()) {
        throw new Error(`${path.basename(file)} hash mismatch for ${payloadName}`)
      }
    }
  }
}

function verifyManifest(artifactRoot, manifestPath, index) {
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'))
  if (!manifest.version || typeof manifest.version !== 'string') {
    throw new Error('manifest version is missing')
  }
  const platforms = manifest.platforms
  if (!platforms || typeof platforms !== 'object' || !Object.keys(platforms).length) {
    throw new Error('manifest has no platform entries')
  }

  for (const [platform, entry] of Object.entries(platforms)) {
    if (platform.startsWith('darwin-')) {
      throw new Error(`macOS updater entry ${platform} is not allowed for the manual-update preview`)
    }
    const expectedArch = platform.startsWith('linux-') || platform.startsWith('windows-')
      ? 'x86_64'
      : null
    if (!expectedArch) throw new Error(`unknown platform key: ${platform}`)
    if (!entry || typeof entry.url !== 'string' || typeof entry.signature !== 'string') {
      throw new Error(`${platform} entry is incomplete`)
    }
    const url = new URL(entry.url)
    if (url.hostname !== 'github.com' || !url.pathname.includes('/releases/download/')) {
      throw new Error(`${platform} URL is not an official GitHub release URL: ${entry.url}`)
    }
    const name = path.basename(decodeURIComponent(url.pathname))
    if (!name || name.endsWith('.sig')) throw new Error(`${platform} URL is not a payload: ${entry.url}`)
    if (!name.toLowerCase().includes(expectedArch) && !(expectedArch === 'x86_64' && /amd64/i.test(name))) {
      throw new Error(`${platform} payload lacks an explicit x86_64/amd64 architecture: ${name}`)
    }
    const payload = index.get(name)
    if (!payload) throw new Error(`${platform} URL payload is missing locally: ${name}`)
    const signatureFile = index.get(`${name}.sig`)
    if (!signatureFile) throw new Error(`${platform} signature sidecar is missing: ${name}.sig`)
    const actualSignature = fs.readFileSync(signatureFile, 'utf8').trim()
    if (!actualSignature || actualSignature !== entry.signature.trim()) {
      throw new Error(`${platform} signature does not match ${name}`)
    }
    // Keep the root argument meaningful in diagnostics and guard against a
    // manifest accidentally pointing outside the artifact tree.
    if (!path.resolve(payload).startsWith(path.resolve(artifactRoot) + path.sep)) {
      throw new Error(`${platform} payload escaped artifact directory: ${name}`)
    }
  }
  return manifest
}

function main() {
  const suppliedRoot = process.argv[2]
  if (!suppliedRoot) throw new Error('usage: node tools/verify-release.js <artifact-dir> [manifest-path]')
  const artifactRoot = path.resolve(suppliedRoot)
  const manifestPath = path.resolve(process.argv[3] || path.join(artifactRoot, 'latest.json'))
  if (!artifactRoot || !fs.existsSync(artifactRoot)) throw new Error('artifact directory not found')
  if (!fs.existsSync(manifestPath)) throw new Error('latest.json not found')
  const files = filesUnder(artifactRoot)
  const index = byBaseName(files)
  const manifest = verifyManifest(artifactRoot, manifestPath, index)
  verifyChecksums(files, index)
  console.log(`release verification passed: ${manifest.version}, ${Object.keys(manifest.platforms).length} platform entries`)
}

try {
  main()
} catch (error) {
  fail(error.message)
}
