/* VisualLLM — canvas logic.
 *
 * Two rules drive everything below:
 *   1. A hall is an ordered list of models. members[0] answers first.
 *   2. The procession draws that list right to left, so the primary sits at the
 *      right-hand edge where the eye lands last and the arrow points.
 *
 * The reversal lives in exactly two places — `renderTrack` and `domSlotToIndex`
 * — and nowhere else touches it. */

// ---------------------------------------------------------------------- bridge

const T = window.__TAURI__
const api = T
  ? {
      readGateway: () => T.core.invoke('read_gateway'),
      copy: (text) => T.core.invoke('copy_text', { text }),
      minimize: () => T.window.getCurrentWindow().minimize(),
      toggleMaximize: () => T.window.getCurrentWindow().toggleMaximize(),
      close: () => T.window.getCurrentWindow().close(),
      startDragging: () => T.window.getCurrentWindow().startDragging(),
      focus: () => T.window.getCurrentWindow().setFocus(),

      providersList: () => T.core.invoke('providers_list'),
      providerSave: (input) => T.core.invoke('provider_save', { input }),
      providerDelete: (id) => T.core.invoke('provider_delete', { id }),
      providerTest: (kind, baseUrl, key) =>
        T.core.invoke('provider_test', { kind, baseUrl, key }),
      catalogRead: (id) => T.core.invoke('catalog_read', { id: id ?? null }),
      lanesRead: () => T.core.invoke('lanes_read'),
      lanesWrite: (lanes) => T.core.invoke('lanes_write', { lanes }),
      incidentsRead: () => T.core.invoke('incidents_read'),
      poolRead: () => T.core.invoke('pool_read'),
      poolWrite: (ids) => T.core.invoke('pool_write', { ids }),
      statsRead: () => T.core.invoke('stats_read'),
      statsRefresh: () => T.core.invoke('stats_refresh'),
      laneTest: (slug) => T.core.invoke('lane_test', { slug }),
      activityRead: (since) => T.core.invoke('activity_read', { since }),
      portGet: () => T.core.invoke('port_get'),
      portSet: (port) => T.core.invoke('port_set', { port }),
      vscodeIntegrateLane: (slug, name) => T.core.invoke('vscode_integrate_lane', { slug, name }),
    }
  : window.vll

// Wrap every API invocation so the UI does not crash on a rejected promise.
// Any exception is logged and stored into `state.error` for display.
Object.keys(api).forEach((k) => {
  const orig = api[k]
  api[k] = async function (...args) {
    try {
      return await orig.apply(this, args)
    } catch (err) {
      console.error('[vll] api.%s failed', k, err)
      try {
        state.error = err && err.message ? err.message : String(err)
      } catch (_) {
        // best-effort: don't let an error while reporting an error crash the UI
      }
      throw err
    }
  }
})

// ----------------------------------------------------------------------- state

const state = {
  connected: false,
  error: null,
  models: [],
  lanes: [],
  traffic: { requests: 0, failures: 0 },
  gateway: '',
  updatedAt: null,
  search: '',
  sort: 'name',
  filters: { free: false, vision: false, tools: false },

  providers: [],
  catalog: [],
  incidents: [],       // engine-recorded failures, with receipts
  activity: [],        // live per-request feed: trying / answered / failed
  activitySeenAt: 0,   // high-water mark for the activity poll
  stats: {},
  statsFetchedAt: 0,
  pool: [],             // model ids the user kept — the sidebar shows only these
  browse: {             // the browser's own controls, separate from the sidebar's
    provider: '',        // provider_id, '' for all
    search: '',
    sorts: [{ field: 'intelligence', desc: true }],  // locked columns = criteria
    scores: new Map(),
    author: '',
    context: 0,
    price: '',
    filters: { vision: false, tools: false, reasoning: false, structured: false, rated: false, pooled: false },
  },
  catalogErrors: [],
  counts: {},          // provider id -> models found, or an error string
  editing: null,       // provider id being edited, null when adding
}

const $ = (id) => document.getElementById(id)

/** One-shot undo for destructive hall edits. A delete or drag-out is a
 *  mis-click away from losing a tuned hall, and re-dragging members back
 *  (with their dials) is real work. The snapshot is taken just before the
 *  change and held for one toast window; acting on it restores and clears. */
let pendingUndo = null

function armUndo(lanes, label) {
  pendingUndo = { lanes, label }
  clearTimeout(armUndo._t)
  armUndo._t = setTimeout(() => (pendingUndo = null), 6000)
}

function takeUndo() {
  const undo = pendingUndo
  pendingUndo = null
  clearTimeout(armUndo._t)
  return undo
}

/** The engine's address. The old Python gateway lived on 4000 and is only
 *  polled for the status bar now; lanes are SERVED from here. */
let enginePort = 4100
const engineHost = () => `127.0.0.1:${enginePort}`

// ------------------------------------------------------------------- identity
//
// A model's identity is (provider, id), not the id alone. Two providers can
// carry the same id — deepseek-chat direct and through a reseller, llama3 on
// Ollama and LM Studio — and everything that stores or compares models has to
// keep them apart. Pool entries and hall members are refs: { provider, id }.
// An EMPTY provider is a ref from before this rule existed and matches the
// first catalog entry with that id, which is exactly the old behaviour.

/** Anything old files might hold — a bare id string — becomes a ref. The
 *  member's dials (`params`) ride along; dropping them here would silently
 *  reset a tuned member every time the app restarted. */
const asRef = (r) =>
  typeof r === 'string'
    ? { provider: '', id: r, params: {}, disabled: false }
    : { provider: r.provider || '', id: r.id, params: r.params || {}, disabled: !!r.disabled }

/** Does this member carry any dial at all? Drives the gear's "tuned" dot. */
const hasParams = (ref) => Object.values(ref.params || {}).some((v) => v != null)

/** Does this ref mean this catalog model? Empty provider is a wildcard. */
const refMatches = (ref, model) =>
  ref.id === model.id && (!ref.provider || ref.provider === (model.provider_id || ''))

/** Everything the sidebar can offer: what the gateway runs, plus every
 *  provider catalog. Gateway lanes win a name collision — they carry live
 *  health and measured throughput, which a catalog entry never will. */
function allModels() {
  const seen = new Set(state.models.map((m) => m.id))
  return state.models.concat(state.catalog.filter((m) => !seen.has(m.id)))
}

const modelByRef = (provider, id) => {
  const all = allModels()
  return (
    all.find((m) => m.id === id && (m.provider_id || '') === (provider || '')) ||
    (!provider ? all.find((m) => m.id === id) : undefined)
  )
}

const poolHas = (model) => state.pool.some((r) => refMatches(r, model))

/** Fill in the provider on refs written before providers were part of
 *  identity, once the catalog can say which provider that was. Nothing is
 *  saved here — the next natural save persists it. */
function qualifyRefs() {
  const fill = (r) => {
    if (r.provider) return
    const m = state.catalog.find((m) => m.id === r.id)
    if (m) r.provider = m.provider_id
  }
  state.pool.forEach(fill)
  state.lanes.forEach((hall) => hall.members.forEach(fill))
}

const slugify = (s) =>
  s.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'hall'

function fmtContext(n) {
  if (!n) return '—'
  if (n >= 1_000_000) return `${(n / 1_048_576).toFixed(1).replace(/\.0$/, '')}M`
  return `${Math.round(n / 1024)}K`
}

function fmtTps(v) {
  if (v == null) return null
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} tok/s`
}

function health(model) {
  if (model.source === 'catalog') return null // a catalog says nothing about health
  if (!model.available) return 'bad'
  return model.healthy ? 'ok' : 'warn'
}

/** Per-token prices are unreadable. Everyone reasons in dollars per million.
 *
 * OpenRouter prices its meta-routers (`openrouter/auto`, `fusion`) at -1, which
 * means "whatever the model it picks costs". Multiplying that by a million
 * produced `$-1000000.000/M`, and negative sorts as cheapest — so the junk sat
 * at the top of the one view where price matters most. */
function pricePerMillion(model) {
  if (model.free) return 0
  if (model.price_out == null || model.price_out < 0) return null
  return model.price_out * 1e6
}

function fmtPrice(model) {
  if (model.price_out != null && model.price_out < 0) return 'variable'
  const perMillion = pricePerMillion(model)
  if (perMillion == null) return null
  if (perMillion === 0) return 'free'
  return `$${perMillion < 1 ? perMillion.toFixed(3) : perMillion.toFixed(2)}/M`
}

const SORT_LABEL = { intelligence: 'iq', coding: 'code', agentic: 'agent' }

// ---------------------------------------------------------------------- icons

const ICON = {
  copy: '<svg viewBox="0 0 16 16"><rect x="5.5" y="5.5" width="8" height="8" rx="1.6"/><path d="M10.5 5.5V4a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 4v5A1.5 1.5 0 0 0 4 10.5h1.5"/></svg>',
  close: '<svg viewBox="0 0 12 12"><path d="M3.5 3.5l5 5M8.5 3.5l-5 5"/></svg>',
  arrow: '<svg viewBox="0 0 16 16"><path d="M3 8h9M8.5 4.5L12 8l-3.5 3.5"/></svg>',
  gear: '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="2.1"/><path d="M8 2.6v1.9M8 11.5v1.9M2.6 8h1.9M11.5 8h1.9M4.2 4.2l1.3 1.3M10.5 10.5l1.3 1.3M11.8 4.2l-1.3 1.3M5.5 10.5l-1.3 1.3"/></svg>',
  brain: '<svg viewBox="0 0 16 16"><path d="M8 2.5a2.6 2.6 0 0 0-2.6 2.6c-1.5.3-2.4 1.5-2.4 3 0 1 .5 1.9 1.2 2.4-.1.3-.2.7-.2 1 0 1.6 1.4 2.9 3 2.9.5 0 1-.1 1.4-.4.3.2.6.3 1 .3a2.9 2.9 0 0 0 2.9-2.9c0-.3 0-.7-.2-1 .8-.5 1.3-1.4 1.3-2.4 0-1.5-1-2.7-2.4-3A2.6 2.6 0 0 0 8 2.5z"/></svg>',
  loop: '<svg viewBox="0 0 16 16"><path d="M3 8a5 5 0 0 1 8.5-3.5M13 8a5 5 0 0 1-8.5 3.5M11.5 2v2.5H9M4.5 14v-2.5H7"/></svg>',
}

// ------------------------------------------------------------------ rendering

/** More than one provider in play means every model needs to say whose it is. */
const multiProvider = () => new Set(state.catalog.map((m) => m.provider_id)).size > 1

function chipEl(model, { inTrack = false, ordinal = null, tuned = false, disabled = false } = {}) {
  const el = document.createElement('div')
  el.className = 'relic'
  el.dataset.model = model.id
  el.dataset.provider = model.provider_id || ''
  el.dataset.class = model.klass
  if (inTrack) el.classList.add('in-track')
  if (ordinal === 1) el.classList.add('is-primary')
  if (disabled) {
    el.classList.add('is-parked')
    el.title = 'Parked — keeps its place and dials, but the hall skips it at request time. Right-click to resume.'
  }

  // Cost first. It is the one figure that matters on every single decision,
  // and it should not be something the eye has to hunt for behind a speed
  // reading. Gateway lanes have no price at all — the gateway reports health
  // and throughput and nothing about money — so those simply start at speed.
  const bits = []
  const price = fmtPrice(model)
  if (price) bits.push(price)
  // Gateway lanes measure throughput themselves as `tps`; catalog entries carry
  // the provider's figure as `throughput`. Same number, two sources.
  const speed = fmtTps(model.tps ?? model.throughput)
  if (speed) bits.push(speed)
  bits.push(fmtContext(model.context))
  // Show whatever the list is currently ranked by, so the ordering is legible
  // rather than something you have to take on trust.
  // Show whatever the list is currently ranked by, so the ordering is legible
  // rather than something you have to take on trust. Only 107 of OpenRouter's
  // 337 models carry benchmark scores, so saying "unrated" is far better than
  // leaving a relic looking identical to a scored one.
  if (SORT_LABEL[state.sort]) {
    const ranked = model[state.sort]
    bits.push(ranked == null ? `${SORT_LABEL[state.sort]} unrated` : `${SORT_LABEL[state.sort]} ${Math.round(ranked)}`)
  }
  // Which provider serves this relic — but only once there are two to confuse.
  if (model.provider_name && multiProvider()) bits.push(model.provider_name)
  if (disabled) bits.push('parked')
  if (model.source !== 'catalog' && !model.available && model.reason) bits.push(model.reason)

  if (model.description) el.title = model.description

  const dot = health(model)
  el.innerHTML = `
    <span class="relic-bar"></span>
    ${ordinal ? `<span class="ordinal">${ordinal}</span>` : ''}
    <span class="relic-body">
      <span class="relic-name">${
        // Track chips are narrow, and the vendor prefix is the least
        // distinguishing part of an id: truncated, "nvidia/nemotron-3-nano"
        // and "nvidia/nemotron-3-super" both read "nvidia/nemotron-…". In a
        // hall, spend the pixels on the part that tells members apart; the
        // sidebar is wide enough to keep the full id.
        inTrack && model.id.includes('/') ? model.id.split('/').slice(1).join('/') : model.id
      }</span>
      <span class="relic-meta">${bits
        .map((b, i) => (i ? `<span class="sep">·</span>${b}` : b))
        .join(' ')}</span>
    </span>
    ${dot ? `<span class="relic-health ${dot}" title="${dot}"></span>` : ''}
    ${inTrack ? `<button class="relic-gear${tuned ? ' tuned' : ''}" title="${
      tuned ? 'This member has its own settings' : 'Member settings'
    }">${ICON.gear}</button>` : ''}
    ${inTrack ? `<button class="relic-remove" title="Remove">${ICON.close}</button>` : ''}
  `
  return el
}

/** A hall member nothing in any catalog explains — deleted upstream, from a
 *  removed provider, or (historically) an old-gateway slug that was never a
 *  model. It cannot be dragged, because there is no model to place; it can
 *  only be removed, and it says why it is dead so the fix is obvious. */
function deadChipEl(ref, ordinal) {
  const el = document.createElement('div')
  el.className = 'relic in-track is-dead'
  el.dataset.model = ref.id
  el.dataset.provider = ref.provider || ''
  el.title = 'No configured provider offers this id. The engine will skip or fail it.'
  el.innerHTML = `
    <span class="relic-bar"></span>
    ${ordinal ? `<span class="ordinal">${ordinal}</span>` : ''}
    <span class="relic-body">
      <span class="relic-name">${ref.id}</span>
      <span class="relic-meta">not in any provider's catalog</span>
    </span>
    <button class="relic-remove" title="Remove">${ICON.close}</button>
  `
  return el
}

/** Missing is not zero. A model with no published score sorts to the bottom
 *  rather than pretending to be the worst one measured. */
function byDescending(field) {
  return (a, b) => {
    const x = a[field]
    const y = b[field]
    if (x == null && y == null) return a.id.localeCompare(b.id)
    if (x == null) return 1
    if (y == null) return -1
    return y - x
  }
}

function renderSidebar() {
  const list = $('modelList')
  const q = state.search.toLowerCase()

  // Only what the user kept. Browsing happens in its own panel; this is the
  // working set you drag from.
  //
  // Gateway entries are excluded outright. They are the OLD gateway's own
  // endpoints — readouts, not provider models — and were only ever in this
  // list because the sidebar predates the pool. Left visible they invite the
  // one drag that can never work. Their health figures still feed the status
  // bar, which is where a readout belongs.
  let models = allModels().filter((m) => {
    if (m.source === 'gateway') return false
    if (!poolHas(m)) return false
    if (q && !m.id.toLowerCase().includes(q) && !(m.name || '').toLowerCase().includes(q)) {
      return false
    }
    if (state.filters.free && !m.free) return false
    if (state.filters.vision && !m.vision) return false
    if (state.filters.tools && !m.tools) return false
    return true
  })

  if (state.sort === 'name') {
    models.sort((a, b) => a.id.localeCompare(b.id))
  } else if (state.sort === 'price') {
    // Unpriced and variable-priced models go last. They are not cheap, they are
    // unknown, and putting them first buries the answer to the question asked.
    models.sort((a, b) => {
      const x = pricePerMillion(a)
      const y = pricePerMillion(b)
      if (x == null && y == null) return a.id.localeCompare(b.id)
      if (x == null) return 1
      if (y == null) return -1
      return x - y
    })
  } else {
    models.sort(byDescending(state.sort === 'speed' ? 'tps' : state.sort))
  }

  list.innerHTML = ''
  if (!models.length) {
    const hasPool = state.pool.length
    list.innerHTML = `<div class="empty-state"><strong>${
      hasPool ? 'No matches' : 'Your pool is empty'
    }</strong>${
      hasPool
        ? 'Loosen the search or filters.'
        : 'Browse models and add the ones you want.'
    }</div>`
  } else {
    // The full OpenRouter catalog is hundreds of rows; building every relic up
    // front costs more than anyone can scroll. Cap it and say so.
    const CAP = 300
    models.slice(0, CAP).forEach((m) => list.appendChild(chipEl(m)))
    if (models.length > CAP) {
      list.insertAdjacentHTML(
        'beforeend',
        `<div class="empty-state">${models.length - CAP} more — narrow the search to see them.</div>`
      )
    }
  }
  $('modelCount').textContent = `${models.length} model${models.length === 1 ? '' : 's'}`
}

function renderTrack(procession, hall) {
  procession.innerHTML = ''
  procession.appendChild(Object.assign(document.createElement('div'), { className: 'flow' }))

  if (!hall.members.length) {
    procession.insertAdjacentHTML(
      'beforeend',
      `<div class="procession-empty"><span class="niche"></span><span>Drop a model here — it becomes the one that answers.</span></div>`
    )
    return
  }

  // members[0] answers first, so it is drawn last: right-hand edge.
  ;[...hall.members].reverse().forEach((ref, domIndex) => {
    const ordinal = hall.members.length - domIndex
    const model = modelByRef(ref.provider, ref.id)
    // A member no catalog can explain still renders — as visibly dead, with
    // its remove button. Skipping it left a hall trying (and failing on) a
    // model its own canvas refused to show, which is undebuggable from the UI.
    procession.appendChild(
      model
        ? chipEl(model, { inTrack: true, ordinal, tuned: hasParams(ref), disabled: !!ref.disabled })
        : deadChipEl(ref, ordinal)
    )
  })

  procession.insertAdjacentHTML(
    'beforeend',
    `<span class="answers-first">${ICON.arrow} answers first</span>`
  )
}

function laneEndpoint(slug) {
  return `http://${engineHost()}/lane/${slug}/v1`
}

