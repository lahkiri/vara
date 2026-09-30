//! All Tauri commands exposed to the webview. Thin layer: validate, call
//! vara-core, map errors to strings.

use crate::{notify_user, settings, tray, AppState, TauriSink, HTTP_CLIENT};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use vara_core::types::*;
use vara_core::{EntityRuntime, LlmClient, MissionInputs};

#[derive(serde::Serialize)]
pub struct EntityStatus {
    pub busy: bool,
    pub paused: bool,
    pub active_mission: Option<Mission>,
    pub stats: Stats,
}

#[derive(serde::Serialize)]
pub struct TestProviderResult {
    pub ok: bool,
    pub latency_ms: u128,
    pub reply: String,
    pub error: String,
}

// ---------- bootstrap / status ----------

#[tauri::command]
pub fn get_bootstrap(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let stats = state.db.stats().map_err(|e| e.to_string())?;
    let active = state.db.active_mission().unwrap_or(None);
    Ok(serde_json::json!({
        "settings": state.settings_snapshot(),
        "status": {
            "busy": state.busy.load(Ordering::SeqCst),
            "paused": state.paused.load(Ordering::SeqCst),
            "active_mission": active,
            "stats": stats,
        },
        "fts_enabled": state.db.fts_enabled(),
        "data_dir": data_dir,
    }))
}

#[tauri::command]
pub fn get_entity_status(state: State<'_, AppState>) -> Result<EntityStatus, String> {
    let stats = state.db.stats().map_err(|e| e.to_string())?;
    let active = state.db.active_mission().unwrap_or(None);
    Ok(EntityStatus {
        busy: state.busy.load(Ordering::SeqCst),
        paused: state.paused.load(Ordering::SeqCst),
        active_mission: active,
        stats,
    })
}

// ---------- settings ----------

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    new_settings: Settings,
) -> Result<(), String> {
    let mut s = state.settings_snapshot();
    let base = new_settings.provider.base_url.trim().to_string();
    if !base.is_empty() && !base.starts_with("http://") && !base.starts_with("https://") {
        return Err("provider URL must start with http:// or https://".into());
    }
    s.language = if new_settings.language == "ar" {
        "ar".into()
    } else {
        "en".into()
    };
    s.persona_style = new_settings.persona_style;
    s.provider = ProviderConfig {
        base_url: base,
        ..new_settings.provider
    };
    s.autonomy = new_settings.autonomy;
    s.mission_defaults = new_settings.mission_defaults;
    s.watched_folder = new_settings.watched_folder;

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("data dir: {e}"))?;
    settings::save(&data_dir, &s)?;
    *state.settings.write().unwrap_or_else(|p| p.into_inner()) = s.clone();
    crate::apply_settings_side_effects(&app);
    let _ = state
        .db
        .insert_event("info", "settings", "settings updated");
    Ok(())
}

#[tauri::command]
pub fn test_provider(provider: ProviderConfig) -> TestProviderResult {
    match LlmClient::new(&provider) {
        Ok(client) => match futures_block(client.health_check()) {
            Ok((reply, latency)) => TestProviderResult {
                ok: true,
                latency_ms: latency,
                reply,
                error: String::new(),
            },
            Err(e) => TestProviderResult {
                ok: false,
                latency_ms: 0,
                reply: String::new(),
                error: e.to_string(),
            },
        },
        Err(e) => TestProviderResult {
            ok: false,
            latency_ms: 0,
            reply: String::new(),
            error: e.to_string(),
        },
    }
}

/// Run an async future to completion inside a sync command (tiny helper).
fn futures_block<F: std::future::Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

// ---------- missions ----------

