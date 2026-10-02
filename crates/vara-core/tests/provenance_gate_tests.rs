//! Gate-hardening regression tests.
//!
//! Two families:
//!   * the body/sources split must never panic — the writer prompt forces the
//!     report to end with `## Sources` and `entity.rs` stores
//!     `reply.content.trim()`, so "report ends exactly with the heading" is a
//!     reachable production input, not a synthetic one;
//!   * the Tier-0 claim audit (`check_provenance_full`) — verbatim quote
//!     grounding, and the honesty rule that "we could not check" is never
//!     reported as "we checked and it was fine".
//!
//! The inputs below are synthetic but shaped like the real failure modes. The
//! v1 experiment numbers live in `provenance_tests.rs` and are untouched.

use vara_core::provenance::{
    check_provenance, check_provenance_full, normalize_for_match, wilson_ci, ClaimAudit,
    ClaimSupport, GateReceipt, RetrievedSnapshot, GATE_VERSION, MIN_QUOTE_CHARS,
};

fn snap(url: &str, text: &str) -> RetrievedSnapshot {
    RetrievedSnapshot {
        url: url.to_string(),
        text: text.to_string(),
    }
}

/// The v1 single-agent report shape: 13 numbered sources, 4 really retrieved.
fn v1_style_report() -> (String, Vec<String>) {
    let sources: Vec<String> = (1..=13)
        .map(|i| format!("https://example{i}.org/doc/paper{i}"))
        .collect();
    let retrieved: Vec<String> = [0usize, 2, 6, 9]
        .iter()
        .map(|&i| sources[i].clone())
        .collect();
    let mut md = String::from("# LLM Inference Cost Research\n\n## Context windows\n");
    for i in 1..=13 {
        md.push_str(&format!(
            "Claim {} is supported by the literature [{}].\n",
            i, i
        ));
    }
    md.push_str("\n## Sources\n");
    for (i, u) in sources.iter().enumerate() {
        md.push_str(&format!("[{}] {} — Paper {}\n", i + 1, u, i + 1));
    }
    (md, retrieved)
}

// ---------- Deliverable 1: the split must not panic ----------

/// The writer prompt (`entity.rs`) demands a closing `## Sources` section and
/// the report is stored `.trim()`ed, so a model that stops right after the
/// heading produces a report whose last line has no trailing newline.
#[test]
fn sources_heading_last_line_without_newline_does_not_panic() {
    let report = "# Title\n\n## Body\nA claim backed by a citation [1].\n\n## Sources";
    let r = check_provenance(report, &[]);
    assert_eq!(r.verdict, "FAIL");
    assert!(!r.checks.c2_refs_resolve_to_source_list);
    assert_eq!(r.unresolved_refs, vec![1]);

    // The full gate must survive the same input.
    let (_, receipt, audits) = check_provenance_full(report, &[], &[]);
    assert_eq!(receipt.verdict, "FAIL");
    assert_eq!(audits.len(), 1);
}

/// Same input shape with the Arabic heading (the product is bilingual).
#[test]
fn arabic_sources_heading_last_line_without_newline_does_not_panic() {
    let report = "# تقرير\n\n## الجسم\nادعاء مدعوم بمرجع [1].\n\n## المصادر";
    let r = check_provenance(report, &[]);
    assert_eq!(r.verdict, "FAIL");
    assert_eq!(r.unresolved_refs, vec![1]);

    let (_, receipt, _) = check_provenance_full(report, &[], &[]);
    assert_eq!(receipt.verdict, "FAIL");
}

/// A report that is *only* the sources heading (truncated generation).
#[test]
fn report_that_is_only_the_sources_heading_does_not_panic() {
    let r = check_provenance("## Sources", &[]);
    assert_eq!(r.metrics.cited_total, 0);
    // Vacuously true structurally; the empty metrics are what the UI shows.
    assert_eq!(r.verdict, "PASS");

    // Identical input plus the newline the model usually emits, for parity.
    let r_nl = check_provenance("## Sources\n", &[]);
    assert_eq!(r_nl.verdict, r.verdict);
    assert_eq!(r_nl.metrics.cited_total, 0);

    // And through the full gate: not evaluable, and it says why.
    let (_, receipt, audits) = check_provenance_full("## Sources", &[], &[]);
    assert_eq!(audits.len(), 0);
    assert_eq!(receipt.n_claims, 0);
    assert_eq!(receipt.c3, None);
    assert!(receipt.claim_support_rate.is_none());
    let reason = receipt.not_evaluable_reason.as_deref().unwrap_or("");
    assert!(reason.contains("quoted span"), "reason: {reason}");
}

