//! Lanes on disk.
//!
//! A lane is a name, a slug, and an ordered list of members — `members[0]`
//! answers first. That is the entire shape, and it is deliberately small:
//! everything else about a lane is derived from the models it points at.
//!
//! The slug is fixed when the lane is created and never follows the name.
//! Renaming a lane must not move the URL a client is already pointed at.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The dials a lane may fix for one member.
///
/// Every field is optional, and an absent field means "whatever the client
/// asked for, or the provider's default" — never zero. That distinction is
/// the whole type: a temperature of 0 is a strong instruction, and a member
/// with no opinion about temperature must not accidentally issue it.
///
/// These are per-MEMBER, not per-lane, deliberately. The same lane can hold
/// a model that needs a repetition penalty to stay on task and one that
/// behaves at its defaults; a lane-wide setting would force the choice on
/// both.
#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MemberParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    /// Not in the OpenAI schema, but OpenRouter and most local servers accept
    /// it, and it is the dial that actually addresses token-level loops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f64>,
    /// A ceiling on the answer, useful for lanes that serve quick lookups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

impl MemberParams {
    /// True when no dial is set — used to keep unset members compact on disk.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One model in a lane or the pool: WHICH PROVIDER serves it, its id there,
/// and the dials the lane holds it to.
///
/// A bare id stopped being an identity the moment a second provider could be
/// configured. Two providers can and do carry the same id — `deepseek-chat`
/// direct and through a reseller, `llama3` on two local servers — and a lane
/// that stores only the string routes to whichever provider happened to load
/// first. The pair is the identity; the params are the member's own tuning.
///
/// `provider` may be empty, and that is not an error: files written before
/// this type existed hold bare strings, and an empty provider means "whichever
/// provider first matches the id" — exactly the old behaviour, so old files
/// keep working unchanged until the UI naturally re-saves them qualified.
#[derive(Serialize, Clone, PartialEq)]
pub struct Member {
    pub provider: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "MemberParams::is_empty")]
    pub params: MemberParams,
    /// Parked members keep their place and their dials but are skipped at
    /// request time, so a lane can be tuned by subtraction without losing the
    /// work of arranging it. Off the disk when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

/// Accept every file shape this type has ever had: `"model-id"` from before
/// providers were part of identity, and `{ provider, id }` with or without
/// `params` since.
impl<'de> Deserialize<'de> for Member {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bare(String),
            Full {
                #[serde(default)]
                provider: String,
                id: String,
                #[serde(default)]
                params: MemberParams,
                #[serde(default)]
                disabled: bool,
            },
        }
        Ok(match Raw::deserialize(de)? {
            Raw::Bare(id) => Member {
                provider: String::new(),
                id,
                params: MemberParams::default(),
                disabled: false,
            },
            Raw::Full {
                provider,
                id,
                params,
                disabled,
            } => Member {
                provider,
                id,
                params,
                disabled,
            },
        })
    }
}

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
    pub members: Vec<Member>,
    /// What the lane was built to be: the criteria that were locked in the
    /// browser when its models were chosen.
    ///
    /// A SNAPSHOT, deliberately — a record of intent, not a query that can be
    /// re-run to reproduce the lane. Two reasons it cannot be the latter:
    ///
    ///   * Metric coverage is per-provider. Only OpenRouter publishes
    ///     benchmarks, throughput and pricing in its catalog; a provider added
    ///     directly returns little more than model ids. A lane mixing the two
    ///     has members living in different metric spaces, and re-running its
    ///     criteria would rank the direct-provider ones as unjudgeable and drop
    ///     them — a lane whose own question cannot find half its members.
    ///
    ///   * Percentiles are computed across whatever is in the catalog. Add a
    ///     provider carrying two hundred more models and every percentile
    ///     shifts, so the same criteria return a different answer for reasons
    ///     unrelated to any model changing.
    ///
    /// As a label it survives both, because it describes what someone was
    /// looking for rather than computing anything.
    ///
    /// `#[serde(default)]` so lanes saved before this existed still load.
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    /// Ask members not to spend tokens thinking before they answer.
    ///
    /// A lane PREFERENCE, not a guarantee: it reaches providers that expose
    /// the knob (OpenRouter normalises it across models), and the engine's
    /// commit gate catches the ones that think anyway. Off by default —
    /// thinking is only a problem when a lane exists to answer fast.
    #[serde(default)]
    pub suppress_reasoning: bool,
    /// Watch for a stuck agent and break it out (see `loopwatch.rs`).
    ///
    /// When on, a request whose own conversation shows a tool-call loop gets
    /// the measured treatment — redundant pairs collapsed, a note at the
    /// tail — before any member is contacted. This is the engine's one
    /// modification of a conversation, so it is opt-in per lane, logged, and
    /// announced in an `x-visualllm-unstuck` header rather than done quietly.
    #[serde(default)]
    pub unstick: bool,
}

