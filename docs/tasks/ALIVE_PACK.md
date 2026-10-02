# ALIVE_PACK — the enabling contracts for ALIVE_V2_SPEC

_Status: written 2026-10-02 by the coding agent, derived from `ALIVE_V2_SPEC.md` §3/§5/§7 and from primary-source research (see §6). Where this pack and the spec disagree, the **security invariants win** (AGENTS.md 1–7 plus §3 of the spec)._

This file exists because `ALIVE_V2_SPEC.md` referenced a pack that was never written. It contains the pieces the spec assumes but does not define: the tool contract (A), the pure-vs-effectful split (B), the budget and silence rules (C), the health/goal checks (D), and the local-first consequences (E). Read it before starting any Sprint 0 task.

---

## A. Tool contract (typed, schema-validated, policy-routed)

Every capability Vara has on this machine is a **tool**: a named function with a JSON schema, a risk class, an output cap, a timeout, and a policy route. There is no other way to touch the machine. The chat loop, missions, routines and the heartbeat all call the same registry — that is what "one native tool-calling loop" means.

```rust
pub struct ToolSpec {
    pub name: &'static str,            // snake_case, stable, what the model sees
    pub description: &'static str,     // one sentence: what it does, not how
    pub params: serde_json::Value,     // JSON Schema (object root, additionalProperties:false)
    pub class: RiskClass,              // R / Wr / Wd / X / N / H  (spec §7.2)
    pub roots: RootPolicy,             // Allowed(Vec<PathBuf>) | None | WatchedFolder
    pub max_output_bytes: usize,       // hard cap; the tool truncates, never the caller
    pub timeout_ms: u64,
    pub examples: &'static [&'static str], // argv-style example calls for the model
    pub redact: bool,                  // scrub paths/secrets in the receipt
}

pub trait Tool {
    fn spec(&self) -> &'static ToolSpec;
    /// Pure validation. MUST NOT touch the filesystem, network or clock beyond
    /// what `ctx.now_unix` already carries. Returns a structurally typed error
    /// so the model can repair its own call instead of guessing.
    fn validate(&self, args: &serde_json::Value, ctx: &ToolCtx) -> Result<(), ToolError>;
    /// The effectful half. Only ever called by the policy middleware.
    fn run(&self, args: &serde_json::Value, ctx: &ToolCtx) -> ToolResult;
}
```

**Rules that are not negotiable**

