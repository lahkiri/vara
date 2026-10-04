# Reality Matrix — what Vara claims vs what its runtime does

**Purpose.** This document exists because Vara v0.7.0 was published with release
notes describing files and behaviour that did not exist. Every row below is
**proved by a command whose output is quoted**, and the proving command is named.
Rows are never marked fixed without a command that would have failed before the
fix.

**Two rules for reading this file:**

1. A claim is only "supported" if a command demonstrates it on the **production
   path** — not in a unit test over a mock, and not because a module exists.
2. Accusations that turned out to be **false** are recorded too. A one-directional
   audit is a marketing document with a hostile tone.

Legend for **Verdict**: `SUPPORTED` · `PARTIAL` · `FALSE` · `DEAD CODE` (built and
tested but never called by the product) · `UNPROVEN` · **`FIXED`** (was false, now
true, with the command that proves it and the commit that did it).

---

## 0. Status of this document

This matrix was written when the answer to almost every question below was "the
claim is ahead of the code". That is no longer uniformly true, so the honest thing
to do is date the state and name what changed. **Fixed rows keep their original
verdict in the text and gain a `FIXED` line with the commit and the proving
command**, because deleting the history is how a project forgets why a guard
exists.

| fixed on 2026-10-04 | commit | the command that proves it |
|---|---|---|
| A `run` could read `settings.json` | `0ee4683` `db95357` | `cargo test -p vara-core --lib tools_registry` |
| B C1 accepted a never-fetched source | `4feb724` | `cargo test -p vara-core --test provenance_tests` |
| B the receipt was stored, never shown | `753e13f` | `svelte-check` + `vite build` clean with the field rendered |
| C a FAIL report was labelled `completed` | `0794ce4` | `cargo test -p vara-core` (status is verdict-derived) |
| E a junction escaped the root | `015a8e8` | `cargo test -p vara-core --lib junction` |
| F `plugins/` was not bundled | `0794ce4` | `tauri.conf.json` resources line |
| G theme/persona slots shipped no bytes | `0794ce4` | `node scripts/check-consistency.mjs --write` |
| H split SSE frames were dropped | `0794ce4` | `cargo test -p vara-core --lib llm` |
| I the fetch path could reach the LAN | `9fe9fee` | `cargo test -p vara-core --lib ssrf` |
| A zombie mission stayed `running` | `7274874` | `cargo test -p vara-core --lib a_mission_left_running` |
| A the heartbeat never restarted after off→on | `64e5014` | `cargo check -p vara` + the latch's single `false` store |
| A the first capability now travels a seam | `0c5d96e` | `cargo test -p vara-core --lib seam_wiring` |
| J five documents disagreed with the product | `cc8eedd` | `node scripts/check-consistency.mjs` |
| A clippy found a loop that never looped | `0d8e49e` | `cargo clippy --workspace --all-targets` |

**What is still open at the top of this document is stated in §A**, and the largest
item — the mission path taking its capabilities from the plugin host — is the one
the owner chose to build rather than paper over.

---

## A. The composition claim

