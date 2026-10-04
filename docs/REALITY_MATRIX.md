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
tested but never called by the product) · `UNPROVEN`.

---

## A. The composition claim

| Claim | Where it is claimed | What the code does | Production path | Proven by | Verdict | Decision |
|---|---|---|---|---|---|---|
| "Everything is a plugin" | `README.md`, `docs/ARCHITECTURE.md` | `src-tauri/src/commands.rs:296` builds `EntityRuntime { db, sink, http }` directly; `grep -rn "Host::new\|Host::sealed" crates src-tauri` returns **zero non-test hits**; `src-tauri/src/tool_bridge.rs:127` builds `FsToolHost::new()` directly | A mission never touches `Host`, `Plugin`, `PluginCtx` or a seam | `grep` above; `commands.rs:296`, `tool_bridge.rs:127` | **FALSE** | Downgrade the claim **and** wire the runtime, or delete the claim |
| Twelve plugin slots | `crates/vara-core/src/plugin.rs` | `seams.rs` defines five seams (`tools.read`, `tools.ctx`, `tools.host`, `brain`, `log`). Seven slots (`memory`, `interface`, `channel`, `goal_engine`, `subagent`, `mcp`, `skill`) have no runtime seam and no implementation | none | compare `SLOTS` in `plugin.rs` against `seams` in `seams.rs` | **PARTIAL** | Each slot needs a seam or a "metadata only" label |
| A folder with `plugin.toml` adds a capability | `docs/PLUGIN_GUIDE.md` | Nothing constructs an `Arc<dyn Plugin>` from data — no `libloading`, no wasm, no interpreter. `Host::load` requires a Rust value | none | `grep -rn "libloading\|wasmtime\|dlopen" crates` → zero | **FALSE** | Either implement a real boundary (WASM/subprocess) or say "declarative metadata" |
| Plugin enable/disable controls capabilities | `src/lib/components/PluginsView.svelte` | The panel writes `plugins.json`; the mission runtime never reads it | none | `grep -rn "plugins.json\|PluginRegistry" src-tauri/src/commands.rs src-tauri/src/lib.rs` → only `plugin_bridge` | **FALSE** | Wire it, or label the panel "not yet enforced" |
| "Nothing built-in is privileged" | `docs/ARCHITECTURE.md` | The shipped plugins are compiled-in Rust types in `seams.rs` | n/a | `seams.rs` | **FALSE** | Reword: "built-ins use the same registration path a third-party plugin would" |
| Profiles/bundles compose a run | `crates/vara-core/src/profile.rs` | `grep -rn "shipped_catalog" crates src-tauri` → only `profile.rs` and its own tests. Ids there (`store.sqlite`, `policy.gate`, `interface.tauri`) exist in **no** manifest | none | `grep` above | **DEAD CODE** | Delete `profile.rs` or make the shipped manifests the single vocabulary |
| Goals know when to stop | `README.md` | `goals.rs` (982 lines, 19 tests) has **zero** non-test references | none | `Select-String "goals::\|GoalBoard\|GoalDecision"` outside `goals.rs` → empty | **DEAD CODE** | Wire into the mission runner, or remove from the product surface |
| Silent initiative (heartbeat v2) | `crates/vara-core/src/heartbeat.rs` | `heartbeat.rs` (519 lines, 12 tests) has zero non-test callers. The live loop calls `entity.rs:900` — the **v1** reflection that v2 was written to replace | `src-tauri/src/lib.rs:249 → entity.rs:900` | `Select-String "heartbeat::"` outside `heartbeat.rs` → empty; `lib.rs:249` | **DEAD CODE** | Wire v2 and delete v1, or stop shipping the ceilings |

## B. The provenance claim

