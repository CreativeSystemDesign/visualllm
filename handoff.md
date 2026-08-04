# VisualLLM Handoff

## Session context

You are continuing work on the **VisualLLM** project at:

`/home/shane/visualllm`

The user is switching from the legacy gateway to direct OpenRouter access to
test whether the gateway’s Loopwatch messages are necessary. Do not assume
prior conversational context; this file contains the current state.

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
