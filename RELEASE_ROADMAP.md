# VisualLLM — Major Release Roadmap

**Target version:** 1.0.0  
**Status:** Planning  
**Date:** 2026-08-05

This document outlines the work required for the first major release. It covers the VS Code model picker improvements and editor integrations for Anti-Gravity, Cursor, Windsurf, and Zed.

---

## Why a major release?

The current VS Code integration is a first pass — it works, but it has hardcoded capabilities, no remove path, and only targets VS Code and Insiders. This release makes the integration accurate, complete, and extensible to all major editors that use the `chatLanguageModels.json` format.

---

## 1. Fix the VS Code model picker integration

### 1.1 Derive capabilities from the lane's members

**Problem:** The integration hardcodes `vision: false`, `tool_calling: true`, `max_input_tokens: 250144`, and `max_output_tokens: 8000` for every lane, regardless of what models are actually in it. VS Code uses these hints to filter and display models in the picker.

**Fix:** Compute capabilities from the lane's actual members and the cached catalog at integration time.

- `vision` — true if any member in the lane has vision capability
- `tool_calling` — true if any member supports tools
- `max_input_tokens` — the maximum context window among all members
- `max_output_tokens` — the maximum output tokens among all members (or a sensible default like 8192)

**Files to change:** `src-tauri/src/main.rs` — `vscode_integrate_lane` function

### 1.2 Remove the `api_key` field or set it to empty

**Problem:** The `apiKey` field is set to `"placeholder"`, which is misleading. The local engine doesn't need an API key, but VS Code may try to use this value and prompt the user for one.

**Fix:** Omit the `api_key` field entirely from the `VscodeProviderEntry` when the vendor is `customendpoint` and the URL is a loopback address. Alternatively, set it to an empty string.

**Files to change:** `src-tauri/src/main.rs` — `vscode_merge_lane` function

### 1.3 Add a `vscode_remove_lane` command

