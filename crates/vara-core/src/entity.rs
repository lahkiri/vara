//! The autonomy loop — Vara's state machine and mission runner.
//!
//! Design principles (from the v1 experiment, frozen in EXPERIMENT_DESIGN.md):
//! - Token budgets are binding, not advisory.
//! - Notes are deduplicated (Jaccard >= 0.72) so memory never rots.
//! - Reports are written by a *clean-context writer* that sees only the
//!   retrieval ledger — and every report passes the structural provenance
//!   checker before it is delivered. FAIL triggers one repair pass.
//! - Mid-mission replanning keeps the organization alive instead of
//!   committing to a blind handoff (the "drifted researcher" failure).

use crate::db::Database;
use crate::llm::{extract_json, LlmClient};
use crate::provenance;
use crate::tools::{truncate, HttpClient};
use crate::types::*;
use crate::{Result, VaraError};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ---------- events ----------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntityEvent {
    State {
        state: EntityState,
        mission_id: Option<i64>,
    },
    Activity {
        kind: String,
        message: String,
        mission_id: Option<i64>,
    },
    MissionUpdate {
        id: i64,
        status: String,
        steps_done: i64,
        max_steps: i64,
        spent_tokens: i64,
    },
    ReportReady {
        id: i64,
        mission_id: i64,
        verdict: String,
        backed_ratio: f64,
    },
}

pub trait EventSink: Send + Sync {
    fn emit(&self, ev: EntityEvent);
}

/// No-op sink for tests / headless runs.
pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _: EntityEvent) {}
}

/// Tokens reserved for writing (and, if needed, repairing) the report.
///
/// Retrieval stops at `budget - REPORT_BUDGET_TOKENS` so the report can always
/// be written; a mission that spends everything on searching and then cannot
/// afford to say what it found is the worst deal the product can make with the
/// owner's money. The writer's own allowance is additionally capped by whatever
/// has not been spent yet — the reserve is a floor, never a licence to overshoot.
pub const REPORT_BUDGET_TOKENS: i64 = 3_000;

/// What the writer/repair calls may ask the provider for, given the reserve.
fn writer_max_tokens(writer_budget: i64) -> u32 {
    writer_budget.clamp(256, REPORT_BUDGET_TOKENS) as u32
}

// ---------- runtime ----------

pub struct EntityRuntime {
    pub db: Arc<Database>,
    /// Where the mission's live events go.
    ///
    /// This is a **real seam** ([`crate::seams::seam::LOG`]): a surface that has
    /// a host can pass a sink resolved from the plugin host, and unregistering
    /// that plugin stops the events. It is a field rather than a host handle on
    /// purpose — the runtime must stay constructible without a host, because
    /// `vara-tui` and the tests run the same entity with no plugin host at all,
    /// and a capability that is mandatory *there* is not a capability, it is a
    /// hard-coded dependency wearing a seam's name.
    pub sink: Arc<dyn EventSink>,
    pub http: Arc<HttpClient>,
}

/// A holder so the seam value is `Sized`.
///
/// The host stores every service as `Arc<dyn Any>`, so a bare `dyn EventSinkCap`
/// cannot be registered. This wrapper is what a plugin registers and what the
/// runtime reads back.
pub struct LogCap(pub Arc<dyn crate::seams::EventSinkCap>);

/// Resolve the mission event sink from a plugin host.
///
/// Returns the shipped sink when no plugin fills the `log` seam, so a composition
/// that omits it degrades to the built-in behaviour instead of failing at the
/// first event. [`require_log_sink`] is the strict variant.
pub fn sink_from_host(
    host: &Arc<crate::host::Host>,
    fallback: Arc<dyn EventSink>,
) -> Arc<dyn EventSink> {
    host.service::<LogCap>(crate::seams::seam::LOG)
        .map(|cap| Arc::new(SeamSink { cap: cap.0.clone() }) as Arc<dyn EventSink>)
        .unwrap_or(fallback)
}

/// The same resolution, but an empty seam is reported instead of defaulted.
///
/// A mission that silently loses its event stream is a mission the owner cannot
/// watch, so a surface that needs the plugin can refuse to start without it.
pub fn require_log_sink(
    host: &Arc<crate::host::Host>,
) -> std::result::Result<Arc<dyn EventSink>, String> {
    host.service::<LogCap>(crate::seams::seam::LOG)
        .map(|cap| Arc::new(SeamSink { cap: cap.0.clone() }) as Arc<dyn EventSink>)
        .ok_or_else(|| {
            "no plugin fills the `log` seam — the mission would have no event stream".to_string()
        })
}

/// Bridges the `log` seam to the runtime's event stream.
struct SeamSink {
    cap: Arc<dyn crate::seams::EventSinkCap>,
}

impl EventSink for SeamSink {
    fn emit(&self, ev: EntityEvent) {
        // The seam carries one durable line per event. `Arc<dyn EventSinkCap>`
        // derefs to the trait object, so the call goes through the plugin's
        // implementation rather than needing an impl for Arc itself.
        //
        // The body matters: a first draft of this bridge ended up empty after a
        // bad textual patch, so every mission event was received and dropped —
        // `cargo check` passed, and only an assertion on what the plugin
        // recorded caught it. The trivial-looking line is the whole seam.
        let cap: &dyn crate::seams::EventSinkCap = self.cap.as_ref();
        cap.record(ev.kind(), &ev.summary());
    }
}

