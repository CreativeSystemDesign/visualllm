### VisualLLM Refactoring Deep Dive

This document provides a detailed analysis and implementation plan for refactoring the two largest files in the VisualLLM codebase: `renderer/app.js` and `src-tauri/src/server.rs`.

---

### 1. Frontend Refactor: `renderer/app.js` (3,574 lines)

The current frontend architecture is monolithic, making it difficult to test individual components or manage the complex drag-and-drop and state logic.

#### Proposed Directory Structure:
```
renderer/js/
├── api.js           # Tauri bridge and API wrappers
├── state.js         # Global state object and core mutations
├── models.js        # Model identity helpers and sorting logic
├── icons.js         # SVG icon constants
├── dom/
│   ├── track.js     # Track/Hall rendering (renderTrack)
│   ├── chip.js      # Model chips and member dials
│   └── lane.js      # Lane elements, headers, and footers
├── ui/
│   ├── drag-drop.js # Lane and member drag-and-drop engine
│   ├── interaction.js # Modals, toasts, search, and global events
│   ├── notifications.js # Incident toasts and sidebar feed
│   └── panels/
│       ├── providers.js # Model browser and provider settings
│       ├── budget.js    # Lane auto-park budget popover
│       └── ide.js       # IDE integration menu
├── persistence.js   # Local lane and pool persistence
└── main.js          # Entry point, refresh loop, and initialization
```

#### Key Extraction Points:
*   **Bridge & State**: Lines 11-106 are pure candidates for `api.js` and `state.js`.
*   **Drag & Drop**: Lines 728-980 contain a self-contained hand-written D&D engine.
*   **Provider Panel**: Lines 2223-3215 is the single largest UI component (>900 lines) and should be moved to `ui/panels/providers.js`.

---

### 2. Backend Refactor: `src-tauri/src/server.rs` (2,687 lines)

The backend engine mixes Axum server configuration with the complex logic of fallback chains and request inspection.

#### Proposed Module Hierarchy:
```
src-tauri/src/engine/
├── mod.rs           # Public Engine struct and module exports
├── activity.rs      # note_activity and activity_read
├── ledger.rs        # record_usage (usage tracking)
├── inspector.rs     # Request Needs inspection and can_serve logic
├── classifier.rs    # Verdict enum and model_limitation/classify logic
├── gate.rs          # usability checks (usable_event, usable_body) and SseScan
├── proxy.rs         # Core chat/chat_inner fallback loop
├── handlers.rs      # Axum route handlers for health, activity, etc.
└── mod.rs           # (Replaces server.rs) Axum router and serve()
```

#### Refinement with Existing Modules:
*   `src-tauri/src/incidents.rs`: Should remain the source of truth for incident persistence.
*   `src-tauri/src/lanes.rs` and `src-tauri/src/providers.rs`: Should remain as domain/persistence modules.
*   The engine logic in `server.rs` that interacts with these modules should be moved to the new `engine/` sub-modules.

---

### 3. Stability Note: Shutdown Crash

The `free(): corrupted unsorted chunks` crash on Linux is likely caused by the `shape_to_allocation` GTK callback in `main.rs` (lines 1213-1226).

**Recommendation**:
1.  Move the GTK callback logic to a safer scope.
2.  Use a proper `AppHandle` and join the server task gracefully during the `tauri::RunEvent::ExitRequested` event.
3.  Ensure the Axum server task handles a shutdown signal (via `watch::Receiver`).

---

### Next Steps:
1.  Start by extracting `api.js` and `state.js` from `app.js` to establish the new module pattern.
2.  Move `server.rs` into a new `engine/` directory and begin splitting it by extracting `classifier.rs` and `gate.rs`.
3.  Implement a graceful shutdown for the Axum server in `main.rs`.
