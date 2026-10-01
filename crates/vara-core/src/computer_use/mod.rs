//! The ActLoop — Vara's native see→act→confirm engine.
//!
//! The loop enforces, structurally, the discipline the owner's MCP taught:
//! 1. Nothing runs until the whole sequence parses (validate-then-execute).
//! 2. Coordinates must be grounded — a SEE op must precede any coordinate op;
//!    the loop refuses ungrounded actions instead of trusting luck.
//! 3. Mutating ops accumulate `pending_verify`; a sequence that ends on an
//!    unverified mutation gets one final screenshot as evidence (the Confirm
//!    step is structural, not optional advice).
//! 4. Destructive close ops stay dry runs unless the owner's policy unlocked
//!    L2 AND the op carries `confirm=true`.
//! 5. One bounded correction: after a failure with no prior mutation, the
//!    loop may re-see and retry once. After any mutation, failures are
//!    reported — never blindly repeated.

pub mod mock;
pub mod ops;
pub mod sidecar;

pub use mock::{MockComputerUse, MockWidget, MockWindow, WidgetKind};
pub use ops::{is_destructive_combo, CuOp, CuResult, CuSequence, GrantLevel};
pub use sidecar::SidecarComputerUse;

use crate::{Result, VaraError};

/// What the owner's policy currently permits without a fresh approval.
#[derive(Debug, Clone, Copy)]
pub struct LoopPolicy {
    /// Highest grant level allowed to execute right now.
    pub max_grant: GrantLevel,
    /// Append one final screenshot when the sequence ends unverified.
    pub final_evidence: bool,
    /// Bounded correction retries (contract: at most one).
    pub correction_retries: u8,
}

impl Default for LoopPolicy {
    fn default() -> Self {
        Self {
            max_grant: GrantLevel::L1,
            final_evidence: true,
            correction_retries: 1,
        }
    }
}

/// One executed step, for the journal and for discipline metrics.
#[derive(Debug, Clone)]
pub struct StepRecord {
    pub index: usize,
    pub op: String,
    pub grant: GrantLevel,
    pub ok: bool,
    pub dry_run: bool,
    pub grounded: bool,
    pub result: CuResult,
}

/// Aggregate outcome of one loop run.
#[derive(Debug, Clone, Default)]
pub struct LoopReport {
    pub steps: Vec<StepRecord>,
    pub failed_at: Option<usize>,
    pub error: Option<String>,
    /// Coordinate ops issued before any SEE op (should stay zero).
    pub blind_refusals: usize,
    pub mutations: usize,
    pub verified_mutations: usize,
    /// Executed after an approval-less gate rejection? Never — but the report
    /// records the requested level when the policy stopped it.
    pub requested_grant: Option<GrantLevel>,
    pub completed: bool,
    pub retries_used: u8,
    /// Final evidence capture path when `final_evidence` kicked in.
    pub evidence_path: Option<String>,
}

impl LoopReport {
    /// Share of mutating ops that received observation evidence afterwards.
    pub fn verify_discipline(&self) -> f64 {
        if self.mutations == 0 {
            return 1.0;
        }
        self.verified_mutations as f64 / self.mutations as f64
    }
    /// True when no ungrounded coordinate action was even attempted.
    pub fn grounded_clean(&self) -> bool {
        self.blind_refusals == 0
    }
}

/// The adapter every executor implements: mock in tests, MCP sidecar on
/// Windows, future Rust-native ports behind the same contract.
pub trait ComputerUseAdapter {
    fn execute(&mut self, op: &CuOp) -> CuResult;
}

/// Runs a validated sequence against an adapter under a policy.
pub struct ActLoop<'a> {
    adapter: &'a mut dyn ComputerUseAdapter,
    policy: LoopPolicy,
}

impl<'a> ActLoop<'a> {
    pub fn new(adapter: &'a mut dyn ComputerUseAdapter, policy: LoopPolicy) -> Self {
        Self { adapter, policy }
    }

    /// Policy gate BEFORE anything runs: the sequence's max grant must fit.
    pub fn check_policy(&self, seq: &CuSequence) -> std::result::Result<(), String> {
        let need = seq.max_grant();
        if need > self.policy.max_grant {
            return Err(format!(
                "sequence needs {} but policy allows {} — split it or ask the owner to raise the gate",
                need.as_str(),
                self.policy.max_grant.as_str()
            ));
        }
        Ok(())
    }

