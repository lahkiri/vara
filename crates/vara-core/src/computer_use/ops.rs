//! The computer-use operation language — a serde port of the proven
//! see→act→confirm discipline from the owner's Windows computer-use MCP.
//!
//! Design contracts carried over verbatim (they are the value, not the tools):
//! - **Validate-then-execute**: a sequence is checked in full before the first
//!   op runs; a typo can never half-execute a UI flow (serde tagged enum).
//! - **Grounded coordinates**: clicks require evidence from a SEE op — the
//!   ActLoop tracks grounding, the adapter refuses blind hits.
//! - **Two-step destructive ops**: close ops default to a harmless dry run
//!   and only execute with explicit `confirm=true` + policy grant.
//! - **Response-as-guidance**: every result tells the agent what to check
//!   next (`check` / `next` / `hint`), mirroring the MCP response format.

use serde::{Deserialize, Serialize};

/// Hotkey combos that can destroy user work. Mirror of the MCP server's
/// `_DESTRUCTIVE_COMBOS` — firing one forces verification evidence.
pub const DESTRUCTIVE_COMBOS: [&str; 5] = ["alt+f4", "ctrl+w", "ctrl+q", "ctrl+f4", "ctrl+shift+w"];

pub fn is_destructive_combo(keys: &str) -> bool {
    DESTRUCTIVE_COMBOS.contains(&keys.trim().to_lowercase().as_str())
}

/// Autonomy ladder — every op maps to exactly one level. The ActLoop and the
/// shell policy both consult this; the model is never asked to classify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantLevel {
    /// Observe-only: screenshots, verification, waiting.
    L0,
    /// Reversible input into the focused window.
    L1,
    /// Window/app lifecycle — reversibility is not guaranteed.
    L2,
    /// System-level / shell — allowlist + explicit owner grant.
    L3,
}

impl GrantLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrantLevel::L0 => "L0",
            GrantLevel::L1 => "L1",
            GrantLevel::L2 => "L2",
            GrantLevel::L3 => "L3",
        }
    }
}

/// One step of a computer-use sequence. The `op` tag is the discriminated
/// union key — unknown ops are rejected at parse time, before anything runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CuOp {
    /// STEP 1 of see-act-confirm. `settle` defeats stale frames of animating
    /// panels; region captures clip to [x,y,w,h] in screen pixels.
    Screenshot {
        #[serde(default)]
        region: Option<[i32; 4]>,
        #[serde(default = "default_settle")]
        settle: f32,
    },
    /// Alias kept for protocol tolerance (`shot` == `screenshot`).
    Shot {
        #[serde(default)]
        region: Option<[i32; 4]>,
        #[serde(default = "default_settle")]
        settle: f32,
    },
    /// Explicit observation op — produce evidence the state is as expected.
    Verify,
    /// Focus a window by case-insensitive title substring (focus-first).
    Focus { title: String },
    /// Click at absolute screen coordinates.
    Click {
        #[serde(default)]
        x: Option<i32>,
        #[serde(default)]
        y: Option<i32>,
        #[serde(default = "default_button")]
        button: String,
        #[serde(default = "one")]
        clicks: u32,
        #[serde(default)]
        window: Option<String>,
    },
    /// Focus a window, then click at window-relative coords (survives moves).
    ClickWin {
        title: String,
        rel_x: i32,
        rel_y: i32,
        #[serde(default = "default_button")]
        button: String,
        #[serde(default = "one")]
        clicks: u32,
    },
    /// Move the cursor.
    Move { x: i32, y: i32 },
    /// Type text; long/unicode text rides the clipboard path.
    Type {
        text: String,
        #[serde(default)]
        window: Option<String>,
    },
    /// Chord like "ctrl+s". Destructive combos force verify evidence.
    Hotkey {
        keys: String,
        #[serde(default)]
        window: Option<String>,
    },
    /// Single key: enter, tab, esc, f5...
    Key {
        key: String,
        #[serde(default)]
        window: Option<String>,
    },
    /// Ordered key sequence: "ctrl+a ctrl+c end".
    Keys {
        sequence: String,
        #[serde(default)]
        window: Option<String>,
    },
    /// Wheel scroll; positive = up.
    Scroll {
        clicks: i32,
        #[serde(default)]
        x: Option<i32>,
        #[serde(default)]
        y: Option<i32>,
    },
    /// Pause for animations/dialogs (capped by the loop, not trusted).
    Wait {
        #[serde(default = "default_wait")]
        seconds: f32,
    },
    /// Control marker: following step failures don't stop the sequence.
    IgnoreErrors,
    /// Destructive: close a window. Dry run unless `confirm`.
    CloseWindow {
        title: String,
        #[serde(default)]
        confirm: bool,
    },
    /// Destructive: kill a process by name. Dry run unless `confirm`.
    CloseApp {
        process: String,
        #[serde(default)]
        confirm: bool,
    },
}

