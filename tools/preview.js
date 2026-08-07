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
    const raw = JSON.parse(fs.readFileSync(path.join(dataDir, name), 'utf8'))
    // State files are version-wrapped: { schema_version, data }. Return the payload.
    if (raw && typeof raw === 'object' && Array.isArray(raw.data)) return raw.data
    return raw
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

// The incident view the real app receives: the disk record calls the lane
// `lane`, the canvas reads `hall`, and the captured request never leaves the
// backend — the UI sees only a `replayable` flag. Mapped here so the preview
// shows exactly what the webview would see.
const incidents = readJson('incidents.json', []).map((i) => ({
  at: i.at,
  hall: i.hall || i.lane || '',
  member: i.member,
  kind: i.kind,
  evidence: i.evidence,
  no_think: i.no_think,
  loopwatch: i.loopwatch,
  tools: i.tools,
  id: i.id || '',
  replayable: Boolean(i.replay && i.replay.body),
}))

const fixtures = {
  providers,
  lanes: readJson('lanes.json', []),
  pool: readJson('pool.json', []),
  catalog: readJson('catalog.json', []),
  stats: readJson('endpoint-stats.json', { fetched_at: 0, models: {} }),
  incidents,
}

// The real markup, restyled to absolute paths so the output can live
// anywhere, with the bridge injected ahead of app.js — the seam demands
// `window.vll` exists before the renderer's first line runs.
const indexHtml = fs.readFileSync(path.join(rendererDir, 'index.html'), 'utf8')
const appSource = fs.readFileSync(path.join(rendererDir, 'app.js'), 'utf8')
// The EGL skin is the active skin: index.html loads egl.css directly, and it
// must be inlined so the preview output can live anywhere. egl.js is inline
// only when present (older builds may not have it).
const readMaybe = (name) => {
  try { return fs.readFileSync(path.join(rendererDir, name), 'utf8') } catch { return null }
}
const eglStyle = readMaybe('egl.css')
const eglScript = readMaybe('egl.js')

const html = indexHtml
  .replace(/<link[^>]*href="egl\.css"[^>]*\/?>/i, eglStyle ? `<style>\n${eglStyle}</style>` : '')
  .replace(/<script src="egl\.js"><\/script>/i, eglScript ? `<script>\n${eglScript}</script>` : '')
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
        startDragging: () => ok(),
        providersList: () => ok(data.providers),
        providerSave: (input) => { console.log('[preview] providerSave', input); return ok({ ...input, id: input.id || 'preview', key_hint: '', has_key: false }) },
        providerDelete: () => ok(),
        providerTest: () => ok(0),
        catalogRead: () => ok({ models: data.catalog, errors: [] }),
        lanesRead: () => ok(data.lanes),
        lanesWrite: (lanes) => { data.lanes = lanes; return ok() },
        laneUnpark: (slug) => { const lane = data.lanes.find((l) => l.slug === slug); if (lane) { lane.parked = false; lane.parked_after = null; lane.budget_hits = [] } return ok() },
        poolRead: () => ok(data.pool),
        poolWrite: (ids) => { data.pool = ids; return ok() },
        statsRead: () => ok(data.stats),
        statsRefresh: () => ok(0),
        incidentsRead: () => ok(data.incidents),
        laneReplay: (id) => {
          // The engine replays server-side; the preview just records a
          // successful replay (and reports it) so the two-step flow can be
          // driven. The replayed request itself is never simulated — no
          // provider is contacted from a browser.
          console.log('[preview] laneReplay', id)
          return ok({ ok: true, status: 200, served_by: 'preview', trail: 'preview replay (no provider contacted)', message: 'the lane answered — see the trail for which member served it' })
        },
        editorList: () => ok(['VS Code', 'VS Code Insiders', 'Windsurf']),
        editorIntegrateLane: (slug, name, editor) => {
          console.log('[preview] editorIntegrateLane', { slug, name, editor })
          return ok({ editor, path: '/preview/' + editor + '/chatLanguageModels.json', written: true, error: null })
        },
        editorRemoveLane: (slug, editor) => {
          console.log('[preview] editorRemoveLane', { slug, editor })
          return ok({ editor, path: '/preview/' + editor + '/chatLanguageModels.json', written: true, error: null })
        },
        stateExport: () => ok('/tmp/preview-export.json'),
        stateImport: () => ok('/tmp/preview-export.json'),
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
          id: 'synthetic-1',
          hall: (data.lanes[0] || {}).slug || 'preview',
          member: 'preview/synthetic-member',
          kind: 'reasoning_burn',
          evidence: 'finish_reason: length — 379 of 600 tokens were hidden reasoning (synthetic preview event)',
          no_think: false,
          loopwatch: false,
          tools: 0,
          replayable: true,
        })
      }, 10000)
    })()
    </script>
    <script>\n${appSource}</script>`
  )

fs.writeFileSync(out, html)
console.log(`preview: ${out} (${fixtures.catalog.length} catalog models, ${fixtures.lanes.length} lanes, ${fixtures.incidents.length} incidents — keys masked)`)
