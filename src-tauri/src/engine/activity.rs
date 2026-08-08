//! Append-only activity feed for live lane status.

use std::path::Path;

use serde_json::{json, Value};

/// Current wall-clock time in unix seconds.
pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append one activity event and keep only a bounded recent history.
pub(crate) fn note_activity(dir: &Path, lane: &str, member: &str, phase: &str, detail: &str) {
    let at = unix_now();
    let line = json!({
        "at": at,
        "lane": lane,
        "member": member,
        "phase": phase,
        "detail": detail.replace(['\n', '\r'], " "),
    })
    .to_string()
        + "\n";
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

/// Read recent activity, ignoring an incomplete line at a trim boundary.
pub(crate) fn read(dir: &Path, since: u64) -> Vec<Value> {
    let text = std::fs::read_to_string(dir.join("activity.jsonl")).unwrap_or_default();
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["at"].as_u64().unwrap_or(0) >= since)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_json_when_detail_contains_quotes() {
        let dir = tempfile::tempdir().unwrap();
        note_activity(
            dir.path(),
            "lane",
            "member",
            "failed",
            "provider said \"no\"",
        );

        let entries = read(dir.path(), 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["detail"], "provider said \"no\"");
    }

    #[test]
    fn filters_entries_before_since_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        note_activity(dir.path(), "lane", "member", "trying", "");

        let now = unix_now();
        assert_eq!(read(dir.path(), now + 1), Vec::<Value>::new());
        assert_eq!(read(dir.path(), now).len(), 1);
    }
}
