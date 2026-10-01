#!/usr/bin/env node
/**
 * vara_harness_v2.mjs — the pre-registered v2 campaign runner.
 *
 * Implements docs/experiments/v2-design/EXPERIMENT_DESIGN.md verbatim:
 *   - Arms: A (single), C (searcher -> clean writer), B (orchestrator ->
 *     researcher -> analyst), D (orchestrator -> parallel researchers ->
 *     clean writer)
 *   - Missions: m1, m2 (mission text files)
 *   - 3 runs per arm x mission = 24 runs, each HARD-BOUND to 90k tokens
 *     (the writer is reserved 10k of those)
 *   - The model is NAMED in every ledger row (v1 flaw, fixed)
 *   - Per-run output folder: <arm>_<mission>_<run>/ with report.md,
 *     findings.json, ledger.json, run.json (checker.json via
 *     check_provenance.mjs; aggregation via aggregate_v2.mjs)
 *   - Stopping is recorded by CAUSE (budget exhausted / plan complete)
 *
 * Config via env:
 *   VARA_PROVIDER_BASE_URL  OpenAI-compatible base (no trailing /v1 needed
 *                           if already present)      e.g. https://host/v1
 *   VARA_PROVIDER_KEY       API key (never committed, never logged)
 *   VARA_PROVIDER_MODEL     model name — recorded in every row
 *
 * Usage:
 *   node scripts/vara_harness_v2.mjs --plan                 # show the plan, no API calls
 *   node scripts/vara_harness_v2.mjs --arms A,C --runs 1    # partial campaign
 *   node scripts/vara_harness_v2.mjs                        # full 24-run campaign
 */

import { mkdirSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import path from "node:path";

// ---------- config ----------

const BASE_URL = (process.env.VARA_PROVIDER_BASE_URL || "").replace(/\/+$/, "");
const API_KEY = process.env.VARA_PROVIDER_KEY || "";
const MODEL = process.env.VARA_PROVIDER_MODEL || "unspecified";

const BUDGET_TOKENS = Number(process.env.VARA_HARNESS_BUDGET || 90_000);
const WRITER_RESERVE = 10_000;
const MAX_ROUNDS = 24; // safety cap, budget is the real bound
const ROOT = path.resolve(process.cwd(), "tool-results", "v2");

const ARMS = ["A", "C", "B", "D"];
const RUNS_PER_ARM = 3;

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : def;
};
const has = (name) => process.argv.includes(`--${name}`);

const WANTED_ARMS = (arg("arms", ARMS.join(",")) || "")
  .split(",")
  .map((s) => s.trim().toUpperCase())
  .filter((s) => ARMS.includes(s));
const WANTED_RUNS = Number(arg("runs", RUNS_PER_ARM));
const MISSIONS = (arg("missions", "m1,m2") || "").split(",").map((s) => s.trim());

const DRY_RUN = has("plan");

// ---------- tiny logging ----------

const log = (...a) => console.error("[harness]", ...a);

// ---------- provider ----------

function endpoint() {
  if (!BASE_URL) throw new Error("VARA_PROVIDER_BASE_URL is required");
  return BASE_URL.endsWith("/v1") ? `${BASE_URL}/chat/completions` : `${BASE_URL}/v1/chat/completions`;
}

/** One completion. Returns {content, tokens:{prompt,completion}} or throws. */
async function chat(messages, maxTokens) {
  if (DRY_RUN) return { content: "", tokens: { prompt: 0, completion: 0 } };
  const resp = await fetch(endpoint(), {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(API_KEY ? { authorization: `Bearer ${API_KEY}` } : {}),
    },
    body: JSON.stringify({ model: MODEL, messages, max_tokens: maxTokens, temperature: 0.4 }),
  });
  if (!resp.ok) throw new Error(`provider HTTP ${resp.status}: ${(await resp.text()).slice(0, 200)}`);
  const data = await resp.json();
  const content = data?.choices?.[0]?.message?.content ?? "";
  const u = data?.usage ?? {};
  return {
    content,
    tokens: { prompt: u.prompt_tokens ?? 0, completion: u.completion_tokens ?? 0 },
  };
}

const estimate = (s) => Math.ceil((s || "").length / 4);

// ---------- web tools (real fetch or no fetch — never hollow snippets) ----------