function laneCurlExample(slug) {
  return `curl ${laneEndpoint(slug)}/chat/completions \\\n  -H 'Content-Type: application/json' \\\n  -d '{"model":"${slug}","messages":[{"role":"user","content":"Hello"}]}'`
}

/** The live story for one hall, from the activity feed. `trying` is only
 *  shown while fresh — a stale "trying" from a crashed poll would read as a
 *  request in flight forever, so it decays. `answered` and `failed` linger
 *  long enough to be noticed after the fact. */
function laneLive(hall) {
  const now = Date.now() / 1000
  const entries = state.activity
    .filter((e) => e.hall === hall.slug)
    .sort((a, b) => b.at - a.at)
  if (!entries.length) return null

  const trying = entries.find((e) => e.phase === 'trying')
  const settled = entries.find((e) => e.phase === 'answered' || e.phase === 'failed' || e.phase === 'exhausted')

  if (trying && trying.at > now - 30 && (!settled || trying.at >= settled.at)) {
    return { kind: 'trying', text: `trying ${shortMember(trying.member)}…`, title: 'A request is walking this hall now' }
  }
  if (settled && settled.at > now - 45) {
    if (settled.phase === 'answered') {
      const passed = (settled.detail.match(/passed over (\d+)/) || [])[1]
      const suffix = passed && passed !== '0' ? ` · ${passed} passed over` : ''
      return { kind: 'answered', text: `answered by ${shortMember(settled.member)}${suffix}`, title: attr(settled.detail) }
    }
    if (settled.phase === 'exhausted') {
      return { kind: 'failed', text: 'every member was skipped or failed', title: attr(settled.detail) }
    }
    return { kind: 'failed', text: `${shortMember(settled.member)} failed`, title: attr(settled.detail) }
  }
  return null
}

/** `id@provider` is precise but wide; the activity line has pixels for the
 *  model name, and the full label stays on hover. */
function shortMember(label) {
  if (!label) return ''
  const id = label.split('@')[0]
  return id.includes('/') ? id.split('/').slice(1).join('/') : id
}

/** The hall footer: a slim status line that carries the words the lights only
 *  hint at. Live activity first, then the skip warnings, then what the hall was
 *  built for. Clicking it opens the hall's trail. A quiet hall shows one faint
 *  line, never nothing — an empty footer reads as broken. */
function laneFoot(hall) {
  const live = laneLive(hall)
  const dead = hall.members.filter((ref) => !modelByRef(ref.provider, ref.id)).length
  const parked = hall.members.filter((ref) => ref.disabled).length
  const criteria = (hall.criteria || []).map(criterionWords).join(' + ')
  const issues = state.incidents.filter((i) => i.hall === hall.slug).length

  const parts = []
  if (live) parts.push(`<span class="foot-live is-${live.kind}">${live.text}</span>`)
  if (dead) parts.push(`<span class="foot-dead">${dead} member${dead === 1 ? '' : 's'} not in any catalog — skipped at request time</span>`)
  if (parked) parts.push(`<span class="foot-parked">${parked} parked</span>`)
  if (issues) parts.push(`<span class="foot-issues">${issues} issue${issues === 1 ? '' : 's'} in the last 24h</span>`)
  if (criteria) parts.push(`<span class="foot-criteria">${attr(criteria)}</span>`)

  const body = parts.length
    ? parts.join('<span class="foot-sep">·</span>')
    : '<span class="foot-quiet">No recent activity — this hall is ready.</span>'
  return `<span class="lane-activity" title="Open this hall's trail">${body}</span>`
}

/** The indicator lights in a hall's header. Health is shown, not told: a row
 *  of small lamps that light up, each with its meaning on hover. Live activity
 *  pulses coral; answered glows green; a failure is red; dead members warn
 *  amber; parked members light a steady neutral lamp. A lamp that is off stays
 *  a faint recess — present, never noisy. */
function laneLights(hall) {
  const live = laneLive(hall)
  const dead = hall.members.filter((ref) => !modelByRef(ref.provider, ref.id)).length
  const parked = hall.members.filter((ref) => ref.disabled).length

  const lamp = (cls, on, title) =>
    `<span class="lamp ${cls}${on ? ' on' : ''}" title="${attr(title)}"></span>`

  const liveState = live ? live.kind : null
  return `<span class="hall-lamps">
    ${lamp('lamp-live', liveState === 'trying', liveState === 'trying' ? live.text : 'No request in flight')}
    ${lamp('lamp-ok', liveState === 'answered', liveState === 'answered' ? live.text : 'Nothing served recently')}
    ${lamp('lamp-bad', liveState === 'failed', liveState === 'failed' ? live.text : 'No recent failure')}
    ${lamp('lamp-warn', dead > 0, dead ? `${dead} member${dead === 1 ? '' : 's'} not in any catalog — skipped at request time` : 'Every member is in a catalog')}
    ${lamp('lamp-park', parked > 0, parked ? `${parked} member${parked === 1 ? '' : 's'} parked — skipped, keeping place and dials` : 'No parked members')}
  </span>`
}

function laneEl(hall) {
  const el = document.createElement('article')
  el.className = 'hall'
  el.dataset.hall = hall.slug

  const head = document.createElement('div')
  head.className = 'hall-head'
  head.innerHTML = `
    <span class="hall-name" contenteditable="plaintext-only" spellcheck="false">${hall.name}</span>
    ${laneLights(hall)}
    <button class="hall-url" title="Copy endpoint URL">
      ${ICON.copy}<span class="host">${engineHost()}</span><span>/lane/${hall.slug}/v1</span>
    </button>
    <span class="hall-spacer"></span>
    <button class="hall-act lane-test" title="Test this hall">Test</button>
    <button class="hall-act lane-copy-setup" title="Copy a curl setup example">Setup</button>
    <button class="hall-act lane-vscode" title="Add this hall to VS Code model picker">VS Code</button>
    <button class="hall-toggle${hall.suppress_reasoning ? ' is-on' : ''}" data-toggle="think" title="${
      hall.suppress_reasoning
        ? 'No thinking: members are asked to answer directly. Click to allow thinking.'
        : 'Thinking allowed: members may reason before answering. Click to ask them not to.'
    }">${ICON.brain}</button>
    <button class="hall-toggle lane-unstick${hall.unstick ? ' is-on' : ''}" data-toggle="unstick" title="${
      hall.unstick
        ? 'Loopwatch on: stuck tool-call loops are collapsed and noted. Click to turn off.'
        : 'Loopwatch off. Click to watch for stuck tool-call loops.'
    }">${ICON.loop}</button>
    <button class="hall-remove" title="Delete hall">${ICON.close}</button>
  `

  const procession = document.createElement('div')
  procession.className = 'procession'
  renderTrack(procession, hall)

  const foot = document.createElement('div')
  foot.className = 'hall-foot'
  foot.innerHTML = laneFoot(hall)

  el.append(head, procession, foot)
  return el
}

function renderLanes() {
  const host = $('lanes')

  // A refresh rebuilds the DOM, which would otherwise throw every procession back to
  // the left. Remember where each one was so the view does not jump under you.
  const scrolls = new Map()
  host.querySelectorAll('.hall').forEach((el) => {
    const procession = el.querySelector('.procession')
    if (procession) scrolls.set(el.dataset.hall, procession.scrollLeft)
  })

  host.innerHTML = ''
  if (!state.lanes.length) {
    host.innerHTML = `<div class="empty-state onboarding-empty">
      <strong>Build your first endpoint</strong>
      <span>Add a provider, choose models for your pool, then drag them here.</span>
      <span class="empty-explain">The model on the right answers first. Models to its left are fallbacks.</span>
      <button class="btn-primary empty-action" type="button" id="emptyNewLane">Create a hall</button>
    </div>`
    $('emptyNewLane').addEventListener('click', () => $('newLane').click())
    return
  }
  state.lanes.forEach((hall) => host.appendChild(laneEl(hall)))

  // A procession long enough to scroll starts at its right-hand end: the model that
  // answers first is the one worth seeing, and it lives at that edge.
  host.querySelectorAll('.hall').forEach((el) => {
    const procession = el.querySelector('.procession')
    if (!procession) return
    const previous = scrolls.get(el.dataset.hall)
    procession.scrollLeft = previous === undefined ? procession.scrollWidth : previous
    updateScrollFade(procession)
    if (!procession.dataset.fadeWired) {
      procession.dataset.fadeWired = '1'
      procession.addEventListener('scroll', () => updateScrollFade(procession), { passive: true })
    }
  })
}

/** The left-edge fade appears only when members hide off that side. */
function updateScrollFade(procession) {
  procession.classList.toggle('can-scroll-left', procession.scrollLeft > 2)
}

