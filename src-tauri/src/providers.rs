//! Providers, their keys, and the catalogs they expose.
//!
//! A provider is a name, a base URL, and a key. Everything else — what models
//! exist, what they cost, how big their windows are — is asked for at runtime
//! rather than written down, because a hand-maintained model list is wrong the
//! week after you write it.
//!
//! Keys never cross back into the webview. `Provider` is what is stored;
//! `ProviderView` is what the UI is allowed to see, and it carries a masked
//! hint instead of the secret.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos",)))]
compile_error!("VisualLLM credential storage requires a Linux, Windows, or macOS native backend");

const OPENROUTER: &str = "https://openrouter.ai/api/v1";
const OPENAI: &str = "https://api.openai.com/v1";
const ANTHROPIC: &str = "https://api.anthropic.com/v1";

/// The engine needs the same auth rules the catalog fetch uses.
pub fn authorise_public(
    request: reqwest::RequestBuilder,
    provider: &Provider,
) -> reqwest::RequestBuilder {
    authorise(request, provider)
}

/// Anthropic is not OpenAI-compatible: the key goes in its own header and the
/// API is versioned by date rather than by path. Everything else here speaks
/// Bearer, so this is the one place that has to branch.
fn authorise(request: reqwest::RequestBuilder, provider: &Provider) -> reqwest::RequestBuilder {
    if provider.key.is_empty() {
        return request;
    }
    if provider.kind == "anthropic" {
        request
            .header("x-api-key", &provider.key)
            .header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(&provider.key)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// `openrouter` unlocks the richer catalog; anything else is treated as a
    /// plain OpenAI-compatible `/models` endpoint.
    pub kind: String,
    pub base_url: String,
    #[serde(default)]
    pub key: String,
}

/// Where a provider's key lives after a save. `Keyring` is the normal path.
/// `Memory` means the OS keychain was unavailable, so the secret exists only
/// for the current process and must be re-entered on the next launch.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum KeyStorage {
    Keyring,
    Memory,
}

#[derive(Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    /// Enough to recognise which key is in place, never enough to use it.
    pub key_hint: String,
    pub has_key: bool,
    /// Where this provider's key ended up after the last save. Fresh views
    /// report the configured backend's actual persistence.
    pub key_storage: KeyStorage,
}

impl From<&Provider> for ProviderView {
    fn from(p: &Provider) -> Self {
        let hint = if p.key.len() > 8 {
            format!("{}…{}", &p.key[..5], &p.key[p.key.len() - 4..])
        } else if p.key.is_empty() {
            String::new()
        } else {
            "•".repeat(p.key.len())
        };
        ProviderView {
            id: p.id.clone(),
            name: p.name.clone(),
            kind: p.kind.clone(),
            base_url: p.base_url.clone(),
            key_hint: hint,
            has_key: !p.key.is_empty(),
            key_storage: if memory_get(&p.id).is_some() {
                KeyStorage::Memory
            } else {
                configured_key_storage()
            },
        }
    }
}

/// One model as the sidebar needs it. Every field past `context` is optional
/// because only OpenRouter publishes them; a generic endpoint returns ids and
/// nothing else, and the UI has to stay useful either way.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub provider_name: String,
    pub context: u64,
    pub price_in: Option<f64>,
    pub price_out: Option<f64>,
    pub free: bool,
    pub vision: bool,
    pub tools: bool,
    /// Whether `vision`/`tools` were actually published, or merely defaulted.
    ///
    /// A generic provider's `/models` usually says nothing about capability, so
    /// its entries carry `vision: false, tools: false` — which reads exactly
    /// like "cannot" when the truth is "unstated". The engine must only treat a
    /// false as disqualifying when this is true; otherwise a tools request
    /// would skip every model from every direct provider, forever, silently.
    ///
    /// `#[serde(default)]` so a cached catalog written before this field
    /// existed loads as "unknown" — the permissive direction, never the one
    /// that starts skipping models it used to serve.
    #[serde(default)]
    pub caps_known: bool,
    pub reasoning_default: bool,
    pub intelligence: Option<f64>,
    pub coding: Option<f64>,
    pub agentic: Option<f64>,

    // --- the rest of what the browser filters on ---
    /// Everything before the slash in the id: `anthropic`, `openai`, `qwen`.
    /// 58 of them across the OpenRouter catalog, and the most natural way a
    /// person narrows a list of hundreds.
    pub author: String,
    /// Unix seconds. The only way to answer "what is new".
    pub created: u64,
    pub reasoning: bool,
    pub structured: bool,
    pub moderated: bool,
    pub description: String,
}

