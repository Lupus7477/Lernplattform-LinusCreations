//! SQLite: Schema und die Abfragen, die Hintergrund-Worker und Befehle teilen.

use rusqlite::{params, Connection, OptionalExtension};

pub type Db = std::sync::Arc<std::sync::Mutex<Connection>>;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS courses(
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, exam_date TEXT, created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS documents(
  id INTEGER PRIMARY KEY, course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  name TEXT NOT NULL, pages INTEGER NOT NULL, created_at INTEGER NOT NULL);
-- status: pending (wartet auf Analyse) | ready
-- q_state: 0 keine Fragen, 1 gewünscht, 2 vorhanden, 3 fehlgeschlagen
CREATE TABLE IF NOT EXISTS units(
  id INTEGER PRIMARY KEY, course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL, page_from INTEGER NOT NULL, page_to INTEGER NOT NULL,
  title TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '', source_text TEXT NOT NULL,
  explanation TEXT, status TEXT NOT NULL DEFAULT 'pending',
  q_state INTEGER NOT NULL DEFAULT 0, q_level INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS terms(
  id INTEGER PRIMARY KEY, unit_id INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
  term TEXT NOT NULL, definition TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS questions(
  id INTEGER PRIMARY KEY, unit_id INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, prompt TEXT NOT NULL, options TEXT NOT NULL, answer TEXT NOT NULL,
  why TEXT NOT NULL DEFAULT '', level INTEGER NOT NULL DEFAULT 1,
  box INTEGER NOT NULL DEFAULT 0, due_at INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS lessons(
  id INTEGER PRIMARY KEY, unit_id INTEGER REFERENCES units(id) ON DELETE SET NULL,
  xp INTEGER NOT NULL, correct INTEGER NOT NULL, total INTEGER NOT NULL, finished_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS units_course ON units(course_id, idx);
CREATE INDEX IF NOT EXISTS questions_unit ON questions(unit_id);
CREATE INDEX IF NOT EXISTS terms_unit ON terms(unit_id);
";

pub fn open(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Nächste Hintergrund-Aufgabe: gewünschte Fragen zuerst, dann noch nicht analysierte Abschnitte.
pub enum Task {
    Questions { unit_id: i64, level: i64, text: String },
    Analyze { unit_id: i64, course_id: i64, idx: i64, page_from: i64, page_to: i64, text: String },
}

pub fn next_task(conn: &Connection) -> rusqlite::Result<Option<Task>> {
    let q = conn
        .query_row(
            "SELECT id, q_level + 1, source_text FROM units WHERE status = 'ready' AND q_state = 1 ORDER BY course_id, idx LIMIT 1",
            [],
            |r| Ok(Task::Questions { unit_id: r.get(0)?, level: r.get(1)?, text: r.get(2)? }),
        )
        .optional()?;
    if q.is_some() {
        return Ok(q);
    }
    conn.query_row(
        "SELECT id, course_id, idx, page_from, page_to, source_text FROM units WHERE status = 'pending' ORDER BY course_id, idx LIMIT 1",
        [],
        |r| {
            Ok(Task::Analyze {
                unit_id: r.get(0)?,
                course_id: r.get(1)?,
                idx: r.get(2)?,
                page_from: r.get(3)?,
                page_to: r.get(4)?,
                text: r.get(5)?,
            })
        },
    )
    .optional()
}
