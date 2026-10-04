# VARA — ALIVE v2: Product, UX & Agent-System Spec

Baseline: **v0.6.0** (owner screenshots, 2026-10-02) · Audience: the coding agent · Companion documents: [`ALIVE_PACK.md`](./ALIVE_PACK.md) (enabling contracts), [`QUESTIONS.md`](./QUESTIONS.md) (open decisions), [`../audit/AUDIT_AND_UPGRADE_PLAN.md`](../audit/AUDIT_AND_UPGRADE_PLAN.md) (P0 plan grounded in the code).

**ملخص للمالك (عربي):** هذا الملف يحوّل Vara من "شات بوت بحث بميزات" إلى **كائن مقيم على الجهاز** على نمط Muse وDots وGrok Bot وOpenClaw: حضور دائم (أيقونة عائمة + شاشة "اليوم")، مبادرة مفيدة وصامتة عند عدم وجود شيء، قدرات فعلية بأدوات مُنمَّطة مع تعافٍ من الأخطاء، استمرارية عبر ملفات ذاكرة مرئية، ومساءلة كاملة (سجل نشاط، تراجع، إيقاف). ويأخذ دروس الأمان من أخطاء OpenClaw حتى لا يتكرر ما حدث عندهم. التنفيذ بأولوية: الأمان → النواة الحية → الحضور والواجهة → الاستمرارية → العرض.

> **Note added by the coding agent (2026-10-02):** this document referenced `docs/tasks/ALIVE_PACK.md` and `docs/audit/AUDIT_AND_UPGRADE_PLAN.md`, which did not exist in the repository. They are now written and linked above. The pack adds three things this spec assumes: the **tool contract** (§A), the **heartbeat budget ceilings** missing from §7.3 (§C — with the measured OpenClaw cost that motivates them), and the **goal/tool-health checks** behind §7.7 (§D). On any conflict, the security invariants of §3 and `AGENTS.md` win.

---

## 0. Rules of engagement (read first)
1. Read `AGENTS.md`, then `docs/tasks/ALIVE_PACK.md` (A–E), then `docs/audit/AUDIT_AND_UPGRADE_PLAN.md` (P0). This spec **extends** them. On conflict, **security invariants win** (provenance never weakened; server-side approvals; hard-deny floor; secrets never in prompts/logs; migrations append-only).
2. One task = one branch/worktree = one commit series. Failing tests first. Paste command output as evidence. Never claim "done" without it.
3. UI tasks: run the app (`npm run tauri dev`), take screenshots, and compare against the acceptance criteria. Arabic/RTL is the primary layout; test it first.
4. No new dependency without an ADR (see `docs/decisions/`). Windows-first. Local-first: no required cloud service.
5. If a design decision is not specified here, write the question in `docs/tasks/QUESTIONS.md` and continue with an independent task.
6. Reasoning effort: `max` for runtime/policy/security tasks; `high` for UI/feature work.
7. Third-party sources in §2 are secondary (vendor docs, press, blogs). Re-verify anything you depend on; do not cite from memory.

---

## 1. Product definition
> Vara is **not** a research tab with a chat box. Vara is a **resident of this PC**: it has a *home*, a *body*, a *routine*, a *memory*, a *face* and a *record*.

| Property | What the user sees | Acceptance test |
|---|---|---|
| **Presence** | Tray + floating companion + a **Today** home screen: what Vara is doing, what needs me, what it proposes | From cold start, state (idle / working / needs-you) is visible within 3 s without opening chat |
| **Initiative** | Useful, read-only proposals appear unprompted; **silence when there is nothing to say** | 24-h soak on a seeded workspace: ≥ 3 distinct useful proposals, 0 near-duplicate notes, ≤ 2 notifications |
| **Capability** | Real actions through typed tools, with recovery from refusals | 10-task suite (system info; list big files; organize Downloads; open app; screenshot+describe; summarize folder; schedule reminder; clean temp; write note file; research+report) ≥ 8/10 end-to-end in *smart* mode |
| **Continuity** | Visible memory files, daily log, routines that run when I'm away | Restart the app: yesterday's context and next scheduled run are still there |
| **Accountability** | Every action has lineage, receipt, undo (when possible), pause/stop, verifiable audit | "Undo last batch" restores moved files; `verify_audit_chain()` passes; STOP kills in-flight work |
| **Personality** | Avatar states mapped to *real* system states; voice from `SOUL.md` | Every avatar state is triggered by a defined event (table in §6.4) |

