# VisualLLM

> Build reliable OpenAI-compatible endpoints by arranging AI models visually.

VisualLLM is a visual fallback router for AI models. Add the providers you use,
browse their models, drag the ones worth keeping into a lane, and connect tools
such as VS Code to the resulting local endpoint.

The model on the right answers first. Models to its left are fallbacks. When a
provider is out of credit, rate-limited, unavailable, too small for the request,
or returns an unusable answer, VisualLLM explains what happened and tries the
next suitable member.

## Why VisualLLM

Most model gateways make routing a configuration problem. VisualLLM makes it a
visible arrangement you can understand at a glance:

- **Providers** supply catalogs; credentials stay on the Rust side of the app.
- **The pool** is your shortlist of models worth considering.
- **Lanes** are local OpenAI-compatible endpoints with an explicit fallback
  order.
- **Receipts** show which model served, which models were passed over, and why.

The current product is Linux-first and available from source. A packaged public
release is planned; see [`ROADMAP.md`](ROADMAP.md).

## Quick start

For development from a cloned repository:

```bash
cd /home/shane/visualllm
node tools/smoke.js
./tools/launch-system.sh
```

Then add a provider, browse its catalog, create a lane, and copy the lane URL
into an OpenAI-compatible client. The default engine listens on
`http://127.0.0.1:4100`.

## Documentation

- [`ROADMAP.md`](ROADMAP.md) — public-release milestones and criteria.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — development and pull-request guidance.
- [`SECURITY.md`](SECURITY.md) — threat model and vulnerability reporting.
- [`LICENSE`](LICENSE) — project license.

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

On Linux, the built desktop binary lives at `src-tauri/target/debug/visualllm`.
It can be launched directly with a clean GTK/X11 environment:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
env -i \
  HOME="$HOME" \
  PATH="/usr/bin:/bin:$HOME/.cargo/bin" \
  DISPLAY="${DISPLAY:-:0}" \
  XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}" \
  ./src-tauri/target/debug/visualllm
```

If the UI shows a connection error, the app is failing to reach its own engine
on `http://127.0.0.1:4100`; the backend serves the renderer from `renderer/`
and the engine endpoints from `/v1/models` and `/lane/{slug}/v1/chat/completions`.

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

**The two 429s are still treated alike.** OpenRouter returns the same status for
"this provider is throttling you" — where another model fixes it — and "your
whole free tier is blocked" — where nothing but waiting does. Walking the rest
of a lane during the second kind burns quota and deepens the hole. They are
distinguishable: the account-wide block reports `provider_name: null` in the
error payload. Reading the body rather than the status is the fix.

**Capability comes from a cached catalog, and a wrong entry fails quietly.** If
the catalog says a model can't do something it can, `can_serve` skips it and
nothing errors. Every response now carries `x-visualllm-passed-over` and
`x-visualllm-trail` so this is visible rather than silent, but the underlying
data is only as good as the last fetch. A capability is only ever *disqualifying*
when the provider actually published it (`caps_known`) — a generic `/models`
that says nothing about vision has not said "no vision".

## What it is

One program: the UI, the gateway, and the engine that runs on it. A lane
designed on the canvas is served by the same binary that drew it — an HTTP
listener on `127.0.0.1:4100` answering `/lane/{slug}/v1/chat/completions`,
walking that lane's models in order and streaming back whichever one answers.
`GET /v1/models` lists your lanes, so a client like VS Code can discover them.
Nothing is configured anywhere else. There is no config file to learn.

Every response says how it was served:

    x-visualllm-served-by:  which model actually answered
    x-visualllm-passed-over: how many were skipped or failed first
    x-visualllm-trail:       each one, and why

A model's identity everywhere — the pool, a lane, these headers — is the pair
of provider and id, written `model@provider` where it has to fit on one line.
The id alone stopped being an identity the moment two providers could carry
the same one: `deepseek-chat` direct and through a reseller are different
endpoints, different keys, different bills. Files from before this rule hold
bare ids and still load; they mean "whichever provider first matches", which
is what they always meant.

