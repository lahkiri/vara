//! Goals — an outcome the entity keeps working toward, with a *measured*
//! stopping condition.
//!
//! The owner's requirement is the one thing a chat wrapper cannot do: "give it
//! a goal like *reach the first 10 real customers* and it does not stop until it
//! is achieved". That needs four properties, and this module is only about
//! those:
//!
//! 1. **A goal is a record, not a message.** It has an id, a state, a budget and
//!    a description of *done* — so it survives a restart and can be resumed.
//! 2. **"Done" is a measure, never an opinion.** [`Success`] is either a number
//!    that must be reached or a checklist; there is deliberately no variant
//!    meaning "the model felt finished". A goal without a measure is refused at
//!    construction, because "keep going until it feels done" is how an agent
//!    burns a budget forever.
//! 3. **Progress is recorded, not inferred.** Every step appends a [`Step`] with
//!    what changed; the next decision reads that history.
//! 4. **Giving up is a state, not a crash.** `Blocked` carries a concrete
//!    reason, so the UI can explain why rather than pretending to work.
//!
//! What this module is **not**: it does not call a model, does not touch the
//! disk and does not execute anything. It decides *what the next step should
//! be*, and the caller — a surface, through the gate — decides whether to do it.
//! That split is what keeps a long-running goal from becoming a long-running
//! privilege.

use serde::{Deserialize, Serialize};

/// How the entity knows it is finished.
///
/// There is deliberately no `ModelDecides` variant. If a goal could be declared
/// complete by the same model that is trying to complete it, "does not stop
/// until it is achieved" would mean "stops when it says so" — which is exactly
/// the failure mode this type exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Success {
    /// A number that must reach a threshold: `signups >= 10`.
    Metric {
        /// What is being counted, in the owner's words.
        name: String,
        /// Current value, updated by recorded evidence only.
        current: i64,
        target: i64,
    },
    /// Every item must be true. Used when the outcome is a set of facts.
    Checklist { items: Vec<ChecklistItem> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub text: String,
    pub done: bool,
}

impl Success {
    pub fn metric(name: &str, target: i64) -> Self {
        Success::Metric {
            name: name.to_string(),
            current: 0,
            target,
        }
    }

    pub fn checklist(items: &[&str]) -> Self {
        Success::Checklist {
            items: items
                .iter()
                .map(|t| ChecklistItem {
                    text: t.to_string(),
                    done: false,
                })
                .collect(),
        }
    }

    /// Is the stopping condition met right now?
    pub fn is_met(&self) -> bool {
        match self {
            Success::Metric {
                current, target, ..
            } => current >= target,
            Success::Checklist { items } => !items.is_empty() && items.iter().all(|i| i.done),
        }
    }

    /// How far along, 0.0..=1.0. Reported to the owner, never used to decide
    /// completion — `is_met` decides.
    pub fn fraction(&self) -> f64 {
        match self {
            Success::Metric {
                current, target, ..
            } => {
                if *target <= 0 {
                    0.0
                } else {
                    (*current as f64 / *target as f64).clamp(0.0, 1.0)
                }
            }
            Success::Checklist { items } => {
                if items.is_empty() {
                    0.0
                } else {
                    items.iter().filter(|i| i.done).count() as f64 / items.len() as f64
                }
            }
        }
    }

    /// One line a UI can show without interpreting the variant.
    pub fn describe(&self) -> String {
        match self {
            Success::Metric {
                name,
                current,
                target,
            } => format!("{name}: {current}/{target}"),
            Success::Checklist { items } => {
                let done = items.iter().filter(|i| i.done).count();
                format!("{done}/{} items done", items.len())
            }
        }
    }
}

/// Where a goal is in its life. `Blocked` and `Achieved` are terminal until the
/// owner intervenes — an entity that can un-block itself has no use for the
/// state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    Active,
    Paused,
    /// Waiting on the owner: approval, a decision, or information only they have.
    Waiting,
    /// Stopped by a concrete, recorded obstacle.
    Blocked {
        reason: String,
    },
    Achieved,
    Abandoned {
        reason: String,
    },
}

