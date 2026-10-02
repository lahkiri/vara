//! Heartbeat v2 — the initiative engine, silent by default.
//!
//! v0.6.0's heartbeat called the model on a timer and **always wrote a
//! reflection note**. After a day the memory was a diary of near-identical
//! philosophical paragraphs (owner screenshot G-01), which is the opposite of
//! what a resident entity should feel like: noise where real events should be.
//!
//! This module is the pure, testable core of the fix. It decides *what to say*
//! from a bounded state snapshot, under three hard rules:
//!
//! 1. **Silence is the default outcome.** A tick on an unchanged workspace
//!    writes one counter row and nothing else — no note, no log line, no
//!    activity event. Saying nothing is a successful tick, not a missed one.
//! 2. **The budget is binding.** A tick that cannot be described inside
//!    [`HEARTBEAT_MAX_INPUT_TOKENS`] is *skipped and counted*, never silently
//!    truncated. (OpenClaw's heartbeat was measured sending ~120k tokens and
//!    costing ~$0.75 per check — a timer must never be able to spend like a
//!    mission.)
//! 3. **A proposal must be new.** Candidates are de-duplicated against the
//!    recent proposal/note history, so the entity cannot repeat itself into
//!    looking busy.
//!
//! The model call itself lives in `entity.rs`; this file owns the policy, the
//! bounds and the arithmetic, which is what the tests pin down.

use crate::dedup;
use serde::{Deserialize, Serialize};

/// In/out ceilings for ONE tick, in tokens. Deliberately tiny compared with a
/// mission: a heartbeat runs 48 times a day and must never be a cost hazard.
pub const HEARTBEAT_MAX_INPUT_TOKENS: usize = 2_000;
pub const HEARTBEAT_MAX_OUTPUT_TOKENS: usize = 300;
/// `HEARTBEAT.md` is user-owned and must stay short enough to read at a glance.
pub const HEARTBEAT_CHECKLIST_MAX_LINES: usize = 50;
/// How many recent proposals/notes a candidate is compared against.
pub const DEDUP_WINDOW: usize = 20;
/// Similarity at or above this is "the same idea again".
pub const DEDUP_THRESHOLD: f64 = 0.72;
/// Notifications per day before only urgent ones get through.
pub const NOTIFY_DAILY_CAP: usize = 2;

/// Rough token estimate (4 chars/token) — good enough to bound a prompt, and
/// honest about being an estimate: the point is to refuse the pathological
/// tick, not to be exact.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 4 + 1
}

/// What the tick may look at. Everything here is a *count plus a few examples*
/// rather than a corpus: the note body never enters the prompt, which is what
/// kept the old tick's context small by accident and this one small by design.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HeartbeatSnapshot {
    /// New files seen in watched folders since the last tick.
    pub new_files: Vec<String>,
    /// Routines that are due now.
    pub due_routines: Vec<String>,
    /// Routines whose last run failed.
    pub failed_routines: Vec<String>,
    /// Approval requests still waiting for the owner.
    pub pending_approvals: usize,
    /// Missions that ended in failure since the last tick.
    pub failed_missions: Vec<String>,
    /// Disk/RAM alarms, already phrased for a human.
    pub resource_alarms: Vec<String>,
    /// The next scheduled run, if any.
    pub next_scheduled: Option<String>,
}

impl HeartbeatSnapshot {
    /// Is there anything at all to react to? An empty snapshot must produce a
    /// silent tick, by construction rather than by prompt luck.
    pub fn is_empty(&self) -> bool {
        self.new_files.is_empty()
            && self.due_routines.is_empty()
            && self.failed_routines.is_empty()
            && self.pending_approvals == 0
            && self.failed_missions.is_empty()
            && self.resource_alarms.is_empty()
            && self.next_scheduled.is_none()
    }

