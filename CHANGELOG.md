# Changelog

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
