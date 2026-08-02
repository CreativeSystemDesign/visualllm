//! THE ENGINE — where a lane you drew becomes something a program can call.
//!
//! ============================================================================
//! HOW THIS FILE WORKS, TOP TO BOTTOM
//! ============================================================================
//!
//! A web server is a loop: wait for a connection, read the request, decide what
//! to do, write a response. `axum` (the library we use) hides the loop and lets
//! us write one function per URL. `router()` near the bottom is the table that
//! says which URL goes to which function.
//!
//! There is one route that matters:
//!
//!     POST /lane/{slug}/v1/chat/completions
//!
//! `{slug}` is a placeholder — a request to `/lane/fast/v1/chat/completions`
//! calls `chat()` with `slug = "fast"`. That function looks the lane up, works
//! out which of its models can serve this particular request, tries them in
//! your order, and streams back whichever one answers.
//!
//! ============================================================================
//! TWO IDEAS THAT SHAPE EVERYTHING BELOW
//! ============================================================================
//!
//! 1. CAPABILITY IS CHECKED, NEVER DISCOVERED.
//!
//!    You might expect that sending an image to a text-only model produces an
//!    error we could catch and move on from. It often doesn't. Many endpoints
//!    accept the request, silently ignore the image, and answer from the text
//!    alone — so you get a confident description of a picture nobody looked at.
//!    A wrong answer is far worse than a failed one, because nothing alerts you.
//!
//!    So we check the cached catalog *before* contacting anything, and skip
//!    models that can't serve the request.
//!
//! 2. STATE IS READ FRESH ON EVERY REQUEST.
//!
//!    If you have a PLC background, the instinct here is a scan cycle: latch a
//!    consistent image of the world, run the logic, write the outputs. There is
//!    no scan cycle in a web server. Ten requests can be in flight at once, each
//!    at a different moment, with no coordination between them.
//!
//!    The usual answer is to hold state in memory and guard it with a lock, so
//!    only one request touches it at a time. Locks are also where concurrent
//!    programs deadlock and stall. We sidestep the whole category: each request
//!    reads the small JSON files off disk itself. It costs microseconds, needs
//!    no lock, and it means a lane you rearrange on the canvas is live on the
//!    very next call — no reload, no restart.

use std::path::PathBuf;

// `use` is just shorthand so we can write `Json` instead of `axum::Json`
// everywhere. The braces group several imports from the same place.
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

// `crate::` means "from this program", as opposed to an external library.
use crate::{lanes, providers};

/// Shared context every route handler receives.
///
/// `#[derive(Clone)]` asks the compiler to write the "make a copy" code for us.
/// axum hands each request its own copy of this, which is cheap here because a
/// `PathBuf` is just a path string. This is the *only* thing shared between
/// requests, and it never changes — which is exactly why no locking is needed.
#[derive(Clone)]
pub struct Engine {
    pub dir: PathBuf,
}

// ============================================================================
// READING THE REQUEST
// ============================================================================

/// What an incoming request actually requires, so members that can't supply it
/// are skipped instead of being tried and failing (or worse, not failing).
struct Needs {
    vision: bool,
    tools: bool,
    /// Characters divided by four — a rough token estimate.
    ///
    /// Real tokenisation differs per model and would mean pulling in a big
    /// dependency. We only ever use this to reject a model whose window is
    /// *clearly* too small, never to choose between two that both fit, so being
    /// approximately right is genuinely good enough.
    tokens: u64,
}

/// Walk the request body and work out what it needs.
///
/// The wrinkle: OpenAI's format allows `content` to be either a plain string,
/// or an array of parts when the message carries images. Both shapes are legal
/// and both arrive in practice, so we handle each.
fn inspect(body: &Value) -> Needs {
    let mut vision = false;
    let mut chars = 0usize;

    // `if let Some(x) = ...` means "if this optional value exists, name it x".
    // Rust has no null; a thing that might be missing is an `Option`, and the
    // compiler forces you to say what happens when it's absent. That is why
    // null-pointer crashes essentially don't happen in this language.
    if let Some(messages) = body["messages"].as_array() {
        for message in messages {
            // `match` is a switch that the compiler checks for completeness.
            match &message["content"] {
                // Simple case: content is just text.
                Value::String(text) => chars += text.len(),

                // Richer case: an array of parts, any of which may be an image.
                Value::Array(parts) => {
                    for part in parts {
                        match part["type"].as_str() {
                            // Three spellings because different clients emit
                            // different ones. Copilot, Cline and the OpenAI SDK
                            // do not agree.
                            Some("image_url") | Some("image") | Some("input_image") => {
                                vision = true
                            }
                            // Anything else, count its text toward the estimate.
                            _ => chars += part["text"].as_str().map(str::len).unwrap_or(0),
                        }
                    }
                }
                // Some other shape we don't recognise. Ignore rather than fail:
                // being wrong about the estimate is survivable, refusing a valid
                // request is not.
                _ => {}
            }
        }
    }

    Needs {
        vision,
        tools: body["tools"].as_array().map(|t| !t.is_empty()).unwrap_or(false),
        tokens: (chars / 4) as u64,
    }
}

