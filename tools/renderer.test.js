/**
 * Unit tests for the pure logic in renderer/app.js.
 *
 * app.js wires its own DOM listeners at the top level, so the harness here is
 * the same stub environment tools/smoke.js uses. The difference is loading with
 * vm.runInThisContext, which hoists the file's top-level `function`
 * declarations onto the global object, and appends an export line so the tests
 * can reach the `const` bindings (state, COLUMNS) too.
 *
 *     npm test
 */

const { test, beforeEach } = require('node:test')
const assert = require('node:assert')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

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

process.on('unhandledRejection', noop)
process.on('uncaughtException', noop)

// Load app.js as a script (function declarations land on the global object)
// and export the const bindings the tests need.
const exportLine = `
globalThis.__t = {
  state,
  COLUMNS,
  DIAGNOSIS,
  pricePerMillion,
  fmtPrice,
  fmtAgo,
  domSlotToIndex,
  criterionValue,
  criterionRanks,
  scoreModels,
  visibleColumns,
  browseMatches,
  cloneLaneShape,
};`
vm.runInThisContext(`${source}\n;${exportLine}`, { filename: 'app.js' })
const t = globalThis.__t

const DEFAULT_BROWSE = {
  provider: '',
  search: '',
  sorts: [{ field: 'intelligence', desc: true }],
  scores: new Map(),
  author: '',
  context: 0,
  price: '',
  filters: { vision: false, tools: false, reasoning: false, structured: false, rated: false, pooled: false },
}

function resetBrowse(catalog) {
  t.state.browse = {
    provider: '',
    search: '',
    sorts: [{ field: 'intelligence', desc: true }],
    scores: new Map(),
    author: '',
    context: 0,
    price: '',
    filters: { ...DEFAULT_BROWSE.filters },
  }
  t.state.catalog = catalog
}

beforeEach(() => {
  resetBrowse([])
})

// ---------------------------------------------------------------------- prices

test('pricePerMillion: free is zero, unpublished and negative are null', () => {
  assert.equal(t.pricePerMillion({ free: true, price_out: 0 }), 0)
  assert.equal(t.pricePerMillion({ free: false, price_out: null }), null)
  assert.equal(t.pricePerMillion({ free: false, price_out: -1 }), null)
  assert.equal(t.pricePerMillion({ free: false, price_out: 0.000002 }), 2)
})

test('fmtPrice: labels the tiers without fabricating figures', () => {
  assert.equal(t.fmtPrice({ free: true, price_out: 0 }), 'free')
  assert.equal(t.fmtPrice({ free: false, price_out: null }), null)
  assert.equal(t.fmtPrice({ price_out: -0.5 }), 'variable')
  assert.equal(t.fmtPrice({ free: false, price_out: 0.000003 }), '$3.00/M')
  assert.equal(t.fmtPrice({ free: false, price_out: 0.0000005 }), '$0.500/M')
})

// ---------------------------------------------------------------------- age

test('fmtAgo: seconds, minutes, then hours', () => {
  const now = Math.round(Date.now() / 1000)
  assert.equal(t.fmtAgo(now - 5), '5s ago')
  assert.equal(t.fmtAgo(now - 90), '2m ago')
  assert.equal(t.fmtAgo(now - 7200), '2h ago')
})

// ------------------------------------------------------------------- dragging

test('domSlotToIndex: display order reverses into chip index', () => {
  // count == slot: far-right slot, past the last chip → primary (index 0)
  assert.equal(t.domSlotToIndex(3, 3), 0)
  // slot 0: far-left gap → last fallback
  assert.equal(t.domSlotToIndex(0, 3), 3)
  assert.equal(t.domSlotToIndex(1, 2), 1)
  // Clamp, never out of bounds.
  assert.equal(t.domSlotToIndex(9, 2), 0)
  assert.equal(t.domSlotToIndex(-3, 2), 2)
})

// ---------------------------------------------------------------- criteria math

test('criterionValue: price/price_in scaled to per-million, zero means unpublished', () => {
  assert.equal(t.criterionValue({ free: true }, 'price'), 0)
  assert.equal(t.criterionValue({ free: false, price_out: 0.000002 }, 'price'), 2)
  assert.equal(t.criterionValue({ free: false, price_out: null }, 'price'), null)
  assert.equal(t.criterionValue({ free: true }, 'price_in'), 0)
  assert.equal(t.criterionValue({ free: false, price_in: 0.000001 }, 'price_in'), 1)
  assert.equal(t.criterionValue({ free: false, price_in: -1 }, 'price_in'), null)
  assert.equal(t.criterionValue({ context: 128000 }, 'context'), 128000)
  assert.equal(t.criterionValue({ context: 0 }, 'context'), null)
  assert.equal(t.criterionValue({ intelligence: 77 }, 'intelligence'), 77)
})

test('criterionRanks: percentile band, 1 best and 0 worst across judged models', () => {
  const models = [
    { id: 'fast', intelligence: 90 },
    { id: 'mid', intelligence: 60 },
    { id: 'slow', intelligence: 30 },
  ]
  const ranks = t.criterionRanks(models, [{ field: 'intelligence', desc: true }])
  assert.equal(ranks.intelligence.get('fast'), 1)
  assert.equal(ranks.intelligence.get('slow'), 0)
  assert.ok(ranks.intelligence.get('mid') > 0 && ranks.intelligence.get('mid') < 1)
})