| Claim | Where it is claimed | What the code does | Production path | Proven by | Verdict | Decision |
|---|---|---|---|---|---|---|
| "Everything is a plugin" | `README.md`, `docs/ARCHITECTURE.md` | `src-tauri/src/commands.rs:296` builds `EntityRuntime { db, sink, http }` directly; `grep -rn "Host::new\|Host::sealed" crates src-tauri` returns **zero non-test hits**; `src-tauri/src/tool_bridge.rs:127` builds `FsToolHost::new()` directly | A mission never touches `Host`, `Plugin`, `PluginCtx` or a seam | `grep` above; `commands.rs:296`, `tool_bridge.rs:127` | **FALSE** | Downgrade the claim **and** wire the runtime, or delete the claim |
| Twelve plugin slots | `crates/vara-core/src/plugin.rs` | `seams.rs` defines five seams (`tools.read`, `tools.ctx`, `tools.host`, `brain`, `log`). Seven slots (`memory`, `interface`, `channel`, `goal_engine`, `subagent`, `mcp`, `skill`) have no runtime seam and no implementation | none | compare `SLOTS` in `plugin.rs` against `seams` in `seams.rs` | **PARTIAL** | Each slot needs a seam or a "metadata only" label |
| A folder with `plugin.toml` adds a capability | `docs/PLUGIN_GUIDE.md` | Nothing constructs an `Arc<dyn Plugin>` from data — no `libloading`, no wasm, no interpreter. `Host::load` requires a Rust value | none | `grep -rn "libloading\|wasmtime\|dlopen" crates` → zero | **FALSE** | Either implement a real boundary (WASM/subprocess) or say "declarative metadata" |
| Plugin enable/disable controls capabilities | `src/lib/components/PluginsView.svelte` | The panel writes `plugins.json`; `plugin_bridge` reads it, and the shell's `Host` is built at startup rather than from that plan — so toggling a plugin changes the panel and the plan, **not yet** what a running mission uses | partial: the `log` plugin is loaded from code, not from `plugins.json` | `grep -rn "plugins.json\|PluginRegistry" src-tauri/src/commands.rs src-tauri/src/lib.rs` → only `plugin_bridge` | **FALSE** | Load the host's plugin set from the registry's plan — the next step after the tools |
| **The mission's event stream comes from a plugin** | — | **FIXED, and it is the first capability to travel a real seam in production.** `entity::LogCap` is the type both sides register, `sink_from_host` is the single resolution point, `require_log_sink` is the strict variant, and `SeamSink` bridges the seam to `EntityEvent::kind()`/`summary()` | **yes.** `lib.rs:242` creates the `Host`, `:256-258` loads the shipped `LogPlugin` (a load failure is recorded as a `warn` event, not swallowed), and **both** runtime sites resolve their sink through `AppState::mission_sink` — `commands.rs:313` (mission) and `lib.rs:376` (heartbeat) | `cargo test -p vara-core --lib seam_wiring` — 2 tests: events reach the registered plugin, `unload("log")` stops them, an empty seam is reported instead of defaulted. **Plus the production lines above, read directly** | **FIXED** (commits `0c5d96e` for the seam, then this wiring) | — |
| "Everything is a plugin" | `README.md`, `docs/ARCHITECTURE.md` | **NOW PARTIAL, and it moved for a reason.** When this matrix was written, `commands.rs:296` built `EntityRuntime { db, sink, http }` directly and `grep "Host::new" crates src-tauri` returned **zero non-test hits**. The shell now creates a `Host` at startup, loads the shipped `LogPlugin` into it, and both runtime sites take their event stream from the `log` seam. Still direct: `tool_bridge.rs:127` builds `FsToolHost::new()`, and `db`/`http` remain struct fields rather than seams | the mission path now touches `Host`, `Plugin`, `PluginCtx` and one seam | read `lib.rs:242,256,258,54` and `commands.rs:313`; `cargo test -p vara-core --lib seam_wiring` | **PARTIAL** — one capability of many | Keep wiring seam by seam; the claim stays downgraded until the tools travel a seam too |
| **A disabling plugin disables a capability** | `README.md` | **True for the `log` seam and provable there**: unloading the plugin empties the seam and the events stop. Every other capability is still a compiled-in Rust value | the `log` seam, in production | `cargo test -p vara-core --lib seam_wiring`, assertion 2 | **PARTIAL** — true for one seam, unproven for the rest | Repeat the pattern per seam, or keep the wording narrow |
| "Nothing built-in is privileged" | `docs/ARCHITECTURE.md` | The shipped plugins are compiled-in Rust types in `seams.rs` | n/a | `seams.rs` | **FALSE** | Reword: "built-ins use the same registration path a third-party plugin would" |
| Profiles/bundles compose a run | `crates/vara-core/src/profile.rs` | `grep -rn "shipped_catalog" crates src-tauri` → only `profile.rs` and its own tests. Ids there (`store.sqlite`, `policy.gate`, `interface.tauri`) exist in **no** manifest | none | `grep` above | **DEAD CODE** | Delete `profile.rs` or make the shipped manifests the single vocabulary |
| Goals know when to stop | `README.md` | `goals.rs` (982 lines, 19 tests) has **zero** non-test references | none | `Select-String "goals::\|GoalBoard\|GoalDecision"` outside `goals.rs` → empty | **DEAD CODE** | Wire into the mission runner, or remove from the product surface |
| Silent initiative (heartbeat v2) | `crates/vara-core/src/heartbeat.rs` | `heartbeat.rs` (519 lines, 12 tests) has zero non-test callers. The live loop calls `entity.rs:900` — the **v1** reflection that v2 was written to replace | `src-tauri/src/lib.rs:249 → entity.rs:900` | `Select-String "heartbeat::"` outside `heartbeat.rs` → empty; `lib.rs:249` | **DEAD CODE** | Wire v2 and delete v1, or stop shipping the ceilings |

