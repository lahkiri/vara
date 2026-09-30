# Roadmap

Scope discipline: each release ships only what we are confident in.

## v0.1.x — hardening (current)
- [ ] CI on every push (core tests + web build + cross-compile check)
- [ ] Windows installer via GitHub Actions release pipeline
- [ ] Crash/log collection opt-in
- [ ] Mission pause persistence across app restarts

## v0.2 — the pre-registered campaign
- [ ] Run the 24-run four-arm campaign (A/C/B/D × m1/m2 × 3) with
      `harness v2` + `aggregate_v2.mjs` once an API budget is allocated
- [ ] Apply frozen rules R1/R2/R3 → the outcome directly shapes v0.3:
      - R1 decides whether *dynamic parallel organization* enters the product
        or Phase 2 becomes "single agent + independent verifier"
      - R2 decides whether the clean-context writer becomes a permanent
        built-in component (it already is for reports; R2 extends it)
- [ ] Publish the campaign artifacts in `docs/experiments/v2-runs/`

## v0.3 — deeper system integration
- [ ] Scheduled missions (cron-like) + mission chains
- [ ] Watched-folder rules (per-folder persona, auto-missions on drop)
- [ ] Report templates + export to PDF/DOCX
- [ ] Global hotkey to summon Vara

## v0.4 — local-first inference
- [ ] llama.cpp sidecar management (download, launch, health)
- [ ] Model catalog with hardware-based recommendations
- [ ] Full offline mode: local model + local memory

## v1.0 — multi-OS + teams (post-campaign)
- [ ] Linux (AppImage/deb) and macOS (dmg/notarized) packaging
- [ ] If R1 said yes: supervised parallel researchers with shared ledger
      (the "living organization"), mid-mission supervision checkpoints
      (ADR-008/010 candidates)
- [ ] Skills with a structural allowlist (self-created networked skills stay
      rejected per ADR-015)

## Non-goals (v1)
- Cloud sync of memory (local & portable state is a brand promise)
- Autonomous purchases/emails/messages without explicit per-action consent
- Hiding provenance metrics behind paywalls — they ship in the core
