### VisualLLM Improvement Plan

This document outlines the recommended actions for improving the VisualLLM codebase, focusing on maintainability, stability, and security as identified in the deep dive analysis.

---

### 1. Architectural & Maintainability Refactoring
**Goal**: Reduce the complexity of monolithic files by decomposing them into logical modules.

*   **Frontend Refactor (`renderer/app.js`)**:
    *   Split `app.js` (currently >3,500 lines) into ES modules.
    *   Target modules: `api.js` (communication with Tauri/Rust), `state.js` (application state management), `dom.js` (UI construction), `events.js` (event listeners), `drag-drop.js` (lane/card movement).
*   **Backend Refactor (`src-tauri/src/server.rs`)**:
    *   Decompose `server.rs` (currently >2,600 lines) into a module hierarchy under `src-tauri/src/engine/`.
    *   Target modules: `gateway.rs` (Axum server setup), `routes.rs` (API endpoints), `ledger.rs` (usage/activity logging), `classifier.rs` (request classification/fallback logic).

### 2. Stability & Technical Debt Resolution
**Goal**: Resolve critical runtime issues and optimize performance bottlenecks.

*   **Shutdown Crash Investigation**:
    *   Debug the heap corruption (`free(): corrupted unsorted chunks`) occurring on process exit.
    *   Ensure graceful shutdown of the background Axum server before Tauri/GTK teardown.
    *   Review GTK/Cairo callback lifecycles in `main.rs`.
*   **Activity Log Optimization**:
    *   Refactor `note_activity` in `server.rs` to avoid full file rewrites.
    *   Implement log rotation or a fixed-size ring buffer for the 64KiB activity log.

### 3. Security Enhancements
**Goal**: Protect the local gateway from unauthorized local access.

*   **Loopback Security**:
    *   Implement a session token mechanism between the Tauri frontend and the Rust backend.
    *   (Linux) Investigate `SO_PEERCRED` to verify that only the current user's processes can reach the gateway.

### 4. Release & Versioning Consistency
**Goal**: Ensure documentation and metadata stay in sync with code releases.

*   **Version Synchronization**:
    *   Automate version checks across `package.json`, `Cargo.toml`, and documentation.
    *   Update `DISTRIBUTION_SETUP.md` to reflect the actual release state.
*   **Packaging Verification**:
    *   Formalize the verification process for `.deb` and AppImage artifacts on clean installations.
