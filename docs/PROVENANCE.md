# Provenance — how Vara decides whether a claim may ship

> The rule that turned an experiment failure into a product feature.

The v1 single-agent report cited 13 sources; 9 of them had never been
retrieved. `backed_ratio` 0.308. The gate exists so that this is *visible*
instead of shipped: evidence before claims.

Two promises frame everything below:

1. **A PASS is a floor, not a certificate.** It says the citation plumbing is
   intact, not that the report is true.
2. **Absence of evidence is never reported as evidence.** "We could not check
   this" is a real, displayable outcome — never a silent pass.

## Definitions

- **Retrieval ledger** — every URL the mission's tools actually surfaced
  (search results with `fetched = false`, fetched pages with `fetched = true`),
  recorded in the mission's `sources` rows.
- **Retrieved set** — the ledger URLs handed to the checker.
- **Snapshot** — the plain text Vara actually obtained for a URL
  (`tools::fetch_page` → `types::PageContent::text`, capped at `max_chars`).
- **Cited set** — every URL in the report body or its `## Sources` list, after
  normalization.
- **Claim** — a body sentence (outside the sources section) that contains at
  least one `[n]` reference or a URL.

## Normalization (harness parity)

Host lowercased, `www.` stripped, fragment stripped, trailing slash stripped,
tracking params (`utm_*`, `fbclid`, `ref`) dropped. Both sides normalize the
same way, so cosmetic URL variants neither hide a match nor fake one.