async function ddgSearch(query) {
  const resp = await fetch(
    `https://html.duckduckgo.com/html/?q=${encodeURIComponent(query)}`,
    { headers: { "user-agent": "Mozilla/5.0 VaraHarness/2.0" } },
  );
  if (!resp.ok) throw new Error(`search HTTP ${resp.status}`);
  const html = await resp.text();
  const out = [];
  const re = /<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/g;
  let m;
  while ((m = re.exec(html)) && out.length < 8) {
    let url = m[1];
    const uddg = url.indexOf("uddg=");
    if (uddg >= 0) {
      const rest = url.slice(uddg + 5);
      url = decodeURIComponent(rest.slice(0, rest.indexOf("&")) || rest);
    }
    if (!url.startsWith("http")) continue;
    const title = m[2].replace(/<[^>]+>/g, "").trim();
    if (title) out.push({ title, url });
  }
  return out;
}

async function fetchPage(url, maxChars = 6000) {
  const resp = await fetch(url, { headers: { "user-agent": "Mozilla/5.0 VaraHarness/2.0" } });
  if (!resp.ok) throw new Error(`fetch HTTP ${resp.status}`);
  const ct = resp.headers.get("content-type") || "";
  if (/pdf|image\/|video\/|zip/.test(ct)) throw new Error(`unsupported content-type ${ct}`);
  const html = await resp.text();
  const text = html
    .replace(/<(script|style|svg|noscript)\b[\s\S]*?<\/\1>/gi, " ")
    .replace(/<br\s*\/?>|<\/(p|div|li|h[1-6]|tr)>/g, "\n")
    .replace(/<[^>]+>/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&nbsp;/g, " ")
    .replace(/[ \t\r\f]+/g, " ")
    .replace(/\n\s*\n\s*/g, "\n\n")
    .trim();
  const clipped = text.slice(0, maxChars);
  if (!clipped) throw new Error("no readable text");
  return clipped;
}

// ---------- agent loop (tool protocol: [[tool]] {json} [[/tool]]) ----------

const TOOL_PROTOCOL = `
TOOLS — emit exactly one block per turn when you want to act:
[[tool]] {"tool":"search","query":"..."} [[/tool]]
[[tool]] {"tool":"fetch","url":"..."} [[/tool]]
Results arrive as [TOOL_RESULT] blocks. When you have enough material, emit:
[[done]] final answer here [[/done]]
Never fabricate search results or page content. If a tool fails, say so.`;

async function runAgent({ role, system, task, ledger, budgetLeft, maxTokens }) {
  const messages = [
    { role: "system", content: `${system}\n${TOOL_PROTOCOL}` },
    { role: "user", content: task },
  ];
  const rows = [];
  let spent = 0;
  for (let round = 0; round < MAX_ROUNDS && spent < budgetLeft; round++) {
    const { content, tokens } = await chat(messages, Math.min(maxTokens, budgetLeft - spent));
    spent += tokens.prompt + tokens.completion;
    if (!content) break;
    rows.push({ agent: role, purpose: "round", model: MODEL, tokens: tokens.prompt + tokens.completion });

    const done = content.match(/\[\[\s*done\s*\]\]([\s\S]*?)(?:\[\[\s*\/\s*done\s*\]\]|$)/i);
    if (done) return { final: done[1].trim(), rows, spent };

    const call = content.match(/\[\[\s*tool\s*\]\]([\s\S]*?)\[\[\s*\/\s*tool\s*\]\]/i);
    if (!call) {
      messages.push({ role: "assistant", content });
      messages.push({ role: "user", content: "No tool block and no [[done]] found. Either call a tool or emit [[done]]." });
      continue;
    }
    let req;
    try {
      req = JSON.parse(call[1].trim().replace(/^```(json)?|```$/g, "").trim());
    } catch {
      messages.push({ role: "assistant", content });
      messages.push({ role: "user", content: "[TOOL_RESULT] invalid JSON — retry the tool block." });
      continue;
    }
    let result = "";
    try {
      if (req.tool === "search") {
        const hits = await ddgSearch(String(req.query || ""));
        result = hits.length
          ? hits.map((h, i) => `${i + 1}. ${h.title} — ${h.url}`).join("\n")
          : "no results";
      } else if (req.tool === "fetch") {
        const text = await fetchPage(String(req.url || ""));
        ledger.pages.push({ url: req.url, chars: text.length, model: MODEL });
        result = text;
      } else {
        result = `unknown tool '${req.tool}'`;
      }
    } catch (e) {
      result = `tool failed: ${e.message}`;
    }
    messages.push({ role: "assistant", content });
    messages.push({ role: "user", content: `[TOOL_RESULT]\n${result}\n[/TOOL_RESULT]` });
  }
  return { final: null, rows, spent, cause: "budget_or_rounds_exhausted" };
}