    /// Bounded, human-readable rendering for the prompt. Caps every list so a
    /// pathological workspace (10k new files) cannot blow the tick budget.
    pub fn render(&self, checklist: &str) -> String {
        const MAX_LIST: usize = 5;
        let mut out = String::new();
        out.push_str("Checklist (owner-owned):\n");
        for line in checklist.lines().take(HEARTBEAT_CHECKLIST_MAX_LINES) {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("\nWorkspace state:\n");
        let list = |name: &str, items: &[String]| {
            if items.is_empty() {
                String::new()
            } else {
                let shown: Vec<&str> = items.iter().take(MAX_LIST).map(|s| s.as_str()).collect();
                format!(
                    "- {name}: {} total, e.g. {}\n",
                    items.len(),
                    shown.join(", ")
                )
            }
        };
        out.push_str(&list("new files", &self.new_files));
        out.push_str(&list("due routines", &self.due_routines));
        out.push_str(&list("failed routines", &self.failed_routines));
        out.push_str(&list("failed missions", &self.failed_missions));
        out.push_str(&list("resource alarms", &self.resource_alarms));
        if self.pending_approvals > 0 {
            out.push_str(&format!(
                "- approvals waiting for the owner: {}\n",
                self.pending_approvals
            ));
        }
        if let Some(next) = &self.next_scheduled {
            out.push_str(&format!("- next scheduled: {next}\n"));
        }
        out
    }
}

/// The only three things a tick may decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeartbeatDecision {
    /// Say nothing. Write nothing. Count it and move on.
    Silent,
    /// Something worth offering, never done without the owner: read-only work
    /// or a draft. Proposals show up in Today/Approvals.
    Propose,
    /// Something needs the owner now (a failure, an alarm, an approval).
    Notify,
}

/// One thing worth telling the owner, with why it is worth telling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatItem {
    pub kind: HeartbeatDecision,
    pub title: String,
    pub detail: String,
    /// True when this is about the owner's decision (never deferred).
    #[serde(default)]
    pub urgent: bool,
}

/// What one tick produced, for the journal and for the tests.
#[derive(Debug, Clone, PartialEq)]
pub struct HeartbeatOutcome {
    pub decision: HeartbeatDecision,
    /// Items that survived the budget and the de-duplication.
    pub items: Vec<HeartbeatItem>,
    /// False when the tick was skipped before the model was called.
    pub ran: bool,
    pub estimate_input_tokens: usize,
    /// Candidates dropped because the entity already said something similar.
    pub dropped_duplicates: usize,
    /// Candidates dropped because they were not actionable at all.
    pub dropped_unactionable: usize,
    /// Why the tick was skipped, when it was.
    pub skip_reason: Option<String>,
}

impl HeartbeatOutcome {
    /// A tick that ran, changed nothing and said nothing. This is the *good*
    /// outcome and the default on an unchanged workspace.
    pub fn silent(estimate: usize) -> Self {
        Self {
            decision: HeartbeatDecision::Silent,
            items: Vec::new(),
            ran: true,
            estimate_input_tokens: estimate,
            dropped_duplicates: 0,
            dropped_unactionable: 0,
            skip_reason: None,
        }
    }

    pub fn skipped(reason: &str, estimate: usize) -> Self {
        Self {
            decision: HeartbeatDecision::Silent,
            items: Vec::new(),
            ran: false,
            estimate_input_tokens: estimate,
            dropped_duplicates: 0,
            dropped_unactionable: 0,
            skip_reason: Some(reason.to_string()),
        }
    }

    /// Did this tick write anything the owner will see?
    pub fn is_quiet(&self) -> bool {
        self.items.is_empty()
    }
}

/// Decide whether a tick is even allowed to start.
///
/// Two refusals, both cheap: an empty snapshot has nothing to reason about
/// (silence must not depend on the model behaving), and a prompt over the
/// ceiling is skipped rather than trimmed — truncating a checklist silently
/// would drop exactly the line the owner cared about.
pub fn plan_tick(checklist: &str, snapshot: &HeartbeatSnapshot) -> Result<usize, HeartbeatOutcome> {
    let prompt = snapshot.render(checklist);
    let estimate = estimate_tokens(&prompt) + 120; // + system prompt allowance
    if snapshot.is_empty() {
        return Err(HeartbeatOutcome::skipped(
            "nothing changed since the last tick",
            estimate,
        ));
    }
    if estimate > HEARTBEAT_MAX_INPUT_TOKENS {
        return Err(HeartbeatOutcome::skipped(
            "state snapshot exceeds the per-tick token ceiling",
            estimate,
        ));
    }
    Ok(estimate)
}

