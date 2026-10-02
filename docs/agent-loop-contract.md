# Agent loop and harness contract

Vara has two different loops. They must remain distinct in code, testing, and
product language.

## 1. Research mission loop

`EntityRuntime::run_mission` owns the durable research lifecycle:

```text
plan → retrieve → deduplicate → replan → write → provenance check → repair once → report
```

Its invariants are: token accounting starts with planning, step and token caps
are checked before each retrieval step, a report is written only from the
retrieval ledger, and a failed provenance result receives at most one repair.
On failure or cancellation the persisted mission retains its completed-step and
token totals; it must never look as if the work cost nothing.

## 2. Computer-use ActLoop

`computer_use::ActLoop` owns a separate, policy-gated UI execution lifecycle:

```text
parse whole sequence → grant check → SEE → ACT → CONFIRM → journal receipts
```

The model may propose a JSON sequence, but it cannot execute one directly. The
shell parses it, applies the autonomy grant, runs the loop through an adapter,
and persists every executed step. Coordinate actions without earlier evidence
are refused; destructive operations require L2 and explicit confirmation;
mutations that end a sequence unverified receive a structural evidence capture —
**conditional on the screen-capture grant** (`LoopPolicy::allow_screenshots`,
wired from `AutonomyConfig::allow_screenshots`). When that grant is off, the
loop refuses every SEE op *and* skips its implicit capture, so a run can finish
with `unverified_mutations > 0`; the honest signal there is
`LoopReport::fully_verified()` (or `verify_discipline() < 1.0`) and the error
text, never `completed == true` alone.

## Harness strategy

The deterministic `MockComputerUse` desktop tests outcomes *and* discipline.
The current scenarios cover a grounded Notepad flow, stale transient UI,
destructive dry-run/L2 behavior, blind-click refusal, focus theft, bounded
correction, schema rejection, action-journal completeness, grant mapping,
entry-point grant rejection, and final-evidence insertion.

This is deliberately not an LLM benchmark. It makes the executor trustworthy
first. An LLM-in-the-loop evaluator can submit generated sequences to the same
mock scenarios and score the existing `verify_discipline`, `grounded_clean`,
retry count, outcome, and journal completeness without loosening the runtime
guards.

## Non-negotiable grading rule

A scenario is not a pass merely because the final screen happens to look right.
It passes only when the outcome is right **and** the sequence used grounded,
policy-compliant, evidenced actions. This prevents luck from being mistaken for
agency.
