//! SQLite: Schema und die Abfragen, die Hintergrund-Worker und Befehle teilen.

use rusqlite::{params, Connection, OptionalExtension};

pub type Db = std::sync::Arc<std::sync::Mutex<Connection>>;

/// Version 0 (bis 0.2): wird immer zuerst angelegt, Änderungen kommen als Migration darüber.
pub const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS courses(
  id INTEGER PRIMARY KEY, name TEXT NOT NULL, exam_date TEXT, created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS documents(
  id INTEGER PRIMARY KEY, course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  name TEXT NOT NULL, pages INTEGER NOT NULL, created_at INTEGER NOT NULL);
-- status: pending (wartet auf Analyse) | ready
-- q_state: 0 keine Fragen, 1 gewünscht, 2 vorhanden, 3 fehlgeschlagen, 4 nach Ersetzen des PDFs prüfen
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

/// Version 1: Konten, Fortschritt pro Nutzer (statt in `questions`), Kurs-Besitzer und Teilen.
/// Nutzer 1 ist der lokale Nutzer der App (ohne Passwort); auf dem Server übernimmt ihn der erste Admin.
const V1: &str = "
CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, pw_hash TEXT,
  admin INTEGER NOT NULL DEFAULT 0, disabled INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
INSERT INTO users(id, name, admin, created_at) VALUES(1, 'Ich', 1, strftime('%s', 'now'));
CREATE TABLE sessions(id INTEGER PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL, last_seen INTEGER NOT NULL, expires_at INTEGER NOT NULL, label TEXT NOT NULL, ip TEXT NOT NULL);
-- user_id NULL = Einladung für ein neues Konto, sonst Link zum Passwort-Zurücksetzen
CREATE TABLE invites(code_hash TEXT PRIMARY KEY, created_by INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  user_id INTEGER REFERENCES users(id) ON DELETE CASCADE, expires_at INTEGER NOT NULL, used_at INTEGER);
CREATE TABLE progress(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
  box INTEGER NOT NULL DEFAULT 0, due_at INTEGER NOT NULL DEFAULT 0, hidden INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(user_id, question_id)) WITHOUT ROWID;
INSERT INTO progress(user_id, question_id, box, due_at) SELECT 1, id, box, due_at FROM questions WHERE box > 0;
ALTER TABLE questions DROP COLUMN box;
ALTER TABLE questions DROP COLUMN due_at;
ALTER TABLE courses ADD COLUMN owner_id INTEGER NOT NULL DEFAULT 1;
ALTER TABLE courses ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
ALTER TABLE lessons ADD COLUMN user_id INTEGER NOT NULL DEFAULT 1;
UPDATE settings SET key = 'goal:1' WHERE key = 'goal';
CREATE INDEX lessons_user ON lessons(user_id, finished_at);
CREATE INDEX sessions_user ON sessions(user_id);
PRAGMA user_version = 1;
";

/// Version 2: Strukturaufgaben (Felder, Spots) und Grafiken aus dem PDF.
/// `image` = data:-URL (jpeg/png/webp); `page` NULL = eigenes Bild.
// ponytail: Bilder als data:-URL-Text, keine Base64-Umwandlung in Rust. Eigener Blob-Endpunkt, falls die Datenbank zu groß wird.
const V2: &str = "
CREATE TABLE figures(id INTEGER PRIMARY KEY, unit_id INTEGER NOT NULL REFERENCES units(id) ON DELETE CASCADE,
  page INTEGER, image TEXT NOT NULL);
ALTER TABLE questions ADD COLUMN groups TEXT NOT NULL DEFAULT '[]';
ALTER TABLE questions ADD COLUMN spots TEXT NOT NULL DEFAULT '[]';
ALTER TABLE questions ADD COLUMN figure_id INTEGER REFERENCES figures(id) ON DELETE CASCADE;
CREATE INDEX figures_unit ON figures(unit_id);
PRAGMA user_version = 2;
";

/// Version 3: „zuletzt falsch“ je Frage und Nutzer, Kurse mit ausgewählten Personen teilen.
/// `shared = 1` bleibt „alle auf dem Server“; `members_invite` = Mitglieder dürfen weitere Personen hinzufügen.
const V3: &str = "
ALTER TABLE progress ADD COLUMN wrong INTEGER NOT NULL DEFAULT 0;
CREATE TABLE course_members(course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, PRIMARY KEY(course_id, user_id)) WITHOUT ROWID;
CREATE INDEX course_members_user ON course_members(user_id);
ALTER TABLE courses ADD COLUMN members_invite INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 3;
";

/// Version 4: Themenfilter, Prüfungsschwerpunkte, Lesezeichen, Kurs verlassen.
/// `units.orga` = Seiten (0-basiert im Abschnitt) nur mit Organisatorischem, NULL = Nutzer will alles lernen.
/// `units.status` kann jetzt auch 'orga' sein (nur Organisatorisches, keine Fragen).
/// `units.exam` = passende Prüfungsthemen als JSON, NULL = noch nicht geprüft. `course_left` = Kurs für alle, den jemand verlassen hat.
const V4: &str = "
ALTER TABLE units ADD COLUMN orga TEXT DEFAULT '[]';
ALTER TABLE units ADD COLUMN exam_hint TEXT NOT NULL DEFAULT '';
ALTER TABLE units ADD COLUMN exam TEXT;
ALTER TABLE courses ADD COLUMN exam_focus TEXT NOT NULL DEFAULT '';
ALTER TABLE progress ADD COLUMN marked INTEGER NOT NULL DEFAULT 0;
CREATE TABLE course_left(course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, PRIMARY KEY(course_id, user_id)) WITHOUT ROWID;
PRAGMA user_version = 4;
";

pub fn open(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        // Vorhandene Daten vor dem Umbau sichern (lernen.v0.bak neben der Datenbank)
        let backup = path.with_file_name("lernen.v0.bak");
        let has_data: i64 = conn.query_row("SELECT COUNT(*) FROM courses", [], |r| r.get(0))?;
        if has_data > 0 && !backup.exists() {
            conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().into_owned()])?;
        }
        // foreign_keys lässt sich nur außerhalb einer Transaktion umschalten; ADD COLUMN mit Default braucht es aus.
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V1} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
    if version < 2 {
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V2} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
    if version < 3 {
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V3} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
    if version < 4 {
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V4} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
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

