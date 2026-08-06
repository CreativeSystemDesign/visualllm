# VisualLLM — Handoff to Kimi K2.7 Code

**Date:** 2026-08-04  
**Branch:** `ux/action-plan` (8 commits ahead of `main`)  
**Workspace:** `/home/shane/visualllm`  
**Remote:** `https://github.com/CreativeSystemDesign/visualllm.git`

---

## What this project is

VisualLLM is a Linux-first Tauri 2.x desktop app for designing LLM fallback lanes by hand. A user adds providers, browses catalogs, selects models into a pool, then drags models into ordered lanes. Each lane becomes a local OpenAI-compatible endpoint.

The model on the **right answers first**; everything to its left is a fallback. In data terms, `members[0]` is always the primary. The display order is the inverse of storage order — this reversal is intentionally isolated to `renderTrack` and `domSlotToIndex` in `renderer/app.js`.

---

## How to build and run

```bash
# Development (external terminal recommended; see Snap note below)
cd /home/shane/visualllm/src-tauri && ~/.cargo/bin/cargo run

# Release build
~/.cargo/bin/cargo build --release

# Tests / checks
node tools/smoke.js            # renderer smoke test
cd src-tauri && cargo test     # 49 tests
cargo clippy
cargo fmt --check
```

The recommended launcher for the user's normal session is:

```bash
tools/launch-system.sh
```

It runs the compiled binary with a clean environment, detects an existing engine on port `4100`, and avoids duplicate windows.

### Important environment note

The agent's integrated terminal is a **Snap-packaged VS Code Insiders** shell. Running the Tauri binary there fails with:

```
symbol lookup error: /snap/core20/current/lib/x86_64-linux-gnu/libpthread.so.0:
undefined symbol: __libc_pthread_init, version GLIBC_PRIVATE
```

This is **not** an app bug — it is a Snap library-path mismatch. The user's external terminal works fine. Always preview the app the way the user does:

```bash
cd /home/shane/visualllm/src-tauri && ~/.cargo/bin/cargo run
```

---

## Architecture at a glance

| Path | Responsibility |
|---|---|
| `renderer/index.html` | UI structure |
| `renderer/style.css` | Neumorphic design system |
| `renderer/app.js` | All renderer logic: state, rendering, drag-drop, bridge calls |
| `src-tauri/src/main.rs` | Tauri shell and every command the UI may invoke |
| `src-tauri/src/server.rs` | Axum engine, fallback routing, `/activity` feed |
| `src-tauri/src/lanes.rs` | Lane/member/pool persistence |
| `src-tauri/src/providers.rs` | Providers, key storage, catalog fetching |
| `src-tauri/src/incidents.rs` | Failure records |
| `src-tauri/src/loopwatch.rs` | Tool-loop detection |
| `tools/preview.js` | Builds a browser-openable preview from real app data |
| `tools/smoke.js` | Headless renderer smoke test |

### Security model

The webview has **no network and no filesystem access**. All state mutates through the `api` object in `renderer/app.js`, which maps to `#[tauri::command]` functions in `main.rs`. Secrets flow one way: `provider_save` accepts a key; no command ever returns one. The UI sees `ProviderView` with a masked hint only.

---

## Design system

The visual language is neumorphic / soft-UI:

```css
--bg: #eceef1;
--light: #ffffff;
--dark: #c4c9d2;
--accent: #f4645f;
--accent-deep: #d94f4a;
```

Shadows:

```css
--e1: 3px 3px 6px var(--dark), -3px -3px 6px var(--light);
--e2: 6px 6px 12px var(--dark), -6px -6px 12px var(--light);
--e3: 9px 9px 18px var(--dark), -9px -9px 18px var(--light);
--in1: inset 2px 2px 4px var(--dark), inset -2px -2px 4px var(--light);
--in2: inset 4px 4px 8px var(--dark), inset -4px -4px 8px var(--light);
```

Current emphasis: compact, premium, one-line lane headers with indicator lights (`lane-lights` + `.lamp`), a status footer (`lane-foot`), and tight chip spacing.

---

## Recent commits on this branch

```
aae7502  renderer: hide chip menu with [hidden] rule; preview: inline css/js
7ca01e5  lanes: compact one-line header with indicator lights + status footer
93faf51  style: rustfmt across the crate (no behavior change)
1d9d6d2  WS9: UX polish — scroll affordance, lane warnings, shortcuts, first-lane moment
b1bbb19  WS4b: chip context menu + per-member park
6742a8b  WS3: live request visibility + neumorphic pass on new elements
5650fd9  WS5: catalog cache never shrinks silently
ab192b1  UX: silence capability-skip alerts, surface trail/served-by, undo, z-order and editor fixes
```

