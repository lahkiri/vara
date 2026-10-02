//! Provenance gate — the honesty spine of a report.
//!
//! Tier 0 (structural + verbatim quote grounding):
//!   C1: every cited link must belong to the actually-retrieved set.
//!   C2: every numeric [n] reference must resolve to a numbered source entry.
//!   C3: every quoted span in a claim must appear verbatim in the retrieved
//!       text of a URL that claim cites (additive, `check_provenance_full`).
//!
//! `check_provenance` (C1/C2, `backed_ratio`) is **frozen**: the v1 experiment
//! numbers — 9 of 13 cited sources never retrieved, backed_ratio 0.308 — are
//! replayed by `tests/provenance_tests.rs` and mirrored by an external harness
//! script, so its semantics and numbers must not move. C3 lives only in the
//! additive `check_provenance_full`, which calls it internally.
//!
//! Everything here is offline, deterministic and token-free: no LLM, no
//! network, no new dependencies. That is the point of Tier 0 — it is the part
//! of the gate that can run on every report, for free, and that never reports
//! "we did not check" as "we checked and it was fine" (see
//! `docs/PROVENANCE.md`).

use crate::types::{ProvenanceChecks, ProvenanceMetrics, ProvenanceResult, ProvenanceSection};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const RULE_BILINGUAL: &str =
    "C1: كل رابط في التقرير ∈ المسترجَع فعلياً | C2: كل مرجع [n] يقابل مدخلاً مرقماً في قائمة المصادر";

pub fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"https?://[^\s)\]}>"'،؛]+"#).unwrap())
}

pub fn numbered_line_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^\s*\[?(\d{1,2})\]?[.):-]?\s+").unwrap())
}

pub fn inline_ref_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\[(\d{1,2})\]").unwrap())
}

/// Normalize a URL the same way the harness does: strip fragment, tracking
/// params (utm_*, fbclid, ref), www., trailing slash; lowercase host.
pub fn normalize_url(input: &str) -> String {
    let cleaned = input.trim().trim_end_matches(['.', '،', '؛']);
    match url::Url::parse(cleaned) {
        Ok(mut u) => {
            u.set_fragment(None);
            let host = u
                .host_str()
                .unwrap_or("")
                .trim_start_matches("www.")
                .to_lowercase();
            let path = u.path().trim_end_matches('/').to_string();
            let kept: Vec<(String, String)> = u
                .query_pairs()
                .filter(|(k, _)| {
                    let k = k.to_ascii_lowercase();
                    !k.starts_with("utm_") && k != "fbclid" && k != "ref"
                })
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let query = if kept.is_empty() {
                String::new()
            } else {
                let mut ser = form_urlencoded::Serializer::new(String::new());
                for (k, v) in &kept {
                    ser.append_pair(k, v);
                }
                format!("?{}", ser.finish())
            };
            format!("{host}{path}{query}")
        }
        Err(_) => cleaned.to_lowercase().trim_end_matches('/').to_string(),
    }
}

pub fn extract_urls(text: &str) -> Vec<String> {
    url_re()
        .find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
}

fn is_sources_heading(line: &str) -> bool {
    let l = line.trim();
    let l = l.trim_start_matches('#').trim();
    let l = l.trim_end_matches(':').trim();
    matches!(
        l.to_lowercase().as_str(),
        "المصادر" | "المراجع" | "sources" | "references"
    )
}

fn heading_title(line: &str) -> Option<String> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 3 {
        return None;
    }
    let rest = &t[hashes..];
    let rest = rest.strip_prefix(' ')?;
    let title = rest.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

fn split_sections(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut title = "(intro)".to_string();
    let mut buf = String::new();
    for line in body.lines() {
        if let Some(t) = heading_title(line) {
            out.push((std::mem::take(&mut title), std::mem::take(&mut buf)));
            title = t;
        }
        buf.push_str(line);
        buf.push('\n');
    }
    out.push((title, buf));
    out
}