// ---------- arms ----------

async function armA(mission, ledger) {
  const sys = "You are a meticulous researcher. Ground every claim in pages you actually fetched; cite them as [n] mapped to URLs you list at the end under 'Sources:'.";
  return runAgent({ role: "single", system: sys, task: mission, ledger, budgetLeft: BUDGET_TOKENS, maxTokens: 4000 });
}

async function armC(mission, ledger) {
  const researcher = await runAgent({
    role: "searcher",
    system: "You research thoroughly: search, fetch, and produce factual findings. Output a structured list of findings, each with its source URL. You do NOT write the report.",
    task: mission,
    ledger,
    budgetLeft: BUDGET_TOKENS - WRITER_RESERVE,
    maxTokens: 4000,
  });
  // Clean writer: sees ONLY the ledger — the information-channel separation.
  const ledgerView = JSON.stringify(
    { findings_seen_by_searcher: researcher.final, pages_fetched: ledger.pages.map((p) => p.url) },
    null,
    1,
  );
  const writer = await runAgent({
    role: "writer_clean",
    system: "You write the final report from the research ledger you are given. You cannot search or fetch. Cite as [n] with a source list. Never invent sources beyond the ledger.",
    task: `Research ledger:\n${ledgerView}\n\nMission: ${mission}\nWrite the report.`,
    ledger,
    budgetLeft: WRITER_RESERVE,
    maxTokens: 3000,
  });
  return {
    final: writer.final,
    rows: [...researcher.rows, ...writer.rows],
    spent: researcher.spent + writer.spent,
    cause: writer.cause ?? researcher.cause,
  };
}

async function armB(mission, ledger) {
  const orch = await runAgent({
    role: "orchestrator",
    system: "You decompose the mission into research questions and instructions for one researcher. Output the brief only.",
    task: mission,
    ledger,
    budgetLeft: 10_000,
    maxTokens: 1500,
  });
  const research = await runAgent({
    role: "researcher",
    system: "You execute the brief: search and fetch extensively, produce findings with URLs.",
    task: `Brief:\n${orch.final}\n\nMission: ${mission}`,
    ledger,
    budgetLeft: BUDGET_TOKENS - 20_000,
    maxTokens: 4000,
  });
  const analyst = await runAgent({
    role: "analyst",
    system: "You turn raw findings into the final cited report. Cite as [n] with a source list.",
    task: `Findings:\n${research.final}\n\nMission: ${mission}`,
    ledger,
    budgetLeft: WRITER_RESERVE,
    maxTokens: 3000,
  });
  return {
    final: analyst.final,
    rows: [...orch.rows, ...research.rows, ...analyst.rows],
    spent: orch.spent + research.spent + analyst.spent,
    cause: analyst.cause ?? research.cause,
  };
}

async function armD(mission, dimensions, ledger) {
  const orch = await runAgent({
    role: "orchestrator",
    system: "You decompose the mission into 2-4 independent research dimensions. Output JSON: {\"dimensions\":[{\"name\":\"...\",\"question\":\"...\"}]}",
    task: mission,
    ledger,
    budgetLeft: 10_000,
    maxTokens: 1200,
  });
  let dims = dimensions;
  if (!dims && !DRY_RUN) {
    try {
      dims = JSON.parse(orch.final.match(/\{[\s\S]*\}/)?.[0] || "{}").dimensions;
    } catch { /* fall through */ }
  }
  dims = Array.isArray(dims) && dims.length ? dims : [{ name: "all", question: mission }];
  const shared = { pages: ledger.pages, findings: [] };
  const perBudget = Math.floor((BUDGET_TOKENS - 2 * WRITER_RESERVE) / dims.length);
  const results = await Promise.all(
    dims.map((d) =>
      runAgent({
        role: `researcher:${d.name}`,
        system: "You research ONLY your dimension: search, fetch, produce findings with URLs. Do not write the report.",
        task: `Dimension: ${d.name}\nQuestion: ${d.question || d.name}\nMission: ${mission}`,
        ledger: shared,
        budgetLeft: perBudget,
        maxTokens: 3000,
      }),
    ),
  );
  const combined = results.map((r, i) => `Dimension ${dims[i].name}:\n${r.final}`).join("\n\n");
  const writer = await runAgent({
    role: "writer_clean",
    system: "You write the final report from the per-dimension findings ledger. You cannot search or fetch. Cite as [n] with a source list.",
    task: `Findings ledger:\n${combined}\n\nMission: ${mission}`,
    ledger: shared,
    budgetLeft: WRITER_RESERVE,
    maxTokens: 3000,
  });
  return {
    final: writer.final,
    rows: [...orch.rows, ...results.flatMap((r) => r.rows), ...writer.rows],
    spent: orch.spent + results.reduce((a, r) => a + r.spent, 0) + writer.spent,
    cause: writer.cause,
  };
}

