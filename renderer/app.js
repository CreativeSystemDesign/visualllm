/* VisualLLM — canvas logic.
 *
 * Two rules drive everything below:
 *   1. A lane is an ordered list of models. members[0] answers first.
 *   2. The track draws that list right to left, so the primary sits at the
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

      providersList: () => T.core.invoke('providers_list'),
      providerSave: (input) => T.core.invoke('provider_save', { input }),
      providerDelete: (id) => T.core.invoke('provider_delete', { id }),
      providerTest: (kind, baseUrl, key) =>
        T.core.invoke('provider_test', { kind, baseUrl, key }),
      catalogRead: (id) => T.core.invoke('catalog_read', { id: id ?? null }),
      lanesRead: () => T.core.invoke('lanes_read'),
      lanesWrite: (lanes) => T.core.invoke('lanes_write', { lanes }),
      poolRead: () => T.core.invoke('pool_read'),
      poolWrite: (ids) => T.core.invoke('pool_write', { ids }),
      statsRead: () => T.core.invoke('stats_read'),
      statsRefresh: () => T.core.invoke('stats_refresh'),
    }
  : window.vll

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
  stats: {},
  statsFetchedAt: 0,
  pool: [],             // model ids the user kept — the sidebar shows only these
  browse: {             // the browser's own controls, separate from the sidebar's
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

/** Everything the sidebar can offer: what the gateway runs, plus every
 *  provider catalog. Gateway lanes win a name collision — they carry live
 *  health and measured throughput, which a catalog entry never will. */
function allModels() {
  const seen = new Set(state.models.map((m) => m.id))
  return state.models.concat(state.catalog.filter((m) => !seen.has(m.id)))
}

const modelById = (id) => allModels().find((m) => m.id === id)

const slugify = (s) =>
  s.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'lane'

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
}

// ------------------------------------------------------------------ rendering