// ------------------------------------------------------------------- storage

pub fn store_path(dir: &std::path::Path) -> PathBuf {
    dir.join("providers.json")
}

pub fn load(dir: &std::path::Path) -> Vec<Provider> {
    let mut providers: Vec<Provider> = match read_state(store_path(dir)) {
        Some(v) => v,
        None => {
            eprintln!(
                "providers: could not read providers.json at {:?}; returning empty list",
                store_path(dir)
            );
            Vec::new()
        }
    };
    hydrate_keys(&mut providers);
    providers
}

pub fn save(dir: &std::path::Path, providers: &[Provider]) -> Result<KeyStorage, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = store_path(dir);

    // Keys live in the OS keychain, not in providers.json. Write the
    // config with a blank string in place of each secret so the file
    // remains editable by hand without exposing credentials.
    let safe: Vec<Provider> = providers
        .iter()
        .map(|p| Provider {
            key: String::new(),
            ..p.clone()
        })
        .collect();
    write_state(path, &safe)?;

    // A keychain that is missing or locked must not make saving a provider
    // impossible — that is the most basic flow there is. When it fails, the
    // key simply stays in memory for this process and the caller is told so
    // the UI can warn the user to re-enter it after a restart.
    // The crate's mock backend reports process-only persistence. Start from
    // the backend's declared lifetime so a successful mock write can never be
    // presented as an OS-keychain write.
    let mut storage = configured_key_storage();
    for provider in providers {
        // Set empty values too: clearing a provider in the UI must remove the
        // old credential rather than leaving an orphaned secret in the keychain.
        if let Err(error) = keyring_set(&provider.id, &provider.key) {
            eprintln!(
                "providers: keychain unavailable for {}; keeping key in memory only: {error}",
                provider.id
            );
            memory_set(&provider.id, &provider.key);
            storage = KeyStorage::Memory;
        } else {
            // A later successful keychain write supersedes any old degraded
            // value retained for this provider in the current process.
            memory_set(&provider.id, "");
        }
    }
    Ok(storage)
}

/// The last catalog seen, kept on disk so the engine can answer "does this
/// model take images?" without a network round trip on every inbound request.
/// Refreshed whenever the panel fetches; stale is fine, absent is not.
pub fn cache_path(dir: &std::path::Path) -> PathBuf {
    dir.join("catalog.json")
}

pub fn cache_write(dir: &std::path::Path, models: &[CatalogModel]) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("providers: failed to create cache directory {:?}: {e}", dir);
        return;
    }
    if let Err(e) = write_state(cache_path(dir), models) {
        eprintln!("providers: failed to write cache file: {e}");
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    cache_meta_write(
        dir,
        &CatalogMeta {
            stale: false,
            retained_at: now,
        },
    );
}

pub fn cache_read(dir: &std::path::Path) -> Vec<CatalogModel> {
    match read_state(cache_path(dir)) {
        Some(v) => v,
        None => {
            eprintln!(
                "providers: could not read catalog cache at {:?}; returning empty list",
                cache_path(dir)
            );
            Vec::new()
        }
    }
}

/// Whether the catalog on disk is a stale cache retained after a failed
/// refresh. The engine uses this to warn, once per request, that its
/// capability checks may be out of date.
#[derive(Serialize, Deserialize, Default)]
pub struct CatalogMeta {
    pub stale: bool,
    /// Unix seconds when the cache was last written or retained.
    pub retained_at: u64,
}

pub fn cache_meta_path(dir: &std::path::Path) -> PathBuf {
    dir.join("catalog-meta.json")
}

pub fn cache_meta_read(dir: &std::path::Path) -> CatalogMeta {
    std::fs::read_to_string(cache_meta_path(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn cache_meta_write(dir: &std::path::Path, meta: &CatalogMeta) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(meta) {
        let temp = cache_meta_path(dir).with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, cache_meta_path(dir));
        }
    }
}

// ------------------------------------------------------------------ catalogs

