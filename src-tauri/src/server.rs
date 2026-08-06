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
//! THREE IDEAS THAT SHAPE EVERYTHING BELOW
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
//! 2. A MEMBER HAS NOT ANSWERED UNTIL A CLIENT COULD USE THE ANSWER.
//!
//!    An HTTP 200 does not mean success. Providers return 200 and then:
//!    stream an error event; stream nothing and close; or spend the entire
//!    token budget on hidden "reasoning" and end with zero visible content —
//!    which a chat client renders as an empty reply and the person reads as
//!    "broken". Status-code fallback cannot see any of this.
//!
//!    So the engine holds every response PROVISIONAL until it carries the
//!    first thing a baseline OpenAI client can render — a content token or a
//!    tool call. Only then are headers sent and bytes forwarded. A response
//!    that ends before that point is a member failure like any other: noted
//!    in the trail, next model tried, the client none the wiser. The cost is
//!    honest and deliberate: nothing reaches the client during a model's
//!    thinking phase, because forwarded bytes cannot be unsent and would
//!    otherwise spend the lane's one chance to fall back.
//!
//! 3. STATE IS READ FRESH ON EVERY REQUEST.
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
use std::sync::Arc;

// `use` is just shorthand so we can write `Json` instead of `axum::Json`
// everywhere. The braces group several imports from the same place.
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::sync::{oneshot, watch};

// `crate::` means "from this program", as opposed to an external library.
use crate::{incidents, lanes, loopwatch, providers};

/// One line of live lane activity, for the canvas.
///
/// Fallback is the product, and it was invisible while it happened: the UI
/// learned about a request only when it failed hard enough to become an
/// incident, seconds later. This is the live feed — one JSON line per phase
/// of a request, appended to `activity.jsonl`. The renderer tails it to show
/// "trying X…" and then "answered by Y · N passed over" on the lane itself.
///
/// A plain text append, not a JSON document: it is written on the hot path,
/// read by polling, and trimmed by size rather than parsed.
fn note_activity(dir: &std::path::Path, lane: &str, member: &str, phase: &str, detail: &str) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Sanitize for a single line: details can carry provider error text.
    let scrub = |s: &str| s.replace(['\n', '\r'], " ");
    let line = format!(
        "{{\"at\":{at},\"lane\":\"{lane}\",\"member\":\"{member}\",\"phase\":\"{phase}\",\"detail\":\"{}\"}}\n",
        scrub(detail)
    );
    let path = dir.join("activity.jsonl");
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(line.as_bytes());
    }
    // Trim to the newest ~64KiB so the file cannot grow unbounded. Reads
    // tolerate a torn first line; the renderer skips unparseable entries.
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 64 * 1024 {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let keep: String = text
                    .lines()
                    .rev()
                    .take(500)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = std::fs::write(&path, keep + "\n");
            }
        }
    }
}

/// Read the newest activity lines, for the renderer. The renderer polls this;
/// unparseable lines (a torn trim boundary) are skipped.
pub fn activity_read(dir: &std::path::Path, since: u64) -> Vec<Value> {
    let text = std::fs::read_to_string(dir.join("activity.jsonl")).unwrap_or_default();
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["at"].as_u64().unwrap_or(0) >= since)
        .collect()
}

/// Record one failure with its receipts, so the canvas can explain it later.
/// The note given here is the same text the trail and the log carry — one set
/// of facts, three audiences.
fn note_incident(dir: &std::path::Path, lane: &lanes::Lane, member: &str, note: &str, tools: u64) {
    incidents::record(
        dir,
        incidents::Incident {
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            lane: lane.slug.clone(),
            member: member.to_string(),
            kind: incidents::kind_of(note).to_string(),
            evidence: note.to_string(),
            no_think: lane.suppress_reasoning,
            loopwatch: lane.unstick,
            tools,
        },
    );
}

/// A member's catalog entry, honouring its provider when it has one.
///
/// An empty provider is a file from before providers were part of identity:
/// first id match, the old behaviour, so nothing built then changes now.
fn find_model<'a>(
    catalog: &'a [providers::CatalogModel],
    member: &lanes::Member,
) -> Option<&'a providers::CatalogModel> {
    catalog.iter().find(|m| {
        m.id == member.id && (member.provider.is_empty() || m.provider_id == member.provider)
    })
}

/// How a member is named in trails and headers. The id alone is ambiguous the
/// moment two providers carry it, so a qualified member says which one.
fn member_label(member: &lanes::Member) -> String {
    if member.provider.is_empty() {
        member.id.clone()
    } else {
        format!("{}@{}", member.id, member.provider)
    }
}

