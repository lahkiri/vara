---
name: vara-provenance-invariants
description: Guards Vara's provenance gate (C1/C2) and honest-reporting rules. Use whenever touching reports, retrieval, citations, or the checker.
---

# Vara provenance invariants

The provenance checker is Vara's honesty spine. Reports may only cite sources
that were actually retrieved during the mission.

## Hard rules

- C1: every URL cited in a report must exist in the mission's retrieval ledger.
- C2: every `[n]` reference must resolve to the report's own `## Sources` list.
- A FAIL verdict triggers exactly ONE repair pass; if it still fails, the
  report ships with its FAIL verdict visible. Never hide the verdict.
- Never weaken `provenance.rs` thresholds, regexes, or section checks to turn a
  FAIL into a PASS. Fix retrieval instead, or declare `Gap:` explicitly.
- Deduplication (Jaccard >= 0.72) protects memory from rot; do not bypass
  `add_note_if_new` to inflate source counts.
- In chat (non-report context), Vara never invents citations or URLs; live
  data requests become mission proposals.

## Regression tests

v1 experiment data must stay green: if you change the checker, run
`cargo test -p vara-core` and keep every provenance test passing without
editing its assertions.