1. **Validate-then-execute**, per call and per batch. An unknown tool name, an unknown field, a wrong type or a missing required field is refused *before* anything runs (same discipline as the computer-use op language).
2. **Errors are for the model.** A refusal is a typed value (`ToolError::Denied{class, rule}`, `ToolError::BadArgs{field, expected}`, `ToolError::Failed{why}`) rendered into the transcript as a short, actionable sentence. "Something went wrong" is a bug in the tool, not a message to the model.
3. **Output is capped and summarised**, never dumped: long listings return a count plus the first N entries plus how to narrow. (SWE-agent's measured lesson: summarised observation beats raw output by a wide margin.)
4. **No tool may** read or write: credential stores, browser password databases, SSH/API key files, Vara's own settings, policy, updater records, or the workspace integrity hashes. That list is a **hard-deny floor** enforced in Rust, not a setting.
5. **Every tool call is journaled** with `(tool, args_digest, risk_class, decision, duration, outcome, receipt_id)` — the Activity lineage tree in spec §6.6 is rendered from these rows, not from prose.
6. **The registry is data, not code paths.** Adding a tool must not add a branch to the agent loop.

**The Phase-1 registry** (read-only first — this is also the honest fix for G-02/G-09):

| Tool | Class | Notes |
|---|---|---|
| `system_info` | R | OS, CPU, RAM, disk totals. The thing that failed in G-02. |
| `disk_usage` | R | per-volume free/total; optionally a path's size |
| `list_dir` | R | one directory level, capped, sorted by the caller's key |
| `find_files` | R | glob under allowed roots, capped results, cap depth |
| `read_file` | R | allowed roots only, byte cap, binary detection, secret-path deny |
| `search_memory` | R | FTS over notes + daily logs |
| `recent_memory` | R | last N durable facts, with timestamps |
| `web_search` | N | existing DuckDuckGo path; results are **untrusted** |
| `web_fetch` | N | → reader quarantine (§8 of the spec); actor never sees raw text |

Phase-2 adds `Wr` (`create_file`, `move_path`, `copy_path`, `trash_path`, `write_note`), `X` (`run_program(argv)`) and the computer-use op. Writing the registry first means those land as **data**, not as new loop branches.

---

## B. Pure core vs effectful shell

The core (`crates/vara-core`) owns: tool schemas, policy decisions, provenance, budgets, the journal model, memory ranking, plan normalisation. The shell (`src-tauri`) owns: spawning processes, reading/writing real files, screenshots, windows, tray, notifications, IPC.

Consequence for tests: **every safety rule must be provable without a machine.** `MockToolHost` (mirroring `MockComputerUse`) is the deterministic host: an in-memory filesystem, scripted failures, a clock the test advances. If a rule can only be tested by running the real app, it is in the wrong layer.

---

## C. Budget, silence and cadence (the G-01 fix, with numbers)

The OpenClaw heartbeat is the cautionary tale: a checklist-style wake that was nonetheless **sending ~120,000 tokens of context per check at ~$0.75 per check** (~$750/month for a reminder) *[source: Adversa 2026-02, §"Operational risks"]*. The spec asks for "silent by default" but sets no ceiling; these are the ceilings.

| Rule | Value | Why |
|---|---|---|
| Heartbeat cadence | 30 min default, user-editable | spec §7.3 |
| **Per-tick token ceiling** | **2,000 tokens in, 300 out**, hard-stopped | ~40× cheaper than the observed OpenClaw tick; a tick that cannot fit is skipped, not truncated |
| Context given to a tick | `HEARTBEAT.md` (≤ 50 lines) + a **state snapshot**, never the note corpus | the corpus is what made the OpenClaw tick huge |
| State snapshot | new files in watched folders · due/failed routines · pending approvals · failed missions · disk/RAM alarms · next scheduled run | bounded: counts + ≤ 5 examples per bucket |
| Output | exactly one of `silent` / `propose` / `notify`, as JSON, repaired once on parse failure | spec §7.3 |
| `silent` | writes **one** counter row and nothing else — no note, no log line, no `activity()` event | kills the reflection spam at the source |
| `notify` budget | ≤ 2/day, urgent bypasses, quiet hours respected | spec §7.3 |
| Proposal dedup | cosine/Jaccard ≥ 0.72 against the last 20 proposals and notes (reuse `dedup.rs`), else dropped | spec §7.3 |
| Daily consolidation | once per day, proposals only, facts carry provenance labels; `web_*` claims never auto-promote | spec §7.3/§7.6 |
| Reflection notes | **removed as a class.** `heartbeat` may no longer create `reflection` notes at all | G-01: the entity's memory must not be its own diary |

**Acceptance (soak, per spec §7.3):** on an unchanged workspace, 20 ticks produce **0 notes, 0 notifications, 20 counter rows**. A new file in a watched folder produces exactly **1 proposal**. A repeated one produces none.

---

## D. Health checks: does the work answer the goal?

Two checks that the current mission runner lacks (G-03):

1. **Goal check** — a report is scored on *did it answer the goal*, not only *was it documented*. Rendered as the spec's badge pair: `Documented ✔ · Goal answered ✘` with the failing dimension named. Local-goal routing (spec §7.7): goals naming this machine/app route to local tools first, and a web-only report for a local question is an automatic ✘.
2. **Tool-recovery check** — a refused or failed tool must produce **one** structured recovery attempt (repair the arguments, pick a legal alternative, or `ask_user`). Two failures in a row on the same tool turn the avatar **Confused** and hand the decision to the owner. Silent failure is not an option: G-02's `systeminfo | findstr` proposal was refused and the conversation simply stopped.

---

## E. Local-first consequences

* No required cloud service: everything above works with a local model and no network; network tools are marked `N` and degrade to "not available" without breaking the loop.
* **Provider disclosure is part of the contract, not a footnote.** If the configured endpoint is not `localhost`/`127.0.0.1`/a LAN address, the onboarding step and Settings both state plainly that prompts, memory excerpts and (if enabled) screenshots leave the machine. The current default endpoint is a **third-party proxy**, so this disclosure ships enabled, not optional.
* No font/CDN requests (strict CSP; one bundled OFL Arabic family).
* No listening socket (spec L-01); IPC only.
* Model profiles (spec §7.8) are data: capability flags + context size + whether the endpoint is remote.

---

## F. Sources actually read for this pack

Primary/vendor pages fetched during this session (secondary press and blog material is *not* treated as fact):

* Adversa AI, "OpenClaw security 101" (CVE-2026-25253 kill chain, ClawHavoc 341/2,857 skills, 21,639 exposed instances, heartbeat cost) — https://adversa.ai/blog/openclaw-security-101-vulnerabilities-hardening-2026/
* xAI Grok Bot docs index (approvals/security/privacy, skills & routines) — https://docs.x.ai/grok-bot/approvals-security-and-privacy

**Not verified in this session** (named in the spec, referenced here only as pointers, to be re-checked before any of them influences a decision): the OpenAI Dots and Meta Muse pages, and the DeepFirst/Wiz/Koi primary advisories behind the incidents above. `web_search` was unavailable during this session (HTTP 401 from the configured search endpoint), so discovery was limited to URLs already known — a known limitation, recorded in `QUESTIONS.md`.
