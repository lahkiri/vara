//! vara-core — the brain of Vara, independent of any UI framework.
//!
//! Everything here is pure logic + I/O against SQLite/HTTP so it can be
//! unit-tested headlessly (see `tests/`). The Tauri shell (`src-tauri`)
//! wires this into a desktop app: tray, window, notifications, events.

pub mod chat;
pub mod computer_use;
pub mod db;
pub mod dedup;
pub mod entity;
pub mod exec_policy;
pub mod heartbeat;
pub mod host;
pub mod llm;
pub mod profile;
pub mod provenance;
pub mod tool_loop;
pub mod tools;
pub mod tools_local;
pub mod tools_registry;
pub mod types;

pub use chat::{build_context, extract_mission_proposal, extract_sys_actions, persona_flavor};
pub use db::Database;
pub use entity::{EntityEvent, EntityRuntime, EventSink, MissionInputs, MissionOutcome};
pub use llm::LlmClient;
pub use types::{EntityState, Settings};

/// Load settings for a headless surface (the TUI, a daemon): the settings file
/// when present, then the `VARA_PROVIDER_*` environment overrides.
///
/// This is the same precedence the desktop shell uses (`env > file`), kept in
/// the core so every interface inherits one rule — a second implementation of
/// "where do credentials come from" is how a key ends up in two places.
pub fn settings_from_env(dir: &std::path::Path) -> Settings {
    let path = dir.join("settings.json");
    let mut settings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<Settings>(&body).ok())
        .unwrap_or_default();
    if let Ok(key) = std::env::var("VARA_PROVIDER_API_KEY") {
        if !key.trim().is_empty() {
            settings.provider.api_key = key.trim().to_string();
        }
    }
    if let Ok(base) = std::env::var("VARA_PROVIDER_BASE_URL") {
        if !base.trim().is_empty() {
            settings.provider.base_url = base.trim().to_string();
        }
    }
    if let Ok(model) = std::env::var("VARA_PROVIDER_MODEL") {
        if !model.trim().is_empty() {
            settings.provider.model = model.trim().to_string();
        }
    }
    settings
}

/// Canonical error type shared by core modules.
#[derive(Debug, thiserror::Error)]
pub enum VaraError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("llm: {0}")]
    Llm(String),
    #[error("json: {0}")]
    Json(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, VaraError>;