/// A report that is only a (non-sources) heading.
#[test]
fn report_that_is_only_a_title_heading_does_not_panic() {
    let r = check_provenance("# Title", &[]);
    assert_eq!(r.metrics.cited_total, 0);
    assert_eq!(r.verdict, "PASS");
}

/// Empty report (e.g. an empty model reply that still reached the checker).
#[test]
fn empty_report_does_not_panic() {
    let r = check_provenance("", &[]);
    assert_eq!(r.metrics.cited_total, 0);
    assert_eq!(r.metrics.retrieved_total, 0);
    assert_eq!(r.verdict, "PASS");
}

/// CRLF line endings: the heading must still be found and the last body
/// citation must survive the split. `str::lines()` strips the `\r`, so any
/// `line.len() + 1` cursor drifts by one byte per preceding line and silently
/// eats the tail of the body (here: the only real citation).
#[test]
fn crlf_report_keeps_last_citation_and_finds_heading() {
    let mut report = String::from("# Title\r\n\r\n## Body\r\n");
    for i in 1..=40 {
        report.push_str(&format!(
            "Filler line {i} adds length to the CRLF body.\r\n"
        ));
    }
    report.push_str("\r\nThe measured throughput is 3000 tok/s [1].\r\n\r\n## Sources\r\n");
    report.push_str("[1] https://bench.dev/llama3\r\n");

    let r = check_provenance(&report, &["https://bench.dev/llama3".to_string()]);
    assert_eq!(r.verdict, "PASS");
    assert_eq!(r.metrics.backed_ratio, 1.0);
    let body = r.sections.iter().find(|s| s.title == "Body").unwrap();
    // The `[1]` citation must still be inside the body, not sliced off.
    assert_eq!(
        body.citations, 1,
        "last body citation was lost in the split"
    );
    assert_eq!(body.backed_citations, 1);
}

/// A section *titled* like the sources heading is not the sources heading:
/// heading detection must stay exact (frozen behaviour).
#[test]
fn section_titled_sources_of_error_is_not_the_sources_section() {
    let report = "\
# Title\n\n## Body\nA claim [1].\n\n## Sources of error\nBias in the benchmark [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let r = check_provenance(report, &["https://bench.dev/a".to_string()]);
    assert_eq!(r.verdict, "PASS");
    assert_eq!(r.metrics.cited_total, 1);
    assert!(r.sections.iter().any(|s| s.title == "Sources of error"));
    assert!(!r.sections.iter().any(|s| s.title == "Sources"));
}

// ---------- Deliverable 2: Tier-0 claim audit ----------

#[test]
fn verbatim_quote_is_grounded() {
    let report = "# Title\n\n## Body\nThe paper states \"inference cost fell by 40 percent in 2024\" [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "Preliminary notes.\nInference cost fell by 40 percent in 2024 according to the vendor.",
    )];

    let (res, receipt, audits): (_, GateReceipt, Vec<ClaimAudit>) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);

    assert_eq!(res.verdict, "PASS");
    assert_eq!(res.metrics.backed_ratio, 1.0);
    assert_eq!(receipt.verdict, "PASS");
    assert_eq!(receipt.gate_version, GATE_VERSION);
    assert!(receipt.c1);
    assert!(receipt.c2);
    assert_eq!(receipt.c3, Some(true));
    assert_eq!(receipt.n_claims, 1);
    assert_eq!(receipt.n_claims_evaluable, 1);
    assert_eq!(receipt.n_quotes_checked, 1);
    assert_eq!(receipt.n_quotes_missing, 0);
    assert_eq!(receipt.claim_support_rate, Some(1.0));
    assert!(receipt.missing_quotes.is_empty());
    assert_eq!(receipt.not_evaluable_reason, None);

    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].support, ClaimSupport::QuoteGrounded);
    assert_eq!(audits[0].cited_refs, vec![1]);
    assert_eq!(audits[0].cited_urls, vec!["bench.dev/a".to_string()]);
    assert_eq!(
        audits[0].quotes,
        vec!["inference cost fell by 40 percent in 2024".to_string()]
    );
}

