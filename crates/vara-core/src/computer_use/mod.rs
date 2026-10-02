//! The ActLoop — Vara's native see→act→confirm engine.
//!
//! The loop enforces, structurally, the discipline the owner's MCP taught:
//! 1. Nothing runs until the whole sequence parses (validate-then-execute).
//! 2. Coordinates must be grounded — a SEE op must precede any pixel target,
//!    absolute or window-relative; the loop refuses ungrounded actions instead
//!    of trusting luck.
//! 3. Mutating ops accumulate `pending_verify`; a sequence that ends on an
//!    unverified mutation gets one final screenshot as evidence (the Confirm
//!    step is structural, not optional advice) — and when the screenshot grant
//!    is off the loop captures nothing and *reports* the mutations it could not
//!    verify, so a run without evidence can never read as verified.
//! 4. Destructive ops are defused by the loop itself: below L2 a close op is
//!    converted to a dry run before any adapter sees it, and a destructive
//!    hotkey is not dispatched at all. The adapter is never asked to really
//!    close something the policy did not grant.
//! 5. One bounded correction: after a failure with no prior mutation, the
//!    loop may re-see and retry once. After any mutation, failures are
//!    reported — never blindly repeated.

pub mod mock;
pub mod ops;
pub mod sidecar;

pub use mock::{MockComputerUse, MockWidget, MockWindow, WidgetKind};
pub use ops::{
    is_destructive_combo, normalize_combo, CuOp, CuResult, CuSequence, GrantLevel, PixelTarget,
};
pub use sidecar::SidecarComputerUse;

use crate::{Result, VaraError};

/// What the owner's policy currently permits without a fresh approval.
#[derive(Debug, Clone, Copy)]
pub struct LoopPolicy {
    /// Highest grant level allowed to execute right now.
    pub max_grant: GrantLevel,
    /// Owner grant for screen capture (`autonomy.allow_screenshots`).
    ///
    /// OFF by default, and enforced here rather than left to the shell: with it
    /// false the loop refuses every SEE op before an adapter is called and
    /// skips its own implicit captures (final evidence, correction re-see).
    /// Capture is a grant, not an adapter feature.
    pub allow_screenshots: bool,
    /// Append one final screenshot when the sequence ends unverified.
    pub final_evidence: bool,
    /// Bounded correction retries (contract: at most one).
    pub correction_retries: u8,
}