fn as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn openrouter_model(raw: &Value, provider: &Provider) -> CatalogModel {
    let arch = &raw["architecture"];
    let modalities = arch["input_modalities"].as_array();
    let params = raw["supported_parameters"].as_array();
    let bench = &raw["benchmarks"]["artificial_analysis"];

    let price_in = as_f64(&raw["pricing"]["prompt"]);
    let price_out = as_f64(&raw["pricing"]["completion"]);

    // `supported_parameters` is a union across every provider serving this
    // model, so treat each of these as "at least one provider offers it".
    let has = |name: &str| {
        params
            .map(|p| p.iter().any(|v| v.as_str() == Some(name)))
            .unwrap_or(false)
    };

    CatalogModel {
        id: raw["id"].as_str().unwrap_or_default().to_string(),
        name: raw["name"].as_str().unwrap_or_default().to_string(),
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        // The provider-scoped window is the real one. The model-level
        // `context_length` overstates it — gemma advertises 262K on an endpoint
        // that caps at 131K — so it is only a fallback.
        context: raw["top_provider"]["context_length"]
            .as_u64()
            .or_else(|| raw["context_length"].as_u64())
            .unwrap_or(0),
        free: price_in == Some(0.0) && price_out == Some(0.0),
        price_in,
        price_out,
        vision: modalities
            .map(|m| m.iter().any(|v| v.as_str() == Some("image")))
            .unwrap_or(false),
        // Note: `supported_parameters` is a union across every provider serving
        // the model, so this is optimistic by nature.
        tools: params
            .map(|p| p.iter().any(|v| v.as_str() == Some("tools")))
            .unwrap_or(false),
        caps_known: true,
        reasoning_default: raw["reasoning"]["default_enabled"]
            .as_bool()
            .unwrap_or(false),
        intelligence: bench["intelligence_index"].as_f64(),
        coding: bench["coding_index"].as_f64(),
        agentic: bench["agentic_index"].as_f64(),

        author: raw["id"]
            .as_str()
            .unwrap_or_default()
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string(),
        created: raw["created"].as_u64().unwrap_or(0),
        reasoning: has("reasoning") || raw["reasoning"]["default_enabled"].is_boolean(),
        structured: has("structured_outputs") || has("response_format"),
        moderated: raw["top_provider"]["is_moderated"]
            .as_bool()
            .unwrap_or(false),
        description: raw["description"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect(),
    }
}

/// A generic `/models` entry: ids for certain, everything else opportunistic.
///
/// Providers disagree wildly about what else they publish, and throwing it all
/// away flattens Mistral (which states capabilities outright) down to Ollama
/// (which states nothing). So: recognise the common spellings where they
/// appear, and leave the rest at their defaults — which the UI and the engine
/// both already treat as "unknown", not "absent".
///
/// Pricing is deliberately NOT read here. The few generic providers that
/// publish it disagree about units (per token, per million, per thousand), and
/// a price wrong by six orders of magnitude is worse than no price at all.
fn generic_model(raw: &Value, provider: &Provider) -> CatalogModel {
    let id = raw["id"].as_str().unwrap_or_default().to_string();
    // An author only exists when the id carries one. OpenRouter ids are
    // `vendor/model`; a direct provider's are bare (`gpt-4o`, `whisper-1`),
    // and splitting those on '/' returned the whole id — which made every
    // model its own author and blew the author dropdown up to one entry per
    // model the moment a second provider was added.
    let id_author = if id.contains('/') {
        id.split('/').next().unwrap_or_default().to_string()
    } else {
        String::new()
    };

    // Mistral publishes `capabilities: { vision, function_calling, ... }`.
    // Only when such an object exists do we claim to know what a model can do.
    let caps = &raw["capabilities"];
    let caps_known = caps.is_object();

    CatalogModel {
        name: raw["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string(),
        id,
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        // Three spellings for the same fact: OpenAI-compatible has no standard
        // field, so Together, Mistral and vLLM each chose their own.
        context: raw["context_length"]
            .as_u64()
            .or_else(|| raw["max_context_length"].as_u64())
            .or_else(|| raw["max_model_len"].as_u64())
            .unwrap_or(0),
        created: raw["created"].as_u64().unwrap_or(0),
        vision: caps["vision"].as_bool().unwrap_or(false),
        tools: caps["function_calling"]
            .as_bool()
            .or_else(|| caps["tool_calling"].as_bool())
            .unwrap_or(false),
        caps_known,
        description: raw["description"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect(),
        author: id_author,
        ..Default::default()
    }
}

pub async fn fetch(provider: &Provider) -> Result<Vec<CatalogModel>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let base = if provider.base_url.trim().is_empty() {
        OPENROUTER.to_string()
    } else {
        provider.base_url.trim_end_matches('/').to_string()
    };

    // Try `{base}/models`, and on a 404 try `{base}/v1/models`.
    //
    // Two things need this. Perplexity genuinely serves its catalog under /v1
    // while serving chat at the root, so no single base URL reaches both. And
    // it rescues the commonest setup mistake there is — pasting a base without
    // the /v1 — which otherwise produces a bare 404 that explains nothing.
    let mut resp = authorise(client.get(format!("{base}/models")), provider)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                format!("could not reach {base}")
            } else {
                e.to_string()
            }
        })?;

    if resp.status().as_u16() == 404 && !base.ends_with("/v1") {
        if let Ok(second) = authorise(client.get(format!("{base}/v1/models")), provider)
            .send()
            .await
        {
            if second.status().is_success() || second.status().as_u16() != 404 {
                resp = second;
            }
        }
    }

    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        return Err(match code {
            401 | 403 if provider.key.is_empty() => "this service needs an API key".to_string(),
            401 | 403 => "the key was rejected".to_string(),
            404 => format!("no /models endpoint at {base}"),
            _ => format!("provider returned {code}"),
        });
    }

    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    let rows = body["data"]
        .as_array()
        .ok_or_else(|| "no `data` array in the response".to_string())?;

    let openrouter = provider.kind == "openrouter";
    Ok(rows
        .iter()
        .map(|raw| {
            if openrouter {
                openrouter_model(raw, provider)
            } else {
                generic_model(raw, provider)
            }
        })
        .filter(|m| !m.id.is_empty())
        .collect())
}