/// Can this model serve this request at all?
///
/// The `known` flag carries something subtle. A generic OpenAI-compatible
/// provider returns a list of model ids and nothing else — no capabilities, no
/// context sizes. If we treated "no published capability" as "cannot do it",
/// every model from such a provider would be skipped forever and the provider
/// would be useless.
///
/// So: absence of evidence is permission. We only skip a model when the catalog
/// positively tells us it can't. Likewise `context == 0` means *unknown*, not
/// *zero-sized*, and must not be used to reject anything.
fn can_serve(model: &providers::CatalogModel, needs: &Needs, known: bool) -> bool {
    if !known {
        return true; // nothing published; let the provider decide
    }
    if needs.vision && !model.vision {
        return false;
    }
    if needs.tools && !model.tools {
        return false;
    }
    if model.context > 0 && needs.tokens > model.context {
        return false;
    }
    true
}

// ============================================================================
// DECIDING WHETHER TO KEEP GOING
// ============================================================================

/// The judgement at the heart of a fallback chain: this model failed — is the
/// next one worth trying?
///
/// Get it wrong in either direction and the lane misbehaves quietly. Too eager
/// to move on, and a momentary blip skips the model you deliberately put first.
/// Too reluctant, and the lane stalls on something that was never going to
/// answer while three working models sit behind it.
///
/// An `enum` here is a value that is exactly one of these cases, and the
/// compiler will not let a `match` forget one.
enum Verdict {
    /// This model couldn't; another might. Carries a note for the report.
    TryNext(String),
    /// The *request* is wrong. Every model would reject it identically, so
    /// trying more only wastes the user's rate limit and their time.
    Fatal(StatusCode, String),
}

fn classify(status: StatusCode, body: &str) -> Verdict {
    let reason = |label: &str| format!("{label} ({})", status.as_u16());

    match status.as_u16() {
        // 400/422 mean "I don't understand this request". Normally fatal —
        // except for one important case below.
        400 | 422 => {
            // A prompt that overflows the window arrives as a 400, but it is
            // this *model's* limitation rather than a defect in the request. A
            // model further down the lane may have a bigger window, so this one
            // case is worth continuing on.
            if body.contains("context") && (body.contains("length") || body.contains("token")) {
                Verdict::TryNext(reason("prompt too long for this model"))
            } else {
                Verdict::Fatal(status, body.to_string())
            }
        }

        // Bad or missing key for this provider. Another provider's model may
        // still work, so keep walking.
        401 | 403 => Verdict::TryNext(reason("key rejected")),

        // Out of credit. Exactly the case that motivated fallback lanes.
        402 => Verdict::TryNext(reason("out of credit")),

        // Model retired, or this provider never carried it.
        404 => Verdict::TryNext(reason("model not available")),

        408 | 409 | 425 => Verdict::TryNext(reason("provider busy")),

        // Rate limited. Note for later: there are really two kinds of 429 —
        // "this provider is throttling you" (another model fixes it) and "your
        // whole free tier is blocked" (nothing fixes it but waiting). They look
        // identical here. Telling them apart needs the response body, and it is
        // worth doing.
        429 => Verdict::TryNext(reason("rate limited")),

        // Anything 500 and up is the provider's problem, not ours.
        s if s >= 500 => Verdict::TryNext(reason("provider error")),

        _ => Verdict::TryNext(reason("unexpected status")),
    }
}

