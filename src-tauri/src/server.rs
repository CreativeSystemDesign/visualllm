//! The engine. Lanes drawn on the canvas are served here.
//!
//! One route matters: `POST /lane/{slug}/v1/chat/completions`. It takes an
//! OpenAI-shaped request, walks that lane's models in order, and streams back
//! whichever one answers. `GET /v1/models` exists so a client can discover the
//! lanes rather than being told about them.
//!
//! Two decisions shape everything below.
//!
//! **Capability is checked, never discovered.** A text-only model sent an image
//! does not reliably fail — plenty of endpoints accept the content array, drop
//! the image parts, and answer from the text alone. Falling through on error
//! would turn that into a confident answer about a picture nobody looked at. So
//! members that cannot serve a request are skipped before they are contacted,
//! using the cached catalog.
//!
//! **State is read per request.** Lanes, providers and catalog come off disk
//! every time. It costs microseconds at this scale and means a lane rearranged
//! on the canvas is live on the next call, with no reload and no shared mutable
//! state between the window and the server.

use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::{lanes, providers};

#[derive(Clone)]
pub struct Engine {
    pub dir: PathBuf,
}

// --------------------------------------------------------------- the request

/// What a request actually needs, so members that cannot supply it are skipped.
struct Needs {
    vision: bool,
    tools: bool,
    /// Characters in, divided by four. Crude, and only ever used to reject a
    /// model whose window is clearly too small — never to choose between two
    /// that both fit.
    tokens: u64,
}

fn inspect(body: &Value) -> Needs {
    let mut vision = false;
    let mut chars = 0usize;

    if let Some(messages) = body["messages"].as_array() {
        for message in messages {
            match &message["content"] {
                Value::String(text) => chars += text.len(),
                Value::Array(parts) => {
                    for part in parts {
                        match part["type"].as_str() {
                            Some("image_url") | Some("image") | Some("input_image") => {
                                vision = true
                            }
                            _ => chars += part["text"].as_str().map(str::len).unwrap_or(0),
                        }
                    }
                }
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

/// Whether a model can serve this request at all.
///
/// A capability flag is only believed when it is present: a generic provider
/// returns model ids and nothing else, and refusing to route to everything it
/// offers would make those providers useless. Absence of evidence is treated as
/// permission, and a `context` of zero means unknown rather than tiny.
fn can_serve(model: &providers::CatalogModel, needs: &Needs, known: bool) -> bool {
    if !known {
        return true; // nothing published about it; let the provider decide
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

// -------------------------------------------------------------- the response

/// Why an attempt failed, and whether the next model is worth trying.
enum Verdict {
    /// This model could not do it; another might.
    TryNext(String),
    /// The request itself is wrong. Every model will say the same, so stop.
    Fatal(StatusCode, String),
}

fn classify(status: StatusCode, body: &str) -> Verdict {
    let reason = |label: &str| format!("{label} ({})", status.as_u16());
    match status.as_u16() {
        400 | 422 => {
            // One exception: a window too small is this model's problem, and a
            // later one may be bigger.
            if body.contains("context") && (body.contains("length") || body.contains("token")) {
                Verdict::TryNext(reason("prompt too long for this model"))
            } else {
                Verdict::Fatal(status, body.to_string())
            }
        }
        401 | 403 => Verdict::TryNext(reason("key rejected")),
        402 => Verdict::TryNext(reason("out of credit")),
        404 => Verdict::TryNext(reason("model not available")),
        408 | 409 | 425 => Verdict::TryNext(reason("provider busy")),
        429 => Verdict::TryNext(reason("rate limited")),
        s if s >= 500 => Verdict::TryNext(reason("provider error")),
        _ => Verdict::TryNext(reason("unexpected status")),
    }
}

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

// ------------------------------------------------------------------- routing

async fn models(State(engine): State<Engine>) -> Json<Value> {
    let lanes = lanes::load(&engine.dir);
    let catalog = providers::cache_read(&engine.dir);

    let data: Vec<Value> = lanes
        .iter()
        .map(|lane| {
            let members: Vec<&providers::CatalogModel> = lane
                .members
                .iter()
                .filter_map(|id| catalog.iter().find(|m| &m.id == id))
                .collect();

            // A lane advertises the union of what its members can do, because
            // any of them may end up answering and the ladder skips the ones
            // that cannot serve a given request. Context is the largest window
            // available, for the same reason.
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

async fn health(State(engine): State<Engine>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "lanes": lanes::load(&engine.dir).len(),
        "models_cached": providers::cache_read(&engine.dir).len(),
    }))
}

async fn chat(
    State(engine): State<Engine>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let lanes = lanes::load(&engine.dir);
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

    let mut tried: Vec<Value> = Vec::new();

    for id in &lane.members {
        let entry = catalog.iter().find(|m| &m.id == id);
        let known = entry.is_some();
        let blank = providers::CatalogModel::default();
        let model = entry.unwrap_or(&blank);

        if !can_serve(model, &needs, known) {
            tried.push(json!({ "model": id, "skipped": "cannot serve this request" }));
            continue;
        }

        // Which provider actually offers it. Without a catalog entry there is
        // nothing to match on, so the first configured provider is the guess.
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

        body["model"] = json!(id);
        let base = provider.base_url.trim_end_matches('/');
        let request = providers::authorise_public(
            client.post(format!("{base}/chat/completions")),
            provider,
        );
        // Pass through what the client asked for, minus anything that would
        // confuse the upstream about who it is talking to.
        let request = match headers.get("accept") {
            Some(accept) => request.header("accept", accept),
            None => request,
        };

        match request.json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                let mut out = Response::builder()
                    .status(StatusCode::OK)
                    .header("x-visualllm-lane", &lane.slug)
                    .header("x-visualllm-served-by", id);
                if streaming {
                    out = out.header("content-type", "text/event-stream");
                } else if let Some(kind) = resp.headers().get("content-type") {
                    out = out.header("content-type", kind);
                }
                let stream = futures_util::TryStreamExt::map_err(
                    resp.bytes_stream(),
                    std::io::Error::other,
                );
                return out.body(Body::from_stream(stream)).unwrap().into_response();
            }
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

    error(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("every model in '{}' was skipped or failed", lane.name),
        "lane_exhausted",
        tried,
    )
}

pub fn router(dir: PathBuf) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/lane/{slug}/v1/chat/completions", post(chat))
        // The lane's own /v1/models, so a client pointed at one lane can still
        // discover something rather than 404.
        .route("/lane/{slug}/v1/models", get(models))
        .with_state(Engine { dir })
}

pub async fn serve(dir: PathBuf, port: u16) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("could not listen on 127.0.0.1:{port} — {e}"))?;
    axum::serve(listener, router(dir))
        .await
        .map_err(|e| e.to_string())
}