Quote grounding uses a second, text-level normalization: markdown emphasis and
code markers (`*`, `_`, `` ` ``) removed, curly quotes mapped to straight ones,
whitespace runs collapsed to one space, case folded. Quote and snapshot go
through the same function, so formatting can neither fake nor hide a match.

## The four tiers

Only Tier 0 exists today. The others are planned; they are written down here so
that the gap between "what we check" and "what we claim" stays explicit.

| tier | what it does | cost | what it catches |
|---|---|---|---|
| **0 — structural + verbatim quotes** (implemented) | C1/C2 on every report, plus: every quoted span of ≥ 12 characters must appear verbatim in the retrieved text of a URL that claim cites | 0 tokens, milliseconds, offline, deterministic | fabricated URLs, dangling `[n]`, quotes no retrieved page contains |
| **1 — local entailment** (planned) | deterministic, model-free comparison of a claim against the snapshot sentences around the matched terms: content-word overlap, numbers/units/dates, negation and polarity, hedging strength | 0 tokens, CPU only, every claim | paraphrase-level over-claiming, number/unit drift, "may improve" written as "improves" — the failures verbatim quoting cannot see |
| **2 — restricted LLM judge on suspicious spans only** (planned) | one call per *flagged* claim (never the whole report); the judge sees only the claim and the candidate snapshot passages — no mission memory, no tools — and must return one label plus the span it relied on | a few hundred tokens per suspicious claim, i.e. only for the minority Tier 1 flags | quote out of context, negation flips, entailment that word overlap gets wrong |
| **3 — human review queue** (planned) | reports that still FAIL after the single repair pass, and cases where Tier 2 disagrees with Tier 0, are queued with the claim, the snapshot spans and both verdicts | human minutes — the most expensive tier, so the most selective | everything the automated tiers cannot decide, including the gate's own false FAILs |

Tier 2 is a judgment, not evidence: the judge prompt hash, model id and raw
verdict are stored with the report so a later reader can tell a measurement
from an opinion. Tier 3 is the only tier allowed to overturn a Tier 0 FAIL.

## Citation failure taxonomy

| # | failure | example | caught by |
|---|---|---|---|
| 1 | fabricated URL | a cited page that was never retrieved | C1 (Tier 0) |
| 2 | real URL, wrong claim | the URL exists and was fetched, but the page never says what the report says | C3 for quoted claims (Tier 0); Tier 1/2 for paraphrases |
| 3 | quote out of context | verbatim words, meaning inverted by what surrounds them | not Tier 0; Tier 2, then Tier 3 |
| 4 | stale or mirrored source | a real, retrieved mirror or an outdated revision of the page | not the token-free tiers; Tier 2 can notice dates, Tier 3 decides |
| 5 | aggregator instead of primary | a blog post about the paper cited as if it were the paper | not automated today: needs source-class metadata and editorial policy; Tier 3 |
| 6 | over-claiming from an abstract | abstract says "may reduce cost", report says "reduces cost" | Tier 1 (hedging strength), Tier 2, Tier 3 |
| 7 | formatting-only mismatch | the quote is real but the page entity-encodes it or the snapshot was truncated | *gate error, not a source defect* — surfaces as `QuoteMissing`; see limits |

## What each check proves — and does not prove

| id | proves | does **not** prove |
|---|---|---|
| C1 | every URL in the report or its source list is in the mission's retrieval ledger | that the URL was fetched, that the page loaded today, that its content supports anything |
| C2 | every `[n]` in the body resolves to a numbered entry in the report's own source list | that the entry is relevant, or that the list is complete |
| C3 | every checked quoted span appears verbatim in the retrieved text of a URL that claim cites | that the quote is in context, that the source is primary or current, that the claim follows from the quote, or anything about claims that quote nothing |

**Verdict: PASS ⇔ C1 ∧ C2 ∧ C3 ≠ `Some(false)`.** A report with zero
citations still passes vacuously (harness parity) but its metrics show
`cited_total = 0` and the UI flags it as weak. `c3 = None` (not evaluable) does
not block a PASS — it blocks the *claim* that quotes were checked: the receipt
carries `not_evaluable_reason` and `claim_support_rate = None`, and any surface
that shows the verdict must show both.

Per-section coverage adds granularity: a section whose citations all resolve to
nothing retrieved is `effectively_uncovered` — the "honest gap" check. A
declared `Gap:` with no citations is honest; a paragraph citing unreachable
links is not.

## The repair pass

On FAIL, the runner performs exactly one repair attempt: the model receives the
failing report plus the checker output (unresolved refs, cited-but-never-
retrieved URLs, and — once wired — the offending quotes) and must remove or
replace invalid citations and declare gaps instead. The repaired version is
checked again and stored **alongside** the original (`repaired = 1`) — never
silently overwriting it. Repair can only fix what the receipt names, which is
why `missing_quotes` is part of the receipt.

## Honest limits (read this before trusting the badge)

1. **An HTTP failure is not proof of fabrication.** Paywalled, bot-blocked,
   geo-blocked and rate-limited publishers (403/429/robots walls) return no
   text. A URL that only appeared in a search result is in the ledger and
   passes C1 while having no snapshot at all: claims quoting it are
   `NotEvaluable`, which is honest — not `QuoteMissing`, which would be a
   false accusation.
2. **Link rot is widespread.** A URL that 404s today may have been live when
   the mission ran, and a URL the model could not have found may still be
   real. C1 is evidence about *Vara's retrieval*, not about the world.
3. **Snapshots are capped and lossy.** `fetch_page` truncates text
   (`max_chars`, min 200), refuses PDFs and non-text content types, and the
   HTML-to-text pass drops tables, captions and JS-rendered content. A genuine
   quote beyond the cap, or inside a chart, cannot be grounded. That is a
   false FAIL caused by the gate, and the fix is a better snapshot, not a
   weaker check.
4. **Formatting mismatches cut both ways.** Entity escapes (`&amp;`), unusual
   quote marks and hyphenation can hide a real quote; ≥ 12-character boilerplate
   ("Accept all cookies to continue") appearing in page chrome can ground a
   quote the article body never made. Normalization is deliberately narrow:
   quotes, emphasis, whitespace, case.
5. **A quote from an unfetched sibling source fails.** If a claim cites two
   URLs and only one has snapshot text, a quote that is not in the available
   text is reported `QuoteMissing` even if the other page really did say it.
   This is intentional — an unverifiable quote must not ship as verified — and
   the fix is to fetch the page, or to drop the citation.
6. **Sentence splitting is heuristic.** Bullets and headings start new claims;
   a wrapped line inside a paragraph is joined so a quote is never cut in half;
   `.` between digits and inside URLs/`[n]` markers never splits. A claim
   merged with its neighbour is checked against the union of both cited
   sources (lenient); a bulleted claim is checked on its own (strict).
7. **Tier 0 is lexical.** It cannot tell "supports" from "mentions", cannot see
   negation, cannot judge whether a number is the right number, and cannot
   price the difference between a primary source and a mirror. Those are
   Tier 1–3 problems.
8. **The gate is only as good as the ledger.** If a future feature fetches
   pages off-ledger, C1 becomes decorative for that path. Every retrieval path
   must write to `sources`.

## Metrics: definitions and the no-bare-PASS rule

All rates are rounded to three decimals, like `backed_ratio`. All intervals are
Wilson score intervals at 95% (`z = 1.96`); `n = 0` returns `(0.0, 1.0)` — with
no observations the honest interval is "anything", never `NaN`.

| metric | formula | read it with |
|---|---|---|
| `cited_total` | \|body URLs ∪ source-list URLs\|, normalized and deduped | — |
| `backed_ratio` | `(cited_total − (cited \ retrieved)) / cited_total`; `0.0` when `cited_total = 0` | `backed_ratio_ci95`, `cited_total` |
| `backed_ratio_ci95` | Wilson(`backed`, `cited_total`) | — |
| `ref_resolution_rate` | `(distinct refs − unresolved refs) / distinct refs`; `0.0` when the body has no refs | `c2` (true when there are no refs) |
| `n_claims` | body sentences citing a `[n]` or a URL | — |
| `n_claims_evaluable` | claims whose quotes could be compared against snapshot text | — |
| `claim_support_rate` | `grounded / evaluable`, or `None` when nothing was evaluable | `n_claims_evaluable`, `claim_support_ci95`, `not_evaluable_reason` |
| `claim_support_ci95` | Wilson(`grounded`, `evaluable`), `None` when nothing was evaluable | — |
| `n_quotes_checked` / `n_quotes_missing` / `missing_quotes` | quotes compared and not found, listed verbatim when `c3 = Some(false)` | `c3` |
| `c1`, `c2`, `c3` | the three checks above | `gate_version` |
| `gate_version` | `GATE_VERSION`, e.g. `vara-provenance/0.6` | always |

**Rule: never report a bare PASS.** Any surface that shows a verdict also shows
`n` and the interval for every rate next to it — a PASS with `cited_total` 0,
or a `claim_support_rate` of `1.0` computed over a single evaluable claim, is
visible as such. A rate without its denominator is not a measurement.

## Evaluating the gate itself (pre-registered protocol)

Frozen before the first run; results are recorded with this section unchanged.
Any change to the rules, thresholds or this protocol is a **new** gate version
and a new pre-registration, not an edit.

**Frozen inputs.** `GATE_VERSION`; the C1/C2/C3 rules and their
normalization; this document. Corpora:

- **Clean set** — reports written by real missions whose snapshots are
  complete. They must pass; every FAIL here is a false accusation.
- **Injected set** — clean reports perturbed by a written script, two
  injections per report, balanced across classes:
  - **I1 fabricated URL** — a cited URL that is not in the ledger. Tier 0 must
    catch every one of these; C1 is structural, it has no excuse.
  - **I2 real URL + fabricated quote** — a plausible quote that no snapshot
    contains. C3's job.
  - **I3 quote transplanted** — a verbatim quote lifted from another *cited*
    page. C3's job (the URL restriction).
  - **I4 quote out of context** — verbatim and correctly attributed, meaning
    inverted. Tier 0 cannot catch it; it is injected to *measure* that
    limitation, and its miss rate is published here rather than argued about.

**Endpoints** (each with n and a 95% Wilson interval — never a bare rate):

- sensitivity per injection class = caught / injected;
- **false-FAIL rate** = clean reports with verdict FAIL / clean reports. The
  gate crying wolf costs a repair pass and user trust, so this is measured, not
  assumed;
- **not-evaluable rate** = claims with `c3 = None` / claims. A gate that
  decides nothing has sensitivity 0 under a friendlier name.

**Decision rule (pre-registered).** Tier 0 may be the default gate only if I1
sensitivity is 1.0, I2 and I3 sensitivity are each ≥ 0.9, and the false-FAIL
rate on the clean set is ≤ 0.05. Misses that remain are named in this document
with their measured rate; they are not smoothed over by loosening a check.

**Hygiene.** Injection labels live outside the report text and are never fed to
the checker; corpora are hashed; runs record `gate_version`, corpus hash, model
id (not needed for Tier 0) and the exact command. Blind evaluation applies to
any human judgment in Tier 2/3.

**Falsifiers.** The gate is broken, and this document is wrong, if any of these
ever holds: an I1 injection passes C1; `c3 = Some(false)` with an empty
`missing_quotes`; a `claim_support_rate` of `Some(x)` with
`n_claims_evaluable = 0`; a verdict without n and CI on any user-facing
surface.

## Regression tests

`crates/vara-core/tests/provenance_tests.rs` replays the v1 failure modes and
must keep passing without edits to its assertions:

- `regression_v1_single_agent_backed_ratio_308` — 13 cited, 4 retrieved →
  exactly **0.308**, verdict FAIL (the fabrication case).
- `structural_fail_when_refs_do_not_resolve` — the team-report pattern
  ([7]…[27] dangling) fails C2.
- `clean_report_passes_with_full_ratio`, `zero_citations_is_vacuous_pass…`,
  `url_normalization_matches_harness`, `section_level_uncovered_detection`.

`crates/vara-core/tests/provenance_gate_tests.rs` guards the hardening work:

- `sources_heading_last_line_without_newline_does_not_panic`,
  `arabic_sources_heading_last_line_without_newline_does_not_panic`,
  `report_that_is_only_the_sources_heading_does_not_panic` — the split used to
  slice past the end of a trimmed report and take the whole mission task down.
- `crlf_report_keeps_last_citation_and_finds_heading` — CRLF drift used to eat
  the tail of the body.
- `quote_missing_fails_the_gate_and_names_the_quote` — real URL, wrong claim:
  C1/C2 pass, the receipt fails and names the quote.
- `missing_snapshots_are_not_evaluable_and_never_a_grounding_pass` — the
  honesty rule, including empty and for-another-URL snapshots.
- `receipt_carries_n_and_ci_for_every_rate`, `wilson_ci_is_honest_at_the_extremes`
  — the metrics contract.

If any of these fail, the gate changed — and that must never happen silently.

## Versioning

`GATE_VERSION` (in `provenance.rs`) plus this document are bumped together on
any change to checks, normalization, extraction or verdict rule. A number
recorded without its gate version is not comparable across runs.

`check_provenance_full(report, retrieved, snapshots) ->
(ProvenanceResult, GateReceipt, Vec<ClaimAudit>)` is the Tier-0 entry point.
`check_provenance(report, retrieved)` keeps its frozen C1/C2 behaviour and
numbers for the harness and existing callers. Wiring `check_provenance_full`
into the mission runner (so it passes the fetched page texts) is the next step;
until that lands, the product path enforces C1/C2 only, and reports say so.
