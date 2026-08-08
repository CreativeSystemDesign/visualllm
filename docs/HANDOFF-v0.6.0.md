# VisualLLM v0.6.0 Session Handoff

**Updated:** 2026-08-08
**Status:** Workstreams 1/2/3/6 implementation complete; hosted verification remains
**Next workstream:** Linux release rehearsal and installation verification

## Start here

1. Read [`INSTALLATION_PLAN-v0.6.0.md`](INSTALLATION_PLAN-v0.6.0.md). It is the
   authoritative ordered checklist, definition of done, evidence log, and
   release policy.
2. Read [`STATUS.md`](STATUS.md) for the architecture and normal validation
   commands.
3. Inspect the current working tree before editing. Do not discard or overwrite
   the documentation and attribution changes listed below.
4. Continue with the verification task below. Update the plan's checkboxes,
   evidence log, and this handoff before ending the session.

## Project context

VisualLLM is a Tauri 2 desktop application with a framework-free renderer and
a Rust backend. It lets users arrange provider models into visual fallback
lanes exposed as loopback-only OpenAI-compatible endpoints.

Important implementation boundaries:

- `renderer/` owns presentation and calls a narrow Tauri bridge. It must not
  gain direct network, filesystem, or credential access.
- `src-tauri/src/main.rs` owns the Tauri shell and command boundary.
- `src-tauri/src/providers.rs` owns provider configuration, credentials, and
  catalogs.
- `src-tauri/src/server.rs` owns the Axum gateway and routing engine.
- `src-tauri/src/lanes.rs` owns lane persistence and engine bookkeeping.
- Provider secrets must never be written into `providers.json` or returned to
  the renderer.

## Decisions already made

- The working release target is **v0.6.0**.
- Linux x86_64 is the supported distribution for this release.
- Windows x86_64 and Apple Silicon macOS are deferred and not offered in
  v0.6.0. No Windows/macOS certificates or native verification are in scope.
- Intel macOS, Homebrew, AUR, and Flatpak are not offered in v0.6.0.
- Tauri updater signing is separate from future Windows Authenticode or Apple
  code signing. Do not describe deferred packages as available in v0.6.0.
- Eric Shane Gross is VisualLLM's sole creator and developer. Published Git
  history will not be force-rewritten unless `.mailmap` fails to correct the
  GitHub presentation and the user separately approves the destructive rewrite.
- Do not commit or push unless the user explicitly asks.

## Findings that drive the plan

- v0.5.2 publishes Linux x86_64 `.deb`/AppImage, Windows x86_64 `.exe`/MSI,
  and an Apple Silicon `.dmg`.
- Windows and macOS OS-signing jobs failed because paid certificate secrets are
  absent, but the release job still published the pre-signing artifacts.
- `latest.json` has Linux and Windows entries but no macOS updater entry.
- The v0.5.2 release body contains only a changelog link, leaving artifact
  selection unexplained.
- The v0.5.2 `keyring` configuration enabled only `linux-native`. On Windows
  and macOS it fell back to a non-persistent mock store while appearing to
  succeed; Workstream 1 now selects native backends for all three release
  targets.
- The Homebrew formula points to a missing tarball and is based on a Linux
  AppImage; AUR is disabled; Flatpak is mentioned but not published.
- GitHub Actions currently refuses to start jobs because the account is locked
  over a billing issue. This is an external blocker, not a code failure.

## Exact next task

Complete the Linux-only release path:

1. Run the Linux CI build and manually verify save → fully quit → relaunch →
   provider test on Linux.
2. Run a tagged Linux release rehearsal and verify artifact selection,
   `SHA256SUMS`, and the Linux updater entry.
3. Confirm Windows/macOS builds, certificates, updater entries, and downloads
   remain out of v0.6.0. Revisit them in a future release.
4. Keep Homebrew, AUR, and Flatpak out of the core release result.

Do not begin the README installation rewrite until credential behavior and the
platform support policy are technically accurate.

## Current uncommitted work

The following intended changes exist locally and must be preserved:

- `.mailmap` — canonicalizes all project authorship as Eric Shane Gross.
- `LICENSE`, `README.md`, `package.json`, `src-tauri/Cargo.toml` — full-name and
  sole-developer attribution.