/// A stable id from the name, unique against what is already stored.
pub fn slug(name: &str, taken: &[Provider]) -> String {
    let base: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-').replace("--", "-");
    let base = if base.is_empty() {
        "provider".into()
    } else {
        base
    };

    let used: BTreeMap<&str, ()> = taken.iter().map(|p| (p.id.as_str(), ())).collect();
    if !used.contains_key(base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !used.contains_key(candidate.as_str()))
        .unwrap_or_else(|| {
            eprintln!(
                "providers::slug: failed to find unique slug for base '{}', returning base",
                base
            );
            base
        })
}

pub fn default_base_url(kind: &str) -> String {
    match kind {
        "openrouter" => OPENROUTER.to_string(),
        "openai" => OPENAI.to_string(),
        "anthropic" => ANTHROPIC.to_string(),
        _ => String::new(),
    }
}

// ============================================================================
// ENDPOINT STATISTICS
// ============================================================================
//
// Throughput and latency are not in the model list. They live on a per-model
// resource — `/models/{id}/endpoints` — one HTTP call each, 337 of them for a
// full OpenRouter catalog. So they are fetched in the background, cached to
// disk, and merged in when the browser renders.
//
// Two things learned the hard way and worth keeping written down:
//
//   * The figures are NULL WITHOUT A KEY. The same request unauthenticated
//     returns the fields present and empty, which reads exactly like "this
//     model has no data" rather than "you did not ask properly".
//
//   * A model can serve from several providers at very different speeds. We
//     keep the BEST throughput and the LOWEST latency on offer, because that is
//     what you get if routing picks well, and it is the number that makes the
//     column comparable between models.

use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct EndpointStats {
    /// Best p50 tokens/sec across providers serving this model.
    pub throughput: Option<f64>,
    /// Lowest p50 milliseconds to first token across providers.
    pub latency: Option<f64>,
    /// How many providers carry it — a rough proxy for how reliably it routes.
    pub providers: usize,
}

#[derive(Serialize, Deserialize, Default)]
pub struct StatsFile {
    pub fetched_at: u64,
    pub models: HashMap<String, EndpointStats>,
}

pub fn stats_path(dir: &std::path::Path) -> PathBuf {
    dir.join("endpoint-stats.json")
}

pub fn stats_read(dir: &std::path::Path) -> StatsFile {
    std::fs::read_to_string(stats_path(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn stats_write(dir: &std::path::Path, file: &StatsFile) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string(file) {
        let temp = stats_path(dir).with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, stats_path(dir));
        }
    }
}