fn default_settle() -> f32 {
    0.1
}
fn default_button() -> String {
    "left".into()
}
fn one() -> u32 {
    1
}
fn default_wait() -> f32 {
    0.1
}

impl CuOp {
    pub fn tag(&self) -> &'static str {
        match self {
            CuOp::Screenshot { .. } | CuOp::Shot { .. } => "screenshot",
            CuOp::Verify { .. } => "verify",
            CuOp::Focus { .. } => "focus",
            CuOp::Click { .. } => "click",
            CuOp::ClickWin { .. } => "click_win",
            CuOp::Move { .. } => "move",
            CuOp::Type { .. } => "type",
            CuOp::Hotkey { .. } => "hotkey",
            CuOp::Key { .. } => "key",
            CuOp::Keys { .. } => "keys",
            CuOp::Scroll { .. } => "scroll",
            CuOp::Wait { .. } => "wait",
            CuOp::IgnoreErrors => "ignore_errors",
            CuOp::CloseWindow { .. } => "close_window",
            CuOp::CloseApp { .. } => "close_app",
        }
    }

    /// The autonomy ladder level this op demands.
    pub fn grant_level(&self) -> GrantLevel {
        match self {
            CuOp::Screenshot { .. } | CuOp::Shot { .. } | CuOp::Verify { .. } => GrantLevel::L0,
            CuOp::Wait { .. } | CuOp::IgnoreErrors => GrantLevel::L0,
            CuOp::Focus { .. }
            | CuOp::Click { .. }
            | CuOp::ClickWin { .. }
            | CuOp::Move { .. }
            | CuOp::Type { .. }
            | CuOp::Key { .. }
            | CuOp::Keys { .. }
            | CuOp::Scroll { .. } => GrantLevel::L1,
            CuOp::Hotkey { keys, .. } if is_destructive_combo(keys) => GrantLevel::L2,
            CuOp::Hotkey { .. } => GrantLevel::L1,
            CuOp::CloseWindow { .. } | CuOp::CloseApp { .. } => GrantLevel::L2,
        }
    }

    /// Does this op *observe* the screen (can ground later coordinates)?
    pub fn is_see(&self) -> bool {
        matches!(
            self,
            CuOp::Screenshot { .. } | CuOp::Shot { .. } | CuOp::Verify { .. }
        )
    }

    /// Does this op mutate UI state (needs verify evidence afterwards)?
    pub fn is_mutating(&self) -> bool {
        !self.is_see() && !matches!(self, CuOp::Wait { .. } | CuOp::IgnoreErrors)
    }

    /// Absolute coordinates this op would act at, when grounded.
    pub fn absolute_coords(&self) -> Option<(i32, i32)> {
        match self {
            CuOp::Click {
                x: Some(x),
                y: Some(y),
                ..
            } => Some((*x, *y)),
            CuOp::Move { x, y } => Some((*x, *y)),
            CuOp::Scroll {
                x: Some(x),
                y: Some(y),
                ..
            } => Some((*x, *y)),
            _ => None,
        }
    }

    /// Structural validation beyond serde — bounds, empties, caps.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            CuOp::Screenshot { region, .. } | CuOp::Shot { region, .. } => {
                if let Some([x, y, w, h]) = region {
                    if *w <= 0 || *h <= 0 {
                        return Err("screenshot region must have positive w/h".into());
                    }
                    if *x < 0 || *y < 0 {
                        return Err("screenshot region must be on-screen".into());
                    }
                }
                Ok(())
            }
            CuOp::Focus { title } | CuOp::CloseWindow { title, .. } => {
                if title.trim().is_empty() {
                    return Err("window title must not be empty".into());
                }
                Ok(())
            }
            CuOp::CloseApp { process, .. } => {
                if process.trim().is_empty() {
                    return Err("process name must not be empty".into());
                }
                Ok(())
            }
            CuOp::ClickWin { title, .. } => {
                if title.trim().is_empty() {
                    return Err("click_win needs a window title".into());
                }
                Ok(())
            }
            CuOp::Type { text, .. } => {
                if text.len() > 100_000 {
                    return Err("type text exceeds 100k chars".into());
                }
                Ok(())
            }
            CuOp::Hotkey { keys, .. } => {
                if keys.trim().is_empty() {
                    return Err("hotkey needs keys".into());
                }
                Ok(())
            }
            CuOp::Key { key, .. } => {
                if key.trim().is_empty() {
                    return Err("key needs a key name".into());
                }
                Ok(())
            }
            CuOp::Keys { sequence, .. } => {
                if sequence.trim().is_empty() {
                    return Err("keys needs a sequence".into());
                }
                Ok(())
            }
            CuOp::Wait { seconds } => {
                if !(0.0..=10.0).contains(seconds) {
                    return Err("wait must be 0..=10 seconds".into());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// A whole sequence — validated as a unit before execution (the MCP
/// `run_actions` contract: nothing runs unless every step parses).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuSequence {
    #[serde(default = "stop_on_error_default")]
    pub stop_on_error: bool,
    pub actions: Vec<CuOp>,
}

fn stop_on_error_default() -> bool {
    true
}

impl CuSequence {
    pub fn parse(json: &str) -> Result<Self, String> {
        let seq: CuSequence =
            serde_json::from_str(json.trim()).map_err(|e| format!("invalid sequence: {e}"))?;
        if seq.actions.is_empty() {
            return Err("sequence is empty".into());
        }
        if seq.actions.len() > 64 {
            return Err("sequence exceeds 64 ops".into());
        }
        for (i, op) in seq.actions.iter().enumerate() {
            op.validate()
                .map_err(|e| format!("action[{i}] ({}): {e}", op.tag()))?;
        }
        Ok(seq)
    }

    /// The highest grant level any op demands — the policy gate reads this.
    pub fn max_grant(&self) -> GrantLevel {
        self.actions
            .iter()
            .map(|o| o.grant_level())
            .max()
            .unwrap_or(GrantLevel::L0)
    }

    /// Human summary for the approval card (per-op badges).
    pub fn summary_lines(&self) -> Vec<String> {
        self.actions
            .iter()
            .map(|op| format!("[{}] {}", op.grant_level().as_str(), op.tag()))
            .collect()
    }
}

/// Unified response contract — every adapter speaks this, mirroring the
/// owner's MCP: response-as-guidance, receipts, and focus diagnosis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuResult {
    pub ok: bool,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Foreground window title at capture time — focus-mismatch diagnosis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// Evidence artifact (screenshot) produced by this op.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_path: Option<String>,
    /// What the agent must confirm in `verify_path` before proceeding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// True when a destructive op intentionally did nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    pub ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl CuResult {
    pub fn fail(op: &str, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            op: op.into(),
            error: Some(error.into()),
            active: None,
            path: None,
            before_path: None,
            check: None,
            dry_run: None,
            focus: None,
            ms: 0,
            next: Some("read the error, re-see the screen, re-plan".into()),
            hint: None,
        }
    }
}
