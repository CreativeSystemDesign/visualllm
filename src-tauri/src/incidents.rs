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
}

pub fn store_path(dir: &PathBuf) -> PathBuf {
    dir.join("incidents.json")
}

pub fn load(dir: &PathBuf) -> Vec<Incident> {
    std::fs::read_to_string(store_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Append one incident, trimming to the newest `KEEP`. Best-effort by
/// design: evidence-keeping must never fail a request that is already
/// having a bad day.
pub fn record(dir: &PathBuf, incident: Incident) {
    if incident.evidence.trim().is_empty() {
        return; // no receipts, no record — the rule, mechanically enforced
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
    if let Ok(text) = serde_json::to_string_pretty(&all) {
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
    if text.contains("reasoning") {
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
        }
    }

    #[test]
    fn no_receipts_no_record() {
        let dir = std::env::temp_dir().join(format!("vll-inc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        record(&dir, incident("   "));
        assert!(load(&dir).is_empty(), "evidence-free incidents are opinions");
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
        assert_eq!(all.last().unwrap().at, (KEEP + 24) as u64, "newest survives");
        assert_eq!(all.first().unwrap().at, 25, "oldest trimmed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_map_to_kinds_and_unknowns_stay_honest() {
        assert_eq!(kind_of("spent the whole token budget reasoning, with no room left to answer"), "reasoning_burn");
        assert_eq!(kind_of("error mid-stream: Rate limit exceeded"), "midstream_error");
        assert_eq!(kind_of("stream ended with no usable content (finish_reason: stop)"), "empty_response");
        assert_eq!(kind_of("rate limited (429)"), "rate_limited");
        assert_eq!(kind_of("this model does not support tools (400)"), "capability_gap");
        assert_eq!(kind_of("prompt too long for this model (400)"), "context_overflow");
        assert_eq!(kind_of("something entirely new"), "unattributed");
    }
}