---

## What got implemented

### WS1 — Stop crying wolf

- `skipped_by_catalog` is no longer recorded as an incident.
- Legacy skipped records render silently (no toast, no bell badge).
- `stalled` diagnosis added for dead connections.
- Per-lane activity line carries the "passed over" story.

### WS2 — Show what the engine knows

- Lane test toast now says `answered by <model>` with trail detail.
- Lane test uses `max_tokens: 64` to exercise the commit gate.
- Per-lane activity line opens the notification center scoped to that lane.

### WS3 — Live request visibility

- Engine writes `activity.jsonl` with phases: `trying`, `answered`, `failed`, `exhausted`.
- `GET /activity` and `activity_read` command exposed.
- Renderer tails incrementally and shows a live pill on each lane.
- File is capped and trimmed by size.

### WS4 — Protect the central interaction

- Undo toast for lane delete, chip removal, drag-out-of-lane (~5s window).
- Right-click context menu on track chips: Member settings / Park / Remove.
- Per-member `disabled` flag in `lanes.rs`; engine skips parked members.

### WS5 — Cache robustness

- `catalog_read` keeps the previous good catalog when a partial fetch would shrink it.
- Logs when stale data is retained.
- Toasts once per distinct failure set: per-provider errors say "<name>: catalog failed — using the last good cache"; a stale cache with no active errors also toasts with the retention timestamp.
- Engine emits one line per request when serving from stale cache: `engine: serving from stale catalog cache retained at <unix>`.

### WS10 — Portability

- Providers panel has Export… / Import… buttons.
- Export writes a JSON file with lanes, pool, and provider config. API keys are never included.
- Import supports Merge (by slug/id, preserving existing keys) and Replace (wipe local state).
- `tauri-plugin-dialog` is used for file picker/save dialogs; capability entries added.

### WS6 — Window / z-order

- `input_shape_combine_region` re-applied on `size-allocate`.
- Stable VS Code path detection added.
- `tools/launch-system.sh` is the supported launcher.

### WS8 — Editor integration

- Detects both `Code` and `Code - Insiders` chatLanguageModels.json.
- Toast prompts user to reload VS Code after integration.

### WS9 — UX polish

- Track scroll affordance (scrollbar + left fade).
- Member popover shows effective placeholder values.
- Shortcuts: `Ctrl+N` new lane, `Ctrl+B` browse, `Ctrl+,` settings, `?` help.
- First-lane completion state with endpoint URL and VS Code inline setup.
- Dead members shown in lane footer.
- Lane headers compacted to one line with indicator lights and status footer.
- Notification center has a lane filter dropdown and per-(lane, kind) mute in addition to global kind mute.
- Sidebar and browse no longer hard-cap at 300/150 rows; they render an initial batch and offer a "Show N more" button, reset when filters/search/sort change.

---

## Known issues / next work

These are the remaining items from `ROADMAP.md`:

2. **WS7 — Single-binary / AppImage verification.** Confirm the AppImage bundles WebKitGTK and runs on a clean VM. Decide if AppImage is the canonical download. Add a release checklist entry.

4. ~~**WS10 — Portability / export-import.**~~ Done: lanes, pool, and providers can be exported and imported; keys stay in the keychain and are not included in the export.

5. **WS9 follow-ups:**
- ~~Notification center: filter by lane; per-(lane, kind) mute.~~ Done.
- ~~Sidebar / browse caps: "show all" or render-on-scroll instead of hard caps.~~ Done.

## Release build verification

A release build was produced with `npm run build`:

- AppImage: `src-tauri/target/release/bundle/appimage/VisualLLM_0.1.0_amd64.AppImage` (80 MB)
- .deb: `src-tauri/target/release/bundle/deb/VisualLLM_0.1.0_amd64.deb` (3.1 MB)

Findings:
- AppImage extraction confirms WebKitGTK, JavaScriptCore, and GTK3 libraries are bundled.
- `.deb` includes desktop entry, icons, and binary; depends on system WebKitGTK/GTK3.
- Release binary single-instance behavior works: a second process exits quickly when one is running.
- One run printed `free(): corrupted unsorted chunks` on shutdown/kill. This needs to be reproduced and investigated before a public release; it may be a GTK/WebKit cleanup ordering issue in the release profile.
- `tools/launch-system.sh` now prefers `target/release/visualllm` when present, falling back to debug.

