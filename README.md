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
| 🛡 **Provenance gate** | Structural citation check (C1: cited ∈ retrieved · C2: refs resolve) on every report, with an automatic repair pass |
| 🌐 **Bring any model** | OpenAI-compatible protocol: Z.ai, OpenAI, Ollama, LM Studio, llama-server, vLLM… |
| 🔎 **Honest research loop** | Plan → search/fetch → live replanning → clean-context writer → checker → repair → report |
| 🛰 **Over-the-air updates** | The app checks GitHub releases on every launch and installs new versions with one click — signed updates, no manual reinstalling |
| 🖥 **Lives with your system** | System tray, close-to-tray, notifications, optional autostart, watched-folder indexing (`file://` provenance) |
| 🎭 **8 states, 5 styles** | Happy / Focused / Thinking / Excited / Serious / Working / Planning / On Mission — Classic / Dark / Stealth / Tech / Nature |
| 🌍 **AR + EN** | Full RTL/LTR interface switching |
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
4. Ask for research (or press **Turn into a mission** on a proposal): Vara plans, searches, and writes a report that passes the **provenance badge** gate (PASS/FAIL + backed %).
5. Press **Discuss report** to open a follow-up conversation with the report attached — dig into the findings without leaving the chat.

## Architecture

```
┌────────────────────────── desktop app (Tauri 2) ──────────────────────────┐
│  Svelte 5 + Tailwind 4            Rust shell: tray, notifications,        │
│  AR/EN · chat-first UI            single-instance, autostart, watcher,    │
│  streaming bubbles                OTA updater (signed)                    │
│        │ events │ commands                │                               │
│        ▼                                  ▼                               │
│  ┌──────────────────────── vara-core (pure logic) ─────────────────────┐  │
│  │ entity.rs  state machine · mission runner · budget · replan         │  │
│  │ chat.rs    identity · persona flavors · memory grounding · proposal │  │
│  │ provenance.rs  C1/C2 citation gate  (regression-tested vs v1 data)  │  │
│  │ db.rs  SQLite WAL · FTS5-with-fallback · notes/sources/conversations│  │
│  │ tools.rs  web search + page fetch (html→text)   llm.rs  any model   │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────┘
```

More: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) ·
[`docs/PROVENANCE.md`](docs/PROVENANCE.md) · [`docs/ROADMAP.md`](docs/ROADMAP.md) ·
[`docs/PERSONAS.md`](docs/PERSONAS.md)

## Security notes

- Your API key is stored **only** in `settings.json` inside the app data folder, never synced.
- The provenance gate is structural, not magical: it proves *citations came from real
  retrievals*, not that a retrieved source is true. Treat it as a floor, not a ceiling.
- See [`SECURITY.md`](SECURITY.md) for reporting guidelines.

## License

MIT — see [LICENSE](LICENSE).