function renderStatusBar() {
  const models = state.models
  const healthy = models.filter((m) => m.healthy && m.available).length
  const host = (state.gateway || '').replace(/^https?:\/\//, '')

  $('barDot').className = `dot ${state.connected ? 'ok' : 'bad'}`
  $('barGateway').textContent = state.connected ? host : state.error || 'gateway offline'

  $('statModels').innerHTML = models.length
    ? `<b>${models.length}</b> <span class="unit">models</span> · <b>${healthy}</b> <span class="unit">healthy</span>`
    : '<span class="unit">no models</span>'

  // Fastest is worth a slot because it is the number that decides routing —
  // and it is measured here, not advertised by anyone.
  const fastest = models
    .filter((m) => m.tps != null && m.available)
    .sort((a, b) => b.tps - a.tps)[0]
  $('statFastest').innerHTML = fastest
    ? `<span class="unit">fastest</span> <b>${fastest.id}</b> ${fmtTps(fastest.tps)}`
    : '<span class="unit">no throughput measured yet</span>'

  const { requests = 0, failures = 0 } = state.traffic || {}
  const rate = requests ? ((failures / requests) * 100).toFixed(1) : '0.0'
  $('statTraffic').innerHTML = requests
    ? `<b>${requests.toLocaleString()}</b> <span class="unit">requests</span> · <b>${failures}</b> <span class="unit">failed (${rate}%)</span>`
    : '<span class="unit">no traffic yet</span>'
  $('statPort').textContent = `engine ${engineHost()}`
}

function renderUpdated() {
  const el = $('statUpdated')
  if (!state.updatedAt) return (el.textContent = '')
  const secs = Math.round((Date.now() - state.updatedAt) / 1000)
  el.textContent = secs < 2 ? 'updated just now' : `updated ${secs}s ago`
}

function render() {
  renderStatusBar()
  renderUpdated()
  renderSidebar()
  renderLanes()
}

// --------------------------------------------------------------- drag and drop
//
// WHY THIS IS HAND-WRITTEN RATHER THAN THE BROWSER'S BUILT-IN DRAG
//
// HTML has a drag-and-drop API. It is nearly twenty years old, it renders a
// blurry screenshot of the element as the drag image with no way to style it,
// it fires events in an order that differs between browsers, and it cannot be
// made to work on touch. It looks cheap, and this is the central interaction of
// the product.
//
// So we use POINTER EVENTS instead — one API that covers mouse, trackpad, pen
// and touch identically. Three things happen:
//
//   pointerdown  remember what was grabbed; build a floating copy (the "ghost")
//   pointermove  move the ghost; work out where it would land; draw the marker
//   pointerup    commit the change, or put everything back
//
// The element you grabbed never actually moves. It dims in place while a copy
// follows the cursor, and on release the underlying data changes and everything
// is redrawn from that. Moving the real element around the DOM mid-drag is how
// you end up with flickering and lost drops.

const drag = {
  active: false,
  model: null,
  from: null, // hall slug, or null when it came from the sidebar
  ghost: null,
  line: null,
  target: null,
  slot: 0,
  offsetX: 0,
  offsetY: 0,
}

/** Start a drag: build the ghost and start listening for movement. */
function beginDrag(event, relic) {
  const model = modelByRef(relic.dataset.provider, relic.dataset.model)
  if (!model) return

  // `closest` walks UP the tree looking for a match. If the relic is inside a
  // hall we are moving it; if not, it came from the sidebar and we are copying,
  // since a model can appear in many lanes at once.
  const laneEl = relic.closest('.hall')

  // The element's exact position and size on screen right now, in pixels from
  // the top-left of the window. Needed so the ghost appears precisely over the
  // real relic instead of jumping to the cursor.
  const rect = relic.getBoundingClientRect()

  drag.active = true
  drag.model = model
  drag.from = laneEl ? laneEl.dataset.hall : null
  // Where inside the relic you grabbed it. Without this the ghost snaps its
  // corner to the cursor, and a relic grabbed by its right edge jumps left the
  // instant you move. Small detail; it is most of what makes dragging feel
  // solid rather than cheap.
  drag.offsetX = event.clientX - rect.left
  drag.offsetY = event.clientY - rect.top

  const ghost = chipEl(model, { inTrack: false })
  ghost.classList.add('ghost')
  ghost.style.width = `${rect.width}px`
  ghost.style.left = `${rect.left}px`
  ghost.style.top = `${rect.top}px`
  document.body.appendChild(ghost)
  drag.ghost = ghost

  relic.classList.add('is-source')
  document.body.classList.add('is-dragging')

  // Listening on `window`, not on the relic. A fast drag outpaces the browser's
  // hit-testing, and events land on whatever is under the cursor instead. Watch
  // the whole window and the drag survives being flung around.
  //
  // `{ once: true }` removes the listener automatically after it fires — one
  // less thing to leak.
  window.addEventListener('pointermove', onDragMove)
  window.addEventListener('pointerup', endDrag, { once: true })
}

function onDragMove(event) {
  if (!drag.active) return
  drag.ghost.style.left = `${event.clientX - drag.offsetX}px`
  drag.ghost.style.top = `${event.clientY - drag.offsetY}px`

  // What is under the cursor right now. The ghost would answer this question
  // about itself, which is why it is styled `pointer-events: none` — the cursor
  // passes straight through it as though it were not there.
  const under = document.elementFromPoint(event.clientX, event.clientY)

  // `?.` means "only if the thing on the left exists". Over empty space
  // `elementFromPoint` returns nothing, and without this the whole drag would
  // die on an error.
  const procession = under?.closest?.('.procession')

  document.querySelectorAll('.hall.is-target').forEach((l) => l.classList.remove('is-target'))
  drag.line?.remove()
  drag.line = null

  if (!procession) {
    drag.target = null
    return
  }

  procession.closest('.hall').classList.add('is-target')
  drag.target = procession

  // Which gap in this procession are we hovering over?
  //
  // Compare the cursor against each relic's MIDPOINT, not its edges. Past the
  // halfway line means you intend to land after it. Using edges leaves dead
  // zones between chips where nothing highlights, which reads as broken.
  //
  // The dragged relic itself is excluded (`:not(.is-source)`) — it is about to
  // move, so it should not count as an obstacle to itself.
  const chips = [...procession.querySelectorAll('.relic:not(.is-source)')]
  let slot = chips.length // default: past everything, at the far right
  for (let i = 0; i < chips.length; i++) {
    const niche = chips[i].getBoundingClientRect()
    if (event.clientX < niche.left + niche.width / 2) {
      slot = i
      break
    }
  }
  drag.slot = slot

  const line = document.createElement('div')
  line.className = 'drop-line'
  const trackBox = procession.getBoundingClientRect()
  let x
  if (!chips.length) x = 18
  else if (slot >= chips.length) {
    const last = chips[chips.length - 1].getBoundingClientRect()
    x = last.right - trackBox.left + 4
  } else {
    x = chips[slot].getBoundingClientRect().left - trackBox.left - 5
  }
  line.style.left = `${x + procession.scrollLeft}px`
  procession.appendChild(line)
  drag.line = line
}

/**
 * Convert a position on screen into a position in the data.
 *
 * This is the one place the display order and the storage order have to be
 * reconciled, and it is worth being slow about.
 *
 * The DATA is a plain list where `members[0]` answers first:
 *
 *     members  = [ A, B, C ]        A is primary
 *
 * The SCREEN draws that list reversed, so the primary sits at the right-hand
 * edge under the arrow:
 *
 *     screen   =   C    B    A  →  answers first
 *     slots    =  0    1    2    3
 *
 * A slot is a GAP, so there is always one more slot than chips. Dropping at
 * slot 3 (far right, past A) must make the new model primary — index 0. Dropping
 * at slot 0 (far left) makes it the last fallback — index 3.
 *
 * Both are `count - slot`. That is the whole conversion.
 */
function domSlotToIndex(slot, count) {
  return Math.max(0, Math.min(count, count - slot))
}

function endDrag() {
  window.removeEventListener('pointermove', onDragMove)
  document.body.classList.remove('is-dragging')
  document.querySelectorAll('.hall.is-target').forEach((l) => l.classList.remove('is-target'))
  drag.ghost?.remove()
  drag.line?.remove()

  const { model, from, target, slot } = drag
  drag.active = false
  drag.ghost = drag.line = drag.target = null

  // A gateway relic is a READOUT — one of the old gateway's own hall slugs,
  // carried in the sidebar for its live health figures. It is not a provider
  // model, and sent upstream as one it fails as a bad model id. If the catalog
  // has a real model under the same id, the drop means that one; otherwise the
  // drop is refused with a reason rather than accepted and broken.
  let placed = model
  if (model.source === 'gateway') {
    const twin = state.catalog.find((m) => m.id === model.id)
    if (twin) placed = twin
    else if (target) {
      toast(`${model.id} is a gateway readout, not a provider model — browse and add models to build lanes`)
      render()
      return
    }
  }

  // The ref this drag is about: this model, from this provider. A member
  // moved out of a hall keeps its dials — the settings belong to the member,
  // and reordering a hall must not amount to resetting it.
  const prior = from
    ? state.lanes
        .find((l) => l.slug === from)
        ?.members.find((r) => refMatches(r, placed))
    : null
  const dragged = { provider: placed.provider_id || '', id: placed.id, params: prior?.params || {} }

  if (!target) {
    // Dropped nowhere. Out of a hall means remove; out of the sidebar means nothing.
    if (from) {
      const hall = state.lanes.find((l) => l.slug === from)
      mutateLanes(`${model.id} removed from ${hall.name}`, () => {
        hall.members = hall.members.filter((r) => !refMatches(r, model))
      })
    } else {
      render()
    }
    return
  }

  const hall = state.lanes.find((l) => l.slug === target.closest('.hall').dataset.hall)
  const source = from ? state.lanes.find((l) => l.slug === from) : null

  if (source) source.members = source.members.filter((r) => !refMatches(r, model))
  const without = hall.members.filter((r) => !refMatches(r, model))
  const index = domSlotToIndex(slot, without.length)
  without.splice(index, 0, dragged)
  hall.members = without

  // A hall takes on the question its first models were found by. Only once —
  // a later drag from a different search should not silently rewrite what the
  // hall says it is for.
  if (!hall.criteria?.length && state.browse.sorts?.length) {
    hall.criteria = state.browse.sorts
      .filter((s) => !NON_CRITERIA.has(s.field))
      .map(({ field, desc }) => ({ field, desc }))
  }

  // The first member is the product moment: the hall just became a live
  // endpoint, and the next step is connecting a client. Celebrate once — the
  // moment is earned exactly one time per hall — with the URL and what to do
  // with it. Later drops just confirm the ordering.
  const firstEverMember = hall.members.length === 1 && !hall._celebrated
  if (firstEverMember) {
    hall._celebrated = true
    toast(
      `${hall.name} is live`,
      `${laneEndpoint(hall.slug)} — point any OpenAI-compatible client here. Setup copies a curl example, VS Code adds it to the model picker.`
    )
  } else if (index === 0) {
    toast(`${model.id} answers first in ${hall.name}`)
  }
  render()
  saveLanes()
}

// ------------------------------------------------------------- notifications
//
// The engine records failures with receipts; this is where they become
// explanations, delivered the way a person can actually absorb them: a
// notification slides in at the bottom right and waits to be clicked. Click
// it and it counts as viewed. Let it fade, and the bell in the status bar
// lights up and keeps the score. Open the bell and everything in it is
// marked seen. Any type can be ignored — the engine keeps recording it, the
// delivery just goes silent. Facts are never suppressed; only their
// announcement is.
//
// The diagnosis contract, in order of importance:
//
//   1. EVIDENCE FIRST. Every diagnosis renders the provider's own bytes (or
//      the engine's counts) beside its conclusion. No receipts, no verdict.
//   2. MECHANISM, NOT LABEL. "Spent its budget thinking" is a label; WHY a
//      thinking model returns an empty answer is an explanation someone can
//      reason from next time.
//   3. PRESCRIPTION, NOT SHRUG. Where the fix is a control in this app, name
//      it — and when it is one of the hall's own toggles, offer the click.
//   4. HONEST ATTRIBUTION. A malformed request is the client's fault and
//      says so. A failure the evidence cannot attribute renders as
//      "unexplained", receipts attached, no blame invented.
//
// This exists because the people this app is for choose free models, and
// free models earn reputations by rumour. Receipts beat rumours.

/** How old an incident can be and still count as news. */
const ISSUE_WINDOW_S = 24 * 3600

const windowedIncidents = () => {
  const cutoff = Date.now() / 1000 - ISSUE_WINDOW_S
  return state.incidents.filter((i) => i.at >= cutoff)
}

/** An incident's identity, since the engine stores facts, not ids. Good
 *  enough: two failures in the same second, on the same member, of the same
 *  kind, are the same news. */
const incidentKey = (i) => `${i.at}|${i.member}|${i.kind}`

/**
 * Read receipts and mutes live in localStorage, not in the engine's files —
 * deliberately. "Which explanations has this person already seen" is
 * interface state: losing it costs one extra glance at old news, never a
 * fact. The facts stay in incidents.json, which this layer only ever reads.
 */
/** localStorage, when the surface actually has one. The smoke stub does not,
 *  and file:// pages in some engines refuse access — either way the fallback
 *  is an in-memory map, which degrades to "read receipts last one session"
 *  rather than to a crash that kills every listener below this line. */
const receipts = (() => {
  try {
    localStorage.setItem('notif.probe', '1')
    localStorage.removeItem('notif.probe')
    return localStorage
  } catch {
    const memory = new Map()
    return { getItem: (k) => memory.get(k) ?? null, setItem: (k, v) => memory.set(k, v) }
  }
})()

const notif = {
  /** Only incidents newer than the moment the app opened get a live toast.
   *  The backlog still counts as unread on the bell — it was never viewed —
   *  but a wall of toasts about yesterday is noise, not news. */
  baseline: Date.now() / 1000,
  toasted: new Set(),
  read: new Set(JSON.parse(receipts.getItem('notif.read') || '[]')),
  muted: new Set(JSON.parse(receipts.getItem('notif.muted') || '[]')),
}

function notifPersist() {
  // Read receipts for incidents that have aged out of the window are dead
  // weight; prune by the timestamp each key carries in its prefix.
  const cutoff = Date.now() / 1000 - ISSUE_WINDOW_S * 2
  const kept = [...notif.read].filter((key) => Number(key.split('|')[0] || 0) >= cutoff)
  notif.read = new Set(kept)
  receipts.setItem('notif.read', JSON.stringify(kept))
  receipts.setItem('notif.muted', JSON.stringify([...notif.muted]))
}

/** Kinds that stay recorded and listed but never badge the bell or fire a
 *  toast: by-design behavior, not news. Capability skips are the hall doing
 *  its job — announcing them trained users to ignore alerts entirely. */
const SILENT_KINDS = new Set(['skipped_by_catalog'])

const unreadIncidents = () =>
  windowedIncidents().filter(
    (i) => !SILENT_KINDS.has(i.kind) && !notif.read.has(incidentKey(i)) && !notif.muted.has(i.kind)
  )

function renderBell() {
  const badge = $('bellBadge')
  const count = unreadIncidents().length
  badge.hidden = count === 0
  badge.textContent = count > 99 ? '99+' : String(count)
  $('notifBell').classList.toggle('is-lit', count > 0)
}

/** Called on every incidents poll: toast what is genuinely new, then let the
 *  bell recount. Muted kinds pass through silently — recorded, not announced. */
function processIncidents() {
  for (const incident of windowedIncidents()) {
    const key = incidentKey(incident)
    if (incident.at < notif.baseline) continue
    if (notif.toasted.has(key)) continue
    notif.toasted.add(key)
    if (SILENT_KINDS.has(incident.kind)) continue
    if (notif.muted.has(incident.kind)) continue
    if (notif.read.has(key)) continue
    showNotifToast(incident)
  }
  renderBell()
}

/** One live notification, bottom right. Click = viewed, and the center opens
 *  on the full diagnosis. The ✕ also counts as viewed — closing something is
 *  handling it. Fading away unclicked is the only path that leaves it unread. */
function showNotifToast(incident) {
  const d = DIAGNOSIS[incident.kind] || DIAGNOSIS.unattributed
  const card = document.createElement('div')
  card.className = 'notif-toast'
  card.innerHTML = `
    <span class="notif-toast-body">
      <span class="notif-toast-title">${d.title}</span>
      <span class="notif-toast-meta">${attr(incident.member)} · hall ${attr(incident.hall)}</span>
    </span>
    <button class="notif-toast-close" title="Dismiss">${ICON.close}</button>
  `
  const stack = $('notifStack')
  stack.appendChild(card)

  const key = incidentKey(incident)
  const done = () => {
    card.remove()
    renderBell()
  }
  // The fade timer, pausable: a notification exists to be read, and a timer
  // must never win a race against a reader who is hovering over it.
  let timer = setTimeout(done, 8000)
  card.addEventListener('mouseenter', () => clearTimeout(timer))
  card.addEventListener('mouseleave', () => (timer = setTimeout(done, 4000)))

  card.addEventListener('click', (event) => {
    clearTimeout(timer)
    notif.read.add(key)
    notifPersist()
    card.remove()
    if (!event.target.closest('.notif-toast-close')) openNotifications()
    renderBell()
  })
}

function openNotifications(laneSlug) {
  notifLaneFilter = laneSlug || null
  $('notifScrim').hidden = false
  renderNotifCenter()
  // Opening the center is viewing it: everything listed is now seen, and the
  // bell goes quiet. Muted kinds were never counted to begin with.
  for (const incident of windowedIncidents()) notif.read.add(incidentKey(incident))
  notifPersist()
  renderBell()
}

function closeNotifications() {
  $('notifScrim').hidden = true
  notifLaneFilter = null
}

/** When opened from a hall's activity line, the center shows just that hall.
 *  `null` is the whole system. */
let notifLaneFilter = null

function renderNotifCenter() {
  const list = $('notifList')

  const visible = notifLaneFilter
    ? windowedIncidents().filter((i) => i.hall === notifLaneFilter)
    : windowedIncidents()

  // Newest first; identical (member, kind) entries fold into one card with a
  // count — fifty loop sweeps as fifty cards would bury the one rate-limit
  // entry that matters.
  const grouped = new Map()
  for (const incident of visible) {
    const key = `${incident.member} ${incident.kind}`
    const group = grouped.get(key)
    if (group) {
      group.count += 1
      if (incident.at > group.latest.at) group.latest = incident
    } else {
      grouped.set(key, { count: 1, latest: incident })
    }
  }
  const groups = [...grouped.values()].sort((a, b) => b.latest.at - a.latest.at)

  const scopeChip = notifLaneFilter
    ? `<div class="notif-scope">Lane: ${attr(notifLaneFilter)} <button class="notif-scope-clear" type="button">show all</button></div>`
    : ''

  if (!groups.length) {
    list.innerHTML = scopeChip + `<div class="empty-state"><strong>Nothing to report</strong>${
      notifLaneFilter ? 'No failures on this hall in the last 24 hours.' : 'No failures recorded in the last 24 hours.'
    }</div>`
    return
  }

  list.innerHTML = scopeChip + groups
    .map(({ count, latest }) => {
      const d = DIAGNOSIS[latest.kind] || DIAGNOSIS.unattributed
      const hall = state.lanes.find((l) => l.slug === latest.hall)
      const muted = notif.muted.has(latest.kind)
      const fix = !muted && hall && d.fix && d.fix(latest, hall)
      return `
      <article class="issue${muted ? ' is-muted' : ''}">
        <header class="issue-head">
          <span class="issue-title">${d.title}</span>
          <span class="issue-meta">${attr(latest.member)} · hall ${attr(latest.hall)} · ${fmtAgo(latest.at)}${
            count > 1 ? ` · ×${count}` : ''
          }</span>
        </header>
        <pre class="issue-evidence">${attr(latest.evidence)}</pre>
        <p class="issue-why">${d.why(latest, hall)}</p>
        <p class="issue-advice">${d.advice(latest, hall)}</p>
        <div class="issue-actions">
          ${fix === 'no-think' ? `<button class="btn-ghost issue-fix" data-fix="no-think" data-hall="${attr(latest.hall)}">Turn on “no thinking” for this hall</button>` : ''}
          <button class="btn-ghost issue-mute" data-mute="${attr(latest.kind)}">${
            muted ? 'Ignored — click to restore' : 'Ignore this type'
          }</button>
        </div>
      </article>`
    })
    .join('')
}

/** What each kind means and what to do about it. `why` and `advice` receive
 *  the incident (with the lane-toggle state AT THE TIME) plus the hall as it
 *  is NOW — the difference decides the advice: "turn thinking off" is only
 *  advice when it wasn't already off when the failure happened. */
const DIAGNOSIS = {
  reasoning_burn: {
    title: 'Spent the answer budget on hidden thinking',
    why: () =>
      'This model reasons before it answers, and reasoning spends the same token budget as the answer. ' +
      'When the budget dies mid-thought, the visible reply never starts — a chat client renders that as an empty response.',
    advice: (i, hall) =>
      i.no_think
        ? 'This happened with “no thinking” already on — the provider ignored the knob. Published capability rosters are optimistic: a setting can be listed and still dropped upstream. The commit gate kept this from reaching the client; if it keeps happening, this member is a poor fit for lanes that need prompt answers.'
        : 'Turn on “no thinking” for this hall. It asks the provider to skip reasoning entirely — verified to stop exactly this on models that honour the knob.',
    fix: (i, hall) => (!i.no_think && !hall.suppress_reasoning ? 'no-think' : null),
  },
  empty_response: {
    title: 'Answered with nothing a client could show',
    why: (i) =>
      `The stream ended cleanly with no visible content and no tool call${
        i.tools ? ` — under a ${i.tools}-tool request, which is a heavy ask for small models` : ''
      }. The commit gate caught it and moved to the next member instead of forwarding a blank reply.`,
    advice: () =>
      'Keep at least one member behind this one — the gate turned this failure into a fallback. If it recurs mainly on tool-heavy requests, this member handles large tool sets poorly and belongs further left in the hall.',
  },
  midstream_error: {
    title: 'The provider sent an error inside a success',
    why: () =>
      'Some providers return HTTP 200 and then deliver the actual failure as an event inside the stream — invisible to status-code handling. The gate reads the stream before trusting it, caught this, and walked on.',
    advice: (i) =>
      /rate.?limit|free/i.test(i.evidence)
        ? 'The quoted message names a rate limit. A per-minute limit clears on its own; a per-day limit only clears at reset. Members on different providers dodge per-provider throttles; nothing but waiting fixes an account-wide one.'
        : 'Read the quoted message — the provider named its own problem. If it recurs, that is this provider’s reliability speaking, not the model’s ability.',
  },
  rate_limited: {
    title: 'Rate limited',
    why: () =>
      'The provider refused with 429. There are two species: this provider throttling you (another member fixes it — exactly what a hall is for) and an account-wide block (only waiting fixes it). They arrive with the same status code.',
    advice: () =>
      'Members spread across different providers dodge per-provider throttles. VisualLLM reads the error body when it can; if the receipt names an account-wide free-tier limit, waiting for reset is usually the only fix.',
  },
  out_of_credit: {
    title: 'Out of credit',
    why: () => 'The provider refused for billing reasons — the case fallback lanes were invented for.',
    advice: () => 'The hall walked on if members remained. Check the provider account if this member should be working.',
  },
  key_rejected: {
    title: 'API key rejected',
    why: () => 'The provider refused the stored key. This is configuration, not the model.',
    advice: () => 'Open Providers and use the pencil on this member’s provider to re-enter the key. Test before saving.',
  },
  model_missing: {
    title: 'Model not found at the provider',
    why: () => 'The provider no longer serves this id — retired, renamed, or never carried on this endpoint.',
    advice: () => 'Browse the provider’s catalog and replace this relic with the current id.',
  },
  capability_gap: {
    title: 'The catalog promised what the endpoint refused',
    why: () =>
      'Published capability lists are a union across every provider serving a model — optimistic by construction. The endpoint actually reached said no. The engine treats this as the model’s limitation and walks on.',
    advice: () =>
      'If the capability matters to this hall, put a member that natively supports it ahead of this one. The refusal is per-endpoint, so the same model via another provider may genuinely differ.',
  },
  context_overflow: {
    title: 'Request larger than this member’s window',
    why: () => 'The prompt did not fit. That is a ceiling, not an error — a later member with a bigger window can still serve it.',
    advice: () => 'Order the hall so a large-window member sits behind the fast ones. The engine already skips members whose published window is clearly too small.',
  },
  provider_trouble: {
    title: 'Provider-side trouble',
    why: () => 'A 5xx or busy signal — the provider’s problem, not the model’s ability.',
    advice: () => 'Transient unless it repeats. Recurring entries here are a provider reliability record, useful when deciding whose free tier to lean on.',
  },
  unreachable: {
    title: 'Could not reach the provider',
    why: () => 'Connection failed or timed out before any response existed.',
    advice: () => 'Check the base URL in Providers if this recurs — for local servers, that the server is running.',
  },
  // The engine no longer records capability skips as incidents — a member
  // passed over for a request it can't serve is the hall working as designed,
  // and badging it trained users to ignore the bell. This entry remains only
  // so records written by older versions still explain themselves; new ones
  // are never announced (see SILENT_KINDS).
  skipped_by_catalog: {
    title: 'Skipped without being contacted',
    why: () =>
      'The cached catalog said this member could not serve what the request needed, so the engine never sent it — sending anyway risks a silently wrong answer (an ignored image reads as a confident description of nothing).',
    advice: () =>
      'This is the hall doing its job. If the catalog looks wrong, refresh it from Browse — a wrong cached entry is exactly the failure this record exists to expose.',
  },
  stalled: {
    title: 'The connection went silent before answering',
    why: () =>
      'Bytes stopped arriving entirely — not a slow model (a stream carries a pulse even while thinking), but a dead connection wearing an open port. The engine waited out the full patience window, then gave the slot to the next member.',
    advice: () =>
      'Transient unless it repeats for the same member — then that provider\u2019s endpoint is unreliable or the model is being retired mid-flight. A member that stalls often belongs further left in the hall, behind something dependable.',
  },
  loop_repeat: {
    title: 'Caught repeating the same call',
    why: () =>
      'The model re-issued an identical, already-answered tool call — pattern-matching its own transcript, where a run of identical calls reads as the thing to do next.',
    advice: () =>
      'Loopwatch collapsed the redundant pairs and named the loop at the tail of the conversation, which is the treatment that measured best. Recurring entries at high counts are this model losing the plot at depth — a capability fact worth knowing, with the numbers right here.',
  },
  loop_futile: {
    title: 'Caught burning calls that taught it nothing',
    why: () =>
      'Different arguments, byte-identical results — a chunked read walking off a file’s end, or a batch of calls all failing the same way. Being stuck is not repeating yourself; it is receiving no new information.',
    advice: () =>
      'Loopwatch quoted the repeated result back to the model — the cause is always in the bytes it kept receiving. Both live catches tonight escaped after the note; entries that do not escape are the model ignoring evidence, which is worth knowing about a model.',
  },
  loop_sweep: {
    title: 'Old duplicate calls swept from the conversation',
    why: () =>
      'Clients resend their whole transcript every turn, dead duplicates included. Nothing was looping now — this just removed the residue so the model reads a smaller, cleaner history.',
    advice: () => 'No action needed; this is maintenance, recorded so the collapse counts are never mysterious.',
  },
  request_rejected: {
    title: 'Not this model’s fault',
    why: () =>
      'The provider rejected the request as malformed — a complaint about the request itself, which every model would reject identically. Walking the hall would only have collected the same error N times.',
    advice: () => 'The quoted rejection names what the client sent wrong. Fix the caller, not the hall.',
  },
  unattributed: {
    title: 'Unexplained — receipts attached',
    why: () => 'This failure does not match any pattern the engine knows. No verdict is invented for it.',
    advice: () =>
      'The evidence is preserved verbatim below. If the same bytes keep appearing, that pattern is the diagnosis waiting to be written — exactly how every rule above started.',
  },
}

function fmtAgo(at) {
  const s = Math.max(1, Math.round(Date.now() / 1000 - at))
  if (s < 60) return `${s}s ago`
  if (s < 3600) return `${Math.round(s / 60)}m ago`
  return `${Math.round(s / 3600)}h ago`
}

$('notifBell').addEventListener('click', openNotifications)
$('closeNotif').addEventListener('click', closeNotifications)

$('notifScrim').addEventListener('click', (event) => {
  if (event.target === $('notifScrim')) return closeNotifications()

  const scopeClear = event.target.closest('.notif-scope-clear')
  if (scopeClear) {
    notifLaneFilter = null
    renderNotifCenter()
    return
  }

  const fix = event.target.closest('.issue-fix')
  if (fix && fix.dataset.fix === 'no-think') {
    const hall = state.lanes.find((l) => l.slug === fix.dataset.hall)
    if (!hall) return
    hall.suppress_reasoning = true
    saveLanes()
    renderLanes()
    renderNotifCenter()
    toast(`${hall.name}: members will be asked to answer without thinking`)
    return
  }

  // Muting silences a TYPE, not the record: the engine keeps writing these,
  // the bell and toasts just stop announcing them. Reversible in place.
  const mute = event.target.closest('.issue-mute')
  if (mute) {
    const kind = mute.dataset.mute
    if (notif.muted.has(kind)) notif.muted.delete(kind)
    else notif.muted.add(kind)
    notifPersist()
    renderNotifCenter()
    renderBell()
  }
})

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !$('notifScrim').hidden) closeNotifications()
})

