# Open questions — decisions the coding agent must not take alone

Format: question · why it blocks · my recommendation · what I do meanwhile.
`ALIVE_V2_SPEC.md` §0.5 requires this file; unanswered questions never stall a task.

---

## Q-01 — Which model endpoint ships as the default?

**Blocked work:** disclosure copy, model profiles (spec §7.8), and how far "local-first" can be claimed.
**Facts:** the shipped default in `types.rs::ProviderConfig` is `https://ktai.koyeb.app/v1` — a **third-party proxy**, not a local runtime. Everything Vara reads (file excerpts, memory snippets, screenshots if enabled) transits it.
**My recommendation:** keep OpenAI-compatible support, but make the **first-run default a local preset** (Ollama / LM Studio on `127.0.0.1`), mark remote endpoints with a visible "leaves this machine" badge, and keep the proxy as an opt-in preset.
**Meanwhile:** the disclosure text and the remote/local badge in the model profile are written regardless (they are correct either way).

## Q-02 — Is `smart` allowed to write without asking?

**Blocked work:** spec §7.2 (`Wr` = allow in smart) and S-03.
**Risk:** a wrong bulk move is annoying, not fatal — *if* the undo journal exists and delete goes to the Recycle Bin.
**My recommendation:** yes for `Wr` (journaled + undo + recycle bin), never for `Wd` (permanent delete/overwrite). But this is the owner's risk appetite, not mine.
**Meanwhile:** I build the journal and the `Wr`/`Wd` split so the switch is a one-line policy change.

## Q-03 — Should the old reflection notes be deleted or archived?

**Blocked work:** T-001 migration.
**My recommendation:** never delete. Collapse them into `memory/<date>.md` daily logs, keep a one-time `.bak` export, and leave a single note explaining the migration so the owner can undo it.
**Meanwhile:** that is what I implement unless told otherwise.

## Q-04 — Who is allowed to read `MEMORY.md`?

**Blocked work:** spec §7.6 ("loaded only in private sessions") presumes a notion of non-private sessions that does not exist yet.
**My recommendation:** treat *all* local sessions as private for now, and gate only **export** (reports, shared artifacts) — memory is never included in a report unless a claim cites it as a source.
**Meanwhile:** I follow the recommendation and keep the rule in one place.

## Q-05 — Competition deadline and scope

**Blocked work:** how many sprints are realistic; whether U-09/U-10/U-12 (onboarding, visual pass, demo build) fit.
**My recommendation:** if the deadline is under two weeks, cut to **T-001 + T-003 + U-01** (silent heartbeat, local tools, Today) — those three change the first two minutes; the rest is depth.
**Meanwhile:** working P0 in order.

## Q-06 — Search endpoint is broken (tooling, not product)

`web_search` returns **HTTP 401** in this environment; only `web_fetch` on known URLs works. This limits discovery of new sources (the spec's §2 patterns were taken on trust and only partly re-verified).
**My recommendation:** point the harness search at a working endpoint, or accept that research is "verify what we name" rather than "discover what exists".
**Meanwhile:** every external claim is labelled in `ALIVE_PACK.md §6` as verified / not verified.

## Q-07 — Where does the companion overlay live?

**Blocked work:** U-02. Spec §6.3 asks for a transparent, always-on-top, click-through-except-avatar window; the Tauri APIs for that on Windows need a real spike (multiple windows + per-region click-through + no focus stealing).
**My recommendation:** spike it in a branch first; if click-through is unreliable on Windows 11, ship **tray states + toast with action** and treat the overlay as an enhancement, not the plan of record.
**Meanwhile:** U-01 (Today) carries presence on its own.

## Q-08 — The desktop window cannot start until this machine is restarted

**Blocked work:** any visual verification (the dark titlebar, U-01/U-05/U-06, screenshots of a real chat).
**Facts, all from measurement rather than inference:** the app **now builds** — `cargo check -p vara` finishes in 54 s on MSVC, and `npm run tauri dev` compiled 467 crates in 2 m 28 s and launched `target\debug\vara.exe`. It then fails at `tauri::app::setup` with `WebView2 error: WindowsError(0x8000FFFF)` (and, with the profile data present, `0x800700AA`). Ruled out: `decorations` (fails identically with the default title bar), a locked profile directory (renaming `EBWebView` succeeds), stale processes (none running), a missing runtime (154.0.4258.48 is installed and its files are present).
**Likely cause, and it is mine:** earlier in this session I force-killed `msedgewebview2` children with `Stop-Process -Force`. A force-killed WebView2 environment stays wedged system-wide until Windows restarts.
**My recommendation:** restart Windows, then re-run `npm run tauri dev`; if it still fails, repair the WebView2 runtime (Settings → Apps → Microsoft Edge WebView2 Runtime → Modify → Repair). Nothing in the product depends on this: `vara-tui` runs the same entity end to end today.
**Meanwhile:** every interface-independent part of the plan continues, verified through `vara-tui` and `cargo test` instead.

## Q-09 — `disk_usage` returns no totals on this machine

**Blocked work:** a truthful answer to "how much space is left", which is a natural first question for a resident agent.
**Facts:** asked through `vara-tui`, the router correctly chose `disk_usage`, and the tool returned `drive totals are not available on this platform yet`. The model then said so plainly instead of inventing a number — the honest behaviour, but the capability is missing.
**My recommendation:** implement volume totals on Windows (a `GetDiskFreeSpaceExW` call, or a `sysinfo` dependency) rather than leaving the tool advertised-but-empty. A tool that always answers "unknown" is worse than an absent one, because the router will keep choosing it.
**Meanwhile:** the failure is visible and honest, so nothing downstream is misled.


