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
so this is the next thing to build — and the engine below cannot start until it
is, because the server has to read the arrangement.

**The engine is not built yet.** Lanes are currently designed here and served
elsewhere. That is temporary scaffolding, not the architecture.

## What this is meant to be

One program: the UI, the gateway, and the engine that runs on it. A lane
designed on the canvas is served by the same binary that drew it — an HTTP
listener answering `/lane/{slug}/v1/chat/completions`, walking that lane's
models in order and streaming back whichever one answers. Nothing is configured
anywhere else. There is no config file to learn.

The hard part is not the proxying, it is **deciding when a model has failed
hard enough to move to the next one**. That judgement is the whole value of a
fallback chain, and it is wrong in both directions: too eager and the user's
chosen primary gets skipped over a blip, too strict and the lane stalls on a
model that was never going to answer. Some of it is unobvious — a 429 meaning
"this provider is throttling, try another model" and a 429 meaning "your whole
free tier is blocked, only waiting helps" look alike and need opposite
responses.

Build order: persist lanes, then the listener serving one model per lane, then
the fallback ladder with real failure classification. Each step is testable on
its own, and the first two are small.
