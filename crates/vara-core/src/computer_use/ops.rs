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

/// Canonical modifier order. `normalize_combo` reorders modifiers so spelling
/// variants of one chord land on one string ("shift+ctrl+w" and
/// "ctrl+shift+w" are the same chord), never on two different policy verdicts.
const MODIFIER_ORDER: [&str; 4] = ["ctrl", "alt", "shift", "win"];

/// Canonical form of a hotkey chord: split on `+`, trim, lowercase, fold
/// aliases (`control`→`ctrl`, `option`→`alt`, `cmd`/`command`/`super`/`win`/
/// `windows`/`meta`→`win`), then emit modifiers first in a fixed order and the
/// remaining keys in the order written: `normalize_combo(" Control + W ")`
/// and `normalize_combo("ctrl+W")` both give `"ctrl+w"`.
///
/// This exists because the destructive-combo gate must not be bypassable by
/// spelling: a chord that *is* `ctrl+w` has to classify like `ctrl+w` no
/// matter how the model typed it. Model-authored spacing and aliases are hit
/// constantly; a classifier that only lowercases is a hole, not a parser.
pub fn normalize_combo(keys: &str) -> String {
    let mut modifiers: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    for part in keys.split('+') {
        let part = part.trim().to_lowercase();
        if part.is_empty() {
            continue; // "ctrl++" or a trailing '+' is noise, not a key
        }
        let canonical = match part.as_str() {
            "control" => "ctrl",
            "option" => "alt",
            "cmd" | "command" | "super" | "windows" | "meta" => "win",
            other => other,
        };
        if MODIFIER_ORDER.contains(&canonical) {
            if !modifiers.iter().any(|m| m == canonical) {
                modifiers.push(canonical.to_string()); // "ctrl+ctrl+w" is one ctrl
            }
        } else {
            others.push(canonical.to_string());
        }
    }
    modifiers.sort_by_key(|m| {
        MODIFIER_ORDER
            .iter()
            .position(|x| *x == m.as_str())
            .unwrap_or(usize::MAX)
    });
    modifiers.extend(others);
    modifiers.join("+")
}

/// Does this chord destroy something? Classification uses the canonical form,
/// so aliases, padding, and modifier order cannot smuggle a chord past L2.
pub fn is_destructive_combo(keys: &str) -> bool {
    DESTRUCTIVE_COMBOS.contains(&normalize_combo(keys).as_str())
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

/// Where a coordinate op lands. The grounding gate treats both frames of
/// reference the same — blind is blind — and only the refusal message differs,
/// because "take a screenshot of the screen" and "take a screenshot of the
/// window" are different instructions for the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelTarget {
    /// Screen pixels.
    Absolute(i32, i32),
    /// Pixels relative to the named window's top-left corner.
    WindowRelative(i32, i32),
}

impl PixelTarget {
    pub fn coords(&self) -> (i32, i32) {
        match *self {
            PixelTarget::Absolute(x, y) | PixelTarget::WindowRelative(x, y) => (x, y),
        }
    }

