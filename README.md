<p align="center">
  <img src="src/assets/characters/face.png" width="120" alt="Vara"/>
</p>

<h1 align="center">Vara</h1>

<p align="center"><b>Not just an assistant. A living autonomous organization.</b></p>

<p align="center">
  You give Vara a mission.<br/>
  It plans, searches the web, remembers — and every report it writes passes a<br/>
  <b>machine-enforced provenance gate</b>: every citation must come from what it actually retrieved.
</p>

<p align="center">
  <a href="#download"><img alt="platform" src="https://img.shields.io/badge/platform-Windows%20x64-blue"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-green"></a>
  <img alt="local-first" src="https://img.shields.io/badge/state-local%20%26%20portable-informational">
  <img alt="provenance" src="https://img.shields.io/badge/reports-provenance--gated-8b8cf0">
</p>

<p align="center">العربية؟ اقرأ <a href="README.ar.md">README بالعربية</a></p>

---

## What's new in v0.6.0 — Approvals you can audit, provenance you can measure

- **The approval card is now a real gate, not a decoration.** A proposed action
  is minted by the core into a database row with a digest, a risk class and a
  two-minute window; you approve *that row* by id, and `sys_execute` re-reads the
  target **from the row** — once. A compromised or buggy UI cannot invent an
  action, name its own target, or replay an old approval.
- **`run` is argv-only. There is no shell.** Command lines are tokenized into
  arguments; pipes, redirections, `&&`, backticks and `$()` are refused; `cmd`,
  `powershell`, `wmic`, `schtasks`, `reg`, `certutil`, `shutdown`… are denied
  outright; inline-code flags (`python -c`, `node -e`) too. Paths are confined
  to your folder and the child gets a **scrubbed environment** — the provider key
  is not in it.
- **Commands ship OFF.** `run_commands` is now opt-in, like screenshots and
  computer use.
- **Provenance C3: quote grounding.** The gate now also checks that every quoted
  span appears **verbatim** in the text that was actually retrieved for the URL
  that claim cites. A report can no longer reach `PASS` with invented quotes —
  and a `GateReceipt` (gate version, backed ratio with a Wilson interval, claim
  counts, and an explicit "not evaluated, because…") is stored with every report
  so a verdict never travels without its denominator.
- **A panic that could freeze the entity is gone.** A report ending exactly in
  `## Sources` used to panic the checker, which aborted the mission task and left
  Vara permanently "busy"; the split is now an exact byte partition, and the busy
  flag resets on every exit path.
- **Migrations self-heal; FTS rebuilds itself.** A crash between applying a
  migration and recording it used to make the database unopenable forever
  (`duplicate column name`). Migrations now run per statement inside a
  transaction, recognise already-applied objects, and rebuild the full-text index
  when FTS5 becomes available after a period in `LIKE` mode.
- **Secrets stay off disk and out of the webview.** An environment-provided key
  is never written to `settings.json`; the UI receives `has_api_key` and its
  source, never the key.
- **One parser per side, locked by a fixture.** Rust and the webview used to
  disagree about `[[mission_close]]` (the marker leaked into the bubble) and
  about multi-byte caps (a mid-character panic). Both now read
  `tests/fixtures/protocol_cases.json` and are asserted on both sides — 209
  frontend assertions plus a Rust parity test in CI.
- **The report can no longer be starved by research.** Missions reserve a writer
  budget: retrieval stops at `budget − reserve`, and the arithmetic is journaled.
- **Your skills are visible.** The same `skills/vara/*/SKILL.md` rules the
  project uses are bundled and readable in Memory ▸ *Vara's own rules* — read-only
  prose, never executable.

Full details, honest limits and the fixes that are still open:
[`SECURITY.md`](SECURITY.md) · [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) ·
[`docs/PROVENANCE.md`](docs/PROVENANCE.md) · [`docs/ROADMAP.md`](docs/ROADMAP.md).

## What's new in v0.5.0 — The entity has hands

- **Computer use as a native capability.** Vara can now propose whole
  *verified UI sequences* from inside the chat: see → act → confirm. A
  validated 16-op operation language (`screenshot`, `focus`, `click`,
  `click_win`, `type`, `hotkey`, `scroll`, `close_window`…), a structural
  **ActLoop** that refuses ungrounded coordinates and dry-runs destructive
  ops, and a **grant ladder L0–L2** under the owner's Settings. Ships OFF.