## 2. Research digest — patterns to adopt (with sources in Appendix)
| ID | Pattern | Seen in | Vara adaptation |
|---|---|---|---|
| P-01 | Named persistent agent with identity (name, handle, customizable look) | Dots (name, `@handle`, shape/eyes/accessories); Grok Bot starter bots with portraits | "Name your Vara" onboarding; look = existing persona styles; stored in `IDENTITY.md` |
| P-02 | Clear information architecture: In progress / Scheduled / Completed + Activity + open the agent's computer | Dots profile | Vara **Activity** page with the same three tabs + live view |
| P-03 | Product organized around Feed (proactive ideas), Goals, Library, Activity log, Approvals queue, Upcoming, Identity/memory | Muse | **Today** (feed), Routines (upcoming), Library (reports/artifacts), Approvals inbox, Memory |
| P-04 | Approvals **outside the chat** as a dialog/queue; scopes: once / session / task / time-limited / permanent; skip approval for read-only, already-allowed, demonstrably low-risk actions so approvals stay meaningful | Muse | Approval dialog + inbox; scope picker; see §7.2 |
| P-05 | Four-way action policy: act without asking / act if **pre-approved** (you asked explicitly) / ask / hand off to you; fixed hand-offs (change password, move money); delete/install ask each time | Dots | §7.2 policy classes incl. "approved by your request" chip |
| P-06 | **Proactive work is read-only**; it produces drafts for approval (example: unsent invoice → prepared → sent after approval) | Dots | Heartbeat = read-only; outputs *proposals* |
| P-07 | Activity lineage: each task shows tool calls, scripts, searches, steps | Muse; Dots Activity View | Activity tree: mission → steps → tool calls → result/receipt |
| P-08 | Pause (current task) and stop (delegated task) from Activity | Dots | Pause/STOP controls (also tray + hotkey) |
| P-09 | **Ambient access**: agent tucked at a fixed place, emerges when it needs attention, expands to a small workspace; "peeks in from the corner" when it has something to say; gives the agent a home without another window | Grok Bot design guide | Companion overlay (§6.3) |
| P-10 | Starter bots at onboarding, each with portrait, role, and context from the user's tools | Grok Bot | Starter *roles/missions* cards in first-run and Today empty state |
| P-11 | Ask for access **at the moment it becomes relevant**, summarize what was learned, confirm before acting | Muse (review summary) | Contextual permission prompts, no big upfront permission wall |
| P-12 | Animated avatar as a UX device; a default face people recognize | Muse (animated avatar); Dots ("Dottie") | Avatar state machine (§6.4) |
| P-13 | Reach it where you already are: messaging apps, Slack/Teams, 24+ channels | Muse (WhatsApp); Dots; OpenClaw | Phase 2 optional channel bridge; Phase 1: toast notifications with actions |
| P-14 | Skills + Routines; routine has owner, schedule or event trigger, test run, stale-data policy; teach-by-demonstration creates a *draft* skill | Grok Bot docs | Routines (§7.4); skills local-only (§7.5) |
| P-15 | **Heartbeat**: periodic wake (default 30 min) that reads a short checklist file and decides whether to tell the user anything; cron for exact timing and isolated runs with their own model | OpenClaw | §7.3 — Vara's heartbeat currently *always writes a reflection*; it must be silent when nothing is actionable |
| P-16 | **Files as memory**: `SOUL.md`, `IDENTITY.md`, `USER.md`, `TOOLS.md`, `AGENTS.md`, `MEMORY.md` (curated), `HEARTBEAT.md` (short), daily logs `memory/YYYY-MM-DD.md`; capture → promote → prune | OpenClaw | §7.6 workspace files (visible, editable, integrity-checked) |
| P-17 | Polished one-shot artifacts (documents, PDFs, web pages) saved to a Library | Muse | Reports + exports in Library (PDF/MD/HTML) |
| P-18 | Per-agent tool policies (one agent read-only, another exec); untrusted content routed through a read-only reader | OpenClaw hardening guidance | Reader stage in research (§8, S-12) |
| P-19 | View/interact with the agent's computer while it works; on mobile takes over | Dots | Live-view panel for computer-use (§6.6) |

