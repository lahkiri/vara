//! vara-core — the brain of Vara, independent of any UI framework.
//!
//! Everything here is pure logic + I/O against SQLite/HTTP so it can be
//! unit-tested headlessly (see `tests/`). The Tauri shell (`src-tauri`)
//! wires this into a desktop app: tray, window, notifications, events.

pub mod db;
pub mod dedup;
pub mod entity;
pub mod llm;
pub mod provenance;
pub mod tools;
pub mod types;

pub use db::Database;
pub use entity::{EntityEvent, EntityRuntime, EventSink, MissionInputs, MissionOutcome};
pub use llm::LlmClient;
pub use types::{EntityState, Settings};

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