test('scoreModels: one criterion is just a sort; average keeps order', () => {
  const models = [
    { id: 'a', intelligence: 100, context: 1000 },
    { id: 'b', intelligence: 50, context: 5000 },
    { id: 'c', intelligence: 0, context: 9000 },
  ]
  const byIq = t.scoreModels(models, [{ field: 'intelligence', desc: true }])
  assert.equal(byIq.get('a'), 1)
  assert.equal(byIq.get('c'), 0)
  assert.ok(byIq.get('a') > byIq.get('b') && byIq.get('b') > byIq.get('c'))

  const both = t.scoreModels(models, [
    { field: 'intelligence', desc: true },
    { field: 'context', desc: true },
  ])
  // b (mid in both) beats a (worst context) and c (worst iq): the average of
  // (0.5, 0.5) = 0.5, a is (1, 0) = 0.5 too — equal average, order decided by
  // the caller. What the average guarantees is that neither dimension vanished.
  assert.ok(both.get('b') >= 0.5)
  assert.equal(both.get('a') + both.get('b'), 1) // 0.5 + 0.5
})

test('scoreModels: a model without a figure for a locked criterion scores null, never zero', () => {
  const models = [
    { id: 'measured', intelligence: 90 },
    { id: 'mystery', intelligence: null },
  ]
  const scores = t.scoreModels(models, [{ field: 'intelligence', desc: true }])
  assert.equal(scores.get('measured'), 1)
  assert.equal(scores.get('mystery'), null)
})

// ------------------------------------------------------------------- columns

test('visibleColumns: a column earns its place by carrying data', () => {
  const models = [
    { id: 'x', context: 128000, price_out: 0.000002 },
    { id: 'y', context: 32768, free: true },
  ]
  resetBrowse(models)
  t.state.browse.sorts = [] // no lock: only columns with real values remain
  const shown = t.visibleColumns(models).map(([key]) => key)
  assert.ok(shown.includes('context'))
  assert.ok(shown.includes('price_in')) // y is free → 0 is a judgement
  assert.ok(!shown.includes('intelligence')) // nobody publishes it here
})

test('visibleColumns: a locked column always stays, even with no data', () => {
  resetBrowse([{ id: 'bare', context: 1024 }])
  t.state.browse.sorts = [
    { field: 'intelligence', desc: true },
    { field: 'context', desc: true },
  ]
  const shown = t.visibleColumns([{ id: 'bare', context: 1024 }]).map(([key]) => key)
  assert.ok(shown.includes('intelligence'))
  assert.ok(shown.includes('context'))
})

// ------------------------------------------------------------------- browsing

test('browseMatches: provider, name and context filters narrow the catalog', () => {
  resetBrowse([
    { id: 'acme/a', provider_id: 'acme', name: 'Alpha', context: 8192, free: true },
    { id: 'acme/b', provider_id: 'acme', name: 'Beta', context: 128000, free: false, price_out: 0.00001 },
    { id: 'zeta/a', provider_id: 'zeta', name: 'Alpha', context: 8192, free: true },
  ])
  t.state.browse.provider = 'acme'
  let ids = t.browseMatches().map((m) => m.id)
  assert.deepEqual(ids, ['acme/a', 'acme/b'])

  t.state.browse.provider = ''
  t.state.browse.search = 'beta'
  assert.deepEqual(t.browseMatches().map((m) => m.id), ['acme/b'])

  t.state.browse.search = ''
  t.state.browse.context = 65536
  assert.deepEqual(t.browseMatches().map((m) => m.id), ['acme/b'])
})

test('browseMatches: price "0" keeps only free models, a ceiling filters by per-million', () => {
  resetBrowse([
    { id: 'f', free: true },
    { id: 'cheap', free: false, price_out: 0.0000005 }, // $0.50/M
    { id: 'pricey', free: false, price_out: 0.00001 },  // $10/M
  ])
  t.state.browse.price = '0'
  assert.deepEqual(t.browseMatches().map((m) => m.id), ['f'])

  t.state.browse.price = '1'
  // Neither publishes intelligence, so both are unjudgeable and fall back to
  // id order — 'cheap' sorts before 'f'.
  assert.deepEqual(t.browseMatches().map((m) => m.id), ['cheap', 'f'])

  t.state.browse.price = ''
  t.state.browse.filters.rated = true
  assert.deepEqual(t.browseMatches().map((m) => m.id), [])
})

test('browseMatches: text-only sorts order by lock chain, name tiebreak', () => {
  resetBrowse([
    { id: 'zeta', name: 'Zeta', provider_name: 'Acme' },
    { id: 'alpha', name: 'Alpha', provider_name: 'Zeta' },
  ])
  t.state.browse.sorts = [{ field: 'name', desc: false }]
  assert.deepEqual(t.browseMatches().map((m) => m.id), ['alpha', 'zeta'])
})

