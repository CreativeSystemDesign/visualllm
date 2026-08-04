//! VisualLLM — the desktop shell.
//!
//! ===========================================================================
//! HOW A TAURI APP IS PUT TOGETHER
//! ===========================================================================
//!
//! There are two halves that cannot see each other directly:
//!
//!   * THE WEBVIEW — `renderer/` — HTML, CSS and JavaScript. It draws
//!     everything. It has NO network access and NO filesystem access.
//!
//!   * THIS FILE — Rust. It owns the disk, the network, and the API keys.
//!
//! They talk over a channel Tauri provides. A function marked `#[tauri::command]`
//! becomes callable from JavaScript, and nothing else is. So the complete list
//! of things the UI can do is the list of commands in this file — you can read
//! the entire attack surface in one screen.
//!
//! That is not incidental tidiness. This program holds the user's API keys. If
//! the webview could make its own HTTP calls, then any injected script — a
//! dependency, a copied snippet, a model name rendered without escaping — could
//! read a key and post it somewhere. It can't, because there is no code here
//! that would let it.
//!
//! Notice too that keys travel in one direction only. `provider_save` accepts
//! one; nothing ever sends one back. The UI receives `ProviderView`, a separate
//! type that holds a masked hint instead of the secret (see `providers.rs`).
//! The compiler enforces that, so it can't be forgotten in a later edit.
//!
//! ===========================================================================
//! WHAT RUNS WHERE
//! ===========================================================================
//!
//! `main()` does two things: starts the engine (`server.rs`) on a background
//! task, and opens the window. Both live in this one process, sharing the same
//! files on disk — which is why a lane you drag is served correctly on the very
//! next request, with nothing to reload.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod incidents;
mod lanes;
mod loopwatch;
mod providers;
mod server;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;
use tokio::sync::watch;

use providers::{CatalogModel, Provider, ProviderView};

/// VS Code chatLanguageModels.json entry for a custom endpoint model.
#[derive(Serialize, Deserialize, Clone)]
struct VscodeModelEntry {
    id: String,
    name: String,
    url: String,
    #[serde(rename = "toolCalling")]
    tool_calling: bool,
    vision: bool,
    #[serde(rename = "maxInputTokens")]
    max_input_tokens: u64,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u64,
}

/// A provider entry in chatLanguageModels.json (contains a models array).
#[derive(Serialize, Deserialize, Clone)]
struct VscodeProviderEntry {
    name: String,
    vendor: String,
    #[serde(rename = "apiKey", skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(rename = "apiType", skip_serializing_if = "Option::is_none")]
    api_type: Option<String>,
    #[serde(default)]
    models: Vec<VscodeModelEntry>,
}

/// The full chatLanguageModels.json structure (array of providers).
type VscodeChatModels = Vec<VscodeProviderEntry>;

/// Path to VS Code Insiders' chatLanguageModels.json.
fn vscode_chat_models_path() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("Code - Insiders")
        .join("User")
        .join("chatLanguageModels.json"))
}

/// Read the existing chatLanguageModels.json.
#[allow(dead_code)]
fn vscode_read_models() -> Result<VscodeChatModels, String> {
    let path = vscode_chat_models_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let models: VscodeChatModels = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(models)
}

/// Write the updated chatLanguageModels.json.
fn vscode_write_models(models: &VscodeChatModels) -> Result<(), String> {
    let path = vscode_chat_models_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(models).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(())
}

