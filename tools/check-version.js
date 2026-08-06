#!/usr/bin/env node
/**
 * The release version lives in three files: Cargo.toml (the crate), tauri.conf.json
 * (the bundle), and package.json (the npm metadata). When they drift, the release
 * job ships a binary whose window title, app ID metadata, and crate version disagree.
 *
 * This reads all three and fails loudly on any mismatch, so a bump in one file alone
 * is caught by CI instead of making it to a tag.
 *
 *     node tools/check-version.js
 */

const fs = require('node:fs')
const path = require('node:path')

const root = path.join(__dirname, '..')
const fail = (msg) => {
  console.error(`version check: ${msg}`)
  process.exit(1)
}

const tauri = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri', 'tauri.conf.json'), 'utf8'))
const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))

const cargo = fs.readFileSync(path.join(root, 'src-tauri', 'Cargo.toml'), 'utf8')
const cargoVersion = cargo.match(/^\s*version\s*=\s*"([^"]+)"/m)
if (!cargoVersion) fail('no version found in src-tauri/Cargo.toml')
const versions = {
  'src-tauri/Cargo.toml': cargoVersion[1],
  'src-tauri/tauri.conf.json': tauri.version,
  'package.json': pkg.version,
}

const distinct = new Set(Object.values(versions))
if (distinct.size > 1) {
  for (const [file, version] of Object.entries(versions)) {
    console.error(`  ${file}: ${version}`)
  }
  fail('versions disagree')
}
if (!/^\d+\.\d+\.\d+$/.test(versions['package.json'])) {
  fail(`unexpected version shape: ${versions['package.json']}`)
}

console.log(`version check: ${versions['package.json']} across all three files`)