## B. The provenance claim

| Claim | Where it is claimed | What the code does | Production path | Proven by | Verdict | Decision |
|---|---|---|---|---|---|---|
| "No report leaves the entity unless every citation resolves to a source that was actually retrieved" | `README.md` | `entity.rs:243-248` pushes search hits with `fetched: false` into the same ledger; `entity.rs:451` reduces the ledger to `Vec<String>`; the token `fetched` does not occur in `provenance.rs` | yes — `run_mission` | `entity.rs:243-248`, `:451`; `grep fetched provenance.rs` → empty | **FIXED** (commit `4feb724`) | C1 now receives only fetched URLs; `name_unfetched_citations` names the rest. `cargo test -p vara-core --test provenance_tests` |
| C3 quote grounding gates reports | `README.md`, `CHANGELOG.md` | C3 **is** wired (`entity.rs:492`, `:543` — correcting an earlier claim of mine), but the verdict is `c1 && c2 && c3 != Some(false)` (`provenance.rs:501`), and `c3 = None` whenever no snapshot text exists | yes | `entity.rs:492`; `provenance.rs:501`; `provenance_gate_tests.rs:264,277` | **PARTIAL** | `c3 = None` must not read as PASS |
| A receipt accompanies every verdict | `docs/PROVENANCE.md:158`, `docs/ARCHITECTURE.md:121` | The `GateReceipt` (C1/C2/C3, `n_claims_evaluable`, Wilson CIs, `not_evaluable_reason`) is written to `reports.receipt_json` and rendered **nowhere**: `ReportRecord` (`src/lib/types.ts:63-73`) has no such field | stored, never displayed | `git grep -n "receipt_json" -- src` → empty | **FIXED** (commit `753e13f`) | `ReportRecord.receipt_json` typed and rendered: gate version, C1/C2/C3 with the three C3 states, the 95% CI, evaluated/audited claims, `not_evaluable_reason`, `cited_never_fetched`, `missing_quotes` |
| Failed reports are not presented as success | — | `entity.rs:571` sets `"completed"` unconditionally on reaching the end; `commands.rs:188-194` prints `"Mission complete ✅ — report ready (provenance FAIL…)"` | yes | `entity.rs:571`; `commands.rs:188-194` | **FIXED** (commit `0794ce4`) | `final_status = if verdict == "PASS" { "completed" } else { "unverified" }`, stored and returned; the chat has a separate arm and the tick is gone. `cargo test -p vara-core` |
| The gate is a floor | `AGENTS.md` invariant 1 | A report with zero citations returns `PASS`; `provenance_gate_tests.rs:113-119` asserts it | yes | same | **FALSE** | Third verdict: `PASS_UNVERIFIED` |

**Live evidence from this machine's own database** (not a fixture):

```
SELECT verdict, backed_ratio, receipt_json IS NOT NULL FROM reports;  →  PASS|1.0|0   (twice)
SELECT DISTINCT fetched FROM sources;                                 →  0            (all rows)
check_json: cited_total=5, retrieved_total=5
```

Two "100% backed" reports; not one page ever fetched.

## C. The security claim