test('browseMatches: criteria sorts rank by score, unjudgeable sinks last', () => {
  resetBrowse([
    { id: 'smart', intelligence: 100 },
    { id: 'dumb', intelligence: 10 },
    { id: 'mystery', intelligence: null },
  ])
  t.state.browse.sorts = [{ field: 'intelligence', desc: true }]
  assert.deepEqual(t.browseMatches().map((m) => m.id), ['smart', 'dumb', 'mystery'])
})

// ---------------------------------------------------------------- diagnosis

test('auto_parked diagnosis exists and offers unpark only while parked', () => {
  const d = t.DIAGNOSIS.auto_parked
  assert.ok(d, 'the engine writes auto_parked; the renderer must explain it')
  assert.ok(d.title && d.why && d.advice, 'title, why and advice are the contract')
  // The fix fires only while the lane is still parked — once unparked there is
  // nothing to unpark, and the button must disappear.
  assert.equal(d.fix({}, { parked: true }), 'unpark')
  assert.equal(d.fix({}, { parked: false }), null)
  assert.equal(d.fix({}, null), null)
})

test('auto_parked advice carries the engine receipt', () => {
  const d = t.DIAGNOSIS.auto_parked
  assert.match(d.advice({ evidence: '3 budgetable failures within the last 600s' }, {}), /3 budgetable failures/)
})

test('every incident kind the engine writes has a diagnosis', () => {
  const kinds = [
    'reasoning_burn', 'empty_response', 'midstream_error', 'rate_limited',
    'out_of_credit', 'key_rejected', 'model_missing', 'capability_gap',
    'context_overflow', 'provider_trouble', 'unreachable', 'stalled',
    'loop_repeat', 'loop_futile', 'loop_sweep', 'request_rejected',
    'skipped_by_catalog', 'auto_parked', 'unattributed',
  ]
  for (const kind of kinds) {
    assert.ok(t.DIAGNOSIS[kind], `DIAGNOSIS.${kind} is missing`)
  }
})

// --------------------------------------------------------------- lane cloning
// The clone contract is the whole feature: what carries over (the definition)
// and what deliberately does not (live state, editor integration). These four
// cases pin both sides so a future tweak to cloneLaneShape cannot silently
// start duplicating a parked flag or a credential-bearing member list.

test('cloneLaneShape: a clone copies the definition, order and dials intact', () => {
  const source = {
    slug: 'hallway',
    name: 'Hallway',
    members: [
      { provider: 'alpha', id: 'a-1', params: { temperature: 0.2 }, disabled: false },
      { provider: 'beta', id: 'b-2', params: { max_tokens: 4000 }, disabled: true },
    ],
    criteria: [{ key: 'price', direction: 'min' }],
    suppress_reasoning: true,
    unstick: false,
    budget: { failures: 10, window_secs: 3600 },
  }
  const clone = t.cloneLaneShape(source, new Set(['hallway']))

  assert.equal(clone.slug, 'hallway-copy')
  assert.equal(clone.name, 'Hallway copy')
  assert.equal(clone.members.length, 2)
  // Order is fallback order: the clone must read exactly like the original.
  assert.deepEqual(clone.members[0], source.members[0])
  assert.equal(clone.members[1].disabled, true, 'park state of each member rides along')
  assert.deepEqual(clone.criteria, source.criteria)
  assert.equal(clone.suppress_reasoning, true)
  assert.deepEqual(clone.budget, { failures: 10, window_secs: 3600 })
})

test('cloneLaneShape: dials and criteria are deep copies, not references', () => {
  const source = { slug: 's', name: 'S', members: [{ provider: 'a', id: '1', params: { t: 1 } }], criteria: [{ key: 'p' }] }
  const clone = t.cloneLaneShape(source, new Set(['s']))
  // Mutating the clone after the fact must never reach back into the source.
  clone.members[0].params.t = 99
  clone.criteria[0].key = 'mutated'
  assert.equal(source.members[0].params.t, 1, 'turning a dial on the clone leaves the original alone')
  assert.equal(source.criteria[0].key, 'p')
})

test('cloneLaneShape: fresh slug stays unique among the lanes it joins', () => {
  const source = { slug: 'hallway', name: 'Hallway', members: [] }
  assert.equal(t.cloneLaneShape(source, new Set(['hallway', 'hallway-copy'])).slug, 'hallway-copy-2')
  assert.equal(t.cloneLaneShape(source, new Set(['hallway', 'hallway-copy', 'hallway-copy-2'])).slug, 'hallway-copy-3')
})

test('cloneLaneShape: no editor integration and no live park state on the clone', () => {
  const source = {
    slug: 'hallway', name: 'Hallway', members: [],
    integrated_editors: ['codex', 'claude'], parked: true, budget_hits: 3,
  }
  const clone = t.cloneLaneShape(source, new Set(['hallway']))
  assert.deepEqual(clone.integrated_editors, [], 'integration stays a deliberate per-lane act')
  assert.equal(clone.parked, undefined, 'the clone is not born parked')
  assert.equal(clone.budget_hits, undefined, 'failure history does not ride along')
})
