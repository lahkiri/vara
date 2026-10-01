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
    // env > file: a VARA_PROVIDER_* injected key survives UI saves and never
    // gets persisted into settings.json (secrets discipline).
    crate::settings::apply_env_overrides(&mut s);

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

/// Bilingual closing message appended to the thread when a mission ends —
/// the thread stays the single source of truth (chat-first, end to end).
fn mission_closing_text(
    status: &str,
    verdict: Option<&String>,
    backed: Option<f64>,
    error: Option<&str>,
    lang: &str,
) -> String {
    let arabic = lang == "ar" || lang == "arabic";
    match status {
        "completed" => {
            let pct = backed.map(|r| (r * 100.0) as i64).unwrap_or(0);
            if arabic {
                format!("أنهيت المهمة ✅ — التقرير جاهز (توثيق مصادره {pct}%). التقرير مربوط بهذه المحادثة الآن: اسألني عن أي تفصيل فيه وسأجيب منه مباشرة.")
            } else {
                let v = verdict.unwrap_or(&String::new()).clone();
                format!("Mission complete ✅ — report ready (provenance {v}, {pct}% backed). It is attached to this thread: ask me about any detail and I will answer from it.")
            }
        }
        "cancelled" => {
            if arabic {
                "أوقفتُ المهمة كما طلبت. يمكنني إعادة تشغيلها أو تعديل الهدف متى شئت.".into()
            } else {
                "Mission cancelled as you asked. I can restart or reshape it anytime.".into()
            }
        }
        _ => {
            let e = error.unwrap_or("unknown error");
            if arabic {
                format!("توقفت المهمة دون تقرير: {e}. قل لي كيف نعيد المحاولة وسأعدّل الخطة.")
            } else {
                format!("The mission stopped without a report: {e}. Tell me how to retry and I will adjust the plan.")
            }
        }
    }
}

/// Creates a mission from inside a chat thread, links the thread to it,
/// inserts a live mission card row, and spawns the runner whose progress
/// and final report flow back into the same thread.
pub fn launch_thread_mission(
    app: &AppHandle,
    conversation_id: i64,
    goal: &str,
) -> Result<i64, String> {
    let state = app.state::<AppState>();
    let goal = goal.trim().to_string();
    if goal.is_empty() {
        return Err("the goal is empty — give Vara a mission".into());
    }
    if state.busy.load(Ordering::SeqCst) {
        return Err("Vara is already working on a mission".into());
    }
    let snapshot = state.settings_snapshot();
    if snapshot.provider.base_url.trim().is_empty() {
        return Err("no model provider configured — open Settings first".into());
    }
    if state.db.get_conversation(conversation_id).is_err() {
        return Err("conversation not found".into());
    }

    let budget = snapshot
        .mission_defaults
        .budget_tokens
        .clamp(3_000, 300_000);
    let steps = snapshot.mission_defaults.max_steps.clamp(4, 40);
    let id = state
        .db
        .create_mission(&goal, budget, steps)
        .map_err(|e| e.to_string())?;
    let _ = state.db.link_conversation_mission(conversation_id, id);

    // the live card inside the thread
    let card = serde_json::json!({ "goal": goal, "mission_id": id });
    let card_id = state
        .db
        .insert_chat_message_typed(
            conversation_id,
            "assistant",
            &card.to_string(),
            None,
            0,
            "ok",
            "mission",
            Some(id),
        )
        .map_err(|e| e.to_string())?;
    if let Ok(rec) = state.db.get_chat_message(card_id) {
        let _ = app.emit("entity://chat/message", &rec);
    }

    spawn_mission_runner(app.clone(), id, Some(conversation_id));
    Ok(id)
}