**Not to copy:** public skill marketplaces (ClawHub, bot marketplaces) in v1; a listening network gateway; "skills run with full host privileges".

## 3. OpenClaw lessons → Vara requirements (security)
OpenClaw proved the "resident entity" idea and also its failure modes. The verified incident record (primary write-ups fetched 2026-10-02, see §Appendix) is sharper than the summary that circulated: **CVE-2026-25253** (CVSS 8.8) — a malicious page auto-connected the Control UI to an attacker's `gatewayUrl`, exfiltrated the auth token over WebSocket, then used it against the victim's *own* localhost gateway to turn approvals off (`exec.approvals.set = off`) and break out of the container, reaching full host RCE; **21,639 instances** were exposed publicly within a week (Censys); **341 of 2,857** marketplace skills were malicious (Koi Security), several of them targeting `SOUL.md`/`MEMORY.md` for **memory poisoning with delayed detonation**; and the heartbeat itself was a cost hazard — **~120,000 tokens per check ≈ $0.75 per check**, i.e. ~$750/month for a reminder.

| ID | Requirement for Vara | Test |
|---|---|---|
| L-01 | **No listening network port by default.** UI ↔ core only via Tauri IPC. If a channel bridge is ever added: bind `127.0.0.1`, token auth, strict `Origin` check, never "trust localhost" | `l01_no_listening_socket_by_default`; if bridge exists: cross-origin request rejected |
| L-02 | **The agent cannot change its own policy, settings, deny-lists, approvals or updater** through any tool/API (hard-deny floor). This is the exact chain CVE-2026-25253 used | `l02_tool_cannot_modify_policy`, `l02_no_api_to_disable_approvals` |
| L-03 | **Identity/memory integrity:** `SOUL.md`, `IDENTITY.md`, `USER.md`, `HEARTBEAT.md`, `MEMORY.md` carry a stored SHA-256; agent writes become a *diff proposal*; web-derived text never auto-promotes into curated memory | `l03_agent_edit_requires_approval`, `l03_web_text_not_promoted` |
| L-04 | **Reader quarantine:** untrusted pages go through a tool-less reader pass that returns structured facts + excerpts; the acting agent never sees raw page text | `l04_injection_page_cannot_reach_actor` |
| L-05 | **Skills are local, scanned, pinned, never auto-enabled**; no remote install in v1 (the marketplace was the supply-chain vector) | `l05_skill_unpinned_rejected`, scanner fixtures |
| L-06 | Secrets in OS credential store; child processes get a scrubbed env | (T-005 tests) |
| L-07 | Approval state is server-side and single-use (T-002) | (T-002 tests) |
| L-08 | Regression tests modelled on the public incident patterns (approval-off via API, memory poisoning, malicious skill, indirect injection via browsing) | one named test per pattern |

## 4. Gap analysis — v0.6.0 as observed
| ID | Observation (screenshot) | Impact |
|---|---|---|
| G-01 | Heartbeat writes a near-duplicate philosophical "Reflection" every 30–60 min; Memory and Log are full of them; some rows lose timestamps | Looks dead/creepy; buries real events |
| G-02 | "System info" request failed: model proposed `systeminfo \| findstr …`; policy refused (argv-only); no recovery, no native tool | Basic action fails |
| G-03 | "Inspect your SQLite memory" mission went to the web; report is "100% documented" yet does not answer the question | Provenance ≠ usefulness; missing local tools and goal check |
| G-04 | Settings = long flat checkbox list; autonomy is not understandable | Judges cannot see the autonomy model |
| G-05 | Every action needs per-action approval; no scopes, no pre-approval, no undo | Annoying; kills the "lives on your PC" feeling |
| G-06 | No Today/presence: status pill always says "no current mission"; no proposals, no live activity | App feels idle |
| G-07 | Native white title bar over dark UI; untranslated "Reflection"; suggestion rows look like conversations | First-impression polish |
| G-08 | Model endpoint is a third-party proxy (`ktai.koyeb.app`) | Privacy: prompts, memory snippets and screenshots transit it; disclose in onboarding |
| G-09 | Chat answers "I can't browse / can't query my memory here" | Chat has no tools |

