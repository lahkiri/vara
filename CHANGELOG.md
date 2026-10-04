# Changelog

## v0.7.0 — Everything is a plugin (unreleased)

### The composition model
- **A plugin host with no privileged core** (`host.rs`): plugins contribute
  services and typed events, and every registration is a **reversible effect**
  that unwinds on unload. A plugin that fails to start is recorded unhealthy with
  its reason and does not take the host down.
- **Capability seams** (`seams.rs`, `host.rs`): `register` / `service` / `has_plugin`.
  Registering twice under one name is refused; unloading a provider closes the
  seam; a consumer loaded afterwards **degrades explicitly** instead of silently
  defaulting. The machine travels over `tools.host` as `Arc<dyn ToolHost>`, so the
  mock and the real filesystem are genuinely interchangeable.
- **Profiles and bundles** (`profile.rs`): `desktop`, `tui`, `headless` and
  `minimal` compose the *same* core and differ only in the interface row — the
  same entity on a laptop, over SSH, or on a server with no graphics.
- **Twelve slots**: tool, toolset, brain, memory, interface, theme, persona,
  channel, goal_engine, subagent, mcp, skill.

### The manifest contract (`plugin.rs`)
- Declared, deny-by-default **permissions** (`read_files`, `write_files`,
  `run_programs`, `network`, `read_memory`, `write_memory`, `notify`) with
  `dangerous()` returning what must appear in an approval card.
- **SHA-256 over the claims** (canonical: id, version, sorted slots, sorted
  requires, permissions, sorted files) plus a **content hash over the folder**,
  because a hash that can be forged by omitting a file is not an integrity check.
  A wrong hash is fatal; a missing one is labelled *unverified*, never *safe*.
- **Deterministic, cycle-safe dependency ordering** that names the participants
  of a cycle instead of breaking it arbitrarily.

### Decisions and visibility (`plugin_registry.rs`, `plugin_bridge.rs`, `vara-plugins`)
- The owner's decisions live apart from the plugin's claims:
  **approval is bound to the claims hash**, so a plugin that later adds
  `run_programs` is un-approved again. Enabling without approval is refused by
  the core, and the refusal travels to the UI verbatim.
- **Fifteen plugins ship with the product**, each a real folder:
  read tools and write tools, browser use, computer use, MCP client, skills,
  subagents, Agent Swarm Team, goals, themes, personas, channels, interfaces,
  model adapters, memory backends. Everything dangerous ships **present but off**.
- **`vara-plugins`** makes the composition inspectable without the GUI:
  `list`, `plan`, `check` (non-zero on a bad plan, for CI), `enable`, `disable`,
  `approve`, `hash`.
- **A plugin panel in the app** grouped by slot, showing integrity
  (`موثّقة` / `غير موثّقة` / `تالفة`), what each plugin asks for *before* the
  switch, and refusals where the attempt was made.

### Interfaces
- **`vara-tui`** — the terminal surface: one file of application code over the
  same core, no window, no WebView2. Verified end to end against a live model:
  asked "اشرح نظامي" it chose `system_info`, ran it on the real machine, and
  answered from what it read — including saying it could not read memory size
  rather than guessing.
- **The dev build launches again**: it sets its own `WEBVIEW2_USER_DATA_FOLDER`
  in debug, because the installed app and a development build share one bundle
  identifier and WebView2 allows one process per profile.
- **Views are addressable by URL hash** (`#plugins`, `#settings`), so every
  screen is reachable by a screenshot script or a test.

### Goals (`goals.rs`)
- Outcomes with a **measured** stopping condition: a metric with a target, or a
  checklist. There is deliberately **no "the model decides it is done" variant** —
  a goal that cannot be measured is refused at construction.
- Metrics move only when a step says they moved, and the step carries its
  evidence; budgets and step ceilings are hard; a step that needs the owner parks
  the goal; consecutive no-change steps become a question to the owner instead of
  a loop; and the default decision is **silence**.

### Reliability and honesty
- A tampered manifest reports `BROKEN` and will not load. Verified by editing a
  shipped plugin and watching it fail, then restoring it.
- **Local debug builds are 2.44 GB instead of 10.4 GB** and finish in 2m28s after
  tuning `[profile.dev]`, which is what makes building on an ordinary laptop
  possible at all.
## v0.6.0 — Approvals you can audit, provenance you can measure (2026-10-02)