/// Nächste Hintergrund-Aufgabe:
/// 1. Prüfungsthemen einer Einheit, die auf Fragen wartet (kurz, damit die Fragen den Schwerpunkt kennen)
/// 2. gewünschte Fragen
/// 3. noch nicht analysierte Abschnitte
/// 4. Prüfungsthemen der übrigen Einheiten
/// 5. Fragen geänderter Abschnitte prüfen (PDF ersetzt)
pub enum Task {
    Questions { unit_id: i64, level: i64, text: String, topics: Vec<String> },
    /// `filter` = Organisatorisches aussortieren (aus, wenn der Nutzer „Doch einbeziehen“ gewählt hat)
    Analyze { unit_id: i64, course_id: i64, idx: i64, page_from: i64, page_to: i64, text: String, course: String, filter: bool },
    /// `info` = Titel, Zusammenfassung, Begriffe und Anfang des Lerntexts
    Focus { unit_id: i64, topics: Vec<String>, info: String },
    Recheck { unit_id: i64, text: String },
}

fn json_list(s: Option<String>) -> Vec<String> {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn focus_task(conn: &Connection, waiting: bool) -> rusqlite::Result<Option<Task>> {
    let sql = format!(
        "SELECT u.id, u.title, u.summary, u.source_text, u.orga, c.exam_focus,
           (SELECT group_concat(t.term, ', ') FROM terms t WHERE t.unit_id = u.id)
         FROM units u JOIN courses c ON c.id = u.course_id
         WHERE u.status = 'ready' AND u.exam IS NULL AND c.exam_focus != '' {}
         ORDER BY u.course_id, u.idx LIMIT 1",
        if waiting { "AND u.q_state = 1" } else { "" }
    );
    conn.query_row(&sql, [], |r| {
        let (title, summary, source, orga, focus, terms): (String, String, String, Option<String>, String, Option<String>) =
            (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?);
        let text = crate::import::learn_text(&source, orga.as_deref());
        let start: String = text.chars().take(800).collect();
        let info = format!("Thema: {title}\nZusammenfassung: {summary}\nBegriffe: {}\nAuszug:\n{start}", terms.unwrap_or_default());
        Ok(Task::Focus { unit_id: r.get(0)?, topics: crate::import::exam_topics(&focus), info })
    })
    .optional()
}

pub fn next_task(conn: &Connection) -> rusqlite::Result<Option<Task>> {
    if let Some(t) = focus_task(conn, true)? {
        return Ok(Some(t));
    }
    // Hat der Kurs Schwerpunkte, warten die Fragen, bis die Einheit geprüft ist (Schritt 1).
    let q = conn
        .query_row(
            "SELECT u.id, u.q_level + 1, u.source_text, u.orga, u.exam FROM units u JOIN courses c ON c.id = u.course_id
             WHERE u.status = 'ready' AND u.q_state = 1 AND (u.exam IS NOT NULL OR c.exam_focus = '') ORDER BY u.course_id, u.idx LIMIT 1",
            [],
            |r| {
                let (source, orga): (String, Option<String>) = (r.get(2)?, r.get(3)?);
                Ok(Task::Questions { unit_id: r.get(0)?, level: r.get(1)?, text: crate::import::learn_text(&source, orga.as_deref()), topics: json_list(r.get(4)?) })
            },
        )
        .optional()?;
    if q.is_some() {
        return Ok(q);
    }
    let a = conn
        .query_row(
            "SELECT u.id, u.course_id, u.idx, u.page_from, u.page_to, u.source_text, c.name, u.orga IS NOT NULL
             FROM units u JOIN courses c ON c.id = u.course_id WHERE u.status = 'pending' ORDER BY u.course_id, u.idx LIMIT 1",
            [],
            |r| {
                Ok(Task::Analyze {
                    unit_id: r.get(0)?,
                    course_id: r.get(1)?,
                    idx: r.get(2)?,
                    page_from: r.get(3)?,
                    page_to: r.get(4)?,
                    text: r.get(5)?,
                    course: r.get(6)?,
                    filter: r.get(7)?,
                })
            },
        )
        .optional()?;
    if a.is_some() {
        return Ok(a);
    }
    if let Some(t) = focus_task(conn, false)? {
        return Ok(Some(t));
    }
    conn.query_row(
        "SELECT id, source_text, orga FROM units WHERE status = 'ready' AND q_state = 4 ORDER BY course_id, idx LIMIT 1",
        [],
        |r| {
            let (source, orga): (String, Option<String>) = (r.get(1)?, r.get(2)?);
            Ok(Task::Recheck { unit_id: r.get(0)?, text: crate::import::learn_text(&source, orga.as_deref()) })
        },
    )
    .optional()
}