fn inline_refs(text: &str) -> Vec<i64> {
    inline_ref_re()
        .captures_iter(text)
        .filter_map(|c| c[1].parse::<i64>().ok())
        .collect()
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Split a report into (body, sources section) at the first sources heading.
///
/// Uses `split_inclusive`, whose pieces are an exact partition of the input,
/// so every slice below is in bounds by construction. The previous version
/// accumulated `line.len() + 1` per line, which overran the buffer — and
/// panicked — whenever the heading was the last line of a report without a
/// trailing newline. That input is reachable in production: the writer prompt
/// forces the report to end with `## Sources` and `entity.rs` stores
/// `reply.content.trim()`. The arithmetic was also wrong for CRLF files (one
/// byte of drift per preceding line), which silently ate the tail of the body.
fn split_body_sources(report: &str) -> (String, String) {
    let mut offset = 0usize;
    for line in report.split_inclusive('\n') {
        // `is_sources_heading` trims, so the `\r` of a CRLF file is harmless.
        if is_sources_heading(line) {
            return (
                report[..offset].to_string(),
                report[offset + line.len()..].to_string(),
            );
        }
        offset += line.len();
    }
    (report.to_string(), String::new())
}

/// Parse the sources section into `(number, normalized url)` pairs, in file
/// order. A line with no URL is ignored, and a line whose leading number
/// cannot be parsed keeps the URL with `None` — same as the v1 harness.
fn parse_source_entries(src_section: &str) -> Vec<(Option<i64>, String)> {
    let mut out: Vec<(Option<i64>, String)> = Vec::new();
    for line in src_section.lines() {
        let urls = extract_urls(line);
        if urls.is_empty() {
            continue;
        }
        let first_norm = normalize_url(&urls[0]);
        let n = numbered_line_re()
            .captures(line)
            .and_then(|c| c[1].parse::<i64>().ok());
        out.push((n, first_norm));
    }
    out
}

/// Run the structural check.
///
/// `retrieved`: every URL that was actually fetched/searched during the
/// mission (the retrieval ledger). Returns the same shape as the harness
/// `checker.json` so numbers stay comparable across v1 / v2 / product.
///
/// This function is frozen (see the module docs). Use
/// [`check_provenance_full`] when you also want the Tier-0 claim audit.
pub fn check_provenance(report: &str, retrieved: &[String]) -> ProvenanceResult {
    // 1) Retrieved set: normalized URL -> first original URL.
    let mut retrieved_map: HashMap<String, String> = HashMap::new();
    for u in retrieved {
        let k = normalize_url(u);
        retrieved_map.entry(k).or_insert_with(|| u.to_string());
    }

    // 2) Split report into body + sources section (panic-free: see the helper).
    let (body, src_section) = split_body_sources(report);

    // 3) Source list entries: numbered [n] or bare URL lines.
    let source_entries = parse_source_entries(&src_section);
    let src_by_n: HashMap<i64, String> = source_entries
        .iter()
        .filter_map(|(n, u)| n.map(|n| (n, u.clone())))
        .collect();

    // 4) Citations inside the body.
    let body_urls: Vec<String> = extract_urls(&body);
    let cited_urls: HashSet<String> = body_urls.iter().map(|u| normalize_url(u)).collect();
    let refs = inline_refs(&body);
    let unresolved_refs: Vec<i64> = {
        let mut seen = HashSet::new();
        let mut v: Vec<i64> = refs
            .iter()
            .filter(|n| !src_by_n.contains_key(n))
            .filter(|n| seen.insert(**n))
            .copied()
            .collect();
        v.sort();
        v
    };

    // 5) All cited = body URLs + source list URLs (unique).
    let mut all_cited: Vec<String> = cited_urls.iter().cloned().collect();
    for (_, u) in &source_entries {
        all_cited.push(u.clone());
    }
    all_cited.sort();
    all_cited.dedup();
    let cited_not_retrieved: Vec<String> = all_cited
        .iter()
        .filter(|u| !retrieved_map.contains_key(*u))
        .cloned()
        .collect();
    let retrieved_not_cited: Vec<String> = retrieved_map
        .keys()
        .filter(|u| !all_cited.contains(u))
        .cloned()
        .collect();

    // 6) Effective per-section coverage.
    let raw_sections = split_sections(&body);
    let has_intro_only = raw_sections.len() > 1;
    let mut sections: Vec<ProvenanceSection> = Vec::new();
    for (title, text) in &raw_sections {
        if title == "(intro)" && text.trim().len() < 40 && has_intro_only {
            continue;
        }
        let srefs = inline_refs(text);
        let surls: HashSet<String> = extract_urls(text)
            .iter()
            .map(|u| normalize_url(u))
            .collect();
        // A citation is "backed" only if its URL is in the actually-retrieved
        // set (harness parity: resolving to a fabricated source-list entry
        // does NOT count as backed).
        let mut backed = srefs
            .iter()
            .filter_map(|n| src_by_n.get(n))
            .filter(|u| retrieved_map.contains_key(*u))
            .count();
        backed += surls
            .iter()
            .filter(|u| retrieved_map.contains_key(*u))
            .count();
        let citations = srefs.len() + surls.len();
        sections.push(ProvenanceSection {
            title: title.clone(),
            citations,
            backed_citations: backed,
            effectively_uncovered: citations > 0 && backed == 0,
            no_citations: citations == 0,
        });
    }

    // 7) Verdict.
    let checks = ProvenanceChecks {
        c1_all_cited_in_retrieved: cited_not_retrieved.is_empty(),
        c2_refs_resolve_to_source_list: unresolved_refs.is_empty(),
    };
    let verdict = if checks.c1_all_cited_in_retrieved && checks.c2_refs_resolve_to_source_list {
        "PASS"
    } else {
        "FAIL"
    };
    let cited_total = all_cited.len();
    let backed_count = cited_total - cited_not_retrieved.len();
    let backed_ratio = if cited_total == 0 {
        0.0
    } else {
        round3(backed_count as f64 / cited_total as f64)
    };

    ProvenanceResult {
        verdict: verdict.to_string(),
        rule: RULE_BILINGUAL.to_string(),
        metrics: ProvenanceMetrics {
            cited_total,
            retrieved_total: retrieved_map.len(),
            backed_ratio,
        },
        cited_not_retrieved,
        retrieved_not_cited,
        unresolved_refs,
        sections,
        checks,
    }
}

// ---------- Tier 0: claim audit with verbatim quote grounding ----------

/// Version of the gate's decision logic. Bump it whenever the checks, the
/// normalization, the sentence/quote extraction or the verdict rule change: a
/// FAIL rate without the gate version that produced it is not comparable
/// across runs (see `docs/PROVENANCE.md`, "evaluating the gate itself").
pub const GATE_VERSION: &str = "vara-provenance/0.6";

/// Shortest quoted span that counts as evidence. Below this, quotes are words
/// or product names ("the model", "GPT-4") that occur in almost any page, so
/// checking them would produce noise rather than evidence.
pub const MIN_QUOTE_CHARS: usize = 12;

/// The page text one citation is checked against.
///
/// Pass the same text the report writer was given (Vara's
/// `types::PageContent::text`, i.e. the capped plain text of a successful
/// fetch). Passing a *shorter* text than the writer saw — a different render,
/// a paywall stub, a smaller char cap — turns honest quotes into
/// `QuoteMissing`; passing a *longer* text than the writer saw would credit
/// the writer with text it never had. Empty text counts as "no snapshot",
/// never as "a page that supports nothing".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedSnapshot {
    /// URL as the mission ledger recorded it. Matching uses
    /// [`normalize_url`], so `www.`/fragment/tracking variants still line up.
    pub url: String,
    /// Plain text of the fetched page.
    pub text: String,
}

