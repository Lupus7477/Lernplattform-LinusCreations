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

/// Version 5: Live-Battles zwischen zwei Konten. `a_qs`/`b_qs` = Fragen-IDs je Seite, an jeder Stelle dieselbe Aufgabenart.
/// `started_ms` = Start nach dem Countdown (ms), `gave_up` = wer aufgegeben hat. `ms` je Antwort = Zeit seit dem Start.
const V5: &str = "
CREATE TABLE battles(id INTEGER PRIMARY KEY,
  a_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, a_course INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  b_id INTEGER REFERENCES users(id) ON DELETE CASCADE, b_course INTEGER REFERENCES courses(id) ON DELETE CASCADE,
  a_qs TEXT NOT NULL DEFAULT '[]', b_qs TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL, started_ms INTEGER, gave_up INTEGER);
CREATE TABLE battle_answers(battle_id INTEGER NOT NULL REFERENCES battles(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL, idx INTEGER NOT NULL, correct INTEGER NOT NULL, ms INTEGER NOT NULL,
  PRIMARY KEY(battle_id, user_id, idx)) WITHOUT ROWID;
CREATE INDEX battles_a_course ON battles(a_course);
CREATE INDEX battles_b_course ON battles(b_course);
PRAGMA user_version = 5;
";

/// Version 6: Motivation. `users.freezes` = Pausentage auf Vorrat (0–2), `frozen` = Tage (julianday wie in den Statistiken),
/// die ein Pausentag überbrückt hat. `badges` = freigeschaltete Erfolge. `cheers` = von der KI geschriebene, vom Code geprüfte
/// Motivationstexte je Anlass (für alle gleich).
const V6: &str = "
ALTER TABLE users ADD COLUMN freezes INTEGER NOT NULL DEFAULT 0;
CREATE TABLE frozen(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  day INTEGER NOT NULL, PRIMARY KEY(user_id, day)) WITHOUT ROWID;
CREATE TABLE badges(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  id TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY(user_id, id)) WITHOUT ROWID;
CREATE TABLE cheers(id INTEGER PRIMARY KEY, occasion TEXT NOT NULL, text TEXT NOT NULL);
CREATE INDEX cheers_occasion ON cheers(occasion);
PRAGMA user_version = 6;
";

/// Version 7: Motivation pro Kurs. `lessons.course_id` = Kurs der Lektion (auch Falsch- und Prüfungsrunden ohne Einheit).
/// `courses.motivation` = von der KI geschriebene, vom Code geprüfte Titel und Erfolgsnamen als JSON (NULL = noch nicht geschrieben).
/// `course_badges` = Erfolge je Kurs; `badges` behält nur die allgemeinen (Serie). `cheers` gehören jetzt zu einem Kurs.
/// `motivation_rounds` = KI-Runden je Kurs und Teil („names“ oder Anlass), damit der Worker nie endlos wiederholt.
const V7: &str = "
ALTER TABLE lessons ADD COLUMN course_id INTEGER REFERENCES courses(id) ON DELETE SET NULL;
UPDATE lessons SET course_id = (SELECT course_id FROM units WHERE units.id = lessons.unit_id);
CREATE INDEX lessons_course ON lessons(user_id, course_id);
ALTER TABLE courses ADD COLUMN motivation TEXT;
CREATE TABLE course_badges(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE, id TEXT NOT NULL, at INTEGER NOT NULL,
  PRIMARY KEY(user_id, course_id, id)) WITHOUT ROWID;
-- Bisherige Kurs-Erfolge mit Datum behalten: sie gehen an den Kurs, in dem die Person am meisten gelernt hat.
INSERT OR IGNORE INTO course_badges(user_id, course_id, id, at)
  SELECT user_id, c, id, at FROM (SELECT b.user_id, b.id, b.at,
    (SELECT l.course_id FROM lessons l WHERE l.user_id = b.user_id AND l.course_id IS NOT NULL GROUP BY l.course_id ORDER BY COUNT(*) DESC, l.course_id LIMIT 1) AS c
    FROM badges b WHERE b.id NOT IN ('week', 'month', 'comeback')) WHERE c IS NOT NULL;
DELETE FROM badges WHERE id NOT IN ('week', 'month', 'comeback');
DROP TABLE cheers;
CREATE TABLE cheers(id INTEGER PRIMARY KEY, course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  occasion TEXT NOT NULL, text TEXT NOT NULL);
CREATE INDEX cheers_course ON cheers(course_id, occasion);
CREATE TABLE motivation_rounds(course_id INTEGER NOT NULL REFERENCES courses(id) ON DELETE CASCADE,
  part TEXT NOT NULL, n INTEGER NOT NULL, PRIMARY KEY(course_id, part)) WITHOUT ROWID;
DELETE FROM settings WHERE key LIKE 'cheers_round:%';
PRAGMA user_version = 7;
";

/// Version 8: XP zählt der Server. `lesson_answers` = Erstantworten der laufenden Lektion (beim Abschluss verbraucht).
/// `lessons.main` = 1, wenn die Zeile als abgeschlossene Lektion zählt; 0 = Anteil einer Wiederholung an einem weiteren Kurs
/// oder abgebrochene Lektion (zählt nur für XP, Aufgaben und Lerntag).
/// Lektionen ohne Kurs bekommen ihn, wenn die Person nur in einem Kurs lernt. Battles bleiben, wenn Kurs oder Konto
/// gelöscht wird: Kurs → NULL, Konten ohne Fremdschlüssel (beim Löschen setzt der Server die ID auf -id).
const V8: &str = "
CREATE TABLE lesson_answers(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE, correct INTEGER NOT NULL,
  PRIMARY KEY(user_id, question_id)) WITHOUT ROWID;
ALTER TABLE lessons ADD COLUMN main INTEGER NOT NULL DEFAULT 1;
UPDATE lessons SET course_id = (SELECT MIN(l.course_id) FROM lessons l WHERE l.user_id = lessons.user_id)
  WHERE course_id IS NULL AND (SELECT COUNT(DISTINCT l.course_id) FROM lessons l WHERE l.user_id = lessons.user_id) = 1;
CREATE TABLE battles_new(id INTEGER PRIMARY KEY,
  a_id INTEGER NOT NULL, a_course INTEGER REFERENCES courses(id) ON DELETE SET NULL,
  b_id INTEGER, b_course INTEGER REFERENCES courses(id) ON DELETE SET NULL,
  a_qs TEXT NOT NULL DEFAULT '[]', b_qs TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL, started_ms INTEGER, gave_up INTEGER);
INSERT INTO battles_new SELECT id, a_id, a_course, b_id, b_course, a_qs, b_qs, created_at, started_ms, gave_up FROM battles;
DROP TABLE battles;
ALTER TABLE battles_new RENAME TO battles;
CREATE INDEX battles_a_course ON battles(a_course);
CREATE INDEX battles_b_course ON battles(b_course);
PRAGMA user_version = 8;
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
    if version < 5 {
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V5} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
    if version < 6 {
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V6} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
    if version < 7 {
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V7} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
    if version < 8 {
        conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {V8} COMMIT; PRAGMA foreign_keys = ON;"))?;
    }
    Ok(conn)
}

