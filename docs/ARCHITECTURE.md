# Architecture

Vara is a two-layer system: a pure-logic core and a thin desktop shell.

## Workspace layout

```
crates/vara-core/   pure logic — no UI framework, fully unit-tested
src-tauri/          Tauri 2 shell — tray, window, notifications, commands
src/                Svelte 5 frontend (Vite + Tailwind 4)
```

The split is deliberate: everything that can be wrong in a *logic* sense
(provenance rules, dedup, budget accounting, plan normalization) compiles and
tests headlessly on any platform via `cargo test -p vara-core`. The shell is
kept thin so the riskiest platform surface stays small.

## vara-core modules

| module | responsibility |
|---|---|
| `entity.rs` | entity state machine (`Dormant → Attentive → Deliberating → Working → Reporting`), mission runner, budget enforcement, live replanning, clean-context writer, repair loop, idle heartbeat |
| `provenance.rs` | the GM-1 structural gate: C1 (every cited link ∈ retrieved set) + C2 (every `[n]` resolves to a numbered source entry) + per-section effective coverage. Ported 1:1 from the v1 harness `check_provenance.mjs` |
| `db.rs` | SQLite with WAL journal; migrations; FTS5 virtual table with triggers when available, `LIKE` fallback when not; tables: notes / missions / sources / actions / reports / events |
| `tools.rs` | DuckDuckGo HTML search (lite fallback), page fetch with content-type guard, HTML→text |
| `llm.rs` | OpenAI-compatible chat client (reqwest + rustls). Works against Z.ai, OpenAI, Ollama, LM Studio, llama-server, vLLM |
| `dedup.rs` | Jaccard ≥ 0.72 near-duplicate detection over alphanumeric tokens (Arabic included) |
| `types.rs` | wire types shared core ↔ shell ↔ frontend |

## Mission lifecycle

```
goal
 │  1. Deliberating: planner LLM → JSON plan (dimensions + steps)
 │     · schema-validated, 2 retries, deterministic fallback plan
 ▼
 2. Working: execute steps
 │     search → DDG hits → dedup(Jaccard + URL) → notes + sources
 │     fetch  → page → text (4500 chars) → notes + sources
 │     every 4 steps: live replan (≤4 steps, may end with "report")
 │     pause/cancel honored between steps; budget binding throughout
 ▼
 3. Reporting: clean-context writer
 │     sees ONLY the retrieval ledger → report with [n] citations
 ▼
 4. Checker: provenance::check_provenance(report, retrieved_urls)
 │     FAIL → one repair pass → re-check → both versions stored
 ▼
 5. ReportReady event → notification → UI badge (PASS/FAIL + backed %)
```

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

## Frontend ↔ core contract

- Commands (`src-tauri/src/commands.rs`): `get_bootstrap`, `save_settings`,
  `test_provider`, `create_and_start_mission`, `pause_entity`,
  `cancel_mission`, `get_entity_status`, `list_missions`, `get_mission_detail`,
  `list_reports`, `get_report`, `export_report`, `list_notes`,
  `add_manual_note`, `delete_note`, `list_events`, `sys_open`, `show_window`.
- Events: `entity://event` (tagged: `state | activity | mission_update |
  report_ready`) and `entity://done`.
- State is a single runes store (`src/lib/state.svelte.ts`); persona style
  switches a CSS custom property on `<html data-persona>`.

## Why the provenance gate is a hard gate

The v1 experiment (see `docs/experiments/v1/`) demonstrated that a
single-agent pipeline can produce a *formally consistent* report whose
sources are mostly fabricated (9/13 never retrieved). Prompt-level instructions
did not prevent it. The only reliable control found was structural and
external to the writer: compare citations against the retrieval ledger. That
control is cheap (regex + sets), deterministic, and now unremovable: reports
are stored with their check JSON, and the UI displays the verdict beside the
ratio. The regression test
(`regression_v1_single_agent_backed_ratio_308`) asserts the exact 0.308 — if
it drifts, the gate changed.