/// The pure filter between "the model said something" and "the owner is told".
///
/// Order matters: urgency first (a failing routine must survive a full
/// notification budget), then de-duplication, then the daily cap.
pub fn filter_items(
    candidates: Vec<HeartbeatItem>,
    recent_texts: &[String],
    notifications_used_today: usize,
) -> (Vec<HeartbeatItem>, usize, usize, HeartbeatDecision) {
    let mut kept: Vec<HeartbeatItem> = Vec::new();
    let mut duplicates = 0usize;
    let mut unactionable = 0usize;
    let mut notifications = notifications_used_today;

    // Urgent items first so the cap can never starve them.
    let mut ordered = candidates;
    ordered.sort_by_key(|i| !i.urgent);

    for item in ordered {
        let title = item.title.trim();
        let detail = item.detail.trim();
        if title.is_empty() && detail.is_empty() {
            unactionable += 1;
            continue;
        }
        if item.kind == HeartbeatDecision::Silent {
            unactionable += 1;
            continue;
        }
        let text = format!("{title} {detail}");
        if recent_texts
            .iter()
            .take(DEDUP_WINDOW)
            .any(|previous| dedup::jaccard(previous, &text) >= DEDUP_THRESHOLD)
        {
            duplicates += 1;
            continue;
        }
        if item.kind == HeartbeatDecision::Notify {
            if notifications >= NOTIFY_DAILY_CAP && !item.urgent {
                // Over the daily budget: keep the information, drop the
                // interruption. It becomes a proposal the owner sees at leisure.
                kept.push(HeartbeatItem {
                    kind: HeartbeatDecision::Propose,
                    ..item
                });
                continue;
            }
            notifications += 1;
        }
        kept.push(item);
    }

    let decision = if kept.iter().any(|i| i.kind == HeartbeatDecision::Notify) {
        HeartbeatDecision::Notify
    } else if kept.iter().any(|i| i.kind == HeartbeatDecision::Propose) {
        HeartbeatDecision::Propose
    } else {
        HeartbeatDecision::Silent
    };

    (kept, duplicates, unactionable, decision)
}