/// The failure C1/C2 cannot see: a real, retrieved URL behind a claim the
/// page does not make.
#[test]
fn quote_missing_fails_the_gate_and_names_the_quote() {
    let report = "# Title\n\n## Body\nThe paper states \"inference cost fell by 90 percent in 2025\" [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let retrieved = vec!["https://bench.dev/a".to_string()];

    // Structural checks see nothing wrong: the URL is real and retrieved.
    let structural = check_provenance(report, &retrieved);
    assert_eq!(structural.verdict, "PASS");
    assert_eq!(structural.metrics.backed_ratio, 1.0);

    let snaps = vec![snap(
        "https://bench.dev/a",
        "Inference cost fell by 40 percent in 2024.",
    )];
    let (res, receipt, audits) = check_provenance_full(report, &retrieved, &snaps);

    assert_eq!(receipt.c3, Some(false));
    assert_eq!(receipt.verdict, "FAIL");
    // The structural result is only ever made stricter, never looser.
    assert_eq!(res.verdict, "FAIL");
    assert_eq!(res.metrics.backed_ratio, 1.0);
    assert_eq!(
        receipt.missing_quotes,
        vec!["inference cost fell by 90 percent in 2025".to_string()]
    );
    assert_eq!(receipt.n_quotes_checked, 1);
    assert_eq!(receipt.n_quotes_missing, 1);
    assert_eq!(receipt.claim_support_rate, Some(0.0));
    assert_eq!(audits[0].support, ClaimSupport::QuoteMissing);
}

#[test]
fn quote_grounding_normalizes_markdown_case_whitespace_and_curly_quotes() {
    let report = "# Title\n\n## Body\nThe release notes say “throughput reached **3,000 tokens per second** on one H100” [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "Notes:\n\nthroughput reached 3,000   tokens\nper second on one H100 (median of 12 runs)",
    )];
    let (_, receipt, _) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);
    assert_eq!(receipt.c3, Some(true));
    assert_eq!(receipt.n_quotes_checked, 1);
    assert_eq!(receipt.claim_support_rate, Some(1.0));
}

/// Absence of evidence is never reported as a grounding pass.
#[test]
fn missing_snapshots_are_not_evaluable_and_never_a_grounding_pass() {
    let report = "# Title\n\n## Body\nThe paper states \"inference cost fell by 40 percent in 2024\" [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let retrieved = vec!["https://bench.dev/a".to_string()];

    for snaps in [
        // No snapshots at all.
        vec![],
        // A snapshot for a URL this claim does not cite.
        vec![snap(
            "https://other.dev/b",
            "unrelated page text that is long enough",
        )],
        // A snapshot with no usable text.
        vec![snap("https://bench.dev/a", "   \n  ")],
    ] {
        let (res, receipt, audits) = check_provenance_full(report, &retrieved, &snaps);
        assert_eq!(receipt.c3, None, "snapshots: {snaps:?}");
        assert!(receipt.claim_support_rate.is_none());
        assert!(receipt.claim_support_ci95.is_none());
        assert_eq!(receipt.n_quotes_checked, 0);
        assert_eq!(receipt.n_quotes_missing, 0);
        assert!(receipt.missing_quotes.is_empty());
        let reason = receipt.not_evaluable_reason.as_deref().unwrap_or("");
        assert!(
            reason.contains("snapshot"),
            "reason must say why it could not check: {reason}"
        );
        assert_eq!(audits[0].support, ClaimSupport::NotEvaluable);
        // Structural pass only — the receipt says grounding is unknown.
        assert_eq!(res.verdict, "PASS");
    }
}

#[test]
fn unquoted_claims_are_not_required_and_stay_out_of_the_rate() {
    let report = "# T\n\n## Body\nThroughput reached 3000 tok/s [1]. The vendor says \"latency stayed under 50 milliseconds\" [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "The vendor says latency stayed under 50 milliseconds in the 2024 run.",
    )];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);

    assert_eq!(receipt.n_claims, 2);
    assert_eq!(receipt.n_claims_evaluable, 1);
    assert_eq!(receipt.claim_support_rate, Some(1.0));
    assert_eq!(audits[0].support, ClaimSupport::QuoteNotRequired);
    assert_eq!(audits[1].support, ClaimSupport::QuoteGrounded);
    assert_eq!(receipt.c3, Some(true));
    assert_eq!(receipt.not_evaluable_reason, None);
}

