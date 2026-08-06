#!/usr/bin/env node
/**
 * Does app.js survive being loaded?
 *
 * The renderer registers every event listener at the top level. So a single
 * uncaught error partway down the file — a `const` read before its declaration
 * line is the easy one to write — stops execution there, and every listener
 * below it is silently never registered.
 *
 * What makes it nasty is that nothing looks broken. The HTML still renders, the
 * app comes up, the layout is perfect. Some buttons just do nothing, and which
 * ones depends on where in the file the error was. That is how "I can't add a
 * provider at all now" happened, and the real damage was every handler after
 * line 787.
 *
 * This runs the file against a stub DOM and asks one question: did it reach the
 * end? Nothing about behaviour — just that the script survives long enough to
 * wire itself up.
 *
 *     node tools/smoke.js
 */

const fs = require('node:fs')
const path = require('node:path')

const file = path.join(__dirname, '..', 'renderer', 'app.js')
const source = fs.readFileSync(file, 'utf8')

const noop = () => {}
const element = new Proxy({}, {
  get(_t, key) {
    switch (key) {
      case 'classList': return { add: noop, remove: noop, toggle: noop, contains: () => false }
      case 'dataset': case 'style': return {}
      case 'options': return []
      case 'querySelectorAll': return () => []
      case 'querySelector': case 'closest': return () => null
      case 'getBoundingClientRect': return () => ({ left: 0, top: 0, width: 0, height: 0, right: 0 })
      default: return typeof key === 'string' ? noop : undefined
    }
  },
  set: () => true,
})

const ok = (value) => Promise.resolve(value)
global.window = {
  vll: {
    readGateway: () => ok({
      connected: false,
      gateway: 'smoke',
      error: null,
      models: [],
      lanes: [],
      traffic: { requests: 0, failures: 0 },
    }),
    copy: () => ok(),
    minimize: () => ok(),
    toggleMaximize: () => ok(),
    close: () => ok(),
    startDragging: () => ok(),
    focus: () => ok(),
    providersList: () => ok([]),
    providerSave: (input) => ok({ ...input, id: input.id || 'smoke', key_hint: '', has_key: false }),
    providerDelete: () => ok(),
    providerTest: () => ok(0),
    catalogRead: () => ok({ models: [], errors: [] }),
    lanesRead: () => ok([]),
    lanesWrite: () => ok(),
    incidentsRead: () => ok([]),
    poolRead: () => ok([]),
    poolWrite: () => ok(),
    statsRead: () => ok({ fetched_at: 0, models: {} }),
    statsRefresh: () => ok(0),
    laneTest: () => ok({ ok: false, status: 0, served_by: null, trail: null, message: 'smoke' }),
    portGet: () => ok(4100),
    portSet: (port) => ok(port),
    editorList: () => ok(['VS Code', 'VS Code Insiders', 'Windsurf']),
    editorIntegrateLane: () => ok({ editor: 'smoke', path: '/smoke', written: true, error: null }),
    editorRemoveLane: () => ok({ editor: 'smoke', path: '/smoke', written: true, error: null }),
  },
}
global.document = new Proxy({}, {
  get(_t, key) {
    if (key === 'getElementById' || key === 'createElement' || key === 'querySelector') return () => element
    if (key === 'querySelectorAll') return () => []
    if (key === 'getSelection') return () => ({ selectAllChildren: noop })
    return noop
  },
})
global.setInterval = () => 0
global.setTimeout = () => 0
global.clearTimeout = noop

// Async work fires after the top-level pass and hits the stub's limits; that is
// not what is being tested here.
process.on('unhandledRejection', noop)
process.on('uncaughtException', noop)

try {
  new Function(`${source}\n;globalThis.__END__ = true;`)()
} catch (err) {
  console.error(`app.js threw while loading: ${err.constructor.name}: ${err.message}`)
  console.error('Every listener registered after that point is dead.')
  process.exit(1)
}

if (!globalThis.__END__) {
  console.error('app.js did not reach the end of the file.')
  process.exit(1)
}
console.log('smoke: app.js loads to completion — all listeners registered')
