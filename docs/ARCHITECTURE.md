# Architecture

Vara is a two-layer system: a pure-logic core and a thin desktop shell.

## Workspace layout

```
crates/vara-core/   pure logic — no UI framework, fully unit-tested
src-tauri/          Tauri 2 shell — tray, window, notifications, commands
src/                Svelte 5 frontend (Vite + Tailwind 4)
skills/vara/        the entity's own rules, bundled as resources (read-only)
tests/              shared fixtures consumed by BOTH parsers (Rust + webview)
```

The split is deliberate: everything that can be wrong in a *logic* sense
(provenance rules, action policy, dedup, budget accounting, plan normalization)
compiles and tests headlessly on any platform via `cargo test -p vara-core`. The
shell is kept thin so the riskiest platform surface stays small.

## vara-core modules

| module | responsibility |
|---|---|
| `entity.rs` | entity state machine (`Dormant → Attentive → Deliberating → Working → Reporting`), mission runner, budget enforcement, live replanning, clean-context writer, gate-driven repair, idle heartbeat |
| `provenance.rs` | the gate: C1 (every cited link ∈ retrieved set) + C2 (every `[n]` resolves) + C3 (every quoted span appears verbatim in the retrieved text of a cited URL), per-section coverage, Wilson intervals and a `GateReceipt` that is stored with the report |
| `exec_policy.rs` | the trust boundary for OS actions: argv-only tokenization, deny lists, path confinement, environment scrubbing, proposal minting (`plan_proposal`), digests, TTL, and the `may_execute` gate |
| `db.rs` | SQLite with WAL journal; append-only migrations applied per statement inside a transaction; FTS5 virtual table with triggers *and* a rebuild when the index drifts, `LIKE` fallback when FTS5 is unavailable; tables: notes / missions / sources / actions / reports / events / conversations / messages / cu_journal / action_proposals |
| `tools.rs` | DuckDuckGo HTML search (lite fallback), page fetch with content-type guard, HTML→text |
| `llm.rs` | OpenAI-compatible chat client (reqwest + rustls), streaming with `stream_options.include_usage` so mission budgets are actually measurable. Works against Z.ai, OpenAI, Ollama, LM Studio, llama-server, vLLM |
| `computer_use/` | the ActLoop: validated see→act→confirm sequences over a pluggable `ComputerUseAdapter` (mock in tests, MCP sidecar on Windows), grant ladder L0–L3, structural grounding and destructive-op guards |
| `dedup.rs` | Jaccard ≥ 0.72 near-duplicate detection over alphanumeric tokens (Arabic included) |
| `types.rs` | wire types shared core ↔ shell ↔ frontend |

## Mission lifecycle

```
goal
 │  1. Deliberating: planner LLM → JSON plan (dimensions + steps)
 │     · schema-validated, 2 retries, deterministic fallback plan
 ▼
 2. Working: execute steps, spending only down to the report reserve
 │     search → DDG hits → dedup(Jaccard + URL) → notes + sources (snippet scope)
 │     fetch  → page → text (4500 chars) → notes + sources (retrieved scope)
 │     every 4 steps: live replan (≤4 steps, may end with "report")
 │     pause/cancel honored between steps; budget binding throughout
 ▼
 3. Reporting: clean-context writer, budget-capped by what is left
 │     sees ONLY the retrieval ledger → report with [n] citations
 ▼
 4. Checker: provenance::check_provenance_full(report, retrieved, snapshots)
 │     C1/C2 as always + C3 quote grounding; FAIL → one repair pass whose
 │     brief is the gate's own failure list → re-check → receipt stored
 ▼
 5. ReportReady event → notification → UI badge (PASS/FAIL + backed % + receipt)
```

The **report reserve** (`REPORT_BUDGET_TOKENS`) is what stops the worst failure
mode: retrieval spending the entire budget and leaving nothing to write the
report with. Retrieval halts at `budget - reserve`, the writer's `max_tokens`
is derived from what remains, and the arithmetic is journaled.

## The action boundary

```
model reply ──[[sys]]──▶ chat.rs parses (tolerant)          [proposal text]
        │
        ▼  shell
mint_action_proposals → action_proposals row (digest, risk, expires_at, pending)
        │
        ▼  UI renders card (id + label + risk)         ← the webview holds no payload
sys_approve(proposal_id)  → pending → approved          (CAS, single-use)
sys_execute(proposal_id)  → approved → executing → executed | failed
        │                    · re-checks the digest
        │                    · reads the target FROM THE ROW
        ▼
argv-only `run` (no shell, denied programs, confined paths, scrubbed env)
 / open_url (http/https only) / open_path (confined) / capture / ActLoop
```

Every executed row leaves a JSON receipt in the conversation, so the thread
stays the audit trail.

## Shell responsibilities (`src-tauri`)

- **Tray**: show/hide, pause/resume, open data folder, quit. Left-click toggles.
- **Close-to-tray**: `CloseRequested` is intercepted; the first hide fires an
  explanatory notification.
- **Single instance**: second launch focuses the existing window.
- **Autostart** via `tauri-plugin-autostart`; **notifications** via
  `tauri-plugin-notification`.
- **Watched folder** (`notify` crate): new/modified `.md`/`.txt` ≤ 512 KB are
  indexed as `file_observation` notes with `file://` provenance (3s debounce).
- **Heartbeat loop**: if enabled, reflects on recent memory every N minutes,
  never while a mission is running.
- **Skills**: `skills/vara/*/SKILL.md` ship as bundle resources and are exposed
  read-only through `list_skills` (Memory ▸ "Vara's own rules").

## Frontend ↔ core contract

- Commands (`src-tauri/src/commands.rs`): `get_bootstrap`, `save_settings`,
  `test_provider`, `create_and_start_mission`, `start_mission_in_conversation`,
  `pause_entity`, `cancel_mission`, `get_entity_status`, `list_missions`,
  `get_mission_detail`, `list_reports`, `get_report`, `export_report`,
  `list_notes`, `add_manual_note`, `delete_note`, `list_events`,
  `list_action_proposals`, `sys_approve`, `sys_execute`, `sys_open`,
  `list_skills`, `show_window`, conversation commands, chat streaming, updater.
- Events: `entity://event` (tagged: `state | activity | mission_update |
  report_ready`), `entity://chat/*`.
- State is a single runes store (`src/lib/state.svelte.ts`); persona style
  switches a CSS custom property on `<html data-persona>`.
- Protocol parsing has exactly one source of truth per side —
  `crates/vara-core/src/chat.rs` and `src/lib/protocol.ts` — locked together by
  `tests/fixtures/protocol_cases.json`, asserted by both
  `crates/vara-core/tests/protocol_parity.rs` and `tests/protocol.test.ts`.

## Why the provenance gate is a hard gate

The v1 experiment (see `docs/experiments/v1/`) demonstrated that a
single-agent pipeline can produce a *formally consistent* report whose sources
are mostly fabricated (9/13 never retrieved). Prompt-level instructions did not
prevent it. The only reliable control found was structural and external to the
writer: compare citations against the retrieval ledger. That control is cheap
(regex + sets), deterministic, and now unremovable: reports are stored with
their check JSON *and* a `GateReceipt`, and the UI displays the verdict beside
the ratio. The regression test
(`regression_v1_single_agent_backed_ratio_308`) asserts the exact 0.308 — if it
drifts, the gate changed.