/// One claim that could not be checked next to one that could: the rate stays
/// honest, the reason names the gap, and c3 covers only what was checked.
#[test]
fn not_evaluable_claims_are_disclosed_next_to_checked_ones() {
    let report = "# T\n\n## Body\nThe first claim quotes \"throughput rose by forty percent\" [1]. The second claim quotes \"latency fell to fifty milliseconds\" [2].\n\n## Sources\n[1] https://a.dev/x\n[2] https://b.dev/y\n";
    let snaps = vec![snap(
        "https://a.dev/x",
        "Benchmarks show throughput rose by forty percent over the previous release.",
    )];
    let retrieved = vec!["https://a.dev/x".to_string(), "https://b.dev/y".to_string()];
    let (_, receipt, audits) = check_provenance_full(report, &retrieved, &snaps);

    assert_eq!(receipt.n_claims, 2);
    assert_eq!(receipt.n_claims_evaluable, 1);
    assert_eq!(receipt.n_quotes_checked, 1);
    assert_eq!(receipt.claim_support_rate, Some(1.0));
    assert_eq!(receipt.c3, Some(true));
    assert_eq!(audits[1].support, ClaimSupport::NotEvaluable);
    let reason = receipt.not_evaluable_reason.as_deref().unwrap_or("");
    assert!(reason.contains("NotEvaluable"), "reason: {reason}");
    assert!(reason.contains("1 of 2"), "reason: {reason}");
}

#[test]
fn claim_citing_a_url_directly_is_grounded() {
    let report = "# T\n\n## Body\nAccording to https://bench.dev/direct the model serves \"at three thousand tokens per second\" on eight H100s.\n\n## Sources\n[1] https://bench.dev/direct\n";
    let snaps = vec![snap(
        "https://bench.dev/direct",
        "The model serves at three thousand tokens per second on eight H100s.",
    )];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/direct".to_string()], &snaps);
    assert_eq!(receipt.c3, Some(true));
    assert!(audits[0].cited_refs.is_empty());
    assert_eq!(audits[0].cited_urls, vec!["bench.dev/direct".to_string()]);
}

#[test]
fn snapshot_url_variants_match_the_cited_url() {
    let report = "# T\n\n## Body\nThe benchmark reports \"three thousand tokens per second\" [1].\n\n## Sources\n[1] https://bench.dev/llama3\n";
    let snaps = vec![snap(
        "https://www.Bench.dev/llama3/?utm_source=news#results",
        "the benchmark reports three thousand tokens per second on 8xH100",
    )];
    let (_, receipt, _) =
        check_provenance_full(report, &["https://bench.dev/llama3".to_string()], &snaps);
    assert_eq!(receipt.c3, Some(true));
    assert_eq!(receipt.n_quotes_checked, 1);
}

#[test]
fn decimals_and_reference_markers_do_not_split_sentences() {
    let report = "# T\n\n## Body\nThe measured cost was 3.5 dollars per million tokens [1], matching the v1.2 baseline of 0.308.\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "The measured cost was 3.5 dollars per million tokens, matching the baseline.",
    )];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);

    // One sentence, not three: no split on `3.5`, `v1.2` or `[1]`.
    assert_eq!(receipt.n_claims, 1, "audits: {audits:?}");
    assert_eq!(audits[0].cited_refs, vec![1]);
}

#[test]
fn arabic_question_mark_splits_but_arabic_comma_does_not() {
    let report = "# تقرير\n\n## الجسم\nهل انخفضت الكلفة فعلاً [1]؟ يقول التقرير «انخفضت كلفة الاستدلال بنسبة أربعين بالمئة في 2024» [1]، وهذا ما سنتحقق منه.\n\n## المصادر\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "انخفضت كلفة الاستدلال بنسبة أربعين بالمئة في 2024 حسب المزود.",
    )];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);

    assert_eq!(receipt.n_claims, 2, "audits: {audits:?}");
    assert_eq!(audits[0].support, ClaimSupport::QuoteNotRequired);
    assert_eq!(audits[1].support, ClaimSupport::QuoteGrounded);
    assert_eq!(receipt.c3, Some(true));
}

#[test]
fn apostrophes_do_not_create_bogus_quotes() {
    let report = "# T\n\n## Body\nThe vendor's note said 'throughput doubled on eight H100s' in 2024 [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "The vendor's note said throughput doubled on eight H100s in 2024.",
    )];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);

    // Exactly the single-quoted span: `vendor's` is not a delimiter.
    assert_eq!(
        audits[0].quotes,
        vec!["throughput doubled on eight H100s".to_string()]
    );
    assert_eq!(receipt.c3, Some(true));
}