fn best_stats(body: &Value) -> EndpointStats {
    let endpoints = body["data"]["endpoints"].as_array();
    let Some(endpoints) = endpoints else {
        return EndpointStats::default();
    };

    let throughput = endpoints
        .iter()
        .filter_map(|e| e["throughput_last_30m"]["p50"].as_f64())
        .fold(None, |best: Option<f64>, v| {
            Some(best.map_or(v, |b| b.max(v)))
        });

    let latency = endpoints
        .iter()
        .filter_map(|e| e["latency_last_30m"]["p50"].as_f64())
        .fold(None, |best: Option<f64>, v| {
            Some(best.map_or(v, |b| b.min(v)))
        });

    EndpointStats {
        throughput,
        latency,
        providers: endpoints.len(),
    }
}

/// Fetch stats for every model of every provider that can supply them.
///
/// Concurrency is capped: 337 simultaneous requests would be throttled, and
/// being throttled here costs the whole column rather than one row. A failure
/// on any single model leaves it absent rather than aborting the run — a
/// partial column is useful, a missing one is not.
pub async fn hydrate_stats(
    dir: &std::path::Path,
    models: &[CatalogModel],
    all: &[Provider],
) -> usize {
    use futures_util::stream::{self, StreamExt};

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(_) => return 0,
    };

    // Only OpenRouter publishes this resource, and only to an authenticated
    // caller.
    let usable: Vec<&Provider> = all
        .iter()
        .filter(|p| p.kind == "openrouter" && !p.key.is_empty())
        .collect();
    if usable.is_empty() {
        return 0;
    }

    // Each job owns its provider rather than borrowing one. A borrow here would
    // have to survive an `.await`, and the compiler cannot prove a reference
    // lives long enough inside a spawned future — cloning a handful of small
    // structs is cheaper than the alternative and the error it produces
    // ("implementation of FnOnce is not general enough") explains nothing.
    let jobs: Vec<(String, Provider)> = models
        .iter()
        .filter_map(|m| {
            usable
                .iter()
                .find(|p| p.id == m.provider_id)
                .map(|p| (m.id.clone(), (*p).clone()))
        })
        .collect();

    let found: Vec<(String, EndpointStats)> = stream::iter(jobs)
        .map(|(id, provider)| {
            let client = client.clone();
            async move {
                let base = provider.base_url.trim_end_matches('/').to_string();
                let request = authorise(
                    client.get(format!("{base}/models/{id}/endpoints")),
                    &provider,
                );
                let stats = match request.send().await {
                    Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
                        Ok(body) => best_stats(&body),
                        Err(_) => EndpointStats::default(),
                    },
                    _ => EndpointStats::default(),
                };
                (id, stats)
            }
        })
        .buffer_unordered(6)
        .collect()
        .await;

    let mut file = StatsFile {
        fetched_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        models: HashMap::new(),
    };
    let mut with_data = 0;
    for (id, stats) in found {
        if stats.throughput.is_some() || stats.latency.is_some() {
            with_data += 1;
        }
        file.models.insert(id, stats);
    }
    stats_write(dir, &file);
    with_data
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

const KEYRING_SERVICE: &str = "com.creativesystemdesign.visualllm";
static MEMORY_KEYS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, String>>> =
    std::sync::OnceLock::new();

/// Return the backend selected for this target. Referencing the target module
/// directly makes an incorrectly configured Cargo feature fail at compile time
/// instead of silently selecting keyring's mock backend.
#[cfg(target_os = "linux")]
fn configured_builder() -> Box<keyring::CredentialBuilder> {
    keyring::keyutils_persistent::default_credential_builder()
}

#[cfg(target_os = "windows")]
fn configured_builder() -> Box<keyring::CredentialBuilder> {
    keyring::windows::default_credential_builder()
}

#[cfg(target_os = "macos")]
fn configured_builder() -> Box<keyring::CredentialBuilder> {
    keyring::macos::default_credential_builder()
}

fn key_storage_for_persistence(
    persistence: keyring::credential::CredentialPersistence,
) -> KeyStorage {
    match persistence {
        keyring::credential::CredentialPersistence::UntilDelete => KeyStorage::Keyring,
        keyring::credential::CredentialPersistence::EntryOnly
        | keyring::credential::CredentialPersistence::ProcessOnly
        | keyring::credential::CredentialPersistence::UntilReboot => KeyStorage::Memory,
        _ => KeyStorage::Memory,
    }
}

fn configured_key_storage() -> KeyStorage {
    key_storage_for_persistence(configured_builder().persistence())
}

fn memory_keys() -> &'static std::sync::Mutex<BTreeMap<String, String>> {
    MEMORY_KEYS.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

fn memory_get(provider_id: &str) -> Option<String> {
    memory_keys()
        .lock()
        .ok()
        .and_then(|keys| keys.get(provider_id).cloned())
}