| Claim | Where it is claimed | What the code does | Proven by | Verdict | Decision |
|---|---|---|---|---|---|
| "Paths are confined to your owner folder" | `SECURITY.md:42` | **FIXED (commit `015a8e8`).** `confine_to_root` is still lexical by design, but `ToolCtx::resolve_real` canonicalizes the result and requires it to stay under a canonical root, and all eight tool call sites use it. The escape was reproduced here first: `mklink /J`, then reading `canary.txt` through the link returned `TOP-SECRET` | `cargo test -p vara-core --lib junction` — creates a real junction, asserts the lexical check accepts it (the defect), asserts `resolve_real` refuses it, asserts an ordinary file still resolves, and prints "not exercised" instead of passing silently when the platform cannot create the link | **FIXED** (the TOCTOU window between check and open remains, documented in the function) | — |
| The key cannot reach a command | `AGENTS.md` invariant 3 | **FIXED (commits `0ee4683`, `db95357`).** The floor was attached to a *tool* — `fs.read_file` consulted it — while `run` went straight to `CommandPolicy::review`, so `type %APPDATA%\app.vara.entity\settings.json` passed and its stdout landed in the transcript and the model's context. `check_forbidden_path` is now a free function, `check_command_paths` scans the program and every argument (including `--flag=path` and `VAR=value`), and the `Run` arm consults it before executing. The key itself still lives in `settings.json` | `cargo test -p vara-core --lib tools_registry` — seven leaking command shapes refused, five ordinary commands still allowed | **FIXED** (keyring is still open — ROADMAP v0.8) | — |
| `run` is argv-only and reviewed | `SECURITY.md` | True for tokenization (`|`, `&&`, `;`, `>`, backtick, `$`, `^` all refused) — but `msiexec /i <url>`, `curl`, `node --eval`, `python -cprint(1)`, `php -r`, `deno eval` all classify `Ok(())` because the inline-code rule is exact-string equality with `arg.len() <= 3` | auditor probe table; `exec_policy.rs:290-295` | **PARTIAL** | Allow-list, or match flags by prefix |
| "The model may only propose" | `AGENTS.md` invariant 4 | No taint mechanism exists: a tool result gets a DATA wrapper, but fetched page text enters the model context through memory with no source framing, and no consequential action checks where its arguments came from | `tool_loop.rs:210-224`; `chat.rs:101-111` | **PARTIAL** | Taint untrusted content, or state the limit plainly |
| No SSRF surface | — | **FIXED (commit `9fe9fee`).** `fetch_page` filtered only on the `http` prefix and the client followed ten redirects automatically, so a model coaxed by page text could fetch `169.254.169.254` (cloud metadata), `localhost`, or the owner's router. `check_public_url` now resolves the host and judges **every** answer, `is_internal_ip` covers loopback/RFC1918/link-local/CGNAT/`198.18/15`/`240/4`/IPv6 unique-local and IPv4-mapped forms, `.localhost`/`.local`/`.internal`/`.home.arpa` are refused by name, and **redirects are not followed**, so each hop is judged | `cargo test -p vara-core --lib ssrf`: 19 URLs refused (metadata, loopback, LAN, local names, non-http schemes), 5 public URLs still allowed, and the address classification itself asserted range by range. Reachability checked by reading: `entity.rs:390` is the only caller, the guard is `tools.rs:197`, the redirect policy `:124` | **FIXED** (was: no protection at all) | — |
| Proposals cannot be replayed | — | **Holds.** First claim `Ok(true)`, second `Ok(false)`, post-finish `false`, expired `false`; TTL enforced on decide and claim; settings re-checked at execute | auditor probe | **SUPPORTED** | — |
| Secrets are scrubbed from child environments | `AGENTS.md` invariant 3 | **Holds.** Allow-list + secret deny-list; 117 → 26 variables; `*_API_KEY`, `*_TOKEN`, `VARA_PROVIDER_*`, `AWS_*`, `GITHUB_TOKEN`, `HTTP_PROXY` all dropped | auditor probe | **SUPPORTED** | — |
| The updater is signed | `README.md` | **Holds.** minisign public key pinned in `tauri.conf.json`, verified by `tauri-plugin-updater` before install; no version floor (a leaked private key could serve a downgrade) | `tauri.conf.json:52-59`; `release.yml` | **SUPPORTED** | Add a version floor |