#[test]
fn short_quotes_are_not_evidence() {
    let report = "# T\n\n## Body\nThe result was \"fine\" and the note was 'ok' [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap("https://bench.dev/a", "The result was acceptable.")];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);

    assert_eq!(MIN_QUOTE_CHARS, 12);
    assert!(audits[0].quotes.is_empty());
    assert_eq!(audits[0].support, ClaimSupport::QuoteNotRequired);
    assert_eq!(receipt.c3, None);
    assert!(receipt.claim_support_rate.is_none());
}

#[test]
fn multiple_quotes_report_only_the_missing_one() {
    let report = "# T\n\n## Body\nThe report says \"cost fell by forty percent\" and also \"latency dropped to five milliseconds\" [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "Cost fell by forty percent, the vendor said.",
    )];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);

    assert_eq!(audits[0].quotes.len(), 2);
    assert_eq!(audits[0].support, ClaimSupport::QuoteMissing);
    assert_eq!(
        receipt.missing_quotes,
        vec!["latency dropped to five milliseconds".to_string()]
    );
    assert_eq!(receipt.n_quotes_checked, 2);
    assert_eq!(receipt.n_quotes_missing, 1);
    assert_eq!(receipt.claim_support_rate, Some(0.0));
    assert_eq!(receipt.verdict, "FAIL");
}

/// A quote that exists on a *sibling* page is not evidence for this claim:
/// only the URLs the claim cites may ground it.
#[test]
fn a_quote_on_an_uncited_page_does_not_ground_the_claim() {
    let report = "# T\n\n## Body\nThe report says \"cost fell by forty percent in 2024\" [1].\n\n## Sources\n[1] https://a.dev/x\n[2] https://b.dev/y\n";
    let snaps = vec![snap(
        "https://b.dev/y",
        "Cost fell by forty percent in 2024 according to the vendor.",
    )];
    let (_, receipt, audits) = check_provenance_full(
        report,
        &["https://a.dev/x".to_string(), "https://b.dev/y".to_string()],
        &snaps,
    );
    assert_eq!(audits[0].support, ClaimSupport::NotEvaluable);
    assert_eq!(receipt.c3, None);
    assert!(receipt.claim_support_rate.is_none());
}

#[test]
fn bullet_list_claims_are_audited_separately() {
    let report = "# T\n\n## Body\n- The first claim quotes \"throughput rose by forty percent\" [1].\n- The second claim quotes \"latency fell to fifty milliseconds\" [2].\n\n## Sources\n[1] https://a.dev/x\n[2] https://b.dev/y\n";
    let snaps = vec![
        snap(
            "https://a.dev/x",
            "Across the suite throughput rose by forty percent.",
        ),
        snap(
            "https://b.dev/y",
            "Tail latency fell to fifty milliseconds in the same run.",
        ),
    ];
    let (_, receipt, audits) = check_provenance_full(
        report,
        &["https://a.dev/x".to_string(), "https://b.dev/y".to_string()],
        &snaps,
    );
    assert_eq!(receipt.n_claims, 2, "audits: {audits:?}");
    assert_eq!(receipt.n_quotes_checked, 2);
    assert_eq!(receipt.claim_support_rate, Some(1.0));
    assert_eq!(receipt.c3, Some(true));
}

/// A quoted span wrapped across two source lines is one quote, not two
/// fragments: the block join keeps it intact.
#[test]
fn quote_wrapped_across_lines_is_still_grounded() {
    let report = "# T\n\n## Body\nThe vendor said \"latency stayed\nunder fifty milliseconds\" in the new benchmark [1].\n\n## Sources\n[1] https://bench.dev/a\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "In the new benchmark latency stayed under fifty milliseconds.",
    )];
    let (_, receipt, audits) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);
    assert_eq!(receipt.n_claims, 1);
    assert_eq!(receipt.c3, Some(true), "audits: {audits:?}");
}

#[test]
fn crlf_quote_grounding_is_unaffected_by_carriage_returns() {
    let report = "# T\r\n\r\n## Body\r\nThe vendor said \"latency stayed under fifty milliseconds\" [1].\r\n\r\n## Sources\r\n[1] https://bench.dev/a\r\n";
    let snaps = vec![snap(
        "https://bench.dev/a",
        "In the new benchmark\nthe vendor reports latency stayed under fifty milliseconds.",
    )];
    let (_, receipt, _) =
        check_provenance_full(report, &["https://bench.dev/a".to_string()], &snaps);
    assert!(receipt.c1);
    assert!(receipt.c2);
    assert_eq!(receipt.c3, Some(true));
    assert_eq!(receipt.n_quotes_checked, 1);
}

