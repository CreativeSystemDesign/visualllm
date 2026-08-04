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
- [ ] **Catalog errors surface as a notification** ("<provider> catalog failed
  — using last good cache"), not just a red count in the provider list.
- [ ] Engine logs one line when serving from a stale cache.

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
- [ ] **Verify transparent window z-order on your hardware.** Run the app
  under your normal session (not the agent's Snap shell), stack it over VS
  Code, and confirm clicks land on VisualLLM. If it still falls through,
  flip `"transparent": false` in `src-tauri/tauri.conf.json` and restyle.

### 7. Single-binary distribution

**Done when:** a user can download one file and run VisualLLM on a clean
supported Linux install without installing WebKitGTK separately.

- [ ] Confirm the built binary embeds the renderer (it does — `frontendDist`)
  and needs no repo files at runtime.
- [ ] Verify the AppImage bundles WebKitGTK and runs on a clean VM (no
  `libwebkit2gtk-4.1-dev` installed).
- [ ] Decide: is the AppImage the canonical "single binary"? If yes, make it
  the primary download; keep `.deb` for apt users. Document in README.
- [ ] Add a release checklist entry: run the AppImage on a clean VM before
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
- [ ] Notification center: filter by lane; per-(lane, kind) mute instead of
  global kind mute.
- [ ] Sidebar (300) and browse (150) caps gain a "show all" /
  render-on-scroll instead of only "narrow the search".

### 10. Portability, packaging, and launch (carried over)

**Done when:** configuration moves between machines without moving secrets,
the packaged app is verified on a clean install, and the project is ready for
public users.

- [ ] Export/import (backup/restore) of lanes, pool, providers — excluding
  API keys (they stay in the keychain; re-entered on the new machine).
- [ ] Manual startup verification: clean start, duplicate launch, restart,
  shutdown, on a packaged build.
- [ ] Publish tested `.deb` + AppImage; verify desktop-menu launch on a clean
  install.
- [ ] Screenshots or a short add-provider → drag-models → connect-client demo.
- [ ] Define the first public release version and support policy; separate
  beginner documentation from implementation history.
- [ ] Add issue labels and a small triage process; invite users to test
  provider setup, lane creation, and fallback behavior.

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
