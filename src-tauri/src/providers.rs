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

const OPENROUTER: &str = "https://openrouter.ai/api/v1";
const OPENAI: &str = "https://api.openai.com/v1";
const ANTHROPIC: &str = "https://api.anthropic.com/v1";

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

#[derive(Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    /// Enough to recognise which key is in place, never enough to use it.
    pub key_hint: String,
    pub has_key: bool,
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
        }
    }
}

/// One model as the sidebar needs it. Every field past `context` is optional
/// because only OpenRouter publishes them; a generic endpoint returns ids and
/// nothing else, and the UI has to stay useful either way.
#[derive(Serialize, Default)]
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
    pub reasoning_default: bool,
    pub intelligence: Option<f64>,
    pub coding: Option<f64>,
    pub agentic: Option<f64>,
}

// ------------------------------------------------------------------- storage

pub fn store_path(dir: &PathBuf) -> PathBuf {
    dir.join("providers.json")
}

pub fn load(dir: &PathBuf) -> Vec<Provider> {
    std::fs::read_to_string(store_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(dir: &PathBuf, providers: &[Provider]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = store_path(dir);
    let text = serde_json::to_string_pretty(providers).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;

    // The file holds API keys in plaintext. Owner-only is the floor, not the
    // fix — this wants the OS keychain before anyone else installs it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
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
        reasoning_default: raw["reasoning"]["default_enabled"]
            .as_bool()
            .unwrap_or(false),
        intelligence: bench["intelligence_index"].as_f64(),
        coding: bench["coding_index"].as_f64(),
        agentic: bench["agentic_index"].as_f64(),
    }
}

fn generic_model(raw: &Value, provider: &Provider) -> CatalogModel {
    let id = raw["id"].as_str().unwrap_or_default().to_string();
    CatalogModel {
        name: id.clone(),
        id,
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        context: raw["context_length"].as_u64().unwrap_or(0),
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

    let request = authorise(client.get(format!("{base}/models")), provider);

    let resp = request.send().await.map_err(|e| {
        if e.is_connect() {
            format!("could not reach {base}")
        } else {
            e.to_string()
        }
    })?;

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
    let base = if base.is_empty() { "provider".into() } else { base };

    let used: BTreeMap<&str, ()> = taken.iter().map(|p| (p.id.as_str(), ())).collect();
    if !used.contains_key(base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !used.contains_key(candidate.as_str()))
        .unwrap()
}

pub fn default_base_url(kind: &str) -> String {
    match kind {
        "openrouter" => OPENROUTER.to_string(),
        "openai" => OPENAI.to_string(),
        "anthropic" => ANTHROPIC.to_string(),
        _ => String::new(),
    }
}
