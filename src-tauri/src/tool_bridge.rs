//! The shell side of the tool loop: wire `vara_core::tool_loop` to the real
//! filesystem, the real database and the real conversation.
//!
//! The core owns *what a tool is* and *whether a call may execute*; this file
//! only supplies the two things the core must not touch: the machine and the
//! transcript. It is deliberately small, because everything interesting is
//! already tested headlessly in `crates/vara-core`.
//!
//! Flow, per user turn (see `chat_send`):
//!
//! 1. Before the main reply, ask the router whether this turn needs a tool.
//! 2. A read-only call runs immediately and its output is appended to the
//!    context as **data** (wrapped in `[tool:…]` markers).
//! 3. Anything above read-only becomes an `action_proposals` row via the
//!    existing v0.6.0 machinery — the model never executes it.
//!
//! Failure policy: if the router call fails, the chat continues without tools.
//! A tool problem must never block the conversation.

use crate::AppState;
use std::path::PathBuf;
use vara_core::tool_loop::{
    execute_intent, parse_intent, planner_system_prompt, render_tool_result, ToolIntent,
    ToolTurnOutcome,
};
use vara_core::tools_local::{FsToolHost, MemoryHit, MemoryProvider};
use vara_core::tools_registry::{Roots, ToolCtx};

/// The owner's allowed folders. Today this is the watched folder (when set)
/// plus the user profile; Settings will make it explicit.
pub fn allowed_roots(state: &AppState) -> Vec<PathBuf> {
    let snapshot = state.settings_snapshot();
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(folder) = snapshot.watched_folder.as_ref() {
        let trimmed = folder.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_dir() {
                roots.push(path);
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let path = PathBuf::from(home);
        if path.is_dir() && !roots.iter().any(|r| r == &path) {
            roots.push(path);
        }
    }
    roots
}

/// Memory search backed by the real database (FTS when available).
struct DbMemory {
    db: std::sync::Arc<vara_core::Database>,
}

impl MemoryProvider for DbMemory {
    fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryHit>, String> {
        let notes = self
            .db
            .search_notes(query, limit as i64)
            .map_err(|e| e.to_string())?;
        Ok(notes
            .into_iter()
            .map(|n| MemoryHit {
                title: n.title,
                body: n.body,
            })
            .collect())
    }
}

/// Build the tool context for one turn.
pub fn tool_context(state: &AppState) -> ToolCtx {
    let roots = allowed_roots(state);
    ToolCtx {
        roots: Roots::new(roots),
        now_unix: vara_core::tools_local::now_unix(),
        denied_paths: Vec::new(),
        memory: Some(std::sync::Arc::new(DbMemory {
            db: state.db.clone(),
        })),
    }
}

/// What one routing attempt produced for the caller to fold into the chat.
pub struct RoutedTurn {
    pub outcome: ToolTurnOutcome,
    /// Text to append to the model's context (already marked as data).
    pub context_note: String,
    /// A proposal row, when the action needs the owner's approval.
    pub proposal_id: Option<i64>,
}

/// Ask the router, run a read-only tool if asked, or record a proposal.
///
/// Returns `None` when the turn needs no tool at all — the common case, and
/// the one that must stay fast and free.
pub async fn route_turn(
    state: &AppState,
    llm: &vara_core::LlmClient,
    user_text: &str,
) -> Option<RoutedTurn> {
    // The registry is rebuilt after the await: a `ToolRegistry` is `Send + Sync`
    // now, but holding it across the model call is unnecessary and the prompt is
    // the only thing the call needs.
    let registry = vara_core::tools_local::read_only_registry();
    let roots: Vec<String> = allowed_roots(state)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let system = planner_system_prompt(&registry, &roots);
    let messages = [
        vara_core::types::ChatMessage::system(system),
        vara_core::types::ChatMessage::user(user_text),
    ];
    drop(registry);
    let reply = llm.chat(&messages, Some(200)).await.ok()?;

    let registry = vara_core::tools_local::read_only_registry();
    let intent = parse_intent(&reply.content, &registry);
    if intent == ToolIntent::None {
        return None;
    }

    let ctx = tool_context(state);
    // `FsToolHost::new()` reads the platform itself now (see
    // `platform_system_info` in the core), so the shell no longer duplicates that
    // reader — one implementation, and `system_info` reports RAM instead of
    // `unknown`.
    let host = FsToolHost::new();
    let outcome = execute_intent(&intent, &registry, &ctx, &host);

    // A proposal is recorded in the same table the [[sys]] path uses, so the
    // approval UI, the digest and the expiry behave identically.
    let proposal_id = if outcome.needs_approval {
        proposal_row(state, &intent).ok().flatten()
    } else {
        None
    };

    let context_note = match &outcome.result {
        Some(result) => render_tool_result(outcome.tool.as_deref().unwrap_or("tool"), result),
        None => String::new(),
    };

    Some(RoutedTurn {
        outcome,
        context_note,
        proposal_id,
    })
}

/// Record an `action_proposals` row for a tool the owner must approve.
///
/// The row carries the tool call in its `target` as a compact JSON envelope, so
/// the existing approve/execute path can hand it to the right executor when
/// write tools land.
fn proposal_row(state: &AppState, intent: &ToolIntent) -> Result<Option<i64>, String> {
    let (tool, args, why, class) = match intent {
        ToolIntent::Propose {
            tool,
            args,
            why,
            class,
        } => (tool, args, why, class),
        _ => return Ok(None),
    };
    let kind = vara_core::exec_policy::ProposalKind::Run;
    let target = serde_json::json!({ "tool": tool, "args": args }).to_string();
    let planned = vara_core::exec_policy::plan_proposal(kind, &target, why, unix_now())
        .map_err(|e| e.to_string())?;
    // Keep the class visible on the card even though the current executor kind
    // is `run`: the risk label is what the owner reads.
    let risk_note = format!("{} (class {})", planned.reason, class.as_str());
    let planned = vara_core::exec_policy::plan_proposal(kind, &target, &risk_note, unix_now())
        .map_err(|e| e.to_string())?;
    state
        .db
        .insert_action_proposal(&planned, None, None)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Exposed for the shell's receipt payloads, so the expiry it prints matches
/// the one the policy actually stored.
pub fn unix_now_pub() -> i64 {
    unix_now()
}
