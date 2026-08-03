#!/usr/bin/env node
/**
 * Build a browser-openable preview of the real renderer, on real data.
 *
 * The renderer talks to the world through one seam: `window.__TAURI__` when
 * it exists, `window.vll` when it does not (see the top of app.js). Inside
 * the desktop app the bridge is Tauri; this script builds the OTHER side of
 * the seam — a `window.vll` backed by a snapshot of the app's own data
 * files — and emits a single HTML file any browser can open.
 *
 * What it is for: seeing and driving the actual UI outside the app window.
 * An assistant (or a CI screenshot job, or a designer) can open the output,
 * click the real buttons, and read the real console — which is exactly the
 * debugging that was impossible the night the issues pill shipped and the
 * only person who could click it was the user.
 *
 * What it is NOT: the app. Writes mutate in-memory state only, the engine
 * is not contacted, and the status bar reports itself as a preview.
 *
 * SECURITY, the part worth reading twice: providers.json holds API keys in
 * plaintext. This script never embeds them — providers pass through the
 * same masking the real app's ProviderView applies (a hint, never the key),
 * applied HERE, before anything touches the output. The output file still
 * embeds your lanes, catalog and incident evidence, so it goes wherever you
 * point it — never into the repository. The output path is a required
 * argument for exactly that reason: no default that someone could commit.
 *
 *     node tools/preview.js /tmp/somewhere/preview.html
 */

const fs = require('node:fs')
const path = require('node:path')
const os = require('node:os')

const out = process.argv[2]
if (!out) {
  console.error('usage: node tools/preview.js <output.html>   (output holds your lane/catalog data — keep it out of the repo)')
  process.exit(1)
}

const dataDir = path.join(os.homedir(), '.local', 'share', 'app.visualllm')
const rendererDir = path.resolve(__dirname, '..', 'renderer')

const readJson = (name, fallback) => {
  try {
    return JSON.parse(fs.readFileSync(path.join(dataDir, name), 'utf8'))
  } catch {
    return fallback
  }
}

// The same masking rule ProviderView applies in Rust: enough to recognise
// which key is in place, never enough to use it. Applied before the data
// can reach the template string below — the raw key's scope ends here.
const mask = (key) => {
  if (!key) return ''
  if (key.length > 8) return `${key.slice(0, 5)}…${key.slice(-4)}`
  return '•'.repeat(key.length)
}
const providers = readJson('providers.json', []).map((p) => ({
  id: p.id,
  name: p.name,
  kind: p.kind,
  base_url: p.base_url,
  key_hint: mask(p.key),
  has_key: Boolean(p.key),
}))

const fixtures = {
  providers,
  lanes: readJson('lanes.json', []),
  pool: readJson('pool.json', []),
  catalog: readJson('catalog.json', []),
  stats: readJson('endpoint-stats.json', { fetched_at: 0, models: {} }),
  incidents: readJson('incidents.json', []),
}

// The real markup, restyled to absolute paths so the output can live
// anywhere, with the bridge injected ahead of app.js — the seam demands
// `window.vll` exists before the renderer's first line runs.
const html = fs
  .readFileSync(path.join(rendererDir, 'index.html'), 'utf8')
  .replace('href="style.css"', `href="file://${path.join(rendererDir, 'style.css')}"`)
  .replace(
    '<script src="app.js"></script>',
    `<script>
    // The preview side of the renderer's one seam. Reads serve the snapshot;
    // writes mutate it in memory so the UI behaves; nothing leaves the page.
    (() => {
      // \\u003c-escaped so nothing in the data can spell a script-closing
      // tag and terminate this block mid-JSON. Two builds earned this
      // comment: the first died to markup inside a vendor description, and
      // the second died to a comment RIGHT HERE that named the closing tag
      // literally — a closer ends the tag from anywhere inside it, comments
      // included. Which is why this comment describes the tag without
      // spelling it.
      const data = ${JSON.stringify(fixtures).replace(/</g, '\\u003c')}
      const ok = (value) => Promise.resolve(value)
      window.vll = {
        readGateway: () => ok({
          connected: false, gateway: 'preview — engine not connected',
          error: 'preview harness', models: [], lanes: [],
          traffic: { requests: 0, failures: 0 },
        }),
        copy: (text) => { console.log('[preview] copy:', text); return ok() },
        minimize: () => ok(), toggleMaximize: () => ok(), close: () => ok(),
        providersList: () => ok(data.providers),
        providerSave: (input) => { console.log('[preview] providerSave', input); return ok({ ...input, id: input.id || 'preview', key_hint: '', has_key: false }) },
        providerDelete: () => ok(),
        providerTest: () => ok(0),
        catalogRead: () => ok({ models: data.catalog, errors: [] }),
        lanesRead: () => ok(data.lanes),
        lanesWrite: (lanes) => { data.lanes = lanes; return ok() },
        poolRead: () => ok(data.pool),
        poolWrite: (ids) => { data.pool = ids; return ok() },
        statsRead: () => ok(data.stats),
        statsRefresh: () => ok(0),
        incidentsRead: () => ok(data.incidents),
        portGet: () => ok(4100),
        portSet: (port) => ok(port),
      }

      // Ten seconds in, a synthetic failure arrives — so the notification
      // path (toast, fade, bell, center) can be watched and driven without
      // waiting for a real provider to have a bad day. Clearly labelled as
      // synthetic in its own evidence, because even test receipts are
      // receipts.
      setTimeout(() => {
        data.incidents.push({
          at: Math.floor(Date.now() / 1000),
          lane: (data.lanes[0] || {}).slug || 'preview',
          member: 'preview/synthetic-member',
          kind: 'reasoning_burn',
          evidence: 'finish_reason: length — 379 of 600 tokens were hidden reasoning (synthetic preview event)',
          no_think: false,
          loopwatch: false,
          tools: 0,
        })
      }, 10000)
    })()
    </script>
    <script src="file://${path.join(rendererDir, 'app.js')}"></script>`
  )

fs.writeFileSync(out, html)
console.log(`preview: ${out} (${fixtures.catalog.length} catalog models, ${fixtures.lanes.length} lanes, ${fixtures.incidents.length} incidents — keys masked)`)
