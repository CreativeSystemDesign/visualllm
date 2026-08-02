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

/** Per-token prices are unreadable. Everyone reasons in dollars per million. */
function fmtPrice(model) {
  if (model.free) return 'free'
  if (model.price_out == null) return null
  const perMillion = model.price_out * 1e6
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

  const bits = []
  const tps = fmtTps(model.tps)
  if (tps) bits.push(tps)
  const price = fmtPrice(model)
  if (price) bits.push(price)
  bits.push(fmtContext(model.context))
  // Show whatever the list is currently ranked by, so the ordering is legible
  // rather than something you have to take on trust.
  const ranked = model[state.sort]
  if (SORT_LABEL[state.sort] && ranked != null) {
    bits.push(`${SORT_LABEL[state.sort]} ${Math.round(ranked)}`)
  }
  if (model.source !== 'catalog' && !model.available && model.reason) bits.push(model.reason)

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

  let models = allModels().filter((m) => {
    if (q && !m.id.toLowerCase().includes(q) && !(m.name || '').toLowerCase().includes(q)) {
      return false
    }
    if (state.filters.free && !m.free) return false
    if (state.filters.vision && !m.vision) return false
    if (state.filters.tools && !m.tools) return false
    return true
  })

  if (state.sort === 'name') models.sort((a, b) => a.id.localeCompare(b.id))
  else if (state.sort === 'price') {
    models.sort((a, b) => {
      const x = a.free ? 0 : a.price_out
      const y = b.free ? 0 : b.price_out
      if (x == null && y == null) return a.id.localeCompare(b.id)
      if (x == null) return 1
      if (y == null) return -1
      return x - y
    })
  } else models.sort(byDescending(state.sort === 'speed' ? 'tps' : state.sort))

  list.innerHTML = ''
  if (!models.length) {
    const hasSource = state.models.length || state.catalog.length
    list.innerHTML = `<div class="empty-state"><strong>${
      hasSource ? 'No matches' : 'No models yet'
    }</strong>${
      hasSource
        ? 'Loosen the search or filters.'
        : 'Add a provider to pull in its catalog.'
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
    <span class="lane-kind ${lane.computed ? 'computed' : ''}">${
      lane.computed ? lane.kind : 'ordered'
    }</span>
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

function beginDrag(event, chip) {
  const id = chip.dataset.model
  const model = modelById(id)
  if (!model) return

  const laneEl = chip.closest('.lane')
  const rect = chip.getBoundingClientRect()

  drag.active = true
  drag.model = model
  drag.from = laneEl ? laneEl.dataset.lane : null
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

  window.addEventListener('pointermove', onDragMove)
  window.addEventListener('pointerup', endDrag, { once: true })
}

function onDragMove(event) {
  if (!drag.active) return
  drag.ghost.style.left = `${event.clientX - drag.offsetX}px`
  drag.ghost.style.top = `${event.clientY - drag.offsetY}px`

  const under = document.elementFromPoint(event.clientX, event.clientY)
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

  const chips = [...track.querySelectorAll('.chip:not(.is-source)')]
  let slot = chips.length
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

/** DOM runs right-to-left, so slot `p` of `n` chips is index `n - p`. */
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

  if (index === 0) toast(`${model.id} answers first in ${lane.name}`)
  render()
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
    return
  }

  const chip = event.target.closest('.chip')
  if (chip) {
    event.preventDefault()
    beginDrag(event, chip)
  }
})

document.addEventListener('click', async (event) => {
  const url = event.target.closest('.lane-url')
  if (url) {
    const slug = url.closest('.lane').dataset.lane
    await api.copy(`http://127.0.0.1:4000/lane/${slug}/v1/chat/completions`)
    toast('Endpoint URL copied')
    return
  }

  const filter = event.target.closest('.filter')
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
  state.lanes.unshift({ slug, name: 'New lane', members: [], kind: 'ladder', computed: false })
  render()
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
  $('pUrl').value = PRESET.openrouter.url
  $('pUrl').placeholder = PRESET.openrouter.url
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
  $('pKind').value = provider.kind
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

// Each service has a home and a key format. Filling them in beats asking
// someone to look them up.
const PRESET = {
  openrouter: { url: 'https://openrouter.ai/api/v1', key: 'sk-or-…', name: 'OpenRouter' },
  anthropic:  { url: 'https://api.anthropic.com/v1', key: 'sk-ant-…', name: 'Anthropic' },
  openai:     { url: 'https://api.openai.com/v1',    key: 'sk-…',     name: 'OpenAI' },
  compatible: { url: '',                             key: 'sk-…',     name: '' },
}

$('pKind').addEventListener('change', (e) => {
  const preset = PRESET[e.target.value] || PRESET.compatible
  $('pUrl').placeholder = preset.url || 'https://api.example.com/v1'
  $('pKey').placeholder = preset.key
  // Only fill what the user has not typed over.
  if (!state.editing) {
    const previous = Object.values(PRESET).map((p) => p.url)
    if (!$('pUrl').value || previous.includes($('pUrl').value)) $('pUrl').value = preset.url
    const names = Object.values(PRESET).map((p) => p.name).filter(Boolean)
    if (!$('pName').value || names.includes($('pName').value)) $('pName').value = preset.name
  }
})

$('pTest').addEventListener('click', async () => {
  note('testing…')
  try {
    const found = await api.providerTest($('pKind').value, $('pUrl').value, $('pKey').value)
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
      kind: $('pKind').value,
      base_url: $('pUrl').value,
      // Blank while editing means "keep what is stored"; blank on a new one is
      // a genuinely empty key.
      key: state.editing && !key ? null : key,
    })
    await loadProviders()
    resetForm()
    note('saved', 'ok')
    loadCatalog()
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

  // Reload lanes only on the first read; after that the canvas is the user's.
  if (!state.lanes.length && data.lanes?.length) state.lanes = data.lanes
  render()
}

document.addEventListener('click', async (event) => {
  if (!event.target.closest('#statGateway')) return
  await api.copy(state.gateway || '')
  toast('Gateway address copied')
})

loadProviders().then(() => {
  renderProviders()
  loadCatalog()
})

refresh()
setInterval(refresh, 4000)
// The "updated Ns ago" clock ticks on its own; a repaint every second just to
// age a label would throw away scroll positions for nothing.
setInterval(renderUpdated, 1000)
