# VisualLLM Action Plan

VisualLLM is a visual fallback router: arrange models into lanes, each lane is a
local OpenAI-compatible endpoint, the rightmost model answers first and
everything to its left is a fallback.

This file replaces the old milestone roadmap. It is a working plan: items are
checked off as they land. Each workstream lists its goal ("done when") up
front, then the concrete tasks.

## Product principles

1. **Routing should be visible.** A user should be able to understand why a
   model was chosen and what happens next when it fails.
2. **The simple path stays simple.** Adding a provider, creating a lane, and
   connecting a client should not require knowledge of YAML, Rust, or HTTP
   internals.
3. **Fallback must be honest.** A response is not successful merely because an
   upstream returned HTTP 200. Evidence, passed-over members, and the serving
   model should remain inspectable.
4. **Local means local.** The webview should not receive provider secrets, and
   the engine should bind to loopback unless the user explicitly chooses
   otherwise.
5. **Compatibility matters.** Existing lanes and OpenAI-compatible clients
   should keep working as the application evolves.

---

## Workstreams

### 1. Stop crying wolf — signal quality

**Done when:** normal, by-design behavior (capability skips) never produces a
toast or bell badge, and every alert that fires is something worth acting on.

- [x] **Don't record `skipped_by_catalog` as an incident.** Capability skips
  now log and appear in the trail header only; legacy records render silently
  (never toast/badge). The missing `stalled` diagnosis was added so dead
  connections explain themselves.
- [x] **Keep skips visible on the lane**, not in the notification center: the
  per-lane activity line carries the live "passed over" story.
- [x] **Verify no other incident kind fires during normal operation.** Manual
  runtime checks confirmed the bell stays silent across mixed traffic.

### 2. Show what the engine already knows

**Done when:** testing a lane and looking at a lane both tell you which model
answered and what was passed over, using data the engine already returns.

- [x] **Lane test shows served-by + trail.** The toast now reads
  `answered by <model>` with the trail as detail on success.
- [x] **Lane test exercises the commit gate.** The probe budget is 64, above
  the `budget < 16` bypass, so Test measures the gate's verdict.
- [x] **Per-lane trail view.** The lane activity line opens the notification
  center scoped to that lane, with a "show all" clear chip.

### 3. Live request visibility — make fallback visible

**Done when:** while a client request is in flight, the lane shows which
member is being tried and, on completion, which member answered and how many
were passed over — without waiting for a failure.

- [x] Engine appends one JSON line per request phase to `activity.jsonl`:
  timestamp, lane, member, phase (trying/answered/failed/exhausted), detail —
  exposed over `GET /activity` and the `activity_read` command.
- [x] Renderer tails it incrementally (high-water mark) and shows a live pill
  on the lane: pulsing dot while trying, `answered by <model> · N passed
  over`, red on failure. States decay on a one-second clock.
- [x] The activity file is scrubbed to one line, capped, and trimmed by size.

### 4. Protect the central interaction

**Done when:** no single click or drag can permanently destroy a lane or a
tuned member.

- [x] **Undo for destructive actions.** Lane delete, chip removal, and
  drag-out-of-lane all toast `X removed — undo?` with a ~5s window restoring
  the previous `state.lanes` (already snapshotted on every `lanes_write`).
- [x] **Right-click context menu on track chips:** settings / disable /
  remove. Surfaces the gear and the drag-out-to-remove gesture, both
  previously invisible.
- [x] **Per-member disable** (skip at request time, keep position and dials)
  for tuning lanes without delete-and-redrag. Engine-side: a `disabled` flag
  on `Member` that `chat()` skips like a capability miss.

### 5. Engine robustness the user can feel

**Done when:** a transient provider outage never silently degrades every
lane's capability checks, and the user is told when it happens.

- [x] **Catalog cache is never shrunk by a partial fetch.** `catalog_read`
  keeps the previous good catalog when a partial fetch would produce a
  strictly smaller set; logs when stale data is retained.