/// Build an error response that says what was attempted.
///
/// A lane that goes quiet is maddening to debug, so every failure carries the
/// list of models tried and why each one was passed over. Explicable beats
/// terse.
fn error(status: StatusCode, message: String, kind: &str, tried: Vec<Value>) -> Response {
    (
        status,
        Json(json!({
            "error": { "message": message, "type": kind },
            "visualllm": { "tried": tried },
        })),
    )
        .into_response()
}

// ============================================================================
// THE ROUTES
// ============================================================================

/// `GET /v1/models` — what a client asks to discover what it can use.
///
/// This is why VS Code's model picker can show your lanes. Point it at this
/// server and it calls here; whatever we list is what appears in the dropdown.
///
/// `State(engine)` looks like magic and isn't: axum sees the parameter type and
/// supplies the matching value. It is a lookup by type rather than by name.
async fn models(State(engine): State<Engine>) -> Json<Value> {
    let lanes = lanes::load(&engine.dir);
    let catalog = providers::cache_read(&engine.dir);

    let data: Vec<Value> = lanes
        .iter()
        .map(|lane| {
            // For each model id in the lane, find its catalog entry. `filter_map`
            // does two jobs at once: transform, and drop anything that came back
            // empty. A model that has since vanished from the catalog simply
            // isn't counted rather than crashing us.
            let members: Vec<&providers::CatalogModel> = lane
                .members
                .iter()
                .filter_map(|id| catalog.iter().find(|m| &m.id == id))
                .collect();

            // A lane advertises the UNION of what its members can do, not the
            // intersection, and not just the first model's.
            //
            // Why union: any member may end up answering, and `can_serve` above
            // skips the ones that can't handle a given request. So the lane
            // really can do anything any member can — it just might not be the
            // model you listed first that does it.
            //
            // Intersection would be the safe-looking choice and it is worse: one
            // text-only fallback would strip vision from the whole lane, and the
            // client would refuse to send images at all.
            json!({
                "id": lane.slug,
                "object": "model",
                "owned_by": "visualllm",
                "name": lane.name,
                "context_length": members.iter().map(|m| m.context).max().unwrap_or(0),
                "capabilities": {
                    "vision": members.iter().any(|m| m.vision),
                    "tools": members.iter().any(|m| m.tools),
                },
                "visualllm": { "members": lane.members },
            })
        })
        .collect();

    Json(json!({ "object": "list", "data": data }))
}

/// `GET /health` — a cheap "are you alive" for scripts and for us.
async fn health(State(engine): State<Engine>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "lanes": lanes::load(&engine.dir).len(),
        "models_cached": providers::cache_read(&engine.dir).len(),
    }))
}