---

## Key files to touch for common tasks

| Task | Files |
|---|---|
| Change how lanes render | `renderer/app.js` (`laneEl`, `renderTrack`, `laneLights`, `laneFoot`) |
| Change lane styling | `renderer/style.css` (`.lane`, `.lane-head`, `.lane-lights`, `.lane-foot`, `.chip`) |
| Change engine routing | `src-tauri/src/server.rs` |
| Change lane persistence schema | `src-tauri/src/lanes.rs` |
| Change provider/catalog behavior | `src-tauri/src/providers.rs` |
| Add a new command | `src-tauri/src/main.rs` + `renderer/app.js` `api` object |
| Change window/compositor behavior | `src-tauri/src/main.rs` (GTK realize/size-allocate hooks) |
| Preview before/after UI | `node tools/preview.js /tmp/vll-preview.html` |

---

## State files

Stored in `~/.local/share/app.visualllm/`:

- `lanes.json` — version-wrapped lane data (`{ schema_version, data: [...] }`)
- `providers.json` — provider configs (keys in plaintext here only)
- `pool.json` — selected model ids
- `catalog.json` — cached provider catalogs
- `incidents.json` — failure records
- `endpoint-stats.json` — per-model health stats
- `activity.jsonl` — live request activity feed

---

## Testing checklist before PR

```bash
node tools/smoke.js
cd src-tauri && cargo test
cargo clippy
cargo fmt
cargo build --release
```

Then manually verify:

- App launches in external terminal.
- Add provider → catalog populates.
- Create lane → drag models → order is correct.
- Lane test returns "answered by ..." toast.
- Right-click a chip → menu opens → park/resume/settings/remove work.
- Delete lane → undo toast restores it.
- Stack app over VS Code → clicks land on VisualLLM.
- VS Code integration button writes `chatLanguageModels.json` and prompts reload.

---

## Notes for a cheaper model

- Do **not** run the Tauri binary from the agent's Snap terminal.
- The preview harness is in `tools/preview.js`; it now inlines CSS/JS and unwraps versioned state files.
- The chip context menu fix was a single CSS rule: `.chip-menu[hidden] { display: none !important; }`.
- Lane header compacting lives in `laneEl()` and the CSS classes `.lane-head`, `.lane-lights`, `.lane-foot`.
- When in doubt, grep for `members[0]` — primary-on-right is the central invariant.


## Product

VisualLLM is a Linux-first Tauri desktop app that lets users:

1. Add providers.
2. Browse their model catalogs.
3. Select models into a pool.
4. Drag models into ordered fallback lanes.
5. Expose each lane as a local OpenAI-compatible endpoint.

Lane ordering is intentionally reversed visually:

- The model on the **right** answers first.
- Models to the **left** are fallbacks.
- `members[0]` is always the primary model.

The local engine runs on:

`http://127.0.0.1:4100`

Important endpoints:

- `GET /health`
- `GET /v1/models`
- `POST /lane/{slug}/v1/chat/completions`

## Current development setup

The legacy Python gateway is in:

`/home/shane/llm_gateway`

It previously served this conversation through:

`workbench/luna`

That endpoint maps to:

`openai/gpt-5.6-luna`

The gateway lane was added and committed as:

`aa0357f Add isolated GPT-5.6 Luna workbench lane`

The user is now switching to direct OpenRouter access to compare behavior,
especially Loopwatch diagnostics.

## VisualLLM repository state

Recent VisualLLM commits include:

- `0db68fe Add reliable system launcher and startup guidance`
- `ceb2ede Establish public project foundation`
- `ccfa009 Make repeated launches safe`
- `eab7501 Polish project metadata and development docs`
- `8e75f1b Add Linux packaging and release automation`
- `eb8e08e Track release automation progress`
- `dd69548 Improve first-lane onboarding`
- `d8d10f3 Track onboarding progress`
- `a85e7e0 Explain first provider setup`
- `df2fe74 Track provider onboarding progress`

The branch should be clean and synchronized with GitHub.

Remote:

`https://github.com/CreativeSystemDesign/visualllm.git`

## Completed public-readiness work

The project includes:

