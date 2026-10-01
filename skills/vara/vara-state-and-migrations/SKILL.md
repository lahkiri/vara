---
name: vara-state-and-migrations
description: SQLite rules for Vara — WAL, append-only migrations, single writer, schema map. Use when touching db.rs, the store, or any persisted state.
---

# Vara state & migrations

Vara's brain is one SQLite file (WAL mode, synchronous=NORMAL, foreign_keys=ON)
written through a single Mutex connection.

## Hard rules

- WAL is never disabled for simplicity or testing convenience.
- Migrations are append-only: add a new entry to `MIGRATIONS` (v3 was the last:
  messages.kind + messages.mission_id). Never rewrite history in an applied
  migration; if a shipped migration is wrong, fix forward with v(N+1).
- All access goes through `Database` methods. No raw `rusqlite` calls outside
  `db.rs`.
- Message rows carry `kind` ("text" | "mission" | "action") and optional
  `mission_id`; the UI renders cards from these. Keep the Rust type
  (`ChatMessageRecord`) and the TS mirror (`src/lib/types.ts`) in sync.
- Conversations link to missions via `conversations.mission_id`; chat-born
  missions must set it (`link_conversation_mission`) so follow-ups ground in
  the report.
- Bundled SQLite version matters: add a minimum-version test if rusqlite is
  upgraded (upstream had a two-connection checkpoint/commit loss pre-3.51.3).

## Definition of done

`cargo test -p vara-core` green — including the in-memory migration chain test
(open_memory runs every migration from scratch).
