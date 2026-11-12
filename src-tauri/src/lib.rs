mod ai;
mod db;
mod import;
mod learn;

use std::sync::{mpsc, Arc, Mutex};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, Emitter, Manager, State};

use ai::{Ai, Gen, Msg};

struct App {
    db: db::Db,
    ai: Arc<Ai>,
    wake: mpsc::Sender<()>,
}

impl App {
    fn wake(&self) {
        let _ = self.wake.send(());
    }
}

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Erste `n` Zeichen (nicht Bytes), damit Umlaute nicht zerschnitten werden.
fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map(|(i, _)| &s[..i]).unwrap_or(s)
}

// ---------- KI ----------

#[derive(Serialize)]
struct TierInfo {
    id: &'static str,
    label: &'static str,
    size: u64,
}

#[tauri::command]
fn ai_status(app: State<'_, App>) -> ai::Status {
    app.ai.status()
}

#[tauri::command]
fn ai_tiers() -> Vec<TierInfo> {
    ai::TIERS.iter().map(|t| TierInfo { id: t.id, label: t.label, size: t.size }).collect()
}

#[tauri::command]
fn ai_retry(app: State<'_, App>) {
    app.ai.ensure_model();
}

#[tauri::command]
fn ai_set_tier(app: State<'_, App>, tier: String) -> R<()> {
    db::set_setting(&app.db.lock().unwrap(), "tier", ai::tier(&tier).id).map_err(err)?;
    app.ai.set_tier(&tier);
    Ok(())
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
}

