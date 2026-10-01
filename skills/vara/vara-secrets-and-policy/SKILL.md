---
name: vara-secrets-and-policy
description: Secrets handling and the allow/ask/deny autonomy policy for OS actions. Use when touching settings, sys_execute, approvals, or anything credential-adjacent.
---

# Vara secrets & policy

## Secrets

- API keys live ONLY in the local settings file (`settings.json` under the OS
  app-data dir) or environment during development. They must never appear in:
  prompts sent to the model, event payloads, logs, the database, or the repo.
- The shipped `ProviderConfig::default()` contains an endpoint + model preset
  with an EMPTY key. Never bake a real key into defaults.
- When sharing logs or screenshots for debugging, scrub keys first.

## Policy (allow/ask/deny)

- The model proposes OS actions via the `[[sys]]` protocol (open_url /
  open_path / run). It never executes anything itself.
- Execution lives in the shell layer (`sys_execute`), which enforces
  settings: `open_urls`, `open_paths`, `run_commands`. Deny wins; anything not
  explicitly allowed is refused with a readable error written back into the
  thread as an action receipt.
- `run` always surfaces an explicit approval card in the chat before
  execution, regardless of toggles. Commands run with a 60s timeout, output
  capped, cwd confined to the watched folder or the user's home.
- Every executed action leaves a receipt message (kind="action") in the thread
  and an events-table row. No invisible actions.
- Precedence to preserve in future work: user deny > ask > allow; team/owner
  config may only ever make policy STRICTER, never looser.