- **Action Journal.** Every executed step — grant level, target, before/after
  evidence — is journaled in SQLite: the entity's deeds are auditable and
  become her memory.
- **The harness (both faces).** 9 headless scenario tests grading the
  see→act→confirm discipline against a deterministic virtual desktop, and the
  restored pre-registered research harness (`scripts/vara_harness_v2.mjs` +
  `scripts/check_provenance.mjs`) for the 24-run four-arm campaign.
- **Skill: `vara-gui-driver-windows`** — the entity's GUI instincts (the
  commands→GUI fallback rule, transient-UI handling, speed & safety rules).

## What's new in v0.4.0 — The entity sees the screen

- **`screenshot` action (first step into computer use).** Vara can propose
  capturing the screen from inside the chat: you tap the approval card, the
  capture runs through the OS's own tooling, and the PNG path lands in the
  thread as a receipt. Ships OFF (`allow_screenshots`), every shot needs an
  explicit tap, and the model never sees the image.
- **Provider via environment variables** — `VARA_PROVIDER_API_KEY`,
  `VARA_PROVIDER_BASE_URL`, `VARA_PROVIDER_MODEL` override the settings file
  (env > file, survives Settings saves). The key can stay out of disk entirely.

## What's new in v0.3.0 — The entity acts

- **Missions live inside the chat.** No separate launcher: ask for a goal and
  Vara chats with you *and* starts the mission in the same thread — a live
  card streams its progress, and the report lands back into the conversation,
  grounding every follow-up.
- **Vara controls the machine, behind policy.** Open URLs/paths, run shell
  commands — each proposal is gated by the autonomy settings, `run` always
  shows an explicit approval card, and every action leaves a visible receipt
  in the thread.
- **Protocol-leak fix**: tolerant marker parsing (models mangle
  `[mission]…{MISSION_CLOSE}`) — raw protocol text can no longer leak into
  chat bubbles.
- Signed **over-the-air updates** from v0.2.0 carry on: the app checks
  GitHub Releases on launch and updates itself.

## Why Vara exists

Most "AI teammates" answer questions. Vara's job is to **accomplish missions** — and to be
honest about what it knows. Our first controlled experiment (v1) showed a single-agent
pipeline fabricating 9 of its 13 cited sources (`backed_ratio = 0.308`) while *looking*
perfectly consistent. That failure became a permanent product feature: **no report leaves
the entity unless every citation resolves to a source that was actually retrieved.**
The checker is not a script in a drawer — it is compiled into the app
([`vara-core::provenance`](crates/vara-core/src/provenance.rs)) with regression tests seeded
from the experiment data itself.

Read the full experiment story in [`docs/experiments/`](docs/experiments/) —
raw ledgers, the fabricated-source audit, the verdict, and the pre-registered
v2 design with frozen decision thresholds.

## Features

| | |
|---|---|
| 💬 **A companion, not a form** | Chat with Vara in persistent conversations: follow-up messages, streaming replies, and answers grounded in her actual memory (FTS-matched notes) — the Dot/Muse-style continuity, on your desktop |
| ✦ **From talk to action** | When a request needs real research, Vara proposes a mission from inside the chat — one click turns it into a fully-gated research run. And any report has a **“Discuss report”** button that opens a follow-up thread with the report in context |
| 🧠 **Persistent entity** | Missions, memory (SQLite WAL + FTS5), reflections, event log — all in one portable data folder |
| 🛡 **Provenance gate** | C1 (cited ∈ retrieved) · C2 (`[n]` resolves) · C3 (quotes verbatim in the retrieved text) on every report, with an automatic repair pass and a stored receipt carrying intervals and denominators |
| 🔐 **Approvals that hold** | Every OS action becomes a backend-minted proposal you approve by id: single-use, time-boxed, digest-bound, and journaled as a receipt in the thread |
| 🌐 **Bring any model** | OpenAI-compatible protocol: Z.ai, OpenAI, Ollama, LM Studio, llama-server, vLLM… |
| 🔎 **Honest research loop** | Plan → search/fetch → live replanning → clean-context writer → checker → repair → report, with a reserved writer budget |
| 🛰 **Over-the-air updates** | The app checks GitHub releases on every launch and installs new versions with one click — signed updates, no manual reinstalling |
| 🖥 **Lives with your system** | System tray, close-to-tray, notifications, optional autostart, watched-folder indexing (`file://` provenance) |
| 🎭 **8 states, 5 styles** | Happy / Focused / Thinking / Excited / Serious / Working / Planning / On Mission — Classic / Dark / Stealth / Tech / Nature |
| 🌍 **AR + EN** | Full RTL/LTR interface switching |
| 📜 **Her rules, in the app** | The entity's own operating rules (`skills/vara/*/SKILL.md`) ship with the app and are readable in Memory — read-only prose, never executable code |
| 🔓 **Open source forever** | MIT. The provenance checker and its tests ship in the repo. |

