# AGENTS.md — Operating contract for any agent working on Vara

Vara is a persistent autonomous entity, not a chatbot wrapper. These rules are
short, binding, and enforce the two promises the product makes: **evidence
before claims** and **the conversation is the entity**.

## Non-negotiable invariants

1. **Provenance gate is a floor, not a knob.** `crates/vara-core/src/provenance.rs`
   (C1: every cited URL was actually retrieved; C2: every `[n]` resolves to the
   source list) must never be weakened to make a failing report pass. Fix the
   retrieval or declare an explicit `Gap:`.
2. **SQLite discipline.** WAL is always on. Migrations are append-only
   (`MIGRATIONS` in `crates/vara-core/src/db.rs`): never edit an applied
   migration, add `v(N+1)`. The single-connection Mutex writer stays.
3. **Secrets never enter prompts, logs, or the repo.** API keys live in the
   local settings file or the `VARA_PROVIDER_*` environment variables
   (`VARA_PROVIDER_API_KEY` / `VARA_PROVIDER_BASE_URL` / `VARA_PROVIDER_MODEL`;
   env > file for the running app, re-applied after every UI save). An
   environment-provided key must **never** be written to disk: persist
   `Settings::without_api_key()`, and hand the webview `Settings::for_webview()`
   (it learns `has_api_key` / `api_key_source`, never the key itself). Spawned
   commands get a scrubbed environment (`exec_policy::child_env`), so a command
   the entity runs cannot read the owner's key. Never hardcode keys in code,
   tests, or CI logs.
4. **The policy engine gates every OS action.** The model can only *propose*
   actions via the `[[sys]]` protocol (`open_url` / `open_path` / `run` /
   `screenshot` / `computer_use`); execution happens in the shell
   (`sys_execute` / `run_computer_use`) after the autonomy settings allow it.
   Proposals are **minted by the core from model output**
   (`exec_policy::plan_proposal`) into `action_proposals` rows; the webview
   receives ids, never payloads, and `sys_execute(proposal_id)` claims an
   approved, unexpired, single-use row and executes the target stored in that
   row. `run`, `screenshot` and `computer_use` always show an explicit approval
   card; screen capture and computer use ship OFF (`allow_screenshots: false`,
   `allow_computer_use: false`), commands ship OFF (`run_commands: false`), and
   destructive L2 ops additionally require `computer_use_allow_close`. `run` is
   **argv-only** — no shell, no composition, secrets scrubbed — through
   `exec_policy::CommandPolicy`. The ActLoop structurally refuses ungrounded
   coordinates and dry-runs destructive ops — never weaken those guards to
   "make a task finish". Never add a code path where model output executes
   directly.
5. **Chat-first.** Missions are born inside a conversation
   (`start_mission_in_conversation` / auto-start on proposal) and their
   progress, receipts, and reports flow back into the same thread. Do not
   reintroduce separate launcher surfaces.
6. **Protocol parsing is tolerant.** Models mangle markers
   (`[mission]…{MISSION_CLOSE}` variants). Extraction must accept known
   variants and the UI must never leak raw protocol text. Both implementations —
   core (Rust, `crates/vara-core/src/chat.rs`) and webview
   (`src/lib/protocol.ts`, re-exported by `state.svelte.ts`) — must stay in sync:
   they are locked together by the shared fixture
   `tests/fixtures/protocol_cases.json`, read by both
   `crates/vara-core/tests/protocol_parity.rs` and `tests/protocol.test.ts`.
   Change a parser, change the fixture, run both suites.
7. **Token budgets are binding.** Mission budget/steps clamps live in types +
   commands; do not loosen them to "make it finish".

## Definition of done (run before claiming completion)

```sh
cargo test -p vara-core          # all green
cargo fmt --all -- --check
npm install && npm run check     # svelte-check: 0 errors
npm run build                    # vite build succeeds
```

Full desktop compile (`cargo check -p vara`) requires GTK/WebKit on Linux and
is verified by CI on `windows-latest`. If you only changed `vara-core`, say so
honestly instead of claiming a full desktop build.

## Versioning & releases

- Bump version together in: root `Cargo.toml`, both crate manifests,
  `package.json`, `src-tauri/tauri.conf.json`.
- Releases are tags (`vX.Y.Z`); `.github/workflows/release.yml` builds the
  Windows NSIS installer, signs updater artifacts (minisign, key in GitHub
  Actions secrets), and publishes `latest.json` for the OTA channel
  (`tauri-plugin-updater`).

## Skills

Project-specific skills for agents live in `skills/vara/`. Read the relevant
one before touching: provenance, DB/migrations, secrets/policy, experiment
protocol, or definition-of-done.