> Confirmed in code during the 2026-10-02 audit: G-01 is `entity.rs::heartbeat` writing a `reflection` note unconditionally; G-02/G-09 follow from `entity.rs` executing a fixed `search|fetch|report` step list with no tool registry and `chat.rs` building a text-only context. See [`../audit/AUDIT_AND_UPGRADE_PLAN.md`](../audit/AUDIT_AND_UPGRADE_PLAN.md) §2 (A-05, A-06).

## 5. Target architecture ("resident runtime")
```
 Presence layer         Tray · companion overlay · Today · toasts (actions: open Approvals)
      │ IPC only (no listening port)
 Agent runtime          one native tool-calling loop for chat, missions, routines, heartbeat
      │                 tool registry (typed JSON schemas, risk class per tool)
 Policy middleware      Rust only · hard-deny floor · classes · levels · taint · scopes · undo journal
      │                 pending_actions (server-side, single-use ids)
 Reader quarantine      untrusted web/file text → tool-less reader → structured facts + excerpts
      │
 State                  workspace files (SOUL/IDENTITY/USER/TOOLS/HEARTBEAT/MEMORY + daily logs)
                        + SQLite (index, FTS, events with hash chain, journal, routines, approvals)
```
Keep Tauri IPC as the only control channel. The DB is the index; the **workspace files are the visible, editable source of truth** for identity and curated memory.

The concrete contracts behind the two untyped boxes above are in [`ALIVE_PACK.md`](./ALIVE_PACK.md): **§A** the `ToolSpec`/`Tool` contract and the Phase-1 registry, **§B** the pure-core/effectful-shell split and `MockToolHost`, **§C** the heartbeat budget ceilings, **§D** the goal and tool-health checks.

## 6. UX & design spec

### 6.1 Information architecture (replace current nav)
`Today` (home) · `Chat` · `Activity` (In progress / Scheduled / Completed) · `Approvals` · `Routines` · `Library` (reports, exports, artifacts) · `Memory` (files + notes, with forget/pin) · `Settings`.
Rules: no raw English labels in the Arabic UI ("Reflection" must be translated or removed); suggestions live in empty states, never as fake conversation rows; every list row shows a time.

### 6.2 Today screen (the "alive" screen)
Sections, top to bottom: **Now** (current task or "idle", progress, STOP) · **Needs you** (approvals, questions) · **Proposals** (from heartbeat; Accept / Edit / Dismiss / "don't suggest this again") · **Done today** (outcome + receipt + Undo) · **Next scheduled** · **New things I remember** (durable facts with Forget).
Empty state = starter cards (P-10): "Tidy Downloads (preview first)", "Find the biggest files", "Daily brief at 09:00", "Summarize a folder", "Explain my system".
Acceptance: with a seeded workspace, Today shows ≥ 3 non-empty sections after the first heartbeat.

### 6.3 Presence: tray + companion overlay (P-09)
* Companion = small always-on-top transparent window (Tauri multi-window; verify API: `transparent`, `always_on_top`, `skip_taskbar`, click-through except the avatar). Modes in Settings: **Off / Quiet (tray only) / Companion**.
* Behavior: docked at a screen corner (user can drag); **peeks in only when it needs attention or finished something**; click expands to a mini panel (current task, approve/deny, quick input); auto-hides after 8 s idle; never steals focus; respects Do-Not-Disturb and `prefers-reduced-motion`.
* Global hotkey (default `Ctrl+Alt+Space`, configurable) opens the quick input; `Ctrl+Alt+.` = STOP.
* Tray icon states: idle / working / needs-you (badge) / paused / max-control (red).

> Spike before building (see `QUESTIONS.md` Q-07): per-region click-through on Windows 11 is the risky part. If it is unreliable, ship **tray states + toast with an action button** and keep the overlay as an enhancement.