| Claim | Where it is claimed | What the code does | Production path | Proven by | Verdict | Decision |
|---|---|---|---|---|---|---|
| "No report leaves the entity unless every citation resolves to a source that was actually retrieved" | `README.md` | `entity.rs:243-248` pushes search hits with `fetched: false` into the same ledger; `entity.rs:451` reduces the ledger to `Vec<String>`; the token `fetched` does not occur in `provenance.rs` | yes — `run_mission` | `entity.rs:243-248`, `:451`; `grep fetched provenance.rs` → empty | **FALSE** | C1 must require `sources.fetched = 1` |
| C3 quote grounding gates reports | `README.md`, `CHANGELOG.md` | C3 **is** wired (`entity.rs:492`, `:543` — correcting an earlier claim of mine), but the verdict is `c1 && c2 && c3 != Some(false)` (`provenance.rs:501`), and `c3 = None` whenever no snapshot text exists | yes | `entity.rs:492`; `provenance.rs:501`; `provenance_gate_tests.rs:264,277` | **PARTIAL** | `c3 = None` must not read as PASS |
| A receipt accompanies every verdict | `docs/PROVENANCE.md:158`, `docs/ARCHITECTURE.md:121` | The `GateReceipt` (C1/C2/C3, `n_claims_evaluable`, Wilson CIs, `not_evaluable_reason`) is written to `reports.receipt_json` and rendered **nowhere**: `ReportRecord` (`src/lib/types.ts:63-73`) has no such field | stored, never displayed | `git grep -n "receipt_json" -- src` → empty | **FALSE** | Add the field and render it |
| Failed reports are not presented as success | — | `entity.rs:571` sets `"completed"` unconditionally on reaching the end; `commands.rs:188-194` prints `"Mission complete ✅ — report ready (provenance FAIL…)"` | yes | `entity.rs:571`; `commands.rs:188-194` | **FALSE** | Completion must depend on the verdict |
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
| "Paths are confined to your owner folder" | `SECURITY.md:42` | `confine_to_root` is documented **lexical only**; `grep -rn "canonicalize\|symlink_metadata\|read_link" crates/vara-core/src` → **zero**. A junction inside a root was read **and written** through, live | auditor probe (junction created with `mklink /J`; read returned the canary; write reported `true`) | **FALSE** | Resolve reparse points on every access |
| The key cannot reach a command | `AGENTS.md` invariant 3 | `commands.rs:1559` calls only `CommandPolicy::default().review(&target)`; `guard_path`/`FORBIDDEN_PATH_MARKERS` are **never consulted** in `sys_execute`. `settings.json` is inside the default root and `type` is not denied | `commands.rs:1559`; `grep guard_path src-tauri/src/commands.rs` → zero | **FALSE** | Apply the forbidden-path floor to argv; move the key to the OS credential store |
| `run` is argv-only and reviewed | `SECURITY.md` | True for tokenization (`|`, `&&`, `;`, `>`, backtick, `$`, `^` all refused) — but `msiexec /i <url>`, `curl`, `node --eval`, `python -cprint(1)`, `php -r`, `deno eval` all classify `Ok(())` because the inline-code rule is exact-string equality with `arg.len() <= 3` | auditor probe table; `exec_policy.rs:290-295` | **PARTIAL** | Allow-list, or match flags by prefix |
| "The model may only propose" | `AGENTS.md` invariant 4 | No taint mechanism exists: a tool result gets a DATA wrapper, but fetched page text enters the model context through memory with no source framing, and no consequential action checks where its arguments came from | `tool_loop.rs:210-224`; `chat.rs:101-111` | **PARTIAL** | Taint untrusted content, or state the limit plainly |
| No SSRF surface | — | `fetch_page` filters only on the `http` prefix; reqwest follows up to 10 redirects; no host allow-list and no private-IP/loopback/`169.254.169.254` block (`tools.rs:19-28`, `:89-92`). Not reachable from model output today (single caller: the mission loop), so this is a latent hole, not an active exploit | `tools.rs:19-28`, `:89-92` | **TRUE (no protection exists)** — exploitation **UNPROVEN** today | Block internal addresses before any fetch tool becomes model-reachable |
| Proposals cannot be replayed | — | **Holds.** First claim `Ok(true)`, second `Ok(false)`, post-finish `false`, expired `false`; TTL enforced on decide and claim; settings re-checked at execute | auditor probe | **SUPPORTED** | — |
| Secrets are scrubbed from child environments | `AGENTS.md` invariant 3 | **Holds.** Allow-list + secret deny-list; 117 → 26 variables; `*_API_KEY`, `*_TOKEN`, `VARA_PROVIDER_*`, `AWS_*`, `GITHUB_TOKEN`, `HTTP_PROXY` all dropped | auditor probe | **SUPPORTED** | — |
| The updater is signed | `README.md` | **Holds.** minisign public key pinned in `tauri.conf.json`, verified by `tauri-plugin-updater` before install; no version floor (a leaked private key could serve a downgrade) | `tauri.conf.json:52-59`; `release.yml` | **SUPPORTED** | Add a version floor |

## D. The entity claim