function chipEl(model, { inTrack = false, rank = null } = {}) {
  const el = document.createElement('div')
  el.className = 'chip'
  el.dataset.model = model.id
  el.dataset.class = model.klass
  if (inTrack) el.classList.add('in-track')
  if (rank === 1) el.classList.add('is-primary')

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
  // leaving a chip looking identical to a scored one.
  if (SORT_LABEL[state.sort]) {
    const ranked = model[state.sort]
    bits.push(ranked == null ? `${SORT_LABEL[state.sort]} unrated` : `${SORT_LABEL[state.sort]} ${Math.round(ranked)}`)
  }
  if (model.source !== 'catalog' && !model.available && model.reason) bits.push(model.reason)

  if (model.description) el.title = model.description

  const dot = health(model)
  el.innerHTML = `
    <span class="chip-bar"></span>
    ${rank ? `<span class="rank">${rank}</span>` : ''}
    <span class="chip-body">
      <span class="chip-name">${model.id}</span>
      <span class="chip-meta">${bits
        .map((b, i) => (i ? `<span class="sep">·</span>${b}` : b))
        .join(' ')}</span>
    </span>
    ${dot ? `<span class="chip-health ${dot}" title="${dot}"></span>` : ''}
    ${inTrack ? `<button class="chip-remove" title="Remove">${ICON.close}</button>` : ''}
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
  let models = allModels().filter((m) => {
    if (m.source === 'catalog' && !state.pool.includes(m.id)) return false
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
    const hasPool = state.pool.length || state.models.length
    list.innerHTML = `<div class="empty-state"><strong>${
      hasPool ? 'No matches' : 'Your pool is empty'
    }</strong>${
      hasPool
        ? 'Loosen the search or filters.'
        : 'Browse models and add the ones you want.'
    }</div>`
  } else {
    // The full OpenRouter catalog is hundreds of rows; building every chip up
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

function renderTrack(track, lane) {
  track.innerHTML = ''
  track.appendChild(Object.assign(document.createElement('div'), { className: 'flow' }))

  if (!lane.members.length) {
    track.insertAdjacentHTML(
      'beforeend',
      `<div class="track-empty"><span class="box"></span><span>Drop a model here — it becomes the one that answers.</span></div>`
    )
    return
  }

  // members[0] answers first, so it is drawn last: right-hand edge.
  ;[...lane.members].reverse().forEach((id, domIndex) => {
    const model = modelById(id)
    if (!model) return
    const rank = lane.members.length - domIndex
    track.appendChild(chipEl(model, { inTrack: true, rank }))
  })

  track.insertAdjacentHTML(
    'beforeend',
    `<span class="answers-first">${ICON.arrow} answers first</span>`
  )
}

function laneEl(lane) {
  const el = document.createElement('article')
  el.className = 'lane'
  el.dataset.lane = lane.slug

  const head = document.createElement('div')
  head.className = 'lane-head'
  head.innerHTML = `
    <span class="lane-name" contenteditable="plaintext-only" spellcheck="false">${lane.name}</span>
    <button class="lane-url" title="Copy endpoint URL">
      ${ICON.copy}<span class="host">127.0.0.1:4000</span><span>/lane/${lane.slug}/v1</span>
    </button>
    ${(lane.criteria || []).length ? `<span class="lane-criteria" title="What this lane was built for. Click to search the catalog with these criteria — it does not change the lane.">${
      lane.criteria.map((c) => `<span class="crit">${criterionWords(c)}</span>`).join('')
    }</span>` : ''}
    <span class="lane-kind ${lane.computed ? 'computed' : ''}">${
      lane.computed ? lane.kind : 'ordered'
    }</span>
    <button class="lane-remove" title="Delete lane">${ICON.close}</button>
  `

  const track = document.createElement('div')
  track.className = 'track'
  renderTrack(track, lane)

  el.append(head, track)
  return el
}

function renderLanes() {
  const host = $('lanes')

  // A refresh rebuilds the DOM, which would otherwise throw every track back to
  // the left. Remember where each one was so the view does not jump under you.
  const scrolls = new Map()
  host.querySelectorAll('.lane').forEach((el) => {
    const track = el.querySelector('.track')
    if (track) scrolls.set(el.dataset.lane, track.scrollLeft)
  })

  host.innerHTML = ''
  if (!state.lanes.length) {
    host.innerHTML = `<div class="empty-state"><strong>No lanes yet</strong>Create one, then drag models into it.</div>`
    return
  }
  state.lanes.forEach((lane) => host.appendChild(laneEl(lane)))

  // A track long enough to scroll starts at its right-hand end: the model that
  // answers first is the one worth seeing, and it lives at that edge.
  host.querySelectorAll('.lane').forEach((el) => {
    const track = el.querySelector('.track')
    if (!track) return
    const previous = scrolls.get(el.dataset.lane)
    track.scrollLeft = previous === undefined ? track.scrollWidth : previous
  })
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
  from: null, // lane slug, or null when it came from the sidebar
  ghost: null,
  line: null,
  target: null,
  slot: 0,
  offsetX: 0,
  offsetY: 0,
}

/** Start a drag: build the ghost and start listening for movement. */
function beginDrag(event, chip) {
  const id = chip.dataset.model
  const model = modelById(id)
  if (!model) return

  // `closest` walks UP the tree looking for a match. If the chip is inside a
  // lane we are moving it; if not, it came from the sidebar and we are copying,
  // since a model can appear in many lanes at once.
  const laneEl = chip.closest('.lane')

  // The element's exact position and size on screen right now, in pixels from
  // the top-left of the window. Needed so the ghost appears precisely over the
  // real chip instead of jumping to the cursor.
  const rect = chip.getBoundingClientRect()

  drag.active = true
  drag.model = model
  drag.from = laneEl ? laneEl.dataset.lane : null
  // Where inside the chip you grabbed it. Without this the ghost snaps its
  // corner to the cursor, and a chip grabbed by its right edge jumps left the
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

  chip.classList.add('is-source')
  document.body.classList.add('is-dragging')

  // Listening on `window`, not on the chip. A fast drag outpaces the browser's
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
  const track = under?.closest?.('.track')

  document.querySelectorAll('.lane.is-target').forEach((l) => l.classList.remove('is-target'))
  drag.line?.remove()
  drag.line = null

  if (!track) {
    drag.target = null
    return
  }

  track.closest('.lane').classList.add('is-target')
  drag.target = track

  // Which gap in this track are we hovering over?
  //
  // Compare the cursor against each chip's MIDPOINT, not its edges. Past the
  // halfway line means you intend to land after it. Using edges leaves dead
  // zones between chips where nothing highlights, which reads as broken.
  //
  // The dragged chip itself is excluded (`:not(.is-source)`) — it is about to
  // move, so it should not count as an obstacle to itself.
  const chips = [...track.querySelectorAll('.chip:not(.is-source)')]
  let slot = chips.length // default: past everything, at the far right
  for (let i = 0; i < chips.length; i++) {
    const box = chips[i].getBoundingClientRect()
    if (event.clientX < box.left + box.width / 2) {
      slot = i
      break
    }
  }
  drag.slot = slot

  const line = document.createElement('div')
  line.className = 'drop-line'
  const trackBox = track.getBoundingClientRect()
  let x
  if (!chips.length) x = 18
  else if (slot >= chips.length) {
    const last = chips[chips.length - 1].getBoundingClientRect()
    x = last.right - trackBox.left + 4
  } else {
    x = chips[slot].getBoundingClientRect().left - trackBox.left - 5
  }
  line.style.left = `${x + track.scrollLeft}px`
  track.appendChild(line)
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
  document.querySelectorAll('.lane.is-target').forEach((l) => l.classList.remove('is-target'))
  drag.ghost?.remove()
  drag.line?.remove()

  const { model, from, target, slot } = drag
  drag.active = false
  drag.ghost = drag.line = drag.target = null

  if (!target) {
    // Dropped nowhere. Out of a lane means remove; out of the sidebar means nothing.
    if (from) {
      const lane = state.lanes.find((l) => l.slug === from)
      lane.members = lane.members.filter((m) => m !== model.id)
      toast(`${model.id} removed from ${lane.name}`)
      render()
      saveLanes()
    } else {
      render()
    }
    return
  }

  const lane = state.lanes.find((l) => l.slug === target.closest('.lane').dataset.lane)
  const source = from ? state.lanes.find((l) => l.slug === from) : null

  if (source) source.members = source.members.filter((m) => m !== model.id)
  const without = lane.members.filter((m) => m !== model.id)
  const index = domSlotToIndex(slot, without.length)
  without.splice(index, 0, model.id)
  lane.members = without

  // A lane takes on the question its first models were found by. Only once —
  // a later drag from a different search should not silently rewrite what the
  // lane says it is for.
  if (!lane.criteria?.length && state.browse.sorts?.length) {
    lane.criteria = state.browse.sorts
      .filter((s) => s.field !== 'name')
      .map(({ field, desc }) => ({ field, desc }))
  }

  if (index === 0) toast(`${model.id} answers first in ${lane.name}`)
  render()
  saveLanes()
}

// ------------------------------------------------------------------ interaction

function toast(message) {
  const el = $('toast')
  el.textContent = message
  el.classList.add('show')
  clearTimeout(toast._t)
  toast._t = setTimeout(() => el.classList.remove('show'), 2000)
}

document.addEventListener('pointerdown', (event) => {
  if (event.button !== 0) return

  const remove = event.target.closest('.chip-remove')
  if (remove) {
    const chip = remove.closest('.chip')
    const lane = state.lanes.find((l) => l.slug === chip.closest('.lane').dataset.lane)
    lane.members = lane.members.filter((m) => m !== chip.dataset.model)
    render()
    saveLanes()
    return
  }

  const chip = event.target.closest('.chip')
  if (chip) {
    event.preventDefault()
    beginDrag(event, chip)
  }
})

document.addEventListener('click', async (event) => {
  const remove = event.target.closest('.lane-remove')
  if (remove) {
    const lane = state.lanes.find((l) => l.slug === remove.closest('.lane').dataset.lane)
    state.lanes = state.lanes.filter((l) => l !== lane)
    render()
    saveLanes()
    toast(`${lane.name} deleted`)
    return
  }

  // Opens a FRESH SEARCH with these criteria. Deliberately not a rebuild of
  // this lane: with several providers configured its members can come from
  // catalogs that publish different metrics, so re-running the criteria would
  // silently drop the ones nothing is published about. The chips are a record
  // of intent; this is a convenient way to ask the same thing again and see
  // what turns up today.
  const crit = event.target.closest('.lane-criteria')
  if (crit) {
    const lane = state.lanes.find((l) => l.slug === crit.closest('.lane').dataset.lane)
    if (lane?.criteria?.length) {
      state.browse.sorts = lane.criteria.map(({ field, desc }) => ({ field, desc }))
      openBrowse()
      toast(`Searching: ${lane.criteria.map(criterionWords).join(' + ')}`)
    }
    return
  }

  const url = event.target.closest('.lane-url')
  if (url) {
    const slug = url.closest('.lane').dataset.lane
    await api.copy(`http://127.0.0.1:4000/lane/${slug}/v1/chat/completions`)
    toast('Endpoint URL copied')
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
  const filter = event.target.closest('#filters .filter')
  if (filter) {
    const key = filter.dataset.filter
    state.filters[key] = !state.filters[key]
    filter.classList.toggle('is-active', state.filters[key])
    renderSidebar()
  }
})

