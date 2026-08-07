# VisualLLM Implementation Plan

Pickup document for a fresh session. This plan is written so the next session can
execute each item without re-doing the codebase review. Read **Context** first,
then follow **Phase 1** in order. Verify each phase with the commands in
**Definition of Done**.

Source of truth: a full code review was delivered 2026-08-06 against commit
`5f404c7` (branch `ide-integration`, the current work branch). Every item below
maps back to a specific finding in that review, not to a guess.

---

## Context (read this first)

### What the product is
VisualLLM is a Tauri 2.x desktop app: a visual fallback router for LLMs.
Models are arranged into horizontal "lanes"; each lane is a local
OpenAI-compatible endpoint. In a lane the **rightmost** model is tried first and
every member to its left is a fallback. The engine runs a loopback HTTP server
(`http://127.0.0.1:<port>/lane/<slug>/v1/chat/completions`) that editors like
VS Code call through `chatLanguageModels.json`.

### Architecture (file map)
- `src-tauri/src/main.rs` — Tauri commands, state (`PortableState`), editor
  integration (`vscode_integrate_lane` writes `chatLanguageModels.json`),
  capability-gap decision, `port.json` (engine port), loopback auth-less gateway.
- `src-tauri/src/server.rs` — the axum engine: `chat()` route, per-member
  fallback loop, timeouts (CONNECT 10s / STREAM 120s / BLOCKING 300s),
  incidents (ring of 200), per-lane traffic stats, `x-visualllm-trail` header,
  `GET /activity` JSONL tail.
- `src-tauri/src/providers.rs` — provider registry + keyring (OS keychain)
  integration; `save()` writes blank keys to providers.json and stores real keys
  in the keyring.
- `src-tauri/src/lanes.rs` — lane model (`Lane`, `Member`, params, criteria).
- `src-tauri/src/incidents.rs` — incident ring buffer, kinds, mute rules.
- `src-tauri/src/state.rs` — `port.json`, `StateFile` persistence.
- `renderer/index.html` — the EGL-skin UI shell; loads only `egl.css`.
- `renderer/egl.css` — active skin (~3300 lines).
- `renderer/app.js` — all canvas/UI logic (~3238 lines, signature-based repaint).
- `renderer/style.css` — **dead code** (2066 lines, old neumorphic skin; nothing
  loads it).
- `renderer-backup-original/` — **dead code** (backup of an older renderer).
- `tools/smoke.js` — renderer smoke test (loads index.html with stubs).
- `tools/preview.js` — builds a single HTML preview of the app with a
  `window.vll` seam; used for quick manual testing and future e2e.
- `.github/workflows/release.yml` — tag-gated release build (deb/AppImage/nsis/
  dmg); macOS job signs the DMG but does **not** notarize.
- `.github/workflows/ci.yml` — renderer smoke + `cargo fmt`/`clippy`/`test` +
  package validation.
- `src-tauri/tauri.conf.json` — CSP `default-src 'none'`, transparent window,
  updater pubkey/endpoints.
- `src-tauri/capabilities/default.json` — renderer capabilities.

### The current invariant set (do not break)
1. **Display reversal**: the rightmost model answers first. `renderTrack` and
   `domSlotToIndex` in `app.js` reverse logical order to DOM order. Any renderer
   edit must preserve this.
2. **Keys are one-way**: provider API keys live ONLY in the OS keyring
   (providers.rs). They are never written to providers.json (which stores blank
   keys) and never exposed to the webview (export/import strips them).
3. **Capability-gap semantics**: a model that cannot serve a request (e.g. no
   vision) is skipped silently as a `skipped_by_catalog`, never recorded as an
   incident, and the passed-over story lives on the lane activity line — not the
   notification center.
4. **The probe budget**: lane test uses a 64-token probe, above the `budget <
   16` gate bypass, so Test actually measures the commit gate.
5. **Compatibility**: existing lanes, `port.json`, and `chatLanguageModels.json`
   files must keep working across versions.

### Verified baseline (2026-08-06)
- `npm run smoke` passes.
- `cargo test` = **52 passed, 0 failed**.
- `cargo fmt --check` and `cargo clippy -- -D warnings` — run before any merge;
  baseline was clean on the review commit.
- Single canonical remote: `origin` → `https://github.com/CreativeSystemDesign/visualllm.git`
  (dual-remote with `CreativeSystemsDevelopment` was removed 2026-08-06; the
  dev account is deleted/logged out everywhere).

### Release posture
Version is `0.4.0` (Cargo.toml, tauri.conf.json, package.json). No public
release has shipped. The product is pre-1.0; see `ROADMAP.md` (workstream plan,
release criteria) and `RELEASE_ROADMAP.md` (1.0 editor-integration work).

---

## Phase 0 — Hygiene (already done, do not redo)

- Removed git remotes `creativesystemsdevelopment` and `creativesystemdesign`;
  kept single `origin`. `main` re-pointed to track `origin/main`.