    /// Execute the sequence. Assumes `check_policy` passed (or you accepted
    /// the requested level after an approval card).
    pub fn run(&mut self, seq: &CuSequence) -> LoopReport {
        let mut report = LoopReport {
            requested_grant: Some(seq.max_grant()),
            ..Default::default()
        };
        let mut grounded = false;
        let mut pending_verify = 0usize;
        let mut ignore_errors = false;
        let mut mutated = false;

        for (i, op) in seq.actions.iter().enumerate() {
            if matches!(op, CuOp::IgnoreErrors) {
                ignore_errors = true;
                continue;
            }
            if matches!(op, CuOp::Wait { .. }) {
                let r = self.adapter.execute(op);
                report.steps.push(StepRecord {
                    index: i,
                    op: op.tag().into(),
                    grant: op.grant_level(),
                    ok: r.ok,
                    dry_run: r.dry_run.unwrap_or(false),
                    grounded: true,
                    result: r,
                });
                continue;
            }

            // Grounding discipline: coordinate ops need a prior SEE.
            if op.absolute_coords().is_some() && !grounded {
                report.blind_refusals += 1;
                report.failed_at = Some(i);
                report.error = Some(format!(
                    "action[{i}] ({}) refused: ungrounded coordinates — take a screenshot first",
                    op.tag()
                ));
                report.completed = false;
                return report;
            }

            // Destructive gate: dry runs are fine, confirm=true needs L2.
            let needs_l2_confirm = matches!(
                op,
                CuOp::CloseWindow { confirm: true, .. } | CuOp::CloseApp { confirm: true, .. }
            ) || matches!(op, CuOp::Hotkey { keys, .. } if ops::is_destructive_combo(keys));
            if needs_l2_confirm && self.policy.max_grant < GrantLevel::L2 {
                report.failed_at = Some(i);
                report.error = Some(format!(
                    "action[{i}] ({}) is destructive — policy does not allow L2 right now",
                    op.tag()
                ));
                report.completed = false;
                return report;
            }

            let grant = op.grant_level();
            let result = self.adapter.execute(op);

            if result.ok && op.is_see() {
                grounded = true;
            }
            if result.ok && op.is_mutating() {
                pending_verify += 1;
                mutated = true;
                report.mutations += 1;
            }
            // A SEE after a mutation is the Confirm step.
            if result.ok && op.is_see() && pending_verify > 0 {
                report.verified_mutations += pending_verify;
                pending_verify = 0;
            }

            let ok = result.ok;
            let dry = result.dry_run.unwrap_or(false);
            report.steps.push(StepRecord {
                index: i,
                op: op.tag().into(),
                grant,
                ok,
                dry_run: dry,
                grounded: true,
                result,
            });

            if !ok {
                // A destructive dry run is an intentional stop, not an error.
                if dry {
                    report.completed = false;
                    report.error = Some(format!(
                        "action[{i}] ({}) stopped on a dry run — ask the owner, then confirm",
                        op.tag()
                    ));
                    report.failed_at = Some(i);
                    return report;
                }
                if ignore_errors {
                    continue;
                }
                if !seq.stop_on_error {
                    continue;
                }
                report.failed_at = Some(i);
                report.error = report.steps.last().and_then(|s| s.result.error.clone());
                report.completed = false;
                return report;
            }
        }

        // Confirm is structural: end unverified → one evidence capture.
        if pending_verify > 0 && self.policy.final_evidence {
            let ev = self.adapter.execute(&CuOp::Screenshot {
                region: None,
                settle: 0.1,
            });
            if ev.ok {
                report.verified_mutations += pending_verify;
                report.evidence_path = ev.path.clone();
            }
        }

        report.completed = true;
        let _ = mutated;
        report
    }

    /// Bounded correction: retry the whole sequence ONCE, and only when the
    /// first attempt failed before mutating anything (a fresh SEE precedes
    /// it). After any mutation, the loop reports instead of repeating.
    pub fn run_with_correction(&mut self, seq: &CuSequence) -> LoopReport {
        let first = self.run(seq);
        if first.completed || self.policy.correction_retries == 0 {
            return first;
        }
        let mutated_before_failure = first
            .failed_at
            .map(|f| seq.actions[..f].iter().any(|o| o.is_mutating()))
            .unwrap_or(true);
        if mutated_before_failure {
            return first;
        }
        // Re-see, then one retry.
        let _ = self.adapter.execute(&CuOp::Screenshot {
            region: None,
            settle: 1.0,
        });
        let mut retry = self.run(seq);
        retry.retries_used = 1;
        retry
    }
}

/// Parse + gate + run in one call — the shell's entry point.
pub fn run_sequence_json(
    adapter: &mut dyn ComputerUseAdapter,
    policy: LoopPolicy,
    json: &str,
) -> Result<LoopReport> {
    let seq = CuSequence::parse(json).map_err(VaraError::Other)?;
    let mut loop_ = ActLoop::new(adapter, policy);
    loop_.check_policy(&seq).map_err(VaraError::Other)?;
    Ok(loop_.run(&seq))
}
