# Roadmap

Scope discipline: each release ships only what we are confident in.

## v0.6 — the honesty release (current)
- [x] Provenance gate hardened: the byte-slice panic that could freeze the
      entity is gone; C3 (verbatim quote grounding) added; a `GateReceipt`
      (gate version, ratios with Wilson intervals, claim counts) is stored with
      every report
- [x] Approvals are backend-enforced: proposals are minted in the core, decided
      by id, single-use and time-boxed; `run` is argv-only with denied programs,
      confined paths and a scrubbed environment; commands ship OFF
- [x] ActLoop: screenshots governed by `allow_screenshots`, every pixel-coordinate
      op grounded, destructive ops defused by the loop itself
- [x] Migrations transactional and self-healing; FTS index rebuilt when stale;
      env-provided API keys never persisted; the webview never receives one
- [x] One protocol parser per side, locked by a shared fixture (`test:vitest` +
      `protocol_parity`); a mid-character panic in the tolerant parser fixed
- [x] Report reserve so retrieval can never spend the budget needed to write
- [x] `SECURITY.md` rewritten around the real trust boundary and its limits
- [ ] Windows restricted-token sandbox for `run` (see below)
- [ ] Pre-registered gate evaluation: injected-fabrication corpus, sensitivity
      and false-FAIL rates published with the gate version

## v0.2 — the pre-registered campaign (still open)
- [ ] Run the 24-run four-arm campaign (A/C/B/D × m1/m2 × 3) with
      `scripts/vara_harness_v2.mjs` (restored in v0.5.0) + `aggregate_v2.mjs`
      once an API budget is allocated
- [ ] Apply frozen rules R1/R2/R3 → the outcome directly shapes the next major:
      - R1 decides whether *dynamic parallel organization* enters the product
        or Phase 2 becomes "single agent + independent verifier"
      - R2 decides whether the clean-context writer becomes a permanent
        built-in component (it already is for reports; R2 extends it)
- [ ] Publish the campaign artifacts in `docs/experiments/v2-runs/`

## v0.7 — sandbox and computer use
- [ ] `run` gets a real boundary: restricted token + private desktop + job
      object (no admin), failing closed when it cannot start
- [ ] Keyring (Windows Credential Manager) instead of `settings.json` for the
      provider key, with migration and a `has_key`-only UI
- [ ] Windows CI run of the sidecar (`vara-cu` via PyInstaller) against the
      harness scenarios — or an honest "bring your own sidecar" story
- [ ] UIA two-tier perception (widget tree over pixel OCR)
- [ ] Secret vault injection (type from vault, never through model context)

## Deeper system integration
- [ ] Scheduled missions (cron-like) + mission chains
- [ ] Watched-folder rules (per-folder persona, auto-missions on drop)
- [ ] Report templates + export to PDF/DOCX
- [ ] Global hotkey to summon Vara

## Local-first inference
- [ ] llama.cpp sidecar management (download, launch, health)
- [ ] Model catalog with hardware-based recommendations
- [ ] Full offline mode: local model + local memory

## v1.0 — multi-OS + teams (post-campaign)
- [ ] Linux (AppImage/deb) and macOS (dmg/notarized) packaging
- [ ] If R1 said yes: supervised parallel researchers with shared ledger
      (the "living organization"), mid-mission supervision checkpoints
      (ADR-008/010 candidates)
- [ ] Third-party skills with a structural allowlist (self-created networked
      skills stay rejected per ADR-015). Bundled skills today are read-only
      prose, deliberately *not* executable.

## Non-goals (v1)
- Cloud sync of memory (local & portable state is a brand promise)
- Autonomous purchases/emails/messages without explicit per-action consent
- Hiding provenance metrics behind paywalls — they ship in the core