/// Parse the model's structured answer. Tolerant of the usual mangling (a code
/// fence, prose around the JSON) and deliberately strict about the decision
/// vocabulary: an unknown decision is not guessed, it is silent.
pub fn parse_decision(reply: &str) -> Option<(HeartbeatDecision, Vec<HeartbeatItem>)> {
    let value = crate::llm::extract_json(reply)?;
    let decision = match value.get("decision").and_then(|d| d.as_str())? {
        "silent" => HeartbeatDecision::Silent,
        "propose" => HeartbeatDecision::Propose,
        "notify" => HeartbeatDecision::Notify,
        _ => return None,
    };
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|raw| {
                    let title = raw
                        .get("title")
                        .and_then(|t| t.as_str())?
                        .trim()
                        .to_string();
                    let detail = raw
                        .get("detail")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let kind = match raw.get("kind").and_then(|k| k.as_str()) {
                        Some("notify") => HeartbeatDecision::Notify,
                        Some("propose") => HeartbeatDecision::Propose,
                        _ => decision,
                    };
                    Some(HeartbeatItem {
                        kind,
                        title,
                        detail,
                        urgent: raw.get("urgent").and_then(|u| u.as_bool()).unwrap_or(false),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some((decision, items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: HeartbeatDecision, title: &str) -> HeartbeatItem {
        HeartbeatItem {
            kind,
            title: title.to_string(),
            detail: String::new(),
            urgent: false,
        }
    }

    #[test]
    fn an_unchanged_workspace_produces_a_skipped_silent_tick() {
        // The G-01 regression: 20 ticks on nothing must write nothing at all.
        let snapshot = HeartbeatSnapshot::default();
        for _ in 0..20 {
            let outcome = plan_tick("check the disk", &snapshot).unwrap_err();
            assert!(!outcome.ran);
            assert!(outcome.is_quiet());
            assert_eq!(outcome.decision, HeartbeatDecision::Silent);
            assert_eq!(outcome.items.len(), 0, "silence must write no items");
        }
    }

    #[test]
    fn a_real_change_makes_the_tick_worth_running() {
        let snapshot = HeartbeatSnapshot {
            new_files: vec!["report.pdf".into()],
            ..Default::default()
        };
        assert!(plan_tick("check the disk", &snapshot).is_ok());
    }

    #[test]
    fn a_pathological_snapshot_is_skipped_not_truncated() {
        // A snapshot that renders past the ceiling must be refused *before* the
        // model is called — never trimmed down to fit, because trimming is how
        // you silently drop the one line that mattered.
        let big = "a very long checklist line that the owner wrote ".repeat(400);
        let snapshot = HeartbeatSnapshot {
            new_files: vec!["report.pdf".into()],
            ..Default::default()
        };
        let outcome = plan_tick(&big, &snapshot).unwrap_err();
        assert!(!outcome.ran);
        assert_eq!(
            outcome.skip_reason.as_deref(),
            Some("state snapshot exceeds the per-tick token ceiling")
        );
    }

    #[test]
    fn the_snapshot_rendering_is_bounded() {
        let snapshot = HeartbeatSnapshot {
            new_files: (0..500).map(|i| format!("f{i}")).collect(),
            pending_approvals: 3,
            ..Default::default()
        };
        let rendered = snapshot.render("line");
        assert!(rendered.contains("500 total"));
        // Only a handful of examples, never the whole list.
        assert!(rendered.matches("f4").count() <= 2);
    }

    #[test]
    fn duplicates_are_dropped_against_recent_history() {
        let recent = vec!["Tidy the Downloads folder by file type".to_string()];
        let (kept, dupes, _, decision) = filter_items(
            vec![
                item(
                    HeartbeatDecision::Propose,
                    "Tidy the Downloads folder by file type",
                ),
                item(
                    HeartbeatDecision::Propose,
                    "Check whether backups ran last night",
                ),
            ],
            &recent,
            0,
        );
        assert_eq!(dupes, 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "Check whether backups ran last night");
        assert_eq!(decision, HeartbeatDecision::Propose);
    }

    #[test]
    fn notifications_are_capped_but_information_is_not_lost() {
        let candidates = vec![
            item(HeartbeatDecision::Notify, "disk is nearly full"),
            item(HeartbeatDecision::Notify, "a routine failed"),
            item(HeartbeatDecision::Notify, "another routine failed"),
        ];
        // Start with one notification already used: of the three, one may still
        // interrupt and the other two are downgraded to proposals — nothing is
        // thrown away, it just stops interrupting.
        let (kept, _, _, decision) = filter_items(candidates, &[], NOTIFY_DAILY_CAP - 1);
        assert_eq!(kept.len(), 3, "nothing is dropped, only downgraded");
        assert_eq!(
            kept.iter()
                .filter(|i| i.kind == HeartbeatDecision::Notify)
                .count(),
            1
        );
        assert_eq!(
            kept.iter()
                .filter(|i| i.kind == HeartbeatDecision::Propose)
                .count(),
            2
        );
        assert_eq!(decision, HeartbeatDecision::Notify);
    }

    #[test]
    fn urgent_items_survive_a_full_notification_budget() {
        let mut urgent = item(HeartbeatDecision::Notify, "mission failed twice");
        urgent.urgent = true;
        let (kept, _, _, _) = filter_items(vec![urgent], &[], NOTIFY_DAILY_CAP + 5);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, HeartbeatDecision::Notify);
    }

    #[test]
    fn silent_candidates_are_counted_as_unactionable() {
        let (kept, _, unactionable, decision) = filter_items(
            vec![item(HeartbeatDecision::Silent, "nothing to do")],
            &[],
            0,
        );
        assert!(kept.is_empty());
        assert_eq!(unactionable, 1);
        assert_eq!(decision, HeartbeatDecision::Silent);
    }

    #[test]
    fn empty_candidates_are_not_a_notification() {
        let (kept, _, _, decision) = filter_items(Vec::new(), &[], 0);
        assert!(kept.is_empty());
        assert_eq!(decision, HeartbeatDecision::Silent);
    }

    #[test]
    fn parsing_is_tolerant_but_never_guesses_a_decision() {
        let ok = r#"```json
        {"decision":"propose","items":[{"title":"Tidy Downloads","detail":"12 files"}]}
        ```"#;
        let (decision, items) = parse_decision(ok).expect("parses through a fence");
        assert_eq!(decision, HeartbeatDecision::Propose);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Tidy Downloads");

        // An unknown decision is silence, not a guess.
        assert!(parse_decision(r#"{"decision":"sleep"}"#).is_none());
        assert!(parse_decision("no json here").is_none());
        // A missing `items` array is fine — the decision still counts.
        let (decision, items) = parse_decision(r#"{"decision":"silent"}"#).unwrap();
        assert_eq!(decision, HeartbeatDecision::Silent);
        assert!(items.is_empty());
    }

    #[test]
    fn the_tick_estimate_stays_under_the_ceiling_for_a_normal_workspace() {
        let snapshot = HeartbeatSnapshot {
            new_files: vec!["a".into(), "b".into()],
            due_routines: vec!["daily brief".into()],
            pending_approvals: 1,
            next_scheduled: Some("09:00 daily brief".into()),
            ..Default::default()
        };
        let estimate = plan_tick(&"x".repeat(200), &snapshot).unwrap();
        assert!(
            estimate < HEARTBEAT_MAX_INPUT_TOKENS / 2,
            "a normal tick should use nowhere near the ceiling (got {estimate})"
        );
    }

    #[test]
    fn token_estimation_is_an_estimate_not_a_promise() {
        assert!(estimate_tokens("") >= 1);
        assert_eq!(estimate_tokens("abcd"), 2);
        // Arabic costs more bytes per character; the estimate is deliberately
        // rough and only used to bound a prompt.
        assert!(estimate_tokens("مرحبا") >= 1);
    }
}