fn memory_set(provider_id: &str, key: &str) {
    if let Ok(mut keys) = memory_keys().lock() {
        if key.is_empty() {
            keys.remove(provider_id);
        } else {
            keys.insert(provider_id.to_string(), key.to_string());
        }
    }
}

fn keyring_account(provider_id: &str) -> String {
    format!("provider:{provider_id}")
}

fn keyring_get(provider_id: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_account(provider_id))
        .ok()
        .and_then(|entry| entry.get_password().ok())
}

fn keyring_set(provider_id: &str, key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, &keyring_account(provider_id))
        .map_err(|e| e.to_string())?;
    if key.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    } else {
        entry.set_password(key).map_err(|e| e.to_string())
    }
}

/// Best-effort removal of a provider's keychain credential. Deleting a
/// provider must never be blocked by a missing keychain, so failures are
/// logged and swallowed.
pub fn forget_key(provider_id: &str) {
    memory_set(provider_id, "");
    if let Err(error) = keyring_set(provider_id, "") {
        eprintln!("providers: could not remove key for {provider_id} from the keychain: {error}");
    }
}

fn hydrate_keys(providers: &mut [Provider]) {
    for provider in providers {
        if let Some(key) = keyring_get(&provider.id).or_else(|| memory_get(&provider.id)) {
            provider.key = key;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vll-providers-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn provider(key: &str) -> Provider {
        Provider {
            id: "test-provider".into(),
            name: "Test".into(),
            kind: "generic".into(),
            base_url: "https://example.com/v1".into(),
            key: key.into(),
        }
    }

    #[test]
    fn save_always_succeeds_and_never_writes_the_secret() {
        let dir = temp_dir("save");
        // Keychain availability varies by machine; the contract is that a save
        // always succeeds and the on-disk file never carries the secret.
        let result = save(&dir, &[provider("sk-test-secret-123")]);
        assert!(
            result.is_ok(),
            "save must degrade gracefully, got {result:?}"
        );
        let raw = std::fs::read_to_string(store_path(&dir)).unwrap();
        let stored: Value = serde_json::from_str(&raw).unwrap();
        assert!(
            !raw.contains("sk-test-secret-123"),
            "providers.json must not contain the key"
        );
        assert_eq!(stored["data"][0]["key"], "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_key_is_best_effort() {
        let dir = temp_dir("delete");
        save(&dir, &[provider("sk-test-secret-123")]).unwrap();
        // Must not panic or error even when the keychain cannot remove it.
        forget_key("test-provider");
        let raw = std::fs::read_to_string(store_path(&dir)).unwrap();
        assert!(!raw.contains("sk-test-secret-123"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn views_carry_the_key_storage_whereabouts() {
        let view = ProviderView::from(&provider("sk-1234567890abcdef"));
        assert_eq!(view.key_storage, configured_key_storage());
        assert!(view.has_key);
        assert_eq!(view.key_hint, "sk-12…cdef");
    }

    #[test]
    fn mock_or_process_only_persistence_is_never_called_keyring() {
        assert_eq!(
            key_storage_for_persistence(keyring::credential::CredentialPersistence::EntryOnly),
            KeyStorage::Memory
        );
        assert_eq!(
            key_storage_for_persistence(keyring::credential::CredentialPersistence::ProcessOnly),
            KeyStorage::Memory
        );
        assert_eq!(
            key_storage_for_persistence(keyring::credential::CredentialPersistence::UntilDelete),
            KeyStorage::Keyring
        );
    }

    #[test]
    fn configured_target_backend_is_persistent() {
        assert_eq!(configured_key_storage(), KeyStorage::Keyring);
    }

    #[test]
    fn memory_fallback_is_available_to_later_provider_loads() {
        let mut saved = provider("");
        saved.id = "memory-fallback".into();
        memory_set(&saved.id, "session-only-secret");

        let mut loaded = vec![saved];
        hydrate_keys(&mut loaded);
        assert_eq!(loaded[0].key, "session-only-secret");

        memory_set("memory-fallback", "");
    }

    #[test]
    fn views_mark_a_memory_fallback_as_memory() {
        let mut saved = provider("session-only-secret");
        saved.id = "memory-view".into();
        memory_set(&saved.id, &saved.key);

        assert_eq!(ProviderView::from(&saved).key_storage, KeyStorage::Memory);

        memory_set("memory-view", "");
    }
}
