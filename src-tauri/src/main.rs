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

#[cfg(target_os = "linux")]
use gtk::prelude::GtkWindowExt;

use providers::{CatalogModel, Provider, ProviderView};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

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
    /// The gateway bearer token, so the editor can call the lane now that the
    /// engine requires it. VS Code reads this as request headers.
    #[serde(rename = "httpHeaders", skip_serializing_if = "Option::is_none")]
    http_headers: Option<serde_json::Value>,
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

/// What one editor's integration attempt did, for the UI to report honestly.
#[derive(Serialize)]
struct VscodeIntegrationResult {
    editor: String,
    path: String,
    written: bool,
    error: Option<String>,
}

/// The per-user config root where editors store their settings, per host OS.
///
/// Confirmed layout (user-provided, 2026-08-06):
/// - Windows: `%APPDATA%\<Product>`
/// - macOS:   `$HOME/Library/Application Support/<Product>`
/// - Linux:   `$HOME/.config/<Product>`
fn config_root() -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .map_err(|_| "APPDATA not set".to_string())
    } else if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support"))
    } else {
        let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
        Ok(PathBuf::from(home).join(".config"))
    }
}

/// Paths to every supported editor's chatLanguageModels.json, with the editor
/// name each one belongs to.
///
/// The lane lands in ALL targets explicitly — writing every target makes the
/// editor a property of the write, not an accident of file history.
///
/// Only editors that actually read chatLanguageModels.json belong here.
/// Verified empirically on Linux 2026-08-06 (binary + first-run probing):
/// - VS Code and VS Code Insiders: consumers (schema registered, vendors
///   customendpoint/customoai handled).
/// - Windsurf (now "Devin Desktop", apt package devin-desktop): consumer —
///   same VS Code schema (vendors customendpoint/customoai present), reads
///   from its config dir. Current builds use `Devin`; pre-rebrand Windsurf
///   builds used `Windsurf` at the same layout.
/// - Cursor 3.14: NOT a consumer (zero chatLanguageModels refs); its base-URL
///   override and custom models live in `User/globalStorage/state.vscdb`
///   (SQLite), not settings.json — not integrable via this file.
/// - Anti-Gravity IDE 2.1.1: NOT a consumer (feature stripped from the
///   VS Code fork; config dir is `Antigravity IDE` but no file is read).
fn editor_chat_models_paths() -> Result<Vec<(PathBuf, &'static str)>, String> {
    let root = config_root()?;
    Ok(vec![
        // VS Code (stable)
        (
            root.join("Code")
                .join("User")
                .join("chatLanguageModels.json"),
            "VS Code",
        ),
        // VS Code Insiders
        (
            root.join("Code - Insiders")
                .join("User")
                .join("chatLanguageModels.json"),
            "VS Code Insiders",
        ),
        // Windsurf (rebranded to Devin Desktop). Current builds use the
        // "Devin" dir; pre-rebrand Windsurf used "Windsurf".
        (
            root.join("Devin")
                .join("User")
                .join("chatLanguageModels.json"),
            "Windsurf",
        ),
    ])
}

/// The editors a lane can be integrated into, in the order the picker menu
/// should show them.
///
/// The frontend renders its editor menu from this list rather than its own
/// copy, so adding an editor is a single change in one place.
#[tauri::command]
fn editor_list() -> Result<Vec<String>, String> {
    Ok(editor_chat_models_paths()?
        .into_iter()
        .map(|(_, name)| name.to_string())
        .collect())
}

/// Write the updated chatLanguageModels.json to one specific editor's file.
fn vscode_write_models_at(path: &std::path::Path, models: &VscodeChatModels) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(models).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Add or update one lane inside an already-parsed chatLanguageModels.json.
///
/// Re-running an integration updates the existing entry by slug rather than
/// duplicating it, and preserves every provider the user already configured.
fn vscode_merge_lane(config: &mut VscodeChatModels, entry: &VscodeModelEntry) {
    let visualllm_idx = config.iter().position(|p| p.name == "visualllm");
    if let Some(idx) = visualllm_idx {
        let provider = &mut config[idx];
        provider.models.retain(|m| m.id != entry.id);
        provider.models.insert(0, entry.clone());
    } else {
        config.push(VscodeProviderEntry {
            name: "visualllm".to_string(),
            vendor: "customendpoint".to_string(),
            api_key: None,
            api_type: Some("chat-completions".to_string()),
            models: vec![entry.clone()],
        });
    }
}

