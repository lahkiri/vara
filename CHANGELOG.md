# Changelog

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
