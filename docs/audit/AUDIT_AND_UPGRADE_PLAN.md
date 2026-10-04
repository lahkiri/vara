# Audit and upgrade plan — state of `vara` at v0.6.0

_Written 2026-10-02 by the coding agent **after reading the code**, not from the spec's assumptions. Every "done" below has a file or a test behind it; every "missing" was confirmed by searching the tree. Commands used are named so they can be re-run._

Method: `grep`/`read` across `crates/vara-core/src`, `src-tauri/src`, `src/lib`; the test suites; `gh` for CI and release state. Where I could not confirm something, it says **unverified** rather than guessing.

---

## 1. What is already true (do not rebuild it)

| Capability | Evidence | Status |
|---|---|---|
| Server-side, single-use, time-boxed approvals | `action_proposals` (migration v5), `exec_policy::plan_proposal`/`may_execute`, `sys_approve` + `sys_execute`, CAS `pending→approved→executing`; tests `proposal_lifecycle_is_single_use_and_time_boxed`, `expired_proposals_cannot_be_approved_or_claimed`, `denied_proposals_never_execute` | **T-002 done** |
| argv-only execution, no shell | `exec_policy::tokenize_command` + `CommandPolicy`; denials for shells/system tools; inline-code flags; tests `rejects_shell_composition`, `denies_shells_and_system_tools_however_they_are_spelled` | **done** |
| Path confinement, host-independent | `confine_to_root` textual rewrite; tests incl. `windows_shaped_paths_are_refused_on_any_host` | **done** |
| Secrets out of child processes; key never persisted when from env; webview never receives it | `child_env`, `Settings::without_api_key`/`for_webview`, `get_bootstrap` secret block; test `scrubs_secrets_from_child_environment` | **partial** — see gap A-01 |
| Provenance gate C1/C2/C3 + receipt | `provenance.rs`, `GateReceipt`, migration v6 `reports.receipt_json`; 35 tests | **done** |
| Migrations transactional + self-healing; FTS rebuild | `apply_migration`, `split_sql_statements`, `rebuild_fts_if_stale`; test `open_heals_a_migration_that_was_applied_but_not_recorded` | **done** |
| ActLoop guards (grounding, destructive defusal, screenshot grant) | `computer_use/mod.rs`; 15 harness scenarios | **done** |
| Protocol parity Rust↔TS | shared fixture + `protocol_parity.rs` + vitest (209 assertions) | **done** |
| CI on all four jobs | run `37028137670`: `success × 4` | **done** |
| Release v0.6.0 published with OTA manifest | `gh release view v0.6.0` → `isDraft:false`, 3 assets; `latest.json` HTTP 200 | **done** |

## 2. What the spec assumes exists and does not

| ID | Gap | Evidence | Blocks |
|---|---|---|---|
| **A-01** | API key still lives in `settings.json` when it came from the file; no OS credential store | `settings.rs` `save/load`; no `keyring` dependency in any manifest | spec L-06, Sprint 0 |
| **A-02** | No audit hash chain | `db.rs` `events` table has `(ts, level, kind, message)` and an index; no `prev_hash`/`hmac` column anywhere | spec T-201, U-08, `verify_audit_chain()` |
| **A-03** | Undo journal absent | no `journal`/`undo` symbol in the tree; `move`/`delete` tools do not exist yet | spec S-03/S-04 |
| **A-04** | No taint tracking (untrusted content in context) | nothing carries provenance-of-value into an action decision; `provenance.rs` grades reports, not decisions | spec §7.2, §8 |
| **A-05** | Heartbeat always writes a reflection note | `entity.rs::heartbeat` → `add_note_if_new("reflection", …)` unconditionally; seen in owner screenshots (G-01) | spec S-05 |
| **A-06** | No native tool loop | `entity.rs` executes a fixed `match step.kind` over `search`/`fetch`/`report`; chat has no tools at all (`chat.rs::build_context` is text-only) | spec S-01/S-06, G-02, G-09 |
| **A-07** | No reader quarantine | `tools.rs::fetch_page` returns page text straight to the writer prompt | spec S-10, L-04 |
| **A-08** | No workspace files (SOUL/IDENTITY/USER/HEARTBEAT/MEMORY, daily logs) | absent; memory is SQLite notes only | spec S-07 |
| **A-09** | No routines/scheduler | no cron or scheduler crate; heartbeat is a fixed interval in the shell | spec S-08 |
| **A-10** | No skills loading from the user workspace | `skills/vara/*` ship as read-only resources via `list_skills`; nothing loads a *user* skill or scans it | spec S-11 |
| **A-11** | No Today/companion/approval-inbox surfaces | `src/lib/components` = Activity, Avatar, ChatView, MemoryView, ProvenanceBadge, ReportsView, SettingsView, Sidebar | spec U-01…U-06 |
| **A-12** | No notification policy (cap, quiet hours, actions) | shell notifies ad hoc (`notify_user`) | spec U-04 |
| **A-13** | Settings is a flat checkbox list | `SettingsView.svelte` | spec U-07, G-04 |
| **A-14** | Reports UI does not render the stored receipt | `ReportsView.svelte` reads `check_json` only; `receipt_json` is written (migration v6) and never shown | quick win |
| **A-15** | Suggestion rows read as fake conversations; untranslated "Reflection" | owner screenshots (G-01/G-07) | spec §6.1 |
| **A-16** | No listening socket **is already true** — and must stay true | no TCP/WebSocket server in `src-tauri`; Tauri IPC only | L-01 already satisfied; add a regression test |