/// C3 can only make the gate stricter: a grounded quote never rescues a
/// structural failure.
#[test]
fn grounded_quotes_never_rescue_a_structural_failure() {
    let report = "# T\n\n## Body\nThe page states \"cost fell by forty percent in 2024\" [1].\n\n## Sources\n[1] https://fabricated.dev/a\n";
    let snaps = vec![snap(
        "https://fabricated.dev/a",
        "Cost fell by forty percent in 2024.",
    )];
    let (res, receipt, _) = check_provenance_full(report, &[], &snaps);
    assert_eq!(receipt.c3, Some(true));
    assert!(!receipt.c1);
    assert_eq!(receipt.verdict, "FAIL");
    assert_eq!(res.verdict, "FAIL");
}

// ---------- Receipt metrics: n and CI, always ----------

#[test]
fn receipt_carries_n_and_ci_for_every_rate() {
    let (report, retrieved) = v1_style_report();
    let (res, receipt, audits) = check_provenance_full(&report, &retrieved, &[]);

    // The frozen v1 numbers, unchanged, through the full gate.
    assert_eq!(res.metrics.cited_total, 13);
    assert_eq!(res.metrics.retrieved_total, 4);
    assert!((receipt.backed_ratio - 0.308).abs() < 1e-9);
    assert!(receipt.backed_ratio_ci95 == wilson_ci(4, 13));
    assert!(!receipt.c1);
    assert!(receipt.c2);
    assert_eq!(receipt.verdict, "FAIL");
    assert_eq!(receipt.ref_resolution_rate, 1.0);

    // 13 claims, none of them quoted: not evaluable, and never a fake number.
    assert_eq!(receipt.n_claims, 13);
    assert_eq!(audits.len(), 13);
    assert_eq!(receipt.n_claims_evaluable, 0);
    assert!(receipt.claim_support_rate.is_none());
    assert!(receipt.claim_support_ci95.is_none());
    assert!(receipt.not_evaluable_reason.is_some());
    assert_eq!(receipt.gate_version, GATE_VERSION);
}

#[test]
fn ref_resolution_rate_tracks_unresolved_refs() {
    let report = "# T\n\n## Body\nOne claim [1]. Another claim [7]. A third [12].\n\n## Sources\n[1] https://a.dev/x\n";
    let (_, receipt, _) = check_provenance_full(report, &["https://a.dev/x".to_string()], &[]);
    // 3 distinct refs, 2 unresolved -> 1/3.
    assert!((receipt.ref_resolution_rate - 0.333).abs() < 1e-9);
    assert!(!receipt.c2);
    assert_eq!(receipt.verdict, "FAIL");
}

#[test]
fn wilson_ci_is_honest_at_the_extremes() {
    // No observations: everything is possible, and nothing is NaN.
    assert_eq!(wilson_ci(0, 0), (0.0, 1.0));
    // Known textbook values (95%, z = 1.96).
    let (lo, hi) = wilson_ci(1, 1);
    assert!((lo - 0.2065).abs() < 1e-3, "lo = {lo}");
    assert!((hi - 1.0).abs() < 1e-9, "hi = {hi}");
    let (lo, hi) = wilson_ci(0, 10);
    assert_eq!(lo, 0.0);
    assert!((hi - 0.2775).abs() < 1e-3, "hi = {hi}");
    let (lo, hi) = wilson_ci(5, 10);
    assert!((lo - 0.2366).abs() < 1e-3, "lo = {lo}");
    assert!((hi - 0.7634).abs() < 1e-3, "hi = {hi}");
    // Monotone in successes.
    let (lo0, _) = wilson_ci(2, 10);
    let (lo1, _) = wilson_ci(8, 10);
    assert!(lo0 < lo1);
}

#[test]
fn normalize_for_match_collapses_formatting_only() {
    assert_eq!(
        normalize_for_match("Vendor's *Latency*   stayed\nunder 50ms"),
        "vendor's latency stayed under 50ms"
    );
    assert_eq!(normalize_for_match("“Quoted”"), "\"quoted\"");
    assert_eq!(normalize_for_match("  spaced  out  "), "spaced out");
    // It must not invent content: different words stay different.
    assert_ne!(
        normalize_for_match("latency fell"),
        normalize_for_match("latency rose")
    );
}