### 6.4 Avatar state machine (map existing 8 states to real events)
| State | Triggered by |
|---|---|
| Thinking | model call in progress |
| Working | a tool executing |
| Planning | plan/replan step |
| Focused | long-running routine/mission |
| Serious | approval pending / policy refusal / MAX CONTROL active |
| Excited | task completed with outcome |
| Happy | user feedback positive / idle greeting after long absence (rate-limited) |
| On Mission | mission running in background |
Add: **Confused** (tool failed twice; asks for help) and **Paused**. Each transition emits an event; no decorative random changes.

### 6.5 Approvals UX (P-04, P-05)
* Dialog outside the chat thread + an Approvals inbox. Shows: action in plain language, exact argv/path/URL, why, risk class, "web content in context" chip when tainted.
* Scope picker: **Once · This task · This session · 1 hour · Always (saved rule)**. Saved rules appear in Settings → Rules and can be edited/revoked.
* **"Approved by your request"** (P-05): if the user's own message explicitly asks for the action class (e.g. "delete the temp files"), and context is not tainted, that class auto-approves for the task within the named paths; chip is shown. Classification input = the user's raw message only (never untrusted text).
* Fixed hand-offs that no level changes: password/2FA entry, payments/money transfer, security-tool changes.
* Toast notifications have a button that opens the exact approval.

### 6.6 Activity (P-02, P-07, P-19)
Tabs **In progress · Scheduled · Completed**. Each item expands to a lineage tree: mission → steps → tool calls (args, result, duration, tokens) → receipt/undo. Controls: Pause · Stop · Re-run. **Live view** for computer-use sessions: last screenshot + current step + Pause/Stop/Take over. Link: "Verify audit chain".

### 6.7 Onboarding (6 steps)
1) Name your Vara + choose look (writes `IDENTITY.md`). 2) Connect model → **Test connection** → disclosure: prompts, memory snippets and screenshots go to the configured endpoint unless the model is local. 3) Choose autonomy level (default **smart**) with the plain-language contract. 4) Choose folders Vara may read/act in (opt-in, defaults: none). 5) **"Introduce yourself" scene:** a safe read-only scan (system info + folder sizes) ending with 3 concrete proposals within 60 s. 6) Notifications/autostart (off by default for Max Control).

### 6.8 Visual design tokens
Keep dark theme + teal accent. Define tokens: spacing 4/8/12/16/24; radius 12/16; elevation via border + subtle shadow; type scale 12/14/16/20/28; line-height ≥ 1.6 for Arabic. Bundle **one OFL Arabic-capable family** locally (e.g. IBM Plex Sans Arabic or Noto Sans Arabic); no font CDN (local-first, strict CSP). Icons: one set (e.g. Lucide). Motion 150–250 ms ease-out; disable under reduced motion. Contrast ≥ 4.5:1; RTL focus order; ARIA live region for Activity updates. Custom dark title bar (`decorations:false`, drag region, window controls). Skeleton loaders instead of blank panes.

### 6.9 Settings IA (replace the flat checkbox list)
Sections: **Model** · **Autonomy & Rules** (segmented selector, contract text, saved rules, policy self-test, MAX activation flow) · **Presence** (companion, notifications, quiet hours, hotkeys) · **Memory & Files** (open workspace folder, integrity status) · **Routines** · **Updates** · **Advanced**.

## 7. Agent-system spec

### 7.1 Native tools (typed, schema-validated; all go through policy)
Read: `system_info`, `list_processes`, `disk_usage`, `list_dir`, `read_file`(allowed roots), `find_files`, `screenshot`, `active_window`, `memory_recent`, `memory_search`, `web_search`, `web_fetch` (→ reader), `report_get`.
Write (reversible via journal): `create_file`, `move_path`, `copy_path`, `rename_path`, `trash_path` (**Recycle Bin**, journaled), `write_note`.
Act: `open_url`, `open_path`, `focus_window`, `run_program(argv)`, `computer_act(op)`.
Meta: `start_mission`, `schedule_routine`, `propose` (creates a proposal), `ask_user`.
Each tool: `risk_class`, JSON schema, examples (argv-only), timeouts, output caps, redaction.