### Security: the model proposes, the backend decides
- **Backend-minted action proposals** (`exec_policy.rs`, migration v5
  `action_proposals`): the shell turns each `[[sys]]` proposal into a row with a
  SHA-256 digest of exactly what was proposed, a risk class, a reason and a
  two-minute expiry. The webview receives ids; `sys_approve(proposal_id)` takes
  the owner's decision (compare-and-swap, single-use); `sys_execute(proposal_id)`
  claims the approved row atomically, re-checks the digest, and executes the
  target **stored in the row**. Approval cards survive reload.
- **`run` is argv-only.** `exec_policy::tokenize_command` splits a command line
  into arguments and refuses shell metacharacters; `CommandPolicy` denies shells
  and system tools (`cmd`, `powershell`, `wmic`, `schtasks`, `reg`, `certutil`,
  `shutdown`, …) and inline-code flags (`python -c`, `node -e`); paths are
  confined with `confine_to_root` (escapes, ADS, reserved device names);
  `child_env` scrubs every `*_API_KEY` / `*_TOKEN` / `VARA_PROVIDER_*` from a
  spawned command's environment.
- **Commands ship OFF** (`run_commands: false` by default); the risky defaults
  are now opt-in across the board.
- **The API key can no longer reach disk or the webview**:
  `Settings::without_api_key()` is what gets persisted when the environment
  supplies the key, `Settings::for_webview()` is what `get_bootstrap` returns
  (plus `has_api_key` / `api_key_source`), and an empty key from the UI means
  "keep the stored one".
- ActLoop hardening: `allow_screenshots` is enforced **inside** the loop (SEE ops
  refused, implicit evidence captures skipped, `unverified_mutations` reported
  instead of faked); every pixel-coordinate op (including `click_win`) is
  grounded; destructive ops are defused by the loop rather than trusted to the
  adapter; combo detection is alias/whitespace tolerant.

### Provenance gate: C3, receipts, and an unbreakable split
- **C3 — quote grounding**: every quoted span in a claim must appear verbatim in
  the retrieved text of a URL that claim cites. `check_provenance_full` returns
  `(ProvenanceResult, GateReceipt, Vec<ClaimAudit>)`; C3 can only make a verdict
  stricter. Missing snapshots are reported as *not evaluable with a reason*,
  never as a pass, and never as a fabricated number.
- **The panic is gone**: the body/sources split now uses an exact byte partition
  (`split_inclusive`). A report ending in `## Sources` with no trailing newline,
  or with CRLF line endings, used to panic the checker — which aborted the
  mission task and left the entity stuck `busy` forever.
- **`GateReceipt` stored with the report** (migration v6 `reports.receipt_json`):
  gate version, backed ratio with a Wilson 95% interval, claim counts,
  ref-resolution rate, C1/C2/C3, unresolved refs, missing quotes, and the
  not-evaluable reason.
- The repair pass is now driven **only** by the gate's deterministic failure
  list (failing citations, unresolved refs, missing quotes) — never by
  free-form self-critique.

### Reliability
- **Migrations are transactional and self-healing**: applied per statement inside
  a transaction, tolerating already-applied objects; a version row lost to a
  crash is re-applied instead of bricking the database. The FTS index rebuilds
  itself when it drifts from the notes table.
- **Report reserve** (`REPORT_BUDGET_TOKENS`): retrieval stops at
  `budget − reserve`, so a mission can never spend everything and be unable to
  write the report; the writer/repair `max_tokens` derives from what remains.
- **Budgets are measurable**: streaming requests ask for
  `stream_options.include_usage`, so mission accounting is no longer zero on
  providers that omit usage by default.
- The entity's `busy` flag resets through a drop guard — a panic in the runner
  can no longer leave Vara permanently "already working".
- `latest_report_for_mission` column/reader mismatch fixed (report attachments in
  follow-up chats were silently dropped).

### Protocol parity (Rust ↔ webview)
- `src/lib/protocol.ts` is the webview's single parser; `state.svelte.ts`
  re-exports it and no longer duplicates regexes.
- `tests/fixtures/protocol_cases.json` (28 cases) is read by **both**
  `crates/vara-core/tests/protocol_parity.rs` and `tests/protocol.test.ts`
  (209 assertions, vitest, wired into CI).
- Fixed: the webview's regex was missing the `[[mission_close]]` variant (raw
  marker + goal leaked into the bubble), its unterminated-goal cap lacked Rust's
  300-char rule, and `stripProtocolBlocks` stripped a different number of
  mission blocks. Fixed in core: two caps sliced at raw byte offsets and could
  **panic mid-character** on Arabic/emoji replies (`floor_char_boundary`).