/// Shared context every route handler receives.
///
/// `#[derive(Clone)]` asks the compiler to write the "make a copy" code for us.
/// axum hands each request its own copy of this, which is cheap here because a
/// `PathBuf` is just a path string. This is the *only* thing shared between
/// requests, and it never changes — which is exactly why no locking is needed.
#[derive(Clone)]
pub struct Engine {
    pub dir: PathBuf,
    /// Bearer token the lane endpoints require, loaded once at listener build.
    /// `None` (dev only) leaves the engine open, matching pre-token behaviour.
    pub secret: Option<String>,
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
        tools: body["tools"]
            .as_array()
            .map(|t| !t.is_empty())
            .unwrap_or(false),
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
/// positively tells us it can't. That rule applies at two depths: a model with
/// no catalog entry at all (`known == false`), and a catalogued model whose
/// entry never stated capabilities (`caps_known == false`) — a generic
/// provider's row says `vision: false` meaning "unstated", and treating that
/// as "cannot" would skip every direct-provider model on every tools request.
/// Likewise `context == 0` means *unknown*, not *zero-sized*, and must not be
/// used to reject anything.
fn can_serve(model: &providers::CatalogModel, needs: &Needs, known: bool) -> bool {
    if !known {
        return true; // nothing published; let the provider decide
    }
    if model.caps_known {
        if needs.vision && !model.vision {
            return false;
        }
        if needs.tools && !model.tools {
            return false;
        }
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

/// Is a 400 really about *this model's* limits rather than a bad request?
///
/// This distinction decides whether the lane continues or dies, so it is worth
/// dwelling on. Providers report two very different things with the same status
/// code:
///
///   "messages[0].role is required"          → the request is malformed.
///                                             Every model says the same. Stop.
///
///   "this model does not support tools"     → the request is fine; this
///                                             particular model can't. The next
///                                             one might. Keep walking.
///
/// The signal that separates them is the language of CAPABILITY — "not
/// supported", "unsupported", "does not accept" — versus the language of
/// VALIDATION — "required", "invalid", "must be". A capability complaint means
/// try the next model.
///
/// This matters more than it looks because `supported_parameters` in the
/// OpenRouter catalog is a UNION across every provider serving a model. A model
/// can be listed as supporting tools because *some* provider does, while the
/// endpoint you actually reach does not. Before this check existed, that
/// mismatch returned a 400, got classified as fatal, and killed the entire lane
/// — with working models sitting untouched behind it.
///
/// Bias is deliberately toward continuing. Wrongly continuing costs a few extra
/// attempts and still reports every failure. Wrongly stopping throws away models
/// that would have answered.
fn model_limitation(body: &str) -> Option<&'static str> {
    let text = body.to_lowercase();

    // A window too small is a ceiling, not a mistake. A later model may be
    // bigger.
    if text.contains("context")
        && (text.contains("length") || text.contains("token") || text.contains("window"))
    {
        return Some("prompt too long for this model");
    }

    let unsupported = text.contains("not support")
        || text.contains("unsupported")
        || text.contains("does not accept")
        || text.contains("no support")
        || text.contains("not available for");
    if !unsupported {
        return None;
    }

    if text.contains("tool") || text.contains("function") {
        Some("this model does not support tools")
    } else if text.contains("image") || text.contains("vision") || text.contains("modality") {
        Some("this model does not accept images")
    } else if text.contains("parameter")
        || text.contains("response_format")
        || text.contains("json_schema")
        || text.contains("structured")
    {
        Some("this model does not support a parameter in the request")
    } else {
        // Something is unsupported and we could not name it. Still this model's
        // limitation rather than a bad request, so keep going.
        Some("this model does not support something in the request")
    }
}

fn classify(status: StatusCode, body: &str) -> Verdict {
    let reason = |label: &str| format!("{label} ({})", status.as_u16());

    if status == StatusCode::TOO_MANY_REQUESTS {
        let text = body.to_lowercase();
        if text.contains("provider_name") && text.contains("null") {
            return Verdict::TryNext(reason("account-wide free-tier limit"));
        }
        return Verdict::TryNext(reason("rate limited"));
    }

    match status.as_u16() {
        // 400/422 mean "I don't understand this request" — usually fatal, but
        // only when the complaint is about the request rather than the model.
        400 | 422 => match model_limitation(body) {
            Some(why) => Verdict::TryNext(reason(why)),
            None => Verdict::Fatal(status, body.to_string()),
        },

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

// ============================================================================
// THE COMMIT POINT — idea #2 from the header
// ============================================================================
//
// `classify` judges refusals. Everything below judges ACCEPTANCES, which turn
// out to need judging too: a 200 whose body a chat client cannot render is a
// failure wearing a success status.
//
// The rule, for both streamed and unstreamed responses, is the same single
// question: did anything arrive that a baseline OpenAI client would show a
// person — assistant content, or a tool call? Reasoning tokens do not count;
// they are commentary about an answer, and several models will happily spend
// the whole token budget on them and send the answer they ran out of room for.

/// Does this event (a streaming chunk or a whole response body) carry
/// something a baseline client can use?
///
/// Checks `delta` and `message` on every choice, so the one function serves
/// both shapes. Content may legally be a string or an array of parts, and
/// tool calls may arrive under the modern `tool_calls` or the legacy
/// `function_call` — all four count.
fn usable_event(event: &Value) -> bool {
    let Some(choices) = event["choices"].as_array() else {
        return false;
    };
    choices.iter().any(|choice| {
        [&choice["delta"], &choice["message"]]
            .into_iter()
            .any(|node| {
                // Whitespace is not content. Several models open with a bare
                // "\n" delta before anything real; committing on it forwards a
                // stream that may never say a visible thing, when the point of
                // the gate is exactly to catch that and move on.
                node["content"]
                    .as_str()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
                    || node["content"]
                        .as_array()
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
                    || node["tool_calls"]
                        .as_array()
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
                    || node["function_call"].is_object()
            })
    })
}

/// Name what made an event usable: `tool:{name}`, or `content` with a short
/// preview — enough to tell a real answer from a tool call written as text.
fn commit_kind(event: &Value) -> String {
    let choices = event["choices"].as_array();
    for choice in choices.iter().flat_map(|c| c.iter()) {
        for node in [&choice["delta"], &choice["message"]] {
            if let Some(calls) = node["tool_calls"].as_array() {
                if let Some(name) = calls
                    .iter()
                    .find_map(|c| c["function"]["name"].as_str().filter(|n| !n.is_empty()))
                {
                    return format!("tool:{name}");
                }
                if !calls.is_empty() {
                    return "tool:?".into();
                }
            }
            if node["function_call"].is_object() {
                let name = node["function_call"]["name"].as_str().unwrap_or("?");
                return format!("tool:{name}");
            }
            if let Some(text) = node["content"].as_str() {
                if !text.trim().is_empty() {
                    let preview: String = text.trim().chars().take(60).collect();
                    return format!("content {preview:?}");
                }
            }
        }
    }
    "content".into()
}

/// Judge a complete (non-streaming) 200 body. `Ok` means forward it; `Err`
/// carries the trail note explaining why the next member gets a turn.
fn usable_body(text: &str) -> Result<(), String> {
    let Ok(body) = serde_json::from_str::<Value>(text) else {
        return Err("returned an unreadable 200 body".into());
    };
    // OpenRouter (and others) can put a whole error object inside a 200.
    if let Some(error) = body.get("error") {
        let message = error["message"]
            .as_str()
            .unwrap_or("unnamed provider error");
        return Err(format!("error in a 200 body: {message}"));
    }
    if usable_event(&body) {
        return Ok(());
    }
    // Nothing usable. Say WHY as precisely as the body allows — this string
    // is what someone reads in the trail when their lane "randomly" fell back.
    let finish = body["choices"][0]["finish_reason"]
        .as_str()
        .unwrap_or("unknown");
    let reasoned = body["choices"][0]["message"]["reasoning"]
        .as_str()
        .map(|s| !s.is_empty())
        .unwrap_or(false)
        || body["usage"]["completion_tokens_details"]["reasoning_tokens"]
            .as_u64()
            .unwrap_or(0)
            > 0;
    Err(if reasoned && finish == "length" {
        "spent the whole token budget reasoning, with no room left to answer".into()
    } else {
        format!("answered with no usable content (finish_reason: {finish})")
    })
}

/// What the scanner concluded from the stream so far.
enum Scan {
    /// Nothing decisive yet; keep reading.
    Wait,
    /// A usable delta arrived — stop judging, start forwarding.
    Commit,
    /// The stream is over (or carried an error) and nothing usable came.
    Die(String),
}

/// Watches an SSE stream for the first delta a client could render.
///
/// Server-sent events are lines: `data: {json}`, blank lines between events,
/// `: comment` keepalives, and a final `data: [DONE]`. Chunks off the wire cut
/// across those lines wherever they please, so this keeps the current
/// unfinished line between `feed` calls and judges only completed ones.
#[derive(Default)]
struct SseScan {
    line: Vec<u8>,
    /// The last `finish_reason` seen, for the post-mortem when nothing commits.
    finish: Option<String>,
    /// Whether reasoning tokens flowed — turns "empty answer" into "spent the
    /// budget thinking", which is the difference between confusion and a fix.
    saw_reasoning: bool,
    /// What the commit was: `content`, or `tool:{name}`. Diagnostic only —
    /// "the turn was served" and "the turn was served with a call to a tool
    /// the client never offered" look identical without it.
    committed_on: Option<String>,
}

impl SseScan {
    fn feed(&mut self, chunk: &[u8]) -> Scan {
        for &byte in chunk {
            if byte != b'\n' {
                self.line.push(byte);
                continue;
            }
            let verdict = self.line_done();
            self.line.clear();
            if !matches!(verdict, Scan::Wait) {
                return verdict;
            }
        }
        Scan::Wait
    }

    /// The stream closed; judge any unterminated final line. A provider that
    /// answers a streaming request with a bare JSON error body ends up here.
    fn flush(&mut self) -> Scan {
        if self.line.is_empty() {
            return Scan::Wait;
        }
        let verdict = self.line_done();
        self.line.clear();
        verdict
    }

    fn line_done(&mut self) -> Scan {
        let Ok(text) = std::str::from_utf8(&self.line) else {
            return Scan::Wait; // not ours to judge; forwarding stays byte-exact
        };
        let text = text.trim_end_matches('\r');
        if text.is_empty() || text.starts_with(':') {
            return Scan::Wait; // event separator, or a keepalive comment
        }
        let payload = match text.strip_prefix("data:") {
            Some(payload) => payload.trim(),
            // Not an SSE line at all. A bare JSON error body shows up this
            // way; judge it directly so the death note can quote it.
            None => text,
        };
        if payload == "[DONE]" {
            return Scan::Die(self.post_mortem());
        }
        let Ok(event) = serde_json::from_str::<Value>(payload) else {
            return Scan::Wait; // partial or foreign line — never fatal on its own
        };
        self.remember(&event);
        if let Some(error) = event.get("error") {
            let message = error["message"]
                .as_str()
                .unwrap_or("unnamed provider error");
            return Scan::Die(format!("error mid-stream: {message}"));
        }
        if usable_event(&event) {
            self.committed_on = Some(commit_kind(&event));
            Scan::Commit
        } else {
            Scan::Wait
        }
    }

    /// Note the facts a post-mortem wants: did it think, and how did it end.
    fn remember(&mut self, event: &Value) {
        if let Some(choices) = event["choices"].as_array() {
            for choice in choices {
                if let Some(reason) = choice["finish_reason"].as_str() {
                    self.finish = Some(reason.to_string());
                }
                if choice["delta"]["reasoning"]
                    .as_str()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
                {
                    self.saw_reasoning = true;
                }
            }
        }
    }

    fn post_mortem(&self) -> String {
        match (self.saw_reasoning, self.finish.as_deref()) {
            (true, Some("length")) => {
                "spent the whole token budget reasoning, with no room left to answer".into()
            }
            (_, Some(reason)) => {
                format!("stream ended with no usable content (finish_reason: {reason})")
            }
            _ => "stream ended with no usable content".into(),
        }
    }
}

/// How much prelude the gate will hold while waiting for a usable delta.
/// Megabytes of pure reasoning is far past anything legitimate; give the slot
/// to the next member instead of holding memory open forever.
const PRELUDE_CAP: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------- deadlines
//
// Every deadline here is on SILENCE, never on total time. A free model
// generating slowly is the normal case this app exists for and must never
// be cut off for it — but a socket delivering nothing at all is a dead
// connection wearing an open port, and without a deadline it would hold
// the whole lane hostage forever. Loopwatch and these share one definition
// of stuck: receiving no new information.

/// Time allowed to establish the connection. No TCP+TLS in ten seconds means
/// the host is down or the address is wrong — the next member's problem now.
const CONNECT_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// Longest gap between bytes on a STREAMING response. Streams carry a pulse
/// even while a model queues or thinks — deltas, or keep-alive comments
/// (OpenRouter sends them every few seconds while processing) — so two
/// minutes of true silence is not slowness, it is absence.
const STREAM_PATIENCE: std::time::Duration = std::time::Duration::from_secs(120);

/// Longest wait for a BLOCKING response, which legitimately says nothing at
/// all until the whole answer exists. Five minutes covers a slow model
/// writing a long answer; a connection that silent for longer is gone.
const BLOCKING_PATIENCE: std::time::Duration = std::time::Duration::from_secs(300);

/// The per-request HTTP client, with its idle deadline chosen by mode. The
/// read timeout resets on every byte received — it measures silence, so a
/// trickle keeps a connection alive and only a flatline ends one.
fn http_client(idle: std::time::Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_PATIENCE)
        .read_timeout(idle)
        .build()
}

/// The gate's verdict on a streaming response.
enum Gated {
    /// A usable delta arrived. `prelude` is every byte seen so far, verbatim
    /// (thinking included — clients that render it still get it, in one piece);
    /// `rest` is the still-live remainder of the stream; `on` names what
    /// committed it, for the log.
    Committed {
        on: String,
        prelude: Vec<Bytes>,
        rest: futures_util::stream::BoxStream<'static, Result<Bytes, reqwest::Error>>,
    },
    /// It ended, errored, or overflowed before anything usable. The note goes
    /// in the trail; the next member gets the request.
    Dead(String),
}

/// Hold a streaming 200 at the door until it proves usable.
async fn gate(resp: reqwest::Response) -> Gated {
    use futures_util::StreamExt;

    let mut rest = resp.bytes_stream().boxed();
    let mut scan = SseScan::default();
    let mut prelude: Vec<Bytes> = Vec::new();
    let mut held = 0usize;

    while let Some(item) = rest.next().await {
        let chunk = match item {
            Ok(chunk) => chunk,
            // A flatline gets its true name. The idle deadline firing inside
            // the gate means the stream started, never committed, and then
            // stopped carrying bytes altogether — distinct from a stream
            // that breaks loudly, and diagnosed differently.
            Err(err) if err.is_timeout() => {
                return Gated::Dead(
                    "went silent mid-stream before any usable content — connection presumed dead"
                        .into(),
                )
            }
            Err(err) => return Gated::Dead(format!("stream broke with no usable content: {err}")),
        };
        held += chunk.len();
        let verdict = scan.feed(&chunk);
        prelude.push(chunk);
        match verdict {
            Scan::Commit => {
                let on = scan.committed_on.take().unwrap_or_else(|| "content".into());
                return Gated::Committed { on, prelude, rest };
            }
            Scan::Die(why) => return Gated::Dead(why),
            Scan::Wait => {}
        }
        if held > PRELUDE_CAP {
            return Gated::Dead("streamed megabytes with no usable content".into());
        }
    }

    // Closed without [DONE]. A trailing unterminated line may still decide it.
    match scan.flush() {
        // Content in the very last gasp: the stream is finished, but the bytes
        // are all in `prelude`, so an empty remainder serves them fine.
        Scan::Commit => Gated::Committed {
            on: scan.committed_on.take().unwrap_or_else(|| "content".into()),
            prelude,
            rest: futures_util::stream::empty().boxed(),
        },
        Scan::Die(why) => Gated::Dead(why),
        Scan::Wait => Gated::Dead(scan.post_mortem()),
    }
}

/// One model that was considered before the one that answered.
///
/// Kept as a struct rather than raw JSON because it now has two audiences: the
/// error body when everything fails, and a response header when something
/// succeeds. Same facts, two renderings.
struct Attempt {
    model: String,
    outcome: String,
}

impl Attempt {
    fn skipped(model: &str, why: &str) -> Self {
        Attempt {
            model: model.into(),
            outcome: format!("skipped: {why}"),
        }
    }
    fn failed(model: &str, why: &str) -> Self {
        Attempt {
            model: model.into(),
            outcome: format!("failed: {why}"),
        }
    }
}

/// Render the trail for a header.
///
/// Header values must be printable ASCII on a single line, and provider error
/// text is arbitrary — it can carry newlines, quotes, or non-Latin characters.
/// Anything outside that range becomes `?`, and the whole thing is capped so a
/// long lane cannot produce a header some proxy refuses to forward.
fn trail_header(tried: &[Attempt]) -> String {
    let mut out = String::new();
    for attempt in tried {
        if !out.is_empty() {
            out.push_str("; ");
        }
        for ch in format!("{}={}", attempt.model, attempt.outcome).chars() {
            out.push(if ch.is_ascii_graphic() || ch == ' ' {
                ch
            } else {
                '?'
            });
        }
        if out.len() > 700 {
            out.push_str(" …");
            break;
        }
    }
    out
}

// ============================================================================
// MEMBER DIALS
// ============================================================================
//
// A member is not just a model id — it can carry its own request settings,
// fixed when the lane was designed: temperature, penalties, a token ceiling.
// The engine applies them here, per member, for the same reason it swaps the
// model id: the client addressed a lane, and the lane knows how each of its
// members should be driven.
//
// The subtle requirement is the same one reasoning suppression has: a dial
// set for THIS member must never leak into the request for the NEXT one.
// Every knob the client did not set is removed again before the next attempt,
// and every knob the client did set is put back.

/// Every request knob a member may fix. One list serves both jobs: capturing
/// what the client asked for, and restoring it between members.
const MEMBER_KNOBS: [&str; 6] = [
    "temperature",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "repetition_penalty",
    "max_tokens",
];

/// The member's value for one knob, as JSON, if the lane set one.
fn knob_value(params: &lanes::MemberParams, knob: &str) -> Option<Value> {
    match knob {
        "temperature" => params.temperature.map(|v| json!(v)),
        "top_p" => params.top_p.map(|v| json!(v)),
        "frequency_penalty" => params.frequency_penalty.map(|v| json!(v)),
        "presence_penalty" => params.presence_penalty.map(|v| json!(v)),
        "repetition_penalty" => params.repetition_penalty.map(|v| json!(v)),
        "max_tokens" => params.max_tokens.map(|v| json!(v)),
        _ => None,
    }
}

/// Shape the request for one member: the member's dial wins, the client's own
/// value is the fallback, and a knob neither of them set is absent — never
/// defaulted, because absence is itself an instruction ("provider decides").
///
/// `client` is the client's original values, captured once before the walk
/// begins; passing them in on every call is what makes the walk stateless.
fn apply_member_params(
    body: &mut Value,
    params: &lanes::MemberParams,
    client: &[(String, Option<Value>)],
) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    for (knob, original) in client {
        match knob_value(params, knob).or_else(|| original.clone()) {
            Some(value) => {
                map.insert(knob.clone(), value);
            }
            None => {
                map.remove(knob.as_str());
            }
        }
    }
}

/// The set dials as one log-friendly line: `temperature=0.2 max_tokens=400`.
fn dials_summary(params: &lanes::MemberParams) -> String {
    MEMBER_KNOBS
        .iter()
        .filter_map(|knob| knob_value(params, knob).map(|v| format!("{knob}={v}")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The headers every successful lane response carries, whatever its body.
fn success_headers(
    slug: &str,
    served: &str,
    tried: &[Attempt],
    unstuck: Option<&loopwatch::Broke>,
) -> axum::http::response::Builder {
    let mut out = Response::builder()
        .status(StatusCode::OK)
        .header("x-visualllm-lane", slug)
        .header("x-visualllm-served-by", served)
        .header("x-visualllm-passed-over", tried.len());
    // THE SILENT-SKIP FIX: every response says what it stepped over and why.
    // A healthy lane reports `passed-over: 0`; anything else is a question
    // worth asking, and the trail is the answer.
    if !tried.is_empty() {
        out = out.header("x-visualllm-trail", trail_header(tried));
    }
    // A repaired conversation is announced, never done quietly. Tool names
    // come from the client's own request, so they get the same ASCII scrub
    // the trail does.
    if let Some(broke) = unstuck {
        let tool: String = broke
            .tool
            .chars()
            .map(|c| if c.is_ascii_graphic() { c } else { '?' })
            .take(60)
            .collect();
        out = out.header(
            "x-visualllm-unstuck",
            format!(
                "{} tool={} times={} collapsed={}",
                broke.kind, tool, broke.times, broke.collapsed
            ),
        );
    }
    out
}

/// Build an error response that says what was attempted.
///
/// A lane that goes quiet is maddening to debug, so every failure carries the
/// list of models tried and why each one was passed over. Explicable beats
/// terse.
fn error(status: StatusCode, message: String, kind: &str, tried: Vec<Attempt>) -> Response {
    let tried: Vec<Value> = tried
        .iter()
        .map(|a| json!({ "model": a.model, "outcome": a.outcome }))
        .collect();
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
                .filter_map(|member| find_model(&catalog, member))
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
        "service": "VisualLLM",
        "ok": true,
        "lanes": lanes::load(&engine.dir).len(),
        "models_cached": providers::cache_read(&engine.dir).len(),
    }))
}

/// `GET /activity?since=<unix>` — the live lane feed the renderer tails.
///
/// The renderer holds no network access of its own, so it reaches this through
/// the `activity_read` Tauri command, which shares this reader.
async fn activity(
    State(engine): State<Engine>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let since = params
        .get("since")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Json(json!({ "activity": activity_read(&engine.dir, since) }))
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
    let meta = providers::cache_meta_read(&engine.dir);
    if meta.stale {
        eprintln!(
            "engine: serving from stale catalog cache retained at {}",
            meta.retained_at
        );
    }
    let configured = providers::load(&engine.dir);
    let needs = inspect(&body);
    let streaming = body["stream"].as_bool().unwrap_or(false);

    // A one-token ping is not a request for an answer, and judging it for
    // content would fail every health probe ever written. Tiny budgets skip
    // the commit gate and stream straight through, as before.
    let budget = body["max_tokens"]
        .as_u64()
        .or_else(|| body["max_completion_tokens"].as_u64());
    let gated = !budget.is_some_and(|b| b < 16);

    // What the client itself said about reasoning and about every dial a
    // member may override — captured once, so one member's settings can be
    // unwound before the next member's request is built.
    let client_reasoning = body.get("reasoning").cloned();
    let client_knobs: Vec<(String, Option<Value>)> = MEMBER_KNOBS
        .iter()
        .map(|knob| (knob.to_string(), body.get(*knob).cloned()))
        .collect();

    // One line per request, one per attempt, to stderr — which `tauri dev`
    // already collects. Cheap eyes for a process that had none: three separate
    // wrong guesses about live behaviour were made from TCP state alone the
    // night this went in.
    let tool_count = body["tools"].as_array().map(|t| t.len()).unwrap_or(0) as u64;
    eprintln!(
        "engine: {} <- {} request: {} tools, budget {}, needs[vision={} tools={} ~{}tok], no-think={}",
        lane.slug,
        if streaming { "streaming" } else { "blocking" },
        tool_count,
        budget.map(|b| b.to_string()).unwrap_or_else(|| "unset".into()),
        needs.vision,
        needs.tools,
        needs.tokens,
        lane.suppress_reasoning,
    );

    // Loopwatch, before any member is contacted: the conversation the client
    // sent may already show an agent going in circles, and forwarding a loop
    // faithfully is not a service. Opt-in per lane, and never silent — the
    // repair is logged here and announced on the response.
    let mut unstuck: Option<loopwatch::Broke> = None;
    if lane.unstick {
        if let Some(messages) = body["messages"].as_array() {
            if let Some((repaired, broke)) = loopwatch::break_loop(messages, loopwatch::THRESHOLD) {
                eprintln!(
                    "engine: {} loopwatch: {} on {} ({}x, {} pairs collapsed)",
                    lane.slug, broke.kind, broke.tool, broke.times, broke.collapsed
                );
                // A loop lives in the client's conversation, generated across
                // earlier turns — usually by the primary, which is the best
                // attribution available and is labelled as such.
                let suspect = lane
                    .members
                    .first()
                    .map(member_label)
                    .unwrap_or_else(|| "(empty lane)".into());
                // The counts alone say a loop happened; the futile species'
                // quote says what the model kept ignoring — which is the
                // diagnosis. It was already shown to the model in the note;
                // the record shows the same thing to the user. Appended after
                // the summary so the "loop (…)" prefix keeps naming the kind.
                let mut receipts = format!(
                    "loop ({}): `{}` — {} calls, {} redundant pairs collapsed",
                    broke.kind, broke.tool, broke.times, broke.collapsed
                );
                if let Some(excerpt) = &broke.excerpt {
                    receipts.push_str(&format!(
                        "; every call returned the identical result: {excerpt}"
                    ));
                }
                note_incident(&engine.dir, lane, &suspect, &receipts, tool_count);
                body["messages"] = Value::Array(repaired);
                unstuck = Some(broke);
            }
        }
    }

    // Streaming has a pulse to check; blocking earns more patience because
    // its silence is legitimate right up until the answer lands whole.
    let idle = if streaming {
        STREAM_PATIENCE
    } else {
        BLOCKING_PATIENCE
    };
    let client = match http_client(idle) {
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
    let mut tried: Vec<Attempt> = Vec::new();

    // ---- 3. Walk the lane in order ----------------------------------------

    for member in &lane.members {
        let label = member_label(member);

        // A parked member keeps its place and dials but is never contacted.
        // The skip is named, so the trail says "parked" rather than a
        // capability miss — the difference between "can't" and "asked not to".
        if member.disabled {
            eprintln!("engine: {}   skip {label}: parked by the lane", lane.slug);
            tried.push(Attempt::skipped(&label, "parked"));
            continue;
        }

        let entry = find_model(&catalog, member);
        let known = entry.is_some();

        // If the catalog has nothing on this model we still need *something* to
        // pass to `can_serve`, so use an empty record. Combined with `known`,
        // that means "no published limits" rather than "no capabilities".
        let blank = providers::CatalogModel::default();
        let model = entry.unwrap_or(&blank);

        if !can_serve(model, &needs, known) {
            eprintln!(
                "engine: {}   skip {label}: cannot serve this request \
                 (needs vision={} tools={} ~{}tok; catalog lists vision={} tools={} context={})",
                lane.slug,
                needs.vision,
                needs.tools,
                needs.tokens,
                model.vision,
                model.tools,
                model.context
            );
            // A capability skip is the lane working as designed — the fast
            // primary being passed over for a request it can't serve is the
            // entire point of the app. It is NOT an incident: recording it as
            // one trained users to ignore the bell, which then missed real
            // failures. The skip is still fully visible where it belongs — in
            // the `x-visualllm-trail` header and on the lane's activity line,
            // which the renderer derives from the trail, not from incidents.
            tried.push(Attempt::skipped(&label, "cannot serve this request"));
            continue; // never contacted — this is idea #1 from the header
        }

        // Whose key and base URL to use. The member names its provider — that
        // is the identity now. A pre-provider member falls back to the catalog
        // entry's provider, and only a member the catalog has never heard of
        // lands on the first configured provider: a guess, and labelled as one
        // in the trail rather than made silently.
        let named = configured
            .iter()
            .find(|p| !member.provider.is_empty() && p.id == member.provider);
        let provider = named
            .or_else(|| {
                configured
                    .iter()
                    .find(|p| known && p.id == model.provider_id)
            })
            .or_else(|| configured.first());

        let Some(provider) = provider else {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "no providers configured".into(),
                "no_provider",
                tried,
            );
        };

        let guessed = named.is_none() && !known;

        // The client addressed a *lane*; the provider needs a real model id.
        // This one line is the whole translation.
        body["model"] = json!(member.id);

        // Then the member's own dials. Note the commit gate's tiny-budget
        // bypass was decided from the CLIENT's budget on purpose: a member
        // whose settings cap the answer short is a design choice the gate
        // still protects, while a client probing with one token is not asking
        // for an answer at all.
        apply_member_params(&mut body, &member.params, &client_knobs);
        if !member.params.is_empty() {
            eprintln!(
                "engine: {}   {label} dials: {}",
                lane.slug,
                dials_summary(&member.params)
            );
        }

        // The lane's word on thinking. Only OpenRouter gets the knob — it
        // normalises `reasoning` across models and ignores it where it cannot
        // apply, while a direct provider would reject the unknown parameter
        // and read as a bad request. Everyone else gets the client's own
        // wishes back, untouched.
        if lane.suppress_reasoning && provider.kind == "openrouter" {
            body["reasoning"] = json!({ "enabled": false });
        } else if let Some(wanted) = &client_reasoning {
            body["reasoning"] = wanted.clone();
        } else if let Some(map) = body.as_object_mut() {
            map.remove("reasoning");
        }

        let base = provider.base_url.trim_end_matches('/');
        let request =
            providers::authorise_public(client.post(format!("{base}/chat/completions")), provider);
        // The canvas shows "trying X…" from this moment.
        note_activity(&engine.dir, &lane.slug, &label, "trying", "");
        let request = match headers.get("accept") {
            Some(accept) => request.header("accept", accept),
            None => request,
        };

        // `.await` is the pause point: this request's work stops here, other
        // requests run, and we resume when the provider responds.
        match request.json(&body).send().await {
            // ---- it answered (provisionally — see idea #2) ----
            Ok(resp) if resp.status().is_success() => {
                // A guessed provider must say so where it can be seen. It may
                // well have answered — with the wrong key billed for it.
                let served = if guessed {
                    format!("{label} (provider guessed: {})", provider.id)
                } else {
                    label.clone()
                };
                let upstream_type = resp.headers().get("content-type").cloned();

                if streaming && gated {
                    // Hold the stream at the door until the first delta a
                    // client could render. Once committed, the buffered
                    // prelude (thinking included) and the live remainder are
                    // piped through as they arrive — the full answer still
                    // never exists in one piece here.
                    match gate(resp).await {
                        Gated::Committed { on, prelude, rest } => {
                            eprintln!(
                                "engine: {}   {label} committed on {on} ({} passed over)",
                                lane.slug,
                                tried.len()
                            );
                            note_activity(
                                &engine.dir,
                                &lane.slug,
                                &served,
                                "answered",
                                &format!("committed on {on}; passed over {}", tried.len()),
                            );
                            let out =
                                success_headers(&lane.slug, &served, &tried, unstuck.as_ref())
                                    .header("content-type", "text/event-stream");
                            let replay = futures_util::stream::iter(
                                prelude.into_iter().map(Ok::<Bytes, reqwest::Error>),
                            );
                            let joined = futures_util::StreamExt::chain(replay, rest);
                            let stream =
                                futures_util::TryStreamExt::map_err(joined, std::io::Error::other);
                            // Build the response but handle any body-construction error
                            // instead of unwrapping, so the engine never panics while
                            // sending a streaming reply.
                            match out.body(Body::from_stream(stream)) {
                                Ok(response) => return response.into_response(),
                                Err(err) => {
                                    let why = format!("failed to build streaming response: {err}");
                                    eprintln!("engine: {} respond error: {why}", lane.slug);
                                    note_incident(&engine.dir, lane, &label, &why, tool_count);
                                    tried.push(Attempt::failed(&label, &why));
                                    return error(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        why,
                                        "engine_error",
                                        tried,
                                    );
                                }
                            }
                        }
                        Gated::Dead(why) => {
                            eprintln!("engine: {}   {label} died in-stream: {why}", lane.slug);
                            note_activity(&engine.dir, &lane.slug, &label, "failed", &why);
                            note_incident(&engine.dir, lane, &label, &why, tool_count);
                            tried.push(Attempt::failed(&label, &why));
                            continue;
                        }
                    }
                }

                if !streaming && gated {
                    // The unstreamed twin: read the whole body — the client
                    // asked for it in one piece anyway — and judge it before
                    // passing it on.
                    let text = match resp.text().await {
                        Ok(t) => t,
                        Err(err) => {
                            let why = format!("failed to read upstream body: {err}");
                            eprintln!("engine: {}   {label} read error: {why}", lane.slug);
                            note_incident(&engine.dir, lane, &label, &why, tool_count);
                            tried.push(Attempt::failed(&label, &why));
                            continue;
                        }
                    };

                    match usable_body(&text) {
                        Ok(()) => {
                            eprintln!(
                                "engine: {}   {label} served a blocking body ({} passed over)",
                                lane.slug,
                                tried.len()
                            );
                            note_activity(
                                &engine.dir,
                                &lane.slug,
                                &served,
                                "answered",
                                &format!("served a blocking body; passed over {}", tried.len()),
                            );
                            let mut out =
                                success_headers(&lane.slug, &served, &tried, unstuck.as_ref());
                            out = match upstream_type {
                                Some(kind) => out.header("content-type", kind),
                                None => out.header("content-type", "application/json"),
                            };
                            match out.body(Body::from(text)) {
                                Ok(response) => return response.into_response(),
                                Err(err) => {
                                    let why = format!("failed to build blocking response: {err}");
                                    eprintln!("engine: {} respond error: {why}", lane.slug);
                                    note_incident(&engine.dir, lane, &label, &why, tool_count);
                                    tried.push(Attempt::failed(&label, &why));
                                    return error(
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        why,
                                        "engine_error",
                                        tried,
                                    );
                                }
                            }
                        }
                        Err(why) => {
                            eprintln!("engine: {}   {label} unusable body: {why}", lane.slug);
                            note_activity(&engine.dir, &lane.slug, &label, "failed", &why);
                            note_incident(&engine.dir, lane, &label, &why, tool_count);
                            tried.push(Attempt::failed(&label, &why));
                            continue;
                        }
                    }
                }

                // Ungated (a tiny-budget probe): pipe straight through, the
                // pre-commit-gate behaviour.
                note_activity(
                    &engine.dir,
                    &lane.slug,
                    &served,
                    "answered",
                    &format!("ungated probe; passed over {}", tried.len()),
                );
                let mut out = success_headers(&lane.slug, &served, &tried, unstuck.as_ref());
                if streaming {
                    out = out.header("content-type", "text/event-stream");
                } else if let Some(kind) = upstream_type {
                    out = out.header("content-type", kind);
                }
                let stream =
                    futures_util::TryStreamExt::map_err(resp.bytes_stream(), std::io::Error::other);
                match out.body(Body::from_stream(stream)) {
                    Ok(response) => return response.into_response(),
                    Err(err) => {
                        let why = format!("failed to build response stream: {err}");
                        eprintln!("engine: {} respond error: {why}", lane.slug);
                        note_incident(&engine.dir, lane, &label, &why, tool_count);
                        tried.push(Attempt::failed(&label, &why));
                        return error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            why,
                            "engine_error",
                            tried,
                        );
                    }
                }
            }

            // ---- it replied, but with an error ----
            Ok(resp) => {
                let status =
                    StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                let text = resp.text().await.unwrap_or_default();

                match classify(status, &text) {
                    Verdict::Fatal(status, message) => {
                        eprintln!(
                            "engine: {}   {label} fatal {}: request rejected",
                            lane.slug,
                            status.as_u16()
                        );
                        // Explicitly NOT the model's fault, and the record
                        // says so: every model would reject this request the
                        // same way. Being sure cuts both directions.
                        let snippet: String = message.chars().take(300).collect();
                        note_incident(
                            &engine.dir,
                            lane,
                            &label,
                            &format!(
                                "request rejected by the provider ({}): {snippet}",
                                status.as_u16()
                            ),
                            tool_count,
                        );
                        tried.push(Attempt::failed(&label, "request rejected"));
                        return error(status, message, "upstream_rejected", tried);
                    }
                    Verdict::TryNext(why) => {
                        eprintln!("engine: {}   {label} failed: {why}", lane.slug);
                        note_activity(&engine.dir, &lane.slug, &label, "failed", &why);
                        note_incident(&engine.dir, lane, &label, &why, tool_count);
                        tried.push(Attempt::failed(&label, &why));
                    }
                }
            }

            // ---- we never reached it, or it went dead before answering ----
            Err(err) => {
                // Order matters: a connect timeout is both `is_connect` and
                // `is_timeout`, and "could not connect" is the truer name.
                // The idle deadline firing HERE means the provider accepted
                // the connection and then never produced a response at all.
                let why = if err.is_connect() {
                    "could not connect".to_string()
                } else if err.is_timeout() {
                    format!(
                        "went silent: no bytes for {}s — connection presumed dead",
                        idle.as_secs()
                    )
                } else {
                    err.to_string()
                };
                eprintln!("engine: {}   {label} unreachable: {why}", lane.slug);
                note_activity(&engine.dir, &lane.slug, &label, "failed", &why);
                note_incident(&engine.dir, lane, &label, &why, tool_count);
                tried.push(Attempt::failed(&label, &why));
            }
        }
    }

    eprintln!(
        "engine: {} exhausted — every member skipped or failed",
        lane.slug
    );
    note_activity(
        &engine.dir,
        &lane.slug,
        "",
        "exhausted",
        "every model in the lane was skipped or failed",
    );

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
///
/// The lane routes are guarded by the gateway token; the read-only surface
/// (`/health`, `/activity`, `/v1/models`) stays open so a user can probe the
/// engine without a credential.
pub fn router(dir: PathBuf, secret: Option<String>) -> Router {
    let engine = Engine { dir, secret };
    Router::new()
        .route("/lane/{slug}/v1/chat/completions", post(chat))
        // Some clients append `/v1/models` to whatever base URL you give them.
        // If someone configures a single lane as their base URL, this stops
        // discovery 404-ing on them.
        .route("/lane/{slug}/v1/models", get(models))
        .route_layer(axum::middleware::from_fn_with_state(
            engine.clone(),
            require_token,
        ))
        .route("/health", get(health))
        .route("/activity", get(activity))
        .route("/v1/models", get(models))
        .with_state(engine)
}

/// Enforce the gateway bearer token on the lane endpoints.
///
/// The lanes can spend the user's money, and they are reachable by any local
/// process (or a DNS-rebinding website), so they demand `Authorization:
/// Bearer <token>` matching the `secret` file. Compare in constant-ish time to
/// avoid a trivial timing oracle over the loopback; the comparison is short,
/// so the difference is a footgun only if someone measures over the network.
async fn require_token(
    axum::extract::State(engine): axum::extract::State<Engine>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::response::Response> {
    let authorised = match engine.secret.as_deref() {
        None => true,
        Some(secret) => request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|presented| {
                let a = presented.as_bytes();
                let b = secret.as_bytes();
                if a.len() != b.len() {
                    return false;
                }
                let mut diff = 0u8;
                for (x, y) in a.iter().zip(b.iter()) {
                    diff |= x ^ y;
                }
                diff == 0
            })
            .unwrap_or(false),
    };
    if authorised {
        Ok(next.run(request).await)
    } else {
        Err((
            axum::http::StatusCode::UNAUTHORIZED,
            "missing or invalid gateway token — see Engine settings in VisualLLM",
        )
            .into_response())
    }
}

/// Start listening. Called once at startup from `main.rs`.
///
/// Bound to `127.0.0.1` deliberately: that address is reachable only from this
/// machine. Using `0.0.0.0` would expose an unauthenticated proxy holding the
/// user's API keys to the entire local network.
pub async fn serve(
    dir: PathBuf,
    port: u16,
    secret: Option<String>,
    mut ports: watch::Receiver<u16>,
) -> Result<(), String> {
    let dir = Arc::new(dir);
    let secret = Arc::new(secret);
    let listener = bind(port).await?;
    let (mut stop_tx, stop_rx) = oneshot::channel();
    let mut active = spawn(listener, Arc::clone(&dir), Arc::clone(&secret), stop_rx);
    let mut current = port;

    loop {
        tokio::select! {
            result = &mut active => {
                return result
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string());
            }
            changed = ports.changed() => {
                if changed.is_err() {
                    let _ = stop_tx.send(());
                    return active.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string());
                }
                let next = *ports.borrow_and_update();
                if next == current {
                    continue;
                }

                // Bind first. A busy new port leaves the current listener live.
                let listener = match bind(next).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        eprintln!("engine: keeping 127.0.0.1:{current}; could not switch to 127.0.0.1:{next}: {error}");
                        continue;
                    }
                };
                let _ = stop_tx.send(());
                let _ = active.await;
                let (next_stop_tx, next_stop_rx) = oneshot::channel();
                stop_tx = next_stop_tx;
                active = spawn(listener, Arc::clone(&dir), Arc::clone(&secret), next_stop_rx);
                current = next;
                eprintln!("engine: now listening on 127.0.0.1:{current}");
            }
        }
    }
}