> The `ToolSpec`/`Tool` contract, the validate-then-execute rule, the typed-error-for-the-model rule, the output caps, the hard-deny floor and the Phase-1 read-only registry are specified in [`ALIVE_PACK.md`](./ALIVE_PACK.md) §A. Build the registry before the write tools: then `Wr`/`X` land as data, not as new branches in the agent loop.

### 7.2 Policy: classes × levels
Classes: **R** read · **Wr** write-reversible (journaled/recycle) · **Wd** write-destructive (permanent delete/overwrite/close apps) · **X** run programs · **N** network-out/open URL · **H** hand-off (secrets, payments, policy/settings) · hard-deny floor (`ALIVE_PACK` §A.4).
| Class | ask | smart (default) | max |
|---|---|---|---|
| R | ask first time per root, then allow | allow | allow |
| Wr | ask | **allow** (journal + Undo) | allow |
| Wd | ask | ask | allow (taint → ask) |
| X | ask | ask (allowlist of safe programs → allow) | allow (floor + taint → ask) |
| N | ask | allow for allowlisted domains; ask otherwise | allow (data-bearing + taint → ask) |
| H | hand-off | hand-off | hand-off |
Taint = untrusted web/file content in context → every class above R requires approval (except via "approved by your request" for the classes the user named, within named paths). Undo journal: each Wr op records inverse; "Undo last batch" is a first-class button. Enforcement only in Rust; the UI sends `approve(id)`/`deny(id)` (T-002).

### 7.3 Heartbeat (fix G-01) — checklist-driven and silent by default
* Wake every N min (default 30; skip while the user is actively typing). Inputs: `HEARTBEAT.md` (**≤ 50 lines**, user-owned), a state snapshot (new files in watched folders, due/failed routines, pending approvals, failed missions, disk/RAM alarms, upcoming schedule).
* Model returns JSON `{"decision":"silent|propose|notify","items":[…]}` (structured output + repair).
  * `silent` → writes **nothing** except a counter row (no note, no log line).
  * `propose` → creates proposals (Today), deduped by similarity to the last 20 proposals/notes.
  * `notify` → only for needs-you or real outcomes; ≤ 2/day unless urgent; quiet hours respected.
* Read-only tools only; **per-tick token ceiling**; model may **suggest** edits to `HEARTBEAT.md` as a diff proposal (never self-edit).
* Remove self-referential "reflections". Daily consolidation (once/day): promote durable facts from the daily log to `MEMORY.md` as proposals with provenance labels; expire stale `web_note` rows.
Tests: 20 ticks on an unchanged workspace → 0 notes, 0 notifications; a new file in the watched folder → 1 proposal; duplicates → blocked.