**Problem:** When a user deletes a lane from VisualLLM, the corresponding entry in `chatLanguageModels.json` is never cleaned up. When a lane is renamed, the old entry stays too (the merge updates by slug, but a deleted lane's slug is never removed).

**Fix:** Add a `vscode_remove_lane(slug)` Tauri command that removes the model entry with the matching slug from all editor config files. The UI should call this when a lane is deleted or when the user explicitly removes the VS Code integration.

**Files to change:** `src-tauri/src/main.rs` — new command + `vscode_chat_models_paths` helper; `renderer/app.js` — call remove on lane delete

### 1.4 Add a VS Code integration status indicator

**Problem:** The user has no way to know if a lane is currently integrated in VS Code without checking the file manually.

**Fix:** Add a read-only check that inspects `chatLanguageModels.json` for each editor and reports whether the lane's slug is present. Show a small indicator (e.g., a checkmark or "VS Code" badge) on each lane header.

**Files to change:** `src-tauri/src/main.rs` — new `vscode_lane_status` command; `renderer/app.js` — status indicator in lane header

### 1.5 Handle port changes

**Problem:** If the user changes the engine port, the VS Code entries still point to the old port. There's no automatic update mechanism.

**Fix:** Either:
- Update the entries automatically when the port changes (read all editor configs, update the URL for each VisualLLM model entry)
- Or document clearly that port changes require re-integration, and add a "Re-integrate" button in the UI

**Files to change:** `src-tauri/src/main.rs` — update logic in `port_set` or a new `vscode_refresh_lane` command; `renderer/app.js` — re-integrate button

---

## 2. Editor integrations (empirically verified on Linux 2026-08-06)

Each editor was downloaded, launched, and its binary + first-run config layout inspected. Only `chatLanguageModels.json` consumers belong in the editor list/menu; everything else is documented here for future writers.

**Config root per OS** (user-confirmed 2026-08-06): Windows `%APPDATA%\<Product>`, macOS `$HOME/Library/Application Support/<Product>`, Linux `$HOME/.config/<Product>`. The engine resolves this natively in Rust (`config_root()`) — no external script needed.

### 2.1 VS Code + VS Code Insiders (confirmed consumer)

Consumes `chatLanguageModels.json` per code.visualstudio.com/docs/agent-customization/language-models; 7 refs in the installed binary's `out/`. Two-level schema:

- Provider: `name: "visualllm"`, `vendor: "customendpoint"`, `apiType: "chat-completions"`, `models: []`. `apiKey` must be a `${input:...}` placeholder or omitted — a raw key is silently dropped, so omitting it is correct for the local gateway.
- Model: `id` (slug), `name`, `url` (full chat-completions path; VS Code POSTs verbatim), `toolCalling`, `vision`, `maxInputTokens`, `maxOutputTokens`, plus optional `thinking`/`streaming`/`editTools`.
- Targets: `Code` and `Code - Insiders` under the per-OS config root, each `<Product>/User/chatLanguageModels.json`.

**Status:** implemented — capability derivation, no-op-proof merge by slug, per-editor results, remove path.

### 2.2 Cursor (confirmed — NOT a consumer)

Cursor 3.14 has **zero** `chatLanguageModels`/`customendpoint` refs in its app tree — it does not read the file. Its "Override OpenAI Base URL" + custom models are stored in a **SQLite state DB** (`User/globalStorage/state.vscdb`, `ItemTable`, keys `openAIBaseUrl`/`useOpenAIKey`), not `settings.json` — the earlier settings.json claim is outdated. Config dir confirmed at `$HOME/.config/Cursor/User/` (empirically created on launch).

**Status:** not integrable via a config file; would require reverse-engineering the state DB + opaque keys. Not offered in the editor menu.

### 2.3 Windsurf → Devin Desktop (confirmed consumer)

Windsurf is rebranded **Devin Desktop** (apt package `devin-desktop`). The 3.6.27 build ships the full VS Code language-model code (`chatLanguageModels` schema registered, vendors `customendpoint` + `customoai` handled, `languageModelsResource` resolution) and created `chatLanguageModels.json` on first run. Config dir empirically confirmed at `$HOME/.config/Devin/User/` (pre-rebrand Windsurf builds used `$HOME/.config/Windsurf/User/`). Same `customendpoint` schema as VS Code, so the existing writer applies verbatim.

**Status:** implemented — "Windsurf" entry writes to the current `Devin` dir. Legacy `Windsurf` dir not auto-detected.

### 2.4 Anti-Gravity IDE (confirmed — NOT a consumer)

Antigravity IDE 2.1.1 is a VS Code fork but with the `chatLanguageModels` feature stripped: **zero** refs in the entire app tree, no `language-models` schema, no `customendpoint` vendor. It uses Google's own Gemini model picker. Config dir empirically confirmed at `$HOME/.config/Antigravity IDE/User/` (space in the name; matches the claimed layout), but no file is read there.

**Status:** not integrable in the current build; the config dir name is confirmed so a future build with the feature can be added by changing one entry.

### 2.5 Zed (confirmed — NOT a consumer, top-level settings.json)

Zed does not use `chatLanguageModels.json`. Global model picker config (the "assistant" block) lives in the top-level `settings.json` (Windows `%APPDATA%\Zed\settings.json`, macOS/Linux `$HOME/.config/zed/settings.json`). A writer would merge an OpenAI-compatible provider into the `language_models`/assistant block.

**Status:** paths confirmed; block schema needs confirmation. Note: the macOS path (`$HOME/.config/zed/settings.json`) differs from the commonly cited `~/Library/Application Support/Zed/settings.json` — verify on a Mac before relying on it.

### 2.6 Architecture conclusion

`editor_chat_models_paths()` is a list of confirmed `chatLanguageModels.json` targets (VS Code, Insiders, Windsurf/Devin), resolved against the per-OS config root. Cursor, Anti-Gravity (current build), and Zed are different mechanisms and are NOT offered in the menu until a confirmed writer exists.

---

## 3. UI changes

### 3.1 Rename the VS Code button to "Editors"

The button currently says "VS Code" but it now targets multiple editors. The label should reflect this.

**Files to change:** `renderer/app.js` — button title and toast messages; `renderer/egl.css` — any button-specific styles

### 3.2 Show integration status per lane

Add a small indicator on each lane header showing which editors the lane is integrated with (e.g., a VS Code icon, a Cursor icon, etc.). Clicking the indicator could open a sub-menu with "Integrate" and "Remove" options per editor.

**Files to change:** `renderer/app.js` — lane header rendering; `renderer/egl.css` — indicator styles

### 3.3 Add a "Re-integrate" action

When the user changes the engine port or wants to update the integration after editing a lane, they should be able to re-integrate without deleting and re-adding.

**Files to change:** `renderer/app.js` — re-integrate handler; `src-tauri/src/main.rs` — refresh command

---

## 4. Documentation

### 4.1 Update README

Add a section documenting the editor integrations:
- Which editors are supported
- How the integration works (writes to `chatLanguageModels.json`)
- How to use the VS Code / Editors button
- How to remove an integration
- Troubleshooting (editor not detected, file not writable, etc.)

**Files to change:** `README.md`

### 4.2 Update this roadmap document

As items are completed, check them off and add notes about any changes to the approach.

**Files to change:** `RELEASE_ROADMAP.md` (this file)

### 4.3 Add integration guide to docs/

Create a `docs/editor-integration.md` file that explains:
- What `chatLanguageModels.json` is
- Which editors use it
- How VisualLLM integrates with each editor
- How to manually integrate if the automatic integration fails
- How to remove an integration

**Files to change:** `docs/editor-integration.md` (new file)

---

## 5. Testing

### 5.1 Unit tests

- Test that `editor_chat_models_paths()` returns the correct paths for all supported editors
- Test that `editor_merge_lane()` correctly updates by slug (existing test covers this for VS Code)
- Test that `editor_remove_lane()` correctly removes a lane entry
- Test that capabilities are derived correctly from lane members

**Files to change:** `src-tauri/src/main.rs` — add tests in `editor_tests` module

### 5.2 Integration tests

- Test the full integration flow: add a lane → click "Editors" → verify the file is written correctly for each editor
- Test the remove flow: delete a lane → verify the entry is removed from all editor configs
- Test the status indicator: integrate a lane → verify the status shows as integrated

**Files to change:** `tools/smoke.js` — add editor integration tests

### 5.3 Manual testing checklist

- [ ] VS Code stable: integrate a lane, verify it appears in the model picker, reload, verify it's there
- [ ] VS Code Insiders: same as above
- [ ] Remove integration: delete a lane, verify the entry is removed from all editor configs
- [ ] Port change: change the engine port, verify the VS Code entries are updated or re-integration works
- [ ] Lane rename: rename a lane, verify the VS Code entry is updated
- [ ] Multiple editors: integrate with VS Code and Windsurf/Devin simultaneously, verify both have the entry
- [ ] Confirmed non-consumers (Cursor, Anti-Gravity, Zed) are absent from the editor menu

---

## 6. Release checklist

- [ ] All unit tests pass (`cargo test`)
- [ ] All integration tests pass (`node tools/smoke.js`)
- [ ] Manual testing checklist is complete
- [ ] README is updated with editor integration documentation
- [ ] `docs/editor-integration.md` is created
- [ ] CHANGELOG.md is updated (create if it doesn't exist)
- [ ] Version bumped to 1.0.0 in `package.json` and `Cargo.toml`
- [ ] AppImage is built and verified on a clean VM
- [ ] `.deb` package is built and verified
- [ ] Release notes are written

---

## 7. macOS distribution status (deferred)

The `macos` / `macos-sign` release jobs currently code-sign only the DMG and
do **not** notarize the `.app` bundle. Tauri's auto-updater on macOS requires a
signed **and** notarized app, so macOS "auto-update" would be broken if
shipped as-is.

**Decision (2026-08-06):** macOS distribution is deferred until there is a real
macOS user base. It is not worth an Apple Developer account (and the
notarization pipeline) before then. The macOS jobs remain in `release.yml`
for manual/on-demand runs, but macOS is **not** part of the supported update
channel. Linux (deb + AppImage) and Windows (NSIS) are the supported matrix.

To re-enable properly later: sign the `.app` bundle (not just the DMG),
notarize it via `xcrun notarytool`, staple, and confirm the updater on a clean
macOS VM. Track in `docs/IMPLEMENTATION_PLAN.md` §1.5.

---

## 8. Timeline estimate
|------|--------|----------|
| 1.1 Derive capabilities from lane members | 2 days | High |
| 1.2 Remove/fix `api_key` field | 0.5 days | High |
| 1.3 Add `vscode_remove_lane` command | 1 day | High |
| 1.4 Add VS Code integration status indicator | 1 day | Medium |
| 1.5 Handle port changes | 1 day | Medium |
| 2.1 VS Code + Insiders integration (implemented) | — | — |
| 2.2 Cursor (not integrable: SQLite state DB) | — | — |
| 2.3 Windsurf/Devin integration (implemented) | — | — |
| 2.4 Anti-Gravity (blocked: feature absent in 2.1.1) | — | — |
| 2.5 Zed settings.json writer (if a user runs Zed) | 1 day | Low |
| 2.6 Trim editor list to confirmed consumers (done) | — | — |
| 3.1 Rename VS Code button to "Editors" | 0.5 days | Medium |
| 3.2 Show integration status per lane | 1 day | Medium |
| 3.3 Add "Re-integrate" action | 0.5 days | Medium |
| 4.1 Update README | 0.5 days | High |
| 4.2 Update this roadmap | 0.25 days | Low |
| 4.3 Add `docs/editor-integration.md` | 1 day | High |
| 5.1 Unit tests | 1 day | High |
| 5.2 Integration tests | 1 day | High |
| 5.3 Manual testing checklist | 1 day | High |
| **Total** | **~16 days** | |

---

## 9. Post-1.0 feature candidates (Phase 3, ranked)

Carried over from `docs/IMPLEMENTATION_PLAN.md` Phase 3. Item 1 shipped
2026-08-06; the rest are not committed work. Re-confirm priority with the user
before starting.

1. **Per-lane failure budgets / auto-park** — **done (2026-08-06).** A lane
   that keeps failing the same transient way (provider errors, dead
   connections, silence, rate limits) parks itself after a rolling-window
   budget (default 5 in 10 min), answers 503 (`lane_parked`) until a human
   unparks it, and records an `auto_parked` incident with the receipt. Header
   button + notification action both unpark and reset the budget. Budget is
   configurable per lane in `lanes.json`.
2. **Request replay in the notification center** — retry a failed request from
   the incident record (confirmation-gated; replay spends money).
3. **Lane cloning** — duplicate a lane with members/params/criteria; no
   auto-write of the editor integration.
4. **Usage/credit line** — rolling 24h / 7d per-lane request/failure counters
   in the UI (read-only; thresholds are budget work).

---

## 10. Open questions

1. ~~**Zed's configuration format**~~ — Resolved 2026-08-06: not a `chatLanguageModels.json` consumer; top-level `settings.json` "assistant" block (see §2.5).
2. ~~**Anti-Gravity's config path**~~ — Resolved 2026-08-06 (empirical): config dir is `<root>/Antigravity IDE/User/`, but the 2.1.1 build has the `chatLanguageModels.json` feature stripped (see §2.4).
3. ~~**Windsurf's config path**~~ — Resolved 2026-08-06 (empirical): rebranded to Devin Desktop; config dir is `<root>/Devin/User/` and it IS a `chatLanguageModels.json` consumer (see §2.3).
4. ~~**Cursor's config path**~~ — Resolved 2026-08-06 (empirical): not a consumer; BYOK state lives in `User/globalStorage/state.vscdb` (SQLite) (see §2.2).
5. **Should the integration be opt-in per editor?** — Some users may not want all editors to be targeted. Consider a settings option to choose which editors to integrate with.
6. **Should the integration be automatic or manual?** — Currently the user must click the "Editors" button on each lane. Should integration happen automatically when a lane is created?
7. **Zed: implement the settings.json writer?** — Feasible via `language_models.openai_compatible` merge; only worth it if a user actually runs Zed.
