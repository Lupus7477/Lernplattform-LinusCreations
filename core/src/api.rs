//! Alle Befehle der Oberfläche an einer Stelle. Tauri-App und Server rufen nur [`call`] auf.
//! Jede Kurs-, Einheiten- und Fragen-ID wird gegen die Rechte des Nutzers geprüft.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::{self, Gen, Msg};
use crate::learn::{Group, Spot};
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
            let (course_id, name, pages, figures) = args!(v, { course_id: i64, name: String, pages: Vec<String>, figures: Option<Vec<import::FigureIn>> });
            owner(role(&c.db.lock().unwrap(), who, course_id)?)?;
            let (_, units, figures) = import::store_document(&c.db, course_id, head(name.trim(), 200), &pages, &figures.unwrap_or_default()).map_err(err)?;
            if units == 0 {
                return Err("In diesem PDF wurde kein Text gefunden. Ist es eingescannt?".into());
            }
            c.wake();
            ok(Imported { units, figures })
        }
        // Grafiken für ein schon eingelesenes PDF nachladen (dieselbe Datei noch einmal gewählt)
        "import_figures" => {
            let (document_id, figures) = args!(v, { document_id: i64, figures: Vec<import::FigureIn> });
            let conn = c.db.lock().unwrap();
            let course: i64 = conn
                .query_row("SELECT course_id FROM documents WHERE id = ?1", [document_id], |r| r.get(0))
                .optional()
                .map_err(err)?
                .ok_or(DENIED)?;
            owner(role(&conn, who, course)?)?;
            let figures = import::store_figures(&conn, document_id, &figures).map_err(err)?;
            ok(Imported { units: 0, figures })
        }
        "figure" => {
            let figure_id = args!(v, { figure_id: i64 });
            let conn = c.db.lock().unwrap();
            let unit_id: i64 = conn.query_row("SELECT unit_id FROM figures WHERE id = ?1", [figure_id], |r| r.get(0)).optional().map_err(err)?.ok_or(DENIED)?;
            unit_role(&conn, who, unit_id)?;
            ok(figures_where(&conn, "id = ?1", figure_id)?.pop())
        }
        "unit_figures" => {
            let unit_id = args!(v, { unit_id: i64 });
            let conn = c.db.lock().unwrap();
            unit_role(&conn, who, unit_id)?;
            ok(figures_where(&conn, "unit_id = ?1 ORDER BY page IS NULL, page, id", unit_id)?)
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
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                question_id: i64,
                #[serde(flatten)]
                q: QuestionIn,
            }
            let a: A = serde_json::from_value(v).map_err(err)?;
            ok(update_question(c, who, a.question_id, a.q)?)
        }
        // Eigene Aufgabe auf der Lernkarte: gilt wie Bearbeiten für alle im Kurs
        "create_question" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                unit_id: i64,
                kind: String,
                #[serde(flatten)]
                q: QuestionIn,
            }
            let a: A = serde_json::from_value(v).map_err(err)?;
            ok(create_question(c, who, a.unit_id, a.kind, a.q)?)
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

#[derive(Serialize, Deserialize)]
struct DocOut {
    id: i64,
    name: String,
}

#[derive(Serialize)]
struct Course {
    id: i64,
    name: String,
    exam_date: Option<String>,
    units: i64,
    ready: i64,
    documents: Vec<DocOut>,
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
               (SELECT json_group_array(json_object('id', d.id, 'name', d.name)) FROM (SELECT id, name FROM documents WHERE course_id = c.id ORDER BY id) d),
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
                documents: serde_json::from_str(&docs).unwrap_or_default(),
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
    figures: usize,
}

#[derive(Serialize)]
struct FigureOut {
    id: i64,
    page: Option<i64>,
    image: String,
}