// -------------------------------------------------------------- member dials
//
// The gear on a relic in a hall opens a small panel of request settings the
// hall fixes for that one member: temperature, penalties, a token ceiling.
// The engine injects them per attempt and unwinds them before the next
// member, so tuning one relic can never bleed into its neighbours.
//
// Writes go straight onto the member ref and save on every change — there is
// no OK button, because there is nothing to confirm: blank a field and the
// dial is gone, type a value and it is set. The popover is positioned beside
// the gear it came from, and closes on Escape, on the close button, or on
// any press outside it.

/** Which member the open popover is editing, by identity — never by element,
 *  because a re-render replaces every element under the popover. */
const popTarget = { hall: null, provider: '', id: '' }

/** The member ref the popover is aimed at, freshly looked up in state. */
function popMember() {
  const hall = state.lanes.find((l) => l.slug === popTarget.hall)
  if (!hall) return null
  return (
    hall.members.find(
      (r) => r.id === popTarget.id && (r.provider || '') === popTarget.provider
    ) || hall.members.find((r) => r.id === popTarget.id)
  )
}

function openMemberPop(gear) {
  const relic = gear.closest('.relic')
  const laneEl = relic.closest('.hall')
  popTarget.hall = laneEl.dataset.hall
  popTarget.provider = relic.dataset.provider || ''
  popTarget.id = relic.dataset.model

  const member = popMember()
  if (!member) return

  $('popTitle').textContent = member.id
  document.querySelectorAll('#memberPop [data-dial]').forEach((input) => {
    const value = (member.params || {})[input.dataset.dial]
    input.value = value == null ? '' : value
  })

  const pop = $('memberPop')
  pop.hidden = false

  // Beside the gear, clamped to the window. Measured after unhiding, because
  // a hidden element has no size to measure.
  const at = gear.getBoundingClientRect()
  const niche = pop.getBoundingClientRect()
  const left = Math.max(8, Math.min(at.left - niche.width / 2, window.innerWidth - niche.width - 8))
  const below = at.bottom + 10
  const top = below + niche.height > window.innerHeight - 8 ? at.top - niche.height - 10 : below
  pop.style.left = `${left}px`
  pop.style.top = `${Math.max(8, top)}px`
}

/** Hide the popover. Re-rendering is the caller's choice: closing by starting
 *  a drag must NOT rebuild the DOM out from under that drag. */
function closeMemberPop({ rerender = true } = {}) {
  const pop = $('memberPop')
  if (pop.hidden) return
  pop.hidden = true
  popTarget.hall = null
  // The gear's "tuned" dot may have changed while the popover was open.
  if (rerender) renderLanes()
}

$('memberPop').addEventListener('change', (event) => {
  const input = event.target.closest('[data-dial]')
  const member = input && popMember()
  if (!member) return
  member.params = member.params || {}
  if (input.value === '') {
    delete member.params[input.dataset.dial]
  } else {
    const value = Number(input.value)
    // An unparseable entry is treated as blank rather than saved as NaN —
    // JSON has no NaN, and a half-written number should not become a dial.
    if (Number.isFinite(value)) {
      member.params[input.dataset.dial] =
        input.dataset.dial === 'max_tokens' ? Math.max(1, Math.round(value)) : value
    } else {
      input.value = ''
      delete member.params[input.dataset.dial]
    }
  }
  saveLanes()
})

// ------------------------------------------------------------- relic menu
//
// Right-click on a hall member. The gear and the drag-out-to-remove gesture
// are invisible until you already know them; a menu is how they are found.
// "Park" is the third action — a member that keeps its place and its dials
// but is skipped at request time, so a hall can be tuned by subtraction
// without losing the work of arranging it.

/** Which member the open menu is aimed at, by identity. */
const menuTarget = { hall: null, provider: '', id: '' }

function menuMember() {
  const hall = state.lanes.find((l) => l.slug === menuTarget.hall)
  if (!hall) return null
  return (
    hall.members.find(
      (r) => r.id === menuTarget.id && (r.provider || '') === menuTarget.provider
    ) || hall.members.find((r) => r.id === menuTarget.id)
  )
}

function openChipMenu(relic, x, y) {
  const laneEl = relic.closest('.hall')
  if (!laneEl) return
  menuTarget.hall = laneEl.dataset.hall
  menuTarget.provider = relic.dataset.provider || ''
  menuTarget.id = relic.dataset.model
  const member = menuMember()
  if (!member) return
  $('chipMenuParkLabel').textContent = member.disabled ? 'Resume this member' : 'Park this member'
  const menu = $('chipMenu')
  menu.hidden = false
  const niche = menu.getBoundingClientRect()
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - niche.width - 8))}px`
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - niche.height - 8))}px`
}

function closeChipMenu() {
  $('chipMenu').hidden = true
  menuTarget.hall = null
}

document.addEventListener('contextmenu', (event) => {
  const relic = event.target.closest('.procession .relic')
  if (!relic) return
  event.preventDefault()
  openChipMenu(relic, event.clientX, event.clientY)
})