/// What grounding one claim's quotes concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimSupport {
    /// Every quoted span appears verbatim (after normalization) in the
    /// retrieved text of a URL the claim cites.
    QuoteGrounded,
    /// At least one quoted span does not appear in any snapshot of the cited
    /// URLs — the "real URL, wrong claim" failure C1/C2 cannot see.
    QuoteMissing,
    /// The claim carries no quoted span, so there is nothing to ground. An
    /// honest paraphrase lands here; it is not a failure.
    QuoteNotRequired,
    /// The claim has quotes but no snapshot text exists for any URL it cites
    /// (or it cites no resolvable URL at all). Absence of evidence: never
    /// reported as grounded, and excluded from the support rate's denominator.
    NotEvaluable,
}

/// One audited claim: a body sentence that cites something, plus the result of
/// trying to ground its quoted spans in the retrieved text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimAudit {
    /// The sentence as written (block-split, before normalization).
    pub claim: String,
    /// `[n]` references found in the sentence, sorted and deduplicated.
    pub cited_refs: Vec<i64>,
    /// Normalized URLs the sentence points at: resolved `[n]` references plus
    /// any URL written directly in the sentence. Empty means the claim cannot
    /// be checked at all — reported as `NotEvaluable`, never as a pass.
    pub cited_urls: Vec<String>,
    /// Quoted spans of at least [`MIN_QUOTE_CHARS`] characters, in order.
    pub quotes: Vec<String>,
    /// What the snapshot check concluded for this claim.
    pub support: ClaimSupport,
}