/// Shared runner for every mission (legacy launcher and chat-born ones).
/// Progress streams through the entity event bus; the outcome lands back
/// inside the originating thread when there is one.
fn spawn_mission_runner(app: AppHandle, mission_id: i64, conversation_id: Option<i64>) {
    let Some(st) = app.try_state::<AppState>() else {
        return;
    };
    let snapshot = st.settings_snapshot();
    let language = snapshot.language.clone();
    let llm = match LlmClient::new(&snapshot.provider) {
        Ok(l) => l,
        Err(e) => {
            let _ = st
                .db
                .update_mission_status(mission_id, "failed", Some(&e.to_string()));
            return;
        }
    };

    st.busy.store(true, Ordering::SeqCst);
    st.cancel.store(false, Ordering::SeqCst);
    st.paused.store(false, Ordering::SeqCst);

    let runtime = EntityRuntime {
        db: st.db.clone(),
        sink: Arc::new(TauriSink { app: app.clone() }),
        http: HTTP_CLIENT.clone(),
    };
    let inputs = MissionInputs {
        language: language.clone(),
        paused: st.paused.clone(),
        cancel: st.cancel.clone(),
    };
    let busy_flag = st.busy.clone();
    let db = st.db.clone();
    let app2 = app.clone();

    tauri::async_runtime::spawn(async move {
        let outcome = runtime.run_mission(mission_id, Arc::new(llm), inputs).await;
        busy_flag.store(false, Ordering::SeqCst);

        if let Some(conv) = conversation_id {
            let (status, verdict, backed) = match &outcome {
                Ok(o) => (o.status.clone(), o.verdict.clone(), o.backed_ratio),
                Err(_) => ("failed".into(), None, None),
            };
            // the honest closing text carries the REAL failure reason
            let error_text = match &outcome {
                Ok(_) => db
                    .get_mission(mission_id)
                    .ok()
                    .and_then(|m| m.error)
                    .unwrap_or_else(|| status.clone()),
                Err(e) => e.to_string(),
            };
            let error_ref = if status == "failed" {
                Some(error_text.as_str())
            } else {
                None
            };
            let text =
                mission_closing_text(&status, verdict.as_ref(), backed, error_ref, &language);
            if let Ok(mid) = db.insert_chat_message(conv, "assistant", &text, None, 0, "ok") {
                if let Ok(rec) = db.get_chat_message(mid) {
                    let _ = app2.emit("entity://chat/message", &rec);
                }
            }
        }

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
                let _ = db.update_mission_status(mission_id, "failed", Some(&e.to_string()));
                notify_user(&app2, "Vara", &format!("Mission failed: {e}"));
            }
        }
    });
}

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
    spawn_mission_runner(app, id, None);
    Ok(id)
}

