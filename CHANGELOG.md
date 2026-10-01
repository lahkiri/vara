# Changelog

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