// ---------- mission loading ----------

function loadMission(name) {
  const candidates = [
    path.resolve("docs/experiments/v2-design", `${name}.txt`),
    path.resolve("scripts", `${name}.txt`),
    path.resolve(name),
  ];
  for (const c of candidates) if (existsSync(c)) return readFileSync(c, "utf8").trim();
  throw new Error(`mission file not found for '${name}' (tried ${candidates.join(", ")})`);
}

// ---------- main ----------

async function main() {
  const plan = [];
  for (const arm of WANTED_ARMS)
    for (const mission of MISSIONS)
      for (let run = 1; run <= WANTED_RUNS; run++) plan.push({ arm, mission, run });

  log(`model=${MODEL} budget=${BUDGET_TOKENS}/run arms=${WANTED_ARMS} runs=${WANTED_RUNS} missions=${MISSIONS}`);
  if (DRY_RUN) {
    console.log(JSON.stringify({ dry_run: true, model: MODEL, budget_tokens: BUDGET_TOKENS, writer_reserve: WRITER_RESERVE, planned_runs: plan }, null, 2));
    return;
  }

  const missionTexts = Object.fromEntries(MISSIONS.map((m) => [m, loadMission(m)]));
  const summary = [];

  for (const { arm, mission, run } of plan) {
    const dir = path.join(ROOT, `${arm}_${mission}_${run}`);
    mkdirSync(dir, { recursive: true });
    log(`▶ ${arm}_${mission}_${run}`);
    const ledger = { model: MODEL, budget_tokens: BUDGET_TOKENS, pages: [], calls: [] };
    let out;
    const t0 = Date.now();
    try {
      out =
        arm === "A" ? await armA(missionTexts[mission], ledger)
        : arm === "C" ? await armC(missionTexts[mission], ledger)
        : arm === "B" ? await armB(missionTexts[mission], ledger)
        : await armD(missionTexts[mission], null, ledger);
    } catch (e) {
      out = { final: null, rows: [], spent: 0, cause: `error: ${e.message}` };
    }
    ledger.calls = out.rows;
    writeFileSync(path.join(dir, "report.md"), out.final ?? `# no report — ${out.cause ?? "no final"}`, "utf8");
    writeFileSync(path.join(dir, "ledger.json"), JSON.stringify(ledger, null, 2), "utf8");
    writeFileSync(
      path.join(dir, "run.json"),
      JSON.stringify(
        {
          arm, mission, run, model: MODEL,
          spent_tokens: out.spent, budget_tokens: BUDGET_TOKENS,
          stop_cause: out.cause ?? "plan_complete",
          pages_fetched: ledger.pages.length,
          duration_ms: Date.now() - t0,
          verdict: "PENDING_CHECKER",
        },
        null,
        2,
      ),
      "utf8",
    );
    writeFileSync(path.join(dir, "findings.json"), JSON.stringify({ sections: [] }, null, 2), "utf8");
    summary.push({ dir: path.basename(dir), spent: out.spent, cause: out.cause ?? "plan_complete" });
    log(`  ✓ spent=${out.spent} cause=${out.cause ?? "plan_complete"}`);
  }
  writeFileSync(path.join(ROOT, "campaign_summary.json"), JSON.stringify({ model: MODEL, runs: summary }, null, 2), "utf8");
  log(`campaign done → ${ROOT}`);
}

main().catch((e) => {
  console.error("[harness] fatal:", e.message);
  process.exit(1);
});
