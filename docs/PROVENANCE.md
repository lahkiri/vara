# Provenance — the GM-1 gate

> The rule that turned an experiment failure into a product feature.

## Definitions

- **Retrieved set** — every URL Vara actually obtained during a mission
  (search results AND fetched pages, each recorded in `sources` with a
  `fetched` flag).
- **Cited set** — every URL appearing in the report body or its `## Sources`
  list, after normalization.
- **backed_ratio** — |cited ∩ retrieved| / |cited|.

## Normalization (harness parity)

Host lowercased, `www.` stripped, fragment stripped, trailing slash stripped,
tracking params (`utm_*`, `fbclid`, `ref`) dropped. Both sides normalize the
same way, so cosmetic URL variants never hide a match nor fake one.

## Checks

| id | rule | failure means |
|---|---|---|
| C1 | every cited URL ∈ retrieved set | a cited source was never actually retrieved — likely fabricated |
| C2 | every numeric `[n]` in the body resolves to a numbered entry in the report's source list | dangling internal references (the v1 team-report defect) |

**Verdict: PASS ⇔ C1 ∧ C2.** A report with zero citations passes vacuously
(harness parity) but the UI flags it as *weak* — metrics are always shown
beside the verdict.

Per-section coverage adds granularity: a section whose citations all resolve
to nothing retrieved is `effectively_uncovered` — the "honest gap" check. A
declared `Gap:` with no citations is honest; a paragraph citing unreachable
links is not.

## The repair pass

On FAIL, the runner performs exactly one repair attempt: the model receives
the failing report plus the checker output (unresolved refs, cited-but-never-
retrieved URLs) and must remove/replace invalid citations and declare gaps
instead. The repaired version is checked again and stored **alongside** the
original (`repaired = 1`) — never silently overwriting it.

## Regression tests seeded from v1

`crates/vara-core/tests/provenance_tests.rs` replays the v1 failure modes:

- `regression_v1_single_agent_backed_ratio_308` — 13 cited, 4 retrieved →
  exactly **0.308**, verdict FAIL (the fabrication case).
- `structural_fail_when_refs_do_not_resolve` — the team-report pattern
  ([7]…[27] dangling) fails C2.
- `clean_report_passes_with_full_ratio`, `zero_citations_is_vacuous_pass…`,
  `url_normalization_matches_harness`, `section_level_uncovered_detection`.

If any of these fail, the gate changed — and that must never happen silently.

## Limits (read this before trusting the badge)

1. C1 proves the link was *retrieved*, not that the source *supports* the
   claim or is *true*. It is a floor against fabrication, not a ceiling on
   correctness.
2. A retrieved page can still contain wrong information; downstream
   verification (and humans) still matter.
3. The gate is only as good as the ledger: if a future feature fetches pages
   off-ledger, the gate must be fed that ledger too.