/// Derive VS Code model entry capabilities from the lane's actual members
/// and the cached catalog, rather than using hardcoded values.
///
/// A lane's members may carry vision, tools, or custom context windows.
/// The catalog tells us which capabilities each member actually has.
/// If a member has no catalog entry (deleted upstream, removed provider),
/// we fall back to the defaults — the lane still works, just without
/// capability hints in the picker.
fn vscode_model_entry(
    slug: &str,
    name: &str,
    port: u16,
    token: Option<&str>,
    catalog: &[providers::CatalogModel],
    lane: &lanes::Lane,
) -> VscodeModelEntry {
    let base_url = format!("http://127.0.0.1:{port}/lane/{slug}/v1");

    // Find catalog entries for each member so we can derive capabilities.
    let member_models: Vec<&providers::CatalogModel> = lane
        .members
        .iter()
        .filter_map(|m| {
            catalog
                .iter()
                .find(|c| c.id == m.id && (m.provider.is_empty() || c.provider_id == m.provider))
        })
        .collect();

    let vision = member_models.iter().any(|m| m.vision);
    let tool_calling = member_models.iter().any(|m| m.tools);
    let max_input_tokens = member_models
        .iter()
        .map(|m| m.context)
        .max()
        .unwrap_or(250_144);
    // CatalogModel has no max_output_tokens field — use a sensible
    // default. The VS Code picker uses this as a hint, not a hard limit.
    let max_output_tokens = 8000;

    VscodeModelEntry {
        id: slug.to_string(),
        name: format!("visualllm: {name}"),
        url: base_url,
        tool_calling,
        vision,
        max_input_tokens,
        max_output_tokens,
        http_headers: token.map(|t| serde_json::json!({ "Authorization": format!("Bearer {t}") })),
    }
}

/// Add or update a VisualLLM lane in a specific editor's model picker.
///
/// The lane is merged into only the named editor's
/// chatLanguageModels.json. Capabilities are derived from the
/// lane's actual members and the cached catalog rather than
/// hardcoded — so the picker accurately reflects what the lane can do.
#[tauri::command]
fn editor_integrate_lane(
    app: tauri::AppHandle,
    slug: String,
    name: String,
    editor: String,
) -> Result<VscodeIntegrationResult, String> {
    let store_path = store_dir(&app).map_err(|e| format!("failed to get app data dir: {e}"))?;
    let port = port_load(&store_path);
    let token = secret_load(&store_path).ok();

    let catalog = providers::cache_read(&store_path);
    let lanes = lanes::load(&store_path);
    let lane = lanes.iter().find(|l| l.slug == slug);

    let entry = match lane {
        Some(lane) => vscode_model_entry(&slug, &name, port, token.as_deref(), &catalog, lane),
        None => VscodeModelEntry {
            id: slug.clone(),
            name: format!("visualllm: {name}"),
            url: format!("http://127.0.0.1:{port}/lane/{slug}/v1"),
            tool_calling: true,
            vision: false,
            max_input_tokens: 250_144,
            max_output_tokens: 8000,
            http_headers: token
                .map(|t| serde_json::json!({ "Authorization": format!("Bearer {t}") })),
        },
    };

    let (path, editor_name) = editor_chat_models_paths()?
        .into_iter()
        .find(|(_, e)| *e == editor)
        .ok_or_else(|| format!("unknown editor: {editor}"))?;

    let outcome: Result<(), String> = (|| {
        let text = if path.exists() {
            std::fs::read_to_string(&path).map_err(|e| e.to_string())?
        } else {
            "[]".to_string()
        };
        let mut config: VscodeChatModels = serde_json::from_str(&text)
            .map_err(|e| format!("could not parse existing config: {e}"))?;
        vscode_merge_lane(&mut config, &entry);
        vscode_write_models_at(&path, &config)
    })();

    match outcome {
        Ok(()) => {
            eprintln!("[editor_integrate_lane] {editor}: wrote {}", path.display());
            // Update the lane's integrated_editors list.
            let mut lanes = lanes::load(&store_path);
            if let Some(l) = lanes.iter_mut().find(|l| l.slug == slug) {
                if !l.integrated_editors.contains(&editor) {
                    l.integrated_editors.push(editor.clone());
                }
            }
            lanes::save(&store_path, &lanes)?;
            Ok(VscodeIntegrationResult {
                editor: editor_name.to_string(),
                path: path.display().to_string(),
                written: true,
                error: None,
            })
        }
        Err(error) => {
            eprintln!("[editor_integrate_lane] {editor}: {error}");
            Err(error)
        }
    }
}