The hard part is not the proxying, it is **deciding when a model has failed
hard enough to move to the next one**. That judgement is the whole value of a
fallback chain, and it is wrong in both directions: too eager and the user's
chosen primary gets skipped over a blip, too strict and the lane stalls on a
model that was never going to answer. Some of it is unobvious — a 429 meaning
"this provider is throttling, try another model" and a 429 meaning "your whole
free tier is blocked, only waiting helps" look alike and need opposite
responses.

That judgement no longer stops at the status line. **A 200 is provisional
until it carries something a client can render** — a content token or a tool
call. Providers return 200 and then stream an error event, or nothing, or
spend the whole token budget on hidden reasoning and end with zero visible
content; a chat client shows all of these as an empty reply. The engine holds
every response at the door until its first usable delta, and one that dies
before that point fails over to the next member with the reason in the trail
("spent the whole token budget reasoning, with no room left to answer").
Three consequences, all deliberate:

- Nothing reaches the client until the commit point, so a thinking model's
  reasoning is not shown live — it arrives in one piece with the answer.
  Forwarded bytes cannot be unsent, and forwarding early would spend the
  lane's one chance to fall back.
- A member that fails *after* its first content token is a failed request,
  not a fallback. Splicing two models' answers mid-stream would be worse.
- Requests with tiny token budgets (under 16) skip the gate: a one-token
  health probe is not a request for an answer, and judging it would fail
  every monitoring script ever written.

Lanes can also ask members not to think at all — the "no thinking" toggle on
a lane injects the reasoning-off knob for providers that expose one (today:
OpenRouter, which normalises it across models). It is a preference, not a
guarantee; the commit gate catches the models that think anyway.

**Loopwatch** (per-lane, opt-in) watches for an agent stuck re-calling the
same tool. Agentic clients resend the whole conversation every turn, so a
loop is visible inside a single request — no cross-request state needed.
Two species, one definition of stuck: *receiving no new information*. The
treatment is the one that measured 0/4 repeats against captured loops
(control: 4/4): collapse the redundant call/result pairs — only ever a pair
whose call *and* result are byte-identical to a later one, so an edit can
never be hidden — and append a note naming the loop as the **last** message.
Placement is the finding: the same note in the system prompt did nothing,
because at 150K tokens the system prompt is 150K tokens in the past. The
note only ever describes a **live** loop — one the conversation's most
recent call belongs to. Clients resend their transcripts forever,
duplicates included, and stale residue is merely swept, or the model would
be told about yesterday's loop while today's goes unnamed. This is the
engine's only modification of a conversation; it is logged, and announced
on the response in `x-visualllm-unstuck`.