impl EntityEvent {
    /// A short, stable kind for the durable log (`state`, `activity`, …).
    pub fn kind(&self) -> &'static str {
        match self {
            EntityEvent::State { .. } => "state",
            EntityEvent::Activity { .. } => "activity",
            EntityEvent::MissionUpdate { .. } => "mission",
            EntityEvent::ReportReady { .. } => "report",
        }
    }

    /// A one-line, secret-free summary for the durable log.
    pub fn summary(&self) -> String {
        match self {
            EntityEvent::State { state, .. } => state.as_str().to_string(),
            EntityEvent::Activity { kind, message, .. } => format!("{kind}: {message}"),
            EntityEvent::MissionUpdate {
                status,
                steps_done,
                max_steps,
                ..
            } => format!("{status} {steps_done}/{max_steps}"),
            EntityEvent::ReportReady {
                verdict,
                backed_ratio,
                ..
            } => format!("report {verdict} ({:.0}% backed)", backed_ratio * 100.0),
        }
    }
}

pub struct MissionInputs {
    pub language: String,
    pub paused: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissionOutcome {
    pub mission_id: i64,
    pub status: String,
    pub report_id: Option<i64>,
    pub verdict: Option<String>,
    pub backed_ratio: Option<f64>,
    pub sources: usize,
    pub spent_tokens: i64,
}

/// Repair pass. The brief is built **only** from the gate's deterministic
/// output — the failing citations, the unresolved references and the quotes
/// that were not found in the retrieved text. Research on self-correction is
/// unambiguous that a model asked to critique its own work without external
/// signal at best does nothing and at worst degrades it; the gate is the
/// external signal, so it is the only thing the repair prompt may contain.
/// What the repair pass needs, grouped.
///
/// Six positional arguments where three are `&str` and two are numbers is a
/// call that compiles happily with two of them swapped — and the report, the
/// verdict and the ledger would then silently disagree.
struct RepairInputs<'a> {
    report: &'a str,
    check: &'a ProvenanceResult,
    receipt: &'a provenance::GateReceipt,
    ledger: &'a [RetrievedSource],
    language: &'a str,
    max_tokens: u32,
}

impl EntityRuntime {
    fn set_state(&self, state: EntityState, mission_id: Option<i64>) {
        self.sink.emit(EntityEvent::State { state, mission_id });
        let _ = self.db.insert_event("info", "state", state.as_str());
    }

    fn activity(&self, kind: &str, message: &str, mission_id: Option<i64>) {
        self.sink.emit(EntityEvent::Activity {
            kind: kind.to_string(),
            message: message.to_string(),
            mission_id,
        });
        let _ = self.db.insert_event("info", kind, message);
    }

    // ---------- mission lifecycle ----------

