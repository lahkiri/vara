# Security Policy

## Supported versions

| version | supported |
|---|---|
| 0.6.x | yes |
| < 0.6 | no |

## Reporting a vulnerability

Open a private security advisory at
<https://github.com/lahkiri/vara/security/advisories/new> or email the
maintainer. Please include the version, the platform, and a minimal
reproduction. We aim to acknowledge within 72 hours. Do not open a public issue
for an exploitable finding before a fix ships.

## The trust boundary

The model is untrusted. It reads web pages, and web pages can carry
instructions. The product is therefore built so that **model output is never
executed directly**:

1. The model can only *propose* actions with the `[[sys]]` protocol
   (`open_url`, `open_path`, `run`, `screenshot`, `computer_use`).
2. The shell mints each proposal into an `action_proposals` row
   (`exec_policy::plan_proposal`) carrying a digest of exactly what was proposed
   and a short expiry.
3. **You** approve or deny that row. The webview only ever holds a proposal id —
   it cannot invent an action, name its own target, or execute something the
   backend never recorded.
4. `sys_execute(proposal_id)` claims an approved, unexpired row **once** (a
   database compare-and-swap), re-checks the digest, executes the target *stored
   in the row*, and writes a receipt back into the conversation.

`run` is additionally **argv-only**: no `cmd /C`, no shell, no pipes, no
redirection. A command line is tokenized into arguments
(`exec_policy::tokenize_command`), shell metacharacters are refused, known shell
and system tools (`cmd`, `powershell`, `bash`, `wmic`, `schtasks`, `reg`,
`certutil`, `shutdown`, …) are denied outright, and inline-code flags
(`python -c`, `node -e`) are refused because they are a shell by another name.
Paths are confined to your owner folder, and the child process gets a scrubbed
environment — a command the entity runs cannot read your provider key.

## What ships off

| Capability | Default | Enabling it |
| --- | --- | --- |
| `open_url` / `open_path` | on | Settings ▸ Autonomy |
| `run` (argv-only commands) | **off** | Settings ▸ Autonomy |
| `screenshot` | **off** | Settings ▸ Autonomy; each capture still needs an approval card |
| `computer_use` | **off** | Settings ▸ Autonomy; also needs `screenshot`, because the ActLoop grounds every coordinate in a capture |
| destructive UI ops (`close_window`, `close_app`, `alt+f4`, `ctrl+w`, …) | **off** | `computer_use_allow_close` (L2) *and* `confirm: true` on the op |

The ActLoop refuses coordinate actions with no preceding observation, and
converts destructive steps below L2 into dry runs itself rather than trusting the
adapter to do it.

## Secrets

- The provider API key lives in the app's local `settings.json` (`%APPDATA%`) or
  can be supplied only through `VARA_PROVIDER_API_KEY`.
- An environment-provided key is **never written to disk**: what is persisted is
  stripped (`Settings::without_api_key`), and the webview receives
  `Settings::for_webview` — it can see *that* a key exists (`has_api_key`,
  `api_key_source`), never the key.
- The key never enters a prompt, a mission ledger, the database, or a spawned
  command's environment.
- Nothing is synced; Vara has no telemetry.

## What this is *not*

Honest limits — a security page that oversells itself is worse than none:

- **No OS-level sandbox yet.** Commands run as you, with your rights. The argv
  policy, the approval gate and the path confinement reduce what is reachable
  through Vara; they are not a container. A Windows restricted-token /
  AppContainer sandbox is on the roadmap.
- **The provenance gate proves provenance, not truth.** A source can be real,
  retrieved, quoted word-for-word, and still be wrong. Treat reports as
  traceable, not infallible (`docs/PROVENANCE.md` §Limits).
- **Prompt injection is mitigated, not solved.** Anything a page says can still
  become a proposal; what it cannot do is execute. Read the approval cards —
  that is what they are for. When untrusted text lands in a consequential slot
  (a URL host, a path, a command argument), that card is the last line of
  defence.
- **Computer use depends on an external sidecar** (`VARA_CU_SIDECAR`) that is
  *not* bundled with this repository. Without it, `computer_use` fails honestly
  with "sidecar not configured".
- **The updater trusts GitHub Releases** signed with the project's minisign key.
  If that key leaks, an attacker could ship an update; treat releases as you
  would any other software.

## Data on disk

Everything lives in your app-data folder: `vara.db` (SQLite, WAL),
`settings.json`, `screenshots/` when you enable capture, and any report you
export. Deleting that folder deletes the entity's memory — there is no cloud
copy to fall back on.

## Watched folder

Files ≤ 512 KB with `.md`/`.txt` extensions only; content is truncated at 2000
chars; nothing found there is ever executed.
