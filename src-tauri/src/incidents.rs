//! Incidents: what went wrong, with the receipts.
//!
//! When a member fails, the engine already knows more than anyone: the status
//! it saw, the bytes it read, which lane toggles were on at the time. Until
//! now that knowledge lived for one response — a trail header, a log line —
//! and then it was gone. This file keeps it, so the canvas can EXPLAIN a
//! member's behaviour instead of badging it.
//!
//! The standard an incident has to meet, stated once and enforced by shape:
//! a diagnosis is only ever issued from captured evidence. Every record
//! carries the bytes (or counts) it rests on, and the UI shows the evidence
//! beside every conclusion. A failure the evidence cannot clearly attribute
//! is recorded as what it is — unattributed — never rounded up to a verdict.
//! Blaming a model without receipts is how folklore starts, and this product
//! exists for people who choose models on facts because they cannot afford
//! to choose them on brand.
//!
//! Facts live here; prose lives in the renderer. The engine records `kind`,
//! `evidence`, and the toggle state; the UI turns those into the "what
//! happened / why / what to try" explanation, because explanation wording is
//! interface, and interface iterates faster than a binary.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Keep this many recent incidents. Enough to show a pattern ("this member
/// has burned its budget on reasoning nine times today"), small enough that
/// the file stays hand-readable — it is debugging evidence, after all.
const KEEP: usize = 200;

const STATE_SCHEMA_VERSION: u32 = 1;

/// The largest captured replay body worth keeping. A chat body can carry a
/// whole conversation; past this the request is still recorded as an incident,
/// just not a replayable one — a snapshot that had to be truncated to fit
/// would replay a different request than the one that failed, which is worse
/// than no replay at all.
pub const REPLAY_BODY_CAP: usize = 32 * 1024;

#[derive(Serialize, Deserialize)]
struct VersionedState<T> {
    schema_version: u32,
    data: T,
}

fn read_state<T>(path: PathBuf) -> Option<T>
where
    T: for<'de> Deserialize<'de>,
{
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<VersionedState<T>>(&text)
        .ok()
        .filter(|state| state.schema_version <= STATE_SCHEMA_VERSION)
        .map(|state| state.data)
        .or_else(|| serde_json::from_str(&text).ok())
}

/// A captured client request, kept so the notification center can offer to
/// retry it. Method and path are stored because the record is self-contained
/// (an incident should still replay correctly if the caller's route ever
/// changes); the body is the JSON the lane was asked to serve.
///
/// The body lives on disk with the incident and never reaches the webview:
/// `incidents_read` returns only a `replayable` flag, and replay happens
/// server-side, re-applying the engine's own gateway token rather than any
/// credential a client may have put in its request.
#[derive(Serialize, Deserialize, Clone)]
pub struct Replay {
    pub method: String,
    pub path: String,
    pub body: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Incident {
    /// Unix seconds.
    pub at: u64,
    pub lane: String,
    /// The member's label, `id@provider` (or bare id for pre-provider refs).
    pub member: String,
    /// A short machine key the UI maps to an explanation: `reasoning_burn`,
    /// `empty_response`, `midstream_error`, `rate_limited`, `loop_repeat`,
    /// `loop_futile`, `request_rejected`, `unattributed`, …
    pub kind: String,
    /// The receipts: quoted provider bytes, counts, or the trail note that
    /// was generated from them. Never empty — an incident without evidence
    /// is an opinion, and opinions do not get written to disk.
    pub evidence: String,
    /// Lane state when it happened. What was already tried changes what is
    /// worth recommending: "turn thinking off" is advice when it was off all
    /// along — then the honest diagnosis is that the provider ignored the
    /// knob.
    pub no_think: bool,
    pub loopwatch: bool,
    /// How demanding the request was, for context: tool count matters when
    /// explaining schema fumbles and loops.
    pub tools: u64,
    /// Stable identity, assigned on record. Records written before this field
    /// existed load with an empty id; they are not replayable anyway.
    #[serde(default)]
    pub id: String,
    /// The request that failed, when it was small enough to keep faithfully.
    /// `None` (or missing — older records) means no replay is offered.
    #[serde(default)]
    pub replay: Option<Replay>,
}

pub fn store_path(dir: &std::path::Path) -> PathBuf {
    dir.join("incidents.json")
}

/// A unique-enough id: the second it happened, plus 64 bits from the OS
/// CSPRNG. Two failures in the same second must not share an id, because the
/// replay action targets one specific incident. Collision odds across the
/// ring are effectively zero without a counter to persist.
fn fresh_id(at: u64) -> String {
    let mut bytes = [0u8; 8];
    let mut rand = 1u64;
    if getrandom::getrandom(&mut bytes).is_ok() {
        rand = u64::from_ne_bytes(bytes);
    }
    format!("{at}-{rand:016x}")
}

pub fn load(dir: &std::path::Path) -> Vec<Incident> {
    match read_state(store_path(dir)) {
        Some(v) => v,
        None => {
            eprintln!(
                "incidents: could not read incidents.json at {:?}; returning empty list",
                store_path(dir)
            );
            Vec::new()
        }
    }
}

/// Append one incident, trimming to the newest `KEEP`. Best-effort by
/// design: evidence-keeping must never fail a request that is already
/// having a bad day.
pub fn record(dir: &std::path::Path, mut incident: Incident) {
    if incident.evidence.trim().is_empty() {
        return; // no receipts, no record — the rule, mechanically enforced
    }
    // The id is assigned here so every path into the ring produces one, and
    // the cap is enforced here so a caller can never grow the file unbounded.
    if incident.id.is_empty() {
        incident.id = fresh_id(incident.at);
    }
    if incident
        .replay
        .as_ref()
        .is_some_and(|r| r.body.len() > REPLAY_BODY_CAP)
    {
        incident.replay = None;
    }
    let mut all = load(dir);
    all.push(incident);
    if all.len() > KEEP {
        let excess = all.len() - KEEP;
        all.drain(..excess);
    }
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&VersionedState {
        schema_version: STATE_SCHEMA_VERSION,
        data: &all,
    }) {
        let temp = store_path(dir).with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, store_path(dir));
        }
    }
}

