# Contributing to VisualLLM

Thanks for helping improve VisualLLM.

## Before opening an issue

Please search existing issues first. For a bug report, include:

- operating system and distribution;
- VisualLLM version or commit;
- provider type, without including credentials;
- the smallest reproduction you can provide;
- relevant error text or a redacted incident receipt.

Do not post API keys, tokens, private prompts, or sensitive provider responses.

## Development

VisualLLM is a Tauri application with a plain HTML/CSS/JavaScript renderer and
Rust backend. See `README.md` for prerequisites and `ROADMAP.md` for the current
priorities.

### Running the app

**Must run from a clean terminal outside VS Code.** VS Code (when installed as a snap) pollutes the terminal environment with snap library paths (`LD_LIBRARY_PATH`, `GTK_PATH`, `GIO_MODULE_DIR`, `LOCPATH`, `XDG_DATA_DIRS`) that cause a `GLIBC_PRIVATE` symbol lookup error at runtime.

```bash
# From a fresh terminal (Ctrl+Alt+T on GNOME):
cd /home/shane/visualllm/src-tauri
~/.cargo/bin/cargo run
```

If you must run from within VS Code's terminal, unset the contaminated variables first:
```bash
unset LD_LIBRARY_PATH GTK_PATH GIO_MODULE_DIR LOCPATH XDG_DATA_DIRS
cd /home/shane/visualllm/src-tauri && ~/.cargo/bin/cargo run
```

### Useful checks from the repository root:

```bash
node tools/smoke.js
cargo test --manifest-path src-tauri/Cargo.toml
```

For Linux development, use `tools/launch-system.sh` when testing the desktop
binary outside a VS Code Snap environment.

## Pull requests

Keep pull requests focused. Explain:

- the user problem being solved;
- the behavior changed;
- how it was tested;
- any migration, security, or compatibility impact.

Changes involving provider keys, Tauri permissions, network binding, persistence
formats, or fallback decisions need explicit tests and documentation.