/// Machine-readable receipt for one gate run.
///
/// Every rate carries its own n or confidence interval so that no caller can
/// print a bare "PASS": a verdict without n and a Wilson interval is not a
/// measurement (see `docs/PROVENANCE.md`, "metrics").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateReceipt {
    /// [`GATE_VERSION`] at the time of the run.
    pub gate_version: String,
    /// "PASS" or "FAIL": `c1 && c2 && c3 != Some(false)`.
    pub verdict: String,
    /// Same number as `ProvenanceResult::metrics::backed_ratio` (C1 coverage).
    pub backed_ratio: f64,
    /// Wilson 95% interval for `backed_ratio`, over `cited_total` citations.
    pub backed_ratio_ci95: (f64, f64),
    /// Share of distinct `[n]` references that resolve to the source list.
    /// 0.0 when the body has no references at all (vacuous, matching
    /// `backed_ratio`'s convention); read it together with `c2`, which is
    /// still true then.
    pub ref_resolution_rate: f64,
    /// `grounded / evaluable` claims, or `None` when nothing was evaluable.
    pub claim_support_rate: Option<f64>,
    /// Wilson 95% interval for `claim_support_rate` over its real denominator
    /// (`n_claims_evaluable`), or `None` when nothing was evaluable.
    pub claim_support_ci95: Option<(f64, f64)>,
    /// Claims audited: body sentences citing a `[n]` reference or a URL.
    pub n_claims: usize,
    /// Claims whose quotes could actually be checked — the denominator of
    /// `claim_support_rate`. Stored explicitly so the rate is never shown bare.
    pub n_claims_evaluable: usize,
    /// Quotes compared against snapshot text (quotes of evaluable claims).
    pub n_quotes_checked: usize,
    /// Distinct quotes that were compared and not found.
    pub n_quotes_missing: usize,
    /// C1: every cited URL is in the retrieved set.
    pub c1: bool,
    /// C2: every `[n]` resolves to the report's source list.
    pub c2: bool,
    /// C3 quote grounding: `Some(true)` all checked quotes grounded,
    /// `Some(false)` at least one missing, `None` not evaluable.
    pub c3: Option<bool>,
    /// Why quote grounding could not be decided (or which claims it could not
    /// cover). Must be surfaced with the verdict, never dropped: this is the
    /// difference between "grounded" and "unknown".
    pub not_evaluable_reason: Option<String>,
    /// The offending quotes when `c3 == Some(false)`. A FAIL has to name what
    /// failed, otherwise it is an unexplainable badge.
    pub missing_quotes: Vec<String>,
}