$('chipMenu').addEventListener('click', (event) => {
  const item = event.target.closest('[data-menu]')
  if (!item) return
  const member = menuMember()
  if (!member) return closeChipMenu()
  const hall = state.lanes.find((l) => l.slug === menuTarget.hall)

  if (item.dataset.menu === 'settings') {
    const relic = document.querySelector(
      `.hall[data-hall="${menuTarget.hall}"] .relic[data-model="${CSS.escape(menuTarget.id)}"] .relic-gear`
    )
    closeChipMenu()
    if (relic) openMemberPop(relic)
    return
  }

  if (item.dataset.menu === 'park') {
    member.disabled = !member.disabled
    closeChipMenu()
    renderLanes()
    saveLanes()
    toast(member.disabled
      ? `${member.id} parked — the hall will skip it`
      : `${member.id} resumed`)
    return
  }

  if (item.dataset.menu === 'remove') {
    closeChipMenu()
    mutateLanes(`${member.id} removed from ${hall.name}`, () => {
      hall.members = hall.members.filter((r) => r !== member)
    })
  }
})

document.addEventListener('pointerdown', (event) => {
  if (!$('chipMenu').hidden && !event.target.closest('#chipMenu')) closeChipMenu()
})
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') closeChipMenu()
})

$('popClose').addEventListener('click', () => closeMemberPop())
$('popClear').addEventListener('click', () => {
  const member = popMember()
  if (!member) return
  member.params = {}
  document.querySelectorAll('#memberPop [data-dial]').forEach((i) => (i.value = ''))
  saveLanes()
  toast(`${member.id}: settings cleared`)
})

// ------------------------------------------------------------------ interaction

function toast(message, detail = '', action = null) {
  const el = $('toast')
  el.textContent = message
  // Detail (e.g. a fallback trail) is secondary: shown as a smaller second
  // line and on hover, never crowding the headline.
  el.title = detail
  const existing = el.querySelector('.toast-detail')
  if (existing) existing.remove()
  if (detail) {
    const sub = document.createElement('span')
    sub.className = 'toast-detail'
    sub.textContent = detail
    el.appendChild(sub)
  }
  // A toast can carry one action (Undo). The button must be clickable, so the
  // toast only goes pointer-transparent when there is nothing to press.
  const oldBtn = el.querySelector('.toast-action')
  if (oldBtn) oldBtn.remove()
  if (action) {
    const btn = document.createElement('button')
    btn.className = 'toast-action'
    btn.type = 'button'
    btn.textContent = action.label
    btn.addEventListener('click', () => {
      action.fn()
      el.classList.remove('show')
    })
    el.appendChild(btn)
  }
  el.classList.toggle('has-detail', !!detail)
  el.classList.toggle('has-action', !!action)
  el.classList.add('show')
  clearTimeout(toast._t)
  toast._t = setTimeout(() => el.classList.remove('show'), action ? 6000 : detail ? 4000 : 2000)
}

/** Snapshot the lanes, run the change, and offer to undo it. The three
 *  destructive edits all funnel here so they behave identically. */
function mutateLanes(label, mutate) {
  const before = state.lanes.map((hall) => ({ ...hall, members: hall.members.map((m) => ({ ...m, params: { ...m.params } })) }))
  mutate()
  render()
  saveLanes()
  armUndo(before, label)
  toast(label, '', {
    label: 'Undo',
    fn: () => {
      const undo = takeUndo()
      if (!undo) return
      state.lanes = undo.lanes
      render()
      saveLanes()
      toast('Restored')
    },
  })
}

document.addEventListener('pointerdown', (event) => {
  if (event.button !== 0) return
  api.focus()

  // A press anywhere outside the open popover closes it. Without a rerender:
  // if this press is also the start of a drag, rebuilding the DOM here would
  // orphan the relic the drag is about to grab.
  if (!$('memberPop').hidden && !event.target.closest('#memberPop')) {
    closeMemberPop({ rerender: false })
  }

  // The gear opens settings; it must not begin a drag. The click handler
  // below does the opening — this guard only keeps the relic from grabbing.
  if (event.target.closest('.relic-gear')) return

  const remove = event.target.closest('.relic-remove')
  if (remove) {
    const relic = remove.closest('.relic')
    const hall = state.lanes.find((l) => l.slug === relic.closest('.hall').dataset.hall)
    mutateLanes(`${relic.dataset.model} removed from ${hall.name}`, () => {
      hall.members = hall.members.filter(
        (r) => !(r.id === relic.dataset.model && (r.provider || '') === relic.dataset.provider)
      )
    })
    return
  }

  const relic = event.target.closest('.relic')
  if (relic) {
    event.preventDefault()
    beginDrag(event, relic)
  }
})

document.addEventListener('click', async (event) => {
  const gear = event.target.closest('.relic-gear')
  if (gear) {
    openMemberPop(gear)
    return
  }

  const remove = event.target.closest('.hall-remove')
  if (remove) {
    const hall = state.lanes.find((l) => l.slug === remove.closest('.hall').dataset.hall)
    mutateLanes(`${hall.name} deleted`, () => {
      state.lanes = state.lanes.filter((l) => l !== hall)
    })
    return
  }

  // The criteria text lives in the footer's activity line now. A click on it
  // opens a FRESH SEARCH with these criteria — deliberately not a rebuild of
  // the hall, for the reasons above. The rest of the line opens the trail.
  const activityLine = event.target.closest('.hall-foot .lane-activity')
  if (activityLine && event.target.closest('.foot-criteria')) {
    const hall = state.lanes.find((l) => l.slug === activityLine.closest('.hall').dataset.hall)
    if (hall?.criteria?.length) {
      state.browse.sorts = hall.criteria.map(({ field, desc }) => ({ field, desc }))
      openBrowse()
      toast(`Searching: ${hall.criteria.map(criterionWords).join(' + ')}`)
    }
    return
  }

  // The two header icon-toggles share one class; `data-toggle` says which.
  const toggle = event.target.closest('.hall-toggle')
  if (toggle) {
    const hall = state.lanes.find((l) => l.slug === toggle.closest('.hall').dataset.hall)
    if (toggle.dataset.toggle === 'unstick') {
      hall.unstick = !hall.unstick
      render()
      saveLanes()
      toast(
        hall.unstick
          ? `${hall.name}: stuck agents will be unstuck (collapsed + noted, announced in headers)`
          : `${hall.name}: conversations pass through untouched`
      )
    } else {
      hall.suppress_reasoning = !hall.suppress_reasoning
      render()
      saveLanes()
      toast(
        hall.suppress_reasoning
          ? `${hall.name}: members will be asked to answer without thinking`
          : `${hall.name}: members may think before answering`
      )
    }
    return
  }

  const url = event.target.closest('.hall-url')
  if (url) {
    const slug = url.closest('.hall').dataset.hall
    await api.copy(`${laneEndpoint(slug)}/chat/completions`)
    toast('Endpoint URL copied')
    return
  }

  const test = event.target.closest('.lane-test')
  if (test) {
    const hall = state.lanes.find((l) => l.slug === test.closest('.hall').dataset.hall)
    if (!hall) return
    try {
      const result = await api.laneTest(hall.slug)
      // The point of a fallback hall is which model answered and what was
      // passed over — `lane_test` already returns both; showing only the
      // message wastes them. The trail is the whole story, so it rides along
      // on hover where it won't flood the toast.
      if (result.ok && result.served_by) {
        toast(`${hall.name}: answered by ${result.served_by}`, result.trail || '')
      } else {
        toast(`${hall.name}: ${result.message}`, result.trail || '')
      }
    } catch (error) {
      toast(`${hall.name}: ${error.message}`)
    }
    return
  }

  const setup = event.target.closest('.lane-copy-setup')
  if (setup) {
    const slug = setup.closest('.hall').dataset.hall
    await api.copy(laneCurlExample(slug))
    toast('Curl setup copied')
    return
  }

  // The activity line opens the hall's own trail: its recent attempts, newest
  // first, each with the evidence. This is the same records the notification
  // center shows, scoped to one hall — the difference between "something
  // failed somewhere" and "this hall's story".
  const activity = event.target.closest('.lane-activity')
  if (activity) {
    openNotifications(activity.closest('.hall').dataset.hall)
    return
  }

  const vscodeBtn = event.target.closest('.lane-vscode')
  if (vscodeBtn) {
    const laneEl = vscodeBtn.closest('.hall')
    const slug = laneEl.dataset.hall
    const hall = state.lanes.find(l => l.slug === slug)
    console.log('[vscode] button clicked', { slug, hall: hall?.name })
    if (!hall) {
      console.error('[vscode] hall not found', slug)
      return
    }
    try {
      console.log('[vscode] calling api.vscodeIntegrateLane', { slug, name: hall.name })
      await api.vscodeIntegrateLane(slug, hall.name)
      console.log('[vscode] success')
      toast(`Added "${hall.name}" to the VS Code model picker`, 'Reload the editor window (Ctrl+R) to see it')
    } catch (error) {
      console.error('[vscode] failed', error)
      toast(`Failed: ${error.message}`)
    }
    return
  }

  // Scoped to `#filters`, not a bare `.filter`.
  //
  // This handler is on `document`, and the browse panel uses the same class for
  // its own filter buttons. An unscoped selector caught those too as they
  // bubbled up, so choosing "Vision" in the browser silently filtered the pool
  // in the sidebar behind it — two states changed by one click, one of them
  // invisible at the time.
  //
  // Any delegated handler on `document` needs a container in its selector for
  // exactly this reason.
  const filter = event.target.closest('#filters .seal')
  if (filter) {
    const key = filter.dataset.filter
    state.filters[key] = !state.filters[key]
    filter.classList.toggle('is-sealed', state.filters[key])
    renderSidebar()
  }

  // The order strip: one engraved choice, lit.
  const sortBtn = event.target.closest('#sortStrip button')
  if (sortBtn) {
    state.sort = sortBtn.dataset.sort
    document.querySelectorAll('#sortStrip button').forEach((b) =>
      b.classList.toggle('is-chosen', b === sortBtn))
    renderSidebar()
  }
})

document.addEventListener('focusout', (event) => {
  const name = event.target.closest?.('.hall-name')
  if (!name) return
  const hall = state.lanes.find((l) => l.slug === name.closest('.hall').dataset.hall)
  const next = name.textContent.trim()
  // The slug is fixed at creation: renaming must never move a live endpoint.
  hall.name = next || hall.name
  name.textContent = hall.name
  saveLanes()
})

document.addEventListener('keydown', (event) => {
  const name = event.target.closest?.('.hall-name')
  if (name && event.key === 'Enter') {
    event.preventDefault()
    name.blur()
  }
})

// Shortcuts. The app is a power tool used alongside editors, so it should be
// drivable without the mouse: the three panels, a new hall, and a help overlay
// on `?`. Guarded against typing — a shortcut must never fire mid-word in the
// search niche or an input.
document.addEventListener('keydown', (event) => {
  const typing = event.target.closest('input, textarea, select, [contenteditable]')
  if (typing) return

  const mod = event.ctrlKey || event.metaKey
  if (mod && event.key.toLowerCase() === 'n') {
    event.preventDefault()
    $('newLane').click()
  } else if (mod && event.key.toLowerCase() === 'b') {
    event.preventDefault()
    openBrowse()
  } else if (mod && event.key === ',') {
    event.preventDefault()
    openSettings()
  } else if (event.key === '?' || (event.shiftKey && event.key === '/')) {
    event.preventDefault()
    toast('Shortcuts', 'Ctrl+N new hall · Ctrl+B browse · Ctrl+, settings · Esc close · right-click a member for its menu', null)
  }
})

$('search').addEventListener('input', (e) => {
  state.search = e.target.value
  renderSidebar()
})

$('newLane').addEventListener('click', () => {
  const base = 'new-lane'
  let slug = base
  let n = 2
  while (state.lanes.some((l) => l.slug === slug)) slug = `${base}-${n++}`
  state.lanes.unshift({ slug, name: 'New hall', members: [] })
  render()
  saveLanes()
  const el = document.querySelector(`.hall[data-hall="${slug}"] .hall-name`)
  el?.focus()
  document.getSelection()?.selectAllChildren(el)
})

$('wcMin').addEventListener('click', () => api.minimize())
$('wcMax').addEventListener('click', () => api.toggleMaximize())
$('wcClose').addEventListener('click', () => api.close())
// The frame is the building: drag the window by any bare stretch of stage —
// never by a control, an input, or a relic in the hand.
$('stage').addEventListener('mousedown', (event) => {
  if (event.button !== 0) return
  if (event.target.closest('button, input, select, .relic, .hall-name, a, [contenteditable]')) return
  api.startDragging()
})


// ------------------------------------------------------------------ providers

function openPanel() {
  $('scrim').hidden = false
  renderProviders()
  resetForm()
}

function closePanel() {
  $('scrim').hidden = true
}

function resetForm() {
  state.editing = null
  $('formTitle').textContent = 'Add a provider'
  $('pId').value = ''
  $('pName').value = ''
  $('pKind').value = 'openrouter'
  $('pUrl').value = presetById('openrouter').url
  $('pUrl').placeholder = presetById('openrouter').url
  $('pKey').value = ''
  $('pKey').placeholder = 'sk-or-…'
  $('pSave').textContent = 'Save provider'
  $('pDelete').hidden = true
  $('pCancel').hidden = true
  note('')
  renderProviders()
}

function note(message, kind = '') {
  const el = $('formNote')
  el.textContent = message
  el.className = `form-note ${kind}`
}

function editProvider(id) {
  const provider = state.providers.find((p) => p.id === id)
  if (!provider) return
  state.editing = id
  $('formTitle').textContent = `Edit ${provider.name}`
  $('pId').value = provider.id
  $('pName').value = provider.name
  const url = (provider.base_url || '').replace(/\/+$/, '')
  const match = PRESETS.find((x) => x.url && x.url.replace(/\/+$/, '') === url)
    || PRESETS.find((x) => x.kind === provider.kind && x.id === provider.kind)
  $('pKind').value = (match || presetById('custom')).id
  $('pUrl').value = provider.base_url
  $('pKey').value = ''
  // Left blank the key is kept, so editing a name never means retyping a secret.
  $('pKey').placeholder = provider.has_key ? `${provider.key_hint} — leave blank to keep` : 'sk-…'
  $('pSave').textContent = 'Save changes'
  $('pDelete').hidden = false
  $('pCancel').hidden = false
  note('')
  renderProviders()
}

function renderProviders() {
  const list = $('providerList')
  list.innerHTML = ''
  if (!state.providers.length) {
    list.innerHTML = `<div class="empty-state provider-onboarding">
      <strong>Connect your first provider</strong>
      <span>A provider is a service that supplies models, such as OpenRouter, OpenAI, Anthropic, or a local server.</span>
      <span>Enter its API key below, test the connection, and its catalog will appear here.</span>
    </div>`
    return
  }
  state.providers.forEach((provider) => {
    const count = state.counts[provider.id]
    const failed = typeof count === 'string'
    const el = document.createElement('div')
    el.className = `provider${state.editing === provider.id ? ' is-editing' : ''}`
    el.dataset.provider = provider.id
    // THE ROW IS THE DOOR TO THE MODELS. Seeing what a provider offers is why
    // anyone opens this panel; changing its key is the rare errand. So the
    // frequent intent gets the whole row, and editing sits behind the pencil.
    // Entering a key and looking at a catalog are different workflows, and
    // for a while the only path to the second ran through the first.
    el.title = failed
      ? 'This provider failed to load — click to fix its settings'
      : "View this provider's models"
    el.innerHTML = `
      <span class="provider-body">
        <span class="provider-name">${provider.name}</span>
        <span class="provider-meta">${provider.base_url}${
          provider.has_key ? ` · ${provider.key_hint}` : ' · no key'
        }</span>
      </span>
      <span class="provider-count${failed ? ' bad' : ''}">${
        failed ? count : count == null ? '—' : `${count} models`
      }</span>
      <button class="provider-edit" data-edit="${provider.id}"
        title="Edit ${attr(provider.name)} — name, URL, API key">
        <svg viewBox="0 0 16 16"><path d="M3.2 12.8l.9-3.2 7.2-7.2a1.3 1.3 0 0 1 1.9 0l.4.4a1.3 1.3 0 0 1 0 1.9l-7.2 7.2-3.2.9z"/></svg>
      </button>
    `
    list.appendChild(el)
  })
}