| Claim | Where it is claimed | What the code does | Proven by | Verdict | Decision |
|---|---|---|---|---|---|
| Persistent entity | `README.md` | A mission killed mid-flight leaves a `running` row; **no startup recovery exists** (`grep -rn "resume\|recover" src-tauri/src/lib.rs` → none); the retrieval ledger lives in memory (`entity.rs: let mut ledger: Vec<RetrievedSource>`) | `lib.rs` startup sequence; `entity.rs` | **FALSE** | Durable state machine + recovery sweep |
| Autonomous organization / swarm / subagents | `README.md`, `plugins/swarm/plugin.toml` | One global `busy: AtomicBool`; a second mission is refused. `subagents` and `swarm` are manifest rows with no implementation | `commands.rs` busy gate | **FALSE** | Build a supervisor, or remove the claim |
| Memory | `README.md` | SQLite + FTS5, keyword `ANY` matching, no recency/salience/type separation, no TTL | `db.rs`, `chat.rs:130-138` | **PARTIAL** — "durable text retrieval", not memory | Rename the claim |
| Headless / TUI | `README.md` | `crates/vara-tui` exists and runs the same core (verified: it called `system_info` on this machine). There is **no** daemon/RPC binary, and `profile.rs`'s `interface.rpc` is dead | `crates/vara-tui/src/main.rs`; `crates/` listing | **PARTIAL** | Claim TUI only; delete `interface.rpc` |
| Real tools | `README.md` | Six read tools work end to end through `vara-tui` (verified: `[router] call system_info`, result `windows · 16 cores · host WISSEM`) — but `disk_usage` returns "drive totals are not available on this platform yet" on this machine while its test asserts `free_bytes == 700` against `MockToolHost` | live run; `tools_local.rs:906` | **PARTIAL** | Implement volume totals; stop counting a mock as proof |
| Chat answers are complete | — | `llm.rs:169-179` splits **each network chunk** on `\n` with no carry buffer, so a `data:` line crossing a chunk boundary is silently dropped — in every streaming reply | `llm.rs:169-179` | **FALSE** | Keep a carry buffer between chunks |

## E. The repository claim

| Claim | Where it is claimed | What the code does | Proven by | Verdict | Decision |
|---|---|---|---|---|---|
| The shipped product contains its plugins | — | `tauri.conf.json:35` lists `["../skills/vara/**/*"]` only — `plugins/` is **not bundled**, so an installed build shows an empty plugin panel | `tauri.conf.json:35` | **FALSE** | Add `../plugins/**/*` to resources |
| The theme/persona slots ship assets | `plugins/themes`, `plugins/personas` | Both folders contain only a manifest; `vara-plugins hash plugins/themes` prints the SHA-256 of the **empty string** | `vara-plugins hash` | **FALSE** | Ship assets inside the slot folders |
| Version is consistent | `AGENTS.md` | **Holds.** `0.7.0` in all seven manifests; `check-consistency.mjs --write` reports "version 0.7.0 agreed in all 7 manifests" | the script | **SUPPORTED** | — |
| Documentation matches the product | — | `SECURITY.md` supports `0.6.x`; `CHANGELOG.md` still says `(unreleased)` after publication; `README.ar.md` is frozen at v0.5.0; README/ARCHITECTURE say fifteen plugins, the release said twenty | `check-consistency.mjs` (4 problems on first run) | **FALSE** | Sync every document |
| Documentation drift is caught automatically | — | `scripts/check-consistency.mjs` exists but is referenced by **neither** `package.json` nor any workflow | `grep -rn "check-consistency" package.json .github` → empty | **FALSE** | Wire it into CI or delete it |
| Tests prove the product works | `README.md` | ~62 of 227 Rust tests cover modules with **no production caller** (`host.rs`, `seams.rs`, `profile.rs`, `goals.rs`, `heartbeat.rs`); `entity.rs` — the mission runner — has **two** tests, both on pure helpers; `commands.rs` (~1700 lines) has none | `Select-String "goals::\|heartbeat::"` outside their files → empty; `entity.rs` test list | **PARTIAL** | Move weight from unit to system tests |
| CI protects `main` | — | **Holds** (and this accusation was **FALSE**): `.github/workflows/ci.yml:5` reads `branches: [main]`; run `37204776351` on `main` completed **success** with four green jobs | `ci.yml:5`; `gh run view 37204776351` | **SUPPORTED** | Add clippy, `--workspace`, secret scan |
| The repository is clean of audit artefacts | — | **Holds** (and this accusation was **FALSE**): no stray `%SystemDrive%` directory, no junction left behind, no probe `eprintln` in `commands.rs`, and `git status` shows only my own staged theme/persona files | `git status --short`; `Select-String "audit-probe\|eprintln!" src-tauri/src/commands.rs` → empty | **SUPPORTED** | — |

---

## What this matrix is for

It is the contract between the repository and its own README. A row may only move
to `SUPPORTED` when a command demonstrates the behaviour **on the production
path**, and the command is recorded in the row. Until then, the claim in the
product text is wrong and must be either fixed or removed.

**The next document is the fix order, and it starts from the rows marked FALSE.**