/// The chat-first entry: start a mission that LIVES in this thread.
#[tauri::command]
pub fn start_mission_in_conversation(
    app: AppHandle,
    conversation_id: i64,
    goal: String,
) -> Result<i64, String> {
    launch_thread_mission(&app, conversation_id, &goal)
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

// ---------- chat (talk with the entity) ----------

#[derive(serde::Serialize)]
pub struct SendChatStart {
    pub conversation_id: i64,
    pub user_message_id: i64,
    pub assistant_message_id: i64,
}

fn estimate_tokens(s: &str) -> i64 {
    (s.chars().count() as i64 / 4).max(1)
}

#[tauri::command]
pub fn create_conversation(
    state: State<'_, AppState>,
    title: Option<String>,
    mission_id: Option<i64>,
) -> Result<Conversation, String> {
    let t = title.unwrap_or_default();
    let id = state
        .db
        .create_conversation(if t.trim().is_empty() { "" } else { &t }, mission_id)
        .map_err(|e| e.to_string())?;
    state.db.get_conversation(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_conversations(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<Conversation>, String> {
    state
        .db
        .list_conversations(limit.unwrap_or(80))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_conversation(
    state: State<'_, AppState>,
    id: i64,
    title: String,
) -> Result<(), String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("empty title".into());
    }
    state
        .db
        .rename_conversation(id, &title)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_conversation(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    // also drop any cancel flag for a stream that may still be running
    state
        .chat_cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id);
    state.db.delete_conversation(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_messages(
    state: State<'_, AppState>,
    conversation_id: i64,
    limit: Option<i64>,
) -> Result<Vec<ChatMessageRecord>, String> {
    state
        .db
        .list_chat_messages(conversation_id, limit.unwrap_or(300))
        .map_err(|e| e.to_string())
}

/// Opens a conversation grounded in a report — the "follow up on the findings"
/// entry point from Missions/Reports views. Follow-up messages now continue
/// the thread with the full report in context.
#[tauri::command]
pub fn start_report_discussion(
    state: State<'_, AppState>,
    report_id: i64,
) -> Result<Conversation, String> {
    let report = state.db.get_report(report_id).map_err(|e| e.to_string())?;
    let goal = state
        .db
        .get_mission(report.mission_id)
        .map(|m| m.goal)
        .unwrap_or_else(|_| "mission".into());
    let title = format!(
        "مناقشة التقرير #{report_id} — {}",
        goal.chars().take(40).collect::<String>()
    );
    let id = state
        .db
        .create_conversation(&title, Some(report.mission_id))
        .map_err(|e| e.to_string())?;
    state.db.get_conversation(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn send_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: i64,
    content: String,
) -> Result<SendChatStart, String> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("the message is empty".into());
    }
    let snapshot = state.settings_snapshot();
    if snapshot.provider.base_url.trim().is_empty() {
        return Err("no model provider configured — open Settings first".into());
    }
    // one stream per conversation; other conversations can chat in parallel
    {
        let map = state.chat_cancels.lock().unwrap_or_else(|p| p.into_inner());
        if map.contains_key(&conversation_id) {
            return Err("Vara is still writing in this conversation".into());
        }
    }
    let conversation = state
        .db
        .get_conversation(conversation_id)
        .map_err(|e| e.to_string())?;

    // store the user's message first (history already contains it when the
    // context is built), then auto-title the thread from the first message
    let user_id = state
        .db
        .insert_chat_message(
            conversation_id,
            "user",
            &content,
            None,
            estimate_tokens(&content),
            "ok",
        )
        .map_err(|e| e.to_string())?;
    if conversation.title.trim().is_empty() {
        let title: String = content.chars().take(48).collect();
        let _ = state.db.rename_conversation(conversation_id, &title);
    }

    // placeholder assistant message that streams into
    let assistant_id = state
        .db
        .insert_chat_message(conversation_id, "assistant", "", None, 0, "streaming")
        .map_err(|e| e.to_string())?;

    let llm = LlmClient::new(&snapshot.provider).map_err(|e| e.to_string())?;
    let msgs = vara_core::build_context(&state.db, &conversation, &snapshot, &content)
        .map_err(|e| e.to_string())?;

    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    state
        .chat_cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(conversation_id, cancel.clone());

    // the entity visibly shifts into deliberation while thinking
    let _ = app.emit(
        "entity://event",
        serde_json::json!({ "type": "state", "state": "deliberating", "mission_id": null }),
    );

    let db = state.db.clone();
    let app2 = app.clone();
    let cancel2 = cancel.clone();

    tauri::async_runtime::spawn(async move {
        let mut acc = String::new();
        let emit_app = app2.clone();
        let result = llm
            .chat_stream(&msgs, None, cancel2, |delta| {
                acc.push_str(delta);
                let _ = emit_app.emit(
                    "entity://chat/delta",
                    serde_json::json!({
                        "conversation_id": conversation_id,
                        "message_id": assistant_id,
                        "delta": delta,
                    }),
                );
            })
            .await;

        let stopped = cancel.load(Ordering::SeqCst);
        // remove the stream handle before emitting the terminal event
        let _ = app2.try_state::<AppState>().map(|st| {
            st.chat_cancels
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&conversation_id)
        });

        match result {
            Ok(reply) => {
                // keep the raw markers in storage so the proposal survives reloads,
                // but never leak them to the UI: extract tolerantly (models mangle
                // "[[mission]]…[[/mission]]" into "[mission]…{MISSION_CLOSE}" etc.)
                let status = if stopped { "stopped" } else { "ok" };
                let tokens = (reply.prompt_tokens + reply.completion_tokens) as i64;
                let _ = db.update_chat_message(
                    assistant_id,
                    &reply.content,
                    tokens,
                    Some(&reply.model),
                    status,
                );
                let (clean_after_mission, goal) =
                    vara_core::extract_mission_proposal(&reply.content);
                let (clean, sys_actions) = vara_core::extract_sys_actions(&clean_after_mission);

                // chat-first autonomy: proposed missions start on their own
                // (budget-capped, read-only) unless the owner is busy or disabled it
                let mut started_mission: Option<i64> = None;
                if let Some(g) = goal.clone() {
                    if !stopped {
                        let should = app2.try_state::<AppState>().map_or(false, |st| {
                            st.settings_snapshot().autonomy.auto_start_missions
                                && !st.busy.load(Ordering::SeqCst)
                        });
                        if should {
                            started_mission =
                                launch_thread_mission(&app2, conversation_id, &g).ok();
                        }
                    }
                }

                let _ = app2.emit(
                    "entity://chat/done",
                    serde_json::json!({
                        "conversation_id": conversation_id,
                        "message_id": assistant_id,
                        "content": clean,
                        "tokens": tokens,
                        "model": reply.model,
                        "status": status,
                        "mission_goal": goal,
                        "mission_started": started_mission,
                        "sys_actions": sys_actions,
                        "error": null,
                    }),
                );
                settle_state(&app2);
            }
            Err(e) => {
                let msg = if stopped {
                    // user stopped early — keep whatever streamed in
                    let _ = db.update_chat_message(
                        assistant_id,
                        &acc,
                        estimate_tokens(&acc),
                        None,
                        "stopped",
                    );
                    let _ = app2.emit(
                        "entity://chat/done",
                        serde_json::json!({
                            "conversation_id": conversation_id,
                            "message_id": assistant_id,
                            "content": acc,
                            "tokens": 0,
                            "model": null,
                            "status": "stopped",
                            "mission_goal": null,
                            "mission_started": null,
                            "sys_actions": [],
                            "error": null,
                        }),
                    );
                    settle_state(&app2);
                    return;
                } else {
                    format!("{e}")
                };
                let _ = db.update_chat_message(assistant_id, &msg, 0, None, "error");
                let _ = app2.emit(
                    "entity://chat/done",
                    serde_json::json!({
                        "conversation_id": conversation_id,
                        "message_id": assistant_id,
                        "content": msg,
                        "tokens": 0,
                        "model": null,
                        "status": "error",
                        "mission_goal": null,
                        "mission_started": null,
                        "sys_actions": [],
                        "error": msg,
                    }),
                );
                settle_state(&app2);
            }
        }
    });

    Ok(SendChatStart {
        conversation_id,
        user_message_id: user_id,
        assistant_message_id: assistant_id,
    })
}

/// Return the entity to the attentive state when nothing else is running —
/// keeps the avatar and the tray truthful after every chat turn.
fn settle_state(app: &AppHandle) {
    let busy = app
        .try_state::<AppState>()
        .map(|st| st.busy.load(Ordering::SeqCst))
        .unwrap_or(false);
    if !busy {
        let _ = app.emit(
            "entity://event",
            serde_json::json!({ "type": "state", "state": "attentive", "mission_id": null }),
        );
    }
}

#[tauri::command]
pub fn stop_chat(state: State<'_, AppState>, conversation_id: i64) -> Result<(), String> {
    if let Some(c) = state
        .chat_cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&conversation_id)
    {
        c.store(true, Ordering::SeqCst);
    }
    Ok(())
}

// ---------- OS actions (the entity controls the machine, gated by policy) ----------

#[derive(serde::Serialize)]
pub struct SysExecuteResult {
    pub ok: bool,
    pub action: String,
    pub target: String,
    pub output: String,
    pub error: String,
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Executes one shell command with a hard timeout, confined to the owner's
/// home (or the watched folder when set). Output is capped — receipts, not dumps.
async fn run_shell_command(target: &str, cwd: Option<String>) -> (bool, String, String) {
    use tokio::process::Command;
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(target);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(target);
        c
    };
    if let Some(dir) = cwd {
        if std::path::Path::new(&dir).is_dir() {
            cmd.current_dir(dir);
        }
    }
    cmd.kill_on_drop(true);
    match tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output()).await {
        Err(_) => (false, String::new(), "timed out after 60s".into()),
        Ok(Err(e)) => (false, String::new(), e.to_string()),
        Ok(Ok(out)) => {
            let mut text = String::from_utf8_lossy(&out.stdout).to_string();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                text.push_str("\n[stderr] ");
                text.push_str(&err);
            }
            let ok = out.status.success();
            let error = if ok {
                String::new()
            } else {
                format!("exit code: {:?}", out.status.code())
            };
            (ok, clip(&text, 4000), error)
        }
    }
}