async function loadProviders() {
  try {
    state.providers = await api.providersList()
  } catch (err) {
    state.providers = []
  }
}

async function loadPort() {
  try {
    enginePort = Number(await api.portGet()) || 4100
  } catch {
    enginePort = 4100
  }
}

function openSettings() {
  $('enginePort').value = enginePort
  $('settingsScrim').hidden = false
}

function closeSettings() {
  $('settingsScrim').hidden = true
}

$('statPort').addEventListener('click', openSettings)
$('openSettings').addEventListener('click', openSettings)
$('closeSettings').addEventListener('click', closeSettings)
$('cancelSettings').addEventListener('click', closeSettings)
$('settingsScrim').addEventListener('click', (event) => {
  if (event.target === $('settingsScrim')) closeSettings()
})
$('settingsForm').addEventListener('submit', async (event) => {
  event.preventDefault()
  const port = Number($('enginePort').value)
  if (!Number.isInteger(port) || port < 1024 || port > 65535) {
    toast('Choose a port from 1024 to 65535')
    return
  }
  try {
    const activePort = Number(await api.portSet(port))
    enginePort = activePort || port
    render()
    closeSettings()
    toast(`Engine moved to ${engineHost()}`)
  } catch (error) {
    toast(`Could not save port: ${error.message}`)
  }
})

function mergeStats() {
  const stats = state.stats || {}
  state.catalog.forEach((m) => {
    const s = stats[m.id]
    m.throughput = s ? s.throughput : null
    m.latency = s ? s.latency : null
    m.providers = s ? s.providers : 0
  })
}

async function loadStats({ refresh = false } = {}) {
  try {
    if (refresh) {
      // 337 HTTP calls. Runs in the background; the catalog is already on
      // screen and simply gains two columns when this returns.
      const found = await api.statsRefresh()
      if (found) toast(`Speed data loaded for ${found} models`)
    }
    const file = await api.statsRead()
    state.stats = (file && file.models) || {}
    state.statsFetchedAt = (file && file.fetched_at) || 0
    mergeStats()
    if (!$('browseScrim').hidden) renderBrowse()
    renderSidebar()
  } catch (err) {
    // A missing speed column is survivable; a broken browser is not.
  }
}

async function loadCatalog() {
  if (!state.providers.length) {
    state.catalog = []
    state.counts = {}
    renderSidebar()
    return
  }
  try {
    const result = await api.catalogRead(null)
    state.catalog = (result.models || []).map((m) => ({ ...m, source: 'catalog' }))
    state.catalogErrors = result.errors || []

    // A failed provider no longer empties the engine's cache (Rust keeps the
    // last good one), but the user should hear about it — a red count in the
    // provider list is too easy to miss, and the failure is exactly when a
    // hall might start behaving unexpectedly. The signature guard keeps a
    // re-poll that fails identically from re-firing the same warning.
    const errorSignature = state.catalogErrors.map((e) => e.provider_id).sort().join(',')
    if (state.catalogErrors.length && errorSignature !== loadCatalog._notifiedFor) {
      loadCatalog._notifiedFor = errorSignature
      state.catalogErrors.forEach((e) => {
        toast(`${e.provider_name}: catalog failed — using the last good cache`, String(e.error))
      })
    } else if (!state.catalogErrors.length) {
      loadCatalog._notifiedFor = null
    }

    const counts = {}
    state.catalog.forEach((m) => (counts[m.provider_id] = (counts[m.provider_id] || 0) + 1))
    state.catalogErrors.forEach((e) => (counts[e.provider_id] = e.error))
    state.counts = counts
    mergeStats()
    // Old refs can finally learn which provider they meant.
    qualifyRefs()

    // Refresh if we have never fetched, or the figures are over an hour old —
    // they describe the last thirty minutes, so anything older is fiction.
    const age = Date.now() / 1000 - (state.statsFetchedAt || 0)
    if (age > 3600) loadStats({ refresh: true })
  } catch (err) {
    state.catalogErrors = [{ provider_name: 'catalog', error: String(err) }]
  }
  renderSidebar()
  renderProviders()
  // Lanes render before the catalog exists, so their members draw as dead
  // until this repaint. The four-second poll used to hide that by repainting
  // unconditionally; now that it repaints only on change, the catalog's
  // arrival must announce itself.
  renderLanes()
}

$('openProviders').addEventListener('click', openPanel)
$('closeProviders').addEventListener('click', closePanel)
$('pCancel').addEventListener('click', resetForm)

$('scrim').addEventListener('click', (event) => {
  if (event.target === $('scrim')) closePanel()
  // The pencil first — it sits inside the row, and the row means "view".
  const edit = event.target.closest('.provider-edit')
  if (edit) {
    editProvider(edit.dataset.edit)
    return
  }
  const row = event.target.closest('.provider')
  if (row) {
    // A provider that failed to load has no models to show; the only useful
    // next step is fixing its settings, so the row opens those instead.
    if (typeof state.counts[row.dataset.provider] === 'string') {
      editProvider(row.dataset.provider)
      return
    }
    closePanel()
    openBrowse(row.dataset.provider)
  }
})

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !$('scrim').hidden) closePanel()
  if (event.key === 'Escape') closeMemberPop()
})

/**
 * THE PROVIDER LIST.
 *
 * A preset is data: a label, a base URL, and what the key looks like. The
 * `kind` is which code path runs, and there are only three of those —
 * `openrouter` for the rich catalog, `anthropic` for its own auth header, and
 * `openai` for everything else that speaks the OpenAI shape.
 *
 * Keeping those separate is the point. Adding the thirty-first provider should
 * be one line in this table, not a branch somewhere in Rust.
 *
 * Base URL is the single most common setup failure, and the error it produces
 * is a bare 404 that explains nothing — some services want `/v1`, some do not,
 * Groq wants `/openai/v1`, DeepInfra wants `/v1/openai`. That is precisely why
 * this table exists and why every field stays editable: these are best-known
 * values, and Test is what proves them.
 */
const PRESETS = [
  { id: 'openrouter', kind: 'openrouter', name: 'OpenRouter',
    url: 'https://openrouter.ai/api/v1', key: 'sk-or-…' },

  { id: 'openai',     kind: 'openai',     name: 'OpenAI',
    url: 'https://api.openai.com/v1', key: 'sk-…' },
  { id: 'anthropic',  kind: 'anthropic',  name: 'Anthropic',
    url: 'https://api.anthropic.com/v1', key: 'sk-ant-…' },

  // Drop-in OpenAI-compatible. No code, just a URL.
  { id: 'groq',       kind: 'openai', name: 'Groq',
    url: 'https://api.groq.com/openai/v1', key: 'gsk_…' },
  { id: 'cerebras',   kind: 'openai', name: 'Cerebras',
    url: 'https://api.cerebras.ai/v1', key: 'csk-…' },
  { id: 'together',   kind: 'openai', name: 'Together AI',
    url: 'https://api.together.xyz/v1', key: '…' },
  { id: 'fireworks',  kind: 'openai', name: 'Fireworks AI',
    url: 'https://api.fireworks.ai/inference/v1', key: 'fw_…' },
  { id: 'deepinfra',  kind: 'openai', name: 'DeepInfra',
    url: 'https://api.deepinfra.com/v1/openai', key: '…' },
  { id: 'deepseek',   kind: 'openai', name: 'DeepSeek',
    url: 'https://api.deepseek.com/v1', key: 'sk-…' },
  { id: 'xai',        kind: 'openai', name: 'xAI (Grok)',
    url: 'https://api.x.ai/v1', key: 'xai-…' },
  { id: 'mistral',    kind: 'openai', name: 'Mistral',
    url: 'https://api.mistral.ai/v1', key: '…' },
  // Chat is at the root here, the catalog is under /v1. The base is set for
  // chat, and the catalog fetch falls back to /v1 on a 404. Verified 2026-08-02.
  { id: 'perplexity', kind: 'openai', name: 'Perplexity',
    url: 'https://api.perplexity.ai', key: 'pplx-…' },
  { id: 'nebius',     kind: 'openai', name: 'Nebius',
    url: 'https://api.studio.nebius.ai/v1', key: '…' },

  // Local. No key at all, which is why the field has to tolerate being empty.
  { id: 'ollama',     kind: 'openai', name: 'Ollama (local)',
    url: 'http://localhost:11434/v1', key: 'not needed' },
  { id: 'lmstudio',   kind: 'openai', name: 'LM Studio (local)',
    url: 'http://localhost:1234/v1', key: 'not needed' },
  { id: 'vllm',       kind: 'openai', name: 'vLLM (local)',
    url: 'http://localhost:8000/v1', key: 'not needed' },

  { id: 'custom',     kind: 'openai', name: 'Other (OpenAI-compatible)',
    url: '', key: 'sk-…' },
]

const presetById = (id) => PRESETS.find((p) => p.id === id) || PRESETS[PRESETS.length - 1]

function fillPresetOptions() {
  $('pKind').innerHTML = PRESETS.map(
    (p) => `<option value="${p.id}">${p.name}</option>`
  ).join('')
}

$('pKind').addEventListener('change', (e) => {
  const preset = presetById(e.target.value)
  $('pUrl').placeholder = preset.url || 'https://api.example.com/v1'
  $('pKey').placeholder = preset.key
  // Overwrite only what the user has not typed over themselves: a field still
  // holding another preset's value is ours to replace, anything else is theirs.
  if (!state.editing) {
    const ourUrls = PRESETS.map((p) => p.url).filter(Boolean)
    if (!$('pUrl').value || ourUrls.includes($('pUrl').value)) $('pUrl').value = preset.url
    const ourNames = PRESETS.map((p) => p.name)
    if (!$('pName').value || ourNames.includes($('pName').value)) $('pName').value = preset.name
  }
})

$('pTest').addEventListener('click', async () => {
  note('testing…')
  try {
    const found = await api.providerTest(
      presetById($('pKind').value).kind, $('pUrl').value, $('pKey').value)
    note(`reached it — ${found} models available`, 'ok')
  } catch (err) {
    note(String(err), 'bad')
  }
})

$('providerForm').addEventListener('submit', async (event) => {
  event.preventDefault()
  const key = $('pKey').value.trim()
  try {
    // The command returns the saved provider — id included, which matters
    // because a new provider's id is minted on the Rust side. (This used to be
    // rediscovered by name afterwards, against a variable that didn't exist,
    // so the browse below always opened unfiltered.)
    const saved = await api.providerSave({
      id: state.editing || null,
      name: $('pName').value,
      // The dropdown carries a preset id; storage carries the code path.
      kind: presetById($('pKind').value).kind,
      base_url: $('pUrl').value,
      // Blank while editing means "keep what is stored"; blank on a new one is
      // a genuinely empty key.
      key: state.editing && !key ? null : key,
    })
    await loadProviders()
    resetForm()
    note('saved', 'ok')
    // Straight into browsing what THIS provider brought: the person who just
    // added Groq wants Groq's models, not Groq shuffled into 337 other rows.
    await loadCatalog()
    closePanel()
    openBrowse(saved.id)
  } catch (err) {
    note(String(err), 'bad')
  }
})

$('pDelete').addEventListener('click', async () => {
  if (!state.editing) return
  try {
    await api.providerDelete(state.editing)
    await loadProviders()
    resetForm()
    loadCatalog()
    toast('Provider removed')
  } catch (err) {
    note(String(err), 'bad')
  }
})



/**
 * What a locked column means in words, per direction.
 *
 * The header says "out/M ↑" because it labels a column of numbers. A hall
 * header has to say "cheapest", because it is describing what the hall is for
 * and will be read months later by someone who was not there.
 */
const CRITERION_WORDS = {
  price:        ['priciest', 'cheapest'],
  price_in:     ['priciest input', 'cheapest input'],
  throughput:   ['slowest', 'fastest'],
  latency:      ['slowest to start', 'quickest to start'],
  intelligence: ['least capable', 'smartest'],
  coding:       ['worst at code', 'best at code'],
  agentic:      ['worst at tools', 'best at tools'],
  context:      ['smallest context', 'biggest context'],
  providers:    ['least hosted', 'most hosted'],
  created:      ['oldest', 'newest'],
  name:         ['Z to A', 'A to Z'],
}

const criterionWords = ({ field, desc }) => {
  const pair = CRITERION_WORDS[field]
  if (!pair) return field
  // `desc` means the high end is good for most columns; for price and latency
  // the natural direction is already reversed, so the pair is read the same way
  // either way — index 1 is whatever the user is actually asking for.
  return desc === NATURAL_DESC(field) ? pair[1] : pair[0]
}

// ============================================================================
// THE MODEL BROWSER
// ============================================================================
//
// Shopping and building are different jobs and want different surfaces. This
// panel is the catalog — hundreds of rows, every metric the provider publishes,
// sortable and filterable. The sidebar is the pool: only what you kept, small
// enough to scan, and the thing you drag from.
//
// The button on each row is the line between them.

const CAP_ICONS = [
  ['vision', 'IMG'],
  ['tools', 'FN'],
  ['reasoning', 'R'],
  ['structured', '{}'],
]

function fmtCreated(seconds) {
  if (!seconds) return '—'
  const days = Math.floor((Date.now() / 1000 - seconds) / 86400)
  if (days < 1) return 'today'
  if (days < 30) return `${days}d`
  if (days < 365) return `${Math.floor(days / 30)}mo`
  return `${(days / 365).toFixed(1)}y`
}

function metric(value, label, dim = false) {
  return `<span class="metric">
    <span class="metric-value${dim ? ' dim' : ''}">${value}</span>
    <span class="metric-label">${label}</span>
  </span>`
}

/** Escape anything that goes into an attribute. Model descriptions are written
 *  by vendors and arrive with quotes and angle brackets in them. */