## Download

Grab the Windows installer (`Vara_x.y.z_x64-setup.exe`) from
**[Releases](../../releases)**. Linux and macOS builds are on the roadmap
(the core is cross-platform; only packaging is pending).

## Build from source

```bash
# prerequisites: Node 20+, Rust stable (edition 2021)
npm install
npm run tauri dev      # develop
npm run tauri build    # produce the installer
```

## First run

1. Open **Settings → Model provider**, pick a preset (Z.ai / OpenAI / Ollama / LM Studio / llama-server), paste base URL + key + model.
2. Click **Test connection**.
3. Go to **Chat** and just talk — that is the heart of the app. Ask “who are you?” or anything else; Vara replies with streaming answers grounded in her memory and keeps the thread for follow-ups.
4. Ask for research (or press **Turn into a mission** on a proposal): Vara plans, searches, and writes a report that passes the **provenance badge** gate (PASS/FAIL + backed % + a stored receipt).
5. Press **Discuss report** to open a follow-up conversation with the report attached — dig into the findings without leaving the chat.
6. Anything Vara wants to *do* on your machine arrives as an **approval card** with a risk class: nothing runs until you press Run, and the receipt lands in the same thread. Commands, screenshots and computer use are off until you enable them in Settings.

## Security, in one paragraph

The model may only *propose*. Proposals become backend rows with a digest and a
two-minute window, you approve them by id, and execution reads the target from
the row — once. Commands are argv-only (no shell) with denied programs, confined
paths and a scrubbed environment; the API key is never written to disk when it
comes from the environment and never reaches the webview. Windows sandboxing is
**not** here yet — see [`SECURITY.md`](SECURITY.md) for what is protected and
what is not.

## Architecture

```
┌────────────────────────── desktop app (Tauri 2) ──────────────────────────┐
│  Svelte 5 + Tailwind 4            Rust shell: tray, notifications,        │
│  AR/EN · chat-first UI            single-instance, autostart, watcher,    │
│  streaming bubbles                OTA updater (signed)                    │
│        │ events │ commands                │                               │
│        ▼                                  ▼                               │
│  ┌──────────────────────── vara-core (pure logic) ─────────────────────┐  │
│  │ entity.rs  state machine · mission runner · budget + report reserve │  │
│  │ chat.rs    identity · persona · memory grounding · proposal parsing │  │
│  │ provenance.rs  C1/C2/C3 gate + receipt (intervals, claim counts)    │  │
│  │ exec_policy.rs argv-only commands · proposals · digests · confinement│ │
│  │ db.rs  SQLite WAL · self-healing migrations · FTS5-with-fallback    │  │
│  │ computer_use/  ActLoop + grant ladder + mock/sidecar adapters       │  │
│  │ tools.rs  web search + page fetch    llm.rs  any OpenAI-compatible  │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────┘
```

More: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) ·
[`docs/PROVENANCE.md`](docs/PROVENANCE.md) · [`docs/ROADMAP.md`](docs/ROADMAP.md) ·
[`docs/PERSONAS.md`](docs/PERSONAS.md)

## Security notes

- Your API key is stored **only** in `settings.json` inside the app data folder (or not at all when you supply it through `VARA_PROVIDER_API_KEY` — an environment key is never persisted), never synced, and never handed to the webview or to a command Vara runs.
- The model can only propose actions. Approvals are backend rows (digest + expiry, single-use); `run` is argv-only with no shell, so pipes, redirections and chained commands cannot slip past a card.
- The provenance gate is structural, not magical: it proves *citations came from real retrievals and quotes from real retrieved text*, not that a retrieved source is true. Treat it as a floor, not a ceiling.
- There is no OS-level sandbox yet, and computer use needs an external sidecar that this repository does not ship. Both are stated plainly in [`SECURITY.md`](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE).
