---
name: vara-definition-of-done
description: The binding checks to run before claiming any Vara work is complete. Use at the end of every task on this repo.
---

# Vara definition of done

Evidence before claims. Run the checks, read their output, then report.

## Checks (in order)

```sh
cargo fmt --all -- --check
cargo test -p vara-core          # unit + integration, all green
bun install                      # only when deps changed
bun run check                    # svelte-check: 0 errors 0 warnings
bun run build                    # production frontend build
```

## Honesty requirements

- Claim exactly what you verified. `vara-core` green + web build green does
  NOT mean the Windows desktop build is verified — that is CI's job
  (.github/workflows). Say so.
- If a check fails, fix the cause. NEVER make a failing check pass by
  weakening the check, skipping a test, or loosening an invariant.
- New behavior needs a test next to it: protocol parsing variants in
  `chat.rs`, migrations in `db.rs`, policy branches wherever they land.
- Before pushing: re-read AGENTS.md invariants 1-7 and confirm none were
  violated by the diff.