function attr(text) {
  return String(text || '')
    .replace(/&/g, '&amp;').replace(/"/g, '&quot;')
    .replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

/** One rendered cell per column key, so the header and every row agree about
 *  which columns exist without agreeing about anything else. */
function metricCell(model, key) {
  const score = (v) => (v == null ? '—' : Math.round(v))
  switch (key) {
    case 'intelligence': return metric(score(model.intelligence), 'intel', model.intelligence == null)
    case 'coding':       return metric(score(model.coding), 'code', model.coding == null)
    case 'agentic':      return metric(score(model.agentic), 'agent', model.agentic == null)
    case 'context':      return metric(fmtContext(model.context), 'ctx')
    case 'price_in': {
      const v = model.price_in != null && model.price_in >= 0
        ? (model.price_in * 1e6 === 0 ? 'free' : `$${(model.price_in * 1e6).toFixed(2)}`)
        : '—'
      return metric(v, 'in/M')
    }
    case 'price':        return metric((fmtPrice(model) || '—').replace('/M', ''), 'out/M')
    case 'throughput':
      return metric(model.throughput == null ? '—' : Math.round(model.throughput), 'tok/s', model.throughput == null)
    case 'latency': {
      // Latency reads better in seconds past a second; nobody compares 3018ms.
      const ttft = model.latency == null ? '—'
        : model.latency >= 1000 ? `${(model.latency / 1000).toFixed(1)}s`
        : `${Math.round(model.latency)}ms`
      return metric(ttft, 'ttft', model.latency == null)
    }
    case 'providers':    return metric(model.providers || '—', 'hosts', !model.providers)
    case 'created':      return metric(fmtCreated(model.created), 'age', true)
    default:             return metric('—', key, true)
  }
}

function rowEl(model, cols) {
  const pooled = poolHas(model)
  const el = document.createElement('div')
  el.className = `row${pooled ? ' is-pooled' : ''}`
  el.dataset.model = model.id
  el.dataset.provider = model.provider_id || ''

  // The vendor's own description, on hover. It is the fastest way to tell two
  // similarly-scored models apart, and it costs nothing — we already cache it.
  if (model.description) el.title = model.description

  // With two or more criteria the composite is the thing actually being sorted
  // on, so it has to be visible — otherwise the order looks arbitrary.
  const criteria = state.browse.sorts.filter((s) => !NON_CRITERIA.has(s.field))
  const composite = criteria.length > 1 ? (state.browse.scores || new Map()).get(model.id) : undefined
  const match = composite === undefined ? ''
    : composite == null
      ? `<span class="match none" title="Not enough published data to judge this model on every locked column">—</span>`
      : `<span class="match"><span class="match-value">${Math.round(composite * 100)}</span><span class="match-label">match</span></span>`

  el.innerHTML = `
    ${match}
    <span class="row-body">
      <span class="row-title">
        <span class="row-name">${model.name || model.id}</span>
        <span class="row-author">${model.author || (multiProvider() ? '' : model.provider_name || '')}</span>
        ${multiProvider() ? `<span class="row-provider">${attr(model.provider_name || '')}</span>` : ''}
      </span>
      <span class="row-id">${model.id}</span>
    </span>

    <span class="row-metrics">
      ${cols.map(([key]) => metricCell(model, key)).join('')}
    </span>

    <span class="row-caps">
      ${CAP_ICONS.map(([key, label]) =>
        // A generic catalog says nothing about capability; a dim icon with an
        // honest tooltip beats an "off" state claiming the model can't.
        model.caps_known === false
          ? `<span class="cap unknown" title="${key}: not published by this provider">${label}</span>`
          : `<span class="cap ${model[key] ? 'on' : ''}" title="${key}">${label}</span>`
      ).join('')}
    </span>

    <button class="row-add${pooled ? ' added' : ''}">${pooled ? 'Added' : 'Add'}</button>
  `
  return el
}

function browseMatches() {
  const b = state.browse
  const q = b.search.toLowerCase()

  let models = state.catalog.filter((m) => {
    if (b.provider && m.provider_id !== b.provider) return false
    if (q && !m.id.toLowerCase().includes(q) && !(m.name || '').toLowerCase().includes(q)) return false
    if (b.author && m.author !== b.author) return false
    if (b.context && (m.context || 0) < b.context) return false

    if (b.price !== '') {
      const ceiling = Number(b.price)
      if (ceiling === 0) {
        if (!m.free) return false
      } else {
        const per = pricePerMillion(m)
        if (per == null || per > ceiling) return false
      }
    }

    for (const [key, on] of Object.entries(b.filters)) {
      if (!on) continue
      if (key === 'rated' && m.intelligence == null) return false
      else if (key === 'pooled' && !poolHas(m)) return false
      else if (key !== 'rated' && key !== 'pooled' && !m[key]) return false
    }
    return true
  })

  const textSorts = b.sorts.filter((s) => NON_CRITERIA.has(s.field))
  const criteria = b.sorts.filter((s) => !NON_CRITERIA.has(s.field))

  // Name and provider are not criteria — there is no such thing as being
  // 80th-percentile alphabetical. Locked alone they sort, in lock order (so
  // provider-then-name groups a mixed catalog by provider); locked alongside
  // real criteria they are ignored.
  if (!criteria.length) {
    state.browse.scores = new Map()
    models.sort((x, y) => {
      for (const s of textSorts) {
        const c = s.field === 'provider'
          ? (x.provider_name || '').localeCompare(y.provider_name || '')
          : x.id.localeCompare(y.id)
        if (c) return s.desc ? -c : c
      }
      return x.id.localeCompare(y.id)
    })
    return models
  }

  const scores = scoreModels(models, criteria)
  state.browse.scores = scores
  models.sort((x, y) => {
    const a = scores.get(x.id)
    const c = scores.get(y.id)
    if (a == null && c == null) return x.id.localeCompare(y.id)
    if (a == null) return 1      // unjudgeable: below everything that could be judged
    if (c == null) return -1
    return c - a
  })
  return models
}

/**
 * SORTING IS THE COLUMNS.
 *
 * There is no sort dropdown. Each column header cycles through three states,
 * and the chain of active columns is the sort:
 *
 *     click once   sort by this column, in its useful direction
 *     click again  reverse it
 *     click again  drop it from the sort
 *
 * Clicking a second column ADDS it as a tiebreaker rather than replacing the
 * first, so "intelligence, then cheapest" is two clicks. The priority number on
 * each active header shows the order.
 *
 * That is worth the machinery specifically because these columns tie constantly.
 * 230 of 337 models carry no benchmark score, so they all tie at the bottom of
 * an intelligence sort; dozens sit at exactly 128K or 1M context; and every free
 * model ties at zero. A single-column sort leaves those groups in arbitrary
 * order — a second column is what makes them readable.
 */

/**
 * CRITERIA, NOT TIE-BREAKERS.
 *
 * "The cheapest, fastest, agentic model" is not a chain of tie-breakers. Read
 * lexicographically it means: sort by price, and if two models cost exactly the
 * same, consider speed. Price decides everything, speed almost never speaks,
 * and agentic never does. That is not the question anyone is asking.
 *
 * The question is: WHICH MODEL IS GOOD AT ALL OF THESE AT ONCE. So every locked
 * column is a criterion of equal weight. Each model is ranked into a percentile
 * on each one, and those percentiles are averaged into a single score.
 *
 * Percentile rather than raw scaling, because the ranges are wildly different
 * and outlier-heavy: one model runs at 2820 tok/s and on a linear scale would
 * compress every other throughput into the bottom few percent, so speed would
 * quietly stop counting.
 *
 * Consequences worth knowing:
 *
 *   * Click order stops mattering. All criteria weigh the same, so the
 *     first-versus-last confusion cannot arise.
 *   * One locked column is just a sort. A single percentile ranking averaged
 *     with nothing is the ordering itself, so nothing is lost.
 *   * Direction chooses which end of a column counts as good. Price locked
 *     cheap-first means cheap is good; reverse it and you are asking for the
 *     premium tier.
 */

/** Text columns: they order the list but can never be a criterion, because
 *  there is no percentile of being alphabetical or of being served by Groq. */
const NON_CRITERIA = new Set(['name', 'provider'])

/** Which end of a column counts as good, before the user reverses it. */
const NATURAL_DESC = (field) =>
  !['price', 'price_in', 'name', 'provider', 'latency'].includes(field)

/** The comparable number for a field, or null when it cannot be judged. */
function criterionValue(model, field) {
  if (field === 'price') return model.free ? 0 : pricePerMillion(model)
  if (field === 'price_in') {
    if (model.free) return 0
    return model.price_in == null || model.price_in < 0 ? null : model.price_in * 1e6
  }
  // Zero means "not published" for these, not "none".
  if (field === 'providers') return model.providers || null
  if (field === 'context') return model.context || null
  if (field === 'created') return model.created || null
  return model[field]
}

/**
 * Percentile ordinal per criterion: 1 is best, 0 is worst, computed only across
 * the models that actually carry a figure.
 */
function criterionRanks(models, criteria) {
  const ranks = {}
  for (const { field, desc } of criteria) {
    const judged = models
      .filter((m) => criterionValue(m, field) != null)
      .sort((a, b) => {
        const x = criterionValue(a, field)
        const y = criterionValue(b, field)
        return desc ? y - x : x - y     // best first
      })
    const last = judged.length - 1 || 1
    ranks[field] = new Map(judged.map((m, i) => [m.id, 1 - i / last]))
  }
  return ranks
}

/**
 * Average the percentiles into one score.
 *
 * A model with no figure for a locked criterion scores `null`, not zero. Zero
 * would ordinal it worst at something we cannot measure at all — a different and
 * much more misleading claim. Those sort below everything that could be judged.
 */
function scoreModels(models, criteria) {
  const ranks = criterionRanks(models, criteria)
  const scores = new Map()
  for (const m of models) {
    const parts = criteria.map((c) => ranks[c.field].get(m.id))
    scores.set(m.id, parts.every((p) => p != null)
      ? parts.reduce((a, b) => a + b, 0) / parts.length
      : null)
  }
  return scores
}

/**
 * LOCKING — why adding a column and pointing it are separate actions.
 *
 * This was one overloaded three-state cycle: sort, reverse, remove. Toggling a
 * direction twice therefore deleted the column, so you could not simply look at
 * a chain both ways round without rebuilding it. Direction is something you
 * flip idly while reading; membership is a decision. They should not share a
 * control.
 *
 * So a column is locked into the chain by clicking it, and stays locked —
 * clicking the label after that only ever reverses it. The padlock on a locked
 * header removes it, and nothing else does.
 */
function lockSort(field) {
  const sorts = state.browse.sorts
  const at = sorts.findIndex((s) => s.field === field)
  if (at < 0) sorts.push({ field, desc: NATURAL_DESC(field) })
  else sorts[at].desc = !sorts[at].desc          // locked: direction only
}

function unlockSort(field) {
  const sorts = state.browse.sorts
  const at = sorts.findIndex((s) => s.field === field)
  if (at >= 0) sorts.splice(at, 1)
  // Something has to order the list, so the last column cannot be unlocked
  // away into nothing.
  if (!sorts.length) sorts.push({ field: 'intelligence', desc: true })
}

const COLUMNS = [
  ['intelligence', 'intel'],
  ['coding', 'code'],
  ['agentic', 'agent'],
  ['context', 'ctx'],
  ['price_in', 'in/M'],
  ['price', 'out/M'],
  ['throughput', 'tok/s'],
  ['latency', 'ttft'],
  ['providers', 'hosts'],
  ['created', 'age'],
]

/**
 * Which metric columns this view actually shows.
 *
 * THE COLUMNS FOLLOW THE DATA. The full table was designed against OpenRouter,
 * which publishes everything; filter to a provider that publishes ids and
 * nothing else and ten columns of "—" remain, implying judgements nobody made.
 * Worse, sorting the mixed view by a column only OpenRouter fills sinks every
 * direct-provider model to the bottom — reading as "worst" when the truth is
 * "unmeasured". So a column earns its place by having at least one value in
 * the current view. A LOCKED column always stays: it is live state the user
 * put there, and state must never be silently discarded by a filter change —
 * it shows its emptiness honestly instead.
 */
function visibleColumns(models) {
  const locked = new Set(state.browse.sorts.map((s) => s.field))
  return COLUMNS.filter(
    ([key]) => locked.has(key) || models.some((m) => criterionValue(m, key) != null)
  )
}

/** A clickable header row, aligned to the metric columns beneath it.
 *
 * Reuses the same classes as a model row, so the widths line up by
 * construction rather than by two sets of numbers that drift apart. */
function renderBrowseHeader(cols) {
  const sorts = state.browse.sorts
  const ordinal = (field) => sorts.findIndex((s) => s.field === field)

  const LOCK = '<svg viewBox="0 0 12 12"><rect x="2.5" y="5.5" width="7" height="5" rx="1.2"/><path d="M4.2 5.5V4a1.8 1.8 0 0 1 3.6 0v1.5"/></svg>'

  const label = (field, text) => {
    const at = ordinal(field)
    if (at < 0) return text
    const arrow = sorts[at].desc ? '↓' : '↑'
    // No priority number any more: every locked column weighs the same, so
    // there is no order to convey.
    const lock = `<span class="col-lock" data-unlock="${field}" title="Remove from criteria">${LOCK}</span>`
    return `${text} ${arrow}${lock}`
  }

  const criteriaCount = sorts.filter((s) => !NON_CRITERIA.has(s.field)).length
  $('bHeader').innerHTML = `
    ${criteriaCount > 1 ? '<span class="match head-match"><span class="match-label">match</span></span>' : ''}
    <span class="row-body">
      <span class="head-text-sorts">
        <button class="col-sort col-name${ordinal('name') >= 0 ? ' is-sorted' : ''}" data-sort="name">
          <span class="metric-label">${label('name', 'model')}</span>
        </button>
        ${multiProvider() ? `
        <button class="col-sort col-name${ordinal('provider') >= 0 ? ' is-sorted' : ''}" data-sort="provider">
          <span class="metric-label">${label('provider', 'provider')}</span>
        </button>` : ''}
      </span>
      <span class="head-hint">${
        criteriaCount > 1
          ? 'ranked by how well each model does on every locked column at once'
          : 'click to lock a column in · click again to reverse · padlock to remove'
      }</span>
    </span>
    <span class="row-metrics">
      ${cols.map(([key, text]) => `
        <button class="metric col-sort${ordinal(key) >= 0 ? ' is-sorted' : ''}" data-sort="${key}">
          <span class="metric-label">${label(key, text)}</span>
        </button>`).join('')}
    </span>
    <span class="row-caps"></span>
    <span class="head-spacer"></span>
  `
}

function renderBrowse() {
  const list = $('bList')
  const models = browseMatches()
  const cols = visibleColumns(models)
  renderBrowseHeader(cols)

  const age = state.statsFetchedAt ? Math.max(0, Math.round(Date.now() / 1000 - state.statsFetchedAt)) : null
  const freshness = age == null
    ? 'speed data not fetched'
    : age < 60 ? 'speed data updated just now'
      : age < 3600 ? `speed data updated ${Math.round(age / 60)}m ago`
        : `speed data is ${Math.floor(age / 3600)}h old`
  $('bCount').textContent = `${models.length} of ${state.catalog.length} models · ${state.pool.length} in your pool · ${freshness}`

  list.innerHTML = ''
  if (!state.catalog.length) {
    list.innerHTML = `<div class="empty-state"><strong>No catalog yet</strong>Add a provider and its models load here.</div>`
    return
  }
  if (!models.length) {
    list.innerHTML = `<div class="empty-state"><strong>Nothing matches</strong>Loosen a filter.</div>`
    return
  }
  // Hundreds of rows, each with a dozen elements. Building them all up front
  // costs more than anyone will scroll through, so cap and say so.
  const CAP = 150
  models.slice(0, CAP).forEach((m) => list.appendChild(rowEl(m, cols)))
  if (models.length > CAP) {
    list.insertAdjacentHTML('beforeend',
      `<div class="empty-state">${models.length - CAP} more — narrow the search to see them.</div>`)
  }
}

function openBrowse(providerId) {
  // Passed when a provider was just saved: the person who added OpenAI wants
  // to see what OpenAI brought, not OpenAI shuffled into 337 other rows.
  if (providerId !== undefined) state.browse.provider = providerId
  $('browseScrim').hidden = false

  const seen = new Map()
  state.catalog.forEach((m) => seen.set(m.provider_id, m.provider_name))
  $('bProvider').innerHTML = '<option value="">All providers</option>' +
    [...seen].map(([id, name]) =>
      `<option value="${id}"${id === state.browse.provider ? ' selected' : ''}>${name}</option>`).join('')
  // Authors come from whatever is actually in the catalog, not a hard-coded
  // list — a new vendor appears the moment a provider carries one.
  const authors = [...new Set(state.catalog.map((m) => m.author).filter(Boolean))].sort()
  const select = $('bAuthor')
  select.innerHTML = '<option value="">All authors</option>' +
    authors.map((a) => `<option value="${a}"${a === state.browse.author ? ' selected' : ''}>${a}</option>`).join('')
  renderBrowse()
}

function closeBrowse() { $('browseScrim').hidden = true }

async function loadPool() {
  try {
    state.pool = ((await api.poolRead()) || []).map(asRef)
  } catch (err) {
    state.pool = []
  }
}

async function savePool() {
  try {
    await api.poolWrite(state.pool)
  } catch (err) {
    toast(`Could not save pool: ${err}`)
  }
}

// NOT `addEventListener('click', openBrowse)`: the handler would receive the
// click event as `providerId`, quietly setting the provider filter to a
// PointerEvent that matches no provider — a full catalog showing zero rows.
$('openBrowse').addEventListener('click', () => openBrowse())
$('closeBrowse').addEventListener('click', closeBrowse)
$('bRefresh').addEventListener('click', async () => {
  const button = $('bRefresh')
  button.disabled = true
  button.textContent = 'Refreshing…'
  try {
    await loadCatalog()
    await loadStats({ refresh: true })
    openBrowse(state.browse.provider || undefined)
    toast('Catalog and speed data refreshed')
  } catch (err) {
    toast(`Refresh failed: ${err}`)
  } finally {
    button.disabled = false
    button.textContent = 'Refresh catalogs'
  }
})

$('browseScrim').addEventListener('click', (event) => {
  if (event.target === $('browseScrim')) return closeBrowse()

  const add = event.target.closest('.row-add')
  if (add) {
    const row = add.closest('.row')
    const ref = { provider: row.dataset.provider || '', id: row.dataset.model }
    const at = state.pool.findIndex(
      (r) => r.id === ref.id && (!r.provider || r.provider === ref.provider)
    )
    if (at >= 0) state.pool.splice(at, 1)
    else state.pool.push(ref)
    savePool()
    renderBrowse()
    renderSidebar()
    toast(at >= 0 ? `${ref.id} removed from your pool` : `${ref.id} added to your pool`)
    return
  }

  // The padlock sits inside the header button, so it has to be checked first —
  // a click on it reaches both.
  const lock = event.target.closest('.col-lock')
  if (lock) {
    unlockSort(lock.dataset.unlock)
    renderBrowse()
    return
  }

  const column = event.target.closest('.col-sort')
  if (column) {
    lockSort(column.dataset.sort)
    renderBrowse()
    return
  }

  const filter = event.target.closest('#bFilters .filter')
  if (filter) {
    const key = filter.dataset.filter
    state.browse.filters[key] = !state.browse.filters[key]
    filter.classList.toggle('is-active', state.browse.filters[key])
    renderBrowse()
  }
})

$('bSearch').addEventListener('input', (e) => { state.browse.search = e.target.value; renderBrowse() })
$('bAuthor').addEventListener('change', (e) => { state.browse.author = e.target.value; renderBrowse() })
$('bProvider').addEventListener('change', (e) => { state.browse.provider = e.target.value; renderBrowse() })
$('bContext').addEventListener('change', (e) => { state.browse.context = Number(e.target.value); renderBrowse() })
$('bPrice').addEventListener('change', (e) => { state.browse.price = e.target.value; renderBrowse() })

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !$('browseScrim').hidden) closeBrowse()
})