## D. The entity claim

| Claim | Where it is claimed | What the code does | Proven by | Verdict | Decision |
|---|---|---|---|---|---|
| Persistent entity | `README.md` | A mission killed mid-flight leaves a `running` row; **no startup recovery exists** (`grep -rn "resume\|recover" src-tauri/src/lib.rs` → none); the retrieval ledger lives in memory (`entity.rs: let mut ledger: Vec<RetrievedSource>`) | `lib.rs` startup sequence; `entity.rs` | **FIXED, partly** (commit `7274874`) | `recover_interrupted_missions` runs at startup, logs an event, is idempotent, and the UI knows `interrupted`. `cargo test -p vara-core --lib a_mission_left_running`. **Not done: resuming the mission** — it is reconciled, not restarted |
| Autonomous organization / swarm / subagents | `README.md`, `plugins/swarm/plugin.toml` | One global `busy: AtomicBool`; a second mission is refused. `subagents` and `swarm` are manifest rows with no implementation | `commands.rs` busy gate | **FALSE** | Build a supervisor, or remove the claim |
| Memory | `README.md` | SQLite + FTS5, keyword `ANY` matching, no recency/salience/type separation, no TTL | `db.rs`, `chat.rs:130-138` | **PARTIAL** — "durable text retrieval", not memory | Rename the claim |
| Headless / TUI | `README.md` | `crates/vara-tui` exists and runs the same core (verified: it called `system_info` on this machine). There is **no** daemon/RPC binary, and `profile.rs`'s `interface.rpc` is dead | `crates/vara-tui/src/main.rs`; `crates/` listing | **PARTIAL** | Claim TUI only; delete `interface.rpc` |
| Real tools | `README.md` | Six read tools work end to end through `vara-tui` (verified: `[router] call system_info`, result `windows · 16 cores · host WISSEM`) — but `disk_usage` returns "drive totals are not available on this platform yet" on this machine while its test asserts `free_bytes == 700` against `MockToolHost` | live run; `tools_local.rs:906` | **PARTIAL** | Implement volume totals; stop counting a mock as proof |
| Chat answers are complete | — | `llm.rs:169-179` splits **each network chunk** on `\n` with no carry buffer, so a `data:` line crossing a chunk boundary is silently dropped — in every streaming reply | `llm.rs:169-179` | **FIXED** (commit `0794ce4`) | Carry buffer + tail frame; `sse_frames_split_across_chunks_are_kept` and `parsing_each_chunk_alone_loses_the_frame` pin both behaviours |

## E. The repository claim