/// Run the structural gate **and** the Tier-0 claim audit.
///
/// `retrieved` is the same ledger [`check_provenance`] takes; `snapshots` is
/// the page text obtained for (some of) those URLs. Missing snapshots make
/// claims `NotEvaluable`; they never make them pass.
///
/// The returned `ProvenanceResult` is the frozen structural result with
/// `verdict` widened to include C3 — it can only turn a PASS into a FAIL,
/// never the other way round, and its numbers (`metrics`, cited/unresolved
/// lists) are untouched. The [`GateReceipt`] is the authoritative verdict.
pub fn check_provenance_full(
    report: &str,
    retrieved: &[String],
    snapshots: &[RetrievedSnapshot],
) -> (ProvenanceResult, GateReceipt, Vec<ClaimAudit>) {
    let mut result = check_provenance(report, retrieved);

    let (body, src_section) = split_body_sources(report);
    let src_by_n: HashMap<i64, String> = parse_source_entries(&src_section)
        .iter()
        .filter_map(|(n, u)| n.map(|n| (n, u.clone())))
        .collect();

    // Normalize every snapshot once: a report has dozens of claims and the
    // same page text is searched for each of them.
    let mut snapshot_texts: HashMap<String, String> = HashMap::new();
    for s in snapshots {
        let text = normalize_for_match(&s.text);
        if text.is_empty() {
            continue; // an empty snapshot is not evidence
        }
        match snapshot_texts.get_mut(&normalize_url(&s.url)) {
            Some(existing) => {
                existing.push(' ');
                existing.push_str(&text);
            }
            None => {
                snapshot_texts.insert(normalize_url(&s.url), text);
            }
        }
    }
    let usable_snapshot_urls = snapshot_texts.len();

    let (audits, stats) = audit_claims(&body, &src_by_n, &snapshot_texts);

    // Reference resolution: over the distinct [n] numbers used in the body.
    let mut body_refs = inline_refs(&body);
    body_refs.sort_unstable();
    body_refs.dedup();
    let unresolved = result.unresolved_refs.len();
    let ref_resolution_rate = if body_refs.is_empty() {
        0.0
    } else {
        // `saturating_sub`: the resolved count can never exceed the total, but a
        // panic here would take the mission down for a metric.
        let resolved = body_refs.len().saturating_sub(unresolved);
        round3(resolved as f64 / body_refs.len() as f64)
    };

    let c1 = result.checks.c1_all_cited_in_retrieved;
    let c2 = result.checks.c2_refs_resolve_to_source_list;
    let c3 = if !stats.missing_quotes.is_empty() {
        Some(false)
    } else if stats.evaluable_claims == 0 {
        None
    } else {
        Some(true)
    };
    let not_evaluable_reason = not_evaluable_reason(&stats, snapshots.len(), usable_snapshot_urls);

    let verdict = if c1 && c2 && c3 != Some(false) {
        "PASS"
    } else {
        "FAIL"
    };
    result.verdict = verdict.to_string();

    let cited_total = result.metrics.cited_total;
    let backed = cited_total.saturating_sub(result.cited_not_retrieved.len());
    let claim_support_rate = if stats.evaluable_claims == 0 {
        None
    } else {
        Some(round3(
            stats.grounded_claims as f64 / stats.evaluable_claims as f64,
        ))
    };
    let claim_support_ci95 = if stats.evaluable_claims == 0 {
        None
    } else {
        Some(wilson_ci(stats.grounded_claims, stats.evaluable_claims))
    };

    let receipt = GateReceipt {
        gate_version: GATE_VERSION.to_string(),
        verdict: verdict.to_string(),
        backed_ratio: result.metrics.backed_ratio,
        backed_ratio_ci95: wilson_ci(backed, cited_total),
        ref_resolution_rate,
        claim_support_rate,
        claim_support_ci95,
        n_claims: audits.len(),
        n_claims_evaluable: stats.evaluable_claims,
        n_quotes_checked: stats.quotes_checked,
        n_quotes_missing: stats.missing_quotes.len(),
        c1,
        c2,
        c3,
        not_evaluable_reason,
        missing_quotes: stats.missing_quotes,
    };
    (result, receipt, audits)
}

/// Wilson score interval (95%, z = 1.96) for `successes` out of `n`.
///
/// The normal approximation is useless exactly where this gate lives — small
/// n and rates near 0 or 1 — so the score interval is used everywhere a ratio
/// is reported. `n == 0` returns `(0.0, 1.0)`: with no observations the honest
/// interval is "anything", not NaN.
pub fn wilson_ci(successes: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let n_f = n as f64;
    let s = successes.min(n) as f64;
    let z = 1.96_f64;
    let z2 = z * z;
    let p = s / n_f;
    let denom = 1.0 + z2 / n_f;
    let center = (p + z2 / (2.0 * n_f)) / denom;
    let half = (z / denom) * (p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)).sqrt();
    (
        (center - half).clamp(0.0, 1.0),
        (center + half).clamp(0.0, 1.0),
    )
}

/// Aggregate counters for the quote audit.
#[derive(Default)]
struct QuoteStats {
    /// Claims with at least one quoted span (evaluable + uncheckable).
    claims_with_quotes: usize,
    /// Claims whose quotes were actually compared (the rate's denominator).
    evaluable_claims: usize,
    /// Evaluable claims with every quote grounded.
    grounded_claims: usize,
    /// Claims with quotes but no snapshot text for any cited URL.
    uncheckable_claims: usize,
    /// Quotes compared against snapshot text.
    quotes_checked: usize,
    /// Distinct quotes that were compared and not found, in first-seen order.
    missing_quotes: Vec<String>,
}

