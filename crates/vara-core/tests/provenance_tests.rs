//! Regression tests seeded from the v1 experiment data. These numbers are
//! not invented: `report_single.md` (v1) had 13 cited sources of which 9 had
//! never been retrieved — backed_ratio 0.308. If these tests fail, the gate
//! that protects users from fabricated provenance is broken.

use vara_core::provenance::{check_provenance, normalize_url};

/// Builds a v1-style report: 13 numbered sources, 4 of them really retrieved.
fn v1_style_report() -> (String, Vec<String>) {
    let sources: Vec<String> = (1..=13)
        .map(|i| format!("https://example{i}.org/doc/paper{i}"))
        .collect();
    // Retrieved: the URLs behind citations [1], [3], [7], [10].
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

#[test]
fn regression_v1_single_agent_backed_ratio_308() {
    let (report, retrieved) = v1_style_report();
    let r = check_provenance(&report, &retrieved);
    assert_eq!(r.metrics.cited_total, 13);
    assert_eq!(r.metrics.retrieved_total, 4);
    // 4 backed of 13 cited = 0.308 — the exact v1 single-agent number.
    assert!(
        (r.metrics.backed_ratio - 0.308).abs() < 1e-9,
        "got {}",
        r.metrics.backed_ratio
    );
    assert_eq!(r.verdict, "FAIL");
    assert!(!r.checks.c1_all_cited_in_retrieved);
    assert_eq!(r.cited_not_retrieved.len(), 9);
    assert!(r.unresolved_refs.is_empty());
}

#[test]
fn structural_fail_when_refs_do_not_resolve() {
    // The v1 team report pattern: internal refs [7]..[27] with no source list.
    let report = "\
# Report\n\n## Findings\nSomething important [7]. Something else [12].\n\n## More\nEven more [27].\n\n## Sources\n[1] https://real.org/a\n[2] https://real.org/b\n[3] https://real.org/c\n[4] https://real.org/d\n";
    let retrieved = vec![
        "https://real.org/a".into(),
        "https://real.org/b".into(),
        "https://real.org/c".into(),
        "https://real.org/d".into(),
    ];
    let r = check_provenance(report, &retrieved);
    assert_eq!(r.verdict, "FAIL");
    assert!(!r.checks.c2_refs_resolve_to_source_list);
    assert!(r.unresolved_refs.contains(&7));
    assert!(r.unresolved_refs.contains(&12));
    assert!(r.unresolved_refs.contains(&27));
}

#[test]
fn clean_report_passes_with_full_ratio() {
    let report = "\
# Inference costs\n\n## Numbers\nLlama 3 70B serves at ~3000 tok/s on 8xH100 [1].\n\n## Sources\n[1] https://bench.dev/llama3\n";
    let r = check_provenance(report, &["https://bench.dev/llama3".to_string()]);
    assert_eq!(r.verdict, "PASS");
    assert_eq!(r.metrics.backed_ratio, 1.0);
    assert!(r.checks.c1_all_cited_in_retrieved);
    assert!(r.checks.c2_refs_resolve_to_source_list);
    // The section is effectively covered.
    assert!(r.sections.iter().all(|s| !s.effectively_uncovered));
}

#[test]
fn zero_citations_is_vacuous_pass_but_flagged_by_metrics() {
    // Parity with the harness: vacuous PASS — the UI surfaces
    // cited_total == 0 as a weak report instead of trusting the verdict.
    let report = "# Title\n\n## Body\nNo citations here at all.\n";
    let r = check_provenance(report, &[]);
    assert_eq!(r.verdict, "PASS");
    assert_eq!(r.metrics.cited_total, 0);
    assert!(r.sections.iter().all(|s| s.no_citations));
}

#[test]
fn url_normalization_matches_harness() {
    assert_eq!(
        normalize_url("https://www.Example.com/path/?utm_source=x&utm_campaign=y&id=7#top"),
        "example.com/path?id=7"
    );
    assert_eq!(normalize_url("https://example.com/"), "example.com");
    assert_eq!(
        normalize_url("https://example.com/a?fbclid=zz&ref=t&keep=1."),
        "example.com/a?keep=1"
    );
}

#[test]
fn section_level_uncovered_detection() {
    let report = "\
# Title\n\n## Covered part\nA fact with a real citation [1].\n\n## Fabricated part\nA suspicious claim [2].\n\n## Sources\n[1] https://ok.dev/a\n[2] https://fabricated.dev/b\n";
    let r = check_provenance(report, &["https://ok.dev/a".to_string()]);
    let covered = r
        .sections
        .iter()
        .find(|s| s.title == "Covered part")
        .unwrap();
    let fabricated = r
        .sections
        .iter()
        .find(|s| s.title == "Fabricated part")
        .unwrap();
    assert!(!covered.effectively_uncovered);
    assert!(fabricated.effectively_uncovered);
    assert_eq!(r.verdict, "FAIL");
}