/// Remove a VisualLLM lane from a specific editor's model picker.
///
/// The lane's model entry is removed from the named editor's
/// chatLanguageModels.json. If the visualllm provider entry
/// becomes empty after removal, the entire provider is dropped.
#[tauri::command]
fn editor_remove_lane(
    app: tauri::AppHandle,
    slug: String,
    editor: String,
) -> Result<VscodeIntegrationResult, String> {
    let store_path = store_dir(&app).map_err(|e| format!("failed to get app data dir: {e}"))?;

    let (path, editor_name) = editor_chat_models_paths()?
        .into_iter()
        .find(|(_, e)| *e == editor)
        .ok_or_else(|| format!("unknown editor: {editor}"))?;

    if !path.exists() {
        return Ok(VscodeIntegrationResult {
            editor: editor_name.to_string(),
            path: path.display().to_string(),
            written: true,
            error: None,
        });
    }

    let outcome: Result<(), String> = (|| {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let mut config: VscodeChatModels = serde_json::from_str(&text)
            .map_err(|e| format!("could not parse existing config: {e}"))?;

        let visualllm_idx = config.iter().position(|p| p.name == "visualllm");
        if let Some(idx) = visualllm_idx {
            let provider = &mut config[idx];
            provider.models.retain(|m| m.id != slug);
            if provider.models.is_empty() {
                config.remove(idx);
            }
        }

        vscode_write_models_at(&path, &config)
    })();

    match outcome {
        Ok(()) => {
            // Update the lane's integrated_editors list.
            let mut lanes = lanes::load(&store_path);
            if let Some(l) = lanes.iter_mut().find(|l| l.slug == slug) {
                l.integrated_editors.retain(|e| e != &editor);
            }
            lanes::save(&store_path, &lanes)?;
            Ok(VscodeIntegrationResult {
                editor: editor_name.to_string(),
                path: path.display().to_string(),
                written: true,
                error: None,
            })
        }
        Err(error) => Err(error),
    }
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
        Ok(resp) => State::offline(
            gateway,
            format!("gateway returned {}", resp.status().as_u16()),
        ),
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
    Ok(providers::load(&dir)
        .iter()
        .map(ProviderView::from)
        .collect())
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

    let mut saved = match input.id.and_then(|id| all.iter().position(|p| p.id == id)) {
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

    saved.key_storage = providers::save(&dir, &all)?;
    Ok(saved)
}

#[tauri::command]
fn provider_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let dir = store_dir(&app)?;
    let mut all = providers::load(&dir);
    // Remove the credential before dropping the provider record. Otherwise a
    // deleted provider leaves a usable secret behind under its old id.
    providers::forget_key(&id);
    all.retain(|p| p.id != id);
    providers::save(&dir, &all)?;
    Ok(())
}

// ------------------------------------------------------------------ portability

/// A portable snapshot of configuration that can move between machines.
/// API keys are intentionally excluded: the destination must re-enter them,
/// and the keyring on the new machine is the right place for them.
#[derive(Serialize, Deserialize)]
struct PortableState {
    version: u32,
    exported_at: u64,
    lanes: Vec<lanes::Lane>,
    pool: Vec<lanes::Member>,
    providers: Vec<providers::Provider>,
}

impl PortableState {
    fn new(dir: &std::path::Path) -> Self {
        let exported_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Load providers without keys: `providers::load` hydrates from the
        // keyring, and we explicitly strip them again so a portable file never
        // carries a secret.
        let providers: Vec<providers::Provider> = providers::load(dir)
            .into_iter()
            .map(|mut p| {
                p.key.clear();
                p
            })
            .collect();
        Self {
            version: 1,
            exported_at,
            lanes: lanes::load(dir),
            pool: lanes::pool_load(dir),
            providers,
        }
    }