- `ROADMAP.md`
- `LICENSE`
- `SECURITY.md`
- `CONTRIBUTING.md`
- `CODE_OF_CONDUCT.md`
- GitHub issue templates
- Pull-request template
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`

CI covers:

- renderer smoke test;
- Rust formatting;
- Rust tests;
- Clippy;
- Linux Tauri packaging.

Tagged releases are configured to build:

- `.deb`
- AppImage
- `SHA256SUMS`

## Startup behavior

The recommended Linux launcher is:

`tools/launch-system.sh`

It starts the compiled binary with a clean environment outside VS Code/Snap.
It detects an existing healthy engine on port `4100` and exits cleanly instead
of opening a broken duplicate window.

Do not stop the currently running VisualLLM process unless the user explicitly
asks. It may still be needed as a fallback connection.

## Current UI work

The renderer has first-run onboarding improvements:

- Empty lane state explains how to create the first endpoint.
- It explains that the rightmost model answers first.
- It provides a `Create a lane` button.
- Empty provider state explains what providers are.
- It explains that testing a provider loads its catalog.
- The advanced provider form remains available.

The renderer smoke test is:

`node tools/smoke.js`

It has been passing.

## Architecture

Important files:

- `src-tauri/src/main.rs` — Tauri shell and UI command boundary.
- `src-tauri/src/providers.rs` — provider persistence, authentication, catalogs, stats.
- `src-tauri/src/lanes.rs` — lane/member/pool persistence and compatibility.
- `src-tauri/src/server.rs` — local OpenAI-compatible engine and fallback routing.
- `src-tauri/src/loopwatch.rs` — tool-loop detection and repair.
- `src-tauri/src/incidents.rs` — evidence-backed failure records.
- `renderer/app.js` — UI state, rendering, drag/drop, provider flow, notifications.
- `renderer/index.html` — UI structure.
- `renderer/style.css` — visual design.
- `tools/preview.js` — browser preview harness using real persisted data without exposing API keys.
- `tools/smoke.js` — renderer load/wiring validation.

## Security boundary

The renderer intentionally has:

- no direct network access;
- no filesystem access;
- no provider-key access.

Network access and secret handling belong in Rust. Do not add renderer-side
`fetch()` calls to upstream providers or the local engine without carefully
preserving this boundary.

Provider keys are currently stored locally in a protected JSON file. The
roadmap identifies OS keychain storage as a future priority.

## Loopwatch

Loopwatch is opt-in per lane.

It detects:

1. **Verbatim loops** — the same tool and same arguments repeated after results were received.
2. **Futile loops** — different arguments with byte-identical results, meaning the model is receiving no new information.

Its treatment:

- collapse only safely identical redundant pairs;
- append a diagnostic note as the final user message;
- preserve evidence;
- announce the repair in `x-visualllm-unstuck`;
- record an incident.

The legacy gateway’s Loopwatch implementation was the original behavioral
reference. VisualLLM’s Rust implementation is in `src-tauri/src/loopwatch.rs`.

## Current roadmap priority

The next remaining UI item is catalog freshness visibility.

After that:

- mock-provider integration tests for the Rust engine;
- blocking and streaming fallback tests;
- capability/context filtering tests;
- persistence migration tests;
- stronger release validation;
- OS keychain storage;
- single-instance desktop behavior.

## Current experiment

The user is testing whether Loopwatch’s repeated todo-list warnings are caused
by the legacy gateway or by the underlying model/client interaction.

When continuing:

1. Treat the direct OpenRouter session as a fresh session.
2. Do not assume the prior gateway’s Loopwatch message is authoritative.
3. Observe whether repeated tool calls or todo updates occur without the gateway.
4. Compare direct OpenRouter behavior, legacy gateway behavior, and VisualLLM Loopwatch behavior.
5. If the issue disappears, document that the gateway’s intervention may be contributing.
6. If it persists, the cause is probably in the model/client/tool interaction rather than the gateway.
7. Avoid unnecessary repeated planning/todo tool calls. Prefer direct progress updates and only use tools when they produce new information.

## Useful validation commands

From `/home/shane/visualllm`:

- `node tools/smoke.js`
- `git diff --check`
- `git status --short`
- `curl http://127.0.0.1:4100/health`

From `/home/shane/llm_gateway`:

- `.venv/bin/python -m pytest tests/ -q`

The user wants work to continue autonomously where safe, but UI/runtime testing
should wait until explicitly requested or until they switch back to a connection
that will not risk the active development session.
