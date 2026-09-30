# Security Policy

## Supported versions

| version | supported |
|---|---|
| 0.1.x | yes |
| < 0.1 | no |

## Reporting a vulnerability

Please open a private security advisory (GitHub → Security → Report a
vulnerability) or contact the maintainers directly. Do not open a public
issue for security reports.

## Design notes relevant to security

- **Secrets**: the provider API key lives in `settings.json` inside the OS
  app-data directory. It is never logged, never sent anywhere except the
  provider base URL you configured, and never included in reports.
- **Report integrity**: reports are stored with their provenance check
  output; the UI never hides a FAIL verdict behind a repair.
- **System actions**: `sys_open` (URL/paths) honors two explicit toggles in
  Settings (`open_urls`, `open_paths`). Vara MVP performs no shell command
  execution and no filesystem writes outside its data folder.
- **Watched folder**: files ≤ 512 KB with `.md`/`.txt` extensions only; content
  is truncated at 2000 chars; nothing is executed.
- **Provenance gate limits**: the checker proves citations came from real
  retrievals — it does not certify the truth of retrieved sources. See
  `docs/PROVENANCE.md` §Limits.
