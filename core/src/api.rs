//! Alle Befehle der Oberfläche an einer Stelle. Tauri-App und Server rufen nur [`call`] auf.
//! Jede Kurs-, Einheiten- und Fragen-ID wird gegen die Rechte des Nutzers geprüft.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::{self, Gen, Msg};
use crate::{db, import, learn, Core, Who};

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Gleiche Meldung für „gibt es nicht“ und „gehört jemand anderem“: so lässt sich nicht durchprobieren, was existiert.
pub const DENIED: &str = "Kein Zugriff";

/// Erste `n` Zeichen (nicht Bytes), damit Umlaute nicht zerschnitten werden.
fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map(|(i, _)| &s[..i]).unwrap_or(s)
}

/// Argumente eines Befehls lesen: `args!(v, { unit_id: i64, retry: bool })` → `(unit_id, retry)`.
/// Die Oberfläche schickt camelCase (`unitId`), wie früher bei Tauri.
macro_rules! args {
    ($v:expr, { $($f:ident : $t:ty),* $(,)? }) => {{
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct A { $($f: $t),* }
        let A { $($f),* } = serde_json::from_value::<A>($v).map_err(err)?;
        ($($f),*)
    }};
}

/// Führt einen Befehl der Oberfläche aus. `on_token` bekommt bei `explain`/`ask` den Text stückweise.
pub fn call(c: &Core, who: &Who, cmd: &str, v: Value, on_token: &mut dyn FnMut(&str)) -> R<Value> {
    fn ok<T: Serialize>(t: T) -> R<Value> {
        serde_json::to_value(t).map_err(err)
    }
    match cmd {
        "ai_status" => ok(c.ai.status()),
        "ai_tiers" => ok(ai::TIERS.iter().map(|t| TierInfo { id: t.id, label: t.label, size: t.size }).collect::<Vec<_>>()),
        "ai_retry" => {
            admin(who)?;
            c.ai.ensure_model();
            ok(())
        }
        "ai_set_tier" => {
            admin(who)?;
            let tier = args!(v, { tier: String });
            db::set_setting(&c.db.lock().unwrap(), "tier", ai::tier(&tier).id).map_err(err)?;
            c.ai.set_tier(&tier);
            ok(())
        }
        "courses" => ok(courses(c, who)?),
        "create_course" => {
            let (name, exam_date) = args!(v, { name: String, exam_date: Option<String> });
            ok(create_course(c, who, &name, exam_date)?)
        }
        "update_course" => {
            let (id, name, exam_date, shared) = args!(v, { id: i64, name: String, exam_date: Option<String>, shared: Option<bool> });
            ok(update_course(c, who, id, &name, exam_date, shared)?)
        }
        "delete_course" => {
            let id = args!(v, { id: i64 });
            let conn = c.db.lock().unwrap();
            owner(role(&conn, who, id)?)?;
            ok(conn.execute("DELETE FROM courses WHERE id = ?1", [id]).map_err(err)?)
        }
        "import_document" => {
            let (course_id, name, pages) = args!(v, { course_id: i64, name: String, pages: Vec<String> });
            owner(role(&c.db.lock().unwrap(), who, course_id)?)?;
            let (_, units) = import::store_document(&c.db, course_id, head(name.trim(), 200), &pages).map_err(err)?;
            if units == 0 {
                return Err("In diesem PDF wurde kein Text gefunden. Ist es eingescannt?".into());
            }
            c.wake();
            ok(Imported { units })
        }
        "course_path" => ok(course_path(c, who, args!(v, { course_id: i64 }))?),
        "unit" => ok(unit(c, who, args!(v, { unit_id: i64 }))?),
        "search_terms" => {
            let query = args!(v, { query: String });
            let like = format!("%{}%", head(query.trim(), 100));
            ok(unit_terms(&c.db.lock().unwrap(), "WHERE (c.owner_id = ?2 OR c.shared = 1) AND (t.term LIKE ?1 OR t.definition LIKE ?1)", params![like, who.id])?)
        }
        "explain" => ok(explain(c, who, args!(v, { unit_id: i64 }), on_token)?),
        "ask" => {
            let (unit_id, messages) = args!(v, { unit_id: Option<i64>, messages: Vec<ChatIn> });
            ok(ask(c, who, unit_id, messages, on_token)?)
        }
        "start_lesson" => ok(start_lesson(c, who, args!(v, { unit_id: Option<i64> }))?),
        "answer" => {
            let (question_id, given, retry) = args!(v, { question_id: i64, given: String, retry: bool });
            ok(answer(c, who, question_id, &given, retry)?)
        }
        "stats" => ok(stats_of(&c.db.lock().unwrap(), who)?),
        "finish_lesson" => {
            let (unit_id, correct, total) = args!(v, { unit_id: Option<i64>, correct: i64, total: i64 });
            ok(finish_lesson(c, who, unit_id, correct, total)?)
        }
        "set_goal" => {
            let xp = args!(v, { xp: i64 });
            ok(db::set_setting(&c.db.lock().unwrap(), &format!("goal:{}", who.id), &xp.clamp(5, 500).to_string()).map_err(err)?)
        }
        "retry_questions" => {
            let unit_id = args!(v, { unit_id: i64 });
            let conn = c.db.lock().unwrap();
            unit_role(&conn, who, unit_id)?;
            conn.execute("UPDATE units SET q_state = 1 WHERE id = ?1 AND q_state = 3", [unit_id]).map_err(err)?;
            drop(conn);
            c.wake();
            ok(())
        }
        // ---------- Fragen bearbeiten ----------
        "unit_questions" => {
            let unit_id = args!(v, { unit_id: i64 });
            let conn = c.db.lock().unwrap();
            unit_role(&conn, who, unit_id)?;
            ok(questions_where(&conn, who, "q.unit_id = ?1 ORDER BY q.id", unit_id)?)
        }
        "question" => {
            let question_id = args!(v, { question_id: i64 });
            let conn = c.db.lock().unwrap();
            question_role(&conn, who, question_id)?;
            ok(questions_where(&conn, who, "q.id = ?1", question_id)?.pop())
        }
        "update_question" => {
            let (question_id, prompt, options, answer, why) =
                args!(v, { question_id: i64, prompt: String, options: Vec<String>, answer: String, why: String });
            ok(update_question(c, who, question_id, prompt, options, answer, why)?)
        }
        "hide_question" => {
            let (question_id, hidden) = args!(v, { question_id: i64, hidden: bool });
            let conn = c.db.lock().unwrap();
            question_role(&conn, who, question_id)?;
            conn.execute(
                "INSERT INTO progress(user_id, question_id, hidden) VALUES(?1, ?2, ?3)
                 ON CONFLICT(user_id, question_id) DO UPDATE SET hidden = excluded.hidden",
                params![who.id, question_id, hidden],
            )
            .map_err(err)?;
            ok(())
        }
        "delete_question" => {
            let question_id = args!(v, { question_id: i64 });
            let conn = c.db.lock().unwrap();
            owner(question_role(&conn, who, question_id)?)?;
            ok(conn.execute("DELETE FROM questions WHERE id = ?1", [question_id]).map_err(err)?)
        }
        _ => Err(format!("Unbekannter Befehl: {cmd}")),
    }
}