const STATE_SCHEMA_VERSION: u32 = 1;

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

fn write_state<T>(path: PathBuf, data: &T) -> Result<(), String>
where
    T: Serialize + ?Sized,
{
    let text = serde_json::to_string_pretty(&VersionedState {
        schema_version: STATE_SCHEMA_VERSION,
        data,
    })
    .map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}

pub fn store_path(dir: &PathBuf) -> PathBuf {
    dir.join("lanes.json")
}

pub fn load(dir: &PathBuf) -> Vec<Lane> {
    match read_state(store_path(dir)) {
        Some(v) => v,
        None => {
            eprintln!(
                "lanes: could not read lanes.json at {:?}; returning empty list",
                store_path(dir)
            );
            Vec::new()
        }
    }
}

/// The whole set is rewritten on every change. At the scale a person can drag
/// chips around, a diff would be more code than it saves.
pub fn save(dir: &PathBuf, lanes: &[Lane]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_state(store_path(dir), lanes)
}

// ============================================================================
// THE POOL
// ============================================================================

/// The models the user has chosen to keep, as (provider, id) pairs.
///
/// A provider's catalog runs to hundreds of models. The pool is the handful
/// worth having in front of you — picked in the browser, and the only thing the
/// sidebar shows. Browsing and building are different jobs, and this is the
/// line between them.
pub fn pool_path(dir: &PathBuf) -> PathBuf {
    dir.join("pool.json")
}

pub fn pool_load(dir: &PathBuf) -> Vec<Member> {
    match read_state(pool_path(dir)) {
        Some(v) => v,
        None => {
            eprintln!(
                "lanes: could not read pool.json at {:?}; returning empty list",
                pool_path(dir)
            );
            Vec::new()
        }
    }
}

pub fn pool_save(dir: &PathBuf, members: &[Member]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_state(pool_path(dir), members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_load_from_every_file_shape() {
        // Bare strings, pairs, and pairs with params have all been valid at
        // some point. All must load, or an upgrade silently empties lanes.
        let lane: Lane = serde_json::from_str(
            r#"{"slug":"s","name":"n","members":[
                "openai/gpt-4o",
                {"provider":"groq","id":"llama-3.3-70b"},
                {"provider":"or","id":"m","params":{"temperature":0.2,"repetition_penalty":1.3}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(lane.members[0].provider, "");
        assert_eq!(lane.members[0].id, "openai/gpt-4o");
        assert!(lane.members[0].params.is_empty());
        assert_eq!(lane.members[1].provider, "groq");
        assert!(lane.members[1].params.is_empty());
        assert_eq!(lane.members[2].params.temperature, Some(0.2));
        assert_eq!(lane.members[2].params.repetition_penalty, Some(1.3));
    }

    #[test]
    fn unset_dials_stay_off_the_disk() {
        // A member with no settings serialises without a `params` key at all.
        // The file is read by hand when things go wrong; noise costs trust.
        let plain = Member {
            provider: "p".into(),
            id: "m".into(),
            params: MemberParams::default(),
            disabled: false,
        };
        assert!(!serde_json::to_string(&plain).unwrap().contains("params"));
        assert!(!serde_json::to_string(&plain).unwrap().contains("disabled"));
    }

    #[test]
    fn state_writes_a_version_and_reads_legacy_arrays() {
        let dir = tempfile::tempdir().unwrap();
        let lane = Lane {
            slug: "s".into(),
            name: "N".into(),
            members: Vec::new(),
            criteria: Vec::new(),
            suppress_reasoning: false,
            unstick: false,
        };
        save(&dir.path().to_path_buf(), std::slice::from_ref(&lane)).unwrap();
        let written = std::fs::read_to_string(store_path(&dir.path().to_path_buf())).unwrap();
        assert!(written.contains("\"schema_version\": 1"));
        assert_eq!(load(&dir.path().to_path_buf())[0].slug, "s");

        std::fs::write(
            store_path(&dir.path().to_path_buf()),
            serde_json::to_string(&[lane]).unwrap(),
        )
        .unwrap();
        assert_eq!(load(&dir.path().to_path_buf())[0].name, "N");
    }

    #[test]
    fn corrupt_and_future_state_is_ignored_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir.path().to_path_buf());
        std::fs::write(&path, "not json").unwrap();
        assert!(load(&dir.path().to_path_buf()).is_empty());
        std::fs::write(&path, r#"{\"schema_version\":99,\"data\":[]}"#).unwrap();
        assert!(load(&dir.path().to_path_buf()).is_empty());
    }
}