/// Decide why quote grounding could not be decided — or, when it was decided
/// but only partly covered, say which part was missed. `None` means every
/// quoted claim was checked.
fn not_evaluable_reason(
    stats: &QuoteStats,
    snapshots_supplied: usize,
    usable_snapshot_urls: usize,
) -> Option<String> {
    if stats.evaluable_claims == 0 {
        if stats.claims_with_quotes == 0 {
            return Some(format!(
                "no claim in the report body contains a quoted span of at least {MIN_QUOTE_CHARS} characters, so quote grounding is not evaluable"
            ));
        }
        if snapshots_supplied == 0 {
            return Some(format!(
                "{} claim(s) contain quotes but no retrieved snapshots were supplied (0 snapshots), so quote grounding is not evaluable",
                stats.claims_with_quotes
            ));
        }
        return Some(format!(
            "{} claim(s) contain quotes but none of their cited URLs has snapshot text ({} snapshot(s) supplied, {} usable after normalization), so quote grounding is not evaluable",
            stats.claims_with_quotes, snapshots_supplied, usable_snapshot_urls
        ));
    }
    if stats.uncheckable_claims > 0 {
        return Some(format!(
            "{} of {} quoted claim(s) are NotEvaluable (no snapshot text for any cited URL); c3 covers only the {} claim(s) that could be checked",
            stats.uncheckable_claims, stats.claims_with_quotes, stats.evaluable_claims
        ));
    }
    None
}

/// Walk the body, collect the claims, and ground their quotes.
fn audit_claims(
    body: &str,
    src_by_n: &HashMap<i64, String>,
    snapshots: &HashMap<String, String>,
) -> (Vec<ClaimAudit>, QuoteStats) {
    let mut out: Vec<ClaimAudit> = Vec::new();
    let mut stats = QuoteStats::default();
    for unit in sentence_units(body) {
        let mut cited_refs = inline_refs(&unit);
        cited_refs.sort_unstable();
        cited_refs.dedup();
        let mut cited_urls: Vec<String> = extract_urls(&unit)
            .iter()
            .map(|u| normalize_url(u))
            .collect();
        cited_urls.sort();
        cited_urls.dedup();
        // A sentence is a claim only if it points at something.
        if cited_refs.is_empty() && cited_urls.is_empty() {
            continue;
        }
        for n in &cited_refs {
            if let Some(u) = src_by_n.get(n) {
                cited_urls.push(u.clone());
            }
        }
        cited_urls.sort();
        cited_urls.dedup();

        let quotes = extract_quotes(&unit);
        let support = if quotes.is_empty() {
            ClaimSupport::QuoteNotRequired
        } else {
            stats.claims_with_quotes += 1;
            // Only text of URLs the claim itself cites may ground its quotes:
            // a quote found elsewhere on the web is not evidence for this claim.
            let texts: Vec<&String> = cited_urls.iter().filter_map(|u| snapshots.get(u)).collect();
            if texts.is_empty() {
                stats.uncheckable_claims += 1;
                ClaimSupport::NotEvaluable
            } else {
                stats.evaluable_claims += 1;
                stats.quotes_checked += quotes.len();
                let missing: Vec<String> = quotes
                    .iter()
                    .filter(|q| {
                        let needle = normalize_for_match(q);
                        needle.is_empty()
                            || !texts.iter().any(|t| t.as_str().contains(needle.as_str()))
                    })
                    .cloned()
                    .collect();
                if missing.is_empty() {
                    stats.grounded_claims += 1;
                    ClaimSupport::QuoteGrounded
                } else {
                    stats.missing_quotes.extend(missing);
                    ClaimSupport::QuoteMissing
                }
            }
        };
        out.push(ClaimAudit {
            claim: unit,
            cited_refs,
            cited_urls,
            quotes,
            support,
        });
    }
    dedupe_in_order(&mut stats.missing_quotes);
    (out, stats)
}

/// Keep the first occurrence of each string, preserving order.
fn dedupe_in_order(items: &mut Vec<String>) {
    let mut seen: HashSet<String> = HashSet::new();
    items.retain(|s| seen.insert(s.clone()));
}

/// Opening/closing delimiter pairs a quoted span may use.
///
/// `'` is listed as a pair with itself but is only an opener at a word
/// boundary (see [`is_opening_position`]). `”` is symmetric on purpose:
/// Arabic (and German) prose uses the same mark on both sides, so the pair
/// named in the spec as `”…”` is implemented as `”` … `”` — the ellipsis is
/// content, not a delimiter.
const QUOTE_PAIRS: &[(char, char)] = &[
    ('"', '"'),
    ('\'', '\''),
    ('“', '”'),
    ('‘', '’'),
    ('«', '»'),
    ('”', '”'),
];

