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
