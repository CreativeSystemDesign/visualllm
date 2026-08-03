# VisualLLM Roadmap

VisualLLM is a visual fallback router for AI models.

It lets people add providers, browse the models they offer, arrange models into
ordered lanes, and expose each lane as a local OpenAI-compatible endpoint. The
model on the right answers first; models to its left are fallbacks.

This roadmap describes the work required to turn the current working prototype
into a public project that is safe to install, easy to understand, and worthy
of trust. It is intentionally outcome-oriented: implementation details may
change as real users try the product.

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

## Current state

The core product is already functional:

- Tauri desktop shell with a framework-free renderer.
- Provider configuration and catalog fetching.
- OpenRouter and generic OpenAI-compatible providers.
- Visual pool and drag-and-drop lane construction.
- Ordered fallback routing on `127.0.0.1:4100`.
- Capability and context filtering.
- Blocking and streaming response handling.
- A pre-forward commit gate for empty, stalled, or unusable 200 responses.
- Per-member request dials and lane-level reasoning suppression.
- Opt-in Loopwatch for stuck tool-call conversations.
- Evidence-backed incident records and renderer notifications.
- Browser preview harness and renderer smoke test.
- Reliable system-terminal launcher for Linux development.

The legacy Python gateway remains the current development safety connection for
this conversation through its isolated `workbench/luna` endpoint. It is a
reference implementation and operational fallback, not a dependency of
VisualLLM.

## Milestones

### 1. Desktop correctness — release blocker

- [ ] Implement single-instance behavior.
- [ ] Make a second launch focus the existing window or exit cleanly.
- [ ] Detect an existing healthy engine without opening a broken duplicate.
- [ ] Handle stale coordination state after a crash.
- [ ] Test clean startup, duplicate startup, restart, and shutdown.

**Done when:** launching VisualLLM twice never leaves a second window showing a
port or engine error.

### 2. Public repository foundation

- [x] Add a project roadmap.
- [x] Add a license, security policy, contribution guide, and code of conduct.
- [x] Add issue and pull-request templates.
- [ ] Add screenshots or a short product demo.
- [ ] Define the first public release version and support policy.
- [ ] Separate beginner documentation from implementation history.

**Done when:** a new visitor can understand the project, its license, its
security model, and how to participate in under five minutes.

### 3. Install and release normally

- [x] Add GitHub Actions CI for Rust, renderer smoke tests, and packaging.
- [x] Add a tagged Linux release workflow for `.deb` and AppImage artifacts.
- [x] Generate checksums for release artifacts.
- [ ] Publish tested `.deb` and AppImage artifacts.
- [ ] Verify desktop-menu launch on a clean Linux installation.
- [ ] Document supported distributions and runtime requirements.
- [ ] Decide whether Linux is the initial supported platform or whether
  macOS/Windows will be release targets too.

**Done when:** a non-developer can download, install, launch, and remove the
application without opening a terminal.

### 4. First-run onboarding

- [x] Explain providers, models, pools, and lanes in the empty states.
- [x] Guide the first provider setup without hiding the advanced form.
- [x] Explain right-to-left fallback priority at the point of use.
- [x] Show a clear next step after the first lane is created.
- [x] Make catalog freshness and refresh state visible.

**Done when:** a first-time user can create a working lane without reading the
architecture documentation.

### 5. Make connection the product moment

- [ ] Give every lane a prominent endpoint card.
- [ ] Add copy endpoint and copy setup instructions actions.
- [ ] Add a test-lane action with a useful result or diagnosis.
- [ ] Show recent serving model and fallback activity.
- [ ] Provide OpenAI-compatible and VS Code setup examples.
- [ ] Explain whether an API key is needed by the local client.

**Done when:** a user can create a lane and connect a client without guessing
which URL, model name, or settings to use.

### 6. Routing and persistence confidence

- [ ] Add mock-provider integration tests for blocking and streaming requests.
- [ ] Test capability skips, context overflow, provider failures, and fallback
  trails end to end.
- [ ] Test stream errors and empty/reasoning-only 200 responses.
- [ ] Add persistence schema versions and migration tests.
- [ ] Define behavior for corrupt or partially written state files.
- [ ] Add export/import or backup/restore for user configuration.

**Done when:** routing behavior is protected by tests that do not spend provider
quota or depend on live upstream services.

### 7. Security and lifecycle maturity

- [ ] Store provider keys in the OS keychain.
- [ ] Keep secrets out of logs, previews, diagnostics, and crash reports.
- [ ] Improve port-conflict and engine-ownership messages.
- [ ] Support a deliberate configurable port without compromising stable URLs.
- [ ] Document the localhost-only threat model and limitations.
- [ ] Review all Tauri permissions and webview boundaries before release.

**Done when:** the security model is explicit, tested, and appropriate for a
public local desktop application.

### 8. Public launch

- [ ] Record a short add-provider → drag-models → connect-client demo.
- [ ] Publish a launch-quality README with screenshots.
- [ ] Create the first tagged release and GitHub release notes.
- [ ] Add issue labels and a small triage process.
- [ ] Invite users to test provider setup, lane creation, and fallback behavior.
- [ ] Use real feedback to prioritize the next release.

**Done when:** the project is easy to discover, easy to try, and has a clear
path for users to report what prevented success.

## Non-goals for the first public release

- Running or downloading model weights locally.
- Becoming a general-purpose model serving platform.
- Exposing the gateway to a LAN or the public internet by default.
- Automatically spending money or selecting paid models without an explicit
  user decision.
- Replacing every provider-specific feature with a universal abstraction.

## Immediate next implementation

The next code milestone is **single-instance desktop behavior**. It directly
addresses the first real startup failure observed during development and is a
release blocker because a duplicate launch currently creates a window whose
engine cannot bind the fixed port.

After that, the next public-facing milestone is the README and first-run
connection experience, followed by CI and release packaging.

## Release criteria

A first public release should meet all of these conditions:

- The app installs and launches from a packaged artifact on a clean supported
  Linux system.
- A second launch behaves cleanly.
- A user can create a lane and copy a working endpoint.
- Provider keys never appear in the renderer or logs.
- Routing and persistence tests pass without live provider access.
- The README explains the product, installation, first lane, client connection,
  limitations, and security model.
- A failed upstream response produces an understandable diagnosis rather than
  a silent or empty client result.
