# Where to start

Vara is a resident agent that runs on your own machine: it plans missions, keeps
durable memory, uses governed capabilities, and refuses to call a report
"verified" unless the sources behind it survive a check. This page says which
document answers which question, so nobody has to read twenty-one files to find
one answer.

---

## "I want to know what this actually does today"

| Read | Why |
|---|---|
| [`../README.md`](../README.md) | The product at a glance, and what shipped in the current release |
| **[`REALITY_MATRIX.md`](REALITY_MATRIX.md)** | **Every product claim, what the code actually does, the production path, who proved it and with which command, and the decision.** Start here when the question is "is this real?" |
| [`ROADMAP.md`](ROADMAP.md) | What shipped, what is open, and what is explicitly not being worked on |

The matrix is deliberately uncomfortable reading: it records the claims that are
**false** as well as the ones that hold, and it records the two accusations an
audit made that turned out to be **wrong**.

## "I want to use it"

| Read | Why |
|---|---|
| [`../README.md`](../README.md) § Download / Build | Install or build from source |
| [`../README.ar.md`](../README.ar.md) | نفس المحتوى بالعربية |
| [`PLUGIN_GUIDE.md`](PLUGIN_GUIDE.md) | Add, remove or write a plugin — for developers **and** for non-developers (§10) |

## "I want to understand how it is built"

| Read | Why |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | The composition model, the module map, the mission lifecycle, the action boundary |
| [`agent-loop-contract.md`](agent-loop-contract.md) | What a loop turn may and may not do |
| [`PROVENANCE.md`](PROVENANCE.md) | The gate: what C1/C2/C3 each prove, and what they deliberately do not |
| [`COMPUTER_USE.md`](COMPUTER_USE.md) | The ActLoop: see → act → confirm, grants, journal |
| [`PERSONAS.md`](PERSONAS.md) | Voice and visual identity as data |

## "I am reviewing it, or attacking it"

| Read | Why |
|---|---|
| [`REALITY_MATRIX.md`](REALITY_MATRIX.md) | Claims vs code, with the proving command per row |
| [`../SECURITY.md`](../SECURITY.md) | The trust boundary, what ships off, and the limits that are not solved |
| [`PLUGIN_INVENTORY.md`](PLUGIN_INVENTORY.md) | Generated from `plugins/` — the file cannot drift from the folders |
| [`../AGENTS.md`](../AGENTS.md) | The invariants a change must not break |

## "I want the experiment this product came from"

| Read | Why |
|---|---|
| [`experiments/v1/HYPOTHESIS_VERDICT.md`](experiments/v1/HYPOTHESIS_VERDICT.md) | The v1 failure: a formally consistent report whose sources were mostly fabricated (9/13) |
| [`experiments/v1/`](experiments/v1/) | Raw ledgers, the fabrication audit, the checker output |
| [`experiments/v2-design/EXPERIMENT_DESIGN.md`](experiments/v2-design/EXPERIMENT_DESIGN.md) | The pre-registered v2 design with frozen decision thresholds |

## Working notes (not product documentation)

These were the working order for a release that has since shipped. They are kept
because they explain *why* several decisions were made, but they are not a
description of the current product — treat them as history, not as a plan.

- [`tasks/ALIVE_V2_SPEC.md`](tasks/ALIVE_V2_SPEC.md), [`tasks/ALIVE_PACK.md`](tasks/ALIVE_PACK.md)
  — the v0.6-era work order
- [`tasks/QUESTIONS.md`](tasks/QUESTIONS.md) — decisions that needed the owner; the
  ones already taken are marked
- [`audit/AUDIT_AND_UPGRADE_PLAN.md`](audit/AUDIT_AND_UPGRADE_PLAN.md) — the v0.6
  code-grounded audit; items still open are marked
- [`design-research.md`](design-research.md), [`product-decisions.md`](product-decisions.md)

---

## The one rule that applies to all of it

A claim in any document is only as good as the command that demonstrates it. When
a document and the code disagree, **the code is right and the document is a bug** —
and `scripts/check-consistency.mjs` (run in CI) exists to make that specific class
of bug fail the build rather than ship.