impl GoalState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, GoalState::Achieved | GoalState::Abandoned { .. })
    }
    pub fn is_workable(&self) -> bool {
        matches!(self, GoalState::Active)
    }
    pub fn label(&self) -> &'static str {
        match self {
            GoalState::Active => "active",
            GoalState::Paused => "paused",
            GoalState::Waiting => "waiting",
            GoalState::Blocked { .. } => "blocked",
            GoalState::Achieved => "achieved",
            GoalState::Abandoned { .. } => "abandoned",
        }
    }
}

/// One recorded attempt. Steps are evidence: what was tried, and what it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// Monotonic within a goal, so ordering survives a restart.
    pub index: u32,
    pub at_unix: i64,
    /// What was done, in one line.
    pub action: String,
    /// What changed as a result — the only thing that moves a metric.
    pub outcome: StepOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// The measure moved. `evidence` must say *how* it was observed.
    Progressed { evidence: String },
    /// Tried, and nothing changed. Recorded so the same step is not repeated
    /// blindly forever.
    NoChange { reason: String },
    /// Could not be done, with the concrete obstacle.
    Failed { error: String },
    /// The step needs the owner before it can proceed.
    NeedsOwner { question: String },
}

/// A goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    /// The owner's own words — kept verbatim so the intent is never paraphrased
    /// away by the entity.
    pub brief: String,
    pub success: Success,
    pub state: GoalState,
    pub created_unix: i64,
    pub updated_unix: i64,
    pub steps: Vec<Step>,
    /// Token budget for the whole goal.
    pub budget_tokens: i64,
    pub spent_tokens: i64,
    /// Ceiling on steps, so a goal that never progresses cannot run forever.
    pub max_steps: u32,
    /// Set when the goal was achieved: the evidence that the measure was met.
    pub achieved_by: Option<String>,
}

impl Goal {
    /// Build a goal, refusing one whose completion cannot be measured.
    pub fn new(
        id: &str,
        title: &str,
        brief: &str,
        success: Success,
        now_unix: i64,
        budget_tokens: i64,
        max_steps: u32,
    ) -> Result<Self, GoalError> {
        if title.trim().is_empty() {
            return Err(GoalError::NoTitle);
        }
        if brief.trim().is_empty() {
            return Err(GoalError::NoBrief);
        }
        match &success {
            Success::Metric { target, name, .. } => {
                if *target <= 0 {
                    return Err(GoalError::Unmeasurable {
                        why: "a metric target must be greater than zero".into(),
                    });
                }
                if name.trim().is_empty() {
                    return Err(GoalError::Unmeasurable {
                        why: "a metric needs a name so the owner knows what is counted".into(),
                    });
                }
            }
            Success::Checklist { items } => {
                if items.is_empty() {
                    return Err(GoalError::Unmeasurable {
                        why: "an empty checklist is already true, which would end the goal at once"
                            .into(),
                    });
                }
            }
        }
        if budget_tokens <= 0 {
            return Err(GoalError::NoBudget);
        }
        if max_steps == 0 {
            return Err(GoalError::NoBudget);
        }
        Ok(Self {
            id: id.to_string(),
            title: title.to_string(),
            brief: brief.to_string(),
            success,
            state: GoalState::Active,
            created_unix: now_unix,
            updated_unix: now_unix,
            steps: Vec::new(),
            budget_tokens,
            spent_tokens: 0,
            max_steps,
            achieved_by: None,
        })
    }

    /// Why the goal cannot currently take another step, if it cannot.
    pub fn next_blocker(&self) -> Option<BlockReason> {
        if self.state.is_terminal() {
            return Some(BlockReason::Finished(self.state.clone()));
        }
        if !self.state.is_workable() {
            return Some(BlockReason::NotActive(self.state.clone()));
        }
        if self.success.is_met() {
            // Should have been recorded by `record`, but never silently continue.
            return Some(BlockReason::AlreadyMet);
        }
        if self.spent_tokens >= self.budget_tokens {
            return Some(BlockReason::OutOfBudget {
                spent: self.spent_tokens,
                budget: self.budget_tokens,
            });
        }
        if self.steps.len() as u32 >= self.max_steps {
            return Some(BlockReason::OutOfSteps {
                steps: self.steps.len() as u32,
                max: self.max_steps,
            });
        }
        None
    }