/// Map a failure note to an incident kind.
///
/// The notes are written for humans first (trail headers, the log), so the
/// kinds are derived from them in ONE place rather than threaded through
/// every failure site. A note that matches nothing is `unattributed` — the
/// evidence still gets kept, the conclusion does not get invented.
pub fn kind_of(note: &str) -> &'static str {
    let text = note.to_lowercase();
    if text.starts_with("loop (repeat") {
        return "loop_repeat";
    } else if text.starts_with("loop (futile") {
        return "loop_futile";
    } else if text.starts_with("loop (sweep") {
        return "loop_sweep";
    } else if text.starts_with("request rejected") {
        return "request_rejected";
    }
    // Checked before "no usable content": the gate's stall message contains
    // both phrases, and silence is the sharper diagnosis of the two.
    if text.contains("went silent") {
        "stalled"
    } else if text.contains("reasoning") {
        "reasoning_burn"
    } else if text.contains("error mid-stream") || text.contains("error in a 200 body") {
        "midstream_error"
    } else if text.contains("no usable content") || text.contains("unreadable 200 body") {
        "empty_response"
    } else if text.contains("rate limited") {
        "rate_limited"
    } else if text.contains("out of credit") {
        "out_of_credit"
    } else if text.contains("key rejected") || text.contains("needs an api key") {
        "key_rejected"
    } else if text.contains("not available") {
        "model_missing"
    } else if text.contains("does not support") || text.contains("does not accept") {
        "capability_gap"
    } else if text.contains("too long for this model") {
        "context_overflow"
    } else if text.contains("provider error") || text.contains("provider busy") {
        "provider_trouble"
    } else if text.contains("timed out") || text.contains("could not connect") {
        "unreachable"
    } else if text.contains("cannot serve this request") {
        "skipped_by_catalog"
    } else {
        "unattributed"
    }
}