    pub async fn run_mission(
        &self,
        mission_id: i64,
        llm: Arc<LlmClient>,
        input: MissionInputs,
    ) -> Result<MissionOutcome> {
        let mission = match self.db.get_mission(mission_id) {
            Ok(m) => m,
            Err(e) => return Err(e),
        };
        let max_steps = mission.max_steps;
        let budget = mission.budget_tokens;

        self.set_state(EntityState::Deliberating, Some(mission_id));
        let _ = self.db.update_mission_status(mission_id, "running", None);
        self.activity(
            "mission",
            &format!("Mission started: {}", truncate(&mission.goal, 90)),
            Some(mission_id),
        );

        // 1) Plan.
        let (plan, planner_spent) = match self.make_plan(&mission.goal, &llm, &input.language).await
        {
            Ok((p, spent)) => {
                self.record_spent(mission_id, spent);
                (p, spent)
            }
            Err(e) => {
                let _ =
                    self.db
                        .insert_action(Some(mission_id), "plan", "{}", false, &e.to_string());
                return Ok(self.fail(mission_id, 0, 0, format!("planning failed: {e}")));
            }
        };
        let dims_json = serde_json::to_string(&plan.dimensions).unwrap_or_else(|_| "[]".into());
        let _ = self.db.set_mission_dimensions(mission_id, &dims_json);
        let _ = self.db.insert_action(
            Some(mission_id),
            "plan",
            &serde_json::to_string(&plan).unwrap_or_else(|_| "{}".into()),
            true,
            &format!(
                "{} dimensions, {} steps",
                plan.dimensions.len(),
                plan.steps.len()
            ),
        );
        self.activity(
            "plan",
            &format!(
                "Plan ready: {} dimensions, {} steps",
                plan.dimensions.len(),
                plan.steps.len()
            ),
            Some(mission_id),
        );

        // 2) Execute steps with live replanning.
        self.set_state(EntityState::Working, Some(mission_id));
        let mut ledger: Vec<RetrievedSource> = Vec::new();
        let mut urls_seen: HashSet<String> = HashSet::new();
        // Planning consumes real model tokens. Seed the mission ledger with
        // that cost before any retrieval starts; otherwise a costly plan can
        // silently reset the budget counter and make the cap advisory.
        let mut spent: i64 = planner_spent as i64;
        let mut steps_done: i64 = 0;
        let mut plan_steps = plan.steps;
        let _ = self
            .db
            .update_mission_progress(mission_id, steps_done, spent);

        let mut i = 0usize;
        while i < plan_steps.len() {
            if input.cancel.load(Ordering::SeqCst) {
                let _ = self
                    .db
                    .update_mission_progress(mission_id, steps_done, spent);
                return Ok(self.cancel(mission_id, steps_done, spent));
            }
            while input.paused.load(Ordering::SeqCst) {
                if input.cancel.load(Ordering::SeqCst) {
                    return Ok(self.cancel(mission_id, steps_done, spent));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            if steps_done >= max_steps || spent >= budget - REPORT_BUDGET_TOKENS {
                self.activity(
                    "budget",
                    "Step/budget limit reached — moving to the report",
                    Some(mission_id),
                );
                break;
            }

            let step = plan_steps[i].clone();
            match step.kind.as_str() {
                "report" => break,
                "search" => {
                    let q = step.query.trim();
                    if !q.is_empty() {
                        self.activity("search", &format!("Searching: {q}"), Some(mission_id));
                        match self.http.web_search(q, 5).await {
                            Ok(hits) => {
                                let mut added = 0usize;
                                for h in hits {
                                    let norm = provenance::normalize_url(&h.url);
                                    if urls_seen.contains(&norm) {
                                        continue;
                                    }
                                    if let Ok(Some(note_id)) = self.db.add_note_if_new(
                                        "research",
                                        &h.title,
                                        &h.snippet,
                                        Some(mission_id),
                                        Some(&h.url),
                                        Some(&h.title),
                                    ) {
                                        urls_seen.insert(norm);
                                        // A search snippet is NOT retrieved
                                        // page text: cap what enters the ledger
                                        // from a hit, so the writer cannot mine
                                        // evidence out of a snippet nobody
                                        // fetched. (C3 grades quotes against
                                        // fetched page text only.)
                                        ledger.push(RetrievedSource {
                                            url: h.url.clone(),
                                            title: h.title.clone(),
                                            fetched: false,
                                            note_id: Some(note_id),
                                        });
                                        let _ = self
                                            .db
                                            .insert_source(mission_id, &h.url, &h.title, false);
                                        added += 1;
                                    }
                                }
                                let _ = self.db.insert_action(
                                    Some(mission_id),
                                    "search",
                                    &serde_json::json!({"query": q}).to_string(),
                                    true,
                                    &format!("new sources: {added}"),
                                );
                                if added == 0 {
                                    self.activity(
                                        "dedup",
                                        "All results were duplicates — nothing added",
                                        Some(mission_id),
                                    );
                                }
                            }
                            Err(e) => {
                                let _ = self.db.insert_action(
                                    Some(mission_id),
                                    "search",
                                    &serde_json::json!({"query": q}).to_string(),
                                    false,
                                    &e.to_string(),
                                );
                                self.activity(
                                    "error",
                                    &format!("Search failed: {e}"),
                                    Some(mission_id),
                                );
                            }
                        }
                    }
                }
                "fetch" => {
                    let u = step.url.trim();
                    if !u.is_empty() {
                        self.activity(
                            "fetch",
                            &format!("Reading: {}", truncate(u, 70)),
                            Some(mission_id),
                        );
                        match self.http.fetch_page(u, 4500).await {
                            Ok(page) => {
                                let norm = provenance::normalize_url(&page.url);
                                if !urls_seen.contains(&norm) {
                                    let excerpt: String = page.text.chars().take(1200).collect();
                                    if let Ok(Some(note_id)) = self.db.add_note_if_new(
                                        "research",
                                        &page.title,
                                        &excerpt,
                                        Some(mission_id),
                                        Some(&page.url),
                                        Some(&page.title),
                                    ) {
                                        urls_seen.insert(norm);
                                        ledger.push(RetrievedSource {
                                            url: page.url.clone(),
                                            title: page.title.clone(),
                                            fetched: true,
                                            note_id: Some(note_id),
                                        });
                                        let _ = self.db.insert_source(
                                            mission_id,
                                            &page.url,
                                            &page.title,
                                            true,
                                        );
                                    }
                                }
                                let _ = self.db.insert_action(
                                    Some(mission_id),
                                    "fetch",
                                    &serde_json::json!({"url": u}).to_string(),
                                    true,
                                    &truncate(&page.title, 60),
                                );
                            }
                            Err(e) => {
                                let _ = self.db.insert_action(
                                    Some(mission_id),
                                    "fetch",
                                    &serde_json::json!({"url": u}).to_string(),
                                    false,
                                    &e.to_string(),
                                );
                                self.activity(
                                    "error",
                                    &format!("Fetch failed: {e}"),
                                    Some(mission_id),
                                );
                            }
                        }
                    }
                }
                _ => {
                    let _ = self.db.insert_action(
                        Some(mission_id),
                        "unknown_step",
                        &serde_json::to_string(&step).unwrap_or_default(),
                        false,
                        "unknown step kind",
                    );
                }
            }

            i += 1;
            steps_done += 1;
            let _ = self
                .db
                .update_mission_progress(mission_id, steps_done, spent);
            self.sink.emit(EntityEvent::MissionUpdate {
                id: mission_id,
                status: "running".into(),
                steps_done,
                max_steps,
                spent_tokens: spent,
            });

            // Live replan every 4 executed steps — the organization stays alive.
            // Retrieval may only spend down to the report reserve.
            if i.is_multiple_of(4) && i < plan_steps.len() && spent < budget - REPORT_BUDGET_TOKENS
            {
                match self
                    .replan(
                        &mission.goal,
                        &ledger,
                        spent,
                        budget,
                        &plan_steps[i..],
                        &llm,
                    )
                    .await
                {
                    Ok((new_tail, r_spent)) => {
                        spent += r_spent as i64;
                        if !new_tail.is_empty() {
                            plan_steps.truncate(i);
                            plan_steps.extend(new_tail);
                            self.activity(
                                "replan",
                                "Plan adjusted mid-mission based on findings",
                                Some(mission_id),
                            );
                        }
                    }
                    Err(_) => { /* keep the original plan */ }
                }
            }
        }

        // 3) Report with provenance gate. The writer gets the LAST slice of the
        // authorisation for its own reserved budget rather than competing with
        // retrieval for whatever is left: on a long mission the retrieval steps
        // could otherwise spend the whole budget and leave the report unwritable,
        // which is the most expensive possible failure (all the cost, no report).
        if ledger.is_empty() {
            let _ = self
                .db
                .update_mission_progress(mission_id, steps_done, spent);
            return Ok(self.fail(
                mission_id,
                steps_done,
                spent,
                "no sources retrieved — nothing to report honestly".into(),
            ));
        }
        let writer_budget = budget.saturating_sub(spent).clamp(0, REPORT_BUDGET_TOKENS);
        if writer_budget == 0 {
            let _ = self
                .db
                .update_mission_progress(mission_id, steps_done, spent);
            return Ok(self.fail(
                mission_id,
                steps_done,
                spent,
                "mission budget exhausted before the report — nothing written".into(),
            ));
        }
        let _ = self.db.insert_action(
            Some(mission_id),
            "budget",
            &serde_json::json!({
                "budget": budget,
                "spent": spent,
                "writer_reserved": writer_budget,
                "retrieval_allowance": budget.saturating_sub(REPORT_BUDGET_TOKENS),
            })
            .to_string(),
            true,
            &format!("writer reserved {writer_budget} tokens"),
        );

        self.set_state(EntityState::Reporting, Some(mission_id));
        self.activity(
            "report",
            "Writing the report (provenance enforced)…",
            Some(mission_id),
        );
        // C1 means "the source was actually retrieved". A search hit proves only
        // that the URL was *seen* in a result list; before this, both kinds were
        // flattened into one `Vec<String>`, so a report could cite a page that
        // was never opened and still pass C1 with backed_ratio = 1.0.
        let retrieved_urls: Vec<String> = ledger
            .iter()
            .filter(|s| s.fetched)
            .map(|s| s.url.clone())
            .collect();
        // Kept separately so the gate can say *why* a citation failed instead of
        // reporting a bare "not retrieved".
        let unfetched_urls: Vec<String> = ledger
            .iter()
            .filter(|s| !s.fetched)
            .map(|s| s.url.clone())
            .collect();
        let sources_json = serde_json::to_value(&ledger).unwrap_or(serde_json::json!([]));

        let writer_allowance = writer_max_tokens(writer_budget);
        let (markdown, w_spent) = match self
            .write_report(
                &mission.goal,
                &plan.dimensions,
                &ledger,
                &input.language,
                writer_allowance,
                &llm,
            )
            .await
        {
            Ok(x) => x,
            Err(e) => {
                let _ = self
                    .db
                    .update_mission_progress(mission_id, steps_done, spent);
                return Ok(self.fail(
                    mission_id,
                    steps_done,
                    spent,
                    format!("report writing failed: {e}"),
                ));
            }
        };
        spent += w_spent as i64;
        let _ = self
            .db
            .update_mission_progress(mission_id, steps_done, spent);

        // The gate's own audit: C1/C2 as always, plus Tier-0 quote grounding
        // (C3) against the text that was actually retrieved. `backed_ratio` and
        // the C1/C2 numbers are frozen; C3 can only make the verdict stricter,
        // never looser. The receipt is stored with the report so a PASS is never
        // shown without its denominator, its interval, and — when something
        // could not be checked — the honest reason why.
        let snapshots = self.snapshots_for(&ledger);
        // A snapshot is only evidence if its page was actually fetched. A search
        // hit's note is a ~200-char snippet the search engine wrote — quoting it
        // would let the snippet source grade its own claim.
        let (check, receipt, claims) =
            provenance::check_provenance_full(&markdown, &retrieved_urls, &snapshots);
        // Naming the never-fetched citations turns a bare FAIL into an instruction
        // the owner (and the repair pass) can act on.
        let (check, receipt) =
            provenance::name_unfetched_citations(check, receipt, &markdown, &unfetched_urls);
        let mut report_id = self
            .db
            .insert_report(
                mission_id,
                &markdown,
                &sources_json,
                &serde_json::to_value(&check).unwrap_or(serde_json::json!({})),
                check.metrics.backed_ratio,
                &check.verdict,
                false,
            )
            .unwrap_or(0);
        let mut final_check = check;
        let mut final_receipt = receipt;
        if report_id > 0 {
            let _ = self.db.set_report_receipt(
                report_id,
                &serde_json::to_value(&final_receipt).unwrap_or(serde_json::json!({})),
                &serde_json::to_value(&claims).unwrap_or(serde_json::json!([])),
            );
        }

        if final_check.verdict == "FAIL" {
            self.activity(
                "checker",
                &format!(
                    "Provenance FAIL ({}% backed, {} claims) — one repair attempt…",
                    (final_check.metrics.backed_ratio * 100.0) as i64,
                    final_receipt.n_claims
                ),
                Some(mission_id),
            );
            // The repair pass draws on the same reserve, and only while the
            // reserve is still there: a repair is worth doing only if it can be
            // paid for.
            let repair_allowance = writer_max_tokens(budget.saturating_sub(spent));
            if let Ok((fixed, r_spent)) = self
                .repair_report(
                    RepairInputs {
                        report: &markdown,
                        check: &final_check,
                        receipt: &final_receipt,
                        ledger: &ledger,
                        language: &input.language,
                        max_tokens: repair_allowance,
                    },
                    &llm,
                )
                .await
            {
                spent += r_spent as i64;
                let (c2, r2, claims2) =
                    provenance::check_provenance_full(&fixed, &retrieved_urls, &snapshots);
                report_id = self
                    .db
                    .insert_report(
                        mission_id,
                        &fixed,
                        &sources_json,
                        &serde_json::to_value(&c2).unwrap_or(serde_json::json!({})),
                        c2.metrics.backed_ratio,
                        &c2.verdict,
                        true,
                    )
                    .unwrap_or(report_id);
                if report_id > 0 {
                    let _ = self.db.set_report_receipt(
                        report_id,
                        &serde_json::to_value(&r2).unwrap_or(serde_json::json!({})),
                        &serde_json::to_value(&claims2).unwrap_or(serde_json::json!([])),
                    );
                }
                final_check = c2;
                final_receipt = r2;
                let _ = self
                    .db
                    .update_mission_progress(mission_id, steps_done, spent);
            }
        }

        // The verdict decides the outcome. Before this, a report whose own gate
        // said FAIL was still stored as `completed` and the chat announced
        // "Mission complete ✅ … (provenance FAIL)" — the product's central
        // promise contradicted by its own status field. A mission whose report
        // the gate rejected is `unverified`, not `completed`.
        let gate_passed = final_check.verdict.eq_ignore_ascii_case("PASS");
        let final_status = if gate_passed {
            "completed"
        } else {
            "unverified"
        };
        let _ = self
            .db
            .update_mission_status(mission_id, final_status, None);
        self.sink.emit(EntityEvent::ReportReady {
            id: report_id,
            mission_id,
            verdict: final_check.verdict.clone(),
            backed_ratio: final_check.metrics.backed_ratio,
        });
        self.activity(
            "report",
            &format!(
                "Report ready — provenance {} ({}% backed, {} claims checked, gate {})",
                final_check.verdict,
                (final_check.metrics.backed_ratio * 100.0) as i64,
                final_receipt.n_claims,
                provenance::GATE_VERSION
            ),
            Some(mission_id),
        );
        self.set_state(EntityState::Attentive, None);

        Ok(MissionOutcome {
            mission_id,
            status: final_status.into(),
            report_id: Some(report_id),
            verdict: Some(final_check.verdict),
            backed_ratio: Some(final_check.metrics.backed_ratio),
            sources: ledger.len(),
            spent_tokens: spent,
        })
    }

    fn record_spent(&self, mission_id: i64, tokens: u64) {
        if tokens > 0 {
            let _ =
                self.db
                    .insert_action(Some(mission_id), "tokens", "{}", true, &format!("{tokens}"));
        }
    }

    fn fail(
        &self,
        mission_id: i64,
        steps_done: i64,
        spent_tokens: i64,
        err: String,
    ) -> MissionOutcome {
        let _ = self
            .db
            .update_mission_progress(mission_id, steps_done, spent_tokens);
        let _ = self
            .db
            .update_mission_status(mission_id, "failed", Some(&err));
        let _ = self.db.insert_event("error", "mission", &err);
        self.activity("error", &format!("Mission failed: {err}"), Some(mission_id));
        self.set_state(EntityState::Attentive, None);
        MissionOutcome {
            mission_id,
            status: "failed".into(),
            report_id: None,
            verdict: None,
            backed_ratio: None,
            sources: 0,
            spent_tokens,
        }
    }

    fn cancel(&self, mission_id: i64, steps_done: i64, spent_tokens: i64) -> MissionOutcome {
        let _ = self
            .db
            .update_mission_progress(mission_id, steps_done, spent_tokens);
        let _ = self.db.update_mission_status(mission_id, "cancelled", None);
        self.activity(
            "mission",
            "Mission cancelled by the owner",
            Some(mission_id),
        );
        self.set_state(EntityState::Attentive, None);
        MissionOutcome {
            mission_id,
            status: "cancelled".into(),
            report_id: None,
            verdict: None,
            backed_ratio: None,
            sources: 0,
            spent_tokens,
        }
    }

    // ---------- LLM phases ----------

    async fn make_plan(&self, goal: &str, llm: &LlmClient, language: &str) -> Result<(Plan, u64)> {
        let system = "You are Vara, a persistent autonomous entity planning a research mission. \
Return ONLY valid JSON — no prose, no markdown fences.\n\
Schema:\n\
{\"dimensions\":[{\"name\":\"<short dimension>\",\"question\":\"<what to find out>\"}],\"steps\":[{\"kind\":\"search\",\"query\":\"<concrete web search query>\",\"dimension\":\"<dimension name>\"},{\"kind\":\"fetch\",\"url\":\"<https url>\",\"dimension\":\"<dimension name>\"},{\"kind\":\"report\"}]}\n\
Rules:\n\
- 3 to 5 dimensions, each a different angle on the goal.\n\
- 6 to 10 steps total. At least 2 fetch steps pointing at specific, plausible pages.\n\
- Search queries: concrete, include product names / versions / benchmarks when relevant. Write queries in English.\n\
- The LAST step must be {\"kind\":\"report\"}.\n\
- JSON only.";
        let user = format!("Mission goal: {goal}");
        let mut last_err = String::new();
        for attempt in 0..2 {
            let mut msgs = vec![ChatMessage::system(system), ChatMessage::user(&user)];
            if attempt > 0 {
                msgs.push(ChatMessage::user(format!(
                    "Your previous answer was invalid: {last_err}. Return ONLY the JSON object."
                )));
            }
            let reply = llm.chat(&msgs, Some(1200)).await?;
            if let Some(v) = extract_json(&reply.content) {
                if let Ok(mut plan) = serde_json::from_value::<Plan>(v.clone()) {
                    normalize_plan(&mut plan, goal);
                    return Ok((plan, reply.total_tokens()));
                }
                last_err = "schema mismatch".into();
            } else {
                last_err = "no JSON object found".into();
            }
        }
        // Deterministic fallback plan — Vara stays useful even with a weak model.
        let _ = language;
        Ok((fallback_plan(goal), 0))
    }

    async fn replan(
        &self,
        goal: &str,
        ledger: &[RetrievedSource],
        spent: i64,
        budget: i64,
        remaining: &[PlanStep],
        llm: &LlmClient,
    ) -> Result<(Vec<PlanStep>, u64)> {
        let have: Vec<String> = ledger
            .iter()
            .map(|s| format!("- {} ({})", truncate(&s.title, 60), truncate(&s.url, 70)))
            .collect();
        let system = "You are Vara mid-mission, re-planning the remaining steps based on what was found. \
Return ONLY JSON: {\"steps\":[{\"kind\":\"search\",\"query\":\"...\",\"dimension\":\"...\"},{\"kind\":\"fetch\",\"url\":\"...\",\"dimension\":\"...\"},{\"kind\":\"report\"}]}\n\
Rules: at most 4 steps. The LAST step must be {\"kind\":\"report\"}. If the material is already sufficient, return only the report step. JSON only.";
        let user = format!(
            "Goal: {goal}\n\nFound so far:\n{}\n\nBudget used: {spent}/{budget} tokens.\nRemaining planned steps: {}\n\nRe-plan now.",
            have.join("\n"),
            serde_json::to_string(remaining).unwrap_or_default()
        );
        let reply = llm
            .chat(
                &[ChatMessage::system(system), ChatMessage::user(user)],
                Some(600),
            )
            .await?;
        if let Some(v) = extract_json(&reply.content) {
            if let Some(steps) = v.get("steps").and_then(|s| s.as_array()) {
                let mut out: Vec<PlanStep> = steps
                    .iter()
                    .filter_map(|s| serde_json::from_value::<PlanStep>(s.clone()).ok())
                    .filter(|s| ["search", "fetch", "report"].contains(&s.kind.as_str()))
                    .take(4)
                    .collect();
                if let Some(last) = out.last() {
                    if last.kind != "report" {
                        out.push(PlanStep {
                            kind: "report".into(),
                            query: String::new(),
                            url: String::new(),
                            dimension: String::new(),
                        });
                    }
                }
                return Ok((out, reply.total_tokens()));
            }
        }
        Err(VaraError::Llm("replan: invalid JSON".into()))
    }

    async fn write_report(
        &self,
        goal: &str,
        dimensions: &[PlanDimension],
        ledger: &[RetrievedSource],
        language: &str,
        max_tokens: u32,
        llm: &LlmClient,
    ) -> Result<(String, u64)> {
        let lang_name = lang_name(language);
        let system = format!(
            "You are Vara's report writer. You receive a research ledger of sources actually retrieved during the mission. Write the final report in {lang_name}.\n\n\
HARD PROVENANCE RULES (enforced by a machine after you write):\n\
1. Cite ONLY sources from the ledger, using [n] where n is the ledger index shown below.\n\
2. Never invent sources, URLs, or citation numbers. Any URL absent from the ledger FAILS the report.\n\
3. If a dimension lacks material, write 'Gap:' explicitly instead of inventing facts. Declared gaps are honest; fabricated sources are fatal.\n\
4. Attach a citation to every factual claim.\n\
5. End with a section headed exactly '## Sources' listing each cited source as: [n] URL — title.\n\
6. Quote only word-for-word text you were given. A machine checks every quoted span against the retrieved text and the report FAILS if a quotation is not found there.\n\
7. A ledger entry marked (snippet only) was seen in search results but its page was NOT retrieved. Use it to direct the reader, never as evidence, and never quote from it.\n\n\
Tone: precise, dense, decision-ready. No filler, no self-praise."
        );
        let dims: Vec<String> = dimensions
            .iter()
            .map(|d| format!("- {}: {}", d.name, d.question))
            .collect();
        let mut ledger_lines = Vec::new();
        for (i, s) in ledger.iter().enumerate() {
            let body = s
                .note_id
                .and_then(|id| self.note_body(id))
                .unwrap_or_default();
            // Retrieval scope is labelled, not implied: an abstract or a search
            // snippet must not be silently promoted to "full text retrieved".
            let scope = if s.fetched {
                "retrieved"
            } else {
                "snippet only"
            };
            ledger_lines.push(format!(
                "[{}] {} — {} ({}{}){}",
                i + 1,
                s.title,
                s.url,
                scope,
                if s.fetched {
                    ", quotable"
                } else {
                    ", not quotable"
                },
                if body.is_empty() {
                    String::new()
                } else {
                    format!("\n    snippet: {}", truncate(&body, 220))
                }
            ));
        }
        let user = format!(
            "Mission goal: {goal}\n\nRequired dimensions:\n{}\n\nResearch ledger:\n{}\n\nWrite the report now.",
            dims.join("\n"),
            ledger_lines.join("\n")
        );
        let reply = llm
            .chat(
                &[ChatMessage::system(system), ChatMessage::user(user)],
                Some(max_tokens),
            )
            .await?;
        Ok((reply.content.trim().to_string(), reply.total_tokens()))
    }
    async fn repair_report(
        &self,
        inputs: RepairInputs<'_>,
        llm: &LlmClient,
    ) -> Result<(String, u64)> {
        let RepairInputs {
            report,
            check,
            receipt,
            ledger,
            language,
            max_tokens,
        } = inputs;
        let lang_name = lang_name(language);
        let ledger_lines: Vec<String> = ledger
            .iter()
            .enumerate()
            .map(|(i, s)| format!("[{}] {} — {}", i + 1, s.title, s.url))
            .collect();
        let system = format!(
            "You repair a report that failed a structural citation check. Write in {lang_name}.\n\
Keep the good content. Remove or fix invalid citations. If evidence is missing, replace the claim with an explicit 'Gap:' note. \
Never keep a quotation that is not word-for-word in the ledger text. \
The '## Sources' list must contain ONLY ledger entries, numbered exactly as in the ledger. Output the full corrected report only."
        );
        let missing_quotes = if receipt.missing_quotes.is_empty() {
            "- (none)".to_string()
        } else {
            receipt
                .missing_quotes
                .iter()
                .take(10)
                .map(|q| format!("- \"{}\"", truncate(q, 160)))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let user = format!(
            "Ledger:\n{}\n\nCheck failures:\n- Unresolved [n] refs: {:?}\n- Cited URLs not in retrieval ledger: {}\n\
             - Quotations not found in the retrieved text (remove them or use the exact wording):\n{}\n\
             - Not evaluated: {}\n\nReport to fix:\n{}",
            ledger_lines.join("\n"),
            check.unresolved_refs,
            check.cited_not_retrieved.join(", "),
            missing_quotes,
            receipt
                .not_evaluable_reason
                .clone()
                .unwrap_or_else(|| "nothing".into()),
            report
        );
        let reply = llm
            .chat(
                &[ChatMessage::system(system), ChatMessage::user(user)],
                Some(max_tokens),
            )
            .await?;
        Ok((reply.content.trim().to_string(), reply.total_tokens()))
    }

    /// The text that was actually retrieved, per URL — what C3 grounds quotes
    /// against. Notes hold what a mission really fetched, so a claim quoting
    /// text nobody retrieved is detectable offline, for free, with no network.
    fn snapshots_for(&self, ledger: &[RetrievedSource]) -> Vec<provenance::RetrievedSnapshot> {
        ledger
            .iter()
            .filter_map(|s| {
                s.note_id
                    .and_then(|id| self.note_body(id))
                    .filter(|text| !text.trim().is_empty())
                    .map(|text| provenance::RetrievedSnapshot {
                        url: s.url.clone(),
                        text,
                    })
            })
            .collect()
    }

    /// Idle reflection — the heartbeat. Disabled by default; gentle by design.
    pub async fn heartbeat(&self, llm: &LlmClient, language: &str) -> Result<String> {
        let notes = self.db.list_notes(12, 0)?;
        if notes.is_empty() {
            return Ok("nothing to reflect on yet".into());
        }
        let digest: Vec<String> = notes
            .iter()
            .map(|n| format!("- {}: {}", truncate(&n.title, 60), truncate(&n.body, 120)))
            .collect();
        let system = format!(
            "You are Vara, reflecting quietly on your memory. Produce ONE short observation worth remembering (max 50 words) in {}. \
It may connect two notes, spot a pattern, or pose a sharp question. Output only the observation text.",
            lang_name(language)
        );
        let reply = llm
            .chat(
                &[
                    ChatMessage::system(system),
                    ChatMessage::user(digest.join("\n")),
                ],
                Some(200),
            )
            .await?;
        let body = reply.content.trim().to_string();
        if body.chars().count() < 15 {
            return Ok("reflection too short — skipped".into());
        }
        match self
            .db
            .add_note_if_new("reflection", "Reflection", &body, None, None, None)?
        {
            Some(_) => {
                self.activity("reflection", &truncate(&body, 100), None);
                Ok(body)
            }
            None => Ok("duplicate reflection — skipped".into()),
        }
    }

    fn note_body(&self, note_id: i64) -> Option<String> {
        self.db.get_note(note_id).ok().map(|n| n.body)
    }
}

// ---------- helpers ----------

fn lang_name(language: &str) -> &'static str {
    match language {
        "ar" | "arabic" => "Arabic",
        _ => "English",
    }
}

fn normalize_plan(plan: &mut Plan, goal: &str) {
    let valid = ["search", "fetch", "report"];
    plan.steps.retain(|s| valid.contains(&s.kind.as_str()));
    if plan.steps.len() > 12 {
        plan.steps.truncate(12);
    }
    if let Some(last) = plan.steps.last() {
        if last.kind != "report" {
            plan.steps.push(PlanStep {
                kind: "report".into(),
                query: String::new(),
                url: String::new(),
                dimension: String::new(),
            });
        }
    } else {
        plan.steps.push(PlanStep {
            kind: "report".into(),
            query: String::new(),
            url: String::new(),
            dimension: String::new(),
        });
    }
    if plan.dimensions.is_empty() {
        plan.dimensions.push(PlanDimension {
            name: "overview".into(),
            question: truncate(goal, 100),
        });
    }
}

fn fallback_plan(goal: &str) -> Plan {
    let g = truncate(goal, 80);
    Plan {
        dimensions: vec![
            PlanDimension {
                name: "overview".into(),
                question: g.clone(),
            },
            PlanDimension {
                name: "evidence".into(),
                question: "concrete numbers and benchmarks".into(),
            },
        ],
        steps: vec![
            PlanStep {
                kind: "search".into(),
                query: g.clone(),
                dimension: "overview".into(),
                url: String::new(),
            },
            PlanStep {
                kind: "search".into(),
                query: format!("{g} benchmark"),
                dimension: "evidence".into(),
                url: String::new(),
            },
            PlanStep {
                kind: "search".into(),
                query: format!("{g} comparison 2026"),
                dimension: "evidence".into(),
                url: String::new(),
            },
            PlanStep {
                kind: "report".into(),
                query: String::new(),
                url: String::new(),
                dimension: String::new(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_plan_ends_with_report() {
        let p = fallback_plan("local inference benchmarks");
        assert_eq!(p.steps.last().unwrap().kind, "report");
        assert!(p.steps.len() >= 3);
    }

    #[test]
    fn normalize_plan_filters_unknown_kinds_and_appends_report() {
        let mut p = Plan {
            dimensions: vec![],
            steps: vec![
                PlanStep {
                    kind: "search".into(),
                    query: "x".into(),
                    url: String::new(),
                    dimension: String::new(),
                },
                PlanStep {
                    kind: "dance".into(),
                    query: String::new(),
                    url: String::new(),
                    dimension: String::new(),
                },
            ],
        };
        normalize_plan(&mut p, "goal");
        assert!(p
            .steps
            .iter()
            .all(|s| ["search", "fetch", "report"].contains(&s.kind.as_str())));
        assert_eq!(p.steps.last().unwrap().kind, "report");
        assert!(!p.dimensions.is_empty());
    }

    #[cfg(test)]
    mod seam_wiring_tests {
        use super::*;
        use crate::host::{Host, HostEnv, LogEntry};
        use crate::seams::{EventSinkCap, LogPlugin};
        use std::sync::Mutex;

        struct TestEnv;
        impl HostEnv for TestEnv {
            fn authorize(&self, _r: &crate::host::GateRequest) -> crate::host::GateDecision {
                crate::host::GateDecision::Allow
            }
            fn log(&self, _e: LogEntry) {}
        }

        /// Records what the seam carried, so the test can prove events went through
        /// the plugin rather than through the built-in sink.
        struct RecordingCap {
            lines: Mutex<Vec<String>>,
        }
        impl EventSinkCap for RecordingCap {
            fn record(&self, kind: &str, message: &str) {
                self.lines
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(format!("{kind}:{message}"));
            }
        }

        /// The smallest possible seam check: register a value, read it back.
        ///
        /// This exists because the log-seam test below failed and could not say
        /// *why*. If this passes, the host and the downcast are sound and the fault
        /// is in the seam's declared type. If it fails, no seam can work at all and
        /// the "everything is a plugin" claim has no implementation underneath it.
        #[test]
        fn a_registered_service_is_readable_for_its_exact_type() {
            struct PlainPlugin;
            impl crate::host::Plugin for PlainPlugin {
                fn id(&self) -> &str {
                    "probe"
                }
                fn start(
                    &self,
                    ctx: &crate::host::PluginCtx<'_>,
                ) -> std::result::Result<Vec<crate::host::Effect>, String> {
                    let effect = ctx
                        .register("probe.value", 41u64)
                        .map_err(|e| e.to_string())?;
                    Ok(vec![effect])
                }
            }
            let host = std::sync::Arc::new(Host::new(std::sync::Arc::new(TestEnv)));
            let plugin: std::sync::Arc<dyn crate::host::Plugin> = std::sync::Arc::new(PlainPlugin);
            host.load(plugin).expect("probe plugin loads");

            let listed = host.services();
            assert!(
                listed.iter().any(|(n, _)| n == "probe.value"),
                "the service must be listed, got {listed:?}"
            );
            let got = host.service::<u64>("probe.value");
            assert_eq!(got.map(|v| *v), Some(41), "exact-type read-back must work");
        }

        /// The load-bearing test for `docs/REALITY_MATRIX.md` row A.
        ///
        /// It proves three things in order, which together are what "the runtime
        /// takes a capability from a plugin" means:
        ///   1. with the `log` plugin loaded, mission events go to the plugin;
        ///   2. unloading the plugin makes the seam resolve to the built-in sink,
        ///      so the plugin's stream stops;
        ///   3. a composition with **no** log plugin refuses to hand out a seam
        ///      handle instead of silently substituting one.
        #[test]
        fn the_log_seam_really_replaces_the_sink() {
            let host = std::sync::Arc::new(Host::new(std::sync::Arc::new(TestEnv)));
            let cap = std::sync::Arc::new(RecordingCap {
                lines: Mutex::new(Vec::new()),
            });
            let plugin: std::sync::Arc<dyn crate::host::Plugin> =
                std::sync::Arc::new(LogPlugin::new(cap.clone()));
            host.load(plugin).expect("the log plugin loads");

            // Diagnostic: what the host actually holds, and whether the seam's
            // declared type reads back. Printed (not asserted) so a failure below
            // says which half is broken.
            let listed = host.services();
            let direct = host.service::<LogCap>(crate::seams::seam::LOG).is_some();
            println!("registered: {listed:?}");
            println!("LogCap readable: {direct}");

            // 1) The seam resolves, and it is the plugin.
            let sink = sink_from_host(&host, std::sync::Arc::new(NullSink));
            sink.emit(EntityEvent::Activity {
                kind: "test".into(),
                message: "through the plugin".into(),
                mission_id: Some(1),
            });
            let lines = cap.lines.lock().unwrap_or_else(|p| p.into_inner()).clone();
            assert_eq!(
                lines,
                vec!["activity:test: through the plugin".to_string()],
                "the event must have gone through the registered plugin"
            );

            // 2) Unloading the plugin empties the seam; the built-in sink takes over.
            assert!(host.unload("log"), "the plugin unloads");
            let fallback = sink_from_host(&host, std::sync::Arc::new(NullSink));
            fallback.emit(EntityEvent::Activity {
                kind: "test".into(),
                message: "after unload".into(),
                mission_id: Some(1),
            });
            let lines_after = cap.lines.lock().unwrap_or_else(|p| p.into_inner()).clone();
            assert_eq!(
                lines_after.len(),
                1,
                "an unloaded plugin must stop receiving events: {lines_after:?}"
            );

            // 3) Strict resolution reports the empty seam.
            let strict = require_log_sink(&host);
            assert!(
                strict.is_err(),
                "without a log plugin the strict resolver must refuse, not default"
            );
            let msg = strict.err().unwrap_or_default();
            assert!(
                msg.contains("log") && msg.contains("event stream"),
                "the refusal must name the seam: {msg}"
            );
        }
    }
}
