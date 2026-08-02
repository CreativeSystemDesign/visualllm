// VisualLLM — desktop shell.
//
// Every byte of network traffic starts here. The webview has no HTTP of its own
// and no filesystem: it asks for state, it gets state. That keeps the attack
// surface to the two commands below and means the gateway needs no CORS policy
// and no knowledge that this app exists.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod lanes;
mod providers;

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

use providers::{CatalogModel, Provider, ProviderView};

fn gateway_url() -> String {
    std::env::var("VISUALLLM_GATEWAY").unwrap_or_else(|_| "http://127.0.0.1:4000".into())
}

#[derive(Serialize)]
struct Model {
    id: String,
    klass: String,
    healthy: bool,
    available: bool,
    reason: Option<String>,
    context: u64,
    tps: Option<f64>,
    #[serde(rename = "tpsSource")]
    tps_source: Option<String>,
    ttfb: Option<f64>,
}

#[derive(Serialize)]
struct Lane {
    slug: String,
    name: String,
    members: Vec<String>,
    kind: String,
    desc: String,
    computed: bool,
}

/// Lifetime totals across every lane. Health is a snapshot; this is the record
/// of what the gateway has actually been asked to do.
#[derive(Serialize, Default)]
struct Traffic {
    requests: u64,
    failures: u64,
}

#[derive(Serialize)]
struct State {
    connected: bool,
    gateway: String,
    error: Option<String>,
    models: Vec<Model>,
    lanes: Vec<Lane>,
    traffic: Traffic,
}

impl State {
    fn offline(error: String) -> Self {
        State {
            connected: false,
            gateway: gateway_url(),
            error: Some(error.chars().take(200).collect()),
            models: Vec::new(),
            lanes: Vec::new(),
            traffic: Traffic::default(),
        }
    }
}