/// Add or update a VisualLLM lane entry in VS Code's model picker.
#[tauri::command]
fn vscode_integrate_lane(
    app: tauri::AppHandle,
    slug: String,
    name: String,
) -> Result<(), String> {
    eprintln!("[vscode_integrate_lane] called with slug={}, name={}", slug, name);
    
    // Get store directory
    let store_path = store_dir(&app).map_err(|e| {
        let err_msg = format!("Failed to get app data dir: {}", e);
        eprintln!("[vscode_integrate_lane] {}", err_msg);
        err_msg
    })?;
    eprintln!("[vscode_integrate_lane] store_path: {:?}", store_path);
    
    let port = port_load(&store_path);
    eprintln!("[vscode_integrate_lane] port: {}", port);
    let base_url = format!("http://127.0.0.1:{port}/lane/{slug}/v1");
    eprintln!("[vscode_integrate_lane] base_url: {}", base_url);

    // Read the file as text
    let path = vscode_chat_models_path()?;
    let text = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| e.to_string())?
    } else {
        "[]".to_string()
    };
    eprintln!("[vscode_integrate_lane] Read file, length: {}", text.len());

    // Parse to find existing visualllm provider and other providers
    let mut config: VscodeChatModels = serde_json::from_str(&text).map_err(|e| {
        let err_msg = format!("Failed to parse VS Code models: {}", e);
        eprintln!("[vscode_integrate_lane] {}", err_msg);
        err_msg
    })?;
    eprintln!("[vscode_integrate_lane] Parsed {} existing provider entries", config.len());

    // Build the model entry for this lane
    let model_entry = VscodeModelEntry {
        id: slug.clone(),
        name: format!("visualllm: {name}"),
        url: base_url,
        tool_calling: true,
        vision: false,
        max_input_tokens: 250144,
        max_output_tokens: 8000,
    };
    eprintln!("[vscode_integrate_lane] Created model entry: id={}, name={}", model_entry.id, model_entry.name);

    // Find or create the "visualllm" provider entry
    let visualllm_idx = config.iter().position(|p| p.name == "visualllm");
    eprintln!("[vscode_integrate_lane] Found visualllm provider at index: {:?}", visualllm_idx);
    
    if let Some(idx) = visualllm_idx {
        // Update existing visualllm provider
        eprintln!("[vscode_integrate_lane] Updating existing visualllm provider at index {}", idx);
        let provider = &mut config[idx];
        // Remove any existing model with the same slug
        provider.models.retain(|m| m.id != slug);
        // Add the new model at the front
        provider.models.insert(0, model_entry);
    } else {
        // Create new visualllm provider entry
        eprintln!("[vscode_integrate_lane] Creating new visualllm provider");
        let provider = VscodeProviderEntry {
            name: "visualllm".to_string(),
            vendor: "customendpoint".to_string(),
            api_key: Some("placeholder".to_string()),
            api_type: Some("chat-completions".to_string()),
            models: vec![model_entry],
        };
        config.push(provider);
    }

    eprintln!("[vscode_integrate_lane] Writing {} providers to config", config.len());
    vscode_write_models(&config).map_err(|e| {
        let err_msg = format!("Failed to write VS Code models: {}", e);
        eprintln!("[vscode_integrate_lane] {}", err_msg);
        err_msg
    })?;
    eprintln!("[vscode_integrate_lane] Success!");
    Ok(())
}

/// Where the old Python gateway lives.
///
/// SCAFFOLDING. This app read its lanes from that gateway before it had an
/// engine of its own. All that remains is the status bar's live health and
/// throughput readings, and it comes out once the engine reports its own.
fn engine_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
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
    fn offline(gateway: String, error: String) -> Self {
        State {
            connected: false,
            gateway,
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

fn parse(raw: &Value, gateway: String) -> State {
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
        gateway,
        error: None,
        models,
        lanes,
        traffic,
    }
}

#[tauri::command]
async fn read_gateway(app: tauri::AppHandle) -> State {
    let gateway = match store_dir(&app) {
        Ok(dir) => engine_url(port_load(&dir)),
        Err(_) => engine_url(DEFAULT_PORT),
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(err) => return State::offline(gateway, err.to_string()),
    };

    match client.get(format!("{gateway}/health")).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(raw) => parse(&raw, gateway),
            Err(err) => State::offline(gateway, format!("unreadable response: {err}")),
        },
        Ok(resp) => State::offline(gateway, format!("gateway returned {}", resp.status().as_u16())),
        Err(err) if err.is_connect() => State::offline(gateway, "gateway offline".into()),
        Err(err) => State::offline(gateway, err.to_string()),
    }
}

// ------------------------------------------------------------------ providers

/// Where this app keeps its files — on Linux, ~/.local/share/app.visualllm.
///
/// Deliberately not next to the program. A user should be able to delete and
/// reinstall the app without losing their lanes, and the install directory is
/// often read-only anyway.
///
/// `Result<PathBuf, String>` means this returns EITHER a path OR an error, and
/// the caller is forced by the compiler to deal with both. The `?` you'll see
/// elsewhere is shorthand for "if this failed, stop and pass the error up".
fn store_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