**Every failure is explained, never just badged.** The engine records each
one with its receipts — the provider's own bytes, the loop counts, which
lane toggles were on at the time — and they arrive as notifications: a card
at the bottom right that waits to be clicked, and a bell in the status bar
that lights up with a count when cards fade away unviewed. Click a card (or
the bell) and the full diagnosis opens: what happened (evidence quoted
verbatim), why it happens (the mechanism), and what to try (a specific
control here, one click away when it is one of the lane's own toggles). Any
type can be ignored — the engine keeps recording it; only the announcement
goes silent, reversibly. The standard is strict in both directions: a
malformed request is recorded as the client's fault, and a failure the
evidence cannot attribute renders as "unexplained, receipts attached"
rather than being rounded up to a verdict. This app is built for people who
choose free models; free models earn reputations by rumour, and receipts
beat rumours.

`classify` (refusals) and the commit gate (acceptances) in `server.rs` are
the judgement, and they are what the tests pin down — `cargo test` from
`src-tauri/`.

Next, in order: split the two 429 *statuses* by reading the error body (the
mid-stream kind inside a 200 is already caught); move keys to the system
keychain; check capabilities per-provider rather than trusting the catalog's
union.

---

## Step-by-step launch (Linux)

### 1. Install system dependencies

```bash
# Ubuntu / Debian
sudo apt update && sudo apt install -y \
  libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev \
  build-essential curl wget file libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev

# Fedora
sudo dnf install -y webkit2gtk4.1-devel libxdo-devel libayatana-appindicator-gtk3-devel librsvg2-devel \
  gcc gcc-c++ make openssl-devel gtk3-devel

# Arch
sudo pacman -S webkit2gtk-4.1 libxdo libayatana-appindicator librsvg base-devel openssl gtk3
```

### 2. Install Rust toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup default stable
```

### 3. Install Node.js (for `npm run dev` / `npm run build`)

```bash
# via nvm (recommended)
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
source ~/.bashrc
nvm install --lts

# or via package manager
# sudo apt install nodejs npm
```

### 4. Build the app

```bash
# From the repo root
cd /path/to/visualllm

# Development build (fast, unoptimised)
cargo build --manifest-path src-tauri/Cargo.toml

# Release build (optimised, used for .deb / AppImage)
cargo build --release --manifest-path src-tauri/Cargo.toml
```

The binary will be at:
- `src-tauri/target/debug/visualllm` (dev)
- `src-tauri/target/release/visualllm` (release)

### 5. Run the app (direct binary launch)

The Tauri dev server (`npm run dev`) works but inherits the VS Code snap environment, which causes library conflicts. The reliable way is to launch the built binary directly with a clean GTK/X11 environment:

```bash
# From the repo root
export PATH="$HOME/.cargo/bin:$PATH"

env -i \
  HOME="$HOME" \
  PATH="/usr/bin:/bin:$HOME/.cargo/bin" \
  DISPLAY="${DISPLAY:-:0}" \
  XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}" \
  ./src-tauri/target/debug/visualllm
```

**What each part does:**
- `env -i` — start with an empty environment (no snap library paths)
- `HOME="$HOME"` — needed for app data directory (`~/.local/share/app.visualllm`)
- `PATH="/usr/bin:/bin:$HOME/.cargo/bin"` — system bins + cargo only
- `DISPLAY="${DISPLAY:-:0}"` — your X11/Wayland display (usually `:0`)
- `XAUTHORITY="${XAUTHORITY:-$HOME/.Xauthority}"` — X authority cookie (Wayland uses a mutter path like `/run/user/1000/.mutter-Xwaylandauth.*`)

If the window doesn't appear, check your X authority file:

```bash
echo $XAUTHORITY
ls -l "$XAUTHORITY"
# If empty or missing, find it with:
xauth list
# Then use that path in the launch command above
```

### 6. Verify it's working

The app opens a frameless window. The engine serves on `http://127.0.0.1:4100`:

```bash
# Health check
curl http://127.0.0.1:4100/health

# List lanes (OpenAI-compatible /v1/models)
curl http://127.0.0.1:4100/v1/models

# Chat completion through a lane
curl -X POST http://127.0.0.1:4100/lane/new-lane/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"new-lane","messages":[{"role":"user","content":"Hello"}],"max_tokens":50}'
```

### 7. Package for distribution (optional)

```bash
npm run build
# Outputs .deb and AppImage in src-tauri/target/release/bundle/
```

### Troubleshooting

| Symptom | Fix |
|---|---|
| `symbol lookup error: libpthread` | You're inheriting the snap `LD_LIBRARY_PATH`. Use the `env -i` launch command above. |
| `Failed to initialize GTK` | Missing `DISPLAY` or `XAUTHORITY`. Ensure you're on a graphical session and the variables are set. |
| `Address already in use (port 4100)` | A previous instance is still running: `pkill -f visualllm && fuser -k 4100/tcp` |
| Window opens but shows "Could not connect" | The engine didn't start. Check the terminal for `engine: could not listen...` — usually a stale process on 4100. |
| Transparent window shows black/garbled | Your compositor doesn't support ARGB visuals. Set `"transparent": false` in `src-tauri/tauri.conf.json` and rebuild. |

## Public project roadmap

The public-release plan, milestones, non-goals, and release criteria live in
[`ROADMAP.md`](ROADMAP.md). Contributions and security guidance are documented
in [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`SECURITY.md`](SECURITY.md).