## 3. P0 — the order I will actually work in

Ordering rule: **nothing above is "polish"; each P0 item removes a way the product can lie or stall.** The spec's Sprint 0 (security first) is honoured, then the single missing core loop, then presence.

| # | Task | Why first | Acceptance |
|---|---|---|---|
| **T-001** | Heartbeat v2: silent by default, checklist input, **2k/300 token ceiling**, proposals only, de-dup, reflection class removed + existing reflection notes collapsed into a daily log with an undo backup | It is the visible "dead/creepy" symptom and the cheapest real fix | soak: 20 ticks on an unchanged workspace → 0 notes, 0 notifications, 20 counter rows; new file → exactly 1 proposal |
| **T-002** | Audit hash chain over `events` (append-only `prev_hash`), `verify_audit_chain()`, "Verify" surfaced in Activity | Accountability is a product promise; without it the Activity page is prose | tamper test: edit one row → verification fails and names the row |
| **T-003** | Native tool registry + read-only tool set + tool loop (S-01/S-06 Phase 1) | G-02/G-09: the app cannot answer "explain my system" or "what do you know about me" | the 10-task suite's local subset ≥ 4/4 by tool, no policy refusal, no mission |
| **T-004** | Tool-recovery discipline (one structured retry, then `Confused`) | turns a refusal into a conversation instead of a dead end | refused call → exactly one rewritten attempt; second failure → `ask_user`, avatar Confused |
| **T-005** | Secrets into the OS credential store (`keyring`) + migration off `settings.json` + `has_key`-only UI | closes spec L-06 and stops the key living in a plaintext file | key written/read via credential store; file contains no key; tests for migration and for the no-key case |
| **T-006** | Reader quarantine (S-10) | untrusted page text currently reaches the writer prompt directly | injection fixture page cannot produce an action; actor prompt contains structured facts only |
| **T-007** | Taint plumbing (A-04) | makes "web content in context" chip and the taint rule possible at all | a decision made with untrusted content in context is refused above class R without approval |
| **T-008** | Workspace files + integrity hashes + diff proposals (S-07/L-03) | identity/memory must be visible, editable, and un-poisonable | agent edit → diff proposal, never a write; hash mismatch is reported, not auto-accepted |

Then the spec's Sprint 1 remainder (S-03 undo journal + policy classes, S-04 approval scopes) and Sprint 2 (U-01 Today, U-02 companion). **Undo journal (S-03) is deliberately after T-003**: undo only means something once write tools exist.

## 4. Quick wins to take while doing the above

* **A-14** — render `receipt_json` in `ReportsView` (gate version, backed ratio + Wilson interval, claim counts, not-evaluable reason). Data is already stored; it is a display change with a real trust payoff.
* **A-16** — add the regression test that no listening socket exists, so the property cannot regress silently.
* **A-15** — translate or remove the "Reflection" label; move suggestions into empty states only.
* Frontend: `lastMissionProposal()` is called four times per row in `ChatView.svelte` (quadratic on long threads) — compute once.

## 5. Explicitly out of scope (v1)

Public skill marketplace; any remote skill install; a messaging/channel bridge; mobile; multi-user; cloud sync. The spec's "not to copy" list is adopted verbatim.

## 6. Honest notes

* `web_search` failed during this audit (HTTP 401 from the configured endpoint), so external research relied on URLs already known and `web_fetch` only. Recorded in `QUESTIONS.md`.
* I have **not** run the app visually in this session; every UI claim above comes from reading components, and owner screenshots are cited as such.
* The 10-task suite in spec §1 is not yet automated; T-003 introduces the local half of it as tests.