impl Default for LoopPolicy {
    fn default() -> Self {
        Self {
            max_grant: GrantLevel::L1,
            // The shipped owner posture: screenshots OFF, no L2. A caller that
            // wants capture has to say so explicitly — see `allow_screenshots`.
            allow_screenshots: false,
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
    /// Mutations that ended the run with no observation evidence. A mutation
    /// without evidence must never be counted as verified, so this is the
    /// honest counter the caller has to look at — `verify_discipline()` drops
    /// below 1.0 whenever it is non-zero.
    pub unverified_mutations: usize,
    /// SEE ops refused by the loop because `allow_screenshots` is false. A
    /// policy denial, recorded here so it is not mistaken for an adapter error
    /// (different failure, different fix).
    pub see_denials: usize,
    /// True once the run went without evidence because the screenshot grant is
    /// off: a refused SEE and/or a skipped implicit capture. A report with this
    /// set is NOT a fully verified run.
    pub screenshots_disabled: bool,
    /// Destructive ops the loop itself defused below L2 (converted to a dry run
    /// or, for destructive hotkeys, never dispatched).
    pub defused_destructive: usize,
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
    /// True only when every mutation this run made was observed afterwards.
    /// Callers must gate "verified" wording on this, not on `completed`.
    pub fn fully_verified(&self) -> bool {
        self.unverified_mutations == 0
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

    /// Policy gate BEFORE anything runs: the sequence's max grant must fit, and
    /// a sequence that wants to SEE must hold the screenshot grant. Checked
    /// here as well as in `run` so a caller can refuse a capture-hungry
    /// sequence without even spawning an adapter.
    pub fn check_policy(&self, seq: &CuSequence) -> std::result::Result<(), String> {
        let need = seq.max_grant();
        if need > self.policy.max_grant {
            return Err(format!(
                "sequence needs {} but policy allows {} — split it or ask the owner to raise the gate",
                need.as_str(),
                self.policy.max_grant.as_str()
            ));
        }
        if !self.policy.allow_screenshots && seq.actions.iter().any(|op| op.is_see()) {
            return Err("sequence captures the screen but screenshots are disabled \
                 (allow_screenshots=false) — the loop will not look at a screen it is \
                 not granted"
                .into());
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

            // Screenshot grant: capture is a policy grant, not an adapter
            // feature, so a SEE is refused HERE — before the adapter is asked —
            // and recorded as a policy denial (its own counter, an error that
            // names the setting) rather than as an adapter error. Continuing
            // would let the rest of the sequence act on evidence that was never
            // taken, so a refused SEE stops the run.
            if op.is_see() && !self.policy.allow_screenshots {
                report.see_denials += 1;
                report.screenshots_disabled = true;
                report.failed_at = Some(i);
                report.error = Some(format!(
                    "action[{i}] ({}) refused: policy denial — screenshots are disabled \
                     (allow_screenshots=false), so the loop did not ask any adapter to capture",
                    op.tag()
                ));
                report.completed = false;
                report.steps.push(StepRecord {
                    index: i,
                    op: op.tag().into(),
                    grant: op.grant_level(),
                    ok: false,
                    dry_run: false,
                    grounded: true,
                    result: denied_result(op, "screenshots are disabled (allow_screenshots=false)"),
                });
                self.seal(&mut report, pending_verify);
                return report;
            }

            // Grounding discipline: every pixel target — absolute or
            // window-relative — needs a prior SEE in this same sequence.
            if let Some(target) = op.pixel_target() {
                if !grounded {
                    let (x, y) = target.coords();
                    report.blind_refusals += 1;
                    report.failed_at = Some(i);
                    report.error = Some(format!(
                        "action[{i}] ({}) refused: ungrounded coordinates ({x},{y} {}) — \
                         take a screenshot first",
                        op.tag(),
                        target.frame()
                    ));
                    report.completed = false;
                    self.seal(&mut report, pending_verify);
                    return report;
                }
            }

            // Destructive gate: the loop owns the defusal, not the adapter.
            // Below L2 a model-authored `confirm` is dropped (the adapter gets
            // an honest dry-run request) and a destructive hotkey — which has
            // no dry-run form — is never dispatched at all.
            let below_l2 = op.is_destructive() && self.policy.max_grant < GrantLevel::L2;
            let mut defused: Option<CuOp> = None;
            let mut not_sent = false;
            if below_l2 {
                match op.as_dry_run() {
                    Some(dry) if dry != *op => {
                        defused = Some(dry);
                        report.defused_destructive += 1;
                    }
                    // Already a dry run: the adapter can only ever preview it.
                    Some(_) => {}
                    // A chord cannot be previewed, so "dry run" means not sent.
                    None => not_sent = true,
                }
            }
            if not_sent {
                report.defused_destructive += 1;
                report.failed_at = Some(i);
                report.error = Some(format!(
                    "action[{i}] ({}) is destructive and policy grants {} — the loop did not \
                     send it to any adapter; ask the owner to unlock L2",
                    op.tag(),
                    self.policy.max_grant.as_str()
                ));
                report.completed = false;
                report.steps.push(StepRecord {
                    index: i,
                    op: op.tag().into(),
                    grant: op.grant_level(),
                    ok: false,
                    dry_run: true,
                    grounded: true,
                    result: dry_run_result(op, self.policy.max_grant),
                });
                self.seal(&mut report, pending_verify);
                return report;
            }
            let sent: &CuOp = defused.as_ref().unwrap_or(op);

            let grant = op.grant_level();
            let result = self.adapter.execute(sent);

            if below_l2 {
                // The loop, not the adapter, decides what "below L2" means: this
                // op was only ever sent as a dry run, so it cannot have closed
                // anything. A success report here means the adapter ignored the
                // defusal — never a verified deed, and never a mutation.
                let mut r = result;
                let claimed_execution = r.ok && !r.dry_run.unwrap_or(false);
                if claimed_execution {
                    r.error = Some(format!(
                        "action[{i}] ({}) was sent as a dry run (policy grants {}, not L2) but \
                         the adapter reported success — the loop does not count this as executed",
                        op.tag(),
                        self.policy.max_grant.as_str()
                    ));
                }
                r.ok = false;
                r.dry_run = Some(true);
                report.steps.push(StepRecord {
                    index: i,
                    op: op.tag().into(),
                    grant,
                    ok: false,
                    dry_run: true,
                    grounded: true,
                    result: r,
                });
                report.failed_at = Some(i);
                report.error = Some(format!(
                    "action[{i}] ({}) defused to a dry run: policy grants {}, not L2 — nothing \
                     was closed",
                    op.tag(),
                    self.policy.max_grant.as_str()
                ));
                report.completed = false;
                self.seal(&mut report, pending_verify);
                return report;
            }

            if result.ok && op.is_see() {
                grounded = true;
            }
            if result.ok && op.is_mutating() {
                pending_verify += 1;
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
                    self.seal(&mut report, pending_verify);
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
                self.seal(&mut report, pending_verify);
                return report;
            }
        }

        // Confirm is structural: end unverified → one evidence capture — but
        // only when the owner granted screenshots. Without the grant the loop
        // must not quietly capture; it reports the gap instead (see `seal`).
        if pending_verify > 0 && self.policy.allow_screenshots && self.policy.final_evidence {
            let ev = self.adapter.execute(&CuOp::Screenshot {
                region: None,
                settle: 0.1,
            });
            if ev.ok {
                report.verified_mutations += pending_verify;
                report.evidence_path = ev.path.clone();
                pending_verify = 0;
            }
        }

        report.completed = true;
        self.seal(&mut report, pending_verify);
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
        // Re-see, then one retry — and the re-see is a capture like any other:
        // without the screenshot grant the loop does not look, and it says so
        // on the retry report instead of leaving a silent hole.
        if self.policy.allow_screenshots {
            let _ = self.adapter.execute(&CuOp::Screenshot {
                region: None,
                settle: 1.0,
            });
        }
        let mut retry = self.run(seq);
        retry.retries_used = 1;
        if !self.policy.allow_screenshots {
            retry.screenshots_disabled = true;
        }
        retry
    }

    /// Close out a report honestly.
    ///
    /// Any mutation still awaiting evidence is counted as `unverified_mutations`
    /// — never folded into `verified_mutations` — and the reason is stated: the
    /// screenshots-off gate when that is what stopped the observation, or a
    /// plain "no evidence" note when the run aborted before the Confirm step.
    /// This is what keeps a partial or capture-less run from reading as
    /// verified.
    fn seal(&self, report: &mut LoopReport, pending_verify: usize) {
        report.unverified_mutations = pending_verify;
        if pending_verify == 0 {
            return;
        }
        let note = if self.policy.allow_screenshots {
            format!("{pending_verify} mutation(s) ran without evidence — this run is NOT verified")
        } else {
            report.screenshots_disabled = true;
            format!(
                "{pending_verify} mutation(s) ran without evidence: screenshots are disabled by \
                 policy (allow_screenshots=false) — this run is NOT verified"
            )
        };
        report.error = Some(match report.error.take() {
            Some(prev) => format!("{prev} — {note}"),
            None => note,
        });
    }
}

/// Receipt for a SEE op the loop refused to send (no screenshot grant). It is
/// a policy denial, so it is shaped like one: the loop explains the setting,
/// and nothing in it can be mistaken for an adapter having tried and failed.
fn denied_result(op: &CuOp, reason: &str) -> CuResult {
    CuResult {
        ok: false,
        op: op.tag().into(),
        error: Some(format!("policy denial — {reason}")),
        active: None,
        path: None,
        before_path: None,
        check: Some(
            "ask the owner to enable screenshots (allow_screenshots) before planning captures"
                .into(),
        ),
        dry_run: None,
        focus: None,
        ms: 0,
        next: Some("re-plan without observation, or request the screenshot grant".into()),
        hint: None,
    }
}

/// Receipt for a destructive op the loop refused to send at all (a chord has
/// no dry-run form). It is a dry run in the honest sense: the loop reports what
/// it declined to do, and which grant would allow it.
fn dry_run_result(op: &CuOp, granted: GrantLevel) -> CuResult {
    CuResult {
        ok: false,
        op: op.tag().into(),
        error: Some(format!(
            "dry run — destructive op not sent (policy grants {}, not L2)",
            granted.as_str()
        )),
        active: None,
        path: None,
        before_path: None,
        check: Some(
            "ask the owner to unlock L2 (computer_use_allow_close) before firing this".into(),
        ),
        dry_run: Some(true),
        focus: None,
        ms: 0,
        next: Some("re-plan without the destructive step, or request the L2 grant".into()),
        hint: None,
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