    fn into_state(
        self,
    ) -> (
        Vec<lanes::Lane>,
        Vec<lanes::Member>,
        Vec<providers::Provider>,
    ) {
        (self.lanes, self.pool, self.providers)
    }
}

/// Export lanes, pool, and provider config to a JSON file. Keys are never
/// included; the destination re-enters them after import.
#[tauri::command]
async fn state_export(app: tauri::AppHandle) -> Result<String, String> {
    let dir = store_dir(&app)?;
    let snapshot = PortableState::new(&dir);
    let text = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;

    let path = app
        .dialog()
        .file()
        .set_file_name("visualllm-export.json")
        .add_filter("VisualLLM export", &["json"])
        .blocking_save_file();

    let Some(path) = path else {
        return Err("export cancelled".into());
    };

    let target: std::path::PathBuf = path.as_path().map(|p| p.to_path_buf()).unwrap();
    std::fs::create_dir_all(target.parent().unwrap_or(&target)).map_err(|e| e.to_string())?;
    let temp = target.with_extension("json.tmp");
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, target.clone()).map_err(|e| e.to_string())?;
    Ok(target.to_string_lossy().into_owned())
}

/// Import lanes, pool, and provider config from a JSON file.
///
/// `mode` controls what happens to existing state:
///   * "merge"   (default) keep existing lanes/providers, add new ones by slug/id,
///               and replace when a collision occurs. Pool is unioned.
///   * "replace" wipe existing state and use the file exactly.
#[tauri::command]
async fn state_import(app: tauri::AppHandle, mode: Option<String>) -> Result<String, String> {
    let dir = store_dir(&app)?;

    let path = app
        .dialog()
        .file()
        .add_filter("VisualLLM export", &["json"])
        .blocking_pick_file();

    let Some(path) = path else {
        return Err("import cancelled".into());
    };

    let path: std::path::PathBuf = path.as_path().map(|p| p.to_path_buf()).unwrap();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let snapshot: PortableState = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a valid VisualLLM export: {e}", path.display()))?;

    let mode = mode.as_deref().unwrap_or("merge");
    match mode {
        "merge" => import_merge(&dir, snapshot),
        "replace" => import_replace(&dir, snapshot),
        other => Err(format!("unknown import mode: {other}")),
    }?;
    Ok(path.to_string_lossy().into_owned())
}

fn import_replace(dir: &std::path::Path, snapshot: PortableState) -> Result<(), String> {
    let (lanes, pool, providers) = snapshot.into_state();
    lanes::save(dir, &lanes)?;
    lanes::pool_save(dir, &pool)?;
    providers::save(dir, &providers)?;
    Ok(())
}

fn import_merge(dir: &std::path::Path, snapshot: PortableState) -> Result<(), String> {
    let (imported_lanes, imported_pool, imported_providers) = snapshot.into_state();

    let mut lanes = lanes::load(dir);
    let mut pool = lanes::pool_load(dir);
    let mut providers = providers::load(dir);

    // Providers merge by id: an imported provider with the same id replaces
    // the local one, but keys are left empty so the existing keyring entry is
    // preserved unless the provider is genuinely new.
    let mut provider_ids: std::collections::HashSet<String> =
        providers.iter().map(|p| p.id.clone()).collect();
    for p in imported_providers {
        if provider_ids.contains(&p.id) {
            let pos = providers.iter().position(|x| x.id == p.id).unwrap();
            // Keep the local key: the export never carries one, and wiping an
            // existing key would force an unnecessary re-entry.
            let local_key = providers[pos].key.clone();
            providers[pos] = p;
            providers[pos].key = local_key;
        } else {
            provider_ids.insert(p.id.clone());
            providers.push(p);
        }
    }

    // Lanes merge by slug: same slug replaces, new slug appends.
    let mut lane_slugs: std::collections::HashSet<String> =
        lanes.iter().map(|l| l.slug.clone()).collect();
    for l in imported_lanes {
        if lane_slugs.contains(&l.slug) {
            let pos = lanes.iter().position(|x| x.slug == l.slug).unwrap();
            lanes[pos] = l;
        } else {
            lane_slugs.insert(l.slug.clone());
            lanes.push(l);
        }
    }

    // Pool is a set of (provider, id) pairs.
    let mut pool_keys: std::collections::HashSet<(String, String)> = pool
        .iter()
        .map(|m| (m.provider.clone(), m.id.clone()))
        .collect();
    for m in imported_pool {
        if pool_keys.insert((m.provider.clone(), m.id.clone())) {
            pool.push(m);
        }
    }

    lanes::save(dir, &lanes)?;
    lanes::pool_save(dir, &pool)?;
    providers::save(dir, &providers)?;
    Ok(())
}

