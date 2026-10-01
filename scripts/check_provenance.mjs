#!/usr/bin/env node
/**
 * check_provenance.mjs — the M1 gate (backed_ratio) as a standalone tool.
 * Port of vara_core::provenance so the harness pipeline can gate runs
 * without a Rust toolchain:
 *   C1: every link cited in the report must appear in ledger pages fetched.
 *   C2: every [n] reference must resolve to the report's source list.
 * Verdict FAIL blocks aggregation (GM-1 rule, v1 lesson: counting sources
 * is not enough — only retrieved-then-cited sources count).
 *
 * Usage: node scripts/check_provenance.mjs <run_folder>
 * Writes <run_folder>/checker.json.
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import path from "node:path";

const dir = process.argv[2];
if (!dir || !existsSync(dir)) {
  console.error("usage: node scripts/check_provenance.mjs <run_folder>");
  process.exit(2);
}

const report = readFileSync(path.join(dir, "report.md"), "utf8");
const ledger = JSON.parse(readFileSync(path.join(dir, "ledger.json"), "utf8"));

const fetched = new Set((ledger.pages || []).map((p) => p.url.replace(/[#?].*$/, "")));
const urls = [...report.matchAll(/https?:\/\/[^\s)\]}"'>]+/g)].map((m) => m[0].replace(/[#?].*$/, ""));
const cited = [...new Set(urls)];

const citedInLedger = cited.filter((u) => fetched.has(u));
const citedNotRetrieved = cited.filter((u) => !fetched.has(u));

const sourceSection = (() => {
  const idx = report.search(/sources?\s*:/i);
  return idx >= 0 ? report.slice(idx) : report;
})();
const numbered = [...sourceSection.matchAll(/^\s*\[?(\d{1,2})\]?\s*https?:\/\//gm)].map((m) => Number(m[1]));
const sourceIds = new Set(numbered);
const refsInBody = [...report.matchAll(/\[(\d{1,2})\]/g)].map((m) => Number(m[1]));
const unresolved = [...new Set(refsInBody.filter((n) => !sourceIds.has(n)))];

const citedTotal = cited.length;
const retrievedTotal = citedInLedger.length;
const backedRatio = citedTotal === 0 ? 0 : retrievedTotal / citedTotal;
const c1 = citedNotRetrieved.length === 0;
const c2 = unresolved.length === 0;

const verdict = c1 && c2 && backedRatio >= 0.95 ? "PASS" : backedRatio >= 0.8 && c1 ? "CONDITIONAL" : "FAIL";

const checker = {
  verdict,
  rule: "M1: cited ⊆ fetched AND [n] resolves; PASS needs backed_ratio >= 0.95 with both checks clean",
  metrics: { cited_total: citedTotal, retrieved_total: retrievedTotal, backed_ratio: Number(backedRatio.toFixed(3)) },
  cited_not_retrieved: citedNotRetrieved,
  unresolved_refs: unresolved,
  checks: { c1_all_cited_in_retrieved: c1, c2_refs_resolve_to_source_list: c2 },
};

writeFileSync(path.join(dir, "checker.json"), JSON.stringify(checker, null, 2));
console.log(`${dir}: ${verdict} (backed_ratio=${checker.metrics.backed_ratio})`);
process.exit(verdict === "FAIL" ? 1 : 0);
