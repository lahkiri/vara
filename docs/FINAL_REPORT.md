# FINAL REPORT — what was fixed, what was proved, what was not

**Date:** 2026-10-07 · **Branch:** `main` at `accca47` · **CI:** green (4 jobs)
**Release:** [v0.7.0](https://github.com/lahkiri/vara/releases/tag/v0.7.0), English, generated

This is the delivery report for the audit-and-repair pass. It follows one rule
throughout: **a claim appears here only with the command whose output proved it.**
Where that was impossible, the claim is in the last section instead.

---

## 1. The audit, and what it looked like before

Vara v0.7.0 was published with release notes describing **twenty plugins and five
theme/persona asset files**. The repository contained **fifteen plugin folders**
and **no asset files**. That was not a typo: it was the visible edge of a pattern
that the audit found in fourteen places — **a mechanism present, and the wiring to
it absent.**

`docs/REALITY_MATRIX.md` records all of it, including the two accusations the
audit made that turned out to be **wrong** (`ci.yml` was intact; there was no stray
probe in `commands.rs`). It now carries **15 `FIXED` rows against 8 that remain
`FALSE`**, each with its evidence.

## 2. Fixed, each with the command that proves it

| # | Defect | Proof | Commit |
|---|---|---|---|
| 1 | A FAIL report was announced as "Mission complete ✅" | status is now verdict-derived; `cargo test -p vara-core` | `0794ce4` |
| 2 | Streaming replies silently dropped split SSE frames | `cargo test -p vara-core --lib llm` — one test requires the text, another **asserts the old behaviour still loses it** | `0794ce4` |
| 3 | `plugins/` was not bundled, so the installer shipped no plugins | `tauri.conf.json` resources | `0794ce4` |
| 4 | Theme/persona slots shipped zero bytes | assets in their slot folders; `check-consistency` reports 15 folders, 15 manifests, 0 asset-only | `0794ce4` |
| 5 | C1 accepted a citation to a page never fetched | `cargo test -p vara-core --test provenance_tests`; the third assertion passes the old input and shows it **passing** | `4feb724` |
| 6 | `run` could read `settings.json` — the provider key reached the model's context | `cargo test -p vara-core --lib tools_registry` — 7 leaking shapes refused, 5 ordinary commands still allowed | `0ee4683` `db95357` |
| 7 | A junction inside an allowed root escaped it | reproduced live (`mklink /J` → `TOP-SECRET`), then `cargo test -p vara-core --lib junction` | `015a8e8` |
| 8 | A PASS could be shown with C3 never evaluated | receipt rendered: C3 in three states, CI, denominators, `not_evaluable_reason`; `svelte-check` + `vite build` clean | `753e13f` |
| 9 | A mission killed mid-flight stayed `running` forever | `cargo test -p vara-core --lib a_mission_left_running` | `7274874` |
| 10 | The heartbeat never restarted after off→on | single `false` store on every exit path via a `Drop` guard | `64e5014` |
| 11 | The fetch path could reach the LAN, loopback and cloud metadata | `cargo test -p vara-core --lib ssrf` — 19 URLs refused, 5 public kept, ranges asserted individually | `9fe9fee` |
| 12 | `clippy` found a loop that never loops — the checker reported only the first broken manifest | `cargo clippy --workspace --all-targets` | `0d8e49e` |
| 13 | Five documents disagreed with the product | `node scripts/check-consistency.mjs` | `cc8eedd` |
| 14 | `system_info` answered `unknown RAM` on a 16-core machine | **live run**: `16 CPU cores · 15.2 GB RAM` (was `unknown RAM`) | `53185e7` |
| 15 | **"Everything is a plugin" had no production path** | `Host::new` at `lib.rs:242`, `LogPlugin` loaded at `:258`, both runtime sites resolve through `mission_sink`; `cargo test -p vara-core --lib seam_wiring` | `0c5d96e` `35082d1` |

Plus `docs/REALITY_MATRIX.md` (`docs/REALITY_MATRIX.md`), the docs index
(`docs/README.md`), and a CI that now blocks: `cargo test --workspace` (the three
crates that build without GTK), `clippy -D warnings`, and `check-consistency`.

## 3. Verified by running the product, not only by tests

```
$ target\debug\vara-tui.exe "system info please"
[router] call system_info {}  (Question about this machine's OS/CPU/memory)
[tool]   windows (x86_64) · 16 CPU cores · 15.2 GB RAM · host WISSEM
[tool]   total=1 truncated=false
```

`cargo test -p vara-core -p vara-tui -p vara-plugins` → 185 + 15 + 5 + 29 + 7 green
`cargo clippy … -- -D warnings` → clean · `svelte-check` 0/0 · `vitest` 209 ·
`vite build` succeeds · `check-consistency` passes.
CI run [37244014074](https://github.com/lahkiri/vara/actions/runs/37244014074): **success**, 4 jobs.

## 4. **Not** verified — and therefore not claimed

- **A live GUI boot.** `vara.exe` panics in WebView2 (`HRESULT 0x8000FFFF`) at
  `tauri app.rs:1444`, before setup completes, because an orphaned
  `msedgewebview2` process cannot be killed without elevation on this machine.
  **Consequence: the startup work in `35082d1` has never executed inside a live GUI
  process.** The database contains no `plugins` event, which is the evidence.
- **A full mission end to end** against a real provider, and **the heartbeat firing
  on its timer**, in a live process.
- **Anything about the Linux build beyond CI.** My local checks are MSVC.

I also made four mistakes *during* this pass that CI caught and I am recording
rather than hiding: an `if` with two identical branches; a `--workspace` flag in a
Linux job that cannot build GTK; **verifying a different command locally from the
one I committed**; and a test of mine that passed because it was never compiled.
The rule I keep from the last one: an extracted command, not a remembered one.

## 5. Still open, in the product's own words

From `docs/REALITY_MATRIX.md`, rows that remain `FALSE`:

- **"Everything is a plugin"** — now `PARTIAL`: one capability (the event stream)
  travels a real seam, in production, with a test proving that unloading the plugin
  stops it. **The tools do not** (`tool_bridge.rs:127` builds `FsToolHost`
  directly), and `db`/`http` are fields rather than seams.
- **Toggling a plugin in the panel does not change a running mission.** The host
  loads `LogPlugin` **from code**, not from the registry's plan.
- **A folder with `plugin.toml` does not add a capability** — it is declarative
  metadata; `Host::load` needs a Rust value.
- **The gate is not a floor**: a report with zero citations still returns `PASS`.
- **Swarm / subagents** are a `busy` flag and two manifest rows.
- **The key still lives in `settings.json`** (the OS keyring is ROADMAP v0.8).

**The next honest step is #1: load the host's plugin set from the registry plan,
then move the tools onto a seam.**