- Deleted GitHub repo `CreativeSystemsDevelopment/VisualLLM`; logged the dev
  gh account out; removed stale `branch.<name>.vscode-merge-base` git-config
  keys; verified via API that no visualllm artifacts remain under the dev
  account.
- Added `docs/IMPLEMENTATION_PLAN.md` (this file).

---

## Phase 1 — Confirmed issues (fix these first; each is a finding from the review)

### 1.1 Remove the dead theme toggle (renderer-only)
**Finding:** `renderer/index.html:75` renders a `themeToggle` button and
`egl.css:977-978` styles `[data-theme="dark"]` glyph swaps, but there is **no
handler anywhere in `app.js`** — the button does nothing. `<body
data-theme="dark">` is hardcoded at `index.html:12`. The active skin is
dark-first; the old light theme lives only in the dead `style.css`.
**Decision:** remove the button and its CSS rules. Do not build a light theme
for the EGL skin in this cycle — the skin is a dark installation and
half-baked light theming is worse than none.
**Steps:**
- In `index.html`, delete the `themeToggle` button element (and any id/class
  references to it).
- In `egl.css`, delete the `[data-theme="dark"]` frame-btn glyph rules at
  ~:977-978 and the `#themeToggle` styles if present.
- Grep `app.js` for `theme`/`themeToggle` to confirm nothing listens for it;
  remove dead handlers if found.
**Verify:** `npm run smoke`; `grep -n themeToggle renderer/*` returns nothing.

**Status: done (2026-08-06).** Button, `[data-theme="dark"]` glyph rules, and any
handlers removed; grep is clean.

### 1.2 Degrade gracefully when the OS keyring is unavailable (backend)
**Finding:** `providers.rs:175` `keyring_set(&provider.id, &provider.key)?`
hard-fails `provider_save` (which propagates through `main.rs:626`/`:664`).
On Linux boxes with no working Secret Service the user **cannot save a
provider** — a show-stopper, and the app fails at "add provider", the most
basic flow.
**Fix:** make keyring failure non-fatal:
- In `providers.rs:save()`, change the keyring loop (~:172-176) so a
  `keyring::Error` is logged (continue using the existing `eprintln!` pattern
  at ~:189) instead of returned.
- Surface the degraded state to the UI: add a `key_storage: String` field to
  the `ProviderView` struct returned to the renderer (e.g. `"keyring"` on
  success, `"memory"` on keyring failure).
- In `main.rs`, have `provider_save` return the view with `key_storage` set;
  do not `?`-propagate keyring errors.
- Renderer: when `key_storage == "memory"`, show a toast after save:
  "Key stored in memory only — OS keychain unavailable. Re-enter after
  restart." (keys stay in-memory for the session only).
- `forget_key` (`providers.rs:739`) must remain best-effort and silent on
  error — it already is; verify.
**Note on security posture:** do NOT fall back to writing keys into
providers.json. Principle 4 in `ROADMAP.md`: "Local means local" — the webview
must never receive secrets, and plaintext key files would leak them.
**Verify:** `cargo test`. Manual: launch with `DBUS_SESSION_BUS_ADDRESS=`
unset (no Secret Service) and confirm a provider saves with the memory toast;
confirm a normal session still reports `keyring`.

**Status: done (2026-08-06).** Keyring failure is non-fatal (logged, not
propagated); `ProviderView.key_storage` is `"keyring"` or `"memory"`; the
renderer toasts the memory-only warning; `forget_key` stays best-effort. Tested
with the degraded path covered in `cargo test`.

### 1.3 Delete dead frontend assets
**Finding:** `renderer/style.css` (2066 lines) is not referenced by
`index.html` (which loads only `egl.css`) and `renderer-backup-original/` is a
backup of an older renderer. Both are dead weight in the shipped bundle.
**Steps:**
- `git rm renderer/style.css` and `git rm -r renderer-backup-original/`.
- Grep the whole repo for `style.css` references:
  - `renderer/index.html` (must be clean),
  - `tools/preview.js` (it currently inlines whichever CSS files exist — remove
    the `style.css` read, keep `egl.css`),
  - `tools/smoke.js`, docs, CI — fix any dangling reference.
**Verify:** `npm run smoke`; `cargo build` (frontendDist still assembles);
run `node tools/preview.js` and open the output to confirm styling is intact.

**Status: done (2026-08-06).** `renderer/style.css` and
`renderer-backup-original/` removed; `tools/preview.js` reads only `egl.css`;
preview output confirmed styled.

### 1.4 Remove the stray zero-length `git` file
**Finding:** a zero-length file literally named `git` sits in the repo root
(no extension). It has no content and no purpose.
**Step:** `git rm git` (after confirming it is empty and untracked-by-content).
**Verify:** `git status` clean; `ls` shows no `git` file.

**Status: done (2026-08-06).** Stray zero-length `git` file removed from the
repo root.

### 1.5 macOS auto-update: sign properly or disable the path
**Finding:** `release.yml` macOS job (~:186) code-signs only the DMG and does
**not** notarize the `.app`; Tauri's updater on macOS requires a signed +
notarized bundle. As shipped, macOS "auto-update" would be broken.
**Decision:** do not spend money on an Apple Developer account now. Make macOS
not pretend to auto-update:
- Gate the macOS release job to manual (`workflow_dispatch`) OR make it skip
  the updater artifacts so the app doesn't advertise an update channel.