fn as_u64(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn parse(raw: &Value) -> State {
    let models: Vec<Model> = raw["lanes"]
        .as_array()
        .map(|lanes| {
            lanes
                .iter()
                .map(|lane| Model {
                    id: lane["id"].as_str().unwrap_or_default().to_string(),
                    klass: lane["class"].as_str().unwrap_or("other").to_string(),
                    healthy: lane["healthy"].as_bool().unwrap_or(false),
                    available: lane["available"].as_bool().unwrap_or(false),
                    reason: lane["unavailable_reason"].as_str().map(str::to_string),
                    context: as_u64(lane.get("context")),
                    tps: lane["tps"]["value"].as_f64(),
                    tps_source: lane["tps"]["source"].as_str().map(str::to_string),
                    ttfb: lane["median_ttfb_s"].as_f64(),
                })
                .collect()
        })
        .unwrap_or_default();

    let known: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();

    let lanes: Vec<Lane> = raw["routes"]
        .as_object()
        .map(|routes| {
            routes
                .iter()
                .map(|(slug, route)| {
                    let kind = route["kind"].as_str().unwrap_or("ladder").to_string();
                    // `resolves_to` is a list only for ordered routes; the
                    // computed ones report a sentence instead, and start empty.
                    let members = route["resolves_to"]
                        .as_array()
                        .map(|ids| {
                            ids.iter()
                                .filter_map(Value::as_str)
                                .filter(|id| known.contains(id))
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    Lane {
                        slug: slug.clone(),
                        name: slug.clone(),
                        members,
                        computed: kind == "auto" || kind == "speed",
                        kind,
                        desc: route["desc"].as_str().unwrap_or_default().to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let traffic = raw["telemetry"]
        .as_object()
        .map(|lanes| {
            lanes.values().fold(Traffic::default(), |mut total, lane| {
                total.requests += as_u64(lane.get("requests"));
                total.failures += as_u64(lane.get("failures"));
                total
            })
        })
        .unwrap_or_default();

    State {
        connected: true,
        gateway: gateway_url(),
        error: None,
        models,
        lanes,
        traffic,
    }
}

#[tauri::command]
async fn read_gateway() -> State {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(err) => return State::offline(err.to_string()),
    };

    match client.get(format!("{}/status", gateway_url())).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(raw) => parse(&raw),
            Err(err) => State::offline(format!("unreadable response: {err}")),
        },
        Ok(resp) => State::offline(format!("gateway returned {}", resp.status().as_u16())),
        Err(err) if err.is_connect() => State::offline("gateway offline".into()),
        Err(err) => State::offline(err.to_string()),
    }
}

// ------------------------------------------------------------------ providers

fn store_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct ProviderInput {
    id: Option<String>,
    name: String,
    kind: String,
    base_url: Option<String>,
    /// Absent means "leave the stored key alone" — so editing a provider's name
    /// does not require retyping the secret, and the UI never has to hold it.
    key: Option<String>,
}

#[tauri::command]
fn providers_list(app: tauri::AppHandle) -> Result<Vec<ProviderView>, String> {
    let dir = store_dir(&app)?;
    Ok(providers::load(&dir).iter().map(ProviderView::from).collect())
}

#[tauri::command]
fn provider_save(app: tauri::AppHandle, input: ProviderInput) -> Result<ProviderView, String> {
    if input.name.trim().is_empty() {
        return Err("give the provider a name".into());
    }
    let dir = store_dir(&app)?;
    let mut all = providers::load(&dir);

    let base_url = input
        .base_url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| providers::default_base_url(&input.kind));

    let saved = match input.id.and_then(|id| all.iter().position(|p| p.id == id)) {
        Some(at) => {
            let existing = &mut all[at];
            existing.name = input.name.trim().to_string();
            existing.kind = input.kind;
            existing.base_url = base_url;
            if let Some(key) = input.key {
                existing.key = key.trim().to_string();
            }
            ProviderView::from(&*existing)
        }
        None => {
            let provider = Provider {
                id: providers::slug(&input.name, &all),
                name: input.name.trim().to_string(),
                kind: input.kind,
                base_url,
                key: input.key.unwrap_or_default().trim().to_string(),
            };
            let view = ProviderView::from(&provider);
            all.push(provider);
            view
        }
    };

    providers::save(&dir, &all)?;
    Ok(saved)
}

#[tauri::command]
fn provider_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let dir = store_dir(&app)?;
    let mut all = providers::load(&dir);
    all.retain(|p| p.id != id);
    providers::save(&dir, &all)
}

/// Every configured provider's catalog, merged. A provider that fails reports
/// its own error rather than emptying the sidebar for the others.
#[derive(Serialize)]
struct Catalog {
    models: Vec<CatalogModel>,
    errors: Vec<CatalogError>,
}

#[derive(Serialize)]
struct CatalogError {
    provider_id: String,
    provider_name: String,
    error: String,
}

#[tauri::command]
async fn catalog_read(app: tauri::AppHandle, id: Option<String>) -> Result<Catalog, String> {
    let dir = store_dir(&app)?;
    let all = providers::load(&dir);
    let wanted: Vec<&Provider> = match &id {
        Some(id) => all.iter().filter(|p| &p.id == id).collect(),
        None => all.iter().collect(),
    };

    let mut models = Vec::new();
    let mut errors = Vec::new();
    for provider in wanted {
        match providers::fetch(provider).await {
            Ok(mut found) => models.append(&mut found),
            Err(error) => errors.push(CatalogError {
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                error,
            }),
        }
    }
    Ok(Catalog { models, errors })
}

/// Check a provider before it is saved, so a bad key is caught at the form
/// rather than as an empty sidebar ten minutes later.
#[tauri::command]
async fn provider_test(kind: String, base_url: Option<String>, key: String) -> Result<usize, String> {
    let probe = Provider {
        id: "probe".into(),
        name: "probe".into(),
        base_url: base_url
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| providers::default_base_url(&kind)),
        kind,
        key: key.trim().to_string(),
    };
    providers::fetch(&probe).await.map(|models| models.len())
}

// ---------------------------------------------------------------------- lanes

#[tauri::command]
fn lanes_read(app: tauri::AppHandle) -> Result<Vec<lanes::Lane>, String> {
    Ok(lanes::load(&store_dir(&app)?))
}

#[tauri::command]
fn lanes_write(app: tauri::AppHandle, lanes: Vec<lanes::Lane>) -> Result<(), String> {
    lanes::save(&store_dir(&app)?, &lanes)
}

#[tauri::command]
fn copy_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            read_gateway,
            copy_text,
            providers_list,
            provider_save,
            provider_delete,
            provider_test,
            catalog_read,
            lanes_read,
            lanes_write
        ])
        .run(tauri::generate_context!())
        .expect("failed to start VisualLLM");
}