- `SECURITY.md` — full-name attribution and current credential/security policy.
- `docs/INSTALLATION_PLAN-v0.6.0.md` — active installation improvement plan.
- `docs/STATUS.md` and `ROADMAP.md` — links and v0.6.0 release focus.
- `docs/HANDOFF-v0.6.0.md` — this file.
- `.github/workflows/ci.yml` and `release.yml` — explicit Linux `libdbus`
  build dependency for the Secret Service backend.
- `DISTRIBUTION_SETUP.md` — current v0.6.0 signing/channel policy and future
  release ordering.
- `tools/verify-release.js` — local release artifact verification helper.
- `tools/verify-release.js` — local artifact, checksum, signature, architecture,
  and updater-manifest verification.

`tools/create-demo-video.sh` was already untracked before this work. It is a
local media-generation helper and must not be included accidentally.

Repository-local Git identity is configured as:

```text
Eric Shane Gross <eshanegross@gmail.com>
```

With `.mailmap`, `git shortlog -sne main` currently reports one canonical
author. Pushed commit `946f3ac` is displayed by GitHub as authored and
committed by Eric Shane Gross <eshanegross@gmail.com>; full rendered
contributor-page verification remains pending.

## Workstream 1 implementation completed

- `src-tauri/Cargo.toml` selects `linux-native-sync-persistent` on Linux;
  Windows/macOS target backend work is retained only for a future release.
- `providers.rs` references each target backend module directly, making a
  missing native feature a compile error instead of silently using keyring's
  mock backend.
- `KeyStorage` is derived from keyring's declared persistence. Mock/process-only
  backends report `memory`; failed native writes use a backend-owned process
  memory fallback and remain non-fatal.
- Focused provider tests: 7 passed. Full Rust tests: 73 passed. Clippy passed.
- `cargo tree` confirmed the Linux target feature selection. Windows/macOS
  native builds and restart tests are deferred rather than release gaps.

## Workstreams 2/6 implementation completed

- Windows/macOS OS-signing and preview jobs are deferred from v0.6.0.
- The release job publishes Linux artifacts only and does not require
  Windows/macOS certificate secrets.
- Homebrew, tarball, and Flatpak jobs are disabled as deferred channels.
- YAML parsing and `git diff --check` pass locally. A hosted dry-run remains
  blocked by the GitHub account billing lock.

## Workstream 3 implementation completed

- Release uploads include Linux updater payloads and a stable Linux x86_64
  download alias. Windows/macOS are omitted from the release and `latest.json`.
- All platform checksum files use the `SHA256SUMS` name and documented SHA-256
  line format.
- `tools/verify-release.js` checks manifest URLs, local payload/signature
  sidecars, architecture labels, checksum files, and the macOS omission before
  publication.

## Validation baseline

Run after implementation changes:

```bash
node tools/smoke.js
npm test
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
node tools/check-version.js
git diff --check
```

For documentation-only changes, at minimum run `git diff --check` and verify
all new relative links resolve.

Latest local run on 2026-08-08: renderer smoke passed; 29 Node tests passed;
Cargo format check passed; 73 Rust tests passed; Clippy passed with warnings
denied; version check passed; and `git diff --check` passed. The downloaded
v0.5.2 release assets were reachable, but the verifier rejected the historical
Windows `x64` filenames. That historical result is outside the revised
Linux-only v0.6.0 scope.

## Current session handoff

**Last completed:** Baseline audit, no-certificate decision, durable plan,
non-destructive sole-developer attribution cleanup, Workstream 1 credential
implementation, Workstream 3 artifact verification, Workstreams 2/6
release-policy workflow changes, the Linux-only scope decision, and the
complete local validation baseline.
**Next task:** Run the Linux release rehearsal and installation verification
when Actions is available. Windows/macOS builds, certificates, and native
restart tests are explicitly deferred.
**Open external blocker:** GitHub Actions run `31277675026` (2026-08-08)
failed every job within seconds, with no runner assigned and no step logs; the
account billing lock blocks hosted Linux verification, but it does not block
local development or the revised release scope.
**Working tree note:** The intended uncommitted changes remain preserved;
`tools/create-demo-video.sh` was already untracked and must not be committed.

## End-of-session protocol

Before handing off again:

1. Check only tasks whose acceptance criteria actually passed.
2. Append dated evidence to the installation plan; do not rewrite old entries.
3. Update the plan's **Current handoff** section.
4. Update this file's date, completed work, exact next task, blockers, and
   working-tree note.
5. Report tests that passed and tests that could not run separately.