fn figures_where(conn: &Connection, where_sql: &str, id: i64) -> R<Vec<FigureOut>> {
    let mut stmt = conn.prepare(&format!("SELECT id, page, image FROM figures WHERE {where_sql}")).map_err(err)?;
    let rows = stmt.query_map([id], |r| Ok(FigureOut { id: r.get(0)?, page: r.get(1)?, image: r.get(2)? })).map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
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

/// Eine Aufgabe, wie sie in die Lektion geht: nie mit Lösung.
#[derive(Serialize, Default)]
struct Exercise {
    id: Option<i64>,
    kind: String,
    prompt: String,
    options: Vec<String>,
    pairs: Vec<[String; 2]>,
    /// Feldnamen (sort)
    groups: Vec<String>,
    /// Stellen [x, y, w, h] im Bild, ohne Beschriftung (label)
    spots: Vec<[f64; 4]>,
    /// data:-URL der Grafik (label)
    image: Option<String>,
}

/// Zeile aus der Datenbank → Aufgabe. Strukturaufgaben werden hier umgebaut, damit die Lösung im Kern bleibt.
fn to_exercise(p: Picked, rng: &mut learn::Shuffler) -> Exercise {
    let mut options: Vec<String> = serde_json::from_str(&p.options).unwrap_or_default();
    let groups: Vec<Group> = serde_json::from_str(&p.groups).unwrap_or_default();
    let spots: Vec<Spot> = serde_json::from_str(&p.spots).unwrap_or_default();
    let mut prompt = p.prompt;
    match p.kind.as_str() {
        "sort" => options = groups.iter().flat_map(|g| g.items.clone()).chain(options).collect(),
        "cloze" => {
            let (text, gaps) = learn::cloze_parts(&prompt);
            prompt = text;
            options = gaps.into_iter().chain(options).collect();
        }
        "label" => options = spots.iter().map(|s| s.label.clone()).chain(options).collect(),
        _ => {}
    }
    if p.kind != "true_false" {
        let solved = options.clone();
        rng.shuffle(&mut options);
        // Reihenfolge nicht zufällig schon richtig hinlegen
        for _ in 0..8 {
            if p.kind != "order" || options != solved {
                break;
            }
            rng.shuffle(&mut options);
        }
    }
    Exercise {
        id: Some(p.id),
        prompt,
        options,
        groups: groups.into_iter().map(|g| g.name).collect(),
        spots: spots.iter().map(|s| [s.x, s.y, s.w, s.h]).collect(),
        image: if p.kind == "label" { p.image } else { None },
        kind: p.kind,
        ..Default::default()
    }
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

    let mut picked: Vec<Picked>;
    if let Some(id) = unit_id {
        let (course_id, _) = unit_role(&conn, who, id)?;
        // Hat die Einheit nur Grafik- oder eigene Aufgaben, schreibt die KI trotzdem ihre Fragen.
        let _ = conn.execute("UPDATE units SET q_state = 1 WHERE id = ?1 AND q_state = 0 AND status = 'ready'", [id]);
        let (idx, q_state): (i64, i64) = conn.query_row("SELECT idx, q_state FROM units WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
        // Nächste Einheit schon vorbereiten, während diese geübt wird.
        let _ = conn.execute("UPDATE units SET q_state = 1 WHERE course_id = ?1 AND idx = ?2 AND q_state = 0", params![course_id, idx + 1]);
        // Fällige zuerst, dann neue, dann die mit der niedrigsten Box. Ausgeblendete nie.
        picked = query_questions(
            &conn,
            "SELECT q.id, q.kind, q.prompt, q.options, q.groups, q.spots, f.image FROM questions q
             LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?4 LEFT JOIN figures f ON f.id = q.figure_id
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
            "SELECT q.id, q.kind, q.prompt, q.options, q.groups, q.spots, f.image FROM progress p
             JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id JOIN courses c ON c.id = u.course_id
             LEFT JOIN figures f ON f.id = q.figure_id
             WHERE p.user_id = ?3 AND p.hidden = 0 AND p.box > 0 AND p.due_at <= ?1 AND (c.owner_id = ?3 OR c.shared = 1)
             ORDER BY p.due_at LIMIT ?2",
            params![now, 10, who.id],
        )?;
    }

    rng.shuffle(&mut picked);
    let ids: Vec<String> = picked.iter().map(|p| p.id.to_string()).collect();
    let mut exercises: Vec<Exercise> = picked.into_iter().map(|p| to_exercise(p, &mut rng)).collect();

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
            exercises.insert(at, Exercise { kind: "match".into(), prompt: "Ordne die Begriffe zu".into(), pairs: terms, ..Default::default() });
        }
    }
    Ok(LessonOut { ready: true, failed: false, exercises })
}