/// Does an incident of this kind count against a lane's auto-park budget?
///
/// Only TRANSIENT, provider-side failures do — the ones a lane cannot fix by
/// trying its other members, because every member was tried and failed the
/// same way. When a 503 or a dead connection keeps happening, the lane is
/// burning attempts on something that will not succeed, and parking it is
/// the honest move.
///
/// Behavioural failures (reasoning burn, empty responses, loops, a request
/// the model simply cannot handle) are NOT budgeted: the answer to those is
/// to change the lane, not to send its traffic nowhere. Parking would hide
/// the diagnosis the canvas is meant to surface.
pub fn counts_toward_budget(kind: &str) -> bool {
    matches!(
        kind,
        "provider_trouble" | "unreachable" | "stalled" | "rate_limited"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incident(evidence: &str) -> Incident {
        Incident {
            at: 0,
            lane: "l".into(),
            member: "m@p".into(),
            kind: "empty_response".into(),
            evidence: evidence.into(),
            no_think: false,
            loopwatch: false,
            tools: 0,
            id: String::new(),
            replay: None,
        }
    }

    #[test]
    fn no_receipts_no_record() {
        let dir = std::env::temp_dir().join(format!("vll-inc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        record(&dir, incident("   "));
        assert!(
            load(&dir).is_empty(),
            "evidence-free incidents are opinions"
        );
        record(&dir, incident("finish_reason: length"));
        assert_eq!(load(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_ring_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("vll-inc-ring-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for n in 0..(KEEP + 25) {
            let mut i = incident("e");
            i.at = n as u64;
            record(&dir, i);
        }
        let all = load(&dir);
        assert_eq!(all.len(), KEEP);
        assert_eq!(
            all.last().unwrap().at,
            (KEEP + 24) as u64,
            "newest survives"
        );
        assert_eq!(all.first().unwrap().at, 25, "oldest trimmed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_map_to_kinds_and_unknowns_stay_honest() {
        assert_eq!(
            kind_of("spent the whole token budget reasoning, with no room left to answer"),
            "reasoning_burn"
        );
        assert_eq!(
            kind_of("error mid-stream: Rate limit exceeded"),
            "midstream_error"
        );
        assert_eq!(
            kind_of("stream ended with no usable content (finish_reason: stop)"),
            "empty_response"
        );
        assert_eq!(kind_of("rate limited (429)"), "rate_limited");
        assert_eq!(
            kind_of("this model does not support tools (400)"),
            "capability_gap"
        );
        assert_eq!(
            kind_of("prompt too long for this model (400)"),
            "context_overflow"
        );
        assert_eq!(kind_of("something entirely new"), "unattributed");
    }

    #[test]
    fn silence_is_named_before_its_symptoms() {
        // The gate's stall message also says "no usable content" — the
        // empty_response phrase. Silence must win that collision, or a dead
        // connection gets diagnosed as a model that answered with nothing.
        assert_eq!(
            kind_of("went silent mid-stream before any usable content — connection presumed dead"),
            "stalled"
        );
        assert_eq!(
            kind_of("went silent: no bytes for 300s — connection presumed dead"),
            "stalled"
        );
        // And a plain broken stream still reads as what it is.
        assert_eq!(
            kind_of("stream broke with no usable content: connection reset"),
            "empty_response"
        );
    }

    #[test]
    fn a_quoted_excerpt_does_not_change_a_loops_kind() {
        // Futile evidence now carries the result the model kept ignoring.
        // Whatever those quoted bytes happen to say, the "loop (…)" prefix
        // decides the kind — quotes are receipts, not verdicts.
        assert_eq!(
            kind_of(
                r#"loop (futile): `read_file` — 13 calls, 0 redundant pairs collapsed; every call returned the identical result: "Error: missing required parameter startLine. The model went silent.""#
            ),
            "loop_futile"
        );
    }

    #[test]
    fn every_record_gets_a_unique_id_that_survives_reload() {
        let dir = std::env::temp_dir().join(format!("vll-inc-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = incident("first");
        let mut b = incident("second");
        a.at = 7;
        b.at = 7; // same second must not mean the same record
        record(&dir, a);
        record(&dir, b);
        let all = load(&dir);
        let ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
        assert!(!ids[0].is_empty() && !ids[1].is_empty());
        assert_ne!(ids[0], ids[1], "same-second incidents must not share an id");
        let reloaded = load(&dir);
        assert_eq!(
            reloaded.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ids,
            "the id is part of the record, not a session artifact"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_round_trips_and_oversized_bodies_are_dropped() {
        let dir = std::env::temp_dir().join(format!("vll-inc-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut keep = incident("keep");
        keep.replay = Some(Replay {
            method: "POST".into(),
            path: "/lane/fast/v1/chat/completions".into(),
            body: r#"{"model":"fast"}"#.into(),
        });
        record(&dir, keep);
        let stored = &load(&dir)[0];
        let replay = stored.replay.as_ref().expect("the snapshot survives");
        assert_eq!(replay.method, "POST");
        assert_eq!(replay.path, "/lane/fast/v1/chat/completions");
        assert_eq!(replay.body, r#"{"model":"fast"}"#);

        let mut huge = incident("too big to keep faithfully");
        huge.replay = Some(Replay {
            method: "POST".into(),
            path: "/lane/fast/v1/chat/completions".into(),
            body: "x".repeat(REPLAY_BODY_CAP + 1),
        });
        record(&dir, huge);
        let last = load(&dir).pop().unwrap();
        assert!(
            last.replay.is_none(),
            "a body that had to be truncated must not be replayable"
        );
        assert!(
            std::fs::read_to_string(store_path(&dir)).unwrap().len() < REPLAY_BODY_CAP * 2,
            "the cap bounds the file even when callers forget"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_without_id_or_replay_still_load() {
        // Older versions wrote incidents with neither field. They must read
        // back (defaults) rather than break the whole ring.
        let dir = std::env::temp_dir().join(format!("vll-inc-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = serde_json::json!({
            "schema_version": 1,
            "data": [{
                "at": 1,
                "lane": "legacy",
                "member": "m@p",
                "kind": "unattributed",
                "evidence": "old receipts",
                "no_think": false,
                "loopwatch": false,
                "tools": 0
            }]
        });
        std::fs::write(store_path(&dir), serde_json::to_string(&old).unwrap()).unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].lane, "legacy");
        assert_eq!(loaded[0].id, "", "missing id defaults to empty");
        assert!(
            loaded[0].replay.is_none(),
            "missing replay defaults to none"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn budget_counts_only_transient_failures() {
        for transient in ["provider_trouble", "unreachable", "stalled", "rate_limited"] {
            assert!(
                counts_toward_budget(transient),
                "{transient} should park a lane"
            );
        }
        for behavioral in [
            "reasoning_burn",
            "empty_response",
            "midstream_error",
            "loop_repeat",
            "loop_futile",
            "request_rejected",
            "capability_gap",
            "context_overflow",
            "key_rejected",
            "out_of_credit",
            "model_missing",
            "unattributed",
        ] {
            assert!(
                !counts_toward_budget(behavioral),
                "{behavioral} should not park a lane"
            );
        }
    }
}