fn quote_pair_for(c: char) -> Option<(char, char)> {
    QUOTE_PAIRS.iter().copied().find(|(open, _)| *open == c)
}

/// True when `byte_idx` may start a quoted span: the beginning of the text, or
/// right after whitespace / an opening bracket / a dash / another quote.
///
/// Without this, ASCII `'` — which is far more often an apostrophe than a
/// delimiter — would turn `don't ship it before friday` into the "quote"
/// `t ship it before friday` and fail an honest report.
fn is_opening_position(text: &str, byte_idx: usize) -> bool {
    match text[..byte_idx].chars().next_back() {
        None => true,
        Some(c) => {
            c.is_whitespace()
                || matches!(
                    c,
                    '(' | '['
                        | '{'
                        | '-'
                        | '—'
                        | '–'
                        | ':'
                        | '>'
                        | '*'
                        | '"'
                        | '“'
                        | '”'
                        | '«'
                        | '\''
                        | '‘'
                )
        }
    }
}

/// True when the `'` at `byte_idx` sits inside a word (`vendor's`, `don't`)
/// and therefore is an apostrophe, not a delimiter.
fn is_word_apostrophe(text: &str, byte_idx: usize) -> bool {
    let before = text[..byte_idx].chars().next_back();
    let after = text[byte_idx + 1..].chars().next();
    matches!((before, after), (Some(b), Some(a)) if b.is_alphanumeric() && a.is_alphanumeric())
}

/// Extract quoted spans from one sentence, in order of appearance.
///
/// Spans shorter than [`MIN_QUOTE_CHARS`] are dropped, and an unterminated
/// delimiter is ignored (it is a formatting accident, not evidence). Both
/// rules cut false FAILs; neither can create a match that is not in the text.
fn extract_quotes(text: &str) -> Vec<String> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let (bidx, c) = chars[i];
        let Some((_, close)) = quote_pair_for(c) else {
            i += 1;
            continue;
        };
        if c == '\'' && !is_opening_position(text, bidx) {
            i += 1;
            continue;
        }
        // Symmetric delimiters: the closer must come after the opener, and an
        // apostrophe inside a word never closes a span.
        let mut close_idx = None;
        for (j, &(cbidx, cc)) in chars.iter().enumerate().skip(i + 1) {
            if cc != close {
                continue;
            }
            if close == '\'' && is_word_apostrophe(text, cbidx) {
                continue;
            }
            close_idx = Some(j);
            break;
        }
        let Some(close_idx) = close_idx else {
            i += 1;
            continue;
        };
        let inner = text[bidx + c.len_utf8()..chars[close_idx].0].trim();
        if inner.chars().count() >= MIN_QUOTE_CHARS {
            out.push(inner.to_string());
        }
        i = close_idx + 1;
    }
    out
}

/// Normalization used for quote grounding: drop markdown emphasis and code
/// markers, map every quote-like mark to a straight one, collapse each run of
/// whitespace (including the `\r` of CRLF files and the breaks of a wrapped
/// paragraph) to a single space, and case-fold.
///
/// Quote and snapshot go through the same function, so the comparison is
/// symmetric: formatting differences cannot fake a match, and cannot hide one
/// either.
pub fn normalize_for_match(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_space = false;
    for ch in input.chars() {
        if matches!(ch, '*' | '_' | '`') {
            continue;
        }
        let ch = match ch {
            '“' | '”' | '„' | '‟' | '«' | '»' => '"',
            '‘' | '’' | '‚' | '‛' => '\'',
            _ => ch,
        };
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        for lower in ch.to_lowercase() {
            out.push(lower);
        }
    }
    out
}

/// Byte ranges that must never act as sentence boundaries: URLs and `[n]`
/// markers, so a `.` inside `https://x.org/a.html` or next to `[1]` cannot
/// split a claim. Trailing punctuation is excluded from the URL range so that
/// the `.` closing "see https://x.org/a." still ends the sentence.
fn protected_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = Vec::new();
    for m in url_re().find_iter(text) {
        let trimmed = m
            .as_str()
            .trim_end_matches(['.', ',', ';', ':', '!', '?', '،', '؛']);
        v.push((m.start(), m.start() + trimmed.len()));
    }
    for m in inline_ref_re().find_iter(text) {
        v.push((m.start(), m.end()));
    }
    v.sort_unstable();
    v
}

fn is_protected(ranges: &[(usize, usize)], byte_idx: usize) -> bool {
    ranges
        .iter()
        .any(|(start, end)| byte_idx >= *start && byte_idx < *end)
}