struct Picked {
    id: i64,
    kind: String,
    prompt: String,
    options: String,
    groups: String,
    spots: String,
    image: Option<String>,
}

fn query_questions(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> R<Vec<Picked>> {
    let mut stmt = conn.prepare(sql).map_err(err)?;
    let rows = stmt
        .query_map(p, |r| Ok(Picked { id: r.get(0)?, kind: r.get(1)?, prompt: r.get(2)?, options: r.get(3)?, groups: r.get(4)?, spots: r.get(5)?, image: r.get(6)? }))
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

#[derive(Serialize)]
struct AnswerOut {
    correct: bool,
    expected: String,
    why: String,
    /// Lösung je Teil, damit die Oberfläche jede Aussage, Lücke oder Stelle einzeln markiert (sonst null)
    solution: Value,
}

/// Prüft ohne KI. Strukturaufgaben schicken `given` als JSON; kaputtes JSON zählt wie eine leere Antwort.
fn grade(kind: &str, prompt: &str, options: &str, expected: &str, groups: &str, spots: &str, given: &str) -> (bool, Value) {
    let options: Vec<String> = serde_json::from_str(options).unwrap_or_default();
    match kind {
        "sort" => {
            let groups: Vec<Group> = serde_json::from_str(groups).unwrap_or_default();
            let placed: HashMap<String, Vec<String>> = serde_json::from_str(given).unwrap_or_default();
            let n = groups.iter().map(|g| g.items.len()).sum::<usize>() + options.len();
            (learn::passed(learn::sort_mistakes(&groups, &options, &placed), n), json!(groups))
        }
        "order" | "cloze" | "label" => {
            let want = match kind {
                "order" => options,
                "cloze" => learn::cloze_parts(prompt).1,
                _ => serde_json::from_str::<Vec<Spot>>(spots).unwrap_or_default().into_iter().map(|s| s.label).collect(),
            };
            let got: Vec<String> = serde_json::from_str(given).unwrap_or_default();
            (learn::passed(learn::seq_mistakes(&want, &got), want.len()), json!(want))
        }
        _ => (learn::check(kind, expected, given), Value::Null),
    }
}

/// Prüft ohne KI. `retry` = Wiederholung innerhalb derselben Lektion → Box bleibt unverändert.
fn answer(c: &Core, who: &Who, question_id: i64, given: &str, retry: bool) -> R<AnswerOut> {
    let conn = c.db.lock().unwrap();
    question_role(&conn, who, question_id)?;
    let (kind, prompt, options, expected, why, groups, spots, boxed): (String, String, String, String, String, String, String, i64) = conn
        .query_row(
            "SELECT q.kind, q.prompt, q.options, q.answer, q.why, q.groups, q.spots, COALESCE(p.box, 0) FROM questions q
             LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?2 WHERE q.id = ?1",
            params![question_id, who.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )
        .map_err(err)?;
    let (correct, solution) = grade(&kind, &prompt, &options, &expected, &groups, &spots, given);
    if !retry {
        let b = learn::next_box(boxed, correct);
        conn.execute(
            "INSERT INTO progress(user_id, question_id, box, due_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(user_id, question_id) DO UPDATE SET box = excluded.box, due_at = excluded.due_at",
            params![who.id, question_id, b, learn::due_at(b, db::now())],
        )
        .map_err(err)?;
    }
    Ok(AnswerOut { correct, expected, why, solution })
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
    groups: Vec<Group>,
    spots: Vec<Spot>,
    figure_id: Option<i64>,
    /// Von mir ausgeblendet
    hidden: bool,
    /// Mein Kurs → darf für alle löschen
    mine: bool,
    shared: bool,
}

fn questions_where(conn: &Connection, who: &Who, where_sql: &str, id: i64) -> R<Vec<QuestionOut>> {
    let sql = format!(
        "SELECT q.id, q.unit_id, q.kind, q.prompt, q.options, q.answer, q.why, COALESCE(p.hidden, 0), c.owner_id = ?2, c.shared,
           q.groups, q.spots, q.figure_id
         FROM questions q JOIN units u ON u.id = q.unit_id JOIN courses c ON c.id = u.course_id
         LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?2
         WHERE {where_sql}"
    );
    let mut stmt = conn.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(params![id, who.id], |r| {
            let options: String = r.get(4)?;
            let (groups, spots): (String, String) = (r.get(10)?, r.get(11)?);
            Ok(QuestionOut {
                id: r.get(0)?,
                unit_id: r.get(1)?,
                kind: r.get(2)?,
                prompt: r.get(3)?,
                options: serde_json::from_str(&options).unwrap_or_default(),
                answer: r.get(5)?,
                why: r.get(6)?,
                groups: serde_json::from_str(&groups).unwrap_or_default(),
                spots: serde_json::from_str(&spots).unwrap_or_default(),
                figure_id: r.get(12)?,
                hidden: r.get(7)?,
                mine: r.get(8)?,
                shared: r.get(9)?,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

/// Eingabe beim Bearbeiten und Anlegen. `image` = neues Bild (data:-URL), sonst `figure_id` einer Grafik derselben Einheit.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuestionIn {
    prompt: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    why: String,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    spots: Vec<Spot>,
    figure_id: Option<i64>,
    image: Option<String>,
}

fn form_hint(kind: &str) -> &'static str {
    match kind {
        "mc" => "Multiple Choice braucht 4 verschiedene Antworten, eine davon richtig.",
        "fill_blank" => "Ein Lückentext braucht genau eine Lücke ___ und die Antwort unter den Optionen.",
        "cloze" => "Markiere 1 bis 5 Lücken mit [eckigen Klammern].",
        "sort" => "Zuordnen braucht 2 bis 4 Felder mit zusammen 3 bis 12 verschiedenen Aussagen.",
        "order" => "Eine Reihenfolge braucht 3 bis 7 verschiedene Schritte.",
        "label" => "Setze 1 bis 12 Stellen ins Bild und beschrifte jede.",
        _ => "",
    }
}

/// Dieselbe Formprüfung wie bei KI-Fragen, nur ohne die Stil-Filter. Speichert ein neues Bild erst, wenn die Aufgabe passt.
fn checked(conn: &Connection, unit_id: i64, kind: String, i: QuestionIn) -> R<(import::Question, Option<i64>)> {
    let long = |s: &str, n: usize| s.chars().count() > n;
    let too_long = long(&i.prompt, 600)
        || long(&i.why, 1000)
        || i.options.len() > 6
        || i.options.iter().any(|o| long(o, 200))
        || i.groups.len() > 4
        || i.groups.iter().any(|g| long(&g.name, 60) || g.items.len() > 8 || g.items.iter().any(|x| long(x, 200)))
        || i.spots.len() > 12;
    if too_long {
        return Err("Die Aufgabe ist zu lang.".into());
    }
    let (label, hint) = (kind == "label", form_hint(&kind));
    let q = import::fix(import::Question { kind, prompt: i.prompt, options: i.options, answer: i.answer, why: i.why, groups: i.groups, spots: i.spots }, false)
        .ok_or_else(|| format!("So geht die Aufgabe nicht: Sie braucht mindestens 8 Zeichen. {hint}"))?;
    if !label {
        return Ok((q, None));
    }
    let figure = match (i.image, i.figure_id) {
        (Some(img), _) => {
            if !import::valid_image(&img) {
                return Err("Das Bild geht so nicht: JPEG, PNG oder WebP bis 2 MB.".into());
            }
            conn.execute("INSERT INTO figures(unit_id, page, image) VALUES(?1, NULL, ?2)", params![unit_id, img]).map_err(err)?;
            conn.last_insert_rowid()
        }
        (None, Some(id)) => {
            let mine: i64 = conn.query_row("SELECT COUNT(*) FROM figures WHERE id = ?1 AND unit_id = ?2", params![id, unit_id], |r| r.get(0)).map_err(err)?;
            if mine == 0 {
                return Err(DENIED.into());
            }
            id
        }
        (None, None) => return Err("Wähle zuerst ein Bild.".into()),
    };
    Ok((q, Some(figure)))
}

/// Ändert die Frage für alle im Kurs. Ohne neues Bild bleibt die bisherige Grafik.
fn update_question(c: &Core, who: &Who, question_id: i64, mut i: QuestionIn) -> R<QuestionOut> {
    let conn = c.db.lock().unwrap();
    question_role(&conn, who, question_id)?;
    let (unit_id, kind, figure): (i64, String, Option<i64>) =
        conn.query_row("SELECT unit_id, kind, figure_id FROM questions WHERE id = ?1", [question_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(err)?;
    i.figure_id = i.figure_id.or(figure);
    let (q, figure) = checked(&conn, unit_id, kind, i)?;
    conn.execute(
        "UPDATE questions SET prompt = ?1, options = ?2, answer = ?3, why = ?4, groups = ?5, spots = ?6, figure_id = ?7 WHERE id = ?8",
        params![
            q.prompt,
            serde_json::to_string(&q.options).map_err(err)?,
            q.answer,
            q.why,
            serde_json::to_string(&q.groups).map_err(err)?,
            serde_json::to_string(&q.spots).map_err(err)?,
            figure,
            question_id
        ],
    )
    .map_err(err)?;
    questions_where(&conn, who, "q.id = ?1", question_id)?.pop().ok_or_else(|| DENIED.into())
}

/// Neue Aufgabe in einer Einheit. Mitglieder dürfen das wie Bearbeiten; sie gilt für alle im Kurs.
fn create_question(c: &Core, who: &Who, unit_id: i64, kind: String, i: QuestionIn) -> R<QuestionOut> {
    let conn = c.db.lock().unwrap();
    unit_role(&conn, who, unit_id)?;
    let (q, figure) = checked(&conn, unit_id, kind, i)?;
    let level: i64 = conn.query_row("SELECT MAX(q_level, 1) FROM units WHERE id = ?1", [unit_id], |r| r.get(0)).map_err(err)?;
    let id = import::insert_question(&conn, unit_id, &q, level, figure).map_err(err)?;
    questions_where(&conn, who, "q.id = ?1", id)?.pop().ok_or_else(|| DENIED.into())
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

    const PNG: &str = "data:image/png;base64,iVBORw0KGgo=";

    #[test]
    fn foreign_course_is_denied() {
        let dir = tmp("denied");
        let c = core(&dir);
        let (course, unit, q, _) = seed(&c);
        c.db.lock().unwrap().execute("INSERT INTO figures(id, unit_id, page, image) VALUES(50, ?1, 1, ?2)", params![unit, PNG]).unwrap();
        for (cmd, v) in [
            ("course_path", serde_json::json!({ "courseId": course })),
            ("unit", serde_json::json!({ "unitId": unit })),
            ("start_lesson", serde_json::json!({ "unitId": unit })),
            ("answer", serde_json::json!({ "questionId": q, "given": "Richtig", "retry": false })),
            ("update_question", serde_json::json!({ "questionId": q, "prompt": "Neu formuliert hier", "options": [], "answer": "Richtig", "why": "" })),
            ("hide_question", serde_json::json!({ "questionId": q, "hidden": true })),
            ("unit_questions", serde_json::json!({ "unitId": unit })),
            ("unit", serde_json::json!({ "unitId": 99999 })),
            ("create_question", serde_json::json!({ "unitId": unit, "kind": "order", "prompt": "Bringe in Reihenfolge", "options": ["a", "b", "c"] })),
            ("figure", serde_json::json!({ "figureId": 50 })),
            ("figure", serde_json::json!({ "figureId": 99999 })),
            ("unit_figures", serde_json::json!({ "unitId": unit })),
            ("import_figures", serde_json::json!({ "documentId": 20, "figures": [] })),
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
    fn structured_lesson() {
        use serde_json::json;
        let dir = tmp("structured");
        let c = core(&dir);
        let (course, unit, _, _) = seed(&c);
        run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "examDate": null, "shared": true })).unwrap();

        // Ein Mitglied legt Aufgaben an (gilt für alle im Kurs)
        let mk = |v: Value| run(&c, &U2, "create_question", v).unwrap()["id"].as_i64().unwrap();
        let sort = mk(json!({ "unitId": unit, "kind": "sort", "prompt": "Ordne die Aussagen der SWOT zu.", "options": ["Das Wetter ist schön"],
            "groups": [{ "name": "Stärken", "items": ["Treue Kunden"] }, { "name": "Risiken", "items": ["Neue Konkurrenz", "Hohe Mieten"] }] }));
        let cloze = mk(json!({ "unitId": unit, "kind": "cloze", "prompt": "Die [Nachfrage] sinkt, wenn der [Preis] steigt.", "options": ["Angebot"] }));
        let order = mk(json!({ "unitId": unit, "kind": "order", "prompt": "Bringe die Schritte in Reihenfolge.", "options": ["Ziel", "Intern", "Extern", "Strategie"] }));
        let spots = json!([{ "x": 0.1, "y": 0.1, "w": 0.2, "h": 0.1, "label": "Wurzel" }, { "x": 0.5, "y": 0.1, "w": 0.2, "h": 0.1, "label": "Blatt" }, { "x": 0.1, "y": 0.6, "w": 0.2, "h": 0.1, "label": "Stängel" }]);
        let label = mk(json!({ "unitId": unit, "kind": "label", "prompt": "Beschrifte die Pflanze.", "image": PNG, "spots": spots }));
        let bad = |v: Value| assert!(run(&c, &U2, "create_question", v).is_err());
        bad(json!({ "unitId": unit, "kind": "label", "prompt": "Beschrifte die Pflanze.", "image": "data:text/html;base64,PGI+", "spots": spots }));
        bad(json!({ "unitId": unit, "kind": "label", "prompt": "Beschrifte die Pflanze.", "figureId": 99999, "spots": spots }));
        bad(json!({ "unitId": unit, "kind": "essay", "prompt": "Schreib einen Aufsatz." }));

        // In der Lektion steht keine Lösung
        let l = run(&c, &crate::LOCAL, "start_lesson", json!({ "unitId": unit })).unwrap();
        let ex = |id: i64| l["exercises"].as_array().unwrap().iter().find(|e| e["id"] == id).unwrap().clone();
        let e = ex(cloze);
        assert_eq!(e["prompt"], "Die ___ sinkt, wenn der ___ steigt.");
        assert_eq!(e["options"].as_array().unwrap().len(), 3);
        let e = ex(label);
        assert!(e["image"].as_str().unwrap().starts_with("data:image/png") && e["spots"].as_array().unwrap().len() == 3);
        assert!(!e["spots"].to_string().contains("Wurzel"));
        let e = ex(sort);
        assert_eq!((e["groups"].clone(), e["options"].as_array().unwrap().len()), (json!(["Stärken", "Risiken"]), 4));
        assert_ne!(ex(order)["options"], json!(["Ziel", "Intern", "Extern", "Strategie"]));

        // Bewertung je Teil
        let grade = |id: i64, given: Value| run(&c, &crate::LOCAL, "answer", json!({ "questionId": id, "given": given.to_string(), "retry": true })).unwrap();
        let a = grade(sort, json!({ "Stärken": ["Treue Kunden"], "Risiken": ["Neue Konkurrenz", "hohe mieten"] }));
        assert_eq!((a["correct"].clone(), a["solution"][1]["name"].clone()), (json!(true), json!("Risiken")));
        assert_eq!(grade(sort, json!({ "Stärken": ["Treue Kunden", "Das Wetter ist schön"], "Risiken": ["Neue Konkurrenz", "Hohe Mieten"] }))["correct"], false);
        assert_eq!(grade(cloze, json!(["nachfrage", "Preis"]))["correct"], true);
        assert_eq!(grade(cloze, json!(["Preis", "Nachfrage"]))["correct"], false);
        assert_eq!(grade(order, json!(["Ziel", "Intern", "Extern", "Strategie"]))["correct"], true);
        assert_eq!(grade(order, json!(["Intern", "Ziel", "Extern", "Strategie"]))["correct"], false);
        assert_eq!(grade(label, json!(["Wurzel", "Blatt", "Stängel"]))["solution"], json!(["Wurzel", "Blatt", "Stängel"]));
        assert_eq!(grade(label, json!(["Wurzel", "Blatt", "Stängel"]))["correct"], true);
        assert_eq!(run(&c, &crate::LOCAL, "answer", json!({ "questionId": order, "given": "kaputt", "retry": true })).unwrap()["correct"], false);

        // Bearbeiten ohne neues Bild behält die Grafik
        let saved = run(&c, &U2, "update_question", json!({ "questionId": label, "prompt": "Beschrifte die Teile der Pflanze.", "spots": spots })).unwrap();
        assert!(saved["figure_id"].as_i64().is_some());
        assert_eq!(saved["answer"], "1 Wurzel · 2 Blatt · 3 Stängel");
    }

    #[test]
    fn figures_from_pdf() {
        use serde_json::json;
        let dir = tmp("figures");
        let c = core(&dir);
        let (_, unit, _, _) = seed(&c);
        c.db.lock().unwrap()
            .execute_batch("INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text, status) VALUES(31, 10, 20, 1, 4, 6, 'Blatt', 'Text', 'ready');")
            .unwrap();
        let spot = |x: f64, label: &str| json!({ "x": x, "y": 0.2, "w": 0.15, "h": 0.08, "label": label });
        let figures = json!([
            { "page": 5, "title": "Aufbau eines Blattes", "image": PNG, "spots": [spot(0.1, "Blattader"), spot(0.4, "Blattstiel"), spot(0.7, "Blattrand")] },
            { "page": 2, "image": PNG, "spots": [] },
            { "page": 9, "image": "data:text/html;base64,PGI+", "spots": [] }
        ]);
        let r = run(&c, &crate::LOCAL, "import_figures", json!({ "documentId": 20, "figures": figures })).unwrap();
        assert_eq!(r["figures"], 2);
        let figs = |u: i64| run(&c, &crate::LOCAL, "unit_figures", json!({ "unitId": u })).unwrap().as_array().unwrap().len();
        assert_eq!((figs(unit), figs(31)), (1, 1));
        let qs = run(&c, &crate::LOCAL, "unit_questions", json!({ "unitId": 31 })).unwrap();
        assert_eq!(qs[0]["prompt"], "Beschrifte die Grafik „Aufbau eines Blattes“.");
        assert_eq!(qs[0]["kind"], "label");
        // Einheit mit nur Grafik-Aufgaben: Lektion läuft, KI-Fragen werden trotzdem bestellt
        let l = run(&c, &crate::LOCAL, "start_lesson", json!({ "unitId": 31 })).unwrap();
        assert_eq!(l["exercises"][0]["kind"], "label");
        let q_state: i64 = c.db.lock().unwrap().query_row("SELECT q_state FROM units WHERE id = 31", [], |r| r.get(0)).unwrap();
        assert_eq!(q_state, 1);
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
