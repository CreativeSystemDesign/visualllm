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

## 2. Add editor integrations for Anti-Gravity, Cursor, Windsurf, and Zed

### 2.1 Anti-Gravity

Anti-Gravity is an AI coding agent that uses VS Code's `chatLanguageModels.json` format for its model picker. It reads the same file as VS Code.

**Integration approach:** The lane should already be written to the VS Code stable and Insiders paths. Anti-Gravity reads from the same location, so no additional file writes are needed — the existing integration covers it.

**Verification needed:** Confirm that Anti-Gravity reads `~/.config/Code/User/chatLanguageModels.json` and that the `customendpoint` vendor format is supported.

**Files to change:** None (covered by existing integration), but add Anti-Gravity to the documentation.

### 2.2 Cursor

Cursor reads `chatLanguageModels.json` from its own config directory, not VS Code's. The file path is:

- Linux: `~/.config/Cursor/User/chatLanguageModels.json`
- macOS: `~/Library/Application Support/Cursor/User/chatLanguageModels.json`
- Windows: `%APPDATA%/Cursor/User/chatLanguageModels.json`

**Integration approach:** Add Cursor's path to `vscode_chat_models_paths()` (rename to `editor_chat_models_paths()` to reflect the broader scope). The merge logic is identical — Cursor uses the same JSON format.

**Files to change:** `src-tauri/src/main.rs` — `vscode_chat_models_paths()` → `editor_chat_models_paths()` with Cursor paths added

### 2.3 Windsurf

Windsurf (by Codeium) also uses the `chatLanguageModels.json` format. Its config path follows the same pattern as Cursor:

- Linux: `~/.config/Windsurf/User/chatLanguageModels.json`
- macOS: `~/Library/Application Support/Windsurf/User/chatLanguageModels.json`
- Windows: `%APPDATA%/Windsurf/User/chatLanguageModels.json`

**Integration approach:** Same as Cursor — add Windsurf's path to the editor config paths list.

**Files to change:** `src-tauri/src/main.rs` — add Windsurf paths to `editor_chat_models_paths()`

### 2.4 Zed

Zed is a high-performance editor with AI features. It uses a different configuration format and location:

- Linux: `~/.config/zed/extensions/` or `~/.config/zed/settings.json`
- macOS: `~/Library/Application Support/Zed/extensions/` or `~/Library/Application Support/Zed/settings.json`

**Integration approach:** Zed does **not** use `chatLanguageModels.json`. It has its own AI provider configuration system. The integration approach needs to be different:

1. **Option A (recommended):** Zed supports custom OpenAI-compatible endpoints through its AI provider settings. The integration should write a Zed-specific configuration file or provide instructions for the user to add the endpoint manually.
2. **Option B:** Zed's extension API may allow programmatic configuration in the future. Monitor the Zed extension API for AI provider support.

For now, the integration should:
- Detect if Zed is installed
- Write a Zed-compatible configuration (if a standard format exists)
- Or provide a copy-to-clipboard setup string that the user can paste into Zed's settings

**Files to change:** `src-tauri/src/main.rs` — new `zed_chat_models_path()` function + integration logic; `renderer/app.js` — Zed button in lane header

### 2.5 Unified editor integration architecture

The current code has `vscode_chat_models_paths()` which is VS Code-specific. This should be generalized to support any editor that uses the `chatLanguageModels.json` format, plus editors with different config formats.

**Proposed structure:**