#[tauri::command]
pub fn create_and_start_mission(
    app: AppHandle,
    state: State<'_, AppState>,
    goal: String,
    budget_tokens: i64,
    max_steps: i64,
) -> Result<i64, String> {
    let goal = goal.trim().to_string();
    if goal.is_empty() {
        return Err("the goal is empty — give Vara a mission".into());
    }
    if state.busy.load(Ordering::SeqCst) {
        return Err("Vara is already working on a mission".into());
    }
    if state
        .settings_snapshot()
        .provider
        .base_url
        .trim()
        .is_empty()
    {
        return Err("no model provider configured — open Settings first".into());
    }

    let budget = budget_tokens.clamp(3_000, 300_000);
    let steps = max_steps.clamp(4, 40);
    let id = state
        .db
        .create_mission(&goal, budget, steps)
        .map_err(|e| e.to_string())?;

    let provider = state.settings_snapshot().provider;
    let llm = match LlmClient::new(&provider) {
        Ok(l) => l,
        Err(e) => {
            let _ = state
                .db
                .update_mission_status(id, "failed", Some(&e.to_string()));
            return Err(e.to_string());
        }
    };

    state.busy.store(true, Ordering::SeqCst);
    state.cancel.store(false, Ordering::SeqCst);
    state.paused.store(false, Ordering::SeqCst);

    let runtime = EntityRuntime {
        db: state.db.clone(),
        sink: Arc::new(TauriSink { app: app.clone() }),
        http: HTTP_CLIENT.clone(),
    };
    let inputs = MissionInputs {
        language: state.settings_snapshot().language,
        paused: state.paused.clone(),
        cancel: state.cancel.clone(),
    };
    let busy_flag = state.busy.clone();
    let db = state.db.clone();
    let app2 = app.clone();

    tauri::async_runtime::spawn(async move {
        let outcome = runtime.run_mission(id, Arc::new(llm), inputs).await;
        busy_flag.store(false, Ordering::SeqCst);
        match outcome {
            Ok(o) => {
                let body = match (&o.verdict, o.backed_ratio) {
                    (Some(v), Some(r)) => format!(
                        "Mission {} — provenance {} ({}% backed)",
                        o.status,
                        v,
                        (r * 100.0) as i64
                    ),
                    _ => format!("Mission {}", o.status),
                };
                notify_user(&app2, "Vara", &body);
                let _ = app2.emit("entity://done", &o);
            }
            Err(e) => {
                let _ = db.update_mission_status(id, "failed", Some(&e.to_string()));
                notify_user(&app2, "Vara", &format!("Mission failed: {e}"));
            }
        }
    });

    Ok(id)
}

#[tauri::command]
pub fn pause_entity(state: State<'_, AppState>, paused: bool) -> Result<(), String> {
    state.paused.store(paused, Ordering::SeqCst);
    let _ = state
        .db
        .insert_event("info", "pause", if paused { "paused" } else { "resumed" });
    Ok(())
}

#[tauri::command]
pub fn cancel_mission(state: State<'_, AppState>) -> Result<(), String> {
    if !state.busy.load(Ordering::SeqCst) {
        return Err("no mission is running".into());
    }
    state.cancel.store(true, Ordering::SeqCst);
    let _ = state.db.insert_event("warn", "mission", "cancel requested");
    Ok(())
}

#[tauri::command]
pub fn list_missions(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<Mission>, String> {
    state
        .db
        .list_missions(limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_mission_detail(
    state: State<'_, AppState>,
    id: i64,
) -> Result<serde_json::Value, String> {
    let mission = state.db.get_mission(id).map_err(|e| e.to_string())?;
    let actions = state.db.list_actions(id, 200).map_err(|e| e.to_string())?;
    let sources = state.db.list_sources(id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "mission": mission, "actions": actions, "sources": sources }))
}

// ---------- reports ----------

#[tauri::command]
pub fn list_reports(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<ReportRecord>, String> {
    state
        .db
        .list_reports(limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_report(state: State<'_, AppState>, id: i64) -> Result<ReportRecord, String> {
    state.db.get_report(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_report(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<String, String> {
    let report = state.db.get_report(id).map_err(|e| e.to_string())?;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("data dir: {e}"))?
        .join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("vara_report_{id}.md"));
    std::fs::write(&file, &report.markdown).map_err(|e| e.to_string())?;
    Ok(file.display().to_string())
}

// ---------- memory ----------

#[tauri::command]
pub fn list_notes(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<Note>, String> {
    let limit = limit.unwrap_or(100);
    match query.as_deref() {
        Some(q) if !q.trim().is_empty() => state.db.search_notes(q.trim(), limit),
        _ => state.db.list_notes(limit, 0),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_manual_note(
    state: State<'_, AppState>,
    title: String,
    body: String,
) -> Result<i64, String> {
    let title = title.trim();
    let body = body.trim();
    if title.is_empty() && body.is_empty() {
        return Err("empty note".into());
    }
    let t = if title.is_empty() { "Note" } else { title };
    state
        .db
        .insert_note("manual", t, body, None, None, None)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_note(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.db.delete_note(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_events(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<EventRecord>, String> {
    state
        .db
        .list_events(limit.unwrap_or(150))
        .map_err(|e| e.to_string())
}

// ---------- system ----------

#[tauri::command]
pub fn sys_open(state: State<'_, AppState>, target: String) -> Result<(), String> {
    let autonomy = state.settings_snapshot().autonomy;
    if target.starts_with("http://") || target.starts_with("https://") {
        if !autonomy.open_urls {
            return Err("opening URLs is disabled in Settings".into());
        }
        open::that(&target).map_err(|e| format!("open: {e}"))
    } else {
        if !autonomy.open_paths {
            return Err("opening paths is disabled in Settings".into());
        }
        open::that(&target).map_err(|e| format!("open: {e}"))
    }
}

#[tauri::command]
pub fn show_window(app: AppHandle) {
    tray::show_main(&app);
}
