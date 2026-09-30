//! Structural citation checker — the GM-1 gate, ported 1:1 from
//! `scripts/check_provenance.mjs` (v1 experiment tooling) into the product.
//!
//! Rules (frozen after the v1 experiment where 9 of 13 sources in the
//! single-agent report had never been retrieved — backed_ratio 0.308):
//!   C1: every cited link must belong to the actually-retrieved set.
//!   C2: every numeric [n] reference must resolve to a numbered source entry.
//!
//! This module is deliberately regex/manual-parsing and API-free: cheap,
//! deterministic, CI-friendly. Regression tests in `tests/` replay v1 data.

use crate::types::{ProvenanceChecks, ProvenanceMetrics, ProvenanceResult, ProvenanceSection};
use regex::Regex;
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

/// Run the structural check.
///
/// `retrieved`: every URL that was actually fetched/searched during the
/// mission (the retrieval ledger). Returns the same shape as the harness
/// `checker.json` so numbers stay comparable across v1 / v2 / product.
pub fn check_provenance(report: &str, retrieved: &[String]) -> ProvenanceResult {
    // 1) Retrieved set: normalized URL -> first original URL.
    let mut retrieved_map: HashMap<String, String> = HashMap::new();
    for u in retrieved {
        let k = normalize_url(u);
        retrieved_map.entry(k).or_insert_with(|| u.to_string());
    }

    // 2) Split report into body + sources section.
    let mut body = report.to_string();
    let mut src_section = String::new();
    let mut byte_cursor = 0usize;
    let mut found_heading = false;
    for line in report.lines() {
        let line_len = line.len() + 1;
        if !found_heading && is_sources_heading(line) {
            body = report[..byte_cursor].to_string();
            src_section = report[byte_cursor + line_len..].to_string();
            found_heading = true;
            break;
        }
        byte_cursor += line_len;
    }
    if !found_heading {
        body = report.to_string();
        src_section = String::new();
    }

    // 3) Source list entries: numbered [n] or bare URL lines.
    let mut source_entries: Vec<(Option<i64>, String)> = Vec::new();
    for line in src_section.lines() {
        let urls = extract_urls(line);
        if urls.is_empty() {
            continue;
        }
        let first_norm = normalize_url(&urls[0]);
        match numbered_line_re().captures(line) {
            Some(c) => {
                let n = c[1].parse::<i64>().ok();
                source_entries.push((n, first_norm));
            }
            None => source_entries.push((None, first_norm)),
        }
    }
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