/// True when a line starts a new markdown block: a heading, a bullet, or an
/// ordered-list item.
///
/// Blocks are split before sentence detection so a bulleted list of cited
/// claims does not collapse into one sentence carrying every reference (which
/// would ground nothing and fail honest reports).
fn starts_block(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with('#') {
        return true;
    }
    if t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.starts_with('•')
        || t.starts_with("– ")
    {
        return true;
    }
    numbered_line_re().is_match(t)
}

/// Split the report body into sentence-sized units, in order.
///
/// Markdown blocks start a new unit; a plain line break inside a block is a
/// soft wrap and joins the previous line, so a quoted span wrapped across two
/// lines survives intact (a quote cut in half can never be grounded).
///
/// Terminators: `.`, `!`, `?`, `؟` (Arabic question mark), `。`, `！`, `？`.
/// Deliberately *not* a terminator: the Arabic comma `،` (it separates clauses
/// inside a sentence), a `.` between digits (`3.5`, `v1.2`), a `.` inside a URL
/// or a `[n]` marker, and any terminator inside a quoted span.
fn sentence_units(body: &str) -> Vec<String> {
    /// Emit the units of a finished block and start the next one.
    fn flush(block: &mut String, out: &mut Vec<String>) {
        if !block.trim().is_empty() {
            split_block_units(block, out);
        }
        block.clear();
    }
    let mut out: Vec<String> = Vec::new();
    let mut block = String::new();
    for line in body.lines() {
        if line.trim().is_empty() || starts_block(line) {
            flush(&mut block, &mut out);
            if line.trim().is_empty() {
                continue;
            }
        }
        if !block.is_empty() {
            block.push(' ');
        }
        block.push_str(line.trim());
    }
    flush(&mut block, &mut out);
    out
}

/// Split one markdown block into sentence-sized units.
fn split_block_units(block: &str, out: &mut Vec<String>) {
    let protected = protected_ranges(block);
    let chars: Vec<(usize, char)> = block.char_indices().collect();
    let mut start = 0usize;
    let mut open_quote: Option<(char, char)> = None;
    for (i, &(bidx, c)) in chars.iter().enumerate() {
        if is_protected(&protected, bidx) {
            continue;
        }
        // Track quoted spans so that a `.` inside a quotation never ends the
        // unit. An unterminated delimiter keeps the flag set to the end of the
        // block: fewer, larger claims (safe) instead of a truncated quote.
        let inside_quote = open_quote.is_some();
        match open_quote {
            Some((_, close))
                if c == close && !(close == '\'' && is_word_apostrophe(block, bidx)) =>
            {
                open_quote = None
            }
            Some(_) => {}
            None => {
                if let Some(pair) = quote_pair_for(c) {
                    if c != '\'' || is_opening_position(block, bidx) {
                        open_quote = Some(pair);
                    }
                }
            }
        }
        if inside_quote {
            continue;
        }
        let boundary = if c == '.' {
            dot_ends_sentence(&chars, i)
        } else {
            matches!(c, '!' | '?' | '؟' | '。' | '！' | '？')
        };
        if boundary {
            let end = bidx + c.len_utf8();
            let unit = block[start..end].trim();
            if !unit.is_empty() {
                out.push(unit.to_string());
            }
            start = end;
        }
    }
    let tail = block[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
}

/// Whether the `.` at `chars[i]` really ends a sentence.
///
/// Not a boundary: a decimal point (`3.5`, `0.308`), the first dots of an
/// ellipsis (`...` — the last dot ends it), and a `.` followed by a lowercase
/// fragment or a digit (`e.g. this`, `et al. 2024`). URLs and `[n]` markers are
/// filtered out by the caller before this runs.
fn dot_ends_sentence(chars: &[(usize, char)], i: usize) -> bool {
    let prev = chars[..i]
        .iter()
        .rev()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(_, c)| *c);
    let next = chars[i + 1..]
        .iter()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(_, c)| *c);
    if let (Some(p), Some(n)) = (prev, next) {
        if p.is_ascii_digit() && n.is_ascii_digit() {
            return false; // 3.5, v1.2, 0.308
        }
    }
    if next == Some('.') {
        return false; // first dots of an ellipsis
    }
    match next {
        None => true,
        Some(n) => !(n.is_lowercase() || n.is_ascii_digit()),
    }
}
