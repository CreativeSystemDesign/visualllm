#!/usr/bin/env bash
# Launch VisualLLM outside VS Code/Snap's inherited GTK/GLib environment.
# This intentionally starts a new instance only; it never stops or replaces
# an already-running engine on port 4100.
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
BIN="$ROOT/src-tauri/target/debug/visualllm"

if [[ ! -x "$BIN" ]]; then
  printf 'VisualLLM binary not found or not executable:\n  %s\n' "$BIN" >&2
  printf 'Build it first with: cargo build --manifest-path src-tauri/Cargo.toml\n' >&2
  exit 1
fi

# Do not create a second broken window when the engine is already serving. The
# desktop app currently has one engine port, so an existing healthy response
# means another VisualLLM process owns the application state.
if command -v curl >/dev/null 2>&1 && curl -fsS --max-time 1 http://127.0.0.1:4100/health >/dev/null 2>&1; then
  printf 'VisualLLM is already running on http://127.0.0.1:4100.\n' >&2
  printf 'Use the existing window, or close it before launching another instance.\n' >&2
  exit 0
fi

# Preserve only the session values needed by GTK/WebKit and the app data path.
# In particular, do not pass LD_LIBRARY_PATH from a VS Code Snap terminal.
env -i \
  HOME="$HOME" \
  USER="${USER:-$(id -un)}" \
  PATH="/usr/bin:/bin:$HOME/.cargo/bin" \
  DISPLAY="${DISPLAY:-:0}" \
  XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}" \
  "$BIN" "$@"
