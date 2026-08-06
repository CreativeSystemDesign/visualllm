#!/usr/bin/env bash
# Launch VisualLLM outside VS Code/Snap's inherited GTK/GLib environment.
# This intentionally starts a new instance only; it never stops or replaces
# an already-running engine on port 4100.
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
RELEASE_BIN="$ROOT/src-tauri/target/release/visualllm"
DEBUG_BIN="$ROOT/src-tauri/target/debug/visualllm"

# Prefer a release build when one exists; otherwise fall back to the debug
# build and build it on demand. This lets the same launcher work for both
# development and release-package testing.
if [[ -x "$RELEASE_BIN" ]]; then
  BIN="$RELEASE_BIN"
elif [[ -x "$DEBUG_BIN" ]]; then
  BIN="$DEBUG_BIN"
else
  BIN="$DEBUG_BIN"
  printf 'VisualLLM binary not found or not executable: %s\n' "$BIN" >&2
  printf 'Attempting to build it now (this may take a minute)...\n'
  # Build in the repo root so the manifest path is correct and the user's
  # cargo environment is respected. If build fails, report and exit.
  if ! (cd "$ROOT" && cargo build --manifest-path src-tauri/Cargo.toml); then
    printf 'Automatic build failed. Please run: cargo build --manifest-path src-tauri/Cargo.toml\n' >&2
    exit 1
  fi
  if [[ ! -x "$BIN" ]]; then
    printf 'Build completed but binary still missing: %s\n' "$BIN" >&2
    exit 1
  fi
fi

# Always start the binary. Tauri's single-instance plugin owns duplicate
# handling and focuses the existing window. A launcher-side health check cannot
# focus that window, so it made clicking the launcher appear to do nothing.

# Preserve only the session values needed by GTK/WebKit and the app data path.
# In particular, do not pass LD_LIBRARY_PATH from a VS Code Snap terminal.
#
# GDK_BACKEND=x11 avoids a Mutter/Wayland bug where transparent frameless windows
# lose stacking position on click, hopping behind the window underneath them.
env -i \
  GDK_BACKEND=x11 \
  HOME="$HOME" \
  USER="${USER:-$(id -un)}" \
  PATH="/usr/bin:/bin:$HOME/.cargo/bin" \
  DISPLAY="${DISPLAY:-:0}" \
  XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}" \
  "$BIN" "$@"