// ------------------------------------------------------------------ persistence

/** Written on every change. Only the three fields that define a hall go to
 *  disk — anything derived from the models is looked up fresh on load, so a
 *  stale price or a renamed model can never be baked into the file. */
async function saveLanes() {
  try {
    await api.lanesWrite(
      state.lanes.map(({ slug, name, members, criteria, suppress_reasoning, unstick }) => ({
        slug,
        name,
        members: members.map(({ provider, id, params, disabled }) => ({
          provider,
          id,
          params: params || {},
          disabled: !!disabled,
        })),
        criteria: criteria || [],
        suppress_reasoning: !!suppress_reasoning,
        unstick: !!unstick,
      }))
    )
  } catch (err) {
    toast(`Could not save: ${err}`)
  }
}

async function loadLanes() {
  try {
    state.lanes = (await api.lanesRead()) || []
  } catch (err) {
    state.lanes = []
  }
  // The backend already normalises old files, but normalise here too so this
  // code never has to wonder which shape it is holding.
  state.lanes.forEach((hall) => (hall.members = (hall.members || []).map(asRef)))
  renderLanes()
}

// ---------------------------------------------------------------------- refresh

async function refresh() {
  if (drag.active) return
  // The engine's incident file rides along on the same tick — it is a small
  // local read, and issues appearing on the canvas within seconds of
  // happening is what makes them trustworthy.
  try {
    state.incidents = (await api.incidentsRead()) || []
  } catch (err) {
    // The canvas without issue counts still works; stale is fine here.
  }
  const data = await api.readGateway()
  state.connected = data.connected
  state.error = data.error || null
  state.models = (data.models || []).map((m) => ({ ...m, source: 'gateway' }))
  state.traffic = data.traffic || { requests: 0, failures: 0 }
  state.gateway = data.gateway || ''
  state.updatedAt = Date.now()

  // The live hall feed. Polled incrementally (only what is newer than the
  // high-water mark), and a change here forces a repaint below — a hall that
  // starts or finishes serving must redraw even when nothing else moved.
  let activityChanged = false
  try {
    const fresh = (await api.activityRead(state.activitySeenAt)) || []
    if (fresh.length) {
      state.activitySeenAt = Math.max(...fresh.map((e) => e.at || 0))
      // Keep the feed small: the canvas reads the newest handful per hall.
      state.activity = state.activity.concat(fresh).slice(-400)
      activityChanged = true
    }
  } catch (err) {
    // No live feed still leaves a working canvas; the feed is enhancement.
  }

  // New incidents become notifications on every poll — toasts for what just
  // happened, the bell's count for what faded unseen. This runs regardless
  // of the repaint decision below: news is news even when the canvas has
  // nothing new to draw.
  processIncidents()

  // Repaint only when this poll actually changed something. Rebuilding
  // identical DOM every four seconds is not just waste — it eats input: a
  // click needs its element to survive from press to release, and a tick
  // that swaps the tree mid-click silently kills it. A button that fails
  // one time in fifty and never under scrutiny is this bug, and it was
  // caught by an automated click losing the race that a finger usually wins.
  //
  // The signature must not carry volatile fields. `updatedAt` is stamped
  // fresh on every poll, and `traffic` ticks with every request — including
  // the poll's own — so signing the raw `data` renders on every tick even
  // when the engine has nothing to say. Only the parts that change WHAT is
  // drawn belong in the comparison.
  const signature = JSON.stringify([
    data.connected,
    data.error || null,
    data.gateway || '',
    (data.models || []).map((m) => [m.id, m.healthy, m.available, m.tps]),
    state.incidents.length,
  ])

  // A render that destroys the element being edited is a bug even when the
  // data changed. If the user is mid-rename, hold the repaint until they
  // finish — the focusout handler saves and re-renders on its own beat.
  const editing = document.activeElement &&
    document.activeElement.closest &&
    document.activeElement.closest('.hall-name[contenteditable]')

  if ((activityChanged || signature !== refresh.last) && !editing) {
    refresh.last = signature
    render()
    // Keep an open notification center current — evidence a few seconds old
    // is evidence; evidence from before the last five failures is a trap.
    if (!$('notifScrim').hidden) renderNotifCenter()
  }
}

document.addEventListener('click', async (event) => {
  if (!event.target.closest('#statGateway')) return
  await api.copy(state.gateway || '')
  toast('Gateway address copied')
})

// Populated here rather than beside `renderProviders`, where it was: `PRESETS`
// is a `const` declared further down the file, and a `const` cannot be read
// before its declaration line. Calling it early threw a ReferenceError that
// killed every line after it — the provider form, the browser, the sort
// headers — with the UI still rendering perfectly and no button working.
fillPresetOptions()

async function bootstrap() {
  // The port is part of every endpoint URL, so load it before the first paint
  // rather than briefly advertising the default while persisted state loads.
  await loadPort()
  await loadLanes()
  await loadPool()
  render()
  loadStats()
  loadProviders().then(() => {
    renderProviders()
    loadCatalog()
  })
  refresh()
  setInterval(refresh, 4000)
}

// ---------------------------------------------------------------- the field
//
// Esoteric Generative Luxury's contract: the interface's motion is the
// software's own state, rendered. Three sources feed the field:
//
//   1. THE PIET FEED. The program below is painted in the six hues of the
//      field's palette (see egl.js). It is executed here, once, and its
//      terminal stack top sets the reaction–diffusion feed rate — the
//      building's metabolism is the output of a program painted in its own
//      colours.
//   2. TRAFFIC. Live lane activity disturbs the chemistry at the hall's own
//      position in the field; the ripple is the request.
//   3. THE HAND. The cursor leans the stage and glows the field.

const thePiet = (() => {
  const HUES = 'RYGCBM'
  // A 6×3 painting. Each letter is a codel of that hue; a prime is the dark
  // row. Executed by the interpreter beneath it.
  const PAINTING = [
    ['R', 'Y', 'G', 'C', 'B', 'M'],
    ['R′', 'R′', 'G′', 'C′', 'B′', 'M′'],
    ['R', 'Y', 'G', 'G′', 'B', 'M'],
  ].map((row) => row.map((c) => ({ hue: HUES.indexOf(c[0]), light: c.endsWith('′') ? 1 : 0 })))

  function run(grid, maxSteps = 120) {
    const H = grid.length, W = grid[0].length
    let x = 0, y = 0, dp = 0, cc = 0
    const stack = []
    const DIRS = [[1, 0], [0, 1], [-1, 0], [0, -1]]
    for (let step = 0; step < maxSteps; step++) {
      const here = grid[y][x]
      const block = []
      const seen = new Set([`${x},${y}`])
      const q = [[x, y]]
      while (q.length) {
        const [cx, cy] = q.pop()
        block.push([cx, cy])
        for (const [dx, dy] of DIRS) {
          const nx = cx + dx, ny = cy + dy, k = `${nx},${ny}`
          if (nx < 0 || ny < 0 || nx >= W || ny >= H || seen.has(k)) continue
          if (grid[ny][nx].hue === here.hue && grid[ny][nx].light === here.light) {
            seen.add(k); q.push([nx, ny])
          }
        }
      }
      let edge = block
      if (dp === 0) { const m = Math.max(...block.map((c) => c[0])); edge = block.filter((c) => c[0] === m) }
      if (dp === 2) { const m = Math.min(...block.map((c) => c[0])); edge = block.filter((c) => c[0] === m) }
      if (dp === 1) { const m = Math.max(...block.map((c) => c[1])); edge = block.filter((c) => c[1] === m) }
      if (dp === 3) { const m = Math.min(...block.map((c) => c[1])); edge = block.filter((c) => c[1] === m) }
      edge.sort(([ax, ay], [bx, by]) => {
        const ka = dp === 0 || dp === 2 ? ay : ax
        const kb = dp === 0 || dp === 2 ? by : bx
        const forward = dp === 0 || dp === 1
        const sign = forward ? (cc ? 1 : -1) : (cc ? -1 : 1)
        return (ka - kb) * sign
      })
      const [cx, cy] = edge[0]
      const nx = cx + DIRS[dp][0], ny = cy + DIRS[dp][1]
      if (nx < 0 || ny < 0 || nx >= W || ny >= H) {
        if (cc === 0) cc = 1; else { cc = 0; dp = (dp + 1) % 4 }
        continue
      }
      const there = grid[ny][nx]
      const dh = (there.hue - here.hue + 6) % 6
      const dl = (there.light - here.light + 3) % 3
      const cmds = [
        [null, 'push', 'pop'],
        ['add', 'sub', 'mul'],
        ['div', 'mod', 'not'],
        ['gt', 'ptr', 'sw'],
        ['dup', 'roll', 'inn'],
        ['inc', 'outc', 'outn'],
      ]
      const cmd = cmds[dh][dl]
      const pop1 = () => (stack.length ? stack.pop() : 0)
      switch (cmd) {
        case 'push': stack.push(block.length); break
        case 'pop': pop1(); break
        case 'add': stack.push(pop1() + pop1()); break
        case 'sub': { const a = pop1(); stack.push(pop1() - a); break }
        case 'mul': stack.push(pop1() * pop1()); break
        case 'div': { const a = pop1() || 1; stack.push(Math.trunc(pop1() / a)); break }
        case 'mod': { const a = pop1() || 1; const b = pop1(); stack.push(((b % a) + a) % a); break }
        case 'not': stack.push(pop1() === 0 ? 1 : 0); break
        case 'gt': { const a = pop1(); stack.push(pop1() > a ? 1 : 0); break }
        case 'ptr': dp = (((dp + pop1()) % 4) + 4) % 4; break
        case 'sw': cc = (cc + Math.abs(pop1())) % 2; break
        case 'dup': stack.push(stack.length ? stack[stack.length - 1] : 0); break
        default: break
      }
      x = nx; y = ny
    }
    return stack
  }
  return run(PAINTING)
})()

const field = (() => {
  if (!window.EGL || !EGL.ok) return { disturb: () => {}, tick: () => {} }
  EGL.mount(document.body)
  // The executed program's terminal stack top, wrapped into 0..1, is the
  // feed. A different painting would metabolise the building differently.
  EGL.setFeed(((thePiet[thePiet.length - 1] || 0) % 7) / 7)
  const reduced = matchMedia('(prefers-reduced-motion: reduce)')

  // The hand leans the stage: a fraction of a degree, heavy, behind a
  // long-tail ease. Reduced-motion keeps the building still.
  const stage = $('stage')
  let tx = 0.5, ty = 0.5
  addEventListener('pointermove', (e) => {
    if (reduced.matches) return
    tx = e.clientX / innerWidth
    ty = e.clientY / innerHeight
    EGL.setCursor(tx, ty)
    stage.style.setProperty('--tilt-y', `${((tx - 0.5) * 1.6).toFixed(3)}deg`)
    stage.style.setProperty('--tilt-x', `${((0.5 - ty) * 1.1).toFixed(3)}deg`)
  }, { passive: true })

  return {
    disturb: (x, y, amt) => EGL.disturb(x, y, amt),
  }
})()

// Traffic disturbs the chemistry. The freshest "trying" in the activity feed
// seeds the field at its hall's position — the ripple *is* the request.
let lastRippleAt = 0
function rippleFromActivity() {
  const trying = state.activity.find((e) => e.phase === 'trying')
  if (!trying) return
  const now = Date.now()
  if (now - lastRippleAt < 900) return
  lastRippleAt = now
  const hallEl = document.querySelector(`.hall[data-hall="${trying.hall}"]`)
  if (!hallEl) return
  const r = hallEl.getBoundingClientRect()
  field.disturb((r.left + r.width / 2) / innerWidth, (r.top + r.height / 2) / innerHeight, 0.85)
  EGL && EGL.ok && EGL.setHashRate(Math.min(1, state.activity.length / 12))
}

bootstrap()
// The "updated Ns ago" clock ticks on its own; a repaint every second just to
// age a label would throw away scroll positions for nothing.
setInterval(renderUpdated, 1000)
// Live hall states decay: a "trying…" older than its freshness window, or an
// "answered" past its linger, disappears on this beat even when no poll has
// anything new to say. Cheap enough to ride the same one-second clock.
setInterval(() => {
  if (state.activity.some((e) => e.at > Date.now() / 1000 - 50)) renderLanes()
}, 1000)
// The field's chemistry follows the work, on the same clock.
setInterval(rippleFromActivity, 700)
