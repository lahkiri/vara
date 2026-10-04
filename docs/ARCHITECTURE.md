# Architecture

Vara is a **composable entity**: a plugin host that composes capabilities, and
surfaces (desktop, terminal, headless) that are themselves plugins.

## Workspace layout

```
crates/vara-core/    the host and the entity — no UI framework, fully unit-tested
crates/vara-tui/     the terminal surface: one file, the same core, no window
crates/vara-plugins/ the composition CLI: list / plan / check / enable / approve / hash
src-tauri/           the desktop surface (Tauri 2) — tray, window, commands
src/                 Svelte 5 frontend (Vite + Tailwind 4)
plugins/             the fifteen plugins Vara ships, each a real folder + manifest
skills/vara/         the entity's own rules, bundled as resources (read-only)
tests/               shared fixtures consumed by BOTH parsers (Rust + webview)
```

The split is deliberate: everything that can be wrong in a *logic* sense
(provenance rules, action policy, plugin loading, budget accounting) compiles and
tests headlessly via `cargo test -p vara-core`. A surface is kept thin, because a
surface is replaceable.

## Everything is a plugin

The rule is literal, and it is enforced by types rather than by convention:

| Layer | Module | What it owns |
|---|---|---|
| **Host** | `host.rs` | Loading and unloading plugins, the event bus, the gate, the log. **Nothing else.** It does not know what a tool, a model or a memory *is*. |
| **Seams** | `seams.rs` | The product's capability names (`tools.read`, `tools.host`, `tools.ctx`, `brain`, `log`) and the plugins that fill them. |
| **Contracts** | `plugin.rs` | The manifest: twelve slots, deny-by-default permissions, a SHA-256 over the claims, deterministic dependency order, real-folder discovery. |
| **Decisions** | `plugin_registry.rs` | What the owner decided: enabled/disabled, and approval bound to the claims that were shown. |
| **Composition** | `profile.rs` | Profiles and bundles: the same host runs a desktop app, a TUI or a headless daemon by composing a different set. |

Three properties make composability real rather than aspirational:

1. **Registrations are reversible effects.** `Plugin::start` returns `Vec<Effect>`;
   unloading runs them in reverse order, so removing a plugin removes exactly
   what it added. A plugin that fails to start is recorded as unhealthy with its
   reason and **does not take the host down**.
2. **A permission that is not declared does not exist.** `write_files`,
   `run_programs`, `network`, `write_memory` and `notify` are opt-in per plugin,
   they surface in the UI *before* the switch, and enabling without approval is
   refused by the core — not by the button.
3. **Nothing built-in is privileged.** The six read tools, the model adapter and
   the interface all arrive through the same seam mechanism a third-party plugin
   would use. `vara-plugins` proves it by loading a composition from disk with no
   binary involvement.

A tampered manifest is detected, not trusted: editing a plugin's claims without
recomputing its hash makes it `BROKEN`, and it will not load.

## vara-core modules

| module | responsibility |
|---|---|
| `host.rs` | the plugin host: load/unload with reversible effects, event bus, gate, log. No product knowledge |
| `plugin.rs` | the manifest contract: twelve slots, permissions, SHA-256 over claims, content hash over the folder, dependency ordering, discovery |
| `plugin_registry.rs` | installed plugins + the owner's decisions (enabled, approved-bound-to-claims), load planning, atomic state file |
| `seams.rs` | the capability seams and the shipped plugins that fill them (`ReadToolsPlugin`, `RealHostPlugin`, `ToolCtxPlugin`, `BrainPlugin`, `LogPlugin`) |
| `profile.rs` | profiles and bundles: `desktop`, `tui`, `headless`, `minimal` over one catalog |
| `goals.rs` | outcomes with a **measured** stopping condition, budgets, step ceilings, stall detection, and a default decision of silence |
| `heartbeat.rs` | the silent-by-default initiative engine (2000 in / 300 out ceiling, dedup ≥ 0.72, notify cap 2/day) |
| `tools_registry.rs` | the tool contract: `ToolSpec`, risk classes (`R`/`Wr`/`Wd`/`X`/`N`/`H`), the hard-deny floor, a typed error vocabulary |
| `tools_local.rs` | the shipped host implementations (`FsToolHost`, `MockToolHost`) and the six read tools |
| `tool_loop.rs` | the router: `none` / `call` / `propose` — anything above read-only becomes a proposal even if the model says "call" |
| `entity.rs` | entity state machine (`Dormant → Attentive → Deliberating → Working → Reporting`), mission runner, budget enforcement, live replanning, clean-context writer, gate-driven repair |
| `provenance.rs` | the gate: C1 (every cited link ∈ retrieved set) + C2 (every `[n]` resolves) + C3 (every quoted span appears verbatim), per-section coverage, Wilson intervals, a `GateReceipt` stored with the report |
| `exec_policy.rs` | the trust boundary for OS actions: argv-only tokenization, deny lists, path confinement, environment scrubbing, proposal minting, digests, TTL |
| `db.rs` | SQLite with WAL; append-only migrations inside a transaction; FTS5 with triggers *and* a rebuild when the index drifts |
| `tools.rs` | DuckDuckGo HTML search (lite fallback), page fetch with content-type guard, HTML→text |
| `llm.rs` | OpenAI-compatible chat client (reqwest + rustls), streaming with `include_usage` so budgets are measurable |
| `computer_use/` | the ActLoop: validated see→act→confirm over a pluggable adapter, grant ladder L0–L3, structural grounding |
| `dedup.rs` | Jaccard ≥ 0.72 near-duplicate detection over alphanumeric tokens (Arabic included) |
| `types.rs` | wire types shared core ↔ shell ↔ frontend |