/// What the form sends when saving a provider.
///
/// The interesting field is `key`, and it is `Option<String>` for a reason:
/// that type carries three distinct meanings a plain string cannot.
///
///     None            leave the stored key exactly as it is
///     Some("")        clear it
///     Some("sk-...")  replace it
///
/// That is why renaming a provider does not make you retype the secret. Without
/// it you would need a separate `keep_existing_key` boolean alongside the
/// string — and eventually the two disagree and someone loses a key.
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
    // Remove the credential before dropping the provider record. Otherwise a
    // deleted provider leaves a usable secret behind under its old id.
    providers::forget_key(&id)?;
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
    // Kept for the engine, which needs capabilities at request time and cannot
    // wait on a provider round trip to get them.
    providers::cache_write(&dir, &models);
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

/// Throughput and latency, from the cache. Cheap; safe to call on every render.
#[tauri::command]
fn stats_read(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let file = providers::stats_read(&store_dir(&app)?);
    serde_json::to_value(file).map_err(|e| e.to_string())
}

/// Refresh them. One HTTP call per model — slow, so the UI runs it in the
/// background and re-renders when it returns rather than waiting on it.
#[tauri::command]
async fn stats_refresh(app: tauri::AppHandle) -> Result<usize, String> {
    let dir = store_dir(&app)?;
    let models = providers::cache_read(&dir);
    let configured = providers::load(&dir);
    Ok(providers::hydrate_stats(&dir, &models, &configured).await)
}

/// What went wrong lately, with receipts — the UI turns these into
/// explanations on the canvas. Read-only; the engine is the only writer.
#[tauri::command]
fn incidents_read(app: tauri::AppHandle) -> Result<Vec<incidents::Incident>, String> {
    Ok(incidents::load(&store_dir(&app)?))
}

#[tauri::command]
fn pool_read(app: tauri::AppHandle) -> Result<Vec<lanes::Member>, String> {
    Ok(lanes::pool_load(&store_dir(&app)?))
}

#[tauri::command]
fn pool_write(app: tauri::AppHandle, ids: Vec<lanes::Member>) -> Result<(), String> {
    lanes::pool_save(&store_dir(&app)?, &ids)
}

#[tauri::command]
fn copy_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct LaneTestResult {
    ok: bool,
    status: u16,
    served_by: Option<String>,
    trail: Option<String>,
    message: String,
}