// ---------- Rechte ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Role {
    Member,
    Owner,
}

/// Besitzer, Mitglied (Kurs ist geteilt) oder kein Zugriff.
pub fn role(conn: &Connection, who: &Who, course_id: i64) -> R<Role> {
    let row: Option<(i64, bool)> = conn
        .query_row("SELECT owner_id, shared FROM courses WHERE id = ?1", [course_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(err)?;
    match row {
        Some((owner, _)) if owner == who.id => Ok(Role::Owner),
        Some((_, true)) => Ok(Role::Member),
        _ => Err(DENIED.into()),
    }
}

fn unit_role(conn: &Connection, who: &Who, unit_id: i64) -> R<(i64, Role)> {
    let course: i64 = conn
        .query_row("SELECT course_id FROM units WHERE id = ?1", [unit_id], |r| r.get(0))
        .optional()
        .map_err(err)?
        .ok_or(DENIED)?;
    Ok((course, role(conn, who, course)?))
}

fn question_role(conn: &Connection, who: &Who, question_id: i64) -> R<Role> {
    let course: i64 = conn
        .query_row("SELECT u.course_id FROM questions q JOIN units u ON u.id = q.unit_id WHERE q.id = ?1", [question_id], |r| r.get(0))
        .optional()
        .map_err(err)?
        .ok_or(DENIED)?;
    role(conn, who, course)
}

fn owner(r: Role) -> R<()> {
    if r == Role::Owner {
        Ok(())
    } else {
        Err(DENIED.into())
    }
}

fn admin(who: &Who) -> R<()> {
    if who.admin {
        Ok(())
    } else {
        Err(DENIED.into())
    }
}

// ---------- KI ----------

#[derive(Serialize)]
struct TierInfo {
    id: &'static str,
    label: &'static str,
    size: u64,
}

// ---------- Kurse & Import ----------

#[derive(Serialize)]
struct Course {
    id: i64,
    name: String,
    exam_date: Option<String>,
    units: i64,
    ready: i64,
    documents: Vec<String>,
    /// Eigener Kurs (darf bearbeiten, löschen, PDFs hinzufügen)
    mine: bool,
    shared: bool,
    owner: String,
}

fn courses(c: &Core, who: &Who) -> R<Vec<Course>> {
    let conn = c.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.name, c.exam_date,
               (SELECT COUNT(*) FROM units u WHERE u.course_id = c.id),
               (SELECT COUNT(*) FROM units u WHERE u.course_id = c.id AND u.status = 'ready'),
               (SELECT COALESCE(group_concat(d.name, char(10)), '') FROM documents d WHERE d.course_id = c.id),
               c.owner_id = ?1, c.shared, COALESCE(o.name, '')
             FROM courses c LEFT JOIN users o ON o.id = c.owner_id
             WHERE c.owner_id = ?1 OR c.shared = 1 ORDER BY c.created_at",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([who.id], |r| {
            let docs: String = r.get(5)?;
            Ok(Course {
                id: r.get(0)?,
                name: r.get(1)?,
                exam_date: r.get(2)?,
                units: r.get(3)?,
                ready: r.get(4)?,
                documents: docs.lines().map(String::from).collect(),
                mine: r.get(6)?,
                shared: r.get(7)?,
                owner: r.get(8)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

fn course_name(name: &str) -> R<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Gib dem Kurs einen Namen.".into());
    }
    Ok(head(name, 120))
}

fn create_course(c: &Core, who: &Who, name: &str, exam_date: Option<String>) -> R<i64> {
    let conn = c.db.lock().unwrap();
    conn.execute(
        "INSERT INTO courses(name, exam_date, created_at, owner_id) VALUES(?1, ?2, ?3, ?4)",
        params![course_name(name)?, exam_date.filter(|d| !d.is_empty()), db::now(), who.id],
    )
    .map_err(err)?;
    Ok(conn.last_insert_rowid())
}

fn update_course(c: &Core, who: &Who, id: i64, name: &str, exam_date: Option<String>, shared: Option<bool>) -> R<()> {
    let conn = c.db.lock().unwrap();
    owner(role(&conn, who, id)?)?;
    conn.execute(
        "UPDATE courses SET name = ?1, exam_date = ?2, shared = COALESCE(?3, shared) WHERE id = ?4",
        params![course_name(name)?, exam_date.filter(|d| !d.is_empty()), shared, id],
    )
    .map(|_| ())
    .map_err(err)
}

#[derive(Serialize)]
struct Imported {
    units: usize,
}

// ---------- Lernpfad & Einheit ----------

#[derive(Serialize)]
struct UnitNode {
    id: i64,
    idx: i64,
    document_id: i64,
    document: String,
    title: String,
    summary: String,
    status: String,
    q_state: i64,
    questions: i64,
    mastery: f64,
    due: i64,
    lessons: i64,
}

fn course_path(c: &Core, who: &Who, course_id: i64) -> R<Vec<UnitNode>> {
    let conn = c.db.lock().unwrap();
    role(&conn, who, course_id)?;
    // Sichtbare Fragen = nicht von mir ausgeblendet; Box und Fälligkeit sind mein eigener Fortschritt.
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.idx, u.title, u.summary, u.status, u.q_state,
               (SELECT COUNT(*) FROM questions q LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?3
                 WHERE q.unit_id = u.id AND COALESCE(p.hidden, 0) = 0),
               (SELECT COALESCE(AVG(COALESCE(p.box, 0)), 0.0) FROM questions q LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?3
                 WHERE q.unit_id = u.id AND COALESCE(p.hidden, 0) = 0),
               (SELECT COUNT(*) FROM questions q JOIN progress p ON p.question_id = q.id AND p.user_id = ?3
                 WHERE q.unit_id = u.id AND p.hidden = 0 AND p.box > 0 AND p.due_at <= ?2),
               (SELECT COUNT(*) FROM lessons l WHERE l.unit_id = u.id AND l.user_id = ?3),
               u.document_id, d.name
             FROM units u JOIN documents d ON d.id = u.document_id WHERE u.course_id = ?1 ORDER BY u.idx",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(params![course_id, db::now(), who.id], |r| {
            Ok(UnitNode {
                id: r.get(0)?,
                idx: r.get(1)?,
                title: r.get(2)?,
                summary: r.get(3)?,
                status: r.get(4)?,
                q_state: r.get(5)?,
                questions: r.get(6)?,
                mastery: r.get(7)?,
                due: r.get(8)?,
                lessons: r.get(9)?,
                document_id: r.get(10)?,
                document: r.get(11)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

#[derive(Serialize)]
struct TermOut {
    term: String,
    definition: String,
    unit_id: i64,
    unit_title: String,
}

#[derive(Serialize)]
struct UnitDetail {
    id: i64,
    course_id: i64,
    title: String,
    summary: String,
    explanation: Option<String>,
    page_from: i64,
    page_to: i64,
    document: String,
    terms: Vec<TermOut>,
}

fn unit_terms(conn: &Connection, where_sql: &str, p: impl rusqlite::Params) -> R<Vec<TermOut>> {
    let sql = format!(
        "SELECT t.term, t.definition, u.id, u.title FROM terms t JOIN units u ON u.id = t.unit_id JOIN courses c ON c.id = u.course_id
         {where_sql} ORDER BY t.term COLLATE NOCASE LIMIT 300"
    );
    let mut stmt = conn.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(p, |r| Ok(TermOut { term: r.get(0)?, definition: r.get(1)?, unit_id: r.get(2)?, unit_title: r.get(3)? }))
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

fn unit(c: &Core, who: &Who, unit_id: i64) -> R<UnitDetail> {
    let conn = c.db.lock().unwrap();
    unit_role(&conn, who, unit_id)?;
    let mut d = conn
        .query_row(
            "SELECT u.id, u.course_id, u.title, u.summary, u.explanation, u.page_from, u.page_to, d.name
             FROM units u JOIN documents d ON d.id = u.document_id WHERE u.id = ?1",
            [unit_id],
            |r| {
                Ok(UnitDetail {
                    id: r.get(0)?,
                    course_id: r.get(1)?,
                    title: r.get(2)?,
                    summary: r.get(3)?,
                    explanation: r.get(4)?,
                    page_from: r.get(5)?,
                    page_to: r.get(6)?,
                    document: r.get(7)?,
                    terms: vec![],
                })
            },
        )
        .map_err(err)?;
    d.terms = unit_terms(&conn, "WHERE u.id = ?1", [unit_id])?;
    Ok(d)
}

fn unit_text(c: &Core, who: &Who, unit_id: i64) -> R<(String, String, Option<String>)> {
    let conn = c.db.lock().unwrap();
    unit_role(&conn, who, unit_id)?;
    conn.query_row("SELECT title, source_text, explanation FROM units WHERE id = ?1", [unit_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(err)
}

/// Ausführliche Erklärung: einmal gestreamt erzeugen, danach aus der Datenbank (gilt für alle im Kurs).
fn explain(c: &Core, who: &Who, unit_id: i64, on_token: &mut dyn FnMut(&str)) -> R<String> {
    let (title, text, cached) = unit_text(c, who, unit_id)?;
    if let Some(x) = cached {
        return Ok(x);
    }
    let user = format!(
        "Erkläre das Thema „{title}“ anhand dieses Lernmaterials so, dass es jemand ohne Vorwissen versteht.\n\n\
         Lernmaterial:\n<<<\n{text}\n>>>\n\n\
         Schreibe Markdown mit kurzen Absätzen und **fetten** Fachbegriffen, genau in diesem Aufbau:\n\
         ## Worum geht's?\n1–2 Sätze: was ist das und warum ist es wichtig.\n\
         ## So funktioniert es\nSchritt für Schritt, ein Gedanke pro Absatz.\n\
         ## Typische Denkfehler\n2–3 Punkte: was oft falsch verstanden wird und warum.\n\
         ## Beispiel\nEin konkretes, einfaches Beispiel.\n\
         Zum Schluss: > **Merke:** ein zentraler Satz."
    );
    let out = c
        .ai
        .generate(Gen { messages: vec![Msg { role: "user", content: user }], grammar: None, max_tokens: 1100, temperature: 0.5 }, |p| on_token(p))
        .map_err(err)?;
    c.db.lock().unwrap().execute("UPDATE units SET explanation = ?1 WHERE id = ?2", params![out, unit_id]).map_err(err)?;
    Ok(out)
}

#[derive(Deserialize)]
struct ChatIn {
    role: String,
    content: String,
}

/// „Frag die KI“: nur der Abschnitt der Einheit als Kontext, nur die letzten 6 Nachrichten.
fn ask(c: &Core, who: &Who, unit_id: Option<i64>, messages: Vec<ChatIn>, on_token: &mut dyn FnMut(&str)) -> R<String> {
    let mut system = String::from(
        "Du bist ein geduldiger Lern-Tutor. Antworte auf Deutsch, kurz und verständlich (höchstens 5 Sätze). \
         Wenn etwas nicht im Lernmaterial steht, sag das ehrlich.",
    );
    if let Some(id) = unit_id {
        let (title, text, _) = unit_text(c, who, id)?;
        system.push_str(&format!("\n\nThema: {title}\nLernmaterial:\n<<<\n{}\n>>>", head(&text, 4000)));
    }
    let mut msgs = vec![Msg { role: "system", content: system }];
    let skip = messages.len().saturating_sub(6);
    for m in messages.into_iter().skip(skip) {
        let role = if m.role == "assistant" { "assistant" } else { "user" };
        msgs.push(Msg { role, content: head(&m.content, 1500).to_string() });
    }
    c.ai.generate(Gen { messages: msgs, grammar: None, max_tokens: 500, temperature: 0.5 }, |p| on_token(p)).map_err(err)
}

// ---------- Lektion ----------

#[derive(Serialize)]
struct Exercise {
    id: Option<i64>,
    kind: String,
    prompt: String,
    options: Vec<String>,
    pairs: Vec<[String; 2]>,
}

#[derive(Serialize)]
struct LessonOut {
    ready: bool,
    /// Fragen-Erstellung fehlgeschlagen → Nutzer entscheidet über neuen Versuch (`retry_questions`)
    failed: bool,
    exercises: Vec<Exercise>,
}

const LESSON_SIZE: usize = 8;

fn start_lesson(c: &Core, who: &Who, unit_id: Option<i64>) -> R<LessonOut> {
    let conn = c.db.lock().unwrap();
    let now = db::now();
    let mut rng = learn::Shuffler::new();

    let mut picked: Vec<(i64, String, String, String)>;
    if let Some(id) = unit_id {
        let (course_id, _) = unit_role(&conn, who, id)?;
        let (idx, q_state): (i64, i64) = conn.query_row("SELECT idx, q_state FROM units WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
        // Nächste Einheit schon vorbereiten, während diese geübt wird.
        let _ = conn.execute("UPDATE units SET q_state = 1 WHERE course_id = ?1 AND idx = ?2 AND q_state = 0", params![course_id, idx + 1]);
        // Fällige zuerst, dann neue, dann die mit der niedrigsten Box. Ausgeblendete nie.
        picked = query_questions(
            &conn,
            "SELECT q.id, q.kind, q.prompt, q.options FROM questions q
             LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?4
             WHERE q.unit_id = ?1 AND COALESCE(p.hidden, 0) = 0
             ORDER BY CASE WHEN p.box > 0 AND p.due_at <= ?2 THEN 0 WHEN COALESCE(p.box, 0) = 0 THEN 1 ELSE 2 END,
                      COALESCE(p.box, 0), COALESCE(p.due_at, 0), q.id
             LIMIT ?3",
            params![id, now, LESSON_SIZE as i64, who.id],
        )?;
        if picked.is_empty() {
            // Noch keine Fragen (0) oder alle ausgeblendet/gelöscht (2) → die KI schreibt eine neue Runde.
            if q_state == 0 || q_state == 2 {
                let _ = conn.execute("UPDATE units SET q_state = 1 WHERE id = ?1", [id]);
            }
            drop(conn);
            c.wake();
            return Ok(LessonOut { ready: false, failed: q_state == 3, exercises: vec![] });
        }
        // Alles sitzt (keine neuen, Ø Box ≥ 3) → nächste, schwierigere Fragenrunde vorbereiten.
        let (new, avg): (i64, f64) = conn
            .query_row(
                "SELECT SUM(COALESCE(p.box, 0) = 0), AVG(COALESCE(p.box, 0)) FROM questions q
                 LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?2
                 WHERE q.unit_id = ?1 AND COALESCE(p.hidden, 0) = 0",
                params![id, who.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        if new == 0 && avg >= 3.0 && q_state == 2 {
            let _ = conn.execute("UPDATE units SET q_state = 1 WHERE id = ?1", [id]);
            c.wake();
        }
    } else {
        picked = query_questions(
            &conn,
            "SELECT q.id, q.kind, q.prompt, q.options FROM progress p
             JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id JOIN courses c ON c.id = u.course_id
             WHERE p.user_id = ?3 AND p.hidden = 0 AND p.box > 0 AND p.due_at <= ?1 AND (c.owner_id = ?3 OR c.shared = 1)
             ORDER BY p.due_at LIMIT ?2",
            params![now, 10, who.id],
        )?;
    }

    rng.shuffle(&mut picked);
    let ids: Vec<String> = picked.iter().map(|p| p.0.to_string()).collect();
    let mut exercises: Vec<Exercise> = picked
        .into_iter()
        .map(|(id, kind, prompt, options)| {
            let mut options: Vec<String> = serde_json::from_str(&options).unwrap_or_default();
            if kind != "true_false" {
                rng.shuffle(&mut options);
            }
            Exercise { id: Some(id), kind, prompt, options, pairs: vec![] }
        })
        .collect();

    // Zuordnen-Runde aus den Begriffen – ganz ohne KI.
    let term_sql = match unit_id {
        Some(_) => "SELECT term, definition FROM terms WHERE unit_id = ?1".to_string(),
        None if !ids.is_empty() => format!(
            "SELECT term, definition FROM terms WHERE unit_id IN (SELECT unit_id FROM questions WHERE id IN ({}))",
            ids.join(",")
        ),
        None => String::new(),
    };
    if !term_sql.is_empty() {
        let mut stmt = conn.prepare(&term_sql).map_err(err)?;
        let mut terms: Vec<[String; 2]> = stmt
            .query_map(rusqlite::params_from_iter(unit_id), |r| Ok([r.get(0)?, r.get(1)?]))
            .map_err(err)?
            .filter_map(Result::ok)
            .collect();
        rng.shuffle(&mut terms);
        terms.truncate(4);
        if terms.len() >= 3 {
            let at = exercises.len().min(3);
            exercises.insert(at, Exercise { id: None, kind: "match".into(), prompt: "Ordne die Begriffe zu".into(), options: vec![], pairs: terms });
        }
    }
    Ok(LessonOut { ready: true, failed: false, exercises })
}

fn query_questions(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> R<Vec<(i64, String, String, String)>> {
    let mut stmt = conn.prepare(sql).map_err(err)?;
    let rows = stmt.query_map(p, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

#[derive(Serialize)]
struct AnswerOut {
    correct: bool,
    expected: String,
    why: String,
}

/// Prüft ohne KI. `retry` = Wiederholung innerhalb derselben Lektion → Box bleibt unverändert.
fn answer(c: &Core, who: &Who, question_id: i64, given: &str, retry: bool) -> R<AnswerOut> {
    let conn = c.db.lock().unwrap();
    question_role(&conn, who, question_id)?;
    let (kind, expected, why, boxed): (String, String, String, i64) = conn
        .query_row(
            "SELECT q.kind, q.answer, q.why, COALESCE(p.box, 0) FROM questions q
             LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?2 WHERE q.id = ?1",
            params![question_id, who.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(err)?;
    let correct = learn::check(&kind, &expected, given);
    if !retry {
        let b = learn::next_box(boxed, correct);
        conn.execute(
            "INSERT INTO progress(user_id, question_id, box, due_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(user_id, question_id) DO UPDATE SET box = excluded.box, due_at = excluded.due_at",
            params![who.id, question_id, b, learn::due_at(b, db::now())],
        )
        .map_err(err)?;
    }
    Ok(AnswerOut { correct, expected, why })
}

#[derive(Serialize)]
struct DayXp {
    day: String,
    xp: i64,
}

#[derive(Serialize)]
struct Stats {
    streak: i64,
    xp_today: i64,
    goal: i64,
    total_xp: i64,
    lessons: i64,
    due: i64,
    week: Vec<DayXp>,
    active_days: Vec<String>,
}

// ponytail: Tagesgrenzen in der Zeitzone des Geräts bzw. Servers. Zeitzone pro Nutzer, falls Leute aus verschiedenen Zeitzonen einen Server teilen.
fn stats_of(conn: &Connection, who: &Who) -> R<Stats> {
    let days: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT CAST(julianday(date(finished_at, 'unixepoch', 'localtime')) AS INTEGER) FROM lessons WHERE user_id = ?1")
            .map_err(err)?;
        let rows = stmt.query_map([who.id], |r| r.get(0)).map_err(err)?;
        rows.filter_map(Result::ok).collect()
    };
    let today: i64 = conn.query_row("SELECT CAST(julianday(date('now', 'localtime')) AS INTEGER)", [], |r| r.get(0)).map_err(err)?;
    let xp_on = |offset: i64| -> R<DayXp> {
        conn.query_row(
            "SELECT date('now', 'localtime', ?1), COALESCE((SELECT SUM(xp) FROM lessons
               WHERE user_id = ?2 AND date(finished_at, 'unixepoch', 'localtime') = date('now', 'localtime', ?1)), 0)",
            params![format!("-{offset} days"), who.id],
            |r| Ok(DayXp { day: r.get(0)?, xp: r.get(1)? }),
        )
        .map_err(err)
    };
    let week = (0..7).rev().map(xp_on).collect::<R<Vec<_>>>()?;
    let active_days = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT date(finished_at, 'unixepoch', 'localtime') FROM lessons WHERE user_id = ?2 AND finished_at >= ?1")
            .map_err(err)?;
        let rows = stmt.query_map(params![db::now() - 42 * 86_400, who.id], |r| r.get(0)).map_err(err)?;
        rows.filter_map(Result::ok).collect()
    };
    let (total_xp, lessons): (i64, i64) = conn
        .query_row("SELECT COALESCE(SUM(xp), 0), COUNT(*) FROM lessons WHERE user_id = ?1", [who.id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(err)?;
    let due: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM progress p JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id JOIN courses c ON c.id = u.course_id
             WHERE p.user_id = ?2 AND p.hidden = 0 AND p.box > 0 AND p.due_at <= ?1 AND (c.owner_id = ?2 OR c.shared = 1)",
            params![db::now(), who.id],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(Stats {
        streak: learn::streak(&days, today),
        xp_today: week.last().map(|d| d.xp).unwrap_or(0),
        goal: db::setting(conn, &format!("goal:{}", who.id)).and_then(|g| g.parse().ok()).unwrap_or(20),
        total_xp,
        lessons,
        due,
        week,
        active_days,
    })
}

#[derive(Serialize)]
struct Finished {
    xp: i64,
    stats: Stats,
}

fn finish_lesson(c: &Core, who: &Who, unit_id: Option<i64>, correct: i64, total: i64) -> R<Finished> {
    let conn = c.db.lock().unwrap();
    if let Some(id) = unit_id {
        unit_role(&conn, who, id)?;
    }
    // Werte kommen vom Client: auf eine echte Lektion begrenzen
    let total = total.clamp(0, 50);
    let xp = learn::lesson_xp(correct.clamp(0, total), total);
    conn.execute(
        "INSERT INTO lessons(unit_id, xp, correct, total, finished_at, user_id) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![unit_id, xp, correct.clamp(0, total), total, db::now(), who.id],
    )
    .map_err(err)?;
    Ok(Finished { xp, stats: stats_of(&conn, who)? })
}

// ---------- Fragen bearbeiten ----------

#[derive(Serialize)]
struct QuestionOut {
    id: i64,
    unit_id: i64,
    kind: String,
    prompt: String,
    options: Vec<String>,
    answer: String,
    why: String,
    /// Von mir ausgeblendet
    hidden: bool,
    /// Mein Kurs → darf für alle löschen
    mine: bool,
    shared: bool,
}

fn questions_where(conn: &Connection, who: &Who, where_sql: &str, id: i64) -> R<Vec<QuestionOut>> {
    let sql = format!(
        "SELECT q.id, q.unit_id, q.kind, q.prompt, q.options, q.answer, q.why, COALESCE(p.hidden, 0), c.owner_id = ?2, c.shared
         FROM questions q JOIN units u ON u.id = q.unit_id JOIN courses c ON c.id = u.course_id
         LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?2
         WHERE {where_sql}"
    );
    let mut stmt = conn.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(params![id, who.id], |r| {
            let options: String = r.get(4)?;
            Ok(QuestionOut {
                id: r.get(0)?,
                unit_id: r.get(1)?,
                kind: r.get(2)?,
                prompt: r.get(3)?,
                options: serde_json::from_str(&options).unwrap_or_default(),
                answer: r.get(5)?,
                why: r.get(6)?,
                hidden: r.get(7)?,
                mine: r.get(8)?,
                shared: r.get(9)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

/// Ändert die Frage für alle im Kurs. Dieselbe Formprüfung wie bei KI-Fragen, nur ohne die Stil-Filter.
fn update_question(c: &Core, who: &Who, question_id: i64, prompt: String, options: Vec<String>, answer: String, why: String) -> R<QuestionOut> {
    let conn = c.db.lock().unwrap();
    question_role(&conn, who, question_id)?;
    let kind: String = conn.query_row("SELECT kind FROM questions WHERE id = ?1", [question_id], |r| r.get(0)).map_err(err)?;
    let too_long = prompt.chars().count() > 600 || why.chars().count() > 1000 || options.len() > 6 || options.iter().any(|o| o.chars().count() > 200);
    if too_long {
        return Err("Die Frage ist zu lang.".into());
    }
    let q = import::fix(import::Question { kind, prompt, options, answer, why }, false).ok_or(
        "So geht die Frage nicht: Sie braucht mindestens 8 Zeichen, die richtige Antwort muss unter den Optionen stehen, \
         Multiple Choice braucht 4 verschiedene Optionen und ein Lückentext genau eine Lücke ___.",
    )?;
    conn.execute(
        "UPDATE questions SET prompt = ?1, options = ?2, answer = ?3, why = ?4 WHERE id = ?5",
        params![q.prompt, serde_json::to_string(&q.options).map_err(err)?, q.answer, q.why, question_id],
    )
    .map_err(err)?;
    questions_where(&conn, who, "q.id = ?1", question_id)?.pop().ok_or_else(|| DENIED.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex};

    /// Core ohne Modell-Download: KI bleibt „fehlt“, alles andere läuft echt.
    fn core(dir: &std::path::Path) -> Core {
        let conn = db::open(&dir.join("lernen.db")).unwrap();
        let (wake, _) = mpsc::channel();
        Core { db: Arc::new(Mutex::new(conn)), ai: crate::Ai::new(dir.join("models"), "kompakt", Box::new(|_| {})), wake }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lernpfad-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(c: &Core, who: &Who, cmd: &str, v: Value) -> R<Value> {
        call(c, who, cmd, v, &mut |_| {})
    }

    /// Kurs von Nutzer 1 mit einer fertigen Einheit und zwei Fragen.
    fn seed(c: &Core) -> (i64, i64, i64, i64) {
        let conn = c.db.lock().unwrap();
        conn.execute_batch(
            "INSERT INTO users(id, name, created_at) VALUES(2, 'Kommilitone', 0);
             INSERT INTO courses(id, name, created_at, owner_id) VALUES(10, 'Mikro', 0, 1);
             INSERT INTO documents(id, course_id, name, pages, created_at) VALUES(20, 10, 'vl.pdf', 3, 0);
             INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text, status, q_state)
               VALUES(30, 10, 20, 0, 1, 3, 'Nachfrage', 'Text', 'ready', 2);
             INSERT INTO questions(id, unit_id, kind, prompt, options, answer) VALUES
               (40, 30, 'true_false', 'Die Nachfragekurve fällt.', '[\"Richtig\",\"Falsch\"]', 'Richtig'),
               (41, 30, 'mc', 'Was steht auf der senkrechten Achse?', '[\"Preis\",\"Menge\",\"Kosten\",\"Nutzen\"]', 'Preis');",
        )
        .unwrap();
        (10, 30, 40, 41)
    }

    const U2: Who = Who { id: 2, admin: false };

    #[test]
    fn migrates_v0_progress() {
        let dir = tmp("migrate");
        let path = dir.join("lernen.db");
        {
            // Datenbank im alten Format (Version 0) mit Fortschritt in `questions`
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(db::SCHEMA).unwrap();
            conn.execute_batch(
                "INSERT INTO courses(id, name, created_at) VALUES(1, 'Alt', 0);
                 INSERT INTO documents(id, course_id, name, pages, created_at) VALUES(1, 1, 'a.pdf', 1, 0);
                 INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text) VALUES(1, 1, 1, 0, 1, 1, 'T', 'x');
                 INSERT INTO questions(id, unit_id, kind, prompt, options, answer, box, due_at) VALUES(1, 1, 'mc', 'Frage eins?', '[]', 'a', 3, 99);
                 INSERT INTO lessons(unit_id, xp, correct, total, finished_at) VALUES(1, 20, 5, 6, 100);
                 INSERT INTO settings VALUES('goal', '30');",
            )
            .unwrap();
        }
        let conn = db::open(&path).unwrap();
        assert!(dir.join("lernen.v0.bak").exists());
        let (b, d): (i64, i64) = conn.query_row("SELECT box, due_at FROM progress WHERE user_id = 1 AND question_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((b, d), (3, 99));
        let owner: i64 = conn.query_row("SELECT owner_id FROM courses WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(owner, 1);
        assert_eq!(db::setting(&conn, "goal:1").as_deref(), Some("30"));
        let lessons: i64 = conn.query_row("SELECT COUNT(*) FROM lessons WHERE user_id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(lessons, 1);
        drop(conn);
        db::open(&path).unwrap(); // zweites Öffnen: keine erneute Migration
    }

    #[test]
    fn foreign_course_is_denied() {
        let dir = tmp("denied");
        let c = core(&dir);
        let (course, unit, q, _) = seed(&c);
        for (cmd, v) in [
            ("course_path", serde_json::json!({ "courseId": course })),
            ("unit", serde_json::json!({ "unitId": unit })),
            ("start_lesson", serde_json::json!({ "unitId": unit })),
            ("answer", serde_json::json!({ "questionId": q, "given": "Richtig", "retry": false })),
            ("update_question", serde_json::json!({ "questionId": q, "prompt": "Neu formuliert hier", "options": [], "answer": "Richtig", "why": "" })),
            ("hide_question", serde_json::json!({ "questionId": q, "hidden": true })),
            ("unit_questions", serde_json::json!({ "unitId": unit })),
            ("unit", serde_json::json!({ "unitId": 99999 })),
        ] {
            assert_eq!(run(&c, &U2, cmd, v).unwrap_err(), DENIED, "{cmd}");
        }
        assert_eq!(run(&c, &U2, "courses", Value::Null).unwrap(), serde_json::json!([]));
        assert_eq!(run(&c, &U2, "ai_set_tier", serde_json::json!({ "tier": "kompakt" })).unwrap_err(), DENIED);
    }

    #[test]
    fn shared_course_edit_for_all_hide_for_me() {
        let dir = tmp("shared");
        let c = core(&dir);
        let (course, unit, q, _) = seed(&c);
        run(&c, &crate::LOCAL, "update_course", serde_json::json!({ "id": course, "name": "Mikro", "examDate": null, "shared": true })).unwrap();

        // Mitglied ändert die Frage → gilt für den Besitzer mit
        run(&c, &U2, "update_question", serde_json::json!({ "questionId": q, "prompt": "Die Nachfragekurve fällt normalerweise.", "options": [], "answer": "richtig", "why": "Gesetz der Nachfrage" })).unwrap();
        let mine = run(&c, &crate::LOCAL, "question", serde_json::json!({ "questionId": q })).unwrap();
        assert_eq!(mine["prompt"], "Die Nachfragekurve fällt normalerweise.");
        assert_eq!(mine["answer"], "Richtig");

        // Mitglied blendet aus → nur für sich
        run(&c, &U2, "hide_question", serde_json::json!({ "questionId": q, "hidden": true })).unwrap();
        let ids = |who: &Who| -> Vec<i64> {
            let l = run(&c, who, "start_lesson", serde_json::json!({ "unitId": unit })).unwrap();
            l["exercises"].as_array().unwrap().iter().filter_map(|e| e["id"].as_i64()).collect()
        };
        assert!(!ids(&U2).contains(&q));
        assert!(ids(&crate::LOCAL).contains(&q));

        // Nur der Besitzer löscht für alle
        assert_eq!(run(&c, &U2, "delete_question", serde_json::json!({ "questionId": q })).unwrap_err(), DENIED);
        run(&c, &crate::LOCAL, "delete_question", serde_json::json!({ "questionId": q })).unwrap();
        assert!(!ids(&crate::LOCAL).contains(&q));

        // Ungültige Bearbeitung wird abgelehnt
        assert!(run(&c, &U2, "update_question", serde_json::json!({ "questionId": 41, "prompt": "Achse?", "options": ["Preis"], "answer": "Preis", "why": "" })).is_err());
    }

    #[test]
    fn all_hidden_requests_new_round() {
        let dir = tmp("regen");
        let c = core(&dir);
        let (_, unit, q1, q2) = seed(&c);
        for q in [q1, q2] {
            run(&c, &crate::LOCAL, "hide_question", serde_json::json!({ "questionId": q, "hidden": true })).unwrap();
        }
        let l = run(&c, &crate::LOCAL, "start_lesson", serde_json::json!({ "unitId": unit })).unwrap();
        assert_eq!(l["ready"], false);
        let q_state: i64 = c.db.lock().unwrap().query_row("SELECT q_state FROM units WHERE id = ?1", [unit], |r| r.get(0)).unwrap();
        assert_eq!(q_state, 1);
    }
}