async fn bind(port: u16) -> Result<tokio::net::TcpListener, String> {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("could not listen on 127.0.0.1:{port} — {e}"))
}

fn spawn(
    listener: tokio::net::TcpListener,
    dir: Arc<PathBuf>,
    secret: Arc<Option<String>>,
    stop: oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<Result<(), std::io::Error>> {
    tokio::spawn(async move {
        axum::serve(listener, router((*dir).clone(), (*secret).clone()))
            .with_graceful_shutdown(async {
                let _ = stop.await;
            })
            .await
    })
}

// ============================================================================
// TESTS
// ============================================================================
//
// `classify` decides whether a lane keeps walking or dies, so it is the one
// function here worth pinning down with examples. Each case below is a real
// error string shape seen from a provider, not an invented one.
//
// Run them with:  cargo test

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::post;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn verdict(status: u16, body: &str) -> Verdict {
        classify(StatusCode::from_u16(status).unwrap(), body)
    }

    fn continues(status: u16, body: &str) -> bool {
        matches!(verdict(status, body), Verdict::TryNext(_))
    }

    #[test]
    fn a_malformed_request_stops_the_lane() {
        // Every model would reject this identically. Walking the rest of the
        // lane only burns the user's rate limit to collect the same error.
        assert!(!continues(400, "messages[0].role is a required property"));
        assert!(!continues(
            400,
            "invalid value for 'temperature': must be <= 2"
        ));
    }

    #[test]
    fn a_capability_gap_keeps_walking() {
        // The bug this test exists for: `supported_parameters` in the catalog is
        // a union across every provider serving a model, so a model can be
        // listed as supporting tools while the endpoint reached does not. That
        // returned a 400, was classified fatal, and killed the whole lane with
        // working models untouched behind it.
        assert!(continues(400, "This model does not support tools"));
        assert!(continues(
            400,
            "function calling is not supported by this model"
        ));
        assert!(continues(400, "Unsupported parameter: 'response_format'"));
        assert!(continues(400, "image input is not supported"));
    }

    #[test]
    fn an_overflowing_prompt_keeps_walking() {
        // This model's ceiling, not a bad request. A later one may be bigger.
        assert!(continues(
            400,
            "This model's maximum context length is 8192 tokens, however you requested 9000"
        ));
    }

    #[test]
    fn provider_trouble_always_keeps_walking() {
        for status in [401, 402, 403, 404, 408, 429, 500, 502, 503] {
            assert!(continues(status, ""), "{status} should try the next model");
        }
    }

    #[test]
    fn account_wide_rate_limits_are_named_from_the_body() {
        match verdict(
            429,
            r#"{"error":{"provider_name":null,"message":"free tier exhausted"}}"#,
        ) {
            Verdict::TryNext(note) => assert!(note.contains("account-wide free-tier limit")),
            Verdict::Fatal(_, _) => panic!("rate limits should continue"),
        }
        match verdict(429, "provider temporarily throttled") {
            Verdict::TryNext(note) => assert!(note.contains("rate limited")),
            Verdict::Fatal(_, _) => panic!("rate limits should continue"),
        }
    }

    #[test]
    fn the_bias_is_toward_continuing() {
        // Wrongly continuing costs a few attempts and still reports every
        // failure. Wrongly stopping throws away models that would have
        // answered. So an unrecognised "not supported" keeps going.
        assert!(continues(400, "widgets are not supported here"));
    }

    #[test]
    fn a_header_trail_survives_hostile_provider_text() {
        // Provider error text is arbitrary; header values must be printable
        // ASCII on one line or the response is unsendable.
        let tried = vec![
            Attempt::skipped("openai/gpt-4o", "cannot serve this request"),
            Attempt::failed("meta/llama", "rate\nlimited — 429 ✗"),
        ];
        let header = trail_header(&tried);
        assert!(!header.contains('\n'));
        assert!(header.is_ascii());
        assert!(header.contains("openai/gpt-4o=skipped"));
    }

    #[test]
    fn an_unknown_model_is_never_skipped() {
        // A generic provider publishes ids and nothing else. Treating silence as
        // "cannot" would make every such provider useless.
        let blank = providers::CatalogModel::default();
        let needs = Needs {
            vision: true,
            tools: true,
            tokens: 999_999,
        };
        assert!(can_serve(&blank, &needs, false));
    }

    #[test]
    fn a_known_model_is_skipped_on_what_it_lacks() {
        // `caps_known` is what makes the false a fact rather than a default.
        let text_only = providers::CatalogModel {
            context: 8192,
            caps_known: true,
            ..Default::default()
        };
        let needs = Needs {
            vision: true,
            tools: false,
            tokens: 10,
        };
        assert!(!can_serve(&text_only, &needs, true));
    }

    #[test]
    fn unstated_capabilities_never_skip() {
        // A generic provider's catalog entry says `vision: false, tools: false`
        // because it said NOTHING — the fields defaulted. Treating that as
        // "cannot" would skip every direct-provider model on every tools
        // request, silently and forever. The entry is in the catalog (known),
        // but its capabilities are not, so the request goes through.
        let unstated = providers::CatalogModel {
            context: 8192,
            ..Default::default()
        };
        let needs = Needs {
            vision: true,
            tools: true,
            tokens: 10,
        };
        assert!(can_serve(&unstated, &needs, true));

        // Context is a separate fact: when published it still applies, whatever
        // the capability fields do or don't say.
        let too_long = Needs {
            vision: false,
            tools: false,
            tokens: 9_000,
        };
        assert!(!can_serve(&unstated, &too_long, true));
    }

    #[test]
    fn an_unknown_context_never_rejects() {
        // `context: 0` means unpublished, not zero-sized.
        let unsized_model = providers::CatalogModel {
            tools: true,
            caps_known: true,
            ..Default::default()
        };
        let needs = Needs {
            vision: false,
            tools: true,
            tokens: 500_000,
        };
        assert!(can_serve(&unsized_model, &needs, true));
    }

    // ------------------------------------------------------ the commit point

    /// Feed a whole transcript through the scanner, ending it the way
    /// `gate()` does: an EOF that decided nothing is a death, with the
    /// scanner's post-mortem as the note.
    fn scan(text: &str) -> Scan {
        let mut scanner = SseScan::default();
        let fed = scanner.feed(text.as_bytes());
        if !matches!(fed, Scan::Wait) {
            return fed;
        }
        match scanner.flush() {
            Scan::Wait => Scan::Die(scanner.post_mortem()),
            decisive => decisive,
        }
    }

    fn commits(text: &str) -> bool {
        matches!(scan(text), Scan::Commit)
    }

    fn dies_with(text: &str, needle: &str) -> bool {
        match scan(text) {
            Scan::Die(why) => why.contains(needle),
            _ => false,
        }
    }

    #[test]
    fn reasoning_only_until_done_dies_and_says_why() {
        // The Copilot incident, in miniature: every delta is reasoning, the
        // budget runs out, and the visible answer never starts. The death
        // note must name the cause, because "no response" already didn't.
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"\",\"reasoning\":\"hmm\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"\",\"reasoning\":\" more\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        assert!(dies_with(stream, "reasoning"));
    }

    #[test]
    fn the_first_content_token_commits() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"\",\"reasoning\":\"hmm\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"H\"}}]}\n\n",
        );
        assert!(commits(stream));
    }

    #[test]
    fn whitespace_is_not_content() {
        // Several models open with a bare "\n" delta. A stream that never
        // says anything visible must die at the gate, not be forwarded as an
        // "answer" a chat client renders as nothing.
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"\\n\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        assert!(dies_with(stream, "no usable content"));

        // But whitespace before a real token delays nothing important.
        let healthy = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"\\n\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
        );
        assert!(commits(healthy));

        // Same rule for a whole body.
        assert!(usable_body(r#"{"choices":[{"message":{"content":"\n \n"}}]}"#).is_err());
    }

    #[test]
    fn a_tool_call_delta_commits() {
        // Agent mode lives on this: a turn can be all tool calls, no prose.
        let stream = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"f\"}}]}}]}\n\n";
        assert!(commits(stream));
        // The legacy spelling counts too.
        let legacy = "data: {\"choices\":[{\"delta\":{\"function_call\":{\"name\":\"f\"}}}]}\n\n";
        assert!(commits(legacy));
    }

    #[test]
    fn a_mid_stream_error_event_dies_with_its_message() {
        // OpenRouter delivers rate limits INSIDE a 200 stream. Before the
        // gate, this reached the client as an empty answer.
        let stream = "data: {\"error\":{\"message\":\"Rate limit exceeded: free-models-per-day\",\"code\":429}}\n";
        assert!(dies_with(stream, "free-models-per-day"));
    }

    #[test]
    fn chunks_split_anywhere_still_parse() {
        // Network chunks cut across SSE lines wherever they please. The
        // scanner keeps the partial line between feeds; the verdict must not
        // depend on where the cuts fall.
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning\":\"x\"}}]}\r\n\r\n",
            ": OPENROUTER PROCESSING\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\r\n\r\n",
        );
        for size in [1, 3, 7, 20] {
            let mut scanner = SseScan::default();
            let mut verdict = Scan::Wait;
            for piece in stream.as_bytes().chunks(size) {
                verdict = scanner.feed(piece);
                if !matches!(verdict, Scan::Wait) {
                    break;
                }
            }
            assert!(matches!(verdict, Scan::Commit), "chunk size {size}");
        }
    }

    #[test]
    fn an_empty_stream_dies_quietly_but_explicably() {
        assert!(dies_with("data: [DONE]\n\n", "no usable content"));
        assert!(dies_with("", "no usable content"));
    }

    #[test]
    fn a_bare_json_error_body_on_a_streaming_request_dies() {
        // Some providers answer a streaming request with a plain JSON error
        // and no SSE framing at all. The unterminated line is judged at EOF.
        let body = "{\"error\":{\"message\":\"upstream fell over\"}}";
        assert!(dies_with(body, "upstream fell over"));
    }

    #[test]
    fn a_200_body_with_no_content_is_unusable() {
        let body = r#"{"choices":[{"message":{"content":"","reasoning":"so much thinking"},"finish_reason":"length"}]}"#;
        let why = usable_body(body).unwrap_err();
        assert!(why.contains("reasoning"), "note was: {why}");
    }

    #[test]
    fn a_200_body_with_an_error_object_is_unusable() {
        let body = r#"{"error":{"message":"quota exhausted","code":429}}"#;
        assert!(usable_body(body).unwrap_err().contains("quota exhausted"));
    }

    #[test]
    fn real_answers_are_usable() {
        assert!(usable_body(r#"{"choices":[{"message":{"content":"Hello."}}]}"#).is_ok());
        assert!(usable_body(
            r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"1","function":{"name":"f","arguments":"{}"}}]}}]}"#
        )
        .is_ok());
        // Content as an array of parts is legal OpenAI, and counts.
        assert!(usable_body(
            r#"{"choices":[{"message":{"content":[{"type":"text","text":"hi"}]}}]}"#
        )
        .is_ok());
    }

    #[test]
    fn member_dials_never_leak_to_the_next_member() {
        // The walk mutates one shared body. Member A's dials must be gone —
        // and the client's own values back — before member B's request goes
        // out, or a fallback quietly behaves like the model ahead of it.
        let mut body = json!({ "messages": [], "temperature": 0.9 });
        let client: Vec<(String, Option<Value>)> = MEMBER_KNOBS
            .iter()
            .map(|knob| (knob.to_string(), body.get(*knob).cloned()))
            .collect();

        let tuned = lanes::MemberParams {
            temperature: Some(0.2),
            repetition_penalty: Some(1.3),
            ..Default::default()
        };
        apply_member_params(&mut body, &tuned, &client);
        assert_eq!(body["temperature"], json!(0.2));
        assert_eq!(body["repetition_penalty"], json!(1.3));

        apply_member_params(&mut body, &lanes::MemberParams::default(), &client);
        assert_eq!(body["temperature"], json!(0.9), "client value restored");
        assert!(
            body.get("repetition_penalty").is_none(),
            "lane dial unwound"
        );
    }

    #[test]
    fn a_member_resolves_to_its_own_provider() {
        // Two providers carry the same id. The member names which one it meant,
        // and that one — not whichever loaded first — must be found.
        let catalog = vec![
            providers::CatalogModel {
                id: "deepseek-chat".into(),
                provider_id: "reseller".into(),
                ..Default::default()
            },
            providers::CatalogModel {
                id: "deepseek-chat".into(),
                provider_id: "deepseek".into(),
                ..Default::default()
            },
        ];

        let member = |provider: &str| lanes::Member {
            provider: provider.into(),
            id: "deepseek-chat".into(),
            params: Default::default(),
            disabled: false,
        };

        assert_eq!(
            find_model(&catalog, &member("deepseek"))
                .unwrap()
                .provider_id,
            "deepseek"
        );

        // A pre-provider member keeps the old behaviour: first id match.
        assert_eq!(
            find_model(&catalog, &member("")).unwrap().provider_id,
            "reseller"
        );

        // A member whose provider vanished matches nothing, rather than
        // quietly borrowing another provider's entry (and key).
        assert!(find_model(&catalog, &member("closed")).is_none());
    }

    // ---------------------------------------------------------- HTTP boundary

    fn test_lane(models: &[&str]) -> lanes::Lane {
        lanes::Lane {
            slug: "fallback".into(),
            name: "Fallback".into(),
            members: models
                .iter()
                .map(|id| lanes::Member {
                    provider: "fake".into(),
                    id: (*id).into(),
                    params: Default::default(),
                    disabled: false,
                })
                .collect(),
            criteria: Vec::new(),
            suppress_reasoning: false,
            unstick: false,
            integrated_editors: Vec::new(),
        }
    }

    async fn fake_provider(
        axum::extract::State(calls): axum::extract::State<
            std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        >,
        Json(body): Json<Value>,
    ) -> Response {
        let model = body["model"].as_str().unwrap_or_default().to_string();
        calls.lock().unwrap().push(model.clone());
        if model == "primary" {
            return (StatusCode::SERVICE_UNAVAILABLE, "primary unavailable").into_response();
        }
        if body["stream"].as_bool().unwrap_or(false) {
            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(Body::from(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"fallback\"}}]}\n\ndata: [DONE]\n\n",
                ))
                .unwrap()
                .into_response();
        }
        Json(json!({"choices":[{"message":{"content":"fallback"}}]})).into_response()
    }

    async fn start_fake_provider(
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/chat/completions", post(fake_provider))
            .with_state(calls);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (address, task)
    }

    fn configure_test_files(dir: &std::path::Path, address: std::net::SocketAddr, models: &[&str]) {
        providers::save(
            dir,
            &[providers::Provider {
                id: "fake".into(),
                name: "Fake provider".into(),
                kind: "generic".into(),
                base_url: format!("http://{address}"),
                key: String::new(),
            }],
        )
        .unwrap();
        lanes::save(dir, &[test_lane(models)]).unwrap();
    }

    async fn call_lane(dir: &std::path::Path, stream: bool) -> Response {
        let body = json!({
            "model": "fallback",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": stream,
            "max_tokens": 32,
        });
        router(dir.to_path_buf(), None)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/lane/fallback/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn blocking_http_request_falls_back_and_reports_the_trail() {
        let dir = tempdir().unwrap();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (address, task) = start_fake_provider(calls.clone()).await;
        configure_test_files(dir.path(), address, &["primary", "fallback"]);

        let response = call_lane(dir.path(), false).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-visualllm-passed-over"], "1");
        assert!(response.headers()["x-visualllm-trail"]
            .to_str()
            .unwrap()
            .contains("primary"));
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, r#"{"choices":[{"message":{"content":"fallback"}}]}"#);
        assert_eq!(&*calls.lock().unwrap(), &["primary", "fallback"]);
        task.abort();
    }

    #[tokio::test]
    async fn empty_stream_falls_back_before_headers_are_committed() {
        let dir = tempdir().unwrap();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (address, task) = start_fake_provider(calls.clone()).await;
        configure_test_files(dir.path(), address, &["primary", "fallback"]);

        let response = call_lane(dir.path(), true).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-visualllm-passed-over"], "1");
        assert!(response.headers()["x-visualllm-trail"]
            .to_str()
            .unwrap()
            .contains("primary"));
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("fallback"));
        assert_eq!(&*calls.lock().unwrap(), &["primary", "fallback"]);
        task.abort();
    }

    fn lane_request(token: Option<&str>) -> axum::http::Request<Body> {
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri("/lane/fallback/v1/chat/completions")
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        builder
            .body(Body::from(
                json!({"model":"fallback","messages":[{"role":"user","content":"hello"}],"max_tokens":32})
                    .to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn lane_endpoints_require_the_gateway_token() {
        let dir = tempdir().unwrap();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (address, task) = start_fake_provider(calls.clone()).await;
        configure_test_files(dir.path(), address, &["primary", "fallback"]);
        let app = router(dir.path().to_path_buf(), Some("secret-token".to_string()));

        // No token: refused before any upstream call.
        let denied = app.clone().oneshot(lane_request(None)).await.unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert!(
            calls.lock().unwrap().is_empty(),
            "upstream must not be called"
        );

        // Wrong token: refused too.
        let wrong = app
            .clone()
            .oneshot(lane_request(Some("wrong-token")))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert!(
            calls.lock().unwrap().is_empty(),
            "upstream must not be called"
        );

        // Correct token: reaches the lane and answers.
        let allowed = app
            .oneshot(lane_request(Some("secret-token")))
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(&*calls.lock().unwrap(), &["primary", "fallback"]);
        task.abort();
    }

    #[tokio::test]
    async fn health_and_models_stay_open_without_a_token() {
        let dir = tempdir().unwrap();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (address, task) = start_fake_provider(calls.clone()).await;
        configure_test_files(dir.path(), address, &["primary"]);
        let app = router(dir.path().to_path_buf(), Some("secret-token".to_string()));

        let health = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let models = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(models.status(), StatusCode::OK);
        task.abort();
    }
}
