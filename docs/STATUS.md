# Current Project Status

VisualLLM is a pre-1.0 Tauri desktop application with a framework-free
renderer and a Rust backend that owns the local OpenAI-compatible gateway,
provider credentials, persistence, and routing decisions.

## Current documentation

- [`README.md`](../README.md) — product behavior, setup, architecture, and user-facing security guidance.
- [`ROADMAP.md`](../ROADMAP.md) — active product roadmap and release criteria.
- [`INSTALLATION_PLAN-v0.6.0.md`](INSTALLATION_PLAN-v0.6.0.md) — active
  installation/distribution workstream, acceptance criteria, evidence log, and
  current-session handoff.
- [`HANDOFF-v0.6.0.md`](HANDOFF-v0.6.0.md) — exact current state and starting
  instructions for the next development session.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — development workflow and validation commands.
- [`SECURITY.md`](../SECURITY.md) — threat model and vulnerability reporting.
- [`CHANGELOG.md`](../CHANGELOG.md) — release history.

Historical planning and session handoff documents are kept in
[`archive/`](archive/) for context. They may describe superseded designs and
should not be treated as the current implementation contract.

## Active release focus

The working target is **v0.6.0**, focused on making installation honest,
discoverable, and verifiable. Linux x86_64 is the only v0.6.0 distribution;
Windows and Apple Silicon macOS are deferred until a future release with
appropriate signing and native verification. Continue from the **Current
handoff** section of the installation plan and update it at the end of every
session.

## Architecture at a glance

- `renderer/` contains the HTML, CSS, and JavaScript UI. It communicates through the narrow `window.vll` Tauri bridge and has no general filesystem or network access.
- `src-tauri/src/main.rs` contains the Tauri shell and command boundary.
- `src-tauri/src/server.rs` contains the loopback Axum gateway and fallback engine.
- `src-tauri/src/providers.rs` owns provider catalogs, keychain access, and catalog/statistics caches.
- `src-tauri/src/lanes.rs` owns lane/member schemas and persistent engine bookkeeping.
- `src-tauri/src/incidents.rs` owns evidence-backed incident records and replay snapshots.
- `src-tauri/src/loopwatch.rs` detects and repairs configured tool-call loops.

## Validation baseline

The repository's expected checks are:

```bash
node tools/smoke.js
npm test
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
node tools/check-version.js
```
