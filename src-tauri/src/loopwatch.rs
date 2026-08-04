//! Catch an agent stuck re-calling the same tool, and break it out.
//!
//! A port of the Python gateway's `loopwatch` + `with_loop_break`, carried
//! over with its measurements. The findings this module encodes were made
//! against captured requests that reproduced real loops on demand
//! (llm_gateway, 2026-08-01), and they are worth restating because every
//! design choice below follows from one of them:
//!
//!   * An agentic client re-sends the entire conversation on every turn, so a
//!     loop is visible inside a SINGLE request. No cross-request state — which
//!     is also why this fits an engine that deliberately holds none.
//!
//!   * There are two species of stuck. VERBATIM: the same call with the same
//!     arguments, over and over — the model pattern-matching its own
//!     transcript, where a run of identical calls reads as the thing to do
//!     next. FUTILE: varying arguments that all return byte-identical results
//!     — a chunked read walking off the end of a file, or a batch of calls
//!     all missing the same required parameter, all getting the same
//!     rejection. The unifying definition of stuck is not "repeating
//!     arguments"; it is RECEIVING NO NEW INFORMATION.
//!
//!   * The treatment that measured 0/4 repeats (control: 4/4) is BOTH
//!     removing the redundant call/result pairs and appending a note naming
//!     the loop — as the LAST message, never the system prompt. The same
//!     note in the system prompt did nothing (2/2 still looped): at 150K
//!     tokens the system prompt is 150K tokens in the past. Placement, not
//!     wording.
//!
//!   * The futile note QUOTES the repeated result instead of guessing the
//!     cause. The first live catch was not the end-of-file walk the note was
//!     written for — it was six parallel reads each missing a parameter. The
//!     cause is always sitting in the bytes the model kept receiving.
//!
//! The collapse's safety invariant, stated once and tested below: a pair is
//! removed only when the call AND the result it returned are byte-identical
//! to a later pair's. If the agent edited a file between two identical reads,
//! the results differ and nothing is removed — this pass cannot hide a change
//! from the model.
//!
//! Rejected, deliberately (and worth not re-deriving): having the engine
//! answer a duplicate call itself from the earlier result. It would fire
//! sooner, but the engine cannot see the filesystem — it can only know that
//! some mutating tool ran, not what it touched, so safe cache invalidation is
//! total invalidation, which is inert during exactly the edit-heavy work
//! where loops happen. Its failure mode — an agent reasoning over a file it
//! never actually re-read — is silent, where a loop is loud.

use std::collections::HashMap;

use serde_json::Value;

/// Identical answered calls before intervening. Three matches the measured
/// configuration; two would fire on ordinary, legitimate re-checks.
pub const THRESHOLD: usize = 3;

/// What was done to the conversation, for the log and the response header.
pub struct Broke {
    /// `repeat` (verbatim species) or `futile` (no-new-information species).
    pub kind: &'static str,
    pub tool: String,
    pub times: usize,
    /// Redundant call/result pairs removed. Zero for the futile species —
    /// nothing repeats verbatim there, so there is nothing safe to remove.
    pub collapsed: usize,
    /// Futile species only: the result the model kept receiving, quoted and
    /// truncated. The note shows it to the model; carrying it here lets the
    /// incident show it to the *user* — the diagnosis card can then say
    /// exactly what went unread, instead of leaving that to be inferred
    /// from the client's transcript after the fact.
    pub excerpt: Option<String>,
}

// ---------------------------------------------------------------- inspection