```rust
/// All supported editors and their chatLanguageModels.json paths.
fn editor_chat_models_paths() -> Result<Vec<(PathBuf, &'static str)>, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let config = PathBuf::from(home).join(".config");

    let mut paths = Vec::new();

    // VS Code stable
    paths.push((
        config.join("Code").join("User").join("chatLanguageModels.json"),
        "VS Code",
    ));

    // VS Code Insiders
    paths.push((
        config.join("Code - Insiders").join("User").join("chatLanguageModels.json"),
        "VS Code Insiders",
    ));

    // Cursor
    paths.push((
        config.join("Cursor").join("User").join("chatLanguageModels.json"),
        "Cursor",
    ));

    // Windsurf
    paths.push((
        config.join("Windsurf").join("User").join("chatLanguageModels.json"),
        "Windsurf",
    ));

    // Anti-Gravity (reads VS Code's config)
    // Already covered by VS Code paths above

    // Zed — different format, handled separately
    // See zed_integration_paths() below

    Ok(paths)
}

/// Zed's AI provider configuration path (different format).
fn zed_config_path() -> Result<Option<PathBuf>, String> {
    // ... detect Zed installation and return its config path
}
```

**Files to change:** `src-tauri/src/main.rs` — refactor `vscode_chat_models_paths()` into `editor_chat_models_paths()` + `zed_config_path()`; rename `vscode_integrate_lane` to `editor_integrate_lane`

---

## 3. UI changes

### 3.1 Rename the VS Code button to "Editors"

The button currently says "VS Code" but it now targets multiple editors. The label should reflect this.

**Files to change:** `renderer/app.js` — button title and toast messages; `renderer/style.css` — any button-specific styles

### 3.2 Show integration status per lane

Add a small indicator on each lane header showing which editors the lane is integrated with (e.g., a VS Code icon, a Cursor icon, etc.). Clicking the indicator could open a sub-menu with "Integrate" and "Remove" options per editor.

**Files to change:** `renderer/app.js` — lane header rendering; `renderer/style.css` — indicator styles

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
- [ ] Cursor: install Cursor, integrate a lane, verify it appears in the model picker
- [ ] Windsurf: install Windsurf, integrate a lane, verify it appears in the model picker
- [ ] Anti-Gravity: install Anti-Gravity, verify the lane appears (covered by VS Code paths)
- [ ] Zed: install Zed, integrate a lane, verify the endpoint is available
- [ ] Remove integration: delete a lane, verify the entry is removed from all editor configs
- [ ] Port change: change the engine port, verify the VS Code entries are updated or re-integration works
- [ ] Lane rename: rename a lane, verify the VS Code entry is updated
- [ ] Multiple editors: integrate with VS Code and Cursor simultaneously, verify both have the entry

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

## 7. Timeline estimate

| Task | Effort | Priority |
|------|--------|----------|
| 1.1 Derive capabilities from lane members | 2 days | High |
| 1.2 Remove/fix `api_key` field | 0.5 days | High |
| 1.3 Add `vscode_remove_lane` command | 1 day | High |
| 1.4 Add VS Code integration status indicator | 1 day | Medium |
| 1.5 Handle port changes | 1 day | Medium |
| 2.1 Anti-Gravity integration | 0.5 days | Low (covered by VS Code) |
| 2.2 Cursor integration | 1 day | High |
| 2.3 Windsurf integration | 1 day | High |
| 2.4 Zed integration | 2 days | High |
| 2.5 Unified editor integration architecture | 1 day | High |
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

## 8. Open questions

1. **Zed's configuration format** — Does Zed have a standard `chatLanguageModels.json` equivalent, or does it use a different format? Needs investigation.
2. **Anti-Gravity's config path** — Does Anti-Gravity use VS Code's config path or its own? Needs verification.
3. **Windsurf's config path** — Confirmed to use `~/.config/Windsurf/User/chatLanguageModels.json` on Linux, but macOS and Windows paths need verification.
4. **Cursor's config path** — Confirmed to use `~/.config/Cursor/User/chatLanguageModels.json` on Linux, but macOS and Windows paths need verification.
5. **Should the integration be opt-in per editor?** — Some users may not want all editors to be targeted. Consider a settings option to choose which editors to integrate with.
6. **Should the integration be automatic or manual?** — Currently the user must click the "VS Code" button on each lane. Should integration happen automatically when a lane is created?