/// Captures the full screen into the app-data `screenshots` folder using the
/// OS's own tooling — no extra native dependencies, no driver installs:
/// Windows: PowerShell + System.Drawing (built into every Windows 10/11),
/// macOS: `screencapture`, Linux: gnome-screenshot / ImageMagick import / scrot.
/// Returns `(ok, output, error)` where output carries the saved file path.
async fn capture_screen(app: &AppHandle) -> (bool, String, String) {
    use tokio::process::Command;

    let dir = match app.path().app_data_dir().map(|d| d.join("screenshots")) {
        Ok(d) => d,
        Err(e) => return (false, String::new(), format!("data dir: {e}")),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (false, String::new(), format!("mkdir: {e}"));
    }
    let stamp = chrono::Local::now().format("Vara-%Y%m%d-%H%M%S");
    let path = dir.join(format!("{stamp}.png"));
    let path_str = path.display().to_string();

    #[cfg(target_os = "windows")]
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
         $b=[System.Windows.Forms.SystemInformation]::VirtualScreen; \
         $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height; \
         $g=[System.Drawing.Graphics]::FromImage($bmp); \
         $g.CopyFromScreen($b.X,$b.Y,0,0,$bmp.Size); \
         $bmp.Save('{path_str}'); $g.Dispose(); $bmp.Dispose()"
    );

    let result = if cfg!(target_os = "windows") {
        #[cfg(target_os = "windows")]
        {
            Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .kill_on_drop(true)
                .output()
                .await
        }
        #[cfg(not(target_os = "windows"))]
        {
            unreachable!()
        }
    } else if cfg!(target_os = "macos") {
        Command::new("screencapture")
            .args(["-x", &path_str])
            .kill_on_drop(true)
            .output()
            .await
    } else {
        // Linux: try the common capture tools in order.
        let mut last_err = String::from("no screen capture tool found");
        let mut out = None;
        for (prog, args) in [
            ("gnome-screenshot", vec!["-f".to_string(), path_str.clone()]),
            (
                "import",
                vec!["-window".to_string(), "root".to_string(), path_str.clone()],
            ),
            ("scrot", vec![path_str.clone()]),
        ] {
            match Command::new(prog)
                .args(&args)
                .kill_on_drop(true)
                .output()
                .await
            {
                Ok(o) if o.status.success() => {
                    out = Some(Ok(o));
                    break;
                }
                Ok(o) => {
                    last_err = format!(
                        "{prog} failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    )
                }
                Err(e) => last_err = format!("{prog}: {e}"),
            }
        }
        match out {
            Some(r) => r,
            None => return (false, String::new(), last_err),
        }
    };

    match result {
        Err(e) => (false, String::new(), e.to_string()),
        Ok(o) if !o.status.success() => (
            false,
            String::new(),
            format!(
                "capture failed: {}",
                clip(String::from_utf8_lossy(&o.stderr).trim(), 300)
            ),
        ),
        Ok(_) => {
            if path.exists() {
                (true, path_str, String::new())
            } else {
                (false, String::new(), "capture produced no file".into())
            }
        }
    }
}

