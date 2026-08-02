# VisualLLM

Design LLM endpoints by hand.

Point it at a provider, and every model that provider offers appears in the
sidebar. Drag the ones you want into a lane. The model on the right answers
first; everything to its left is a fallback, tried in order when the one ahead
of it cannot serve — out of credit, rate limited, context too small, provider
down. The lane gets a name and a URL, and any OpenAI-compatible client can call
it.

Routing is configuration in every other tool of this kind. Here it is the
interface.

```bash
npm run dev      # run it
npm run build    # .deb and AppImage in src-tauri/target/release/bundle/
```

Requires the Rust toolchain, Node, and on Linux the WebKit development
packages: `libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev
librsvg2-dev`.

## How it is put together

| path | what |
|---|---|
| `src-tauri/src/main.rs` | the shell, and every command the UI is allowed to call |
| `src-tauri/src/providers.rs` | providers, key storage, and catalog fetching |
| `renderer/` | the surface: `index.html`, `style.css`, `app.js`. No build step, no framework |
| `src-tauri/capabilities/default.json` | exactly what the webview may ask for |

All network access lives in Rust. The webview has no HTTP and no filesystem —
it asks for state and gets state, which keeps the reachable surface to the
handful of commands in `main.rs`.

## Two rules the canvas is built around

1. **`members[0]` answers first.** That is the only ordering the data has.
2. **The track draws that list right to left**, so the primary sits at the
   right-hand edge under the arrow.

The reversal lives in exactly two functions — `renderTrack` and
`domSlotToIndex`. If the direction ever changes, those are the only two places
to touch, and there is no third.

## What the catalog is trusted for

OpenRouter publishes more than most providers, and three details are worth
knowing because getting them wrong is silent:

- **Context comes from `top_provider.context_length`**, never the model-level
  `context_length`. The latter overstates — one model advertises 262K on an
  endpoint that caps at 131K.
- **`supported_parameters` is a union across every provider serving a model**,
  so a capability listed there is optimistic rather than promised. Real support
  is per-provider, on the endpoints resource.
- **A missing benchmark score is missing, not zero.** Unscored models sort to
  the bottom rather than ranking as the worst measured.

## Known, and deliberate

**API keys are stored in plaintext** in `providers.json` under the OS app-data
directory, owner-read-only. That is a floor, not a solution: it wants the system
keychain before anyone else installs this. Storage is behind a seam in
`providers.rs`, so replacing the backend touches nothing else.

**Lane arrangements are not persisted.** Rearranging changes the canvas, not
anything on disk. The ordering model was the thing under test; it has held up,
so this is the next thing to build.

**There is no engine yet.** Lanes are designed here but served by a separate
gateway. Whether this app grows its own proxy or drives an external one is the
open architectural question.