- Update `RELEASE_ROADMAP.md` with an explicit "macOS distribution deferred"
  note (needs Apple Developer ID + notarization before auto-update is real).
- Keep Linux + Windows updater paths (they need only the minisign artifacts the
  workflow already emits; Windows shows SmartScreen without signing — document,
  don't block).
**Verify:** `release.yml` is valid YAML (parse it); no CI run needed for a
workflow-only change.

**Status: done (2026-08-06).** macOS distribution deferred and documented in
`RELEASE_ROADMAP.md` §7; macOS stays out of the supported update channel and
the macOS jobs remain in `release.yml` for manual runs.

---

## Phase 2 — Real gaps (trust + testability)

### 2.1 Loopback engine auth (backend + renderer + docs)
**Finding:** the engine binds `127.0.0.1:<port>` with **no authentication**.
Any local process (or website via DNS rebinding) can POST to a lane and burn
credits / trigger paid models. This is a trust gap, not a cosmetic one.
**Design:**
- On first run, generate a random secret: 32 random bytes hex-encoded. Store
  it in the app data dir next to `port.json` (e.g. `secret.json`), chmod 600.
- Require `Authorization: Bearer <secret>` (or a dedicated
  `x-visualllm-token` header) on engine routes. Return 401 on missing/bad
  token with a plain-text explanation pointing at the UI settings.
- Editor integration must include the header: `vscode_merge_lane` /
  `vscode_integrate_lane` write the URL *and* `httpHeaders` into the
  `chatLanguageModels.json` entry (VS Code's schema supports `httpHeaders`).
  Re-run integration on upgrade so existing files gain the header.
- UI: a settings row "Gateway token" with reveal-copy (warning: anyone with
  this can call your lanes) and a "regenerate" button.
- Version/breaking note: existing `chatLanguageModels.json` entries and any
  hand-written clients will break until re-integrated. This is an accepted
  cost for a pre-1.0 app; call it out in the README + release notes. Keep
  `GET /activity` and health endpoints token-free (they leak only what the
  renderer already sees).
**Files:** `src-tauri/src/server.rs` (middleware), `src-tauri/src/state.rs`
(secret persistence), `src-tauri/src/main.rs` (commands, editor writer),
`renderer/app.js` + `renderer/index.html` (settings row),
`README.md` / `docs/editor-integration.md` (docs).
**Verify:** unit tests for the token check (valid / missing / wrong / non-loopback
origin); integration test that a no-token request gets 401 and a token request
passes; `cargo test`; re-integrate a lane and confirm the editor file contains
the header.

**Status: done (2026-08-06).** Random 32-byte secret persisted
(`secret.json`, chmod 600); `Authorization: Bearer` required on `/lane` routes
(middleware in `server.rs`, token-free `/health`, `/activity`, `/v1/models`);
editor integration writes `httpHeaders`; renderer settings row with
reveal/copy/regenerate. `cargo test` = 57 passed including token
missing/wrong/valid and open-endpoint tests.

### 2.2 Unit-test the renderer's scoring/sort logic (renderer)
**Finding:** the whole renderer is 3238 lines with `tools/smoke.js` as the only
test (it loads the file and checks it parses). The scoring (`scoreModels`),
`visibleColumns`, `browseMatches`, `pricePerMillion`, `fmtAge`, and the
display-reversal helpers are untested.
**Approach (least invasive):** `tools/smoke.js` already loads `app.js` under a
stub harness; extend that pattern instead of refactoring app.js into modules.
- Add `tools/renderer.test.js` using Node's built-in `node:test`:
  - Load `app.js` with the same DOM/`window.vll` stubs smoke.js uses.
  - Unit-test pure functions on a realistic catalog fixture: percentile
    scoring (ties, unjudged/null entries), `visibleColumns` (data-driven vs
    locked, persistence), `browseMatches` filters, `pricePerMillion` free
    handling, `domSlotToIndex` reversal correctness.
- Add npm script `"test": "node --test tools/*.test.js"`.
**Verify:** `npm test` green; `npm run smoke` still green.

**Status: done (2026-08-06).** `tools/renderer.test.js` loads app.js under the
smoke harness (via `vm.runInThisContext` + an export line) and covers 14 cases:
price/free handling, percentile scoring incl. ties and nulls, data-driven vs
locked columns, browse filters, and drag-reversal. `npm test` green; smoke
unaffected.

### 2.3 Version single-source-of-truth check (CI)
**Finding:** version lives in three places (Cargo.toml, tauri.conf.json,
package.json); `release.yml` only cross-checks the tag against tauri.conf.
**Fix:** add a small script `tools/check-version.js` that reads all three and
exits non-zero on mismatch; wire it into `ci.yml` (and the release job's
pre-flight) so drift fails CI instead of shipping.
**Verify:** `node tools/check-version.js` passes on a matched tree; fails when
one file is bumped alone.

**Status: done (2026-08-06).** `tools/check-version.js` reads Cargo.toml,
tauri.conf.json, and package.json, exits non-zero on mismatch. Wired into
`ci.yml` (renderer job) and the release job's tag pre-flight. Both pass/fail
paths verified locally.

### 2.4 (Optional, low priority) Preview e2e with Playwright
**Finding:** no end-to-end path exists; only smoke + manual `tools/preview.js`.
**Add (only if CI budget allows):** a small Playwright suite (devDependency)
that opens the preview HTML and asserts: vault populates from fixture, a sort
header click reorders, gallery opens, a synthetic incident produces a toast.
Mark the suite `local-only` if the user wants to keep CI lean — the engine
cannot be exercised headlessly without a full Tauri run, so keep this to the
renderer/preview seam.
**Verify:** `npx playwright test` locally passes.

---

## Phase 3 — Ranked feature ideas (after Phase 1+2 land)

**Status: started (2026-08-06).** 3.1 is complete. The remaining items below
are ranked candidate features — pick them up after the user confirms the next
priority order.

Build in this order; each builds on data the engine already emits.

### 3.1 Per-lane failure budgets / auto-park
**Context:** incidents ring (200) already stores per-lane failures with kinds;
the renderer already renders a "parked" chip and a park toggle. The gap is
automation: nothing parks a lane when it fails repeatedly.
**Design:** for each lane, track recent failures (kinds: `midstream_error`,
`rate_limited`, `out_of_credit`, `timeout`, not `skipped_by_catalog`) in a
rolling window (default: 5 in 10 min). When the budget is hit, automatically
set the lane's parked flag, record an incident `auto_parked`, and let the user
unpark (which resets the budget). Make budget/window configurable per lane
(lanes.json schema extension). Surface an "auto" badge on the parked chip and
show "parked after N failures" as the incident detail. This directly supports
ROADMAP principle 1 (routing should be visible) and 3 (fallback must be
honest).
**Files:** `src-tauri/src/server.rs` (failure classification + park decision),
`src-tauri/src/lanes.rs` (schema), `src-tauri/src/incidents.rs` (kind),
`renderer/app.js` (badge/detail).
**Verify:** pure-function unit test for the budget decision; server test that N
consecutive failures park the lane and an unpark resets it.

**Status: done (2026-08-06).** Implementation notes, where the plan and the
shipped shape differ:
- **Budgetable kinds are narrower than the plan's list.** The plan named
  `midstream_error`; the shipped `incidents::counts_toward_budget` counts only
  *transient, provider-side* failures — `provider_trouble`, `unreachable`,
  `stalled`, `rate_limited`. Behavioural failures (reasoning burn, empty
  responses, loops, a request the model cannot handle) are NOT budgeted: the
  answer to those is to change the lane, not to park it and stop the traffic.
  `out_of_credit` (billing) and `midstream_error` are treated as behavioural
  for the same reason — parking won't fix an empty account.
- **Parking is lane-level, not per-member.** The pre-existing "parked" UI is
  per-member (`Member.disabled`, a tuning choice); auto-park sets a NEW lane
  flag (`Lane.parked` + `parked_after`), which answers 503 with `error.type =
  lane_parked` before any member is contacted. The two states are independent
  and both survive on the lane.
- **Budget bookkeeping lives on the lane** (`budget` = `{failures,
  window_secs}`, `budget_hits` = timestamps), written by the engine on every
  budgetable failure so the sliding window survives restarts. `lanes.rs`:
  `over_budget()` is the pure, unit-tested decision.
- **Unpark resets the budget.** `lane_unpark` tauri command (registered in
  `main.rs`) → `lanes::unpark` clears `parked`, `parked_after`, and
  `budget_hits`. The header shows an amber "Parked — resume" button; the
  notification center's `auto_parked` card carries an "Unpark this endpoint"
  action. Both call the same `unparkLane()` path in the renderer.
- **The `auto_parked` incident is visible, not silent** — it notifies like
  every other kind, so the ring keeps the cause and the canvas explains it.
  Renderer `DIAGNOSIS.auto_parked` renders the engine's receipt
  ("N budgetable failures within the last Xs").

**Tests:** `lanes::over_budget` sliding-window unit test; `park`/`unpark` disk
round-trip; `incidents::counts_toward_budget` kind classifier;
`server` integration tests: N consecutive 503s park the lane (provider not
called again), unpark clears the budget and the lane runs again.

### 3.2 Request replay in the notification center
**Context:** incidents carry evidence text but not a structured request
snapshot, so a failed request cannot be retried from the UI.
**Design:** extend the incident record with an optional `replay` field
(captured method/path/body, capped, local-only, never shown to the webview in
full unless the user clicks "Replay"). Add a "Replay" action in the
notification center that re-POSTs through the lane and shows the resulting
trail. Replaying spends money — require a confirmation click.
**Files:** `src-tauri/src/server.rs` (capture + replay command),
`src-tauri/src/incidents.rs` (schema), `renderer/app.js` (action + confirm).
**Verify:** server test that replay re-enters the lane and records a new
incident/trail; manual confirmation-gate test.

**Status: done (2026-08-06).** Implementation notes, where the plan and the
shipped shape differ:
- **The replay body never reaches the webview.** `incidents_read` now returns
  an `IncidentView` (main.rs) carrying every field the renderer reads plus a
  `replayable` flag — never the captured request. `lane_replay` runs entirely
  server-side: it looks the incident up by its new `id`, re-POSTs the captured
  body through the lane's own endpoint with the engine's gateway token, and
  returns only status / served-by / trail / message. A credential a client put
  in its request can never cross the bridge.
- **Incidents gained an `id`** (assigned on record in `incidents.rs`, CSPRNG
  suffix so same-second failures stay distinct) and an optional
  `replay: {method, path, body}` — both `#[serde(default)]`, so older
  incidents.json files load unchanged (and are simply not replayable).
  `REPLAY_BODY_CAP` (32 KiB) bounds the file: an oversized body is still an
  incident, just not a replayable one — enforced in `record()` so no caller
  can grow the file unbounded.
- **Pre-existing `hall` bug fixed.** Incidents serialize the lane as `lane`
  but the renderer always read `incident.hall` — so per-hall filtering, toast
  meta, and the no-think/unpark fixes were `undefined`. The `IncidentView`
  maps `lane → hall`, which repairs every renderer read without touching the
  disk format.
- **`lane_test` now sends the gateway token.** The lane endpoints demand
  `Bearer <token>` when a secret exists; the probe previously measured the
  auth wall and would stack `401 Unauthorized`.
- **Replay is a two-step action, not resend-on-click.** First click arms the
  button ("Click again to confirm — it can spend money"), second fires; the
  armed id lives in `replayArmedId` so a four-second poll can't disarm a
  confirmation already given, and the button disables while in flight. A
  failed replay is recorded as a new incident by the engine and the center
  re-reads and repaints.

**Tests:** incident `id` uniqueness + reload, replay round-trip + cap
enforcement, and migration of records lacking `id`/`replay` (all in
`incidents.rs`); server integration test that a failed member leaves a
replayable snapshot (method/path/body verified).

### 3.3 Lane cloning
**Design:** duplicate a lane (new slug, name "X copy"), copying members,
params, and criteria in-app. Do **not** auto-write the editor integration for
the clone (integration stays a deliberate per-lane action).
**Files:** renderer-only (`app.js` + `index.html` action).
**Verify:** `npm run smoke`; manual: clone preserves member order/dials.

**Status: done (2026-08-06).** Implementation notes:
- **The Duplicate button lives in the lane footer**, beside Integrate — a
  pill with its own icon and label, not another icon squeezed into the
  header's already-full action row.
- **A clone carries the whole definition:** member order, each member's
  params and park state, criteria, `suppress_reasoning`/`unstick` toggles,
  and the budget config (`failures`/`window_secs`). Dials and criteria are
  deep copies, so tuning the clone never touches the original.
- **A clone starts clean of everything live:** no `integrated_editors`, no
  `parked`, no `budget_hits`. It is a place to try a fix, not a second copy
  of a lane that is itself parked.
- **Fresh slug generation** reuses the "suffix until unique" pattern
  (`hallway-copy`, `hallway-copy-2`, …) from `newLane`, and the clone is
  inserted right after the original.
- **`cloneLaneShape` is pure** and exported to the test harness, so the
  carry-over contract (order/dials preserved, deep-copied, no editor
  integration, no park state) is pinned down by tests.

**Tests:** four `cloneLaneShape` cases — definition/order/dials intact, deep
copies not references, slug uniqueness, and no integration/park state carried.

### 3.4 Usage/credit line (24h / 7d per-lane counters)
**Context:** `server.rs` already tracks per-lane traffic counters and the
activity feed exists; nothing aggregates spend/volume for the user.
**Design:** persist per-lane rolling counters (requests, failures, approximate
tokens if measurable) in the state file. Render a small line on the lane or in
the plinth/statusbar: "today 42 req · 3 fail · 7d 310 req". Read-only display;
no thresholds in this cycle (budgets are 3.1).
**Files:** `src-tauri/src/state.rs` (counters), `src-tauri/src/server.rs`
(increment), `renderer/app.js` (display).
**Verify:** unit test for counter rollover across the window boundary.

**Status: done (2026-08-06).** Implementation notes, where the plan and the
shipped shape differ:
- **No `state.rs` exists; the counters live on the `Lane`** in `lanes.rs`,
  alongside `budget_hits` — the same "the engine owns this bookkeeping"
  discipline, and the same file. `lanes_read` already serialises the full
  `Lane`, so no new command is needed for the renderer to see them.
- **One line per REQUEST, not per member attempt.** A request that burned
  through three members is one failure, not three. `chat` (server.rs) is now
  a thin counting wrapper around `chat_inner`: it records the request once
  the lane exists (the 404 is answered before the ledger is touched), and
  records the failure when the response status is not success. The one case
  it cannot see — a stream dying after its 200 was committed — is exactly the
  case a coarse credit meter does not need.
- **The ledger is two timestamp lists** (`usage_requests`, `usage_failures`),
  pruned to 7 days on every write (`lanes::prune_usage`), so the file is
  bounded by a week of real traffic. A failed request's timestamp appears in
  both lists; the renderer's pure `usageCounts` counts each window at read
  time, so the "24h / 7d" numbers are never stale.
- **`lanes_write` no longer lets the UI wipe engine bookkeeping.** The
  renderer sends only the fields it understands; a renderer save used to drop
  `budget_hits` (a latent bug this fixes), and would have dropped the ledger
  too. `lanes::merge_engine_owned` now folds the prior file's engine-owned
  fields onto the incoming lanes by slug. A clone (3.3) has no prior entry
  and simply starts its own empty ledger.
- **Display is a footer span, shown only when the lane has moved.** A quiet
  hall keeps the single faint "ready" line instead of a row of zeros:
  `24h 42 req · 3 fail · 7d 310 req`.
- **Approximate tokens were dropped** as planned-if-measurable: streaming
  responses make them unreliable without buffering, and the meter is about
  volume, not spend.

**Tests:** `prune_usage` boundary rollover and `merge_engine_owned` carry-over
(lanes.rs); `usageCounts` window boundary + nesting, and the no-ledger case
(renderer.test.js); a server integration test that a success counts one
request, a 404 counts nothing, and an exhausted lane counts a request and a
failure.

---

## Phase 4 — Post-review hardening batch (second review round, 2026-08-06)

Phases 0–3 above are DONE (~95% executed; marker commit `12808c1`, current
`main` @ `da8f5fa`). A second full read of the codebase (all of `server.rs`
2740 lines, `main.rs` 1668 lines, `renderer/app.js` 3489 lines, `lanes.rs`,
`providers.rs`, `incidents.rs`, `index.html`) produced the seven gaps below.
They are small and mostly independent. Recommended order: **4.1 → 4.3 → 4.2 →
4.4 → 4.5 → 4.6 → 4.7**. Each section lists the finding, the design, the exact
files/lines, the steps, and the tests. If HEAD has moved since `da8f5fa`,
re-verify each referenced line before editing. After each fix run the global
**Definition of Done** below and update `ROADMAP.md`.

### 4.1 Status bar reads the engine, not the dead gateway scaffold (HIGH)

**Finding.** `read_gateway` (main.rs:559-585) GETs `{gateway}/health` and parses
it with `parse()` (main.rs:483-557) against the OLD Python gateway JSON shape
(`raw["lanes"]`, `raw["routes"]`, `raw["telemetry"]`). The engine's `/health`
(server.rs:1168-1175) returns only `{service, ok, lanes: count,
models_cached: count}`. Result: `state.models` and `state.traffic` are always
empty, so `renderStatusBar` (app.js:675-702) shows "no models" and "no traffic
yet" forever. main.rs:415-422 marks `engine_url` as scaffolding "to be removed
once the engine reports its own" — the engine now CAN via the per-lane ledger
(`usage_requests`/`usage_failures`, pruned 7d; lanes.rs:224-250, 356).

**Design.** The engine owns the ledger, so the engine reports the readouts:
- Extend `/health` to also return `models_total` (sum of `lane.members.len()`
  over all lanes) and the trailing-24h aggregates `requests_24h` /
  `failures_24h` (a failed request appears in BOTH lists — lanes.rs:238-248 —
  so `failures <= requests` always; reuse the same boundary as `usageCounts` in
  app.js:508: `now - at < window` counts). Extract the aggregation into a pure
  `fn usage_24h(lanes: &[Lane], now: u64) -> (u64, u64)` so it is unit-testable.
- Rewrite `parse()` (main.rs:483-557) for the new shape; delete the dead
  structs `Model` (main.rs:424-436), `Lane` (main.rs:438-446), `Traffic`
  (main.rs:450-454) and the old branches. Change `State` (main.rs:456-464) to
  `{ connected, gateway, error, lanes: u64, models_total: u64, requests_24h:
  u64, failures_24h: u64 }`. Keep `connected/gateway/error` semantics — the
  renderer's dot, host and offline text depend on them. Fix the stale
  scaffolding comment at main.rs:415-422 (`engine_url` is still needed by
  `read_gateway`; say so). Grep before deleting `as_u64` (main.rs:479-481) — it
  may be used elsewhere.
- `renderStatusBar` (app.js:675-702) reads the new fields:
  - `statModels` → "`N` endpoints · `M` models" (N = `data.lanes`, M =
    `data.models_total`).
  - `statFastest` → the engine has no per-model tps, so compute it
    **client-side** from `state.catalog` rows that carry a measured
    `throughput` after `mergeStats` (the stats cache from
    `providers::hydrate_stats`). Fall back to "no throughput measured yet".
  - `statTraffic` → `data.requests_24h` / `data.failures_24h`, keeping the
    failure-rate line.
- The 4s repaint signature (app.js:3280-3286) currently omits traffic
  deliberately (it ticks on every request). Now the status bar depends on the
  aggregates, and they only change when a request settles — so add
  `data.requests_24h`, `data.failures_24h`, `data.lanes`, `data.models_total`
  to the signature. Also remap `refresh()` (app.js:3238-3244): stop copying
  `data.models`/`data.traffic`; copy the new scalars into `state`.

**Steps.** (1) server.rs: add `usage_24h` + extend `health`; unit-test the pure
fn in the existing server test module. (2) main.rs: rewrite `State`/`parse`,
delete dead structs, fix comment. (3) app.js: `refresh()` mapping + signature.
(4) app.js: `renderStatusBar` rewrite. (5) index.html: no structural change
needed (spans stay; only textContent changes).

**Tests.** `cargo test` (new `usage_24h` cases: empty lanes → 0,0; hits inside
and exactly-a-window-old; a failure counted in both lists). If a pure text
helper emerges (e.g. `statusModelsText`), pin it in `renderer.test.js`. Manual:
`cargo tauri dev` → real counts appear and `requests_24h` climbs after a curl.

### 4.2 Setup curl example omits the auth header (401s) (HIGH)

**Finding.** `laneCurlExample` (app.js:456-458) builds a curl with no
`Authorization` header, but lane routes require `Bearer <token>`
(server.rs:1831-1846, token checked at 1835-1870). Every copied example 401s.
The renderer already can fetch the token via `api.gatewayToken(true)`
(app.js:43) and the copy handler is at app.js:1911-1921.

**Design.** Parameterize `laneCurlExample(slug, token)` to emit
`-H 'Authorization: Bearer <token>'` when the token is non-empty. In the Setup
handler, fetch the token first, then copy. On token error, toast and skip the
copy — a headerless example is broken, not helpful. No new security surface:
the renderer can already reveal the token on demand.

**Steps.** (1) app.js:456-458 — add the token param and the header line.
(2) app.js:1911-1921 — `const token = (await api.gatewayToken(true)).token`,
pass to `laneCurlExample`. (3) Grep the repo for other copy-to-clipboard curl
examples (settings note index.html:310-313 documents the header already;
check any `docs/editor-integration.md` / README curl snippets) and fix any that
omit it.

**Tests.** `renderer.test.js`: `laneCurlExample` is pure — assert the header
line appears with a token and is absent without. Manual via `preview.js`: copy
from a lane, run the curl, expect a real chat response.

### 4.3 (merged into 4.1 — status bar readouts, traffic, fastest)

### 4.4 Auto-park budget is schema-only — surface it in the UI (MEDIUM)

**Finding.** `Lane.budget` (`LaneBudget { failures: u32, window_secs: u64 }`,
lanes.rs:137-150, default 5/600) is written by the renderer on clone
(app.js:1767) and save (app.js:3178) and consumed by the engine
(`consider_auto_park`, server.rs:215-263; `over_budget`, lanes.rs) — but there
is no control anywhere in the UI. Phase 3.1's plan text said "surface it in the
lane header" and it never landed.

**Design.** A lane-scoped popover in the same family as `#memberPop`, opened
from a small button in the hall header (app.js:589-613, between the Setup
button and the toggles). Two number fields: "Auto-park after N failures" and
"within M minutes". No OK button (same philosophy as the member dials,
app.js:1455-1459): editing writes `hall.budget` and calls `saveLanes()`
immediately. Blank a field → reset to the default (5 / 600); `LaneBudget` is a
non-optional Copy struct, so there is deliberately NO "off" switch — that would
be a schema change, out of scope (the budget is "deliberately coarse and
lane-wide" by design, lanes.rs:135-136). Clamp in the renderer: failures
1..9999, window_secs 60..86400 (mirror the `max_tokens` dial).

**Steps.** (1) lanes.rs: verify `over_budget` no-ops only at
`hits.len() < failures` — confirm blanking isn't needed; pin behavior with an
existing/added unit test. (2) index.html: add `#lanePop` next to `#memberPop`
(line ~329) with the two inputs. (3) app.js: add `lanePopTarget { slug }`,
open/close logic mirroring `openMemberPop` (app.js:1476-1489) incl. Escape,
close button and outside-press close; on input, clamp, write `hall.budget`,
`saveLanes()`. (4) renderer/egl.css: mirror the `.dials` rules for the new
popover (two fields, slightly wider). (5) The clone (app.js:1767) and save
(app.js:3178) paths already carry `budget` — no change.

**Tests.** `renderer.test.js`: a pure clamp helper (e.g. `clampBudget`) — test
boundaries and blank→default. Manual: set failures=1/window=60s, hit the lane
with a failing request, watch it auto-park; unpark via header or
notification-center button.

### 4.5 Mute is per-kind globally; ROADMAP claims per-(lane, kind) (MEDIUM)

**Finding.** `notif.muted` is a `Set` of kind strings (app.js:1040,
1434-1437), checked as `notif.muted.has(i.kind)` at app.js:1060 (bell), 1080
(toasts) and 1189 (center), with the mute button carrying only `data-mute=kind`
(app.js:1206). Muting a kind on one lane silences it on every lane. ROADMAP §9
says per-(lane, kind) mute landed.

**Design.** Key mutes as `"${hall}::${kind}"` and add a pure helper
`notifMuted(kind, hall)` that returns true when the set holds the composite key
OR a bare `kind` (legacy entries from before this change keep working). Switch
all four call sites to `notifMuted(i.kind, i.hall)`. The mute button (app.js:1206)
gains `data-mute-hall="${latest.hall}"` and text
"Ignore this type on this lane" / "Ignored on this lane — click to restore";
the handler (app.js:1433-1441) toggles the composite key. Persistence stays in
localStorage key `notif.muted`; `notifPersist` (app.js:1043-1051) writes the set
as-is (legacy bare keys are honored, never rewritten).

**Steps.** (1) app.js: add `notifMuted`, update 1060 / 1080 / 1189 / 1206 /
1433-1441. (2) ROADMAP.md §9 stays as-is (it becomes true).

**Tests.** `renderer.test.js`: `notifMuted` — bare key matches any lane,
composite matches only its lane, absent → false. Manual via `preview.js`: mute
a kind on one lane; confirm another lane still badges/toasts.

### 4.6 Re-integrate all editors after token rotation (MEDIUM)

**Finding.** `gateway_token_regenerate` (main.rs:1330-1340) writes a new secret;
the toast (app.js:2287) says "re-integrate editors" — but re-integration is
manual per lane per editor (`editor_integrate_lane`, main.rs:281-350). After
rotation every existing `chatLanguageModels.json` entry carries a dead token.

**Design.** New command `editor_reapply_token(app)` that walks
`editor_chat_models_paths` (main.rs:148-174) and, for each existing file,
rewrites the `httpHeaders` of every model under a provider named `visualllm` to
`{Authorization: Bearer <current secret>}` and refreshes each `url` to the
current port (`port_load`, main.rs:1218) — port moves break entries the same
way. Other providers and lane order untouched (same discipline as
`vscode_merge_lane`, main.rs:205-220). Write atomically via
`vscode_write_models_at` (main.rs:190-199). Return one `VscodeIntegrationResult`
per editor (reuse main.rs:100) including a lanes-updated count; a missing file
or no `visualllm` provider is a success (0), not an error. Factor the per-editor
rewrite into `vscode_reapply_auth(path, token, port) -> Result<usize, String>`
so the test module (main.rs:1591) can exercise it on temp dirs without touching
real user config.

**Steps.** (1) main.rs: `vscode_reapply_auth` + `editor_reapply_token`; register
in the invoke handler list. (2) app.js: `editorReapplyToken` in the `api` bridge
(app.js:46 area). (3) index.html token panel (314-319): add an "Apply to
editors" button; handler calls it and toasts per-editor results. (4) The regen
handler (app.js:2282-2290): keep regeneration non-destructive, but follow the
"re-integrate editors" toast with an action button "Apply" (toast already
supports actions, app.js:1686/1704-1712).

**Tests.** `vscode_tests` (main.rs:1591): existing `visualllm` entry → headers +
url rewritten; non-visualllm provider untouched; missing file → 0; unparseable
file → Err; a stale-url entry → url refreshed. Manual: integrate a lane, rotate
the token, Apply, open `chatLanguageModels.json`, confirm the new token + url.

### 4.7 Minor: cap the toast stack and pause polling when hidden (LOW)

**Finding.** Failure bursts stack unbounded notif toasts (`showNotifToast`,
app.js:1090-1123). `refresh()` (app.js:3228) and the three 1s/700ms intervals
(app.js:3481, 3485, 3489) keep running while the window is hidden.

**Design.** (1) In `showNotifToast`, before appending, evict the oldest cards
while `$('notifStack').children.length >= 3`. (2) At the top of `refresh()`:
`if (document.hidden) return`; in `bootstrap()` add a `visibilitychange` listener
that calls `refresh()` when the document becomes visible (catches up the 4s
clock). Guard the three short intervals the same way.

**Steps.** app.js only; no backend. Manual: minimize the window, confirm no
polling in devtools network, restore → bar updates.

**Tests.** Manual (above). Not unit-testable without DOM stubs — skip.

---

## Definition of Done (run for the whole tree before committing anything)

```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
npm run smoke
npm test                       # after 2.2
node tools/check-version.js    # after 2.3
node tools/preview.js          # + open the output once for the touched UI
```

Then update `ROADMAP.md` (check off finished items, add new workstreams for the
phases above) and `RELEASE_ROADMAP.md` (macOS note from 1.5, feature timeline
from Phase 3). Commit on `ide-integration` in small, reviewable pieces; the
release guard compares tags to tauri.conf.json only.

---

## Session handoff note
If this is a fresh session: read `README.md` for the product pitch, `handoff.md`
for operational state, and this plan. The codebase review (which produced all
findings above) was delivered against `5f404c7`; if HEAD has moved since,
re-verify each referenced line before editing. The `graphify-out/` knowledge
graph is stale (built Aug 3) — refresh it before relying on it for structure.
