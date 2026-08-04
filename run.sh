#!/usr/bin/env bash
# Convenience wrapper: build then launch VisualLLM with a sanitized env
set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
cd "$ROOT"

echo "Building backend..."
cargo build --manifest-path src-tauri/Cargo.toml

echo "Launching VisualLLM (sanitized environment)..."
exec ./tools/launch-system.sh "$@"