    pub fn can_continue(&self) -> bool {
        self.next_blocker().is_none()
    }

    /// Record one step. This is the only way a metric moves.
    ///
    /// Returns the error when the goal cannot continue, so a caller cannot
    /// accidentally keep spending on a finished or blocked goal.
    pub fn record(&mut self, step: Step) -> Result<StepEffect, GoalError> {
        if let Some(blocker) = self.next_blocker() {
            return Err(GoalError::CannotContinue(blocker));
        }
        let index = self.steps.len() as u32;
        let mut step = step;
        step.index = index;
        if step.at_unix < self.updated_unix {
            step.at_unix = self.updated_unix;
        }
        self.updated_unix = step.at_unix;
        self.spent_tokens += 0; // tokens are accounted by the caller
        self.steps.push(step.clone());

        let effect = match &step.outcome {
            StepOutcome::Progressed { .. } => {
                // A metric advances only when the step says it did, and the
                // evidence travels with it in the recorded step.
                if let Success::Metric { current, .. } = &mut self.success {
                    *current += 1;
                }
                StepEffect::Progressed
            }
            StepOutcome::NoChange { .. } => StepEffect::NoChange,
            StepOutcome::Failed { .. } => StepEffect::Failed,
            StepOutcome::NeedsOwner { question } => {
                self.state = GoalState::Waiting;
                StepEffect::Waiting {
                    question: question.clone(),
                }
            }
        };

        if self.success.is_met() {
            let evidence = match &step.outcome {
                StepOutcome::Progressed { evidence } => evidence.clone(),
                _ => format!("step {} completed the final condition", index),
            };
            self.state = GoalState::Achieved;
            self.achieved_by = Some(evidence);
            return Ok(StepEffect::Achieved);
        }
        Ok(effect)
    }

    /// Move the checklist. Separate from `record` because ticking an item is the
    /// owner's or a verifier's act, not progress the entity reports about
    /// itself.
    pub fn complete_item(&mut self, text: &str) -> Result<(), GoalError> {
        match &mut self.success {
            Success::Checklist { items } => {
                let item = items
                    .iter_mut()
                    .find(|i| i.text == text)
                    .ok_or_else(|| GoalError::UnknownItem(text.to_string()))?;
                item.done = true;
                if self.success.is_met() {
                    self.state = GoalState::Achieved;
                    self.achieved_by = Some(format!("checklist item '{text}' completed"));
                }
                Ok(())
            }
            Success::Metric { .. } => Err(GoalError::NotAChecklist),
        }
    }

    /// Charge tokens spent by a step. Kept explicit so the budget is never a
    /// surprise: an over-budget goal stops rather than continuing.
    pub fn charge(&mut self, tokens: i64) {
        self.spent_tokens += tokens.max(0);
    }

    pub fn block(&mut self, reason: &str) {
        if reason.trim().is_empty() {
            // A blocked goal with no reason is indistinguishable from a broken
            // one; refuse to enter the state at all.
            return;
        }
        if !self.state.is_terminal() {
            self.state = GoalState::Blocked {
                reason: reason.to_string(),
            };
        }
    }

    pub fn remaining_tokens(&self) -> i64 {
        (self.budget_tokens - self.spent_tokens).max(0)
    }

    pub fn remaining_steps(&self) -> u32 {
        self.max_steps.saturating_sub(self.steps.len() as u32)
    }

    /// Consecutive `NoChange` steps at the end of the history.
    ///
    /// The entity uses this to notice it is spinning: repeating a step that
    /// changed nothing is the most common way a goal-writer wastes a budget.
    pub fn stalled_for(&self) -> u32 {
        self.steps
            .iter()
            .rev()
            .take_while(|s| matches!(s.outcome, StepOutcome::NoChange { .. }))
            .count() as u32
    }