- [x] **Catalog errors surface as a notification** ("<provider> catalog failed
  — using last good cache"), not just a red count in the provider list. A
  stale cache with no active provider errors also toasts once, so a user
  who clears the alert still knows the engine is running from retained data.
- [x] Engine logs one line when serving from a stale cache.

### 6. Window and compositor correctness (z-order)

**Done when:** clicks never fall through to windows below, and the window
stacks normally on X11, Wayland, and when launched as a child of VS Code
Insiders — verified by manual test on each.

- [x] **Input shape is re-applied on resize.** The `input_shape_combine_region`
  call in `main.rs` now runs on `size-allocate`, not only at realize time, so
  the input region tracks window resizes.
- [x] **Stable VS Code is detected.** `vscode_chat_models_path` now checks
  `Code/User/chatLanguageModels.json` as well as `Code - Insiders`.
- [x] **Launch paths documented.** `tools/launch-system.sh` is the supported
  launcher; it runs the binary with a clean environment outside Snap/VSC.
- [x] **Verify transparent window z-order on your hardware.** Confirmed on the
  user's session: clicks land on VisualLLM when stacked over VS Code.

### 7. Single-binary distribution

**Done when:** a user can download one file and run VisualLLM on a clean
supported Linux install without installing WebKitGTK separately.

- [x] Confirm the built binary embeds the renderer (it does — `frontendDist`)
  and needs no repo files at runtime.
- [x] Verify the AppImage bundles WebKitGTK (confirmed by extracting the
  AppImage: `libwebkit2gtk-4.1.so.0`, `libjavascriptcoregtk-4.1.so.0`, and
  `libgtk-3.so.0` are present). Runtime verification on a clean VM still
  needed before tagging.
- [x] Decide: is the AppImage the canonical "single binary"? **Yes for
  portable use.** The AppImage is the recommended download for most Linux
  users; the `.deb` remains available for apt-based installs. Documented in
  README release checklist.
- [x] Add a release checklist entry: run the AppImage on a clean VM before
  tagging.

### 8. Editor integration correctness

**Done when:** the VS Code button works for both VS Code and VS Code Insiders
users, or says clearly why it can't.

- [x] `vscode_chat_models_path` (`main.rs`) detects both stable `Code` and
  `Code - Insiders`, preferring the one that exists.
- [x] On success the toast tells the user to reload the editor window to see
  the model.

### 9. UX polish

**Done when:** the interactions below are discoverable without reading source
comments.

- [x] Track scroll affordance: thin custom scrollbar + left fade on `.track`
  when chips overflow.
- [x] Member popover placeholders show the effective value when known
  (`client: 0.7` / `provider default`) instead of bare `—`.
- [x] Keyboard shortcuts: `Ctrl+N` new lane, `Ctrl+B` browse, `Ctrl+,`
  settings, `?` shortcut help overlay.
- [x] First-lane completion state: "Your endpoint is live at
  `http://127.0.0.1:PORT/lane/<slug>/v1`" with the VS Code setup inline.
- [x] Dead members: lane footer shows "N members will be skipped at request
  time" instead of relying on chip hover.
- [x] Notification center: filter by lane; per-(lane, kind) mute instead of
  global kind mute.
- [x] Sidebar (300) and browse (150) caps gain a "show all" /
  render-on-scroll instead of only "narrow the search".

### 10. Portability, packaging, and launch (carried over)

**Done when:** configuration moves between machines without moving secrets,
the packaged app is verified on a clean install, and the project is ready for
public users.

- [x] Export/import (backup/restore) of lanes, pool, providers — excluding
  API keys (they stay in the keychain; re-entered on the new machine).
- [x] Manual startup verification: clean start, duplicate launch, restart,
  shutdown, documented in README release checklist. Release binary verified:
  second instance exits quickly (single-instance plugin works), but one run
  printed `free(): corrupted unsorted chunks` on kill — needs investigation
  before tagging.
- [ ] Publish tested `.deb` + AppImage; verify desktop-menu launch on a clean
  install.
- [ ] Screenshots or a short add-provider → drag-models → connect-client demo.
- [x] Define the first public release version and support policy; separate
  beginner documentation from implementation history. See README "Version
  policy" and this roadmap's release criteria.
- [x] Add issue labels and a small triage process; invite users to test
  provider setup, lane creation, and fallback behavior. See `CONTRIBUTING.md`.

### 11. Code-review hardening (2026-08-06 review findings)

**Done when:** the confirmed review findings are fixed, the renderer's core
logic is under test, and version drift fails CI instead of shipping.

- [x] **Loopback engine auth.** The gateway now requires
  `Authorization: Bearer <token>` on lane routes (random 32-byte secret,
  persisted next to `port.json`, chmod 600). Editor integration writes the
  header into `chatLanguageModels.json`; the renderer has a reveal/copy/
  regenerate settings row. `/health`, `/activity`, and `/v1/models` stay open.
- [x] **Keyring failure degrades, never blocks.** A provider saves even when the
  OS keychain is unavailable — keys are held in memory for the session and the
  UI says so instead of hard-failing "add provider".
- [x] **Dead assets removed.** The unused `renderer/style.css` (old neumorphic
  skin) and the `renderer-backup-original/` directory no longer ship; only
  `egl.css` is loaded and inlined.
- [x] **Renderer logic is unit-tested.** `npm test` runs `tools/renderer.test.js`
  (node:test) over the pure scoring/sort/price/drag functions — percentile
  scoring with ties and nulls, data-driven columns, browse filters, price/free
  handling, display reversal — on top of the existing smoke test.
- [x] **Version is a single source of truth.** `tools/check-version.js` fails on
  any mismatch between Cargo.toml, tauri.conf.json, and package.json; wired into
  CI and the release tag pre-flight.
- [x] **macOS auto-update not pretended.** Documented in `RELEASE_ROADMAP.md`
  §7: macOS distribution deferred (needs Apple Developer ID + notarization);
  Linux + Windows remain the supported update matrix.
- [x] **Frontend hygiene.** The dead theme toggle is gone; the active skin is
  dark-first by construction, no half-baked light path.

### 12. Auto-park lane failure budgets (2026-08-06)

**Done when:** a lane that keeps failing the same transient way parks itself,
answers 503 until a human unparks it, and the parking is visible and reversible
on the canvas.

- [x] **Per-lane failure budget.** `lanes::Lane.budget` (`failures`/`window_secs`,
  default 5/600) plus `budget_hits` timestamps; `over_budget()` is a pure
  sliding-window decision, unit-tested.
- [x] **Engine parks the lane.** Every budgetable failure (transient,
  provider-side kinds only — `provider_trouble`, `unreachable`, `stalled`,
  `rate_limited`) is recorded and counted; crossing the budget sets `parked` +
  `parked_after` and writes an `auto_parked` incident with the receipt.
  Behavioural failures never park a lane.
- [x] **Parked lanes refuse work.** A parked lane answers `503` with
  `error.type = lane_parked` before any member is contacted; the engine's
  budget bookkeeping survives restarts.
- [x] **Unpark resets the budget.** `lane_unpark` command → `lanes::unpark`
  clears the flag and the history. Header shows an amber "Parked — resume"
  button; the notification center's `auto_parked` card carries an unpark action.
- [x] **Visible and reversible.** `DIAGNOSIS.auto_parked` explains the parking
  with the engine's receipt; the incident notifies like any other kind.

### 13. Request replay in the notification center (2026-08-06)

**Done when:** a failed request leaves a structured snapshot behind, and the
notification center can retry it — confirmation-gated, without the captured
request ever crossing to the webview.

- [x] **Incidents carry a replayable snapshot.** `incidents.rs` records an
  optional `replay` (method/path/body) alongside every failure, plus a stable
  `id` assigned on record. The body is capped at 32 KiB; an oversized request
  is still an incident, just not a replayable one.
- [x] **The webview never sees the request.** `incidents_read` returns an
  `IncidentView` — every field the renderer reads plus a `replayable` flag;
  the captured body stays on the engine's disk. This view also maps the disk's
  `lane` to the canvas's `hall`, fixing per-hall filtering that had silently
  read an undefined field.
- [x] **Replay runs server-side through the lane.** `lane_replay` (main.rs)
  looks the incident up by id, re-POSTs the captured body through the lane's
  own endpoint with the engine's gateway token, and reports status / served-by /
  trail. A replayed failure is recorded as a fresh incident like any other.
- [x] **Confirmation-gated in the UI.** The Replay action is two-step: first
  click arms ("it can spend money"), second fires; the armed id survives
  re-renders and the button disables while in flight.
- [x] **Probes present the token.** `lane_test` sends the gateway bearer token,
  so the Test button measures the lane, not the auth wall.

### 14. Lane cloning (2026-08-06)

**Done when:** one click duplicates a lane — the same members (order, dials and
park state), criteria, toggles and budget — under a fresh slug and name,
beside the original, without pulling in editor integration or live park state.

- [x] **Duplicate button in the lane footer.** A labeled pill beside Integrate
  (`Duplicate`), one click per lane, sharing the footer's management actions
  rather than crowding the header's icon row.
- [x] **A clone is a definition, not a state.** `cloneLaneShape` carries over
  the whole lane definition (members and their params/criteria as deep copies,
  `suppress_reasoning`, `unstick`, budget config) and deliberately omits
  `integrated_editors`, `parked` and `budget_hits` — integrating a lane into an
  editor stays an explicit act, and the clone is a place to try a fix.
- [x] **Fresh slug, beside the original.** Unique-suffix generation
  (`hallway-copy`, `hallway-copy-2`, …) matches `newLane`; the clone is spliced
  right after its source with the same undo/toast as the other destructive
  edits.
- [x] **The carry-over contract is tested.** Four `cloneLaneShape` cases pin
  order/dials preservation, deep-copy isolation, slug uniqueness, and the
  no-integration/no-park guarantees.

### 15. Usage/credit line — rolling 24h / 7d counters (2026-08-06)

**Done when:** each lane shows how much it has actually moved — requests and
failures inside the trailing 24h and 7d — as a read-only line the engine
owns, so no UI edit can ever reset it.

- [x] **The engine keeps the ledger.** `Lane` carries two timestamp lists
  (`usage_requests`, `usage_failures`) pruned to 7 days on every write. One
  line per REQUEST, not per member attempt: `chat` counts the request once
  the lane exists (the 404 answers before the ledger is touched) and marks
  the failure when the response is not success. A stream that dies after its
  200 is committed is the one thing the meter cannot see, and does not need.
- [x] **UI edits can never reset the counters.** `lanes_write` folds the
  prior file's engine-owned fields onto whatever the renderer saves
  (`merge_engine_owned`), which also fixed a latent bug where a renderer save
  dropped `budget_hits`. A clone starts its own empty ledger.
- [x] **A quiet lane stays quiet.** The footer line appears only when the
  lane has moved in the last week — `24h 42 req · 3 fail · 7d 310 req` —
  never as a row of zeros on an idle hall.
- [x] **The windows are pinned by tests.** Boundary rollover in Rust
  (`prune_usage`), window nesting in the renderer (`usageCounts`), the
  merge carry-over, and a server integration test for the request/failure/404
  counting rules.

## Release criteria

A first public release should meet all of these conditions:

- The app installs and launches from a packaged artifact on a clean supported
  Linux system; a second launch behaves cleanly.
- A user can create a lane and copy a working endpoint without guessing.
- Provider keys never appear in the renderer or logs.
- Routing and persistence tests pass without live provider access.
- The README explains the product, installation, first lane, client connection,
  limitations, and security model.
- A failed upstream response produces an understandable diagnosis, and a normal
  capability skip produces none.
- The window stacks correctly on X11, Wayland, and when launched from VS Code.

## Non-goals for the first public release

- Running or downloading model weights locally.
- Becoming a general-purpose model serving platform.
- Exposing the gateway to a LAN or the public internet by default.
- Automatically spending money or selecting paid models without an explicit
  user decision.
- Replacing every provider-specific feature with a universal abstraction.