#[tauri::command]
fn courses(app: State<'_, App>) -> R<Vec<Course>> {
    let conn = app.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.name, c.exam_date,
               (SELECT COUNT(*) FROM units u WHERE u.course_id = c.id),
               (SELECT COUNT(*) FROM units u WHERE u.course_id = c.id AND u.status = 'ready'),
               (SELECT COALESCE(group_concat(d.name, char(10)), '') FROM documents d WHERE d.course_id = c.id)
             FROM courses c ORDER BY c.created_at",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            let docs: String = r.get(5)?;
            Ok(Course {
                id: r.get(0)?,
                name: r.get(1)?,
                exam_date: r.get(2)?,
                units: r.get(3)?,
                ready: r.get(4)?,
                documents: docs.lines().map(String::from).collect(),
            })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

#[tauri::command]
fn create_course(app: State<'_, App>, name: String, exam_date: Option<String>) -> R<i64> {
    let conn = app.db.lock().unwrap();
    conn.execute(
        "INSERT INTO courses(name, exam_date, created_at) VALUES(?1, ?2, ?3)",
        params![name.trim(), exam_date.filter(|d| !d.is_empty()), db::now()],
    )
    .map_err(err)?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
fn update_course(app: State<'_, App>, id: i64, name: String, exam_date: Option<String>) -> R<()> {
    app.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE courses SET name = ?1, exam_date = ?2 WHERE id = ?3",
            params![name.trim(), exam_date.filter(|d| !d.is_empty()), id],
        )
        .map(|_| ())
        .map_err(err)
}

#[tauri::command]
fn delete_course(app: State<'_, App>, id: i64) -> R<()> {
    app.db.lock().unwrap().execute("DELETE FROM courses WHERE id = ?1", [id]).map(|_| ()).map_err(err)
}

#[derive(Serialize)]
struct Imported {
    units: usize,
}

#[tauri::command]
fn import_document(app: State<'_, App>, course_id: i64, name: String, pages: Vec<String>) -> R<Imported> {
    let (_, units) = import::store_document(&app.db, course_id, &name, &pages).map_err(err)?;
    if units == 0 {
        return Err("In diesem PDF wurde kein Text gefunden. Ist es eingescannt?".into());
    }
    app.wake();
    Ok(Imported { units })
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

#[tauri::command]
fn course_path(app: State<'_, App>, course_id: i64) -> R<Vec<UnitNode>> {
    let conn = app.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.idx, u.title, u.summary, u.status, u.q_state,
               (SELECT COUNT(*) FROM questions q WHERE q.unit_id = u.id),
               (SELECT COALESCE(AVG(q.box), 0.0) FROM questions q WHERE q.unit_id = u.id),
               (SELECT COUNT(*) FROM questions q WHERE q.unit_id = u.id AND q.box > 0 AND q.due_at <= ?2),
               (SELECT COUNT(*) FROM lessons l WHERE l.unit_id = u.id),
               u.document_id, d.name
             FROM units u JOIN documents d ON d.id = u.document_id WHERE u.course_id = ?1 ORDER BY u.idx",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map(params![course_id, db::now()], |r| {
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

fn unit_terms(conn: &rusqlite::Connection, where_sql: &str, p: impl rusqlite::Params) -> R<Vec<TermOut>> {
    let sql = format!(
        "SELECT t.term, t.definition, u.id, u.title FROM terms t JOIN units u ON u.id = t.unit_id {where_sql} ORDER BY t.term COLLATE NOCASE LIMIT 300"
    );
    let mut stmt = conn.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(p, |r| Ok(TermOut { term: r.get(0)?, definition: r.get(1)?, unit_id: r.get(2)?, unit_title: r.get(3)? }))
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

#[tauri::command]
fn unit(app: State<'_, App>, unit_id: i64) -> R<UnitDetail> {
    let conn = app.db.lock().unwrap();
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

#[tauri::command]
fn search_terms(app: State<'_, App>, query: String) -> R<Vec<TermOut>> {
    let like = format!("%{}%", query.trim());
    unit_terms(&app.db.lock().unwrap(), "WHERE t.term LIKE ?1 OR t.definition LIKE ?1", [like])
}

fn unit_text(app: &App, unit_id: i64) -> R<(String, String, Option<String>)> {
    app.db
        .lock()
        .unwrap()
        .query_row("SELECT title, source_text, explanation FROM units WHERE id = ?1", [unit_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(err)
}

/// Ausführliche Erklärung: einmal gestreamt erzeugen, danach aus der Datenbank.
#[tauri::command]
async fn explain(app: State<'_, App>, unit_id: i64, on_token: Channel<String>) -> R<String> {
    let (title, text, cached) = unit_text(&app, unit_id)?;
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
    let ai = app.ai.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        ai.generate(
            Gen { messages: vec![Msg { role: "user", content: user }], grammar: None, max_tokens: 1100, temperature: 0.5 },
            |p| {
                let _ = on_token.send(p.to_string());
            },
        )
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    app.db.lock().unwrap().execute("UPDATE units SET explanation = ?1 WHERE id = ?2", params![out, unit_id]).map_err(err)?;
    Ok(out)
}

#[derive(Deserialize)]
struct ChatIn {
    role: String,
    content: String,
}

/// „Frag die KI“: nur der Abschnitt der Einheit als Kontext, nur die letzten 6 Nachrichten.
#[tauri::command]
async fn ask(app: State<'_, App>, unit_id: Option<i64>, messages: Vec<ChatIn>, on_token: Channel<String>) -> R<String> {
    let mut system = String::from(
        "Du bist ein geduldiger Lern-Tutor. Antworte auf Deutsch, kurz und verständlich (höchstens 5 Sätze). \
         Wenn etwas nicht im Lernmaterial steht, sag das ehrlich.",
    );
    if let Some(id) = unit_id {
        let (title, text, _) = unit_text(&app, id)?;
        system.push_str(&format!("\n\nThema: {title}\nLernmaterial:\n<<<\n{}\n>>>", head(&text, 4000)));
    }
    let mut msgs = vec![Msg { role: "system", content: system }];
    let skip = messages.len().saturating_sub(6);
    for m in messages.into_iter().skip(skip) {
        let role = if m.role == "assistant" { "assistant" } else { "user" };
        msgs.push(Msg { role, content: head(&m.content, 1500).to_string() });
    }
    let ai = app.ai.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ai.generate(Gen { messages: msgs, grammar: None, max_tokens: 500, temperature: 0.5 }, |p| {
            let _ = on_token.send(p.to_string());
        })
    })
    .await
    .map_err(err)?
    .map_err(err)
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

#[tauri::command]
fn start_lesson(app: State<'_, App>, unit_id: Option<i64>) -> R<LessonOut> {
    let conn = app.db.lock().unwrap();
    let now = db::now();
    let mut rng = learn::Shuffler::new();

    let mut picked: Vec<(i64, String, String, String)>;
    if let Some(id) = unit_id {
        let (course_id, idx, q_state): (i64, i64, i64) =
            conn.query_row("SELECT course_id, idx, q_state FROM units WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(err)?;
        // Nächste Einheit schon vorbereiten, während diese geübt wird.
        let _ = conn.execute("UPDATE units SET q_state = 1 WHERE course_id = ?1 AND idx = ?2 AND q_state = 0", params![course_id, idx + 1]);
        // Fällige zuerst, dann neue, dann die mit der niedrigsten Box.
        picked = query_questions(
            &conn,
            "SELECT id, kind, prompt, options FROM questions WHERE unit_id = ?1
             ORDER BY CASE WHEN box > 0 AND due_at <= ?2 THEN 0 WHEN box = 0 THEN 1 ELSE 2 END, box, due_at, id LIMIT ?3",
            params![id, now, LESSON_SIZE as i64],
        )?;
        if picked.is_empty() {
            if q_state == 0 {
                let _ = conn.execute("UPDATE units SET q_state = 1 WHERE id = ?1", [id]);
            }
            drop(conn);
            app.wake();
            return Ok(LessonOut { ready: false, failed: q_state == 3, exercises: vec![] });
        }
        // Alles sitzt (keine neuen, Ø Box ≥ 3) → nächste, schwierigere Fragenrunde vorbereiten.
        let (new, avg): (i64, f64) = conn
            .query_row("SELECT SUM(box = 0), AVG(box) FROM questions WHERE unit_id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?;
        if new == 0 && avg >= 3.0 && q_state == 2 {
            let _ = conn.execute("UPDATE units SET q_state = 1 WHERE id = ?1", [id]);
            app.wake();
        }
    } else {
        picked = query_questions(
            &conn,
            "SELECT id, kind, prompt, options FROM questions WHERE box > 0 AND due_at <= ?1 ORDER BY due_at LIMIT ?2",
            params![now, 10],
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

fn query_questions(conn: &rusqlite::Connection, sql: &str, p: impl rusqlite::Params) -> R<Vec<(i64, String, String, String)>> {
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
#[tauri::command]
fn answer(app: State<'_, App>, question_id: i64, given: String, retry: bool) -> R<AnswerOut> {
    let conn = app.db.lock().unwrap();
    let (kind, expected, why, boxed): (String, String, String, i64) = conn
        .query_row("SELECT kind, answer, why, box FROM questions WHERE id = ?1", [question_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(err)?;
    let correct = learn::check(&kind, &expected, &given);
    if !retry {
        let b = learn::next_box(boxed, correct);
        conn.execute("UPDATE questions SET box = ?1, due_at = ?2 WHERE id = ?3", params![b, learn::due_at(b, db::now()), question_id])
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

fn stats_of(conn: &rusqlite::Connection) -> R<Stats> {
    let days: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT CAST(julianday(date(finished_at, 'unixepoch', 'localtime')) AS INTEGER) FROM lessons")
            .map_err(err)?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(err)?;
        rows.filter_map(Result::ok).collect()
    };
    let today: i64 = conn.query_row("SELECT CAST(julianday(date('now', 'localtime')) AS INTEGER)", [], |r| r.get(0)).map_err(err)?;
    let xp_on = |offset: i64| -> R<DayXp> {
        conn.query_row(
            "SELECT date('now', 'localtime', ?1), COALESCE((SELECT SUM(xp) FROM lessons WHERE date(finished_at, 'unixepoch', 'localtime') = date('now', 'localtime', ?1)), 0)",
            [format!("-{offset} days")],
            |r| Ok(DayXp { day: r.get(0)?, xp: r.get(1)? }),
        )
        .map_err(err)
    };
    let week = (0..7).rev().map(xp_on).collect::<R<Vec<_>>>()?;
    let active_days = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT date(finished_at, 'unixepoch', 'localtime') FROM lessons WHERE finished_at >= ?1")
            .map_err(err)?;
        let rows = stmt.query_map([db::now() - 42 * 86_400], |r| r.get(0)).map_err(err)?;
        rows.filter_map(Result::ok).collect()
    };
    let (total_xp, lessons): (i64, i64) = conn.query_row("SELECT COALESCE(SUM(xp), 0), COUNT(*) FROM lessons", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
    let due: i64 = conn.query_row("SELECT COUNT(*) FROM questions WHERE box > 0 AND due_at <= ?1", [db::now()], |r| r.get(0)).map_err(err)?;
    Ok(Stats {
        streak: learn::streak(&days, today),
        xp_today: week.last().map(|d| d.xp).unwrap_or(0),
        goal: db::setting(conn, "goal").and_then(|g| g.parse().ok()).unwrap_or(20),
        total_xp,
        lessons,
        due,
        week,
        active_days,
    })
}

#[tauri::command]
fn stats(app: State<'_, App>) -> R<Stats> {
    stats_of(&app.db.lock().unwrap())
}

#[derive(Serialize)]
struct Finished {
    xp: i64,
    stats: Stats,
}

#[tauri::command]
fn finish_lesson(app: State<'_, App>, unit_id: Option<i64>, correct: i64, total: i64) -> R<Finished> {
    let conn = app.db.lock().unwrap();
    let xp = learn::lesson_xp(correct, total);
    conn.execute(
        "INSERT INTO lessons(unit_id, xp, correct, total, finished_at) VALUES(?1, ?2, ?3, ?4, ?5)",
        params![unit_id, xp, correct, total, db::now()],
    )
    .map_err(err)?;
    Ok(Finished { xp, stats: stats_of(&conn)? })
}

#[tauri::command]
fn set_goal(app: State<'_, App>, xp: i64) -> R<()> {
    db::set_setting(&app.db.lock().unwrap(), "goal", &xp.clamp(5, 500).to_string()).map_err(err)
}

#[tauri::command]
fn retry_questions(app: State<'_, App>, unit_id: i64) -> R<()> {
    app.db.lock().unwrap().execute("UPDATE units SET q_state = 1 WHERE id = ?1 AND q_state = 3", [unit_id]).map_err(err)?;
    app.wake();
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("lernen.db"))?;
            let tier = db::setting(&conn, "tier").unwrap_or_else(|| ai::default_tier().into());
            let db: db::Db = Arc::new(Mutex::new(conn));

            let (wake, wake_rx) = mpsc::channel();
            let handle = app.handle().clone();
            let wake_on_ready = wake.clone();
            let ai = Ai::new(
                dir.join("models"),
                &tier,
                Box::new(move |s| {
                    let _ = handle.emit("ai-status", s);
                    if s.state == ai::State::Ready {
                        let _ = wake_on_ready.send(());
                    }
                }),
            );
            ai.ensure_model();

            let handle = app.handle().clone();
            import::spawn_worker(db.clone(), ai.clone(), wake_rx, move |course_id| {
                let _ = handle.emit("course-updated", course_id);
            });
            app.manage(App { db, ai, wake });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ai_status,
            ai_tiers,
            ai_retry,
            ai_set_tier,
            courses,
            create_course,
            update_course,
            delete_course,
            import_document,
            course_path,
            unit,
            search_terms,
            explain,
            ask,
            start_lesson,
            answer,
            stats,
            finish_lesson,
            set_goal,
            retry_questions,
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der App");
}