/// Every configured provider's catalog, merged. A provider that fails reports
/// its own error rather than emptying the sidebar for the others.
#[derive(Serialize)]
struct Catalog {
    models: Vec<CatalogModel>,
    errors: Vec<CatalogError>,
    stale: bool,
    retained_at: u64,
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
    //
    // A partial fetch must not shrink what the engine knows. When some
    // providers answer and others error, the successful half would overwrite a
    // complete cache with a smaller one — and every `can_serve` check for the
    // missing providers' models silently degrades to "unknown". Keep the last
    // good cache whenever this fetch errored and returned strictly less than
    // what is already stored. A genuinely empty catalog (a provider that
    // dropped every model) is rare next to a transient outage; the UI refresh
    // button is the deliberate way to force a rewrite.
    let cached = providers::cache_read(&dir);
    let shrank = !errors.is_empty() && models.len() < cached.len();
    let meta = providers::cache_meta_read(&dir);
    if shrank {
        eprintln!(
            "catalog: partial fetch ({} errors, {} models vs {} cached) — keeping the last good cache",
            errors.len(),
            models.len(),
            cached.len()
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        providers::cache_meta_write(
            &dir,
            &providers::CatalogMeta {
                stale: true,
                retained_at: now,
            },
        );
    } else {
        providers::cache_write(&dir, &models);
    }
    Ok(Catalog {
        models,
        errors,
        stale: meta.stale || shrank,
        retained_at: meta.retained_at,
    })
}

/// Check a provider before it is saved, so a bad key is caught at the form
/// rather than as an empty sidebar ten minutes later.
#[tauri::command]
async fn provider_test(
    kind: String,
    base_url: Option<String>,
    key: String,
) -> Result<usize, String> {
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

/// Lift an auto-parked lane back into rotation: clears the parked flag and
/// the accumulated failure history, so the budget starts clean. The engine
/// parks lanes; only a human unparks them.
#[tauri::command]
fn lane_unpark(app: tauri::AppHandle, slug: String) -> Result<(), String> {
    lanes::unpark(&store_dir(&app)?, &slug)
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

/// Live lane activity, for the canvas. The renderer polls this with the
/// timestamp of the newest entry it has seen; the engine is the only writer.
#[tauri::command]
fn activity_read(app: tauri::AppHandle, since: Option<u64>) -> Result<Vec<Value>, String> {
    Ok(server::activity_read(&store_dir(&app)?, since.unwrap_or(0)))
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
        // A probe must exercise the same path a real request takes. Budgets
        // under 16 bypass the engine's commit gate (server.rs), so a tiny
        // probe would pass lanes that return empty or reasoning-only bodies —
        // exactly the failures Test exists to catch. 64 is enough to answer
        // and far over the bypass, so the gate's verdict is what is measured.
        .json(&serde_json::json!({
            "model": slug,
            "messages": [{"role": "user", "content": "Reply with the single word READY."}],
            "max_tokens": 64,
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
    Ok(LaneTestResult {
        ok: status < 300,
        status,
        served_by,
        trail,
        message,
    })
}

/// The port the engine answers on. Defaults to 4100; persisted in port.json.
const DEFAULT_PORT: u16 = 4100;

static ENGINE_PORT_TX: OnceLock<watch::Sender<u16>> = OnceLock::new();

fn port_path(dir: &std::path::Path) -> PathBuf {
    dir.join("port.json")
}

fn port_load(dir: &std::path::Path) -> u16 {
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

fn port_save(dir: &std::path::Path, port: u16) -> Result<(), String> {
    let clamped = port.clamp(1024, 65535);
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

/// Where the gateway bearer token lives. A plain file (not JSON) whose content
/// is the token and nothing else; the file mode is 0600.
fn secret_path(dir: &std::path::Path) -> PathBuf {
    dir.join("secret")
}

/// The gateway bearer token, created on first use.
///
/// The loopback engine is reachable by any local process, and a web page can
/// attempt DNS rebinding. A token that clients must present makes the lane
/// endpoints — the ones that can spend money — callable only by the user's own
/// configured clients. `/health` and `/activity` stay open: they leak nothing.
fn secret_load(dir: &std::path::Path) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = secret_path(dir);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let text = text.trim();
        if !text.is_empty() {
            return Ok(text.to_string());
        }
    }
    let secret = secret_generate()?;
    secret_save(dir, &secret)?;
    Ok(secret)
}

/// 64 hex characters from the operating system's CSPRNG.
fn secret_generate() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| format!("could not generate the gateway token: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn secret_save(dir: &std::path::Path, secret: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join("secret.tmp");
    std::fs::write(&tmp, secret.as_bytes()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(tmp, secret_path(dir)).map_err(|e| e.to_string())
}

/// What the settings UI may show about the token. `token` is empty unless the
/// user asked to reveal it; `masked` is always safe to display.
#[derive(Serialize)]
struct GatewayToken {
    has: bool,
    token: String,
    masked: String,
}

#[tauri::command]
fn gateway_token(app: tauri::AppHandle, reveal: bool) -> Result<GatewayToken, String> {
    let dir = store_dir(&app)?;
    let token = secret_load(&dir)?;
    let masked = format!("{}…{}", &token[..8], &token[token.len() - 4..]);
    Ok(GatewayToken {
        has: true,
        token: if reveal { token } else { String::new() },
        masked,
    })
}

/// Rotate the token for the next engine start. The running listener keeps the
/// current token until it is rebuilt (port change or restart), so the response
/// carries the new value and the UI says when it takes effect.
#[tauri::command]
fn gateway_token_regenerate(app: tauri::AppHandle) -> Result<GatewayToken, String> {
    let dir = store_dir(&app)?;
    let token = secret_generate()?;
    secret_save(&dir, &token)?;
    let masked = format!("{}…{}", &token[..8], &token[token.len() - 4..]);
    Ok(GatewayToken {
        has: true,
        token,
        masked,
    })
}

#[tauri::command]
fn port_get(app: tauri::AppHandle) -> Result<u16, String> {
    let dir = store_dir(&app)?;
    Ok(port_load(&dir))
}

#[tauri::command]
fn port_set(app: tauri::AppHandle, port: u16) -> Result<u16, String> {
    let dir = store_dir(&app)?;
    let port = port.clamp(1024, 65535);
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
        sender
            .send(port)
            .map_err(|_| "engine is not running".to_string())?;
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
            // A token missing/corrupt on disk is regenerated here; the same
            // loader backs the settings UI, so both agree on the live secret.
            let secret = secret_load(&dir).ok();
            let (port_tx, port_rx) = watch::channel(server_port);
            let _ = ENGINE_PORT_TX.set(port_tx);
            tauri::async_runtime::spawn(async move {
                if let Err(err) = server::serve(dir, server_port, secret, port_rx).await {
                    eprintln!("engine: {err}");
                }
            });

            // Force Mutter to recognize the entire frameless transparent window
            // surface as clickable, preventing Z-order drops on click.
            //
            // The region must track the window: applied only at realize it
            // goes stale on the first resize, and clicks then fall through the
            // uncovered area to whatever window is stacked below — the "z-order"
            // bug this exists to fix. So it is re-applied on every allocation,
            // not just the first.
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                use gtk::prelude::WidgetExt;
                if let Ok(gtk_window) = window.gtk_window() {
                    fn shape_to_allocation(win: &gtk::ApplicationWindow) {
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
                    }
                    gtk_window.connect_realize(shape_to_allocation);
                    gtk_window.connect_size_allocate(|win, _| shape_to_allocation(win));

                    // Set window icon for taskbar/dock
                    if let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file("icons/icon.png") {
                        gtk_window.set_icon(Some(&pixbuf));
                    }
                }
            }

            // Self-update: shortly after startup, ask GitHub whether a newer
            // build exists, download and install it, then offer to restart.
            //
            // Every network byte moves through Rust, so the webview keeps its
            // no-network posture — an update needs no CSP relaxation and no
            // new command. The endpoint, pubkey, and version come from
            // tauri.conf.json (plugins.updater), so nothing is hardcoded here.
            let updater_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Give the window a moment to come up before touching the
                // network. 5s keeps startup snappy on a slow connection.
                tokio::time::sleep(Duration::from_secs(5)).await;

                let updater = match updater_app.updater() {
                    Ok(updater) => updater,
                    Err(err) => {
                        eprintln!("updater: not available: {err}");
                        return;
                    }
                };
                let update = match updater.check().await {
                    Ok(update) => update,
                    Err(err) => {
                        eprintln!("updater: check failed: {err}");
                        return;
                    }
                };
                let Some(update) = update else {
                    return;
                };
                eprintln!("updater: found version {}", update.version);

                if let Err(err) = update.download_and_install(|_, _| {}, || {}).await {
                    eprintln!("updater: install failed: {err}");
                    return;
                }

                let restart = updater_app
                    .dialog()
                    .message(format!(
                        "VisualLLM {} is installed.\nRestart now to apply the update?",
                        update.version
                    ))
                    .title("Update ready")
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Restart".to_string(),
                        "Later".to_string(),
                    ))
                    .blocking_show();
                if restart {
                    updater_app.restart();
                }
            });

            Ok(())
        })
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            lane_unpark,
            incidents_read,
            pool_read,
            pool_write,
            activity_read,
            stats_read,
            stats_refresh,
            lane_test,
            port_get,
            port_set,
            gateway_token,
            gateway_token_regenerate,
            editor_list,
            editor_integrate_lane,
            editor_remove_lane,
            state_export,
            state_import
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

#[cfg(test)]
mod vscode_tests {
    use super::{vscode_merge_lane, vscode_write_models_at, VscodeChatModels, VscodeModelEntry};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn entry(id: &str) -> VscodeModelEntry {
        VscodeModelEntry {
            id: id.to_string(),
            name: format!("visualllm: {id}"),
            url: format!("http://127.0.0.1:4100/lane/{id}/v1"),
            tool_calling: true,
            vision: false,
            max_input_tokens: 250144,
            max_output_tokens: 8000,
            http_headers: None,
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("visualllm-vscode-test-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn merge_creates_the_visualllm_provider_when_absent() {
        let mut config: VscodeChatModels = serde_json::from_str(
            r#"[{"name":"Mistral","vendor":"customendpoint","models":[{"id":"m","name":"M","url":"u","toolCalling":true,"vision":false,"maxInputTokens":1000,"maxOutputTokens":1000}]}]"#,
        )
        .unwrap();
        vscode_merge_lane(&mut config, &entry("lane-a"));

        assert_eq!(config.len(), 2);
        let visualllm = config.iter().find(|p| p.name == "visualllm").unwrap();
        assert_eq!(visualllm.models.len(), 1);
        assert_eq!(visualllm.models[0].id, "lane-a");
    }

    #[test]
    fn merge_updates_by_slug_instead_of_duplicating() {
        let mut config: VscodeChatModels = serde_json::from_str(
            r#"[{"name":"visualllm","vendor":"customendpoint","models":[
                {"id":"old-lane","name":"Old","url":"u1","toolCalling":true,"vision":false,"maxInputTokens":1000,"maxOutputTokens":1000},
                {"id":"lane-a","name":"LaneA","url":"u0","toolCalling":true,"vision":false,"maxInputTokens":1000,"maxOutputTokens":1000}
            ]}]"#,
        )
        .unwrap();
        vscode_merge_lane(&mut config, &entry("lane-a"));

        let visualllm = config.iter().find(|p| p.name == "visualllm").unwrap();
        let ids: Vec<&str> = visualllm.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["lane-a", "old-lane"]);
        assert_eq!(
            visualllm.models[0].url,
            "http://127.0.0.1:4100/lane/lane-a/v1"
        );
    }

    #[test]
    fn write_round_trips_a_readable_config() {
        let dir = temp_dir();
        let path = dir.join("chatLanguageModels.json");
        let mut config: VscodeChatModels = Vec::new();
        vscode_merge_lane(&mut config, &entry("lane-a"));
        vscode_write_models_at(&path, &config).unwrap();

        let back: VscodeChatModels =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back[0].models[0].id, "lane-a");
        // No stale temp file left behind.
        assert!(!dir.join("chatLanguageModels.json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
