---
name: vara-experiment-protocol
description: How to run Vara's pre-registered experiments (four-arm design) without invalidating them. Use when touching entity.rs, harnesses, metrics, or experiment docs.
---

# Vara experiment protocol

The v2 design (docs/EXPERIMENT_DESIGN.md) is pre-registered: arms A/B/C/D,
decision thresholds frozen BEFORE running (R1: dynamic organization survives
only if M2(D) >= M2(C)+1.0 and M3(D) <= 1.5*M3(C)).

## Hard rules

- Freeze thresholds before the first run of an experiment. If results force a
  rule change, that is a NEW pre-registration, not an edit.
- Never modify the rubric after seeing results; never relax a checker to make
  an arm pass.
- Record queries, URLs, model name, and token spend in the ledger for every
  run (the v1 provenance failure taught us this the hard way).
- n >= 3 per arm; blind evaluation for coverage/quality judgments.
- Notes dedup (Jaccard >= 0.72) and query dedup stay enabled during runs —
  inflated coverage is dishonest coverage.
- The "report failures instead of using stale data" policy applies: an arm
  that cannot retrieve says so; it does not quietly fall back to memory.