fn tool_calls_of(message: &Value) -> &[Value] {
    message["tool_calls"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// A call's identity: tool name plus its arguments, verbatim. Reading the
/// same file twice with different ranges is progress; the identical call
/// twice is not.
fn signature(call: &Value) -> (String, String) {
    let function = &call["function"];
    let name = function["name"].as_str().unwrap_or("?").to_string();
    let args = match &function["arguments"] {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(), // serde_json orders keys, so this is stable
    };
    (name, args)
}

fn call_id(call: &Value) -> String {
    call["id"].as_str().unwrap_or("").to_string()
}

fn result_text(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// What each tool call returned, keyed by call id. The full text is kept and
/// compared byte-for-byte — grouping by a digest and trusting it would let a
/// hash collision collapse two different results, and the collapse's safety
/// rests entirely on equality meaning equality.
fn results_by_id(messages: &[Value]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for message in messages {
        if message["role"].as_str() == Some("tool") {
            let id = message["tool_call_id"].as_str().unwrap_or("").to_string();
            out.insert(id, result_text(message));
        }
    }
    out
}

/// One verbatim repeat: the same (tool, arguments) issued `times` times.
pub struct Repeat {
    pub tool: String,
    /// The arguments, verbatim — kept so a repeat can be tested for
    /// LIVENESS against the conversation's most recent call.
    pub args: String,
    pub times: usize,
    /// Whether every one of those calls got a result back. An agent
    /// re-trying a call that never came back is behaving correctly, and
    /// telling it "you already have the result" would be false.
    pub answered: bool,
}

/// Identical tool calls repeated at least `threshold` times, worst first.
pub fn find_repeats(messages: &[Value], threshold: usize) -> Vec<Repeat> {
    let results = results_by_id(messages);
    let mut counts: HashMap<(String, String), (usize, bool)> = HashMap::new();
    for message in messages {
        if message["role"].as_str() != Some("assistant") {
            continue;
        }
        for call in tool_calls_of(message) {
            let entry = counts.entry(signature(call)).or_insert((0, true));
            entry.0 += 1;
            entry.1 &= results.contains_key(&call_id(call));
        }
    }

    let mut repeats: Vec<Repeat> = counts
        .into_iter()
        .filter(|(_, (times, _))| *times >= threshold)
        .map(|((tool, args), (times, answered))| Repeat {
            tool,
            args,
            times,
            answered,
        })
        .collect();
    repeats.sort_by(|a, b| b.times.cmp(&a.times));
    repeats
}

/// The signature of the conversation's most recent tool call — what decides
/// whether a detected pattern is a LIVE loop or old residue.
fn last_call_signature(messages: &[Value]) -> Option<(String, String)> {
    messages
        .iter()
        .rev()
        .find(|m| m["role"].as_str() == Some("assistant") && !tool_calls_of(m).is_empty())
        .and_then(|m| tool_calls_of(m).last().map(signature))
}

/// The futile species: one tool answering VARYING arguments with the same
/// bytes, and the run still live (the most recent call belongs to it). A pair
/// of same-result calls hours apart in a long session is history, not a loop.
pub struct Futile {
    pub tool: String,
    pub times: usize,
    pub variants: usize,
    /// The result the model kept receiving, flattened and truncated — quoted
    /// in the note, because the cause is always in these bytes.
    pub excerpt: String,
}

pub fn find_futile(messages: &[Value], threshold: usize) -> Option<Futile> {
    let results = results_by_id(messages);

    struct Group {
        tool: String,
        args: Vec<String>,
        times: usize,
    }
    let mut groups: HashMap<(String, String), Group> = HashMap::new();
    let mut last_key: Option<(String, String)> = None;

    for message in messages {
        if message["role"].as_str() != Some("assistant") {
            continue;
        }
        for call in tool_calls_of(message) {
            let Some(result) = results.get(&call_id(call)) else {
                continue; // unanswered: a retry in flight, not evidence
            };
            let (tool, args) = signature(call);
            let key = (tool.clone(), result.clone());
            let group = groups.entry(key.clone()).or_insert(Group {
                tool,
                args: Vec::new(),
                times: 0,
            });
            if !group.args.contains(&args) {
                group.args.push(args);
            }
            group.times += 1;
            last_key = Some(key);
        }
    }

    let key = last_key?;
    let group = &groups[&key];
    if group.times < threshold || group.args.len() < 2 {
        return None;
    }

    let flat = key.1.split_whitespace().collect::<Vec<_>>().join(" ");
    let excerpt = if flat.is_empty() {
        "(an empty result)".to_string()
    } else if flat.chars().count() > 300 {
        format!("\"{}…\"", flat.chars().take(300).collect::<String>())
    } else {
        format!("\"{flat}\"")
    };

    Some(Futile {
        tool: group.tool.clone(),
        times: group.times,
        variants: group.args.len(),
        excerpt,
    })
}

// ------------------------------------------------------------------ collapse

/// Remove redundant call/result pairs, keeping the most recent copy.
///
/// A pair is redundant only when the call and the result it returned are both
/// byte-identical to a later pair's — the same read handing back the same
/// bytes. The copy kept is the LAST, not the first: at 150K tokens,
/// "reachable" means near the end, because recency is what the model attends
/// to.
///
/// An assistant turn is only removed when every call it carries is redundant;
/// a turn mixing a redundant call with a live one is left exactly as it is.
/// Anything the model *said* in a removed turn is kept — only the calls go.
pub fn collapse_repeats(messages: &[Value], threshold: usize) -> (Vec<Value>, usize) {
    let results = results_by_id(messages);

    // Every (tool, arguments, result) identity and the call ids that share it.
    let mut identities: HashMap<(String, String, String), Vec<String>> = HashMap::new();
    for message in messages {
        if message["role"].as_str() != Some("assistant") {
            continue;
        }
        for call in tool_calls_of(message) {
            let id = call_id(call);
            let Some(result) = results.get(&id) else {
                continue; // never answered: a retry, not a redundancy
            };
            let (tool, args) = signature(call);
            identities
                .entry((tool, args, result.clone()))
                .or_default()
                .push(id);
        }
    }

    // All but the most recent of each identity, once it repeats enough.
    let stale: Vec<&String> = identities
        .values()
        .filter(|ids| ids.len() >= threshold)
        .flat_map(|ids| &ids[..ids.len() - 1])
        .collect();
    if stale.is_empty() {
        return (messages.to_vec(), 0);
    }
    let stale: std::collections::HashSet<&str> = stale.iter().map(|s| s.as_str()).collect();

    // Whole turns only, so no tool result is ever orphaned from a call that
    // stayed behind.
    let mut removable: std::collections::HashSet<String> = Default::default();
    for message in messages {
        if message["role"].as_str() != Some("assistant") {
            continue;
        }
        let ids: Vec<String> = tool_calls_of(message).iter().map(call_id).collect();
        if !ids.is_empty() && ids.iter().all(|id| stale.contains(id.as_str())) {
            removable.extend(ids);
        }
    }

    let mut kept = Vec::with_capacity(messages.len());
    let mut pairs = 0;
    for message in messages {
        match message["role"].as_str() {
            Some("tool") if removable.contains(message["tool_call_id"].as_str().unwrap_or("")) => {
                continue;
            }
            Some("assistant") => {
                let ids: Vec<String> = tool_calls_of(message).iter().map(call_id).collect();
                if !ids.is_empty() && ids.iter().all(|id| removable.contains(id)) {
                    pairs += 1;
                    // Keep what the model said, drop only the redundant calls.
                    if let Some(said) = message["content"].as_str() {
                        if !said.trim().is_empty() {
                            let mut spoken = message.clone();
                            spoken.as_object_mut().map(|m| m.remove("tool_calls"));
                            kept.push(spoken);
                        }
                    }
                    continue;
                }
                kept.push(message.clone());
            }
            _ => kept.push(message.clone()),
        }
    }
    (kept, pairs)
}

// ----------------------------------------------------------------- treatment

/// The note for the verbatim species. Wording carried from the measured
/// configuration; what mattered in ablation was placement, not phrasing.
fn repeat_note(tool: &str, times: usize) -> String {
    format!(
        "You have already called `{tool}` with these exact arguments {times} times \
         in this conversation, and every call returned a result which is above. \
         Calling it again will return the same thing. Use the result you already \
         have and take the next step instead."
    )
}

/// The note for the futile species: quote the result, prescribe nothing the
/// bytes don't support.
fn futile_note(f: &Futile) -> String {
    format!(
        "Your last {times} calls to `{tool}` used {variants} different argument sets, \
         and every single one returned the identical result: {excerpt}. Varying the \
         arguments is not producing new information. Read that result closely. If it \
         describes something wrong with the call itself, fix that before calling \
         `{tool}` again. If it is empty, you are likely past the end of the file — \
         establish the file's real length instead of guessing another range. Either \
         way, do not call `{tool}` with another variation until you have dealt with \
         what it is telling you.",
        times = f.times,
        tool = f.tool,
        variants = f.variants,
        excerpt = f.excerpt,
    )
}

fn with_tail_note(messages: Vec<Value>, note: String) -> Vec<Value> {
    let mut out = messages;
    out.push(serde_json::json!({ "role": "user", "content": note }));
    out
}

/// Break a stuck agent out: remove the pattern, then name it — at the tail,
/// where it will actually be read. Returns `None` when nothing is stuck,
/// which is nearly always.
///
/// THE NOTE DESCRIBES THE LIVE LOOP, NOT HISTORY. A client resends its own
/// transcript forever, duplicates included, so a verbatim repeat detected in
/// the history may be long over — residue, not a loop. Residue is still
/// collapsed (that is always safe and always shrinks the context), but the
/// note only ever names a pattern whose most recent call proves it is
/// happening now. Without this rule, old residue permanently outranks a live
/// futile run, and the model is told about yesterday's problem on every turn
/// while today's goes unnamed — observed live, 2026-08-02, ~50 turns of it.
pub fn break_loop(messages: &[Value], threshold: usize) -> Option<(Vec<Value>, Broke)> {
    let stuck: Vec<Repeat> = find_repeats(messages, threshold)
        .into_iter()
        .filter(|r| r.answered)
        .collect();
    let last = last_call_signature(messages);
    let is_live = |r: &&Repeat| {
        last.as_ref()
            .is_some_and(|(tool, args)| *tool == r.tool && *args == r.args)
    };

    // A live verbatim loop gets the measured treatment: collapse plus note.
    if let Some(worst) = stuck.iter().find(is_live) {
        let (collapsed_messages, collapsed) = collapse_repeats(messages, threshold);
        let out = with_tail_note(collapsed_messages, repeat_note(&worst.tool, worst.times));
        return Some((
            out,
            Broke {
                kind: "repeat",
                tool: worst.tool.clone(),
                times: worst.times,
                collapsed,
                excerpt: None,
            },
        ));
    }

    // A live futile run gets its note — and any stale verbatim residue is
    // swept in the same pass, so the model reads the right diagnosis over
    // the smallest possible transcript.
    if let Some(futile) = find_futile(messages, threshold) {
        let (base, collapsed) = if stuck.is_empty() {
            (messages.to_vec(), 0)
        } else {
            collapse_repeats(messages, threshold)
        };
        let out = with_tail_note(base, futile_note(&futile));
        return Some((
            out,
            Broke {
                kind: "futile",
                tool: futile.tool,
                times: futile.times,
                collapsed,
                excerpt: Some(futile.excerpt),
            },
        ));
    }

    // Nothing live — but stale residue is still dead weight in every request
    // that follows, so it is swept without a note. There is nothing to say,
    // only something to tidy.
    let worst = stuck.first()?;
    let (out, collapsed) = collapse_repeats(messages, threshold);
    if collapsed == 0 {
        return None;
    }
    Some((
        out,
        Broke {
            kind: "sweep",
            tool: worst.tool.clone(),
            times: worst.times,
            collapsed,
            excerpt: None,
        },
    ))
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, tool: &str, args: &str) -> Value {
        json!({ "role": "assistant", "tool_calls": [
            { "id": id, "function": { "name": tool, "arguments": args } } ] })
    }

    fn result(id: &str, content: &str) -> Value {
        json!({ "role": "tool", "tool_call_id": id, "content": content })
    }

    #[test]
    fn a_verbatim_repeat_is_found_only_when_answered() {
        // Three identical answered calls are a loop. Three identical calls
        // that never got answers are a client retrying correctly, and the
        // intervention would be telling it a lie.
        let answered = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "text"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "text"),
            call("3", "read_file", r#"{"path":"a"}"#),
            result("3", "text"),
        ];
        let found = find_repeats(&answered, 3);
        assert_eq!(found.len(), 1);
        assert!(found[0].answered);

        let unanswered = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            call("2", "read_file", r#"{"path":"a"}"#),
            call("3", "read_file", r#"{"path":"a"}"#),
        ];
        assert!(!find_repeats(&unanswered, 3)[0].answered);
    }

    #[test]
    fn futile_needs_varying_arguments_and_a_live_run() {
        // Different ranges, identical empty answer: the end-of-file walk.
        let walk = vec![
            call("1", "read_file", r#"{"lines":"130-170"}"#),
            result("1", ""),
            call("2", "read_file", r#"{"lines":"130-160"}"#),
            result("2", ""),
            call("3", "read_file", r#"{"lines":"155-170"}"#),
            result("3", ""),
        ];
        let futile = find_futile(&walk, 3).expect("a live futile run");
        assert_eq!(futile.variants, 3);
        assert_eq!(futile.excerpt, "(an empty result)");

        // The same history with a different call at the end is not a live
        // loop — it is something the agent already got past.
        let mut history = walk.clone();
        history.push(call("4", "run_terminal", r#"{"cmd":"wc -l"}"#));
        history.push(result("4", "114"));
        assert!(find_futile(&history, 3).is_none());

        // One argument set repeated is the verbatim species' job, not this one.
        let verbatim = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "same"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "same"),
            call("3", "read_file", r#"{"path":"a"}"#),
            result("3", "same"),
        ];
        assert!(find_futile(&verbatim, 3).is_none());
    }

    #[test]
    fn the_futile_note_quotes_the_repeated_result() {
        // The first live catch was six reads all missing a parameter, not an
        // end-of-file walk — the note must quote, never theorise.
        let rejected = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "must have required property 'startLine'"),
            call("2", "read_file", r#"{"path":"b"}"#),
            result("2", "must have required property 'startLine'"),
            call("3", "read_file", r#"{"path":"c"}"#),
            result("3", "must have required property 'startLine'"),
        ];
        let futile = find_futile(&rejected, 3).expect("a live futile run");
        assert!(futile.excerpt.contains("startLine"));
    }

    #[test]
    fn collapse_keeps_the_last_copy_and_the_words() {
        let mut messages = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "text"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "text"),
            call("3", "read_file", r#"{"path":"a"}"#),
            result("3", "text"),
        ];
        // The second turn also said something; the words must survive.
        messages[2]["content"] = json!("Let me check that file again.");

        let (kept, pairs) = collapse_repeats(&messages, 3);
        assert_eq!(pairs, 2);
        // The survivor is the LAST call — recency is what the model attends to.
        let ids: Vec<&str> = kept
            .iter()
            .flat_map(|m| {
                tool_calls_of(m)
                    .iter()
                    .map(|c| c["id"].as_str().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(ids, vec!["3"]);
        assert!(kept
            .iter()
            .any(|m| m["content"] == json!("Let me check that file again.")));
    }

    #[test]
    fn an_edit_between_reads_stops_the_collapse() {
        // THE invariant. Same call, different bytes back — the agent edited
        // the file in between, and hiding either read would hide the edit.
        let messages = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "before edit"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "after edit"),
            call("3", "read_file", r#"{"path":"a"}"#),
            result("3", "after edit"),
        ];
        let (kept, pairs) = collapse_repeats(&messages, 3);
        assert_eq!(pairs, 0);
        assert_eq!(kept.len(), messages.len());
    }

    #[test]
    fn a_mixed_turn_is_never_partially_rewritten() {
        // One turn carries a redundant call AND a live one. It stays intact:
        // partially rewriting a turn risks orphaning results from calls.
        let mixed = json!({ "role": "assistant", "tool_calls": [
            { "id": "3", "function": { "name": "read_file", "arguments": r#"{"path":"a"}"# } },
            { "id": "4", "function": { "name": "grep", "arguments": r#"{"q":"x"}"# } } ] });
        let messages = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "text"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "text"),
            mixed,
            result("3", "text"),
            result("4", "matches"),
            call("5", "read_file", r#"{"path":"a"}"#),
            result("5", "text"),
        ];
        let (kept, _) = collapse_repeats(&messages, 3);
        assert!(
            kept.iter().any(|m| tool_calls_of(m).len() == 2),
            "mixed turn survives whole"
        );
    }

    #[test]
    fn break_loop_repairs_and_then_names_the_pattern() {
        let messages = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "text"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "text"),
            call("3", "read_file", r#"{"path":"a"}"#),
            result("3", "text"),
        ];
        let (out, broke) = break_loop(&messages, 3).expect("a stuck agent");
        assert_eq!(broke.kind, "repeat");
        assert_eq!(broke.collapsed, 2);
        assert!(
            broke.excerpt.is_none(),
            "the verbatim species has no quote to carry"
        );
        // The note is the LAST message and speaks as the user — placement is
        // the entire finding: the same text in the system prompt did nothing.
        let tail = out.last().unwrap();
        assert_eq!(tail["role"], json!("user"));
        assert!(tail["content"].as_str().unwrap().contains("read_file"));
    }

    #[test]
    fn stale_residue_is_swept_but_the_note_names_the_live_loop() {
        // Old verbatim repeats sit at the top (the client never cleans its
        // transcript); a fresh futile run is happening at the tail. The note
        // must describe the live loop — quoting the result it keeps getting —
        // while the residue is only collapsed. This exact confusion ran for
        // ~50 turns on 2026-08-02 before the rule existed.
        let messages = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "old text"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "old text"),
            call("3", "read_file", r#"{"path":"a"}"#),
            result("3", "old text"),
            call("4", "read_file", r#"{"lines":"130-170"}"#),
            result("4", "same tail"),
            call("5", "read_file", r#"{"lines":"130-160"}"#),
            result("5", "same tail"),
            call("6", "read_file", r#"{"lines":"155-170"}"#),
            result("6", "same tail"),
        ];
        let (out, broke) = break_loop(&messages, 3).expect("a live futile run");
        assert_eq!(broke.kind, "futile");
        assert_eq!(broke.collapsed, 2, "stale residue swept in the same pass");
        let note = out.last().unwrap()["content"].as_str().unwrap();
        assert!(note.contains("same tail"), "the live result is quoted");
        assert!(
            !note.contains("these exact arguments"),
            "not the stale diagnosis"
        );
        // The same quote rides out on the record, so the diagnosis card can
        // show the user exactly what the model kept ignoring.
        assert!(
            broke
                .excerpt
                .as_deref()
                .is_some_and(|e| e.contains("same tail")),
            "the incident carries the quoted result"
        );
    }

    #[test]
    fn stale_residue_alone_is_swept_without_a_note() {
        // The loop is over — the most recent call is something new. The dead
        // weight still goes, but there is nothing to tell the model.
        let messages = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "text"),
            call("2", "read_file", r#"{"path":"a"}"#),
            result("2", "text"),
            call("3", "read_file", r#"{"path":"a"}"#),
            result("3", "text"),
            call("4", "grep", r#"{"q":"port"}"#),
            result("4", "4100"),
        ];
        let (out, broke) = break_loop(&messages, 3).expect("residue to sweep");
        assert_eq!(broke.kind, "sweep");
        assert_eq!(broke.collapsed, 2);
        assert_eq!(
            out.last().unwrap()["role"],
            json!("tool"),
            "no note appended"
        );
    }

    #[test]
    fn a_healthy_conversation_is_left_alone() {
        let messages = vec![
            call("1", "read_file", r#"{"path":"a"}"#),
            result("1", "text"),
            call("2", "read_file", r#"{"path":"b"}"#),
            result("2", "other"),
        ];
        assert!(break_loop(&messages, 3).is_none());
    }
}