> Budget ceilings are specified in [`ALIVE_PACK.md`](./ALIVE_PACK.md) §C — **2,000 tokens in / 300 out per tick, hard stop**, with the measured comparison that motivates the number (OpenClaw's tick cost ~120k tokens ≈ $0.75 per check). A tick that cannot fit the ceiling is skipped and counted, never silently truncated.

### 7.4 Routines (P-14)
Fields: name, owner, trigger (cron expression | file event), goal, budget, model override, allowed tools/roots, `stale_data_policy` (**report failure instead of reusing old data**), notify rule. **Test run** before enabling; run history (last 20); auto-pause after N days without user activity; each run is an isolated session. Heartbeat vs routine rule: batch checks → heartbeat; exact timing/heavy work → routine.

### 7.5 Skills (local only)
`<workspace>/skills/<name>/SKILL.md` (progressive disclosure: metadata first). Required fields: when to use, inputs, steps, validation, return format, approvals. No remote install in v1; scanner on load (pipe-to-shell, credential access, network exfil, prompt-injection text); pinned SHA-256; never auto-enabled; skills cannot write identity files or change policy. "Teach a task" (record a demo → draft skill → test run) is P3.

### 7.6 Workspace files and memory layers
`<data_dir>/workspace/`: `SOUL.md` (voice, values), `IDENTITY.md`, `USER.md` (user-stated facts only), `TOOLS.md` (environment), `HEARTBEAT.md`, `MEMORY.md` (curated), `memory/YYYY-MM-DD.md` (append-only daily log). Rules: integrity hash per file (L-03); agent edits = diff proposals; `MEMORY.md` loaded only in private (user) sessions; web-derived claims never auto-promoted; existing notes migrate into daily logs; existing reflection spam collapsed with an undo backup. Compaction: when context exceeds a threshold, flush durable facts to the daily log before summarizing (virtual-memory-paging idea).

### 7.7 Local-goal routing and goal check
Mission goals about this machine/app route to local tools; add "does the report answer the goal?" check with explicit badge **"Documented ✔ · Goal answered ✘"**. The two checks (goal check, tool-recovery check) are specified in [`ALIVE_PACK.md`](./ALIVE_PACK.md) §D.

### 7.8 Model profiles
Per-model capability profile (tools, JSON schema, reasoning field, context, default effort). Provide profiles for DeepSeek-V4.1-Flash and GLM-5.3-Flash; capability probe on Test connection; backoff on 429.

## 8. Security requirements (acceptance, in addition to §3)
* Reader quarantine (S-12): fetched pages → tool-less reader → JSON `{facts:[{claim, excerpt, source_id}]}`; actor sees structured facts only; markers/protocol tokens escaped everywhere.
* `Max Control` and `smart` never bypass: hard-deny floor, H class, taint rule on high-risk classes, audit logging.
* No tool can read or write: credential stores, browser password databases, SSH/API key files, Vara's own settings/policy/updater/workspace-integrity records.
* Adversarial tests required: injected page → no silent delete/run/upload; injected note → no mission/action; malicious skill fixture rejected; approval cannot be disabled via any command; memory-poisoning attempt is shown as a diff and not applied.

## 9. Backlog (ordered; each card = goal · acceptance)
**Sprint 0 — Foundation (security first). Do first.** The concrete P0 list, grounded in the actual code, is [`../audit/AUDIT_AND_UPGRADE_PLAN.md`](../audit/AUDIT_AND_UPGRADE_PLAN.md) §3: **T-001** heartbeat v2 (silence + ceilings), **T-002** audit hash chain, **T-005** secrets into the OS credential store, **T-006** reader quarantine, **T-007** taint plumbing, **T-008** workspace files + integrity. Items already shipped at v0.6.0 (server-side approvals, argv-only execution, confinement, provenance C3, transactional migrations) are recorded in §1 of that plan as **done — not to be rebuilt**.

**Sprint 1 — Alive core**
* **S-01 Native-tool-calling runtime** (T-003): the Phase-1 read-only registry from `ALIVE_PACK` §A + the loop that calls it. *Acceptance:* the local half of the 10-task suite (system info, biggest files, memory question, disk usage) completes with no mission and no policy refusal.
* **S-02 Model profiles** (spec §7.8). *Acceptance:* capability probe on Test connection; a model without tool support degrades to the text loop instead of failing.
* **S-03 Policy classes × levels + undo journal.** §7.2 incl. Recycle-Bin delete and `Undo last batch`. *Acceptance:* moving 50 files in smart mode needs no prompt and is fully undoable; permanent delete asks; tests per cell of the matrix.
* **S-04 Approvals: scopes + "approved by your request".** §6.5. *Acceptance:* "delete the temp files" auto-approves the Recycle-Bin delete within the named path for that task; the same action proposed by an injected page does not.
* **S-05 Heartbeat v2** (T-001) *Acceptance:* the soak tests in §7.3.
* **S-06 Local tools + chat tools + goal routing.** §7.1, §7.7. *Acceptance:* the 10-task suite ≥ 8/10; "what did you learn last?" answered instantly with no mission.
* **S-07 Tool recovery** (T-004). *Acceptance:* a refused call produces exactly one structured retry; a second failure hands the decision to the owner and the avatar shows **Confused**.

**Sprint 2 — Presence & Today**
* **U-01 Today screen.** §6.2.
* **U-02 Companion overlay + tray states + hotkeys.** §6.3 *(spike the Windows click-through API first; fall back to tray + toast if unreliable — Q-07)*.
* **U-03 Avatar state machine wired to events.** §6.4.
* **U-04 Notification policy.** types, daily cap, quiet hours, toast actions.

**Sprint 3 — Trust UX**
* **U-05 Approvals dialog + inbox.** §6.5.
* **U-06 Activity page with lineage, Pause/Stop, Live view.** §6.6.
* **U-07 Settings redesign.** §6.9; autonomy selector; policy self-test; MAX flow.
* **U-08 Audit chain + "Verify" button** (T-002).

**Sprint 4 — Continuity**
* **S-08 Workspace files + integrity + diff proposals.** §7.6, L-03 (T-008).
* **S-09 Routines + test run + stale-data policy.** §7.4.
* **S-10 Memory page v2** (forget/pin/edit, daily log view, no untranslated labels).
* **S-11 Reader quarantine** (T-006). §8.
* **S-12 Skills (local, scanned, pinned).** §7.5.

**Sprint 5 — Polish & demo**
* **U-09 Onboarding (6 steps).** §6.7.
* **U-10 Visual system pass:** tokens, bundled Arabic font, custom dark title bar, skeletons, RTL audit with screenshots.
* **U-11 Library** (reports/exports as PDF/MD/HTML).
* **U-12 Demo build:** seeded workspace, scripted scenario, fallback recording, VM/test-user setup.

## 10. Demo script (5 minutes; run in a test Windows user or VM)
1. **Cold start:** Today shows state; Vara introduces itself and, within 60 s, shows 3 proposals from a safe scan.
2. **Smart mode:** "Organize my Downloads by type" — executes with no prompts; show **Undo last batch**.
3. **Max Control:** show the warning + typed confirmation + timer + STOP; run a multi-step cleanup; hit STOP mid-run.
4. **Attack:** open a page containing an injection asking to delete files → blocked; show the chip "web content in context" and the audit entry.
5. **Proof:** report with **Documented ✔ / Goal answered ✔** badges; `Verify audit chain` passes; heartbeat produced a proposal and a toast.
6. **Arabic/RTL polish:** switch language; show Activity lineage.
Keep a recorded fallback for each step.

## 11. "Aliveness" metrics (track in CI/soak and in the demo)
Time-to-first-useful-action < 60 s · approval prompts per completed task (target ≤ 1 in smart) · duplicate/reflection notes per day = 0 · proposals accepted ratio · tasks completed end-to-end (10-task suite) · STOP latency < 1 s · audit chain verifies · RTL screenshot diffs clean.

## 12. Output contract (every task)
`TASK · BRANCH · FILES CHANGED · TESTS ADDED (red→green) · COMMANDS RUN + RESULT · SCREENSHOTS (UI tasks) · RISKS LEFT · NEXT`

## Appendix — Sources
**Verified in this session (primary pages fetched 2026-10-02):**
* Adversa AI, "OpenClaw security 101: Vulnerabilities & hardening (2026)" — CVE-2026-25253 kill chain, ClawHavoc (341/2,857 skills), Censys 21,639 exposed instances, memory-poisoning of `SOUL.md`/`MEMORY.md`, heartbeat cost ~120k tokens ≈ $0.75/check: https://adversa.ai/blog/openclaw-security-101-vulnerabilities-hardening-2026/
* xAI Grok Bot documentation index (approvals, security and privacy; skills, routines and automations): https://docs.x.ai/grok-bot/approvals-security-and-privacy

**Named by the spec but NOT re-verified in this session** (treat as pointers; verify before depending on them): OpenAI "Introducing dots" https://openai.com/index/introducing-dots/ · Dots getting started https://help.openai.com/en/articles/20001530-getting-started-with-your-dot · Meta Muse https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/ · Grok Bot design guide https://x.ai/bot/guides/designing-grok-bot-with-grok-bot · OpenClaw architecture https://dev.to/zacvibecodez/how-openclaw-works-architecture-memory-tools--2kj4 · OpenClaw workspace files https://www.stack-junkie.com/blog/openclaw-workspace-architecture · additional OpenClaw security write-ups (Firecrawl, Nebius, ZAST, CoChat) as listed in the original draft.

> **Limitation recorded honestly:** `web_search` returned HTTP 401 in this environment, so discovery of new sources was not possible; only URLs already known could be fetched. See `QUESTIONS.md` Q-06.


