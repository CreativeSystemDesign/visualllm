//! Lanes on disk.
//!
//! A lane is a name, a slug, and an ordered list of model ids — `members[0]`
//! answers first. That is the entire shape, and it is deliberately small:
//! everything else about a lane is derived from the models it points at.
//!
//! The slug is fixed when the lane is created and never follows the name.
//! Renaming a lane must not move the URL a client is already pointed at.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One of the qualities a lane was built to satisfy.
///
/// `desc` is which end of that column counted as good — cheap or expensive,
/// fastest or slowest — so the phrase can be reconstructed exactly.
#[derive(Serialize, Deserialize, Clone)]
pub struct Criterion {
    pub field: String,
    pub desc: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Lane {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub members: Vec<String>,
    /// What the lane was built to be: the criteria that were locked in the
    /// browser when its models were chosen.
    ///
    /// This is provenance, and it is the difference between a lane being a
    /// snapshot and a standing question. Catalogs move constantly — the
    /// cheapest fast agentic model in March is not the one in September — and a
    /// lane that remembers its own question can be asked again.
    ///
    /// `#[serde(default)]` so lanes saved before this existed still load.
    #[serde(default)]
    pub criteria: Vec<Criterion>,
}

pub fn store_path(dir: &PathBuf) -> PathBuf {
    dir.join("lanes.json")
}

pub fn load(dir: &PathBuf) -> Vec<Lane> {
    std::fs::read_to_string(store_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The whole set is rewritten on every change. At the scale a person can drag
/// chips around, a diff would be more code than it saves.
pub fn save(dir: &PathBuf, lanes: &[Lane]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(lanes).map_err(|e| e.to_string())?;
    let path = store_path(dir);

    // Written beside the target and renamed over it: a crash mid-write leaves
    // the previous arrangement intact rather than a truncated file.
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())
}

// ============================================================================
// THE POOL
// ============================================================================

/// The models the user has chosen to keep, by id.
///
/// A provider's catalog runs to hundreds of models. The pool is the handful
/// worth having in front of you — picked in the browser, and the only thing the
/// sidebar shows. Browsing and building are different jobs, and this is the
/// line between them.
pub fn pool_path(dir: &PathBuf) -> PathBuf {
    dir.join("pool.json")
}

pub fn pool_load(dir: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(pool_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn pool_save(dir: &PathBuf, ids: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(ids).map_err(|e| e.to_string())?;
    let path = pool_path(dir);
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())
}
