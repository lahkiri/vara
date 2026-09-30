//! SQLite persistence: WAL journal, migrations, FTS5 when available with a
//! LIKE fallback when not (bundled SQLite may or may not ship FTS5 — detected
//! at runtime so search never breaks the app).

use crate::dedup;
use crate::types::*;
use crate::{Result, VaraError};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub struct Database {
    conn: Mutex<Connection>,
    fts: bool,
}

const MIGRATIONS: &[&str] = &[/* v1 */ r#"
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
    "#];

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

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations(
               version INTEGER PRIMARY KEY,
               applied_at TEXT NOT NULL DEFAULT (datetime('now'))
             );",
        )?;
        let current: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let v = (i + 1) as i64;
            if v > current {
                conn.execute_batch(sql)?;
                conn.execute(
                    "INSERT INTO schema_migrations(version) VALUES (?1)",
                    params![v],
                )?;
            }
        }
        // FTS5 detection: if anything fails, we silently run in LIKE mode.
        let fts = Self::try_setup_fts(&conn).unwrap_or(false);
        Ok(Self {
            conn: Mutex::new(conn),
            fts,
        })
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
            let sql = format!(
                "SELECT n.id, n.created_at, n.kind, n.title, n.body, n.mission_id, n.source_url, n.source_title
                 FROM notes_fts f JOIN notes n ON n.id = f.rowid
                 WHERE notes_fts MATCH ?1 ORDER BY bm25(notes_fts) LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mapped =
                stmt.query_map(params![sanitized, limit.clamp(1, 200)], Self::note_from_row);
            let collected: Option<Vec<Note>> = match mapped {
                Ok(rows) => match rows.collect::<std::result::Result<Vec<_>, _>>() {
                    Ok(v) => Some(v),
                    Err(_) => None,
                },
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
        })
    }

    pub fn get_report(&self, id: i64) -> Result<ReportRecord> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, mission_id, created_at, markdown, sources_json, check_json,
                    backed_ratio, verdict, repaired
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
                    backed_ratio, verdict, repaired
             FROM reports ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 200)], Self::report_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
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
}