/// Runs one OS action Vara proposed in the chat. The autonomy policy is
/// enforced HERE, in the shell — the model never executes anything itself.
/// Every receipt lands in the thread as an action card.
#[tauri::command]
pub async fn sys_execute(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: i64,
    action: String,
    target: String,
) -> Result<SysExecuteResult, String> {
    let snapshot = state.settings_snapshot();
    let target = target.trim().to_string();
    if target.is_empty() {
        return Err("empty target".into());
    }

    let (ok, output, error) = match action.as_str() {
        "open_url" => {
            if !target.starts_with("http://") && !target.starts_with("https://") {
                (false, String::new(), "not an http(s) URL".into())
            } else if !snapshot.autonomy.open_urls {
                (
                    false,
                    String::new(),
                    "opening URLs is disabled in Settings".into(),
                )
            } else {
                match open::that(&target) {
                    Ok(_) => (true, String::new(), String::new()),
                    Err(e) => (false, String::new(), e.to_string()),
                }
            }
        }
        "open_path" => {
            if !snapshot.autonomy.open_paths {
                (
                    false,
                    String::new(),
                    "opening paths is disabled in Settings".into(),
                )
            } else if !std::path::Path::new(&target).exists() {
                (false, String::new(), "path does not exist".into())
            } else {
                match open::that(&target) {
                    Ok(_) => (true, String::new(), String::new()),
                    Err(e) => (false, String::new(), e.to_string()),
                }
            }
        }
        "run" => {
            if !snapshot.autonomy.run_commands {
                (
                    false,
                    String::new(),
                    "running commands is disabled in Settings".into(),
                )
            } else {
                let cwd = snapshot
                    .watched_folder
                    .clone()
                    .filter(|f| !f.trim().is_empty())
                    .or_else(|| {
                        std::env::var("USERPROFILE")
                            .or_else(|_| std::env::var("HOME"))
                            .ok()
                    });
                run_shell_command(&target, cwd).await
            }
        }
        "screenshot" => {
            // Privacy-sensitive: ships OFF, and even when enabled the chat
            // shows the explicit approval card before this arm is reached.
            if !snapshot.autonomy.allow_screenshots {
                (
                    false,
                    String::new(),
                    "screen capture is disabled in Settings".into(),
                )
            } else {
                capture_screen(&app).await
            }
        }
        _ => (false, String::new(), format!("unknown action: {action}")),
    };

    // the thread keeps the receipt — every OS action is visible in the chat
    let receipt = serde_json::json!({
        "action": action,
        "target": target,
        "ok": ok,
        "output": clip(&output, 2000),
        "error": error,
    });
    if let Ok(mid) = state.db.insert_chat_message_typed(
        conversation_id,
        "assistant",
        &receipt.to_string(),
        None,
        0,
        "ok",
        "action",
        None,
    ) {
        if let Ok(rec) = state.db.get_chat_message(mid) {
            let _ = app.emit("entity://chat/message", &rec);
        }
    }
    let _ = state.db.insert_event(
        if ok { "info" } else { "warn" },
        "sys_action",
        &format!("{action}: {}", clip(&target, 120)),
    );

    Ok(SysExecuteResult {
        ok,
        action,
        target,
        output,
        error,
    })
}

// ---------- updates (over-the-air) ----------

#[derive(serde::Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: String,
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(u) => Ok(Some(UpdateInfo {
            version: u.version.clone(),
            current_version: u.current_version.clone(),
            notes: u.body.clone().unwrap_or_default(),
        })),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let progress_app = app.clone();
    let mut downloaded: u64 = 0;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit(
                    "entity://update/progress",
                    serde_json::json!({ "downloaded": downloaded, "total": total }),
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    notify_user(&app, "Vara", "Update installed — restarting now.");
    app.restart(); // never returns
}
