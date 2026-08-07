# Changelog

All notable changes to VisualLLM are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version
policy in the README applies: MAJOR for contract breaks, MINOR for features,
PATCH for fixes.

## [0.5.1] — 2026-08-07

Hotfix: regenerating the gateway token now takes effect immediately.

### Fixed

- **Token rotation applies live** — previously a regenerated token was
  written to disk but the running engine kept the old one until restart, so
  lane endpoints kept rejecting the new token and the editor re-apply could
  not bring them back. The engine now follows rotations through a channel and
  the lane middleware reads the token per request; `Regenerate` no longer
  claims it only applies to the next engine start.

## [0.5.0] — 2026-08-06

Post-review hardening batch: the status bar now reports the engine's own
ledger instead of gateway scaffolding, and a round of reliability and
usability fixes.

### Added

- **Auto-park budget popover** — the gauge in each hall's header tunes when
  the engine parks the endpoint (failures within a window), lit amber when
  tuned away from the standard budget.
- **Per-lane notification mutes** — silencing a notification type applies to
  one lane only (legacy global mutes still work).
- **Apply to editors** — one click refreshes the bearer token and endpoint
  URL in every saved editor integration after a token regeneration or a port
  move; the regenerate action offers it directly.

### Changed

- The status bar reads the engine's `/health` (live lane/model counts and
  trailing-24h requests/failures) rather than the retired gateway sidecar.
- Lane **Setup** copies a curl example with the `Authorization: Bearer` header
  already filled in; lane routes always required it.
- Notification toast cards are capped at three so a failure burst can't bury
  the screen; polling and the short clocks pause while the window is hidden
  and catch up on `visibilitychange`.
- README and the token panel now state the bearer requirement and the
  re-apply path.

### Fixed

- Removed the stale gateway scaffolding comment on the engine URL helper.

## [0.4.0] — earlier release

Rolling per-lane request/failure counters, lane auto-parking after repeated
transient failures, incident replay from the notification center, and checked
error handling across the engine.
