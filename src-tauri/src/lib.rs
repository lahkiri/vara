//! Vara desktop shell — wires vara-core into a Tauri 2 app with tray,
//! notifications, autostart, single instance, and a folder watcher.

mod commands;
mod plugin_bridge;
mod settings;
mod tool_bridge;
mod tray;
mod watcher;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use vara_core::db::Database;
use vara_core::types::Settings;
use vara_core::{EntityEvent, EntityRuntime};

pub struct AppState {
    pub db: Arc<Database>,
    pub settings: RwLock<Settings>,
    pub paused: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
    pub busy: Arc<AtomicBool>,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// Per-conversation stop flags for streaming chat replies.
    pub chat_cancels: Mutex<HashMap<i64, Arc<AtomicBool>>>,
}

impl AppState {
    pub fn settings_snapshot(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

pub static HTTP_CLIENT: LazyLock<Arc<vara_core::tools::HttpClient>> = LazyLock::new(|| {
    Arc::new(vara_core::tools::HttpClient::new().expect("failed to build the HTTP client"))
});

static HEARTBEAT_SPAWNED: AtomicBool = AtomicBool::new(false);
static FIRST_CLOSE_NOTIFIED: AtomicBool = AtomicBool::new(false);

/// Forwards core events to the webview.
pub struct TauriSink {
    pub app: AppHandle,
}

impl vara_core::EventSink for TauriSink {
    fn emit(&self, ev: EntityEvent) {
        let _ = self.app.emit("entity://event", ev);
    }
}

pub fn notify_user(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let enabled = app
        .try_state::<AppState>()
        .map(|s| s.settings_snapshot().autonomy.notifications_enabled)
        .unwrap_or(true);
    if !enabled {
        return;
    }
    let _ = app.notification().builder().title(title).body(body).show();
}

pub fn run() {
    // A development build must not share the installed app's WebView2 profile.
    //
    // Both binaries carry the same bundle identifier (`app.vara.entity`), so
    // WebView2 puts both in one user-data folder — and it only permits one
    // process at a time. Running `tauri dev` while the installed app is open
    // (or after it has been open) made window creation fail with
    // `0x800700AA ERROR_BUSY`, which looked like a broken runtime but was
    // really two apps fighting over one profile.
    //
    // Giving the dev build its own folder is also correct on its own terms: a
    // development instance must never touch the owner's real conversations and
    // memory, which is what sharing the profile would mean.
    if cfg!(debug_assertions) {
        if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none() {
            if let Ok(base) = std::env::var("LOCALAPPDATA") {
                let dir = std::path::Path::new(&base)
                    .join("app.vara.entity.dev")
                    .join("EBWebView");
                if std::fs::create_dir_all(&dir).is_ok() {
                    std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &dir);
                }
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap,
            commands::save_settings,
            commands::test_provider,
            commands::create_and_start_mission,
            commands::start_mission_in_conversation,
            commands::sys_execute,
            commands::sys_approve,
            commands::list_action_proposals,
            commands::list_skills,
            plugin_bridge::list_plugins,
            plugin_bridge::set_plugin_enabled,
            plugin_bridge::approve_plugin,
            plugin_bridge::reveal_plugin,
            plugin_bridge::open_user_plugin_dir,
            commands::pause_entity,
            commands::cancel_mission,
            commands::get_entity_status,
            commands::list_missions,
            commands::get_mission_detail,
            commands::list_reports,
            commands::get_report,
            commands::list_notes,
            commands::add_manual_note,
            commands::delete_note,
            commands::list_events,
            commands::sys_open,
            commands::export_report,
            commands::show_window,
            commands::create_conversation,
            commands::list_conversations,
            commands::rename_conversation,
            commands::delete_conversation,
            commands::get_messages,
            commands::start_report_discussion,
            commands::send_chat,
            commands::stop_chat,
            commands::check_for_update,
            commands::install_update,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = handle
                .path()
                .app_data_dir()
                .map_err(|e| format!("cannot resolve data dir: {e}"))?;
            std::fs::create_dir_all(&data_dir).ok();

            let db_path = data_dir.join("vara.db");
            let db = Arc::new(
                Database::open(&db_path).map_err(|e| format!("cannot open database: {e}"))?,
            );

            // Reconcile durable state with reality before anything reads it.
            //
            // No mission can be `running` at this instant: the process that was
            // executing it no longer exists. Leaving those rows as they were is
            // what produced the v0.7.0 zombie — a mission shown as in progress
            // that the cancel button refused (its guard is `busy`, which is
            // false) and that nothing ever moved. This is the same reconciliation
            // the approval queue already does for expired proposals.
            match db.recover_interrupted_missions(
                "the app stopped while this mission was running — it can be started again",
            ) {
                Ok(n) if n > 0 => {
                    let _ = db.insert_event(
                        "warn",
                        "recovery",
                        &format!("{n} mission(s) were interrupted by a restart"),
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = db.insert_event("warn", "recovery", &format!("recovery failed: {e}"));
                }
            }
            let cfg = settings::load(&data_dir);
            app.manage(AppState {
                db: db.clone(),
                settings: RwLock::new(cfg.clone()),
                paused: Arc::new(AtomicBool::new(false)),
                cancel: Arc::new(AtomicBool::new(false)),
                busy: Arc::new(AtomicBool::new(false)),
                watcher: Mutex::new(None),
                chat_cancels: Mutex::new(HashMap::new()),
            });

            tray::create(&handle).map_err(|e| e.to_string())?;

            if let Some(folder) = cfg.watched_folder.clone() {
                if !folder.trim().is_empty() {
                    if let Err(e) = watcher::start(&handle, std::path::PathBuf::from(&folder)) {
                        let _ = db.insert_event("warn", "watcher", &e);
                    }
                }
            }

            if cfg.autonomy.heartbeat_enabled && !cfg.provider.base_url.trim().is_empty() {
                spawn_heartbeat(handle.clone());
            }

            let _ = db.insert_event(
                "info",
                "app",
                &format!("Vara started — data dir: {}", data_dir.display()),
            );
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle().clone();
                let close_to_tray = app
                    .try_state::<AppState>()
                    .map(|s| s.settings_snapshot().autonomy.close_to_tray)
                    .unwrap_or(true);
                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                    if !FIRST_CLOSE_NOTIFIED.swap(true, Ordering::SeqCst) {
                        notify_user(
                            &app,
                            "Vara",
                            "Vara keeps working from the tray. Right-click the tray icon to quit.",
                        );
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running vara");
}

fn spawn_heartbeat(app: AppHandle) {
    if HEARTBEAT_SPAWNED.swap(true, Ordering::SeqCst) {
        return;
    }
    // The latch must mean "a loop is alive", not "a loop was started once".
    // The loop returns permanently when the owner disables the heartbeat, but
    // nothing cleared the flag — so ON → OFF → ON in one session never restarted
    // it, and the toggle read "on" while the entity never reflected again. This
    // guard clears the flag on every exit path, including the early returns,
    // which is the same drop-based pattern `BusyGuard` already uses.
    struct HeartbeatLatch;
    impl Drop for HeartbeatLatch {
        fn drop(&mut self) {
            HEARTBEAT_SPAWNED.store(false, Ordering::SeqCst);
        }
    }
    let _latch = HeartbeatLatch;
    tauri::async_runtime::spawn(async move {
        let _latch = _latch;
        loop {
            let (minutes, enabled) = {
                match app.try_state::<AppState>() {
                    Some(st) => {
                        let s = st.settings_snapshot();
                        (s.autonomy.heartbeat_minutes, s.autonomy.heartbeat_enabled)
                    }
                    None => return,
                }
            };
            if !enabled {
                return; // owner turned the heartbeat off — end the loop
            }
            tokio::time::sleep(std::time::Duration::from_secs(minutes.max(5) * 60)).await;
            let st = match app.try_state::<AppState>() {
                Some(st) => st,
                None => return,
            };
            if st.busy.load(Ordering::SeqCst) {
                continue; // never interrupt a mission
            }
            let snap = st.settings_snapshot();
            if snap.provider.base_url.trim().is_empty() {
                continue;
            }
            let runtime = EntityRuntime {
                db: st.db.clone(),
                sink: Arc::new(TauriSink { app: app.clone() }),
                http: HTTP_CLIENT.clone(),
            };
            if let Ok(llm) = vara_core::LlmClient::new(&snap.provider) {
                let _ = runtime.heartbeat(&llm, &snap.language).await;
            }
        }
    });
}

/// Re-evaluate background services after settings changes.
pub fn apply_settings_side_effects(app: &AppHandle) {
    let st = match app.try_state::<AppState>() {
        Some(st) => st,
        None => return,
    };
    let snap = st.settings_snapshot();

    // Watcher: restart only if the folder changed.
    {
        let current = st
            .watcher
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        let want = snap
            .watched_folder
            .as_ref()
            .map(|f| !f.trim().is_empty())
            .unwrap_or(false);
        if want {
            watcher::start(
                app,
                std::path::PathBuf::from(snap.watched_folder.clone().unwrap_or_default()),
            )
            .unwrap_or_else(|e| {
                let _ = st.db.insert_event("warn", "watcher", &e);
            });
        } else if current {
            *st.watcher.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }

    // Autostart.
    {
        use tauri_plugin_autostart::ManagerExt;
        let mgr = app.autolaunch();
        if snap.autonomy.autostart {
            let _ = mgr.enable();
        } else {
            let _ = mgr.disable();
        }
    }

    // Heartbeat.
    if snap.autonomy.heartbeat_enabled && !snap.provider.base_url.trim().is_empty() {
        spawn_heartbeat(app.clone());
    }
}