### Product
- `list_skills` + Memory ▸ *Vara's own rules*: `skills/vara/*/SKILL.md` ship as
  bundle resources and are readable offline — read-only prose, never executable.
- Approval cards show a risk class (`low`/`medium`/`high`) and i18n'd labels;
  policy-refused proposals render as a refusal instead of a dead button.
- `SECURITY.md` rewritten around the real trust boundary (and what is *not*
  protected); `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `README.md` updated to
  match the code.

### Tests
`cargo test -p vara-core`: 53 lib + 15 computer-use harness + 5 protocol parity +
29 provenance gate + 6 provenance regression. `npm run test`: 209 assertions.
`npm run check`: 0 errors. `cargo fmt --all -- --check`: clean.

## v0.5.0 — The entity has hands: computer use as a native capability (2026-10-01)

### vara-core::computer_use — the ActLoop and the operation language
- New module: a serde port of the owner's Windows computer-use MCP design
  philosophy (see docs/COMPUTER_USE.md for the full mapping). The value
  carried over is the **discipline**, not the tools:
  - `CuOp`: 16-op tagged enum (`screenshot`, `verify`, `focus`, `click`,
    `click_win`, `move`, `type`, `hotkey`, `key`, `keys`, `scroll`, `wait`,
    `ignore_errors`, `close_window`, `close_app`) — unknown ops and malformed
    steps are rejected at parse time: **validate-then-execute**, a typo can
    never half-execute a UI flow (the MCP's run_actions discriminated union).
  - `CuSequence::parse` + `max_grant` + per-op summary for approval cards.
  - **Grant ladder L0–L3** (`GrantLevel`): observe / reversible input /
    destructive lifecycle / system. Destructive hotkey combos
    (alt+f4, ctrl+w, ctrl+q, ctrl+f4, ctrl+shift+w) auto-escalate to L2.
  - **ActLoop**: structural enforcement of the see→act→confirm contract —
    ungrounded coordinates are refused (never blind-click), destructive ops
    stay dry runs without an L2 policy, sequences ending unverified get one
    final evidence capture, and correction is bounded to one retry that only
    fires before any mutation happened.
  - `LoopReport` discipline metrics: `verify_discipline()`, `grounded_clean()`,
    retries, blind refusals — the harness grades these, not just outcomes.

### Three adapters behind one contract
- `MockComputerUse`: a deterministic virtual Windows desktop that reproduces
  the real failure modes — stale frames from animating panels (settle
  discipline), focus theft (the `active` diagnosis field), blind-click
  refusal, focus-first typing. It is the harness world.
- `SidecarComputerUse`: MCP stdio client for the owner's Python server frozen
  via PyInstaller (see sidecar/README.md). Full tool mapping + response-
  as-guidance passthrough (ok/error/active/path/check/dry_run survive).
- Phase-2 native Rust port targets documented (enigo / windows-rs / DXGI /
  Windows.Media.Ocr) behind the same `ComputerUseAdapter`.

### Action Journal (audit log + memory of deeds)
- New `cu_journal` table (migration v4): every executed step with grant
  level, target, before/after refs, `check` note, and errors — the entity's
  deeds are auditable and become her own memory. Retention pruning included.

### Chat protocol v2: [[sys]] "computer_use"
- Vara can now propose whole **verified UI sequences from inside the
  conversation** — chat and task execution stay one interface (no separate
  task launcher). The identity prompt teaches the sequence language and its
  six discipline rules; extraction is tolerant (mangled markers, code
  fences) and parses into `CuOp`s before anything is proposed to the owner.
- Policy: ships **OFF** (`allow_computer_use: false`); L2 (close ops) needs
  the separate `computer_use_allow_close` unlock. Every proposal shows the
  in-chat approval card; every execution is policy-gated in the shell —
  the model never runs anything by itself.

### The harness (both faces)
- **Entity harness**: 9 scenario tests (`computer_use_harness.rs`) asserting
  grounded flows, stale-frame recovery, destructive gates, blind-click
  refusal, focus-mismatch diagnosis, bounded correction, schema rejection,
  journal completeness, and grant classification — the ported contract of
  the owner's MCP test suite.
- **Research harness restored**: `scripts/vara_harness_v2.mjs` (referenced by
  the pre-registered EXPERIMENT_DESIGN.md but previously uncommitted) — the
  24-run four-arm campaign runner (A/C/B/D × m1/m2 × 3) with hard 90k token
  budgets, named-model ledger rows, real search/fetch tools, and stop-cause
  accounting; `--plan` mode smokes the full plan without API calls. Plus
  `scripts/check_provenance.mjs`: the M1 backed-ratio gate as a standalone
  step (cited ⊆ fetched + [n] resolution, PASS ≥ 0.95).

### Skill: vara-gui-driver-windows
- Sixth project skill — the entity's GUI instincts: capability map, the
  commands→GUI fallback rule (first-class path, two failures then pivot),
  transient-UI handling, verify-state rules, speed and safety rules.

### Settings & UI
- Two new bilingual Settings toggles (computer use; destructive unlock)
  shown only when computer use is on; `computer_use` receipts render in the
  thread with their own icon and label.

---

## v0.4.0 — The entity sees the screen (2026-10-01)

### First step into computer use: `screenshot`
- New `[[sys]]` action: `{"action":"screenshot","target":"screen"}`. Vara can
  now propose capturing the screen — the owner taps the approval card, the
  shell layer captures via the OS's own tooling (Windows: PowerShell +
  System.Drawing; macOS: `screencapture`; Linux: gnome-screenshot / ImageMagick
  / scrot), and the PNG lands in `app-data/screenshots/` with its path in the
  receipt. Zero new native dependencies.
- Privacy posture: ships **OFF** (`allow_screenshots: false`). Even when
  enabled, every capture still requires the explicit in-chat approval card,
  leaves a receipt in the thread and an events-table row. The model still
  cannot see the image — vision grounding is a future, separately-discussed
  step.
- New Settings toggle with bilingual labels; core + webview extraction kept in
  sync and unit-tested (empty target normalizes to `screen`; other actions
  still require a target).

### Provider via environment variables (dev & test discipline)
- `VARA_PROVIDER_API_KEY` / `VARA_PROVIDER_BASE_URL` / `VARA_PROVIDER_MODEL`
  override the settings-file provider at launch. Precedence: env > file, and
  overrides are re-applied after every UI save — an env-injected key can never
  be wiped by a Settings save nor leak into `settings.json`.

### Reliability
- README platform badge corrected to what actually ships today: Windows x64
  (macOS/Linux packaging remains on the roadmap).

## v0.3.0 — The entity acts (2026-10-01)

The critique that drove this release: *"where do I chat with the entity — not
just open separate missions? Missions should happen IN the chat, and Vara
should actually control the machine."* So v0.3.0 removes the seams.

### Missions live inside the conversation (chat-first, end to end)
- **No more separate mission launcher.** The Dashboard and Missions pages are
  gone; the chat is the product. When a request is a goal, Vara chats about it
  AND starts the mission in the same thread.
- **Auto-start by default** (`auto_start_missions`, budget-capped, read-only):
  a proposed mission begins on its own, a live mission card appears in the
  thread, and progress (steps, tokens, status) streams into it in real time.
- The final report lands back in the thread as a closing message; the
  conversation is linked to the mission, so every follow-up is grounded in the
  report. Manual "turn into mission" remains as fallback when Vara is busy.
- Migration v3: messages gain `kind` ("text" | "mission" | "action") and
  `mission_id`.

### Vara controls the machine (policy-gated)
- New `[[sys]]` protocol: the model proposes `open_url` / `open_path` / `run`
  actions; the **shell layer** executes them — the model never runs anything
  itself.
- Every action lands in the thread as a receipt card (target, ok/fail, output
  snippet) and an events-table row. Nothing executes invisibly.
- `run` commands always require an explicit in-chat approval card, run with a
  60s timeout, capped output, and cwd confined to the watched folder or home.
- Toggles: `run_commands`, `open_urls`, `open_paths` — deny wins.

### Protocol leak fixed for real
- Models mangle markers (`[mission] … {MISSION_CLOSE}`); v0.2.0 matched the
  exact protocol only, so raw markers leaked into bubbles. Extraction is now
  tolerant (core + webview, in sync, unit-tested) — the UI can no longer leak
  protocol text. During streaming, blocks are stripped live.

### Provider & platform
- Shipped default provider preset (OpenAI-compatible endpoint + model, empty
  key — BYOK, the key never ships).
- tokio::process based command runner; new `start_mission_in_conversation` and
  `sys_execute` commands.

### Hygiene
- Git history rewrite: all commits now correctly attributed to the owner
  (a stray `vara-dev` contributor identity is gone).
- `AGENTS.md` + five project skills under `skills/vara/` encode the
  invariants for any agent working on the repo.

## v0.2.0 — The conversation is the entity (2026-10-01)

The v0.1.0 build answered "what can the entity do?" with a mission runner.
This release answers the question that actually matters: **where do I talk to
her?** Vara is now conversation-first — missions are something she does *from
within the chat*, not a separate mode.

### Chat with the entity (new)
- Persistent conversations in SQLite (migration v2): threads with follow-up
  messages, auto-titled from the first message.
- Streaming replies: the model writes token-by-token into the bubble via SSE
  (`chat_stream`), with a graceful fallback for providers that ignore
  `stream:true`.
- Memory grounding: each reply is contextualized with FTS-matched notes from
  Vara's persistent memory — keyword-based OR-matching tuned for natural
  sentences (`search_notes_any`), not naive phrase matching.
- Identity + persona flavors: the chat system prompt carries Vara's identity
  ("persistent entity, not a resetting chatbot"), mirrors the user's language,
  and shifts tone per persona (classic / dark / stealth / tech / nature).
- Mission proposals from inside chat: Vara may end a reply with a
  `[[mission]] … [[/mission]]` block; the UI renders a proposal card with a
  one-click **Turn into a mission** action.
- Stop button: abort a streaming reply while keeping the partial answer.
- Copy per message; model + token accounting per reply.

### From talk to action and back
- **Discuss report**: every report (Reports view + Dashboard) opens a chat
  thread with the report attached in context — follow-ups continue against
  the actual findings.
- Chat-first home screen: the app opens in the conversation; the dashboard
  hero has a quick-ask input that hands off to the chat.

### Design
- Living background: persona-tinted aurora gradients that breathe (slow
  hue/brightness drift) + fine film grain so dark surfaces never look flat.
- Deep-glass cards, gradient buttons, glowing avatar halo, streaming caret,
  thinking dots, message fade-ins.

### Over-the-air updates (new)
- `tauri-plugin-updater` wired to GitHub Releases `latest.json` with minisign
  key verification; update artifacts signed in CI.
- The app probes for updates silently on every launch; a sidebar badge +
  Settings card offer one-click **install & restart** with download progress.
- No manual reinstalling for users, ever.

### Under the hood
- reqwest `stream` feature + `futures-util` in vara-core.
- New core module `chat.rs` (identity, memory grounding, proposal protocol)
  with unit tests; `db.rs` gains conversation/message methods and a keyword
  extractor (`keywords_of`, Arabic + Latin aware).
- Browser mock of the backend (`src/lib/mock.ts`) so the whole UI runs in a
  plain browser for development and visual verification.
- CI: cargo fmt + 21 core tests + svelte-check + vite build + Windows
  `cargo check`; Release workflow now signs and publishes updater artifacts.

### Known limitations
- Windows packaging only (NSIS); Linux/macOS builds planned.
- Web search uses DuckDuckGo HTML endpoints — best effort, no API key.
- The checker proves citation integrity, not factual truth of sources.
- Chat replies do not themselves search the web — Vara proposes a mission
  instead (honesty over theater).

## v0.1.0 — First public release (2026-10-01)

The first release contains what we are confident in.

### The entity
- Mission runner: plan → search/fetch → live replan → report, with a binding
  token budget and step cap.
- Clean-context report writer: sees only the retrieval ledger, must cite [n],
  forbidden from inventing URLs.
- Provenance gate: structural citation check (C1/C2) after every report, with
  one automatic repair pass on FAIL. Regression-tested against the v1
  experiment data (backed_ratio 0.308 fabrication case is caught).
- Memory: SQLite (WAL) with FTS5 search (LIKE fallback), note deduplication
  (Jaccard ≥ 0.72), sources with fetched/seen provenance.
- Idle reflection heartbeat (off by default).

### The app (Windows first)
- Tauri 2 + Svelte 5, Arabic (RTL) and English UI.
- System tray: show/hide, pause/resume, open data folder, quit.
- Close-to-tray with a first-time explanation notification.
- Optional start-with-system, desktop notifications.
- Watched folder: new .md/.txt files are indexed into memory with file:// provenance.
- Persona styles (Classic / Dark / Stealth / Tech / Nature) with the 8
  character states mapped to entity activity.

### Experiment tooling (open by design)
- `docs/experiments/v1/` — raw ledgers, reports, the verdict, checker outputs.
- `docs/experiments/v2-design/EXPERIMENT_DESIGN.md` — pre-registered four-arm
  design with frozen decision rules R1/R2/R3.
- `scripts/aggregate_v2.mjs` — campaign aggregation + automatic rule
  evaluation + blind-review anonymization.

### Known limitations
- Windows packaging only (NSIS); Linux/macOS builds planned.
- Web search uses DuckDuckGo HTML endpoints — best effort, no API key.
- The checker proves citation integrity, not factual truth of sources.