/// Erste Spalte aller Zeilen.
pub fn column<T: rusqlite::types::FromSql>(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> rusqlite::Result<Vec<T>> {
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map(p, |r| r.get(0))?;
    rows.collect()
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
/// 2b. Titel und Erfolgsnamen eines Kurses (ein kurzer Aufruf, gleich nach seinen ersten Fragen; nur genutzte Kurse)
/// 3. noch nicht analysierte Abschnitte
/// 4. Prüfungsthemen der übrigen Einheiten
/// 5. Fragen geänderter Abschnitte prüfen (PDF ersetzt)
/// 6. Motivationstexte eines Kurses schreiben (nur wenn sonst nichts ansteht)
pub enum Task {
    Questions { unit_id: i64, level: i64, text: String, topics: Vec<String> },
    /// `filter` = Organisatorisches aussortieren (aus, wenn der Nutzer „Doch einbeziehen“ gewählt hat)
    Analyze { unit_id: i64, course_id: i64, idx: i64, page_from: i64, page_to: i64, text: String, course: String, filter: bool },
    /// `info` = Titel, Zusammenfassung, Begriffe und Anfang des Lerntexts
    Focus { unit_id: i64, topics: Vec<String>, info: String },
    Recheck { unit_id: i64, text: String },
    /// `part` = „names“ (Titel und Erfolge) oder ein Anlass für Motivationstexte
    Motivation { course_id: i64, part: &'static str },
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
    if let Some(course_id) = crate::motivation::wanted_names(conn) {
        return Ok(Some(Task::Motivation { course_id, part: crate::motivation::NAMES }));
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
    let r = conn
        .query_row("SELECT id, source_text, orga FROM units WHERE status = 'ready' AND q_state = 4 ORDER BY course_id, idx LIMIT 1", [], |r| {
            let (source, orga): (String, Option<String>) = (r.get(1)?, r.get(2)?);
            Ok(Task::Recheck { unit_id: r.get(0)?, text: crate::import::learn_text(&source, orga.as_deref()) })
        })
        .optional()?;
    if r.is_some() {
        return Ok(r);
    }
    Ok(crate::motivation::wanted_cheers(conn).map(|(course_id, part)| Task::Motivation { course_id, part }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lernpfad-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("lernen.db")
    }

    #[test]
    fn v7_v8_keep_courses_badges_and_battles() {
        let path = tmp("v8");
        {
            // Datenbank auf Version 6: Lektionen einer Einheit und eine ohne Einheit, Erfolge, ein altes Battle
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            for v in [V1, V2, V3, V4, V5, V6] {
                conn.execute_batch(&format!("PRAGMA foreign_keys = OFF; BEGIN; {v} COMMIT; PRAGMA foreign_keys = ON;")).unwrap();
            }
            conn.execute_batch(
                "INSERT INTO users(id, name, created_at) VALUES(2, 'Ben', 0);
                 INSERT INTO courses(id, name, created_at) VALUES(5, 'Marketing', 0), (6, 'Makro', 0);
                 INSERT INTO documents(id, course_id, name, pages, created_at) VALUES(1, 5, 'a.pdf', 1, 0);
                 INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text) VALUES(1, 5, 1, 0, 1, 1, 'T', 'x');
                 INSERT INTO lessons(unit_id, xp, correct, total, finished_at) VALUES(1, 20, 5, 6, 100), (NULL, 10, 1, 1, 200);
                 INSERT INTO badges(user_id, id, at) VALUES(1, 'first', 77), (1, 'week', 1);
                 INSERT INTO cheers(occasion, text) VALUES('done', 'Gut gemacht!');
                 INSERT INTO battles(id, a_id, a_course, b_id, b_course, created_at, started_ms) VALUES(1, 1, 5, 2, 6, 0, 0);
                 INSERT INTO battle_answers(battle_id, user_id, idx, correct, ms) VALUES(1, 1, 0, 1, 10);",
            )
            .unwrap();
        }
        let conn = open(&path).unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 8);
        // Lektion ohne Einheit: nur ein Kurs → gehört zu ihm
        let courses: Vec<Option<i64>> = column(&conn, "SELECT course_id FROM lessons ORDER BY id", []).unwrap();
        assert_eq!(courses, vec![Some(5), Some(5)]);
        // Kurs-Erfolg mit altem Datum im Kurs mit den meisten Lektionen, Serien-Erfolg bleibt allgemein
        let kept: (i64, String, i64) = conn.query_row("SELECT course_id, id, at FROM course_badges", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(kept, (5, "first".to_string(), 77));
        assert_eq!(column::<String>(&conn, "SELECT id FROM badges", []).unwrap(), vec!["week".to_string()]);
        // Kurs gelöscht: das Battle (und damit die Siege) bleibt, nur ohne Kurs
        conn.execute("DELETE FROM courses WHERE id = 6", []).unwrap();
        let (b_course, answers): (Option<i64>, i64) =
            conn.query_row("SELECT b_course, (SELECT COUNT(*) FROM battle_answers WHERE battle_id = 1) FROM battles WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((b_course, answers), (None, 1));
    }

    #[test]
    fn names_only_for_courses_in_use() {
        let conn = open(&tmp("wanted-names")).unwrap();
        conn.execute_batch(
            "INSERT INTO courses(id, name, created_at) VALUES(1, 'Alt', 0), (2, 'Alt, aber genutzt', 0), (3, 'Ohne Fragen', strftime('%s', 'now')), (4, 'Neu', strftime('%s', 'now'));
             INSERT INTO documents(id, course_id, name, pages, created_at) VALUES(1, 1, 'a.pdf', 1, 0), (2, 2, 'b.pdf', 1, 0), (4, 4, 'd.pdf', 1, 0);
             INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text) VALUES(1, 1, 1, 0, 1, 1, 'T', 'x'), (2, 2, 2, 0, 1, 1, 'T', 'x'), (4, 4, 4, 0, 1, 1, 'T', 'x');
             INSERT INTO questions(unit_id, kind, prompt, options, answer) VALUES(1, 'mc', 'F', '[]', 'a'), (2, 'mc', 'F', '[]', 'a'), (4, 'mc', 'F', '[]', 'a');
             INSERT INTO lessons(unit_id, xp, correct, total, finished_at, course_id) VALUES(2, 10, 1, 1, strftime('%s', 'now'), 2);",
        )
        .unwrap();
        // Der neue Kurs mit Fragen zuerst, dann der genutzte alte; der ruhende und der ohne Fragen nie
        assert_eq!(crate::motivation::wanted_names(&conn), Some(4));
        conn.execute("UPDATE courses SET motivation = '{}' WHERE id = 4", []).unwrap();
        assert_eq!(crate::motivation::wanted_names(&conn), Some(2));
        conn.execute("UPDATE courses SET motivation = '{}' WHERE id = 2", []).unwrap();
        assert_eq!(crate::motivation::wanted_names(&conn), None);
    }
}