/// Test a lane through the same loopback endpoint a client uses. The renderer
/// asks Rust for this result; it never receives general network access.
#[tauri::command]
async fn lane_test(app: tauri::AppHandle, slug: String) -> Result<LaneTestResult, String> {
    let port = port_load(&store_dir(&app)?);
    let url = format!("http://127.0.0.1:{port}/lane/{slug}/v1/chat/completions");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .post(url)
        .json(&serde_json::json!({
            "model": slug,
            "messages": [{"role": "user", "content": "Reply with the single word READY."}],
            "max_tokens": 8,
            "stream": false,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let served_by = response
        .headers()
        .get("x-visualllm-served-by")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let trail = response
        .headers()
        .get("x-visualllm-trail")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = response.text().await.unwrap_or_default();
    let message = if status < 300 {
        "lane answered successfully".to_string()
    } else {
        serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("lane returned HTTP {status}"))
    };
    Ok(LaneTestResult { ok: status < 300, status, served_by, trail, message })
}

/// The port the engine answers on. Defaults to 4100; persisted in port.json.
const DEFAULT_PORT: u16 = 4100;

static ENGINE_PORT_TX: OnceLock<watch::Sender<u16>> = OnceLock::new();

fn port_path(dir: &PathBuf) -> PathBuf {
    dir.join("port.json")
}

fn port_load(dir: &PathBuf) -> u16 {
    let stored = std::fs::read_to_string(port_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["port"].as_u64())
        .unwrap_or(DEFAULT_PORT as u64) as u16;
    // Loopback safety: refuse ports that would expose the engine to the LAN.
    if stored < 1024 || stored == 22 || stored == 80 || stored == 443 {
        DEFAULT_PORT
    } else {
        stored
    }
}

fn port_save(dir: &PathBuf, port: u16) -> Result<(), String> {
    let clamped = port.max(1024).min(65535);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = port_path(dir);
    let tmp = dir.join("port.json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(&serde_json::json!({ "port": clamped }))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

#[tauri::command]
fn port_get(app: tauri::AppHandle) -> Result<u16, String> {
    let dir = store_dir(&app)?;
    Ok(port_load(&dir))
}

#[tauri::command]
fn port_set(app: tauri::AppHandle, port: u16) -> Result<u16, String> {
    let dir = store_dir(&app)?;
    let port = port.max(1024).min(65535);
    // Fail before changing persistence or notifying the server when another
    // process already owns the requested loopback port. The listener reload
    // still binds again as the final authority, but this avoids the normal
    // conflict path leaving the UI advertising an endpoint that did not move.
    if port_load(&dir) != port {
        let probe = std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|error| format!("127.0.0.1:{port} is unavailable — {error}"))?;
        drop(probe);
    }
    if let Some(sender) = ENGINE_PORT_TX.get() {
        sender.send(port).map_err(|_| "engine is not running".to_string())?;
    }
    port_save(&dir, port)?;
    Ok(port)
}

fn main() {
    tauri::Builder::default()
        // The engine owns a fixed loopback port, so a second process cannot
        // ever be a useful second window. The plugin exits the newcomer and
        // keeps the original process authoritative.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                // On Linux, especially under Wayland, a hidden or previously
                // backgrounded window may accept `show` without becoming the
                // active surface. Toggle the transient always-on-top state to
                // request focus, then immediately restore the user's normal
                // window behavior.
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_always_on_top(true);
                let _ = window.set_focus();
                let _ = window.set_always_on_top(false);
            }
        }))
        .setup(|app| {
            // The engine runs beside the window, on the same state, in the same
            // process. Closing the window stops it, which is the behaviour a
            // desktop app should have.
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let server_port = port_load(&dir);
            let (port_tx, port_rx) = watch::channel(server_port);
            let _ = ENGINE_PORT_TX.set(port_tx);
            tauri::async_runtime::spawn(async move {
                if let Err(err) = server::serve(dir, server_port, port_rx).await {
                    eprintln!("engine: {err}");
                }
            });

            // Force Mutter to recognize the entire frameless transparent window
            // surface as clickable, preventing Z-order drops on click.
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                use gtk::prelude::WidgetExt;
                if let Ok(gtk_window) = window.gtk_window() {
                    gtk_window.connect_realize(move |win| {
                        let rect = cairo::RectangleInt::new(
                            0,
                            0,
                            win.allocated_width(),
                            win.allocated_height(),
                        );
                        let region = cairo::Region::create_rectangle(&rect);
                        if let Some(gdk_window) = win.window() {
                            gdk_window.input_shape_combine_region(&region, 0, 0);
                        }
                    });
                }
            }

            Ok(())
        })
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
            lanes_write,
            incidents_read,
            pool_read,
            pool_write,
            stats_read,
            stats_refresh,
            lane_test,
            port_get,
            port_set,
            vscode_integrate_lane
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| {
            eprintln!("failed to start VisualLLM: {err}");
            std::process::exit(1);
        });
}

#[cfg(test)]
mod port_tests {
    use super::{port_load, port_save, DEFAULT_PORT};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("visualllm-port-test-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_port_uses_default() {
        let dir = temp_dir();
        assert_eq!(port_load(&dir), DEFAULT_PORT);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn valid_port_round_trips() {
        let dir = temp_dir();
        port_save(&dir, 49123).unwrap();
        assert_eq!(port_load(&dir), 49123);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsafe_ports_fall_back_to_default() {
        let dir = temp_dir();
        for port in [22, 80, 443, 1023] {
            fs::write(
                dir.join("port.json"),
                serde_json::json!({ "port": port }).to_string(),
            )
            .unwrap();
            assert_eq!(port_load(&dir), DEFAULT_PORT);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_port_uses_default() {
        let dir = temp_dir();
        fs::write(dir.join("port.json"), "not json").unwrap();
        assert_eq!(port_load(&dir), DEFAULT_PORT);
        fs::remove_dir_all(dir).unwrap();
    }
}
