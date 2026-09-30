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
  <a href="#download"><img alt="platform" src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-green"></a>
  <img alt="local-first" src="https://img.shields.io/badge/state-local%20%26%20portable-informational">
  <img alt="provenance" src="https://img.shields.io/badge/reports-provenance--gated-8b8cf0">
</p>

<p align="center">العربية؟ اقرأ <a href="README.ar.md">README بالعربية</a></p>

---

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
| 🧠 **Persistent entity** | Missions, memory (SQLite WAL + FTS5), reflections, event log — all in one portable data folder |
| 🛡 **Provenance gate** | Structural citation check (C1: cited ∈ retrieved · C2: refs resolve) on every report, with an automatic repair pass |
| 🌐 **Bring any model** | OpenAI-compatible protocol: Z.ai, OpenAI, Ollama, LM Studio, llama-server, vLLM… |
| 🔎 **Honest research loop** | Plan → search/fetch → live replanning → clean-context writer → checker → repair → report |
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
3. On the Dashboard, give Vara a mission and a token budget. Watch the live
   "Vara is working…" checklist.
4. When the report lands, open it: the **provenance badge** (PASS/FAIL + backed %)
   is computed by the same checker that caught the v1 fabrication.

## Architecture

```
┌────────────────────────── desktop app (Tauri 2) ──────────────────────────┐
│  Svelte 5 + Tailwind 4            Rust shell: tray, notifications,        │
│  AR/EN · live checklist           single-instance, autostart, watcher     │
│        │ events │ commands                │                               │
│        ▼                                  ▼                               │
│  ┌──────────────────────── vara-core (pure logic) ─────────────────────┐  │
│  │ entity.rs  state machine · mission runner · budget · replan         │  │
│  │ provenance.rs  C1/C2 citation gate  (regression-tested vs v1 data)  │  │
│  │ db.rs  SQLite WAL · FTS5-with-fallback · notes/sources/reports      │  │
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