/// `POST /lane/{slug}/v1/chat/completions` — the one that does the work.
///
/// `async fn` means this function can pause partway through — at every `.await`
/// below — and let other requests run while it waits on the network. That is
/// how one process serves many simultaneous requests without a thread each.
/// Unlike a PLC scan, execution here is genuinely interleaved.
async fn chat(
    State(engine): State<Engine>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    // ---- 1. Find the lane -------------------------------------------------

    let lanes = lanes::load(&engine.dir);

    // `let ... else` handles the missing case and leaves the happy path
    // unindented. The `else` block must exit — return, break, or panic — so the
    // compiler knows `lane` definitely exists after this line.
    let Some(lane) = lanes.iter().find(|l| l.slug == slug) else {
        return error(
            StatusCode::NOT_FOUND,
            format!("no lane called '{slug}'"),
            "lane_not_found",
            vec![],
        );
    };

    if lane.members.is_empty() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("lane '{}' has no models in it", lane.name),
            "lane_empty",
            vec![],
        );
    }

    // ---- 2. Work out what the request needs -------------------------------

    let catalog = providers::cache_read(&engine.dir);
    let configured = providers::load(&engine.dir);
    let needs = inspect(&body);
    let streaming = body["stream"].as_bool().unwrap_or(false);

    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(err) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                err.to_string(),
                "engine_error",
                vec![],
            )
        }
    };

    // A running account of what we attempted, returned if everything fails.
    let mut tried: Vec<Value> = Vec::new();

    // ---- 3. Walk the lane in order ----------------------------------------

    for id in &lane.members {
        let entry = catalog.iter().find(|m| &m.id == id);
        let known = entry.is_some();

        // If the catalog has nothing on this model we still need *something* to
        // pass to `can_serve`, so use an empty record. Combined with `known`,
        // that means "no published limits" rather than "no capabilities".
        let blank = providers::CatalogModel::default();
        let model = entry.unwrap_or(&blank);

        if !can_serve(model, &needs, known) {
            tried.push(json!({ "model": id, "skipped": "cannot serve this request" }));
            continue; // never contacted — this is idea #1 from the header
        }

        // Which provider actually offers this model, and therefore whose key and
        // base URL to use. With no catalog entry there is nothing to match on,
        // so the first configured provider is the best available guess.
        let provider = configured
            .iter()
            .find(|p| known && p.id == model.provider_id)
            .or_else(|| configured.first());

        let Some(provider) = provider else {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "no providers configured".into(),
                "no_provider",
                tried,
            );
        };

        // The client addressed a *lane*; the provider needs a real model id.
        // This one line is the whole translation.
        body["model"] = json!(id);

        let base = provider.base_url.trim_end_matches('/');
        let request = providers::authorise_public(
            client.post(format!("{base}/chat/completions")),
            provider,
        );
        let request = match headers.get("accept") {
            Some(accept) => request.header("accept", accept),
            None => request,
        };

        // `.await` is the pause point: this request's work stops here, other
        // requests run, and we resume when the provider responds.
        match request.json(&body).send().await {
            // ---- it answered ----
            Ok(resp) if resp.status().is_success() => {
                let mut out = Response::builder()
                    .status(StatusCode::OK)
                    // Two headers of our own, so you can always tell which lane
                    // handled a call and which model inside it actually spoke.
                    .header("x-visualllm-lane", &lane.slug)
                    .header("x-visualllm-served-by", id);

                if streaming {
                    out = out.header("content-type", "text/event-stream");
                } else if let Some(kind) = resp.headers().get("content-type") {
                    out = out.header("content-type", kind);
                }

                // STREAMING, and why it matters.
                //
                // We do not wait for the full answer and then forward it. Bytes
                // are piped through as they arrive, so the complete response
                // never exists in one piece anywhere in this program.
                //
                // Two reasons. Memory: a long answer would otherwise sit in RAM
                // in full. And far more visibly — buffering means the client
                // sees nothing for thirty seconds and probably gives up. Words
                // appearing one at a time in Copilot is this choice, made
                // visible.
                let stream = futures_util::TryStreamExt::map_err(
                    resp.bytes_stream(),
                    std::io::Error::other,
                );
                return out.body(Body::from_stream(stream)).unwrap().into_response();
            }

            // ---- it replied, but with an error ----
            Ok(resp) => {
                let status = StatusCode::from_u16(resp.status().as_u16())
                    .unwrap_or(StatusCode::BAD_GATEWAY);
                let text = resp.text().await.unwrap_or_default();

                match classify(status, &text) {
                    Verdict::Fatal(status, message) => {
                        tried.push(json!({ "model": id, "failed": "request rejected" }));
                        return error(status, message, "upstream_rejected", tried);
                    }
                    Verdict::TryNext(why) => tried.push(json!({ "model": id, "failed": why })),
                }
            }

            // ---- we never reached it ----
            Err(err) => {
                let why = if err.is_timeout() {
                    "timed out".to_string()
                } else if err.is_connect() {
                    "could not connect".to_string()
                } else {
                    err.to_string()
                };
                tried.push(json!({ "model": id, "failed": why }));
            }
        }
    }

    // ---- 4. Nothing worked ------------------------------------------------

    error(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("every model in '{}' was skipped or failed", lane.name),
        "lane_exhausted",
        tried,
    )
}

// ============================================================================
// WIRING
// ============================================================================

/// The URL table. `{slug}` is a placeholder captured into the `Path` parameter.
pub fn router(dir: PathBuf) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/lane/{slug}/v1/chat/completions", post(chat))
        // Some clients append `/v1/models` to whatever base URL you give them.
        // If someone configures a single lane as their base URL, this stops
        // discovery 404-ing on them.
        .route("/lane/{slug}/v1/models", get(models))
        .with_state(Engine { dir })
}

/// Start listening. Called once at startup from `main.rs`.
///
/// Bound to `127.0.0.1` deliberately: that address is reachable only from this
/// machine. Using `0.0.0.0` would expose an unauthenticated proxy holding the
/// user's API keys to the entire local network.
pub async fn serve(dir: PathBuf, port: u16) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("could not listen on 127.0.0.1:{port} — {e}"))?;

    axum::serve(listener, router(dir))
        .await
        .map_err(|e| e.to_string())
}