## The plugin boundary

```
plugins/<name>/plugin.toml          twelve slots · permissions · sha256
        │
        ▼  vara_core::plugin::discover
  manifest validated (id shape, slots, self-require)
  hash verified      → verified | unverified | BROKEN
        │
        ▼  vara_core::plugin_registry
  owner's decision   → enabled? approved? (approval bound to the claims hash)
  load_plan()        → dependency order, or a named refusal
        │
        ▼  vara_core::host
  Plugin::start(&ctx) → Vec<Effect>          (registrations are effects)
  ctx.register(seam, value)                  (a named service)
  ctx.authorize(class, action, target, taint) (the gate — every call, logged)
        │
        ▼  unload
  effects unwind in reverse order
```

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
- **Dev WebView2 isolation**: both the installed app and a development build
  carry the bundle identifier `app.vara.entity`, and WebView2 permits one process
  per user-data folder. A debug build therefore points
  `WEBVIEW2_USER_DATA_FOLDER` at its own directory before Tauri starts — which is
  both a fix for `0x800700AA` and the correct behaviour, since a development
  instance must never touch the owner's real conversations.


## Surfaces are plugins

```
                    vara_core  (the entity: host + plugins + seams + gates)
                         │
     ┌───────────────────┼───────────────────┬──────────────────┐
     │                   │                   │                  │
 vara-desktop        vara-tui          vara-plugins        (a future
 (src-tauri)         (crates/          (crates/             daemon/web)
                     vara-tui)         vara-plugins)
     │                   │                   │
  tray, window,      one file, no        list / plan /      the same
  approvals UI       window, same core   check / approve    core again
```

`crates/vara-tui` is the proof: it is **one file of application code**, and every
capability it has (memory, model, tool routing, execution) comes from
`vara_core`. It runs the entity end to end on a machine with no GPU and no
WebView2 — which is exactly the "VPS with no graphics" case the product promises.

## Frontend ↔ core contract

- Commands (`src-tauri/src/commands.rs`): `get_bootstrap`, `save_settings`,
  `test_provider`, `create_and_start_mission`, `start_mission_in_conversation`,
  `pause_entity`, `cancel_mission`, `get_entity_status`, `list_missions`,
  `get_mission_detail`, `list_reports`, `get_report`, `export_report`,
  `list_notes`, `add_manual_note`, `delete_note`, `list_events`,
  `list_action_proposals`, `sys_approve`, `sys_execute`, `sys_open`,
  `list_skills`, `show_window`, conversation commands, chat streaming, updater.
- Plugin commands (`src-tauri/src/plugin_bridge.rs`): `list_plugins`,
  `set_plugin_enabled`, `approve_plugin`, `reveal_plugin`, `open_user_plugin_dir`.
  Thin on purpose — **every policy decision lives in the core**, so the app and
  the CLI cannot drift apart on "may this be enabled".
- Events: `entity://event` (tagged: `state | activity | mission_update |
  report_ready`), `entity://chat/*`.
- State is a single runes store (`src/lib/state.svelte.ts`); persona style
  switches a CSS custom property on `<html data-persona>`. The current view is
  addressable via the URL hash (`#plugins`, `#settings`, …), which is what makes
  every screen reachable by a screenshot script or a test.
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