$('sort').addEventListener('change', (e) => {
  state.sort = e.target.value
  renderSidebar()
})

document.addEventListener('focusout', (event) => {
  const name = event.target.closest?.('.lane-name')
  if (!name) return
  const lane = state.lanes.find((l) => l.slug === name.closest('.lane').dataset.lane)
  const next = name.textContent.trim()
  // The slug is fixed at creation: renaming must never move a live endpoint.
  lane.name = next || lane.name
  name.textContent = lane.name
  saveLanes()
})

document.addEventListener('keydown', (event) => {
  const name = event.target.closest?.('.lane-name')
  if (name && event.key === 'Enter') {
    event.preventDefault()
    name.blur()
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
  state.lanes.unshift({ slug, name: 'New lane', members: [] })
  render()
  saveLanes()
  const el = document.querySelector(`.lane[data-lane="${slug}"] .lane-name`)
  el?.focus()
  document.getSelection()?.selectAllChildren(el)
})

$('wcMin').addEventListener('click', () => api.minimize())
$('wcMax').addEventListener('click', () => api.toggleMaximize())
$('wcClose').addEventListener('click', () => api.close())


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
    list.innerHTML = `<div class="empty-state"><strong>No providers yet</strong>Add one below and its models load straight into the sidebar.</div>`
    return
  }
  state.providers.forEach((provider) => {
    const count = state.counts[provider.id]
    const failed = typeof count === 'string'
    const el = document.createElement('div')
    el.className = `provider${state.editing === provider.id ? ' is-editing' : ''}`
    el.dataset.provider = provider.id
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

    const counts = {}
    state.catalog.forEach((m) => (counts[m.provider_id] = (counts[m.provider_id] || 0) + 1))
    state.catalogErrors.forEach((e) => (counts[e.provider_id] = e.error))
    state.counts = counts
    mergeStats()

    // Refresh if we have never fetched, or the figures are over an hour old —
    // they describe the last thirty minutes, so anything older is fiction.
    const age = Date.now() / 1000 - (state.statsFetchedAt || 0)
    if (age > 3600) loadStats({ refresh: true })
  } catch (err) {
    state.catalogErrors = [{ provider_name: 'catalog', error: String(err) }]
  }
  renderSidebar()
  renderProviders()
}