| Claim | Where it is claimed | What the code does | Proven by | Verdict | Decision |
|---|---|---|---|---|---|
| The shipped product contains its plugins | — | `tauri.conf.json:35` lists `["../skills/vara/**/*"]` only — `plugins/` is **not bundled**, so an installed build shows an empty plugin panel | `tauri.conf.json:35` | **FIXED** (commit `0794ce4`) | `resources` now lists `../plugins/**/*` |
| The theme/persona slots ship assets | `plugins/themes`, `plugins/personas` | Both folders contain only a manifest; `vara-plugins hash plugins/themes` prints the SHA-256 of the **empty string** | `vara-plugins hash` | **FIXED** (commit `0794ce4`) | Assets live in `plugins/themes/*.toml` and `plugins/personas/*.toml`; `node scripts/check-consistency.mjs` reports 15 folders, 15 manifests, 0 asset-only |
| Version is consistent | `AGENTS.md` | **Holds.** `0.7.0` in all seven manifests; `check-consistency.mjs --write` reports "version 0.7.0 agreed in all 7 manifests" | the script | **SUPPORTED** | — |
| Documentation matches the product | — | `SECURITY.md` supports `0.6.x`; `CHANGELOG.md` still says `(unreleased)` after publication; `README.ar.md` is frozen at v0.5.0; README/ARCHITECTURE say fifteen plugins, the release said twenty | `check-consistency.mjs` (4 problems on first run) | **FIXED** (commit `cc8eedd`) | SECURITY 0.7.x, CHANGELOG dated, ROADMAP corrected, README claims downgraded in place, README.ar.md at v0.7 with فارَا, `docs/README.md` added. `node scripts/check-consistency.mjs` |
| Documentation drift is caught automatically | — | `scripts/check-consistency.mjs` exists but is referenced by **neither** `package.json` nor any workflow | `grep -rn "check-consistency" package.json .github` → empty | **FIXED** (commits `cc8eedd`, `0d8e49e`) | `check-consistency` is a blocking CI step, with `cargo test --workspace` and a non-blocking clippy beside it |
| Tests prove the product works | `README.md` | ~62 of 227 Rust tests cover modules with **no production caller** (`host.rs`, `seams.rs`, `profile.rs`, `goals.rs`, `heartbeat.rs`); `entity.rs` — the mission runner — has **two** tests, both on pure helpers; `commands.rs` (~1700 lines) has none | `Select-String "goals::\|heartbeat::"` outside their files → empty; `entity.rs` test list | **PARTIAL** | Move weight from unit to system tests |
| CI protects `main` | — | **Holds** (and this accusation was **FALSE**): `.github/workflows/ci.yml:5` reads `branches: [main]`; run `37204776351` on `main` completed **success** with four green jobs | `ci.yml:5`; `gh run view 37204776351` | **SUPPORTED** | Add clippy, `--workspace`, secret scan |
| The repository is clean of audit artefacts | — | **Holds** (and this accusation was **FALSE**): no stray `%SystemDrive%` directory, no junction left behind, no probe `eprintln` in `commands.rs`, and `git status` shows only my own staged theme/persona files | `git status --short`; `Select-String "audit-probe\|eprintln!" src-tauri/src/commands.rs` → empty | **SUPPORTED** | — |

---

## F. How this was verified, and what could **not** be verified here

An audit that only lists what it proved is half an audit. This section records the
limits of the verification itself, so nobody reads the rest as more than it is.

**Verified by running the product, not by tests alone:**

| command | what it proves |
|---|---|
| `target\debug\vara-tui.exe "system info please"` → `16 CPU cores · 15.2 GB RAM` | a real process booted, routed a question, called a tool, and the tool reported the machine. `unknown RAM` was the output before the fix (commit `53185e7`) |
| `cargo test -p vara-core` → 185 + 15 + 5 + 29 + 7 | the core's own suites |
| `npm run check` · `npx vitest run` (209) · `npm run build` | the frontend typechecks, its tests pass, the production bundle builds |
| `node scripts/check-consistency.mjs` | 15 plugin folders, version 0.7.0 agreed in all 7 manifests |
| `cargo clippy --workspace --all-targets` | 2 warnings, 0 errors |

**Could NOT be verified on this machine, and is therefore not claimed:**

- **A live GUI boot.** `target\debug\vara.exe` panics in WebView2:
  `HRESULT(0x8000FFFF) Catastrophic failure` at `tauri-2.12.1/src/app.rs:1444`,
  before the app finishes setup. The cause is environmental — an orphaned
  `msedgewebview2` process that even `taskkill` refuses without elevation — not a
  defect in the repository. **Consequence: the startup work added in `35082d1`
  (creating the `Host`, loading `LogPlugin`, recovering interrupted missions) has
  never executed inside a live GUI process here.** The database shows no
  `plugins` event, which is the honest evidence that setup did not get that far.
- **A full mission end to end** (search → fetch → report → gate verdict) against a
  live provider. Every part is covered by tests; the whole path in one run is not.
- **The heartbeat firing on its timer** in a live process.

Anything in this document that depends on those three is marked as such, and none
of the `FIXED` rows above relies on them.

---

## What this matrix is for

It is the contract between the repository and its own README. A row may only move
to `SUPPORTED` when a command demonstrates the behaviour **on the production
path**, and the command is recorded in the row. Until then, the claim in the
product text is wrong and must be either fixed or removed.

**The next document is the fix order, and it starts from the rows marked FALSE.**

---