    /// One-line status for a UI.
    pub fn summary(&self) -> String {
        format!(
            "{} — {} ({}) · {}/{} steps · {}% of budget",
            self.title,
            self.success.describe(),
            self.state.label(),
            self.steps.len(),
            self.max_steps,
            if self.budget_tokens > 0 {
                (self.spent_tokens * 100 / self.budget_tokens).min(100)
            } else {
                0
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepEffect {
    Progressed,
    NoChange,
    Failed,
    Waiting { question: String },
    Achieved,
}

/// Why a goal cannot take another step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    Finished(GoalState),
    NotActive(GoalState),
    AlreadyMet,
    OutOfBudget { spent: i64, budget: i64 },
    OutOfSteps { steps: u32, max: u32 },
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockReason::Finished(state) => write!(f, "the goal is {}", state.label()),
            BlockReason::NotActive(state) => write!(f, "the goal is {}", state.label()),
            BlockReason::AlreadyMet => write!(f, "the stopping condition is already met"),
            BlockReason::OutOfBudget { spent, budget } => {
                write!(f, "out of budget ({spent}/{budget} tokens)")
            }
            BlockReason::OutOfSteps { steps, max } => {
                write!(f, "out of steps ({steps}/{max})")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalError {
    NoTitle,
    NoBrief,
    Unmeasurable { why: String },
    NoBudget,
    CannotContinue(BlockReason),
    NotAChecklist,
    UnknownItem(String),
}

impl std::fmt::Display for GoalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalError::NoTitle => write!(f, "a goal needs a title"),
            GoalError::NoBrief => write!(f, "a goal needs the owner's own words, verbatim"),
            GoalError::Unmeasurable { why } => write!(f, "the goal cannot be measured: {why}"),
            GoalError::NoBudget => write!(f, "a goal needs a token budget and a step ceiling"),
            GoalError::CannotContinue(reason) => write!(f, "cannot continue: {reason}"),
            GoalError::NotAChecklist => {
                write!(f, "this goal is measured by a metric, not a checklist")
            }
            GoalError::UnknownItem(item) => write!(f, "no checklist item '{item}'"),
        }
    }
}

impl std::error::Error for GoalError {}

/// The whole set of goals, with the selector a caller needs.
#[derive(Debug, Default, Clone)]
pub struct GoalBoard {
    goals: Vec<Goal>,
}

impl GoalBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, goal: Goal) {
        self.goals.push(goal);
    }

    pub fn all(&self) -> &[Goal] {
        &self.goals
    }

    pub fn get(&self, id: &str) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Goal> {
        self.goals.iter_mut().find(|g| g.id == id)
    }

    /// The goal a heartbeat should work on next: the oldest active one that can
    /// actually continue. `None` means there is nothing useful to do — which is
    /// the honest answer, and the one that keeps the entity silent.
    pub fn next_workable(&self) -> Option<&Goal> {
        self.goals
            .iter()
            .filter(|g| g.can_continue())
            .min_by_key(|g| g.created_unix)
    }

    /// Goals nobody can move: the entity is waiting on the owner, or is blocked.
    /// Surfaced so the UI can say what it needs instead of appearing idle.
    pub fn needing_owner(&self) -> Vec<&Goal> {
        self.goals
            .iter()
            .filter(|g| matches!(g.state, GoalState::Waiting | GoalState::Blocked { .. }))
            .collect()
    }

    pub fn achieved(&self) -> Vec<&Goal> {
        self.goals
            .iter()
            .filter(|g| matches!(g.state, GoalState::Achieved))
            .collect()
    }
}

/// How a heartbeat should treat a goal right now.
///
/// The decision is deliberately conservative: an entity that reports on work it
/// did not do is worse than one that stays quiet. `Nothing` is a first-class
/// answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalDecision {
    /// Take another step on this goal.
    Work { id: String, hint: String },
    /// Ask the owner one concrete question.
    Ask { id: String, question: String },
    /// Report a real change to the owner.
    Report { id: String, line: String },
    /// Stay silent. The default, and the correct answer most of the time.
    Nothing,
}

