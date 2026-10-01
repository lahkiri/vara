//! Shared types across core + shell. All wire types are serde-friendly.

use serde::{Deserialize, Serialize};

/// Visible state of the entity (drives the avatar + tray tooltip).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntityState {
    Dormant,
    Attentive,
    Deliberating,
    Working,
    Reporting,
    Sleeping,
}

impl EntityState {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityState::Dormant => "dormant",
            EntityState::Attentive => "attentive",
            EntityState::Deliberating => "deliberating",
            EntityState::Working => "working",
            EntityState::Reporting => "reporting",
            EntityState::Sleeping => "sleeping",
        }
    }
}

/// A single memory record (research note, file observation, reflection, manual).
#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub id: i64,
    pub created_at: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub mission_id: Option<i64>,
    pub source_url: Option<String>,
    pub source_title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mission {
    pub id: i64,
    pub created_at: String,
    pub goal: String,
    pub status: String,
    pub budget_tokens: i64,
    pub spent_tokens: i64,
    pub max_steps: i64,
    pub steps_done: i64,
    pub dimensions: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRecord {
    pub id: i64,
    pub mission_id: i64,
    pub url: String,
    pub title: String,
    pub fetched: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionRecord {
    pub id: i64,
    pub mission_id: Option<i64>,
    pub ts: String,
    pub kind: String,
    pub ok: bool,
    pub summary: String,
}

/// One auditable line of the Action Journal: what the entity did, at what
/// grant level, with what evidence. The journal IS the audit log and the
/// memory of her deeds (Muse-style inspectability, on a real desktop).
#[derive(Debug, Clone, Serialize)]
pub struct CuJournalEntry {
    pub id: i64,
    pub ts: String,
    pub conversation_id: Option<i64>,
    pub seq_index: i64,
    pub op: String,
    pub grant_level: String,
    pub target: String,
    pub ok: bool,
    pub dry_run: bool,
    pub active: Option<String>,
    pub before_ref: Option<String>,
    pub after_ref: Option<String>,
    pub check_note: Option<String>,
    pub ms: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportRecord {
    pub id: i64,
    pub mission_id: i64,
    pub created_at: String,
    pub markdown: String,
    pub sources_json: Option<serde_json::Value>,
    pub check_json: Option<serde_json::Value>,
    pub backed_ratio: Option<f64>,
    pub verdict: Option<String>,
    pub repaired: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRecord {
    pub id: i64,
    pub ts: String,
    pub level: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Stats {
    pub missions_total: i64,
    pub missions_completed: i64,
    pub notes_total: i64,
    pub reports_total: i64,
    pub avg_backed_ratio: Option<f64>,
}

// ---------- Provenance checker output ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceMetrics {
    pub cited_total: usize,
    pub retrieved_total: usize,
    pub backed_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceChecks {
    pub c1_all_cited_in_retrieved: bool,
    pub c2_refs_resolve_to_source_list: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceSection {
    pub title: String,
    pub citations: usize,
    pub backed_citations: usize,
    pub effectively_uncovered: bool,
    pub no_citations: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceResult {
    pub verdict: String,
    pub rule: String,
    pub metrics: ProvenanceMetrics,
    pub cited_not_retrieved: Vec<String>,
    pub retrieved_not_cited: Vec<String>,
    pub unresolved_refs: Vec<i64>,
    pub sections: Vec<ProvenanceSection>,
    pub checks: ProvenanceChecks,
}

// ---------- Tools ----------

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageContent {
    pub url: String,
    pub title: String,
    pub text: String,
}

// ---------- LLM ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

// ---------- Conversations (chat with the entity) ----------

/// One conversation thread with Vara — the unit of continuity.
#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub id: i64,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    /// When this thread was opened from a mission/report, the link lives here
    /// so every reply is grounded in that report.
    pub mission_id: Option<i64>,
}

/// One stored chat message (user or assistant side).
/// `kind` selects how the UI renders it:
/// "text" (default) | "mission" (live mission card, content = JSON) |
/// "action" (OS action result card, content = JSON).
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessageRecord {
    pub id: i64,
    pub conversation_id: i64,
    pub role: String, // "user" | "assistant"
    pub content: String,
    pub model: Option<String>,
    pub tokens: i64,
    pub status: String, // "streaming" | "ok" | "stopped" | "error"
    pub created_at: String,
    #[serde(default = "default_message_kind")]
    pub kind: String,
    #[serde(default)]
    pub mission_id: Option<i64>,
}

fn default_message_kind() -> String {
    "text".into()
}

/// An OS action Vara proposes from inside the chat ([[sys]] protocol).
/// Execution always passes the autonomy policy — the model never runs
/// anything by itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SysAction {
    pub action: String, // "open_url" | "open_path" | "run"
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LlmReply {
    pub content: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl LlmReply {
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

// ---------- Settings ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_temperature() -> f32 {
    0.4
}

impl Default for ProviderConfig {
    fn default() -> Self {
        // Shipped preset: an OpenAI-compatible endpoint with an empty key.
        // The key never ships with the app — the owner enters theirs once.
        Self {
            base_url: "https://ktai.koyeb.app/v1".into(),
            api_key: String::new(),
            model: "deepseek-ai/deepseek-v4.1-flash".into(),
            temperature: default_temperature(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutonomyConfig {
    pub heartbeat_enabled: bool,
    pub heartbeat_minutes: u64,
    pub close_to_tray: bool,
    pub notifications_enabled: bool,
    pub open_urls: bool,
    pub open_paths: bool,
    pub autostart: bool,
    /// When Vara proposes a mission from inside the chat, start it without
    /// asking (read-only research; costs are budget-capped). One tap in the
    /// thread still exists as fallback when Vara is busy.
    pub auto_start_missions: bool,
    /// Whether the [[sys]] "run" action is available at all.
    pub run_commands: bool,
    /// Whether Vara may propose capturing the screen ([[sys]] "screenshot").
    /// Reading the screen is privacy-sensitive, so this ships OFF; even when
    /// enabled, every capture still shows the explicit approval card.
    pub allow_screenshots: bool,
    /// Whether Vara may propose full computer-use sequences ([[sys]]
    /// "computer_use"): see→act→confirm flows driving mouse/keyboard/windows
    /// through the ActLoop. Ships OFF — this is the entity's hands, gated by
    /// the owner. When enabled, L0/L1 steps run under the policy, while any
    /// L2 op (destructive close) still requires its explicit unlock AND the
    /// chat approval card.
    pub allow_computer_use: bool,
    /// L2 unlock for computer-use sequences: window/app close operations.
    /// Without it, close ops stay dry runs with a preview — by design.
    pub computer_use_allow_close: bool,
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            heartbeat_enabled: false,
            heartbeat_minutes: 30,
            close_to_tray: true,
            notifications_enabled: true,
            open_urls: true,
            open_paths: true,
            autostart: false,
            auto_start_missions: true,
            run_commands: true,
            allow_screenshots: false,
            allow_computer_use: false,
            computer_use_allow_close: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MissionDefaults {
    pub budget_tokens: i64,
    pub max_steps: i64,
}

impl Default for MissionDefaults {
    fn default() -> Self {
        Self {
            budget_tokens: 30_000,
            max_steps: 14,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: String,      // "ar" | "en"
    pub persona_style: String, // classic | dark | stealth | tech | nature
    pub provider: ProviderConfig,
    pub autonomy: AutonomyConfig,
    pub mission_defaults: MissionDefaults,
    pub watched_folder: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "ar".into(),
            persona_style: "classic".into(),
            provider: ProviderConfig::default(),
            autonomy: AutonomyConfig::default(),
            mission_defaults: MissionDefaults::default(),
            watched_folder: None,
        }
    }
}

// ---------- Plan protocol (planner <-> runner) ----------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlanDimension {
    pub name: String,
    #[serde(default)]
    pub question: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlanStep {
    pub kind: String, // "search" | "fetch" | "report"
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub dimension: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Plan {
    #[serde(default)]
    pub dimensions: Vec<PlanDimension>,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
}

/// Ledger entry for one retrieved source during a mission.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievedSource {
    pub url: String,
    pub title: String,
    pub fetched: bool,
    pub note_id: Option<i64>,
}