$('openProviders').addEventListener('click', openPanel)
$('closeProviders').addEventListener('click', closePanel)
$('pCancel').addEventListener('click', resetForm)

$('scrim').addEventListener('click', (event) => {
  if (event.target === $('scrim')) closePanel()
  const row = event.target.closest('.provider')
  if (row) editProvider(row.dataset.provider)
})

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !$('scrim').hidden) closePanel()
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
    await api.providerSave({
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
    // Straight into browsing: a provider with no models chosen from it does
    // nothing, so the next step should not have to be found.
    await loadCatalog()
    closePanel()
    openBrowse()
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
 * The header says "out/M ↑" because it labels a column of numbers. A lane
 * header has to say "cheapest", because it is describing what the lane is for
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

function rowEl(model) {
  const pooled = state.pool.includes(model.id)
  const el = document.createElement('div')
  el.className = `row${pooled ? ' is-pooled' : ''}`
  el.dataset.model = model.id

  // The vendor's own description, on hover. It is the fastest way to tell two
  // similarly-scored models apart, and it costs nothing — we already cache it.
  if (model.description) el.title = model.description

  const inPrice = model.price_in != null && model.price_in >= 0
    ? (model.price_in * 1e6 === 0 ? 'free' : `$${(model.price_in * 1e6).toFixed(2)}`)
    : '—'
  const outPrice = fmtPrice(model) || '—'

  const score = (v) => (v == null ? '—' : Math.round(v))
  // Latency reads better in seconds past a second; nobody compares 3018ms.
  const ttft = model.latency == null ? '—'
    : model.latency >= 1000 ? `${(model.latency / 1000).toFixed(1)}s`
    : `${Math.round(model.latency)}ms`

  // With two or more criteria the composite is the thing actually being sorted
  // on, so it has to be visible — otherwise the order looks arbitrary.
  const criteria = state.browse.sorts.filter((s) => s.field !== 'name')
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
        <span class="row-author">${model.author || ''}</span>
      </span>
      <span class="row-id">${model.id}</span>
    </span>

    <span class="row-metrics">
      ${metric(score(model.intelligence), 'intel', model.intelligence == null)}
      ${metric(score(model.coding), 'code', model.coding == null)}
      ${metric(score(model.agentic), 'agent', model.agentic == null)}
      ${metric(fmtContext(model.context), 'ctx')}
      ${metric(inPrice, 'in/M')}
      ${metric(outPrice.replace('/M', ''), 'out/M')}
      ${metric(model.throughput == null ? '—' : Math.round(model.throughput), 'tok/s', model.throughput == null)}
      ${metric(ttft, 'ttft', model.latency == null)}
      ${metric(model.providers || '—', 'hosts', !model.providers)}
      ${metric(fmtCreated(model.created), 'age', true)}
    </span>

    <span class="row-caps">
      ${CAP_ICONS.map(([key, label]) =>
        `<span class="cap ${model[key] ? 'on' : ''}" title="${key}">${label}</span>`
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
      else if (key === 'pooled' && !state.pool.includes(m.id)) return false
      else if (key !== 'rated' && key !== 'pooled' && !m[key]) return false
    }
    return true
  })

  const nameSort = b.sorts.find((s) => s.field === 'name')
  const criteria = b.sorts.filter((s) => s.field !== 'name')

  // Name is not a criterion — there is no such thing as being 80th-percentile
  // alphabetical. Locked alone it sorts; locked alongside others it is ignored.
  if (!criteria.length) {
    state.browse.scores = new Map()
    models.sort((x, y) =>
      nameSort && !nameSort.desc ? x.id.localeCompare(y.id) : y.id.localeCompare(x.id))
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

/** Price for sorting: free is zero, negative means "variable" and is unknown. */
function priceValue(model, field) {
  if (model.free) return 0
  const raw = model[field]
  return raw == null || raw < 0 ? null : raw
}

/**
 * One comparator for every column, with direction as a parameter.
 *
 * The rule worth stating: A MISSING VALUE ALWAYS SORTS LAST, in both
 * directions. It is tempting to treat null as negative infinity and let it
 * flip with everything else, but "unrated" is not the same as "worst" — 230 of
 * 337 models carry no benchmark score, and reversing the sort should not bury
 * every rated model beneath them.
 */
function comparator(field, desc) {
  return (a, b) => {
    if (field === 'name') {
      return desc ? b.id.localeCompare(a.id) : a.id.localeCompare(b.id)
    }

    let x, y
    if (field === 'price' || field === 'price_in') {
      const key = field === 'price' ? 'price_out' : 'price_in'
      x = priceValue(a, key)
      y = priceValue(b, key)
    } else {
      x = a[field]
      y = b[field]
    }

    if (x == null && y == null) return 0   // a tie: let the next column decide
    if (x == null) return 1
    if (y == null) return -1
    return desc ? y - x : x - y
  }
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

/** Which end of a column counts as good, before the user reverses it. */
const NATURAL_DESC = (field) =>
  !['price', 'price_in', 'name', 'latency'].includes(field)

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
  return model[field]
}

/**
 * Percentile rank per criterion: 1 is best, 0 is worst, computed only across
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
 * would rank it worst at something we cannot measure at all — a different and
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

/** A clickable header row, aligned to the metric columns beneath it.
 *
 * Reuses the same classes as a model row, so the widths line up by
 * construction rather than by two sets of numbers that drift apart. */
function renderBrowseHeader() {
  const sorts = state.browse.sorts
  const rank = (field) => sorts.findIndex((s) => s.field === field)

  const LOCK = '<svg viewBox="0 0 12 12"><rect x="2.5" y="5.5" width="7" height="5" rx="1.2"/><path d="M4.2 5.5V4a1.8 1.8 0 0 1 3.6 0v1.5"/></svg>'

  const label = (field, text) => {
    const at = rank(field)
    if (at < 0) return text
    const arrow = sorts[at].desc ? '↓' : '↑'
    // No priority number any more: every locked column weighs the same, so
    // there is no order to convey.
    const lock = `<span class="col-lock" data-unlock="${field}" title="Remove from criteria">${LOCK}</span>`
    return `${text} ${arrow}${lock}`
  }

  const criteriaCount = sorts.filter((s) => s.field !== 'name').length
  $('bHeader').innerHTML = `
    ${criteriaCount > 1 ? '<span class="match head-match"><span class="match-label">match</span></span>' : ''}
    <span class="row-body">
      <button class="col-sort col-name${rank('name') >= 0 ? ' is-sorted' : ''}" data-sort="name">
        <span class="metric-label">${label('name', 'model')}</span>
      </button>
      <span class="head-hint">${
        state.browse.sorts.filter((s) => s.field !== 'name').length > 1
          ? 'ranked by how well each model does on every locked column at once'
          : 'click to lock a column in · click again to reverse · padlock to remove'
      }</span>
    </span>
    <span class="row-metrics">
      ${COLUMNS.map(([key, text]) => `
        <button class="metric col-sort${rank(key) >= 0 ? ' is-sorted' : ''}" data-sort="${key}">
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
  renderBrowseHeader()


  $('bCount').textContent = `${models.length} of ${state.catalog.length} models · ${state.pool.length} in your pool`

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
  models.slice(0, CAP).forEach((m) => list.appendChild(rowEl(m)))
  if (models.length > CAP) {
    list.insertAdjacentHTML('beforeend',
      `<div class="empty-state">${models.length - CAP} more — narrow the search to see them.</div>`)
  }
}

function openBrowse() {
  $('browseScrim').hidden = false
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
    state.pool = (await api.poolRead()) || []
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

$('openBrowse').addEventListener('click', openBrowse)
$('closeBrowse').addEventListener('click', closeBrowse)

$('browseScrim').addEventListener('click', (event) => {
  if (event.target === $('browseScrim')) return closeBrowse()

  const add = event.target.closest('.row-add')
  if (add) {
    const id = add.closest('.row').dataset.model
    const at = state.pool.indexOf(id)
    if (at >= 0) state.pool.splice(at, 1)
    else state.pool.push(id)
    savePool()
    renderBrowse()
    renderSidebar()
    toast(at >= 0 ? `${id} removed from your pool` : `${id} added to your pool`)
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
$('bContext').addEventListener('change', (e) => { state.browse.context = Number(e.target.value); renderBrowse() })
$('bPrice').addEventListener('change', (e) => { state.browse.price = e.target.value; renderBrowse() })

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !$('browseScrim').hidden) closeBrowse()
})

// ------------------------------------------------------------------ persistence

/** Written on every change. Only the three fields that define a lane go to
 *  disk — anything derived from the models is looked up fresh on load, so a
 *  stale price or a renamed model can never be baked into the file. */
async function saveLanes() {
  try {
    await api.lanesWrite(
      state.lanes.map(({ slug, name, members, criteria }) => ({
        slug, name, members, criteria: criteria || [],
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
  renderLanes()
}

// ---------------------------------------------------------------------- refresh

async function refresh() {
  if (drag.active) return
  const data = await api.readGateway()
  state.connected = data.connected
  state.error = data.error || null
  state.models = (data.models || []).map((m) => ({ ...m, source: 'gateway' }))
  state.traffic = data.traffic || { requests: 0, failures: 0 }
  state.gateway = data.gateway || ''
  state.updatedAt = Date.now()

  render()
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

loadLanes()
loadPool().then(renderSidebar)
loadStats()
loadProviders().then(() => {
  renderProviders()
  loadCatalog()
})

refresh()
setInterval(refresh, 4000)
// The "updated Ns ago" clock ticks on its own; a repaint every second just to
// age a label would throw away scroll positions for nothing.
setInterval(renderUpdated, 1000)