/// Decide what to do about the goals, given how much the owner has authorised.
///
/// `stall_limit` is the number of consecutive no-change steps after which the
/// goal is considered stuck rather than progressing — this is what stops the
/// "never stops improving" loop from becoming a "never stops repeating" loop.
pub fn decide(board: &GoalBoard, stall_limit: u32) -> GoalDecision {
    // 1) Something already needs the owner: say so, once.
    if let Some(goal) = board.needing_owner().first() {
        let question = match &goal.state {
            GoalState::Waiting => goal
                .steps
                .iter()
                .rev()
                .find_map(|s| match &s.outcome {
                    StepOutcome::NeedsOwner { question } => Some(question.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "the goal is waiting on you".into()),
            GoalState::Blocked { reason } => format!("blocked: {reason}"),
            _ => "needs your attention".into(),
        };
        return GoalDecision::Ask {
            id: goal.id.clone(),
            question,
        };
    }

    // 2) Something was achieved since the last look: it is worth one line.
    if let Some(goal) = board.achieved().first() {
        if let Some(evidence) = &goal.achieved_by {
            return GoalDecision::Report {
                id: goal.id.clone(),
                line: format!("{} — {}", goal.title, evidence),
            };
        }
    }

    // 3) Work on the next goal that can actually move.
    if let Some(goal) = board.next_workable() {
        if goal.stalled_for() >= stall_limit {
            return GoalDecision::Ask {
                id: goal.id.clone(),
                question: format!(
                    "{} has not moved in {} steps; should I change the approach or drop it?",
                    goal.title,
                    goal.stalled_for()
                ),
            };
        }
        return GoalDecision::Work {
            id: goal.id.clone(),
            hint: format!("next step toward: {}", goal.success.describe()),
        };
    }

    GoalDecision::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000;

    fn metric_goal(target: i64) -> Goal {
        Goal::new(
            "g1",
            "First ten customers",
            "reach the first 10 real customers",
            Success::metric("signups", target),
            T0,
            50_000,
            40,
        )
        .unwrap()
    }

    fn progressed() -> Step {
        Step {
            index: 0,
            at_unix: T0 + 1,
            action: "shipped the landing page".into(),
            outcome: StepOutcome::Progressed {
                evidence: "one signup arrived through the form".into(),
            },
        }
    }

    #[test]
    fn a_goal_without_a_measure_is_refused() {
        // This is the whole point of the type: "keep going until it feels done"
        // has no representation.
        assert_eq!(
            Goal::new("x", "t", "b", Success::metric("signups", 0), T0, 100, 10).unwrap_err(),
            GoalError::Unmeasurable {
                why: "a metric target must be greater than zero".into()
            }
        );
        assert!(matches!(
            Goal::new("x", "t", "b", Success::checklist(&[]), T0, 100, 10).unwrap_err(),
            GoalError::Unmeasurable { .. }
        ));
    }

    #[test]
    fn a_goal_needs_verbatim_intent_and_a_budget() {
        assert_eq!(
            Goal::new("x", "t", "  ", Success::metric("m", 1), T0, 100, 10).unwrap_err(),
            GoalError::NoBrief
        );
        assert_eq!(
            Goal::new("x", "t", "b", Success::metric("m", 1), T0, 0, 10).unwrap_err(),
            GoalError::NoBudget
        );
        assert_eq!(
            Goal::new("x", "t", "b", Success::metric("m", 1), T0, 100, 0).unwrap_err(),
            GoalError::NoBudget
        );
    }

    #[test]
    fn a_metric_only_moves_when_evidence_says_it_did() {
        let mut goal = metric_goal(3);
        assert_eq!(goal.success.fraction(), 0.0);

        // A no-change step leaves the measure exactly where it was.
        goal.record(Step {
            index: 0,
            at_unix: T0 + 1,
            action: "checked the form".into(),
            outcome: StepOutcome::NoChange {
                reason: "no new signup".into(),
            },
        })
        .unwrap();
        assert_eq!(goal.success.fraction(), 0.0);

        goal.record(progressed()).unwrap();
        assert_eq!(goal.success.fraction(), 1.0 / 3.0);
    }

    #[test]
    fn the_goal_is_achieved_by_the_measure_not_by_an_opinion() {
        let mut goal = metric_goal(2);
        goal.record(progressed()).unwrap();
        assert!(!matches!(goal.state, GoalState::Achieved));
        let effect = goal.record(progressed()).unwrap();
        assert_eq!(effect, StepEffect::Achieved);
        assert!(matches!(goal.state, GoalState::Achieved));
        assert!(goal.achieved_by.is_some());
    }

    #[test]
    fn an_achieved_goal_refuses_further_steps() {
        let mut goal = metric_goal(1);
        goal.record(progressed()).unwrap();
        let err = goal.record(progressed()).unwrap_err();
        assert!(matches!(
            err,
            GoalError::CannotContinue(BlockReason::Finished(_))
        ));
    }

    #[test]
    fn the_budget_is_a_ceiling_not_a_suggestion() {
        let mut goal = metric_goal(100);
        goal.charge(50_000);
        assert_eq!(goal.remaining_tokens(), 0);
        assert_eq!(
            goal.next_blocker(),
            Some(BlockReason::OutOfBudget {
                spent: 50_000,
                budget: 50_000
            })
        );
        assert!(!goal.can_continue());
        // A step after the budget is spent is refused, not quietly recorded.
        assert!(goal.record(progressed()).is_err());
    }

    #[test]
    fn the_step_ceiling_stops_a_goal_that_never_progresses() {
        let mut goal =
            Goal::new("g", "t", "b", Success::metric("m", 100), T0, 1_000_000, 2).unwrap();
        for i in 0..2 {
            goal.record(Step {
                index: i,
                at_unix: T0 + i as i64 + 1,
                action: "tried".into(),
                outcome: StepOutcome::NoChange {
                    reason: "nothing".into(),
                },
            })
            .unwrap();
        }
        assert_eq!(
            goal.next_blocker(),
            Some(BlockReason::OutOfSteps { steps: 2, max: 2 })
        );
    }

    #[test]
    fn a_step_that_needs_the_owner_parks_the_goal() {
        let mut goal = metric_goal(10);
        let effect = goal
            .record(Step {
                index: 0,
                at_unix: T0 + 1,
                action: "wanted to post to the community".into(),
                outcome: StepOutcome::NeedsOwner {
                    question: "may I post from your account?".into(),
                },
            })
            .unwrap();
        assert_eq!(
            effect,
            StepEffect::Waiting {
                question: "may I post from your account?".into()
            }
        );
        assert!(matches!(goal.state, GoalState::Waiting));
        // And it refuses to keep working while it waits.
        assert!(goal.record(progressed()).is_err());
    }

    #[test]
    fn a_blocked_goal_must_carry_a_concrete_reason() {
        let mut goal = metric_goal(10);
        goal.block("   ");
        assert!(
            matches!(goal.state, GoalState::Active),
            "a vague block is refused"
        );
        goal.block("the only channel needs a paid account");
        assert!(matches!(goal.state, GoalState::Blocked { .. }));
    }

    #[test]
    fn stalling_is_measured_from_the_tail_of_the_history() {
        let mut goal = metric_goal(10);
        goal.record(progressed()).unwrap();
        assert_eq!(goal.stalled_for(), 0);
        for i in 0..3 {
            goal.record(Step {
                index: i,
                at_unix: T0 + i as i64 + 2,
                action: "retried".into(),
                outcome: StepOutcome::NoChange {
                    reason: "same result".into(),
                },
            })
            .unwrap();
        }
        assert_eq!(goal.stalled_for(), 3);
    }

    #[test]
    fn the_checklist_is_ticked_by_the_owner_not_by_progress() {
        let mut goal = Goal::new(
            "g",
            "Launch",
            "publish it",
            Success::checklist(&["repo is public", "release is downloadable"]),
            T0,
            10_000,
            20,
        )
        .unwrap();
        assert!(!goal.success.is_met());
        // Progress reports do not tick items.
        goal.record(progressed()).unwrap();
        assert!(!goal.success.is_met());
        goal.complete_item("repo is public").unwrap();
        assert!(!goal.success.is_met());
        goal.complete_item("release is downloadable").unwrap();
        assert!(goal.success.is_met());
        assert!(matches!(goal.state, GoalState::Achieved));
        // An unknown item is an error, not a silent no-op.
        assert_eq!(
            goal.complete_item("invented item").unwrap_err(),
            GoalError::UnknownItem("invented item".into())
        );
    }

    #[test]
    fn the_board_picks_the_oldest_goal_that_can_actually_move() {
        let mut board = GoalBoard::new();
        let mut newer = metric_goal(5);
        newer.id = "newer".into();
        newer.created_unix = T0 + 10;
        let mut older = metric_goal(5);
        older.id = "older".into();
        board.add(newer);
        board.add(older);
        assert_eq!(board.next_workable().unwrap().id, "older");

        // A paused goal is skipped, not chosen and then refused.
        board.get_mut("older").unwrap().state = GoalState::Paused;
        assert_eq!(board.next_workable().unwrap().id, "newer");
    }

    #[test]
    fn the_board_reports_what_needs_the_owner() {
        let mut board = GoalBoard::new();
        let mut waiting = metric_goal(5);
        waiting.id = "waiting".into();
        waiting.state = GoalState::Waiting;
        let mut blocked = metric_goal(5);
        blocked.id = "blocked".into();
        blocked.block("needs a bank account");
        let active = metric_goal(5);
        board.add(active);
        board.add(waiting);
        board.add(blocked);
        let needing = board.needing_owner();
        assert_eq!(needing.len(), 2);
        assert!(needing.iter().all(|g| g.id != "g1"));
    }

    #[test]
    fn the_decision_stays_silent_when_there_is_nothing_to_do() {
        // The default answer must be silence, not invented activity.
        assert_eq!(decide(&GoalBoard::new(), 3), GoalDecision::Nothing);

        let mut paused = GoalBoard::new();
        let mut goal = metric_goal(5);
        goal.state = GoalState::Paused;
        paused.add(goal);
        assert_eq!(decide(&paused, 3), GoalDecision::Nothing);
    }

    #[test]
    fn the_decision_asks_rather_than_spinning() {
        let mut board = GoalBoard::new();
        let mut goal = metric_goal(50);
        for i in 0..4 {
            goal.record(Step {
                index: i,
                at_unix: T0 + i as i64 + 1,
                action: "posted the same link again".into(),
                outcome: StepOutcome::NoChange {
                    reason: "no response".into(),
                },
            })
            .unwrap();
        }
        board.add(goal);
        match decide(&board, 3) {
            GoalDecision::Ask { question, .. } => {
                assert!(question.contains("has not moved"), "{question}");
            }
            other => panic!("expected an Ask, got {other:?}"),
        }
    }

    #[test]
    fn the_decision_surfaces_an_achievement_once() {
        let mut board = GoalBoard::new();
        let mut goal = metric_goal(1);
        goal.record(progressed()).unwrap();
        board.add(goal);
        match decide(&board, 3) {
            GoalDecision::Report { line, .. } => {
                assert!(line.contains("one signup arrived"), "{line}");
            }
            other => panic!("expected a Report, got {other:?}"),
        }
    }

    #[test]
    fn the_decision_asks_the_owners_question_verbatim() {
        let mut board = GoalBoard::new();
        let mut goal = metric_goal(10);
        goal.record(Step {
            index: 0,
            at_unix: T0 + 1,
            action: "wanted to email the list".into(),
            outcome: StepOutcome::NeedsOwner {
                question: "may I email your 40 contacts?".into(),
            },
        })
        .unwrap();
        board.add(goal);
        assert_eq!(
            decide(&board, 3),
            GoalDecision::Ask {
                id: "g1".into(),
                question: "may I email your 40 contacts?".into()
            }
        );
    }

    #[test]
    fn a_workable_goal_produces_work_with_the_measure_in_the_hint() {
        let mut board = GoalBoard::new();
        board.add(metric_goal(10));
        match decide(&board, 3) {
            GoalDecision::Work { id, hint } => {
                assert_eq!(id, "g1");
                assert!(hint.contains("signups: 0/10"), "{hint}");
            }
            other => panic!("expected Work, got {other:?}"),
        }
    }

    #[test]
    fn the_summary_is_one_line_a_ui_can_show() {
        let mut goal = metric_goal(10);
        goal.charge(25_000);
        let line = goal.summary();
        assert!(line.contains("First ten customers"));
        assert!(line.contains("signups: 0/10"));
        assert!(line.contains("active"));
        assert!(line.contains("50% of budget"), "{line}");
    }
}