    /// Frame of reference, for the message the model has to act on.
    pub fn frame(&self) -> &'static str {
        match self {
            PixelTarget::Absolute(..) => "screen",
            PixelTarget::WindowRelative(..) => "window-relative",
        }
    }
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

    /// Does this op demand L2 because it can destroy user work — a close op or
    /// a destructive hotkey chord? Read from the grant ladder so there is one
    /// classification, not two that can drift apart.
    pub fn is_destructive(&self) -> bool {
        self.grant_level() >= GrantLevel::L2
    }

    /// The pixel target this op would act at, if it has one — the input to the
    /// ActLoop's grounding gate.
    ///
    /// `click_win` belongs here. Window-relative pixels are still pixels aimed
    /// at a spot on screen: firing one before any SEE is exactly the blind hit
    /// the gate exists to refuse, and a window that moved since the model
    /// planned makes the point wrong even when the title is right. Ops
    /// addressed by *name* (`focus`, `type`/`hotkey`/`key`/`keys` with a
    /// window, `close_*`) are deliberately not pixel targets — they cannot land
    /// on the wrong widget, and gating them would only add friction.
    pub fn pixel_target(&self) -> Option<PixelTarget> {
        match self {
            CuOp::Click {
                x: Some(x),
                y: Some(y),
                ..
            } => Some(PixelTarget::Absolute(*x, *y)),
            // A click with no coordinates lands wherever the cursor happens to
            // be; the adapter decides, so there is nothing here to ground.
            CuOp::ClickWin { rel_x, rel_y, .. } => {
                Some(PixelTarget::WindowRelative(*rel_x, *rel_y))
            }
            CuOp::Move { x, y } => Some(PixelTarget::Absolute(*x, *y)),
            CuOp::Scroll {
                x: Some(x),
                y: Some(y),
                ..
            } => Some(PixelTarget::Absolute(*x, *y)),
            _ => None,
        }
    }

    /// Absolute coordinates this op would act at, when it has any.
    /// Window-relative targets are excluded — see `pixel_target`.
    pub fn absolute_coords(&self) -> Option<(i32, i32)> {
        match self.pixel_target()? {
            PixelTarget::Absolute(x, y) => Some((x, y)),
            PixelTarget::WindowRelative(..) => None,
        }
    }

    /// The harmless twin of a destructive op: `confirm` forced off, so the best
    /// an adapter can do with it is preview. The ActLoop applies this itself
    /// whenever the policy has not granted L2 — a model-authored `confirm` is a
    /// request, never an authorization, and the loop must not depend on the
    /// adapter to remember that.
    ///
    /// `None` means the op has no dry-run form. A destructive hotkey is the
    /// case that matters: a chord cannot be "previewed", so the loop's dry run
    /// is to never send it at all.
    pub fn as_dry_run(&self) -> Option<CuOp> {
        match self {
            CuOp::CloseWindow { title, .. } => Some(CuOp::CloseWindow {
                title: title.clone(),
                confirm: false,
            }),
            CuOp::CloseApp { process, .. } => Some(CuOp::CloseApp {
                process: process.clone(),
                confirm: false,
            }),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The normalizer is the whole defence against spelling bypasses, so its
    /// contract is pinned directly: aliases, padding, case, modifier order.
    #[test]
    fn normalize_combo_canonicalizes_spelling() {
        let cases = [
            ("ctrl+s", "ctrl+s"),
            (" Control + S ", "ctrl+s"),
            ("CTRL+S", "ctrl+s"),
            ("control+s", "ctrl+s"),
            ("alt+f4", "alt+f4"),
            ("Option + F4", "alt+f4"),
            ("shift+ctrl+w", "ctrl+shift+w"),
            ("w+ctrl", "ctrl+w"),
            ("ctrl+ctrl+w", "ctrl+w"),
            ("cmd+q", "win+q"),
            ("command+q", "win+q"),
            ("super+q", "win+q"),
            ("win+q", "win+q"),
            ("shift+F4", "shift+f4"),
            ("ctrl++w", "ctrl+w"),
        ];
        for (input, want) in cases {
            assert_eq!(normalize_combo(input), want, "normalize_combo({input:?})");
        }
    }

    /// Every spelling of a destructive chord must demand L2 — including the
    /// padded and aliased forms models actually emit, which the old
    /// trim+lowercase classifier waved through as L1.
    #[test]
    fn destructive_combo_variants_all_classify_l2() {
        let variants = [
            "alt+f4",
            "Alt+F4",
            "ALT + F4",
            "alt + f4",
            "ctrl+w",
            "ctrl+W",
            "CTRL + w",
            "Control + W",
            "control + w",
            "w+ctrl",
            "ctrl+q",
            "Ctrl + Q",
            "ctrl+f4",
            "Ctrl + F4",
            "ctrl+shift+w",
            "Ctrl + Shift + W",
            "shift+ctrl+w",
        ];
        for v in variants {
            assert!(is_destructive_combo(v), "{v:?} must classify destructive");
            let op = CuOp::Hotkey {
                keys: v.into(),
                window: None,
            };
            assert_eq!(op.grant_level(), GrantLevel::L2, "{v:?} must demand L2");
            assert!(op.is_destructive(), "{v:?} is destructive");
        }
        // Harmless chords stay L1: the classifier must not over-block either.
        for v in ["ctrl+s", "ctrl+a", "ctrl+c", "alt+tab", "enter"] {
            assert!(!is_destructive_combo(v), "{v:?} is not destructive");
            assert_eq!(
                CuOp::Hotkey {
                    keys: v.into(),
                    window: None
                }
                .grant_level(),
                GrantLevel::L1,
                "{v:?} stays L1"
            );
        }
    }

    /// The grounding gate's input: every op that aims at a pixel has a target,
    /// every op that aims at a name does not.
    #[test]
    fn pixel_target_covers_every_coordinate_op() {
        let click = CuOp::Click {
            x: Some(10),
            y: Some(20),
            button: "left".into(),
            clicks: 1,
            window: None,
        };
        assert_eq!(click.pixel_target(), Some(PixelTarget::Absolute(10, 20)));
        assert_eq!(click.absolute_coords(), Some((10, 20)));

        let click_win = CuOp::ClickWin {
            title: "Notepad".into(),
            rel_x: 5,
            rel_y: 6,
            button: "left".into(),
            clicks: 1,
        };
        assert_eq!(
            click_win.pixel_target(),
            Some(PixelTarget::WindowRelative(5, 6)),
            "window-relative pixels are still pixels"
        );
        assert_eq!(click_win.absolute_coords(), None);
        assert_eq!(click_win.pixel_target().unwrap().frame(), "window-relative");

        assert_eq!(
            CuOp::Move { x: 1, y: 2 }.pixel_target(),
            Some(PixelTarget::Absolute(1, 2))
        );
        assert_eq!(
            CuOp::Scroll {
                clicks: 3,
                x: Some(4),
                y: Some(5)
            }
            .pixel_target(),
            Some(PixelTarget::Absolute(4, 5))
        );
        // A scroll with no coordinates goes to the cursor/window — not a target.
        assert_eq!(
            CuOp::Scroll {
                clicks: 3,
                x: None,
                y: None
            }
            .pixel_target(),
            None
        );

        // Name-targeted ops stay out of the coordinate gate.
        let named = [
            CuOp::Focus {
                title: "Notepad".into(),
            },
            CuOp::Type {
                text: "hi".into(),
                window: Some("Notepad".into()),
            },
            CuOp::Hotkey {
                keys: "ctrl+s".into(),
                window: Some("Notepad".into()),
            },
            CuOp::Key {
                key: "enter".into(),
                window: Some("Notepad".into()),
            },
            CuOp::Keys {
                sequence: "ctrl+a ctrl+c".into(),
                window: Some("Notepad".into()),
            },
            CuOp::Wait { seconds: 0.5 },
            CuOp::CloseWindow {
                title: "Notepad".into(),
                confirm: false,
            },
        ];
        for op in named {
            assert_eq!(op.pixel_target(), None, "{} is name-targeted", op.tag());
        }
    }

    /// The loop's dry-run conversion: the confirm flag is dropped, nothing else
    /// about the op changes, and non-destructive ops are left alone.
    #[test]
    fn as_dry_run_forces_confirm_off() {
        let close = CuOp::CloseWindow {
            title: "Notepad".into(),
            confirm: true,
        };
        assert_eq!(
            close.as_dry_run(),
            Some(CuOp::CloseWindow {
                title: "Notepad".into(),
                confirm: false
            })
        );
        let kill = CuOp::CloseApp {
            process: "notepad.exe".into(),
            confirm: true,
        };
        assert_eq!(
            kill.as_dry_run(),
            Some(CuOp::CloseApp {
                process: "notepad.exe".into(),
                confirm: false
            })
        );
        // Already a dry run → nothing to change; the adapter still previews.
        assert_eq!(
            CuOp::CloseWindow {
                title: "x".into(),
                confirm: false
            }
            .as_dry_run(),
            Some(CuOp::CloseWindow {
                title: "x".into(),
                confirm: false
            })
        );
        // A destructive hotkey has no dry-run form: "not sent" is the loop's job.
        assert_eq!(
            CuOp::Hotkey {
                keys: "alt + f4".into(),
                window: None
            }
            .as_dry_run(),
            None
        );
        assert_eq!(
            CuOp::Type {
                text: "hi".into(),
                window: None
            }
            .as_dry_run(),
            None
        );
    }

    /// Destructiveness comes from the ladder, so a variant chord is L2 for both
    /// the loop and the shell's approval card.
    #[test]
    fn destructive_predicate_follows_the_ladder() {
        assert!(CuOp::CloseWindow {
            title: "x".into(),
            confirm: false
        }
        .is_destructive());
        assert!(CuOp::CloseApp {
            process: "x".into(),
            confirm: false
        }
        .is_destructive());
        assert!(CuOp::Hotkey {
            keys: "Control + W".into(),
            window: None
        }
        .is_destructive());
        assert!(!CuOp::Screenshot {
            region: None,
            settle: 0.1
        }
        .is_destructive());
        assert!(!CuOp::Click {
            x: Some(1),
            y: Some(1),
            button: "left".into(),
            clicks: 1,
            window: None
        }
        .is_destructive());
    }
}
