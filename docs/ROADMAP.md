# Roadmap

Scope discipline: each release ships only what we are confident in.

**Where this file stands (2026-10-04).** v0.7.0 is released. The section below
called "v0.7 — sandbox and computer use" was written *before* that release and
describes work that did **not** ship in it; it is therefore relabelled as still
open rather than presented as v0.7's contents. For the honest status of every
product claim — including the ones that are currently false — see
[`REALITY_MATRIX.md`](REALITY_MATRIX.md).

## v0.7 — everything is a plugin (released 2026-10-04)
- [x] Plugin manifest contract: twelve slots, deny-by-default permissions, a
      SHA-256 over the claims, folder-wide content hash, deterministic
      dependency order, real-folder discovery
- [x] A plugin host with no privileged core: registrations are reversible
      effects that unwind on unload; a failing plugin is recorded unhealthy and
      does not take the host down
- [x] Capability seams (provider/consumer) and profiles/bundles for composing a
      run from a different set of plugins
- [x] Fifteen plugins ship, each a real folder, all hash-verified; everything
      dangerous ships off
- [x] `vara-plugins` (list/plan/check/hash/enable/disable/approve) and a plugin
      panel in the app
- [x] `vara-tui`: the same entity in a terminal, one file over the same core
- [x] Goals with a measured stopping condition (no "the model decides it is
      done" variant)
- [ ] **Honesty gaps found by audit and now fixed**: C1 requires an actually
      fetched source; the gate receipt is rendered; a FAIL report is not labelled
      completed; the hard-deny floor covers commands; junctions are resolved on
      the filesystem; zombie missions are reconciled at startup; the heartbeat
      latch survives off→on; streaming replies keep split SSE frames; `plugins/`
      is bundled
- [ ] **Still open — the largest one**: the production mission path does not yet
      load its capabilities through the plugin host. Until it does, the README
      says "declared and manageable as plugins", not "everything is a plugin"
- [ ] Resume-instead-of-mark-interrupted after a crash (missions are now
      reconciled to `interrupted`; they are not restarted)

## v0.8 — a real boundary for OS actions
- [ ] `run` gets a real boundary: restricted token + private desktop + job
      object (no admin), failing closed when it cannot start
- [ ] Keyring (Windows Credential Manager) instead of `settings.json` for the
      provider key, with migration and a `has_key`-only UI
- [ ] Internal-address blocking in the fetch path (loopback, private ranges,
      link-local, metadata endpoints) and an explicit redirect policy
- [ ] Windows CI run of the sidecar (`vara-cu` via PyInstaller) against the
      harness scenarios — or an honest "bring your own sidecar" story
- [ ] UIA two-tier perception (widget tree over pixel OCR)
- [ ] Secret vault injection (type from vault, never through model context)

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

## v0.6 — the honesty release (shipped 2026-10-02)
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
- [ ] Windows restricted-token sandbox for `run` (moved to v0.8)
- [ ] Pre-registered gate evaluation: injected-fabrication corpus, sensitivity
      and false-FAIL rates published with the gate version

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
