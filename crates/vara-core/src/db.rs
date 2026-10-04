//! SQLite persistence: WAL journal, migrations, FTS5 when available with a
//! LIKE fallback when not (bundled SQLite may or may not ship FTS5 — detected
//! at runtime so search never breaks the app).

use crate::dedup;
use crate::types::*;
use crate::{Result, VaraError};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// The non-essential half of a chat message.
///
/// Grouped because five trailing positional arguments are unreadable, and
/// `None, 0, "ok"` compiles just as happily with two of them swapped.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChatMessageMeta<'a> {
    pub model: Option<&'a str>,
    pub tokens: i64,
    pub status: &'a str,
    pub kind: &'a str,
    pub mission_id: Option<i64>,
}
pub struct Database {
    conn: Mutex<Connection>,
    fts: bool,
}

const MIGRATIONS: &[&str] = &[
    /* v1 */
    r#"
    CREATE TABLE IF NOT EXISTS notes(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      kind TEXT NOT NULL DEFAULT 'research',
      title TEXT NOT NULL DEFAULT '',
      body TEXT NOT NULL DEFAULT '',
      mission_id INTEGER,
      source_url TEXT,
      source_title TEXT
    );
    CREATE TABLE IF NOT EXISTS missions(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      goal TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'planning',
      budget_tokens INTEGER NOT NULL DEFAULT 30000,
      spent_tokens INTEGER NOT NULL DEFAULT 0,
      max_steps INTEGER NOT NULL DEFAULT 14,
      steps_done INTEGER NOT NULL DEFAULT 0,
      dimensions TEXT,
      error TEXT
    );
    CREATE TABLE IF NOT EXISTS sources(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      mission_id INTEGER NOT NULL,
      url TEXT NOT NULL,
      title TEXT NOT NULL DEFAULT '',
      fetched INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE IF NOT EXISTS actions(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      mission_id INTEGER,
      ts TEXT NOT NULL DEFAULT (datetime('now')),
      kind TEXT NOT NULL,
      payload TEXT NOT NULL DEFAULT '{}',
      ok INTEGER NOT NULL DEFAULT 1,
      summary TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE IF NOT EXISTS reports(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      mission_id INTEGER NOT NULL,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      markdown TEXT NOT NULL,
      sources_json TEXT,
      check_json TEXT,
      backed_ratio REAL,
      verdict TEXT,
      repaired INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS events(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      ts TEXT NOT NULL DEFAULT (datetime('now')),
      level TEXT NOT NULL DEFAULT 'info',
      kind TEXT NOT NULL,
      message TEXT NOT NULL DEFAULT ''
    );
    "#,
    /* v2 — conversations: the entity is a companion you talk with */
    r#"
    CREATE TABLE IF NOT EXISTS conversations(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now')),
      title TEXT NOT NULL DEFAULT '',
      mission_id INTEGER
    );
    CREATE TABLE IF NOT EXISTS messages(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      conversation_id INTEGER NOT NULL,
      role TEXT NOT NULL,
      content TEXT NOT NULL DEFAULT '',
      model TEXT,
      tokens INTEGER NOT NULL DEFAULT 0,
      status TEXT NOT NULL DEFAULT 'ok',
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, id);
    CREATE INDEX IF NOT EXISTS idx_conversations_upd ON conversations(updated_at DESC);
    "#,
    /* v3 — missions and OS actions live INSIDE the conversation thread */
    r#"
    ALTER TABLE messages ADD COLUMN kind TEXT NOT NULL DEFAULT 'text';
    ALTER TABLE messages ADD COLUMN mission_id INTEGER;
    CREATE INDEX IF NOT EXISTS idx_messages_mission ON messages(mission_id);
    "#,
    /* v4 — the Action Journal: every computer-use step is evidence, not a
    log line. before/after artifact refs + grant level make the entity's
    deeds auditable (Muse-style) and feed her own memory of what she did. */
    r#"
    CREATE TABLE IF NOT EXISTS cu_journal(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      ts TEXT NOT NULL DEFAULT (datetime('now')),
      conversation_id INTEGER,
      seq_index INTEGER NOT NULL DEFAULT 0,
      op TEXT NOT NULL,
      grant_level TEXT NOT NULL DEFAULT 'L0',
      target TEXT NOT NULL DEFAULT '',
      ok INTEGER NOT NULL DEFAULT 0,
      dry_run INTEGER NOT NULL DEFAULT 0,
      active TEXT,
      before_ref TEXT,
      after_ref TEXT,
      check_note TEXT,
      ms INTEGER NOT NULL DEFAULT 0,
      error TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_cu_journal_conv ON cu_journal(conversation_id, id);
    "#,
    /* v5 — Action proposals: the owner's approval is a row, not a UI state.
    The model's [[sys]] proposals are minted HERE (in the core, from model
    output), carry a digest of exactly what was proposed, and can only be
    executed after an approval that is atomic, single-use and time-boxed.
    A webview can name a proposal id; it can never invent one. */
    r#"
    CREATE TABLE IF NOT EXISTS action_proposals(
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      conversation_id INTEGER,
      message_id INTEGER,
      kind TEXT NOT NULL,
      target TEXT NOT NULL DEFAULT '',
      reason TEXT NOT NULL DEFAULT '',
      risk TEXT NOT NULL DEFAULT 'medium',
      state TEXT NOT NULL DEFAULT 'pending',
      digest TEXT NOT NULL DEFAULT '',
      expires_at INTEGER NOT NULL DEFAULT 0,
      decided_at INTEGER,
      executed_at INTEGER,
      result TEXT,
      error TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_action_proposals_conv
      ON action_proposals(conversation_id, id DESC);
    CREATE INDEX IF NOT EXISTS idx_action_proposals_state
      ON action_proposals(state, expires_at);
    "#,
    /* v6 — the provenance receipt travels with the report: gate version,
    ratios with Wilson intervals, claim counts, C1/C2/C3, and the honest reason
    when something could not be evaluated. Without it a stored report could be
    shown as PASS with no denominator and no interval behind it. */
    r#"
    ALTER TABLE reports ADD COLUMN receipt_json TEXT;
    "#,
];

/// Split a migration script into statements on `;`, ignoring semicolons inside
/// single-quoted string literals. Migrations here are DDL only, but a stray
/// literal must never silently truncate a schema change.
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    for c in sql.chars() {
        match c {
            '\'' => {
                in_single = !in_single;
                current.push(c);
            }
            ';' if !in_single => {
                out.push(std::mem::take(&mut current));
            }
            c => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Errors that mean "a previous run already created this object", which is
/// progress rather than failure when a migration is replayed.
fn is_already_applied(e: &rusqlite::Error) -> bool {
    let msg = e.to_string().to_ascii_lowercase();
    msg.contains("duplicate column name") || msg.contains("already exists")
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations(
               version INTEGER PRIMARY KEY,
               applied_at TEXT NOT NULL DEFAULT (datetime('now'))
             );",
        )?;
        // Which versions are actually recorded, rather than just the highest
        // one: `MAX(version)` silently skips a gap, so a version row lost to a
        // crash (or a hand-edited database) could never be re-applied.
        let applied: std::collections::HashSet<i64> = {
            let mut stmt = conn.prepare("SELECT version FROM schema_migrations")?;
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
            rows.collect::<std::result::Result<std::collections::HashSet<_>, _>>()?
        };
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let v = (i + 1) as i64;
            if !applied.contains(&v) {
                Self::apply_migration(&mut conn, v, sql)?;
            }
        }
        // FTS5 detection: if anything fails, we silently run in LIKE mode.
        let fts = Self::try_setup_fts(&conn).unwrap_or(false);
        if fts {
            // Heal a desynchronised external-content index. This matters when
            // FTS5 becomes available on a machine whose notes were written
            // while Vara was running in LIKE mode: the virtual table is created
            // empty and every pre-existing note would be silently unsearchable.
            Self::rebuild_fts_if_stale(&conn);
        }
        Ok(Self {
            conn: Mutex::new(conn),
            fts,
        })
    }

    /// Apply one migration: statement by statement, inside a transaction, and
    /// tolerant of objects a previous crashed run already created.
    ///
    /// The old code ran `execute_batch(sql)` and *then* inserted the version
    /// row. A crash in between (or any mid-batch failure) meant the next launch
    /// replayed the batch; for v3 that is two `ALTER TABLE ... ADD COLUMN`
    /// statements, which fail with "duplicate column name" and made
    /// `Database::open` return `Err` forever — the app could never open its own
    /// database again. Splitting the batch and treating already-exists errors
    /// as progress makes replay safe, and the surrounding transaction keeps the
    /// schema and the version row in step.
    fn apply_migration(conn: &mut Connection, version: i64, sql: &str) -> Result<()> {
        let tx = conn.transaction()?;
        for statement in split_sql_statements(sql) {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            if let Err(e) = tx.execute_batch(statement) {
                if is_already_applied(&e) {
                    continue;
                }
                return Err(e.into());
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version) VALUES (?1)",
            params![version],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Rebuild the FTS index when its row count no longer matches `notes`.
    fn rebuild_fts_if_stale(conn: &Connection) {
        let indexed: Option<i64> = conn
            .query_row("SELECT COUNT(*) FROM notes_fts", [], |r| r.get(0))
            .ok();
        let total: Option<i64> = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))
            .ok();
        if let (Some(indexed), Some(total)) = (indexed, total) {
            if indexed != total {
                let _ = conn.execute_batch("INSERT INTO notes_fts(notes_fts) VALUES('rebuild');");
            }
        }
    }

    fn try_setup_fts(conn: &Connection) -> Option<bool> {
        conn.execute_batch(
            r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
              title, body, content='notes', content_rowid='id'
            );
            CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
              INSERT INTO notes_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
            END;
            CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
              INSERT INTO notes_fts(notes_fts, rowid, title, body)
              VALUES ('delete', old.id, old.title, old.body);
            END;
            "#,
        )
        .ok()?;
        // Verify it actually works (bundled builds may reject fts5 at CREATE).
        conn.query_row("SELECT COUNT(*) FROM notes_fts LIMIT 1", [], |r| {
            r.get::<_, i64>(0)
        })
        .ok()?;
        Some(true)
    }

    pub fn fts_enabled(&self) -> bool {
        self.fts
    }

    /// Highest migration version applied to this database.
    pub fn schema_version(&self) -> Result<i64> {
        let v = self.conn().query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        Ok(v)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    // ---------- events ----------

    pub fn insert_event(&self, level: &str, kind: &str, message: &str) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO events(level, kind, message) VALUES (?1, ?2, ?3)",
            params![level, kind, message],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn list_events(&self, limit: i64) -> Result<Vec<EventRecord>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT id, ts, level, kind, message FROM events ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 500)], |r| {
                Ok(EventRecord {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    level: r.get(2)?,
                    kind: r.get(3)?,
                    message: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---------- notes ----------

    pub fn insert_note(
        &self,
        kind: &str,
        title: &str,
        body: &str,
        mission_id: Option<i64>,
        source_url: Option<&str>,
        source_title: Option<&str>,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO notes(kind, title, body, mission_id, source_url, source_title)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![kind, title, body, mission_id, source_url, source_title],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Insert only if not a near-duplicate of a recent note in the same scope
    /// and the URL was not already recorded. Returns id on success.
    pub fn add_note_if_new(
        &self,
        kind: &str,
        title: &str,
        body: &str,
        mission_id: Option<i64>,
        source_url: Option<&str>,
        source_title: Option<&str>,
    ) -> Result<Option<i64>> {
        if let Some(u) = source_url {
            if self.url_already_retrieved(mission_id, u)? {
                return Ok(None);
            }
            // also dedupe against notes that already carry this URL
            let dup: i64 = {
                let conn = self.conn();
                conn.query_row(
                    "SELECT COUNT(*) FROM notes
                     WHERE source_url = ?2 AND (?1 IS NULL AND mission_id IS NULL OR mission_id = ?1)",
                    params![mission_id, u],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            };
            if dup > 0 {
                return Ok(None);
            }
        }
        let recent = self.recent_note_texts(mission_id, 40)?;
        let candidate = format!("{title} {body}");
        for t in recent {
            if dedup::is_duplicate(&candidate, &t) {
                return Ok(None);
            }
        }
        Ok(Some(self.insert_note(
            kind,
            title,
            body,
            mission_id,
            source_url,
            source_title,
        )?))
    }

    pub fn url_already_retrieved(&self, mission_id: Option<i64>, url: &str) -> Result<bool> {
        let conn = self.conn();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sources WHERE (?1 IS NULL OR mission_id = ?1) AND url = ?2",
                params![mission_id, url],
                |r| r.get(0),
            )
            .map_err(VaraError::Db)?;
        Ok(n > 0)
    }

    fn recent_note_texts(&self, mission_id: Option<i64>, limit: i64) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT title || ' ' || body FROM notes
             WHERE (?1 IS NULL AND mission_id IS NULL) OR mission_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![mission_id, limit], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn note_from_row(r: &rusqlite::Row) -> rusqlite::Result<Note> {
        Ok(Note {
            id: r.get(0)?,
            created_at: r.get(1)?,
            kind: r.get(2)?,
            title: r.get(3)?,
            body: r.get(4)?,
            mission_id: r.get(5)?,
            source_url: r.get(6)?,
            source_title: r.get(7)?,
        })
    }

    const NOTE_COLS: &'static str =
        "id, created_at, kind, title, body, mission_id, source_url, source_title";

    pub fn list_notes(&self, limit: i64, offset: i64) -> Result<Vec<Note>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM notes ORDER BY id DESC LIMIT ?1 OFFSET ?2",
            Self::NOTE_COLS
        ))?;
        let rows = stmt
            .query_map(
                params![limit.clamp(1, 500), offset.max(0)],
                Self::note_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn search_notes(&self, q: &str, limit: i64) -> Result<Vec<Note>> {
        let conn = self.conn();
        if self.fts {
            let sanitized = format!("\"{}\"", q.replace('"', "\"\""));
            let sql = "SELECT n.id, n.created_at, n.kind, n.title, n.body, n.mission_id, n.source_url, n.source_title
                 FROM notes_fts f JOIN notes n ON n.id = f.rowid
                 WHERE notes_fts MATCH ?1 ORDER BY bm25(notes_fts) LIMIT ?2".to_string();
            let mut stmt = conn.prepare(&sql)?;
            let mapped =
                stmt.query_map(params![sanitized, limit.clamp(1, 200)], Self::note_from_row);
            let collected: Option<Vec<Note>> = match mapped {
                Ok(rows) => rows.collect::<std::result::Result<Vec<_>, _>>().ok(),
                Err(_) => None,
            };
            if let Some(v) = collected {
                return Ok(v);
            }
            // fall through to LIKE on any FTS failure
        }
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM notes n
             WHERE n.title LIKE '%'||?1||'%' OR n.body LIKE '%'||?1||'%'
             ORDER BY n.id DESC LIMIT ?2",
            Self::NOTE_COLS
        ))?;
        let rows = stmt
            .query_map(params![q, limit.clamp(1, 200)], Self::note_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Memory search tuned for natural chat sentences: splits the text into
    /// significant keywords and matches ANY of them (FTS OR-query, or a set of
    /// LIKE clauses without FTS). Whole-sentence phrase matching almost never
    /// hits real conversation text, so keywords are the honest approach.
    pub fn search_notes_any(&self, text: &str, limit: i64) -> Result<Vec<Note>> {
        let keywords = keywords_of(text);
        if keywords.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn();
        if self.fts {
            let q = keywords
                .iter()
                .map(|k| format!("\"{}\"", k.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = "SELECT n.id, n.created_at, n.kind, n.title, n.body, n.mission_id, n.source_url, n.source_title
                 FROM notes_fts f JOIN notes n ON n.id = f.rowid
                 WHERE notes_fts MATCH ?1 ORDER BY bm25(notes_fts) LIMIT ?2".to_string();
            let mut stmt = conn.prepare(&sql)?;
            let mapped = stmt.query_map(params![q, limit.clamp(1, 200)], Self::note_from_row);
            if let Ok(rows) = mapped {
                if let Ok(v) = rows.collect::<std::result::Result<Vec<_>, _>>() {
                    return Ok(v);
                }
            }
            // fall through to LIKE on any FTS failure
        }
        let clauses = keywords
            .iter()
            .map(|_| "(n.title LIKE '%'||?||'%' OR n.body LIKE '%'||?||'%')")
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT {} FROM notes n WHERE {} ORDER BY n.id DESC LIMIT {}",
            Self::NOTE_COLS,
            clauses,
            limit.clamp(1, 200)
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind_vals: Vec<String> = Vec::new();
        for k in &keywords {
            bind_vals.push(k.clone());
            bind_vals.push(k.clone());
        }
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(bind_vals.iter()),
                Self::note_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn mission_notes(&self, mission_id: i64, limit: i64) -> Result<Vec<Note>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM notes WHERE mission_id = ?1 ORDER BY id ASC LIMIT ?2",
            Self::NOTE_COLS
        ))?;
        let rows = stmt
            .query_map(
                params![mission_id, limit.clamp(1, 1000)],
                Self::note_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_note(&self, id: i64) -> Result<Note> {
        let conn = self.conn();
        conn.query_row(
            &format!("SELECT {} FROM notes WHERE id = ?1", Self::NOTE_COLS),
            params![id],
            Self::note_from_row,
        )
        .map_err(|e| VaraError::Other(format!("note {id}: {e}")))
    }

    pub fn count_notes(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))?)
    }

    pub fn delete_note(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- missions ----------

    pub fn create_mission(&self, goal: &str, budget_tokens: i64, max_steps: i64) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO missions(goal, status, budget_tokens, max_steps) VALUES (?1, 'planning', ?2, ?3)",
            params![goal, budget_tokens, max_steps],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn update_mission_status(&self, id: i64, status: &str, error: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE missions SET status = ?2, error = ?3 WHERE id = ?1",
            params![id, status, error],
        )?;
        Ok(())
    }

    /// Every mission still marked `running`, oldest first.
    ///
    /// At startup, before any worker exists, no mission can be running: the
    /// process that was executing it is gone. These rows are therefore zombie
    /// state — the UI showed a mission in progress, the cancel button refused
    /// them (`busy` is false), and nothing ever moved them.
    pub fn missions_marked_running(&self) -> Result<Vec<i64>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id FROM missions WHERE status = 'running' ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Clear zombie missions after a restart.
    ///
    /// This is the mission counterpart of [`Database::expire_action_proposals`],
    /// and it exists for the same reason: durable state written by a process that
    /// died must not be presented as live. A killed mission becomes
    /// **`interrupted`** with a reason, which is a state the owner can act on
    /// (restart it), instead of `running`, which is a state only the dead process
    /// could have left behind.
    ///
    /// It deliberately does **not** resume anything. Resuming is a product
    /// decision with a cost, and pretending to resume while re-running a mission
    /// from its beginning would be worse than saying plainly that it stopped.
    /// The ledger, the notes and any partial report are all still in the database,
    /// so a future recovery can be built on top of this without another migration.
    pub fn recover_interrupted_missions(&self, reason: &str) -> Result<usize> {
        let n = self.conn().execute(
            "UPDATE missions SET status = 'interrupted', error = ?1 WHERE status = 'running'",
            params![reason],
        )?;
        Ok(n)
    }

    pub fn update_mission_progress(&self, id: i64, steps_done: i64, spent: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE missions SET steps_done = ?2, spent_tokens = ?3 WHERE id = ?1",
            params![id, steps_done, spent],
        )?;
        Ok(())
    }

    pub fn set_mission_dimensions(&self, id: i64, dims_json: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE missions SET dimensions = ?2 WHERE id = ?1",
            params![id, dims_json],
        )?;
        Ok(())
    }

    pub fn get_mission(&self, id: i64) -> Result<Mission> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, created_at, goal, status, budget_tokens, spent_tokens, max_steps,
                    steps_done, dimensions, error
             FROM missions WHERE id = ?1",
            params![id],
            |r| {
                let dims: Option<String> = r.get(8)?;
                Ok(Mission {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    goal: r.get(2)?,
                    status: r.get(3)?,
                    budget_tokens: r.get(4)?,
                    spent_tokens: r.get(5)?,
                    max_steps: r.get(6)?,
                    steps_done: r.get(7)?,
                    dimensions: dims.and_then(|d| serde_json::from_str(&d).ok()),
                    error: r.get(9)?,
                })
            },
        )
        .map_err(|e| VaraError::Other(format!("mission {id}: {e}")))
    }

    pub fn list_missions(&self, limit: i64) -> Result<Vec<Mission>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, created_at, goal, status, budget_tokens, spent_tokens, max_steps,
                    steps_done, dimensions, error
             FROM missions ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 200)], |r| {
                let dims: Option<String> = r.get(8)?;
                Ok(Mission {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    goal: r.get(2)?,
                    status: r.get(3)?,
                    budget_tokens: r.get(4)?,
                    spent_tokens: r.get(5)?,
                    max_steps: r.get(6)?,
                    steps_done: r.get(7)?,
                    dimensions: dims.and_then(|d| serde_json::from_str(&d).ok()),
                    error: r.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn active_mission(&self) -> Result<Option<Mission>> {
        let conn = self.conn();
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM missions WHERE status IN ('planning','running') ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        drop(conn);
        match id {
            Some(id) => Ok(Some(self.get_mission(id)?)),
            None => Ok(None),
        }
    }

    // ---------- sources / actions ----------

    pub fn insert_source(
        &self,
        mission_id: i64,
        url: &str,
        title: &str,
        fetched: bool,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO sources(mission_id, url, title, fetched) VALUES (?1, ?2, ?3, ?4)",
            params![mission_id, url, title, fetched as i64],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn list_sources(&self, mission_id: i64) -> Result<Vec<SourceRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, mission_id, url, title, fetched FROM sources WHERE mission_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map(params![mission_id], |r| {
                let f: i64 = r.get(4)?;
                Ok(SourceRecord {
                    id: r.get(0)?,
                    mission_id: r.get(1)?,
                    url: r.get(2)?,
                    title: r.get(3)?,
                    fetched: f != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_action(
        &self,
        mission_id: Option<i64>,
        kind: &str,
        payload: &str,
        ok: bool,
        summary: &str,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO actions(mission_id, kind, payload, ok, summary) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![mission_id, kind, payload, ok as i64, summary],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn list_actions(&self, mission_id: i64, limit: i64) -> Result<Vec<ActionRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, mission_id, ts, kind, ok, summary FROM actions
             WHERE mission_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![mission_id, limit.clamp(1, 500)], |r| {
                let ok: i64 = r.get(4)?;
                Ok(ActionRecord {
                    id: r.get(0)?,
                    mission_id: r.get(1)?,
                    ts: r.get(2)?,
                    kind: r.get(3)?,
                    ok: ok != 0,
                    summary: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---------- action journal (computer use) ----------

    /// One auditable line per executed computer-use step: what she did, at
    /// what grant level, with what evidence. The journal IS the audit log
    /// and the memory of her deeds.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_cu_step(
        &self,
        conversation_id: Option<i64>,
        seq_index: usize,
        op: &str,
        grant_level: &str,
        target: &str,
        ok: bool,
        dry_run: bool,
        active: Option<&str>,
        before_ref: Option<&str>,
        after_ref: Option<&str>,
        check_note: Option<&str>,
        ms: u64,
        error: Option<&str>,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO cu_journal(conversation_id, seq_index, op, grant_level, target, ok,
             dry_run, active, before_ref, after_ref, check_note, ms, error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                conversation_id,
                seq_index as i64,
                op,
                grant_level,
                target,
                ok as i64,
                dry_run as i64,
                active,
                before_ref,
                after_ref,
                check_note,
                ms as i64,
                error
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn list_cu_journal(
        &self,
        conversation_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<CuJournalEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, ts, conversation_id, seq_index, op, grant_level, target, ok, dry_run,
                    active, before_ref, after_ref, check_note, ms, error
             FROM cu_journal
             WHERE (?1 IS NULL OR conversation_id = ?1)
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![conversation_id, limit.clamp(1, 1000)], |r| {
                let ok: i64 = r.get(7)?;
                let dry: i64 = r.get(8)?;
                Ok(CuJournalEntry {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    conversation_id: r.get(2)?,
                    seq_index: r.get(3)?,
                    op: r.get(4)?,
                    grant_level: r.get(5)?,
                    target: r.get(6)?,
                    ok: ok != 0,
                    dry_run: dry != 0,
                    active: r.get(9)?,
                    before_ref: r.get(10)?,
                    after_ref: r.get(11)?,
                    check_note: r.get(12)?,
                    ms: r.get(13)?,
                    error: r.get(14)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Retention: drop journal rows older than N days (screenshots pile up —
    /// the owner's MCP troubleshooting says so too).
    pub fn prune_cu_journal(&self, keep_days: i64) -> Result<usize> {
        let n = self.conn().execute(
            "DELETE FROM cu_journal WHERE ts < datetime('now', ?1 || ' days')",
            params![format!("-{}", keep_days.max(1))],
        )?;
        Ok(n)
    }

    // ---------- reports ----------

    #[allow(clippy::too_many_arguments)]
    pub fn insert_report(
        &self,
        mission_id: i64,
        markdown: &str,
        sources_json: &serde_json::Value,
        check_json: &serde_json::Value,
        backed_ratio: f64,
        verdict: &str,
        repaired: bool,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO reports(mission_id, markdown, sources_json, check_json, backed_ratio, verdict, repaired)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                mission_id,
                markdown,
                sources_json.to_string(),
                check_json.to_string(),
                backed_ratio,
                verdict,
                repaired as i64
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    fn report_from_row(r: &rusqlite::Row) -> rusqlite::Result<ReportRecord> {
        let sources_json: Option<String> = r.get(4)?;
        let check_json: Option<String> = r.get(5)?;
        let repaired: i64 = r.get(8)?;
        let receipt_json: Option<String> = r.get(9)?;
        Ok(ReportRecord {
            id: r.get(0)?,
            mission_id: r.get(1)?,
            created_at: r.get(2)?,
            markdown: r.get(3)?,
            sources_json: sources_json.and_then(|s| serde_json::from_str(&s).ok()),
            check_json: check_json.and_then(|s| serde_json::from_str(&s).ok()),
            backed_ratio: r.get(6)?,
            verdict: r.get(7)?,
            repaired: repaired != 0,
            receipt_json: receipt_json.and_then(|s| serde_json::from_str(&s).ok()),
        })
    }

    /// Attach the provenance receipt (and the per-claim audit) to a stored
    /// report. Kept separate from `insert_report` so the receipt is written by
    /// the same code path that computed the verdict, and so a failure to store
    /// it can never turn a FAIL into a PASS.
    pub fn set_report_receipt(
        &self,
        report_id: i64,
        receipt: &serde_json::Value,
        claims: &serde_json::Value,
    ) -> Result<()> {
        let payload = serde_json::json!({ "receipt": receipt, "claims": claims });
        self.conn().execute(
            "UPDATE reports SET receipt_json = ?1 WHERE id = ?2",
            params![payload.to_string(), report_id],
        )?;
        Ok(())
    }

    pub fn get_report(&self, id: i64) -> Result<ReportRecord> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, mission_id, created_at, markdown, sources_json, check_json,
                    backed_ratio, verdict, repaired, receipt_json
             FROM reports WHERE id = ?1",
            params![id],
            Self::report_from_row,
        )
        .map_err(|e| VaraError::Other(format!("report {id}: {e}")))
    }

    pub fn list_reports(&self, limit: i64) -> Result<Vec<ReportRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, mission_id, created_at, markdown, sources_json, check_json,
                    backed_ratio, verdict, repaired, receipt_json
             FROM reports ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 200)], Self::report_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---------- conversations (chat with the entity) ----------

    pub fn create_conversation(&self, title: &str, mission_id: Option<i64>) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO conversations(title, mission_id) VALUES (?1, ?2)",
            params![title.trim(), mission_id],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn get_conversation(&self, id: i64) -> Result<Conversation> {
        self.conn()
            .query_row(
                "SELECT id, created_at, updated_at, title, mission_id
                 FROM conversations WHERE id = ?1",
                params![id],
                |r| {
                    Ok(Conversation {
                        id: r.get(0)?,
                        created_at: r.get(1)?,
                        updated_at: r.get(2)?,
                        title: r.get(3)?,
                        mission_id: r.get(4)?,
                    })
                },
            )
            .map_err(|e| VaraError::Other(format!("conversation {id}: {e}")))
    }

    pub fn list_conversations(&self, limit: i64) -> Result<Vec<Conversation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, created_at, updated_at, title, mission_id
             FROM conversations ORDER BY updated_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 200)], |r| {
                Ok(Conversation {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    updated_at: r.get(2)?,
                    title: r.get(3)?,
                    mission_id: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn rename_conversation(&self, id: i64, title: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE conversations SET title = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![id, title.trim()],
        )?;
        Ok(())
    }

    /// Binds a thread to a mission so every later reply is grounded in the
    /// mission's report (the "mission lives in the chat" link).
    pub fn link_conversation_mission(&self, id: i64, mission_id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE conversations SET mission_id = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![id, mission_id],
        )?;
        Ok(())
    }

    pub fn touch_conversation(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE conversations SET updated_at = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn delete_conversation(&self, id: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM messages WHERE conversation_id = ?1",
            params![id],
        )?;
        conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_chat_message(
        &self,
        conversation_id: i64,
        role: &str,
        content: &str,
        model: Option<&str>,
        tokens: i64,
        status: &str,
    ) -> Result<i64> {
        self.insert_chat_message_typed(
            conversation_id,
            role,
            content,
            ChatMessageMeta {
                model,
                tokens,
                status,
                kind: "text",
                mission_id: None,
            },
        )
    }

    /// Full variant: `kind` selects the card the UI renders ("text" |
    /// "mission" | "action"), `mission_id` links the row to a live mission.
    ///
    /// The metadata travels as a struct rather than as five positional arguments
    /// because `insert_chat_message_typed(1, "assistant", text, None, 0, "done",
    /// "text", Some(7))` is a call nobody can read — and a swapped pair of
    /// arguments here would still compile.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_chat_message_typed(
        &self,
        conversation_id: i64,
        role: &str,
        content: &str,
        meta: ChatMessageMeta<'_>,
    ) -> Result<i64> {
        let ChatMessageMeta {
            model,
            tokens,
            status,
            kind,
            mission_id,
        } = meta;
        self.conn().execute(
            "INSERT INTO messages(conversation_id, role, content, model, tokens, status, kind, mission_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![conversation_id, role, content, model, tokens, status, kind, mission_id],
        )?;
        let id = self.conn().last_insert_rowid();
        self.touch_conversation(conversation_id)?;
        Ok(id)
    }

    pub fn update_chat_message(
        &self,
        id: i64,
        content: &str,
        tokens: i64,
        model: Option<&str>,
        status: &str,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE messages SET content = ?2, tokens = ?3, model = ?4, status = ?5 WHERE id = ?1",
            params![id, content, tokens, model, status],
        )?;
        Ok(())
    }

    pub fn get_chat_message(&self, id: i64) -> Result<ChatMessageRecord> {
        self.conn()
            .query_row(Self::MESSAGE_SQL_WHERE, params![id], Self::msg_from_row)
            .map_err(|e| VaraError::Other(format!("message {id}: {e}")))
    }

    /// The last `limit` messages of a conversation, oldest first — ready to
    /// feed straight into the model context.
    pub fn list_chat_messages(
        &self,
        conversation_id: i64,
        limit: i64,
    ) -> Result<Vec<ChatMessageRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM ({} ORDER BY id DESC LIMIT ?2) ORDER BY id ASC",
            Self::MESSAGE_SQL
        ))?;
        let rows = stmt
            .query_map(
                params![conversation_id, limit.clamp(1, 200)],
                Self::msg_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Recent assistant messages that proposed missions, for the sidebar
    /// "suggestions" — kept simple: not exposed in v1 UI.
    pub fn count_conversations(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM conversations", [], |r| r.get(0))?)
    }

    const MESSAGE_SQL: &'static str =
        "SELECT id, conversation_id, role, content, model, tokens, status, created_at, kind, mission_id
         FROM messages WHERE conversation_id = ?1";

    const MESSAGE_SQL_WHERE: &'static str =
        "SELECT id, conversation_id, role, content, model, tokens, status, created_at, kind, mission_id
         FROM messages WHERE id = ?1";

    fn msg_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessageRecord> {
        Ok(ChatMessageRecord {
            id: r.get(0)?,
            conversation_id: r.get(1)?,
            role: r.get(2)?,
            content: r.get(3)?,
            model: r.get(4)?,
            tokens: r.get(5)?,
            status: r.get(6)?,
            created_at: r.get(7)?,
            kind: r
                .get::<_, Option<String>>(8)?
                .unwrap_or_else(|| "text".into()),
            mission_id: r.get(9)?,
        })
    }

    /// Latest report of a mission — used to ground "discuss this report" chats.
    pub fn latest_report_for_mission(&self, mission_id: i64) -> Result<Option<ReportRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, mission_id, created_at, markdown, sources_json, check_json,
                    backed_ratio, verdict, repaired, receipt_json
             FROM reports WHERE mission_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt
            .query_map(params![mission_id], Self::report_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows.pop())
    }

    // ---------- stats ----------

    pub fn stats(&self) -> Result<Stats> {
        let conn = self.conn();
        let missions_total: i64 =
            conn.query_row("SELECT COUNT(*) FROM missions", [], |r| r.get(0))?;
        let missions_completed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM missions WHERE status = 'completed'",
            [],
            |r| r.get(0),
        )?;
        let notes_total: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))?;
        let reports_total: i64 =
            conn.query_row("SELECT COUNT(*) FROM reports", [], |r| r.get(0))?;
        let avg_backed_ratio: Option<f64> = conn
            .query_row(
                "SELECT AVG(backed_ratio) FROM reports WHERE verdict IS NOT NULL AND repaired = 0",
                [],
                |r| r.get(0),
            )
            .ok();
        Ok(Stats {
            missions_total,
            missions_completed,
            notes_total,
            reports_total,
            avg_backed_ratio: avg_backed_ratio.map(|x| (x * 1000.0).round() / 1000.0),
        })
    }
}

/// Extracts significant keywords from a natural sentence (Arabic or Latin).
/// Tokens shorter than 3 chars are dropped (Arabic particles, English "the"),
/// edge punctuation is trimmed, at most 10 keywords are kept.
pub fn keywords_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in text.split_whitespace() {
        let trimmed = tok
            .trim_matches(|c: char| {
                c.is_ascii_punctuation() || matches!(c, '؟' | '،' | '؛' | '«' | '»' | '…' | 'ـ')
            })
            .to_lowercase();
        if trimmed.chars().count() >= 3 && !out.contains(&trimmed) {
            out.push(trimmed);
        }
        if out.len() >= 10 {
            break;
        }
    }
    out
}

/// A second inherent impl keeps the approval-boundary code together and
/// reviewable; Rust allows multiple `impl Database` blocks.
impl Database {
    // ---------- action proposals (the approval boundary) ----------
    //
    // A proposal is minted here from model output, approved by the owner, and
    // claimed for execution exactly once. The transitions use
    // `UPDATE ... WHERE state = ?` so two concurrent executes cannot both win,
    // and the shell executes the target stored in this row — never a target
    // supplied by the caller — so the webview cannot widen what was approved.

    pub fn insert_action_proposal(
        &self,
        planned: &crate::exec_policy::PlannedProposal,
        conversation_id: Option<i64>,
        message_id: Option<i64>,
    ) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO action_proposals(
               conversation_id, message_id, kind, target, reason, risk, state,
               digest, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8)",
            params![
                conversation_id,
                message_id,
                planned.kind.as_str(),
                planned.target,
                planned.reason,
                planned.risk.as_str(),
                planned.digest,
                planned.expires_at
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    fn row_to_proposal(
        r: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<crate::exec_policy::ActionProposal> {
        use crate::exec_policy::{ProposalKind, ProposalState, Risk};
        let kind_raw: String = r.get("kind")?;
        let risk_raw: String = r.get("risk")?;
        let state_raw: String = r.get("state")?;
        Ok(crate::exec_policy::ActionProposal {
            id: r.get("id")?,
            conversation_id: r.get("conversation_id")?,
            message_id: r.get("message_id")?,
            kind: ProposalKind::parse(&kind_raw).unwrap_or(ProposalKind::Run),
            target: r.get("target")?,
            reason: r.get("reason")?,
            risk: match risk_raw.as_str() {
                "low" => Risk::Low,
                "high" => Risk::High,
                _ => Risk::Medium,
            },
            state: ProposalState::parse(&state_raw).unwrap_or(ProposalState::Pending),
            digest: r.get("digest")?,
            created_at: r.get("created_at")?,
            expires_at: r.get("expires_at")?,
            result: r.get("result")?,
            error: r.get("error")?,
        })
    }

    pub fn get_action_proposal(
        &self,
        id: i64,
    ) -> Result<Option<crate::exec_policy::ActionProposal>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM action_proposals WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], Self::row_to_proposal)?;
        Ok(rows.next().transpose()?)
    }

    /// Proposals for one conversation, newest first — what the thread renders.
    pub fn list_action_proposals(
        &self,
        conversation_id: i64,
        limit: i64,
    ) -> Result<Vec<crate::exec_policy::ActionProposal>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM action_proposals WHERE conversation_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(
                params![conversation_id, limit.clamp(1, 200)],
                Self::row_to_proposal,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The owner's decision. Returns true only for the call that performed the
    /// transition, so a double-click cannot approve twice and an expired
    /// proposal cannot be approved at all.
    pub fn decide_action_proposal(&self, id: i64, approve: bool, now: i64) -> Result<bool> {
        let next = if approve { "approved" } else { "denied" };
        let changed = self.conn().execute(
            "UPDATE action_proposals SET state = ?1, decided_at = ?2
             WHERE id = ?3 AND state = 'pending' AND expires_at > ?4",
            params![next, now, id, now],
        )?;
        Ok(changed == 1)
    }

    /// Claim an approved proposal for execution. Atomic: exactly one caller can
    /// win, and a second attempt is impossible because the row leaves
    /// `approved`.
    pub fn claim_action_proposal(
        &self,
        id: i64,
        now: i64,
    ) -> Result<Option<crate::exec_policy::ActionProposal>> {
        let changed = self.conn().execute(
            "UPDATE action_proposals SET state = 'executing'
             WHERE id = ?1 AND state = 'approved' AND expires_at > ?2",
            params![id, now],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        self.get_action_proposal(id)
    }

    pub fn finish_action_proposal(
        &self,
        id: i64,
        ok: bool,
        result: &str,
        error: &str,
        now: i64,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE action_proposals
             SET state = ?1, executed_at = ?2, result = ?3, error = ?4
             WHERE id = ?5",
            params![
                if ok { "executed" } else { "failed" },
                now,
                result,
                error,
                id
            ],
        )?;
        Ok(())
    }

    /// Mark stale pending/approved proposals as expired. Keeps the approval
    /// window honest across restarts: an approval from ten minutes ago is not
    /// an approval for now.
    pub fn expire_action_proposals(&self, now: i64) -> Result<usize> {
        let n = self.conn().execute(
            "UPDATE action_proposals SET state = 'expired'
             WHERE state IN ('pending', 'approved') AND expires_at <= ?1",
            params![now],
        )?;
        Ok(n)
    }

    /// How many proposals are waiting for the owner — the UI badge, and tests.
    pub fn pending_proposal_count(&self, conversation_id: i64, now: i64) -> Result<i64> {
        let n = self.conn().query_row(
            "SELECT COUNT(*) FROM action_proposals
             WHERE conversation_id = ?1 AND state = 'pending' AND expires_at > ?2",
            params![conversation_id, now],
            |r| r.get(0),
        )?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_migrate_and_basic_ops() {
        let db = Database::open_memory().unwrap();
        let id = db
            .create_mission("Build a bench of local LLM inference servers", 30_000, 12)
            .unwrap();
        db.set_mission_dimensions(id, r#"[{"name":"speed"}]"#)
            .unwrap();
        db.update_mission_status(id, "running", None).unwrap();
        db.update_mission_progress(id, 3, 1200).unwrap();

        let n1 = db
            .add_note_if_new(
                "research",
                "llama.cpp server",
                "llama-server exposes an OpenAI compatible endpoint on port 8080",
                Some(id),
                Some("https://github.com/ggml-org/llama.cpp"),
                Some("llama.cpp"),
            )
            .unwrap();
        assert!(n1.is_some());

        // Same URL again → skipped.
        let n2 = db
            .add_note_if_new(
                "research",
                "another llama.cpp",
                "different text entirely about inference servers",
                Some(id),
                Some("https://github.com/ggml-org/llama.cpp"),
                None,
            )
            .unwrap();
        assert!(n2.is_none());

        // Near-duplicate text → skipped.
        let n3 = db
            .add_note_if_new(
                "research",
                "llama.cpp server",
                "llama-server exposes an OpenAI compatible endpoint on port 8080",
                Some(id),
                None,
                None,
            )
            .unwrap();
        assert!(n3.is_none());

        let notes = db.mission_notes(id, 50).unwrap();
        assert_eq!(notes.len(), 1);
        let m = db.get_mission(id).unwrap();
        assert_eq!(m.status, "running");
        assert_eq!(m.steps_done, 3);
        assert_eq!(m.spent_tokens, 1200);
        assert!(m.dimensions.is_some());

        let act = db.active_mission().unwrap();
        assert_eq!(act.unwrap().id, id);
        db.update_mission_status(id, "completed", None).unwrap();
        assert!(db.active_mission().unwrap().is_none());
    }

    #[test]
    fn search_works_with_or_without_fts() {
        let db = Database::open_memory().unwrap();
        db.insert_note(
            "research",
            "Model context",
            "Qwen3 supports a 32k context window natively",
            None,
            None,
            None,
        )
        .unwrap();
        db.insert_note(
            "research",
            "Inference",
            "vLLM uses paged attention for KV cache",
            None,
            None,
            None,
        )
        .unwrap();
        let hits = db.search_notes("context window", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].title.contains("Model context"));
    }

    #[test]
    fn reports_and_stats() {
        let db = Database::open_memory().unwrap();
        let m = db.create_mission("m", 1000, 5).unwrap();
        let check = serde_json::json!({"verdict": "PASS"});
        db.insert_report(
            m,
            "# Report",
            &serde_json::json!([]),
            &check,
            1.0,
            "PASS",
            false,
        )
        .unwrap();
        db.insert_report(
            m,
            "# Report v2",
            &serde_json::json!([]),
            &check,
            0.5,
            "FAIL",
            true,
        )
        .unwrap();
        let reports = db.list_reports(10).unwrap();
        assert_eq!(reports.len(), 2);
        assert!(reports[0].repaired);
        let s = db.stats().unwrap();
        assert_eq!(s.reports_total, 2);
        assert_eq!(s.missions_completed, 0);
    }

    /// The old migration runner ran the batch and *then* recorded the version.
    /// A crash in between replayed the batch on the next launch; for v3 that
    /// means `ALTER TABLE ... ADD COLUMN` on columns that already exist, which
    /// used to make `Database::open` fail forever. This reproduces that state
    /// (columns present, version row missing) and requires the app to recover.
    #[test]
    fn open_heals_a_migration_that_was_applied_but_not_recorded() {
        let dir = std::env::temp_dir().join(format!("vara-mig-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("vara.db");
        let _ = std::fs::remove_file(&path);

        {
            let db = Database::open(&path).unwrap();
            assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        }

        // Simulate "batch ran, version row never written" for v3, and also drop
        // the index that the same batch was supposed to create.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("DELETE FROM schema_migrations WHERE version = 3", [])
                .unwrap();
            conn.execute_batch("DROP INDEX IF EXISTS idx_messages_mission;")
                .unwrap();
            let has_kind: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name = 'kind'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(has_kind, 1, "precondition: v3 columns are already there");
        }

        let db = Database::open(&path).expect("reopening must not brick the database");
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        let index_exists: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_messages_mission'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index_exists, 1, "the partial migration must be completed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_twice_is_idempotent_and_reports_the_current_version() {
        let db = Database::open_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        assert_eq!(db.pending_proposal_count(1, 1_000).unwrap(), 0);
    }

    #[test]
    fn proposal_lifecycle_is_single_use_and_time_boxed() {
        use crate::exec_policy::{plan_proposal, ProposalKind, ProposalState};
        let db = Database::open_memory().unwrap();
        let now = 10_000i64;
        let planned =
            plan_proposal(ProposalKind::Run, "cargo test", "verify the build", now).unwrap();
        let id = db
            .insert_action_proposal(&planned, Some(7), Some(3))
            .unwrap();

        let stored = db.get_action_proposal(id).unwrap().unwrap();
        assert_eq!(stored.state, ProposalState::Pending);
        assert_eq!(stored.conversation_id, Some(7));
        assert_eq!(stored.message_id, Some(3));
        assert_eq!(stored.target, "cargo test");
        assert!(stored.digest_matches(), "digest must bind kind + target");
        assert_eq!(db.pending_proposal_count(7, now).unwrap(), 1);

        // It cannot run before the owner approves it.
        assert!(db.claim_action_proposal(id, now).unwrap().is_none());

        assert!(db.decide_action_proposal(id, true, now).unwrap());
        // Double-click cannot approve twice.
        assert!(!db.decide_action_proposal(id, true, now).unwrap());

        let claimed = db.claim_action_proposal(id, now).unwrap().unwrap();
        assert_eq!(claimed.state, ProposalState::Executing);
        assert!(claimed.may_execute_now(now).is_ok());
        // Single use: a second claim finds nothing to claim.
        assert!(db.claim_action_proposal(id, now).unwrap().is_none());

        db.finish_action_proposal(id, true, "exit 0", "", now + 1)
            .unwrap();
        let done = db.get_action_proposal(id).unwrap().unwrap();
        assert_eq!(done.state, ProposalState::Executed);
        assert_eq!(done.result.as_deref(), Some("exit 0"));
        assert_eq!(db.pending_proposal_count(7, now).unwrap(), 0);
    }

    #[test]
    fn expired_proposals_cannot_be_approved_or_claimed() {
        use crate::exec_policy::{plan_proposal, ProposalKind, ProposalState};
        let db = Database::open_memory().unwrap();
        let now = 50_000i64;
        let planned = plan_proposal(ProposalKind::Screenshot, "", "look", now).unwrap();
        let id = db.insert_action_proposal(&planned, Some(1), None).unwrap();

        // Ten minutes later the approval window is long gone.
        let later = now + 600;
        assert!(!db.decide_action_proposal(id, true, later).unwrap());
        assert!(db.claim_action_proposal(id, later).unwrap().is_none());
        assert_eq!(db.pending_proposal_count(1, later).unwrap(), 0);

        assert_eq!(db.expire_action_proposals(later).unwrap(), 1);
        assert_eq!(
            db.get_action_proposal(id).unwrap().unwrap().state,
            ProposalState::Expired
        );
    }

    #[test]
    fn denied_proposals_never_execute() {
        use crate::exec_policy::{plan_proposal, ProposalKind, ProposalState};
        let db = Database::open_memory().unwrap();
        let now = 1_000i64;
        let planned = plan_proposal(ProposalKind::Run, "rm -rf build", "", now).unwrap();
        let id = db.insert_action_proposal(&planned, Some(2), None).unwrap();
        assert!(db.decide_action_proposal(id, false, now).unwrap());
        assert!(db.claim_action_proposal(id, now).unwrap().is_none());
        assert_eq!(
            db.get_action_proposal(id).unwrap().unwrap().state,
            ProposalState::Denied
        );
    }

    /// The zombie state an auditor found in v0.7.0: kill the app mid-mission and
    /// the row stayed `running` forever, with no sweep at startup and a cancel
    /// button that refused it because `busy` was false.
    ///
    /// `recover_interrupted_missions` closes that, on the same pattern the
    /// approval queue already used for stale proposals.
    #[test]
    fn a_mission_left_running_is_recovered_as_interrupted() {
        let db = Database::open_memory().unwrap();
        let running = db.create_mission("killed mid-flight", 10_000, 10).unwrap();
        let finished = db.create_mission("finished normally", 10_000, 10).unwrap();
        db.update_mission_status(running, "running", None).unwrap();
        db.update_mission_status(finished, "completed", None)
            .unwrap();

        // Before recovery the zombie is visible and would be shown as live.
        assert_eq!(db.missions_marked_running().unwrap(), vec![running]);

        let n = db
            .recover_interrupted_missions("the app stopped while this was running")
            .unwrap();
        assert_eq!(n, 1, "exactly the running mission is recovered");

        // The interrupted mission is no longer running, carries a reason, and
        // the completed one was not touched.
        assert!(db.missions_marked_running().unwrap().is_empty());
        let zombie = db.get_mission(running).unwrap();
        assert_eq!(zombie.status, "interrupted");
        assert!(zombie
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("stopped while this was running"));
        assert_eq!(db.get_mission(finished).unwrap().status, "completed");

        // Idempotent: a second boot with nothing running recovers nothing.
        assert_eq!(db.recover_interrupted_missions("again").unwrap(), 0);
    }
}
