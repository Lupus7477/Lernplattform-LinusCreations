//! Alle Befehle der Oberfläche an einer Stelle. Tauri-App und Server rufen nur [`call`] auf.
//! Jede Kurs-, Einheiten- und Fragen-ID wird gegen die Rechte des Nutzers geprüft.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::{self, Gen, Msg};
use crate::learn::{Group, Spot};
use crate::{db, import, learn, report, Core, Who};

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Gleiche Meldung für „gibt es nicht“ und „gehört jemand anderem“: so lässt sich nicht durchprobieren, was existiert.
pub const DENIED: &str = "Kein Zugriff";
const NO_TEXT: &str = "In diesem PDF wurde kein Text gefunden. Ist es eingescannt?";

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
        // Version, Projekt und die Meldungen, die kein Programmfehler sind (Fehler-Toast ohne „Melden“)
        "version" => ok(json!({ "version": env!("CARGO_PKG_VERSION"), "repo": report::REPO, "normal": report::NORMAL })),
        "triage_error" => {
            let (message, cmd) = args!(v, { message: String, cmd: Option<String> });
            ok(report::triage(&c.ai, &message, cmd.as_deref().unwrap_or("")))
        }
        // Issue-Text für den Link ohne Server; der Server ruft das mit seinen eigenen Systemdaten auf.
        "draft_issue" => ok(report::make(&c.ai, serde_json::from_value(v).map_err(err)?)?),
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
            let (name, exam_date, exam_focus) = args!(v, { name: String, exam_date: Option<String>, exam_focus: Option<String> });
            ok(create_course(c, who, &name, exam_date, exam_focus)?)
        }
        "update_course" => {
            let (id, name, exam_date, share, members, members_invite, exam_focus) = args!(v, {
                id: i64, name: String, exam_date: Option<String>, share: Option<String>, members: Option<Vec<i64>>, members_invite: Option<bool>, exam_focus: Option<String>
            });
            ok(update_course(c, who, id, &name, exam_date, share.as_deref(), members, members_invite, exam_focus)?)
        }
        // Mitglied geht: Auswahl verlassen; bei „alle auf dem Server“ zusätzlich ausblenden. Fortschritt bleibt.
        "leave_course" => {
            let course_id = args!(v, { course_id: i64 });
            let conn = c.db.lock().unwrap();
            if role(&conn, who, course_id)? != Role::Member {
                return Err("Deinen eigenen Kurs kannst du nur löschen.".into());
            }
            conn.execute("DELETE FROM course_members WHERE course_id = ?1 AND user_id = ?2", params![course_id, who.id]).map_err(err)?;
            conn.execute("INSERT OR IGNORE INTO course_left(course_id, user_id) SELECT id, ?2 FROM courses WHERE id = ?1 AND shared = 1", params![course_id, who.id])
                .map_err(err)?;
            ok(())
        }
        // Verlassene Kurse, die weiter für alle geteilt sind (zum Wieder-Anzeigen)
        "left_courses" => {
            let conn = c.db.lock().unwrap();
            let mut st = conn
                .prepare("SELECT c.id, c.name FROM course_left l JOIN courses c ON c.id = l.course_id WHERE l.user_id = ?1 AND c.shared = 1 ORDER BY c.name COLLATE NOCASE")
                .map_err(err)?;
            let rows = st.query_map([who.id], |r| Ok(DocOut { id: r.get(0)?, name: r.get(1)? })).map_err(err)?;
            ok(rows.collect::<Result<Vec<_>, _>>().map_err(err)?)
        }
        "rejoin_course" => {
            let course_id = args!(v, { course_id: i64 });
            let conn = c.db.lock().unwrap();
            ok(conn.execute("DELETE FROM course_left WHERE course_id = ?1 AND user_id = ?2", params![course_id, who.id]).map_err(err)?)
        }
        // Personen, mit denen man teilen kann (aktive Konten des Servers), mit Häkchen, wer schon drin ist
        "share_people" => {
            let course_id = args!(v, { course_id: i64 });
            let conn = c.db.lock().unwrap();
            may_invite(&conn, who, course_id)?;
            let mut st = conn
                .prepare(
                    "SELECT u.id, u.name, EXISTS(SELECT 1 FROM course_members m WHERE m.course_id = ?1 AND m.user_id = u.id)
                     FROM users u JOIN courses c ON c.id = ?1
                     WHERE u.pw_hash IS NOT NULL AND u.disabled = 0 AND u.id != c.owner_id AND u.id != ?2 ORDER BY u.name COLLATE NOCASE",
                )
                .map_err(err)?;
            let rows = st.query_map(params![course_id, who.id], |r| Ok(Person { id: r.get(0)?, name: r.get(1)?, member: r.get(2)? })).map_err(err)?;
            ok(rows.collect::<Result<Vec<_>, _>>().map_err(err)?)
        }
        // Mitglied mit Einladerecht: nur hinzufügen, nie entfernen
        "invite_to_course" => {
            let (course_id, user_ids) = args!(v, { course_id: i64, user_ids: Vec<i64> });
            let conn = c.db.lock().unwrap();
            may_invite(&conn, who, course_id)?;
            ok(add_members(&conn, course_id, &user_ids)?)
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
            let r = import::store_document(&c.db, course_id, head(name.trim(), 200), &pages, &figures.unwrap_or_default()).map_err(err)?;
            if r.units == 0 {
                return Err(NO_TEXT.into());
            }
            c.wake();
            ok(r)
        }
        // Geändertes PDF: gleiche Abschnitte bleiben, geänderte prüft die KI, neue kommen dazu, weggefallene gehen.
        "replace_document" => {
            let (document_id, name, pages, figures) = args!(v, { document_id: i64, name: String, pages: Vec<String>, figures: Option<Vec<import::FigureIn>> });
            doc_owner(&c.db.lock().unwrap(), who, document_id)?;
            let r = import::replace_document(&c.db, document_id, head(name.trim(), 200), &pages, &figures.unwrap_or_default()).map_err(err)?;
            if r.same + r.changed + r.units == 0 {
                return Err(NO_TEXT.into());
            }
            c.wake();
            ok(r)
        }
        // Mit dem PDF gehen Einheiten, Fragen, Grafiken und der Fortschritt darin; XP-Historie bleibt (lessons.unit_id → NULL).
        "delete_document" => {
            let document_id = args!(v, { document_id: i64 });
            let conn = c.db.lock().unwrap();
            doc_owner(&conn, who, document_id)?;
            ok(conn.execute("DELETE FROM documents WHERE id = ?1", [document_id]).map_err(err)?)
        }
        // Reihenfolge der PDFs im Lernpfad: Einheiten eines PDFs bleiben zusammen und behalten ihre Folge.
        "reorder_documents" => {
            let (course_id, document_ids) = args!(v, { course_id: i64, document_ids: Vec<i64> });
            let conn = c.db.lock().unwrap();
            owner(role(&conn, who, course_id)?)?;
            let order = serde_json::to_string(&document_ids).map_err(err)?;
            ok(conn
                .execute(
                    // UPDATE … FROM rechnet die neue Folge einmal vorab, nicht je Zeile auf halb geänderten Daten.
                    "UPDATE units SET idx = o.n FROM (SELECT u.id, ROW_NUMBER() OVER (ORDER BY COALESCE(j.key, 1e9), u.idx) - 1 AS n
                       FROM units u LEFT JOIN json_each(?2) j ON j.value = u.document_id WHERE u.course_id = ?1) o WHERE o.id = units.id",
                    params![course_id, order],
                )
                .map_err(err)?)
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
            ok(unit_terms(&c.db.lock().unwrap(), &format!("WHERE {} AND (t.term LIKE ?1 OR t.definition LIKE ?1)", can_see("?2")), params![like, who.id])?)
        }
        "explain" => ok(explain(c, who, args!(v, { unit_id: i64 }), on_token)?),
        "ask" => {
            let (unit_id, messages) = args!(v, { unit_id: Option<i64>, messages: Vec<ChatIn> });
            ok(ask(c, who, unit_id, messages, on_token)?)
        }
        "start_lesson" => {
            let (unit_id, wrong_course, exam_course) = args!(v, { unit_id: Option<i64>, wrong_course: Option<i64>, exam_course: Option<i64> });
            ok(start_lesson(c, who, unit_id, wrong_course, exam_course)?)
        }
        "answer" => {
            let (question_id, given, retry) = args!(v, { question_id: i64, given: String, retry: bool });
            ok(answer(c, who, question_id, &given, retry)?)
        }
        "battle_lobby" => ok(battle_lobby(c, who)?),
        "battle_create" => ok(battle_create(c, who, args!(v, { course_id: i64 }))?),
        "battle_join" => {
            let (battle_id, course_id) = args!(v, { battle_id: i64, course_id: i64 });
            ok(battle_join(c, who, battle_id, course_id)?)
        }
        "battle" => {
            let (battle_id, exercises) = args!(v, { battle_id: i64, exercises: Option<bool> });
            ok(battle_state(c, who, battle_id, exercises.unwrap_or(false))?)
        }
        "battle_answer" => {
            let (battle_id, idx, given) = args!(v, { battle_id: i64, idx: usize, given: String });
            ok(battle_answer(c, who, battle_id, idx, &given)?)
        }
        "battle_leave" => ok(battle_leave(c, who, args!(v, { battle_id: i64 }))?),
        "leaderboard" => ok(leaderboard(c, who, args!(v, { course_id: Option<i64> }))?),
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
        // Für die Klausur merken: nur für mich, wie Ausblenden
        "mark_question" => {
            let (question_id, marked) = args!(v, { question_id: i64, marked: bool });
            let conn = c.db.lock().unwrap();
            question_role(&conn, who, question_id)?;
            conn.execute(
                "INSERT INTO progress(user_id, question_id, marked) VALUES(?1, ?2, ?3)
                 ON CONFLICT(user_id, question_id) DO UPDATE SET marked = excluded.marked",
                params![who.id, question_id, marked],
            )
            .map_err(err)?;
            ok(())
        }
        // Themenfilter überstimmen: ohne Aussortieren neu analysieren. Fragen und Fortschritt bleiben.
        "include_orga" => {
            let unit_id = args!(v, { unit_id: i64 });
            let conn = c.db.lock().unwrap();
            owner(unit_role(&conn, who, unit_id)?.1)?;
            conn.execute("UPDATE units SET orga = NULL, status = 'pending', exam = NULL, explanation = NULL WHERE id = ?1", [unit_id]).map_err(err)?;
            drop(conn);
            c.wake();
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

/// SQL: Nutzer `p` (Parameter wie „?2“) darf Kurs `c` sehen: eigener Kurs, für alle geteilt (und nicht verlassen) oder ausgewählt.
fn can_see(p: &str) -> String {
    format!(
        "(c.owner_id = {p} OR (c.shared = 1 AND NOT EXISTS(SELECT 1 FROM course_left l WHERE l.course_id = c.id AND l.user_id = {p}))
          OR EXISTS(SELECT 1 FROM course_members m WHERE m.course_id = c.id AND m.user_id = {p}))"
    )
}

/// Besitzer, Mitglied (für alle geteilt oder ausgewählt) oder kein Zugriff. Gleiche Regel wie [`can_see`].
pub fn role(conn: &Connection, who: &Who, course_id: i64) -> R<Role> {
    let row: Option<(i64, bool)> = conn
        .query_row(
            "SELECT owner_id, (shared AND NOT EXISTS(SELECT 1 FROM course_left l WHERE l.course_id = c.id AND l.user_id = ?2))
               OR EXISTS(SELECT 1 FROM course_members m WHERE m.course_id = c.id AND m.user_id = ?2) FROM courses c WHERE id = ?1",
            params![course_id, who.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
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

/// Besitzer immer; Mitglieder nur, wenn der Besitzer es erlaubt und der Kurs nicht ohnehin für alle ist.
fn may_invite(conn: &Connection, who: &Who, course_id: i64) -> R<()> {
    if role(conn, who, course_id)? == Role::Owner {
        return Ok(());
    }
    let allowed: bool = conn.query_row("SELECT members_invite AND NOT shared FROM courses WHERE id = ?1", [course_id], |r| r.get(0)).map_err(err)?;
    if allowed {
        Ok(())
    } else {
        Err(DENIED.into())
    }
}

/// Nur aktive Konten mit Passwort, nie der Besitzer; unbekannte IDs fallen still weg.
/// Wer den Kurs verlassen hatte und neu eingeladen wird, ist wieder dabei.
fn add_members(conn: &Connection, course_id: i64, ids: &[i64]) -> R<()> {
    for id in ids.iter().take(1000) {
        conn.execute("DELETE FROM course_left WHERE course_id = ?1 AND user_id = ?2", params![course_id, id]).map_err(err)?;
        conn.execute(
            "INSERT OR IGNORE INTO course_members(course_id, user_id)
             SELECT c.id, u.id FROM users u JOIN courses c ON c.id = ?1
             WHERE u.id = ?2 AND u.pw_hash IS NOT NULL AND u.disabled = 0 AND u.id != c.owner_id",
            params![course_id, id],
        )
        .map_err(err)?;
    }
    Ok(())
}

fn doc_owner(conn: &Connection, who: &Who, document_id: i64) -> R<()> {
    let course: i64 = conn
        .query_row("SELECT course_id FROM documents WHERE id = ?1", [document_id], |r| r.get(0))
        .optional()
        .map_err(err)?
        .ok_or(DENIED)?;
    owner(role(conn, who, course)?)
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
    /// Außer dem Besitzer sieht ihn noch jemand (alle oder ausgewählte Personen)
    shared: bool,
    share_all: bool,
    /// Anzahl ausgewählter Personen
    members: i64,
    members_invite: bool,
    /// Darf Personen hinzufügen (Besitzer, oder Mitglied mit Erlaubnis)
    can_invite: bool,
    owner: String,
    /// Meine zuletzt falsch beantworteten Fragen in diesem Kurs
    wrong: i64,
    /// Prüfungsschwerpunkte (Text des Besitzers)
    exam_focus: String,
    /// Vorschläge der KI aus Orga-Folien
    exam_hints: Vec<String>,
    /// Meine Klausurfragen: aus prüfungsrelevanten Einheiten oder von mir gemerkt
    exam: i64,
}

/// SQL: Frage `q` gehört für Nutzer `p` zum Prüfungstraining: Einheit passt zu den Schwerpunkten oder Frage gemerkt.
fn exam_question(p: &str) -> String {
    format!("(COALESCE(u.exam, '[]') != '[]' OR EXISTS(SELECT 1 FROM progress pm WHERE pm.question_id = q.id AND pm.user_id = {p} AND pm.marked = 1))")
}

#[derive(Serialize)]
struct Person {
    id: i64,
    name: String,
    member: bool,
}

fn courses(c: &Core, who: &Who) -> R<Vec<Course>> {
    let conn = c.db.lock().unwrap();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT c.id, c.name, c.exam_date,
               (SELECT COUNT(*) FROM units u WHERE u.course_id = c.id),
               (SELECT COUNT(*) FROM units u WHERE u.course_id = c.id AND u.status != 'pending'),
               (SELECT json_group_array(json_object('id', d.id, 'name', d.name)) FROM (SELECT id, name FROM documents WHERE course_id = c.id ORDER BY (SELECT MIN(idx) FROM units WHERE document_id = documents.id), id) d),
               c.owner_id = ?1, c.shared, COALESCE(o.name, ''),
               (SELECT COUNT(*) FROM course_members m WHERE m.course_id = c.id), c.members_invite,
               (SELECT COUNT(*) FROM progress p JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id
                 WHERE u.course_id = c.id AND p.user_id = ?1 AND p.wrong = 1 AND p.hidden = 0),
               c.exam_focus,
               (SELECT json_group_array(h) FROM (SELECT exam_hint AS h FROM units WHERE course_id = c.id AND exam_hint != '' GROUP BY exam_hint ORDER BY MIN(idx))),
               (SELECT COUNT(*) FROM questions q JOIN units u ON u.id = q.unit_id LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?1
                 WHERE u.course_id = c.id AND COALESCE(p.hidden, 0) = 0 AND {})
             FROM courses c LEFT JOIN users o ON o.id = c.owner_id
             WHERE {} ORDER BY c.created_at",
            exam_question("?1"),
            can_see("?1")
        ))
        .map_err(err)?;
    let rows = stmt
        .query_map([who.id], |r| {
            let (docs, hints): (String, String) = (r.get(5)?, r.get(13)?);
            let (mine, share_all, members, members_invite): (bool, bool, i64, bool) = (r.get(6)?, r.get(7)?, r.get(9)?, r.get(10)?);
            Ok(Course {
                id: r.get(0)?,
                name: r.get(1)?,
                exam_date: r.get(2)?,
                units: r.get(3)?,
                ready: r.get(4)?,
                documents: serde_json::from_str(&docs).unwrap_or_default(),
                mine,
                shared: share_all || members > 0,
                share_all,
                members,
                members_invite,
                can_invite: mine || (members_invite && !share_all),
                owner: r.get(8)?,
                wrong: r.get(11)?,
                exam_focus: r.get(12)?,
                exam_hints: serde_json::from_str(&hints).unwrap_or_default(),
                exam: r.get(14)?,
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

/// Prüfungsschwerpunkte: höchstens 4000 Zeichen, ohne Leerraum am Rand.
fn exam_focus(focus: Option<String>) -> Option<String> {
    focus.map(|f| head(f.trim(), 4000).to_string())
}

fn create_course(c: &Core, who: &Who, name: &str, exam_date: Option<String>, focus: Option<String>) -> R<i64> {
    let conn = c.db.lock().unwrap();
    conn.execute(
        "INSERT INTO courses(name, exam_date, created_at, owner_id, exam_focus) VALUES(?1, ?2, ?3, ?4, ?5)",
        params![course_name(name)?, exam_date.filter(|d| !d.is_empty()), db::now(), who.id, exam_focus(focus).unwrap_or_default()],
    )
    .map_err(err)?;
    Ok(conn.last_insert_rowid())
}

/// `share`: „none“ = nur ich, „some“ = ausgewählte Personen (`members` ersetzt die Auswahl), „all“ = alle auf dem Server.
/// Bei „all“ bleibt die Auswahl gespeichert, falls man zurückwechselt.
/// `exam_focus` None = unverändert. Geänderte Schwerpunkte: alle Einheiten neu zuordnen (Worker).
#[allow(clippy::too_many_arguments)]
fn update_course(c: &Core, who: &Who, id: i64, name: &str, exam_date: Option<String>, share: Option<&str>, members: Option<Vec<i64>>, members_invite: Option<bool>, focus: Option<String>) -> R<()> {
    let mut conn = c.db.lock().unwrap();
    owner(role(&conn, who, id)?)?;
    let all = match share {
        None => None,
        Some("all") => Some(true),
        Some("some" | "none") => Some(false),
        Some(_) => return Err("Unbekannte Freigabe".into()),
    };
    let tx = conn.transaction().map_err(err)?;
    tx.execute(
        "UPDATE courses SET name = ?1, exam_date = ?2, shared = COALESCE(?3, shared), members_invite = COALESCE(?4, members_invite) WHERE id = ?5",
        params![course_name(name)?, exam_date.filter(|d| !d.is_empty()), all, members_invite, id],
    )
    .map_err(err)?;
    if let Some(f) = exam_focus(focus) {
        let changed = tx.execute("UPDATE courses SET exam_focus = ?1 WHERE id = ?2 AND exam_focus != ?1", params![f, id]).map_err(err)?;
        if changed > 0 {
            tx.execute("UPDATE units SET exam = NULL WHERE course_id = ?1", [id]).map_err(err)?;
        }
    }
    match (share, members) {
        (Some("none"), _) => {
            tx.execute("DELETE FROM course_members WHERE course_id = ?1", [id]).map_err(err)?;
        }
        (Some("some"), Some(m)) => {
            tx.execute("DELETE FROM course_members WHERE course_id = ?1", [id]).map_err(err)?;
            add_members(&tx, id, &m)?;
        }
        _ => {}
    }
    tx.commit().map_err(err)?;
    c.wake();
    Ok(())
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
    /// Passt zu den Prüfungsschwerpunkten des Kurses
    exam: bool,
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
               u.document_id, d.name, COALESCE(u.exam, '[]') != '[]'
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
                exam: r.get(12)?,
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
    /// Seiten, die als Organisatorisches nicht abgefragt werden
    orga: usize,
    /// Passende Prüfungsthemen
    exam: Vec<String>,
}

/// Prüfungsthemen einer Einheit aus `units.exam`.
fn topics(exam: Option<String>) -> Vec<String> {
    exam.and_then(|e| serde_json::from_str(&e).ok()).unwrap_or_default()
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
            "SELECT u.id, u.course_id, u.title, u.summary, u.explanation, u.page_from, u.page_to, d.name, u.orga, u.exam
             FROM units u JOIN documents d ON d.id = u.document_id WHERE u.id = ?1",
            [unit_id],
            |r| {
                let orga: Option<String> = r.get(8)?;
                Ok(UnitDetail {
                    orga: orga.and_then(|o| serde_json::from_str::<Vec<usize>>(&o).ok()).map_or(0, |v| v.len()),
                    exam: topics(r.get(9)?),
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

/// Titel, Lerntext (ohne Organisatorisches), gespeicherte Erklärung und Prüfungsthemen einer Einheit.
fn unit_text(c: &Core, who: &Who, unit_id: i64) -> R<(String, String, Option<String>, Vec<String>)> {
    let conn = c.db.lock().unwrap();
    unit_role(&conn, who, unit_id)?;
    conn.query_row("SELECT title, source_text, explanation, orga, exam FROM units WHERE id = ?1", [unit_id], |r| {
        let (source, orga): (String, Option<String>) = (r.get(1)?, r.get(3)?);
        Ok((r.get(0)?, import::learn_text(&source, orga.as_deref()), r.get(2)?, topics(r.get(4)?)))
    })
    .map_err(err)
}

/// Ausführliche Erklärung: einmal gestreamt erzeugen, danach aus der Datenbank (gilt für alle im Kurs).
fn explain(c: &Core, who: &Who, unit_id: i64, on_token: &mut dyn FnMut(&str)) -> R<String> {
    let (title, text, cached, exam) = unit_text(c, who, unit_id)?;
    if let Some(x) = cached {
        return Ok(x);
    }
    let focus = if exam.is_empty() { String::new() } else { format!(" Geh besonders ein auf: {}.", exam.join("; ")) };
    let user = format!(
        "Erkläre das Thema „{title}“ anhand dieses Lernmaterials so, dass es jemand ohne Vorwissen versteht.{focus}\n\n\
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
        let (title, text, _, _) = unit_text(c, who, id)?;
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
    /// Gelernte Frage aus einer anderen Einheit, eingestreut zum Auffrischen
    refresh: bool,
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

/// Lektion einer Einheit, „Falsche wiederholen“ (`wrong_course`), Prüfungstraining (`exam_course`) eines Kurses
/// oder die tägliche Wiederholung (alles None).
fn start_lesson(c: &Core, who: &Who, unit_id: Option<i64>, wrong_course: Option<i64>, exam_course: Option<i64>) -> R<LessonOut> {
    let conn = c.db.lock().unwrap();
    let now = db::now();
    let mut rng = learn::Shuffler::new();
    const SELECT: &str = "SELECT q.id, q.kind, q.prompt, q.options, q.groups, q.spots, f.image FROM";

    let mut picked: Vec<Picked>;
    let mut refresh: Vec<i64> = vec![];
    if let Some(id) = unit_id {
        let (course_id, _) = unit_role(&conn, who, id)?;
        // Hat die Einheit nur Grafik- oder eigene Aufgaben, schreibt die KI trotzdem ihre Fragen.
        let _ = conn.execute("UPDATE units SET q_state = 1 WHERE id = ?1 AND q_state = 0 AND status = 'ready'", [id]);
        let (idx, q_state): (i64, i64) = conn.query_row("SELECT idx, q_state FROM units WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
        // Nächste Einheit schon vorbereiten, während diese geübt wird.
        // Orga-Einheiten überspringen: die nächste mit Lerninhalt.
        let _ = conn.execute(
            "UPDATE units SET q_state = 1 WHERE course_id = ?1 AND q_state = 0 AND idx = (SELECT MIN(idx) FROM units WHERE course_id = ?1 AND idx > ?2 AND status != 'orga')",
            params![course_id, idx],
        );
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
        // Auffrischen: bis zu 2 schon gelernte Fragen aus anderen Einheiten des Kurses, fällige zuerst, sonst zufällig.
        let mut old = query_questions(
            &conn,
            &format!(
                "{SELECT} progress p JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id LEFT JOIN figures f ON f.id = q.figure_id
                 WHERE p.user_id = ?3 AND p.hidden = 0 AND p.box >= 2 AND u.course_id = ?1 AND q.unit_id != ?2
                 ORDER BY p.due_at <= ?4 DESC, RANDOM() LIMIT 2"
            ),
            params![course_id, id, who.id, now],
        )?;
        refresh = old.iter().map(|p| p.id).collect();
        picked.append(&mut old);
    } else if let Some(course) = wrong_course {
        role(&conn, who, course)?;
        picked = query_questions(
            &conn,
            &format!(
                "{SELECT} progress p JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id LEFT JOIN figures f ON f.id = q.figure_id
                 WHERE p.user_id = ?2 AND p.hidden = 0 AND p.wrong = 1 AND u.course_id = ?1 ORDER BY p.due_at LIMIT ?3"
            ),
            params![course, who.id, 12],
        )?;
    } else if let Some(course) = exam_course {
        role(&conn, who, course)?;
        // Prüfungsthemen und Gemerktes: fällige zuerst, dann neue, dann die mit der niedrigsten Box
        picked = query_questions(
            &conn,
            &format!(
                "{SELECT} questions q JOIN units u ON u.id = q.unit_id LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?2
                 LEFT JOIN figures f ON f.id = q.figure_id
                 WHERE u.course_id = ?1 AND COALESCE(p.hidden, 0) = 0 AND {}
                 ORDER BY CASE WHEN p.box > 0 AND p.due_at <= ?3 THEN 0 WHEN COALESCE(p.box, 0) = 0 THEN 1 ELSE 2 END, COALESCE(p.box, 0), RANDOM()
                 LIMIT ?4",
                exam_question("?2")
            ),
            params![course, who.id, now, 12],
        )?;
    } else {
        picked = query_questions(
            &conn,
            &format!(
                "{SELECT} progress p
                 JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id JOIN courses c ON c.id = u.course_id
                 LEFT JOIN figures f ON f.id = q.figure_id
                 WHERE p.user_id = ?3 AND p.hidden = 0 AND p.box > 0 AND p.due_at <= ?1 AND {}
                 ORDER BY p.marked = 1 OR COALESCE(u.exam, '[]') != '[]' DESC, p.due_at LIMIT ?2",
                can_see("?3")
            ),
            params![now, 10, who.id],
        )?;
    }

    rng.shuffle(&mut picked);
    let ids: Vec<String> = picked.iter().map(|p| p.id.to_string()).collect();
    let mut exercises: Vec<Exercise> = picked.into_iter().map(|p| to_exercise(p, &mut rng)).collect();
    for e in &mut exercises {
        e.refresh = e.id.is_some_and(|id| refresh.contains(&id));
    }

    // Zuordnen-Runde aus den Begriffen – ganz ohne KI. Nicht bei „Falsche wiederholen“: dort zählen nur die Fragen.
    let term_sql = match unit_id {
        Some(_) => "SELECT term, definition FROM terms WHERE unit_id = ?1".to_string(),
        None if wrong_course.is_none() && exam_course.is_none() && !ids.is_empty() => format!(
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
    let (out, boxed) = graded(&conn, who, question_id, given)?;
    if !retry {
        let b = learn::next_box(boxed, out.correct);
        conn.execute(
            "INSERT INTO progress(user_id, question_id, box, due_at, wrong) VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user_id, question_id) DO UPDATE SET box = excluded.box, due_at = excluded.due_at, wrong = excluded.wrong",
            params![who.id, question_id, b, learn::due_at(b, db::now()), !out.correct],
        )
        .map_err(err)?;
    }
    Ok(out)
}

/// Antwort prüfen (mit Rechteprüfung), ohne etwas zu speichern. Dazu die aktuelle Box des Nutzers.
fn graded(conn: &Connection, who: &Who, question_id: i64, given: &str) -> R<(AnswerOut, i64)> {
    question_role(conn, who, question_id)?;
    let (kind, prompt, options, expected, why, groups, spots, boxed): (String, String, String, String, String, String, String, i64) = conn
        .query_row(
            "SELECT q.kind, q.prompt, q.options, q.answer, q.why, q.groups, q.spots, COALESCE(p.box, 0) FROM questions q
             LEFT JOIN progress p ON p.question_id = q.id AND p.user_id = ?2 WHERE q.id = ?1",
            params![question_id, who.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )
        .map_err(err)?;
    let (correct, solution) = grade(&kind, &prompt, &options, &expected, &groups, &spots, given);
    Ok((AnswerOut { correct, expected, why, solution }, boxed))
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
            &format!(
                "SELECT COUNT(*) FROM progress p JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id JOIN courses c ON c.id = u.course_id
                 WHERE p.user_id = ?2 AND p.hidden = 0 AND p.box > 0 AND p.due_at <= ?1 AND {}",
                can_see("?2")
            ),
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
    /// Von mir für die Klausur gemerkt
    marked: bool,
    /// Mein Kurs → darf für alle löschen
    mine: bool,
    shared: bool,
}

fn questions_where(conn: &Connection, who: &Who, where_sql: &str, id: i64) -> R<Vec<QuestionOut>> {
    let sql = format!(
        "SELECT q.id, q.unit_id, q.kind, q.prompt, q.options, q.answer, q.why, COALESCE(p.hidden, 0), c.owner_id = ?2,
           c.shared OR EXISTS(SELECT 1 FROM course_members m WHERE m.course_id = c.id),
           q.groups, q.spots, q.figure_id, COALESCE(p.marked, 0)
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
                marked: r.get(13)?,
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

// ---------- Battle: zwei Konten treten live gegeneinander an ----------

const BATTLE_ROUNDS: usize = 8;
const BATTLE_MIN: usize = 3;
const COUNTDOWN_MS: i64 = 3000;
/// Danach ist ein Battle vorbei, auch wenn jemand nicht fertig wurde (Tab zu, Verbindung weg).
const BATTLE_MS: i64 = 15 * 60 * 1000;
/// Offene Herausforderungen verfallen nach 30 Minuten.
const OPEN_SECS: i64 = 30 * 60;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Was die Bearbeitungszeit bestimmt: Art und Anzahl der Teile, so wie die Aufgabe auf dem Bildschirm steht.
/// Gleicher Schlüssel = gleich viel zu tun (z. B. Matrix mit 2 Feldern und 6 Aussagen).
fn battle_key(e: &Exercise) -> String {
    format!("{}:{}:{}:{}:{}", e.kind, e.options.len(), e.groups.len(), e.spots.len(), e.prompt.matches("___").count())
}

/// Paart zwei gemischte Listen (ID, Schlüssel): an jeder Stelle steht bei beiden derselbe Schlüssel.
fn pair_up(a: &[(i64, String)], b: &[(i64, String)], n: usize) -> Vec<(i64, i64)> {
    let mut free: HashMap<&str, Vec<i64>> = HashMap::new();
    for (id, k) in b {
        free.entry(k.as_str()).or_default().push(*id);
    }
    a.iter().filter_map(|(id, k)| Some((*id, free.get_mut(k.as_str())?.pop()?))).take(n).collect()
}

/// Fragen eines Kurses für das Battle, gemischt, ohne die von `u1` oder `u2` ausgeblendeten.
fn battle_pool(conn: &Connection, course_id: i64, u1: i64, u2: i64) -> R<Vec<(i64, String)>> {
    let mut rng = learn::Shuffler::new();
    let picked = query_questions(
        conn,
        "SELECT q.id, q.kind, q.prompt, q.options, q.groups, q.spots, NULL FROM questions q JOIN units u ON u.id = q.unit_id
         WHERE u.course_id = ?1 AND NOT EXISTS(SELECT 1 FROM progress p WHERE p.question_id = q.id AND p.hidden = 1 AND p.user_id IN (?2, ?3))
         ORDER BY RANDOM()",
        params![course_id, u1, u2],
    )?;
    Ok(picked.into_iter().map(|p| (p.id, battle_key(&to_exercise(p, &mut rng)))).collect())
}

struct Battle {
    a: i64,
    b: Option<i64>,
    a_course: i64,
    b_course: Option<i64>,
    a_qs: Vec<i64>,
    b_qs: Vec<i64>,
    created_at: i64,
    started_ms: Option<i64>,
    gave_up: Option<i64>,
}

fn battle_row(conn: &Connection, id: i64) -> R<Battle> {
    conn.query_row("SELECT a_id, b_id, a_course, b_course, a_qs, b_qs, created_at, started_ms, gave_up FROM battles WHERE id = ?1", [id], |r| {
        let (a_qs, b_qs): (String, String) = (r.get(4)?, r.get(5)?);
        Ok(Battle {
            a: r.get(0)?,
            b: r.get(1)?,
            a_course: r.get(2)?,
            b_course: r.get(3)?,
            a_qs: serde_json::from_str(&a_qs).unwrap_or_default(),
            b_qs: serde_json::from_str(&b_qs).unwrap_or_default(),
            created_at: r.get(6)?,
            started_ms: r.get(7)?,
            gave_up: r.get(8)?,
        })
    })
    .optional()
    .map_err(err)?
    .ok_or_else(|| DENIED.into())
}

#[derive(Serialize, Clone, Copy, Default, PartialEq, Debug)]
struct Score {
    answered: usize,
    correct: usize,
    /// Zeit der letzten Antwort seit dem Start
    ms: i64,
}

fn score(conn: &Connection, battle_id: i64, user: i64) -> R<Score> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(correct), 0), COALESCE(MAX(ms), 0) FROM battle_answers WHERE battle_id = ?1 AND user_id = ?2",
        params![battle_id, user],
        |r| Ok(Score { answered: r.get::<_, i64>(0)? as usize, correct: r.get::<_, i64>(1)? as usize, ms: r.get(2)? }),
    )
    .map_err(err)
}

impl Battle {
    fn rounds(&self) -> usize {
        self.a_qs.len()
    }
    fn over(&self, a: Score, b: Score, now: i64) -> bool {
        self.started_ms.is_some_and(|t| self.gave_up.is_some() || (a.answered == self.rounds() && b.answered == self.rounds()) || now >= t + BATTLE_MS)
    }
    /// Aus Sicht von A: Greater = A gewinnt. Aufgeben verliert immer; sonst mehr Richtige,
    /// bei Gleichstand die kürzere Gesamtzeit. Wer nicht fertig wurde, ist bei Gleichstand der Langsamere.
    fn outcome(&self, a: Score, b: Score) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match self.gave_up {
            Some(u) if u == self.a => return Less,
            Some(_) => return Greater,
            None => {}
        }
        let time = |s: Score| if s.answered == self.rounds() { s.ms } else { i64::MAX };
        a.correct.cmp(&b.correct).then(time(b).cmp(&time(a)))
    }
}

#[derive(Serialize)]
struct Side {
    name: String,
    /// Nur, wenn ich den Kurs auch sehen darf
    course: Option<String>,
    #[serde(flatten)]
    score: Score,
    gave_up: bool,
}

#[derive(Serialize)]
struct BattleOut {
    /// waiting | countdown | running | done
    state: &'static str,
    starts_in: i64,
    rounds: usize,
    me: Side,
    them: Option<Side>,
    /// 1 gewonnen, 0 unentschieden, -1 verloren (nur wenn vorbei)
    result: Option<i8>,
    /// Meine Aufgaben, erst ab dem Start (sonst ließe sich im Countdown vorlesen)
    exercises: Vec<Exercise>,
}

fn side(conn: &Connection, who: &Who, bt: &Battle, user: i64, course: i64, s: Score) -> R<Side> {
    let name: String = conn.query_row("SELECT name FROM users WHERE id = ?1", [user], |r| r.get(0)).map_err(err)?;
    let course = if role(conn, who, course).is_ok() { conn.query_row("SELECT name FROM courses WHERE id = ?1", [course], |r| r.get(0)).optional().map_err(err)? } else { None };
    Ok(Side { name, course, score: s, gave_up: bt.gave_up == Some(user) })
}

fn battle_create(c: &Core, who: &Who, course_id: i64) -> R<i64> {
    let conn = c.db.lock().unwrap();
    role(&conn, who, course_id)?;
    // Je Konto höchstens eine offene Herausforderung
    conn.execute("DELETE FROM battles WHERE a_id = ?1 AND b_id IS NULL", [who.id]).map_err(err)?;
    conn.execute("INSERT INTO battles(a_id, a_course, created_at) VALUES(?1, ?2, ?3)", params![who.id, course_id, db::now()]).map_err(err)?;
    Ok(conn.last_insert_rowid())
}

/// Annehmen mit eigenem Kurs. Der Code (nicht die KI) wählt die Aufgaben so, dass an jeder Stelle beide dieselbe Art
/// mit gleich vielen Teilen haben. Gleicher Kurs → genau dieselben Fragen.
fn battle_join(c: &Core, who: &Who, id: i64, course_id: i64) -> R<()> {
    let conn = c.db.lock().unwrap();
    role(&conn, who, course_id)?;
    let bt = battle_row(&conn, id)?;
    if bt.a == who.id || bt.b.is_some() || bt.created_at < db::now() - OPEN_SECS {
        return Err("Dieses Battle ist nicht mehr offen.".into());
    }
    let same = course_id == bt.a_course;
    let a = battle_pool(&conn, bt.a_course, bt.a, if same { who.id } else { bt.a })?;
    let pairs = if same { a.iter().take(BATTLE_ROUNDS).map(|(q, _)| (*q, *q)).collect() } else { pair_up(&a, &battle_pool(&conn, course_id, who.id, who.id)?, BATTLE_ROUNDS) };
    if pairs.len() < BATTLE_MIN {
        return Err(format!(
            "Eure Kurse haben zu wenige gleichartige Aufgaben ({} von mindestens {BATTLE_MIN}). Wähle einen anderen Kurs oder lern erst weiter, dann entstehen mehr Fragen.",
            pairs.len()
        ));
    }
    let (a_qs, b_qs): (Vec<i64>, Vec<i64>) = pairs.into_iter().unzip();
    conn.execute(
        "UPDATE battles SET b_id = ?2, b_course = ?3, a_qs = ?4, b_qs = ?5, started_ms = ?6 WHERE id = ?1 AND b_id IS NULL",
        params![id, who.id, course_id, json!(a_qs).to_string(), json!(b_qs).to_string(), now_ms() + COUNTDOWN_MS],
    )
    .map_err(err)?;
    Ok(())
}

/// Meine Seite: (bin ich A?, meine Fragen). Fremde Battles → kein Zugriff.
fn my_side<'a>(bt: &'a Battle, who: &Who) -> R<(bool, &'a [i64])> {
    if bt.a == who.id {
        Ok((true, &bt.a_qs))
    } else if bt.b == Some(who.id) {
        Ok((false, &bt.b_qs))
    } else {
        Err(DENIED.into())
    }
}

fn battle_state(c: &Core, who: &Who, id: i64, with_exercises: bool) -> R<BattleOut> {
    let conn = c.db.lock().unwrap();
    let bt = battle_row(&conn, id)?;
    let (is_a, qs) = my_side(&bt, who)?;
    let (sa, sb) = (score(&conn, id, bt.a)?, bt.b.map(|b| score(&conn, id, b)).transpose()?.unwrap_or_default());
    let now = now_ms();
    let over = bt.over(sa, sb, now);
    let state = match bt.started_ms {
        None => "waiting",
        _ if over => "done",
        Some(t) if now < t => "countdown",
        _ => "running",
    };
    let a = side(&conn, who, &bt, bt.a, bt.a_course, sa)?;
    let b = match (bt.b, bt.b_course) {
        (Some(u), Some(course)) => Some(side(&conn, who, &bt, u, course, sb)?),
        _ => None,
    };
    let (me, them) = if is_a { (a, b) } else { (b.ok_or(DENIED)?, Some(a)) };
    let result = over.then(|| {
        let o = bt.outcome(sa, sb);
        (if is_a { o } else { o.reverse() }) as i8
    });
    let mut exercises = vec![];
    if with_exercises && state == "running" {
        let mut rng = learn::Shuffler::new();
        for q in qs {
            let p = query_questions(&conn, "SELECT q.id, q.kind, q.prompt, q.options, q.groups, q.spots, f.image FROM questions q LEFT JOIN figures f ON f.id = q.figure_id WHERE q.id = ?1", [q])?;
            // shortcut: mitten im Battle gelöschte Frage → hier ist Schluss, das Battle endet per Zeitlimit; Platzhalter-Aufgabe, falls das vorkommt.
            let Some(p) = p.into_iter().next() else { break };
            exercises.push(to_exercise(p, &mut rng));
        }
    }
    Ok(BattleOut { state, starts_in: bt.started_ms.map_or(0, |t| (t - now).max(0)), rounds: bt.rounds(), me, them, result, exercises })
}

/// Antworten nur der Reihe nach, nur während das Battle läuft. Die Zeit misst der Server.
fn battle_answer(c: &Core, who: &Who, id: i64, idx: usize, given: &str) -> R<AnswerOut> {
    let conn = c.db.lock().unwrap();
    let bt = battle_row(&conn, id)?;
    let (_, qs) = my_side(&bt, who)?;
    let (sa, sb) = (score(&conn, id, bt.a)?, bt.b.map(|b| score(&conn, id, b)).transpose()?.unwrap_or_default());
    let now = now_ms();
    if bt.started_ms.is_none_or(|t| now < t) {
        return Err("Das Battle hat noch nicht begonnen.".into());
    }
    if bt.over(sa, sb, now) {
        return Err("Das Battle ist schon vorbei.".into());
    }
    let mine = if bt.a == who.id { sa } else { sb };
    let question_id = *qs.get(idx).ok_or(DENIED)?;
    if idx != mine.answered {
        return Err("Diese Aufgabe ist schon beantwortet.".into());
    }
    let (out, _) = graded(&conn, who, question_id, given)?;
    conn.execute(
        "INSERT INTO battle_answers(battle_id, user_id, idx, correct, ms) VALUES(?1, ?2, ?3, ?4, ?5)",
        params![id, who.id, idx as i64, out.correct, now - bt.started_ms.unwrap_or(now)],
    )
    .map_err(err)?;
    Ok(out)
}

/// Warten → Herausforderung zurückziehen. Läuft es schon → aufgeben (zählt als verloren).
fn battle_leave(c: &Core, who: &Who, id: i64) -> R<()> {
    let conn = c.db.lock().unwrap();
    let bt = battle_row(&conn, id)?;
    my_side(&bt, who)?;
    if bt.b.is_none() {
        conn.execute("DELETE FROM battles WHERE id = ?1", [id]).map_err(err)?;
    } else {
        conn.execute("UPDATE battles SET gave_up = ?2 WHERE id = ?1 AND gave_up IS NULL", params![id, who.id]).map_err(err)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct OpenBattle {
    id: i64,
    name: String,
    /// Nur, wenn ich den Kurs auch sehen darf
    course: Option<String>,
}

/// Offene Herausforderungen der anderen und mein noch nicht beendetes Battle (zum Zurückkehren).
fn battle_lobby(c: &Core, who: &Who) -> R<Value> {
    let conn = c.db.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT b.id, u.name, b.a_course, c.name FROM battles b JOIN users u ON u.id = b.a_id JOIN courses c ON c.id = b.a_course
             WHERE b.b_id IS NULL AND b.a_id != ?1 AND b.created_at > ?2 AND u.disabled = 0 ORDER BY b.id DESC LIMIT 50",
        )
        .map_err(err)?;
    let rows: Vec<(i64, String, i64, String)> =
        stmt.query_map(params![who.id, db::now() - OPEN_SECS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
    let open: Vec<OpenBattle> = rows.into_iter().map(|(id, name, course, cname)| OpenBattle { id, name, course: role(&conn, who, course).is_ok().then_some(cname) }).collect();
    let active: Option<i64> = conn
        .query_row(
            "SELECT id FROM battles WHERE (a_id = ?1 OR b_id = ?1) AND gave_up IS NULL
               AND ((b_id IS NULL AND created_at > ?2) OR started_ms > ?3) ORDER BY id DESC LIMIT 1",
            params![who.id, db::now() - OPEN_SECS, now_ms() - BATTLE_MS],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    // Beide fertig → nicht mehr „laufend“
    let active = match active {
        Some(id) if !battle_over(&conn, id)? => Some(id),
        _ => None,
    };
    Ok(json!({ "open": open, "active": active }))
}

fn battle_over(conn: &Connection, id: i64) -> R<bool> {
    let bt = battle_row(conn, id)?;
    let (sa, sb) = (score(conn, id, bt.a)?, bt.b.map(|b| score(conn, id, b)).transpose()?.unwrap_or_default());
    Ok(bt.over(sa, sb, now_ms()))
}

#[derive(Serialize, Default)]
struct Rank {
    name: String,
    me: bool,
    battles: i64,
    wins: i64,
    correct: i64,
    total: i64,
}

/// Bestenliste eines Kurses (alle beendeten Battles, in denen jemand mit diesem Kurs angetreten ist)
/// oder ohne Kurs die des ganzen Servers (nur Namen und Zahlen, darum für alle Konten). Reihenfolge: Siege, dann Trefferquote.
fn leaderboard(c: &Core, who: &Who, course_id: Option<i64>) -> R<Vec<Rank>> {
    let conn = c.db.lock().unwrap();
    if let Some(id) = course_id {
        role(&conn, who, id)?;
    }
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM battles WHERE (?1 IS NULL OR a_course = ?1 OR b_course = ?1) AND b_id IS NOT NULL").map_err(err)?;
        let ids = stmt.query_map([course_id], |r| r.get(0)).map_err(err)?.collect::<Result<_, _>>().map_err(err)?;
        ids
    };
    let now = now_ms();
    let mut by_user: HashMap<i64, Rank> = HashMap::new();
    for id in ids {
        let bt = battle_row(&conn, id)?;
        let Some(b) = bt.b else { continue };
        let (sa, sb) = (score(&conn, id, bt.a)?, score(&conn, id, b)?);
        if !bt.over(sa, sb, now) {
            continue;
        }
        let o = bt.outcome(sa, sb);
        for (user, course, s, won) in [(bt.a, bt.a_course, sa, o.is_gt()), (b, bt.b_course.unwrap_or(0), sb, o.is_lt())] {
            if course_id.is_some_and(|id| id != course) {
                continue;
            }
            let r = by_user.entry(user).or_default();
            r.battles += 1;
            r.wins += won as i64;
            r.correct += s.correct as i64;
            r.total += bt.rounds() as i64;
        }
    }
    let mut out: Vec<Rank> = by_user
        .into_iter()
        .map(|(user, mut r)| {
            r.name = conn.query_row("SELECT name FROM users WHERE id = ?1", [user], |x| x.get(0)).unwrap_or_default();
            r.me = user == who.id;
            r
        })
        .collect();
    out.sort_by(|x, y| y.wins.cmp(&x.wins).then((y.correct * x.total.max(1)).cmp(&(x.correct * y.total.max(1)))).then(x.name.cmp(&y.name)));
    Ok(out)
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
            "INSERT INTO users(id, name, pw_hash, created_at) VALUES(2, 'Kommilitone', 'x', 0), (3, 'Fremde', 'x', 0);
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
    const U3: Who = Who { id: 3, admin: false };

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
    const PNG2: &str = "data:image/png;base64,iVBORw0KGgoA";

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
            ("replace_document", serde_json::json!({ "documentId": 20, "name": "neu.pdf", "pages": ["Text"] })),
            ("delete_document", serde_json::json!({ "documentId": 20 })),
            ("delete_document", serde_json::json!({ "documentId": 99999 })),
            ("start_lesson", serde_json::json!({ "wrongCourse": course })),
            ("share_people", serde_json::json!({ "courseId": course })),
            ("invite_to_course", serde_json::json!({ "courseId": course, "userIds": [2] })),
            ("reorder_documents", serde_json::json!({ "courseId": course, "documentIds": [20] })),
        ] {
            assert_eq!(run(&c, &U2, cmd, v).unwrap_err(), DENIED, "{cmd}");
        }
        assert_eq!(run(&c, &U2, "courses", Value::Null).unwrap(), serde_json::json!([]));
        assert_eq!(run(&c, &U2, "ai_set_tier", serde_json::json!({ "tier": "kompakt" })).unwrap_err(), DENIED);
    }

    #[test]
    fn reorder_documents() {
        let dir = tmp("reorder");
        let c = core(&dir);
        let (course, _, _, _) = seed(&c);
        c.db.lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO documents(id, course_id, name, pages, created_at) VALUES(21, 10, 'b.pdf', 2, 0), (22, 10, 'c.pdf', 1, 0);
                 INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text) VALUES
                   (31, 10, 20, 1, 4, 4, 'A2', 'x'), (32, 10, 21, 2, 1, 1, 'B1', 'x'), (33, 10, 21, 3, 2, 2, 'B2', 'x'), (34, 10, 22, 4, 1, 1, 'C1', 'x');",
            )
            .unwrap();
        // c.pdf fehlt in der Liste → bleibt hinten; innerhalb eines PDFs bleibt die Folge
        run(&c, &crate::LOCAL, "reorder_documents", serde_json::json!({ "courseId": course, "documentIds": [21, 20] })).unwrap();
        let path = run(&c, &crate::LOCAL, "course_path", serde_json::json!({ "courseId": course })).unwrap();
        let titles: Vec<&str> = path.as_array().unwrap().iter().map(|u| u["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["B1", "B2", "Nachfrage", "A2", "C1"]);
        let idx: Vec<i64> = path.as_array().unwrap().iter().map(|u| u["idx"].as_i64().unwrap()).collect();
        assert_eq!(idx, [0, 1, 2, 3, 4]);
        let docs = &run(&c, &crate::LOCAL, "courses", Value::Null).unwrap()[0]["documents"];
        assert_eq!(docs.as_array().unwrap().iter().map(|d| d["id"].as_i64().unwrap()).collect::<Vec<_>>(), [21, 20, 22]);
    }

    #[test]
    fn shared_course_edit_for_all_hide_for_me() {
        let dir = tmp("shared");
        let c = core(&dir);
        let (course, unit, q, _) = seed(&c);
        run(&c, &crate::LOCAL, "update_course", serde_json::json!({ "id": course, "name": "Mikro", "examDate": null, "share": "all" })).unwrap();

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
        run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "examDate": null, "share": "all" })).unwrap();

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
            { "page": 6, "title": "Aufbau eines Blattes", "image": PNG2, "spots": [spot(0.1, "Blattader"), spot(0.4, "Blattstiel")] }, // Animationsschritt davor
            { "page": 2, "image": "data:image/png;base64,iVBORw0KGgoB", "spots": [] },
            { "page": 9, "image": "data:text/html;base64,PGI+", "spots": [] }
        ]);
        let figures: Vec<import::FigureIn> = serde_json::from_value(figures).unwrap();
        let stored = import::store_figures(&c.db.lock().unwrap(), 20, &figures, &Default::default()).unwrap();
        assert_eq!(stored, 2);
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


    #[test]
    fn replace_and_delete_document() {
        use serde_json::json;
        let dir = tmp("replace");
        let c = core(&dir);
        let (course, _, _, _) = seed(&c);
        let pages = |v: &[String]| json!({ "documentId": 20, "name": "vl.pdf", "pages": v });
        // Ausgangslage: ein PDF mit drei Abschnitten (je eine Seite ~3600 Zeichen, eigener Wortschatz), Fragen in den ersten beiden
        let long = |t: &str| format!("{} ", (0..15).map(|i| format!("{t}wort{i}")).collect::<Vec<_>>().join(" ")).repeat(20);
        let (a, b, d) = (long("Alpha"), long("Beta"), long("Delta"));
        run(&c, &crate::LOCAL, "replace_document", pages(&[a.clone(), b.clone(), d.clone()])).unwrap();
        let units = |c: &Core| -> Vec<(i64, i64, String, i64)> {
            let conn = c.db.lock().unwrap();
            let mut st = conn.prepare("SELECT id, idx, status, q_state FROM units WHERE document_id = 20 ORDER BY idx").unwrap();
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().map(Result::unwrap).collect()
        };
        let u = units(&c);
        assert_eq!(u.len(), 3);
        {
            let conn = c.db.lock().unwrap();
            conn.execute_batch(&format!(
                "UPDATE units SET status = 'ready', q_state = 2, explanation = 'alt' WHERE document_id = 20;
                 INSERT INTO questions(id, unit_id, kind, prompt, options, answer) VALUES
                   (60, {0}, 'mc', 'Was ist Alpha genau?', '[\"a\",\"b\",\"c\",\"d\"]', 'a'),
                   (61, {1}, 'mc', 'Was ist Beta genau?', '[\"a\",\"b\",\"c\",\"d\"]', 'a');
                 INSERT INTO progress(user_id, question_id, box, due_at) VALUES(1, 60, 3, 0), (1, 61, 2, 0);
                 INSERT INTO lessons(unit_id, xp, correct, total, finished_at, user_id) VALUES({0}, 10, 2, 3, 0, 1);",
                u[0].0, u[1].0
            ))
            .unwrap();
        }

        // Gleiche Datei noch einmal: nichts ändert sich
        let r = run(&c, &crate::LOCAL, "replace_document", pages(&[a.clone(), b.clone(), d.clone()])).unwrap();
        assert_eq!((r["same"].as_i64(), r["changed"].as_i64(), r["units"].as_i64(), r["removed"].as_i64()), (Some(3), Some(0), Some(0), Some(0)));

        // Beta leicht geändert, Delta weg, Gamma neu
        let b2 = format!("{b} Zusatz Höchstpreis");
        let r = run(&c, &crate::LOCAL, "replace_document", pages(&[a.clone(), b2, long("Gamma")])).unwrap();
        assert_eq!((r["same"].as_i64(), r["changed"].as_i64(), r["units"].as_i64(), r["removed"].as_i64()), (Some(1), Some(1), Some(1), Some(1)));
        let after = units(&c);
        assert_eq!(after[0], (u[0].0, u[0].1, "ready".to_string(), 2)); // Alpha: unverändert
        assert_eq!((after[1].0, after[1].2.as_str(), after[1].3), (u[1].0, "pending", 4)); // Beta: neu analysieren, Fragen prüfen
        let third: String = c.db.lock().unwrap().query_row("SELECT source_text FROM units WHERE id = ?1", [after[2].0], |r| r.get(0)).unwrap();
        assert!(third.starts_with("Gammawort") && after[2].2 == "pending"); // Gamma neu statt Delta
        let (box60, expl): (i64, Option<String>) = c.db.lock().unwrap()
            .query_row("SELECT p.box, u.explanation FROM progress p JOIN questions q ON q.id = p.question_id JOIN units u ON u.id = q.unit_id WHERE q.id = 60", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((box60, expl.as_deref()), (3, Some("alt")));
        let expl_b: Option<String> = c.db.lock().unwrap().query_row("SELECT explanation FROM units WHERE id = ?1", [u[1].0], |r| r.get(0)).unwrap();
        assert!(expl_b.is_none());
        assert!(run(&c, &crate::LOCAL, "replace_document", pages(&["  ".into()])).is_err()); // ohne Text bleibt alles

        // Löschen: Einheiten, Fragen, Fortschritt weg; XP-Historie bleibt ohne Einheit
        run(&c, &crate::LOCAL, "delete_document", json!({ "documentId": 20 })).unwrap();
        let conn = c.db.lock().unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM units WHERE course_id = 10"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM questions"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM progress"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM lessons WHERE unit_id IS NULL"), 1);
        let _ = course;
    }

    #[test]
    fn wrong_questions_round() {
        use serde_json::json;
        let dir = tmp("wrong");
        let c = core(&dir);
        let (course, _, q, _) = seed(&c);
        let wrong = || run(&c, &crate::LOCAL, "courses", Value::Null).unwrap()[0]["wrong"].as_i64().unwrap();
        let round = || -> Vec<i64> {
            let l = run(&c, &crate::LOCAL, "start_lesson", json!({ "wrongCourse": course })).unwrap();
            l["exercises"].as_array().unwrap().iter().filter_map(|e| e["id"].as_i64()).collect()
        };
        let answer = |given: &str, retry: bool| run(&c, &crate::LOCAL, "answer", json!({ "questionId": q, "given": given, "retry": retry })).unwrap();
        assert_eq!(wrong(), 0);
        answer("Falsch", false);
        answer("Richtig", true); // Wiederholung in derselben Lektion zählt nicht
        assert_eq!((wrong(), round()), (1, vec![q]));
        answer("Richtig", false);
        assert_eq!((wrong(), round()), (0, vec![]));
        let (b, due): (i64, i64) = c.db.lock().unwrap().query_row("SELECT box, due_at FROM progress WHERE question_id = ?1", [q], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(b, 2);
        assert!(due - db::now() > 2 * 86_400); // erst in ein paar Tagen wieder fällig
    }

    #[test]
    fn share_with_selected() {
        use serde_json::json;
        let dir = tmp("members");
        let c = core(&dir);
        let (course, unit, q, _) = seed(&c);
        let share = |v: Value| run(&c, &crate::LOCAL, "update_course", v).unwrap();
        let sees = |who: &Who| run(&c, who, "courses", Value::Null).unwrap().as_array().unwrap().len() == 1;
        share(json!({ "id": course, "name": "Mikro", "share": "some", "members": [2, 1, 99] })); // Besitzer und Unbekannte fallen weg

        let list = run(&c, &crate::LOCAL, "courses", Value::Null).unwrap();
        assert_eq!((list[0]["shared"].clone(), list[0]["share_all"].clone(), list[0]["members"].clone()), (json!(true), json!(false), json!(1)));
        assert!(sees(&U2) && !sees(&U3));
        assert!(run(&c, &U2, "start_lesson", json!({ "unitId": unit })).is_ok());
        run(&c, &U2, "update_question", json!({ "questionId": q, "prompt": "Die Nachfragekurve fällt meistens.", "answer": "Richtig" })).unwrap();
        assert_eq!(run(&c, &U3, "course_path", json!({ "courseId": course })).unwrap_err(), DENIED);
        assert_eq!(run(&c, &U3, "search_terms", json!({ "query": "" })).unwrap(), json!([]));

        // Weiterteilen nur mit Erlaubnis des Besitzers, und dann nur hinzufügen
        assert_eq!(run(&c, &U2, "share_people", json!({ "courseId": course })).unwrap_err(), DENIED);
        assert_eq!(run(&c, &U2, "invite_to_course", json!({ "courseId": course, "userIds": [3] })).unwrap_err(), DENIED);
        share(json!({ "id": course, "name": "Mikro", "membersInvite": true }));
        let people = run(&c, &U2, "share_people", json!({ "courseId": course })).unwrap();
        assert_eq!(people, json!([{ "id": 3, "name": "Fremde", "member": false }])); // weder ich noch der Besitzer
        run(&c, &U2, "invite_to_course", json!({ "courseId": course, "userIds": [3] })).unwrap();
        assert!(sees(&U3));
        assert_eq!(run(&c, &U2, "update_course", json!({ "id": course, "name": "Mikro", "share": "none" })).unwrap_err(), DENIED);

        // Nur ich: alle sind raus
        share(json!({ "id": course, "name": "Mikro", "share": "none" }));
        assert!(!sees(&U2) && !sees(&U3));
        assert!(run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "share": "egal" })).is_err());
    }

    #[test]
    fn orga_units_and_exam_training() {
        use serde_json::json;
        let dir = tmp("exam");
        let c = core(&dir);
        let (course, unit, q, q2) = seed(&c);
        c.db.lock().unwrap().execute_batch(
            "INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text, status, orga) VALUES(31, 10, 20, 1, 4, 4, 'Organisatorisches', 'Klausur am 14.02.', 'orga', '[0]');
             UPDATE units SET source_text = 'Agenda\n\nDie Nachfrage fällt mit dem Preis.', orga = '[0]' WHERE id = 30;",
        )
        .unwrap();
        // Lerntext ohne Orga-Seite, Lernkarte zählt sie
        assert_eq!(unit_text(&c, &crate::LOCAL, unit).unwrap().1, "Die Nachfrage fällt mit dem Preis.");
        assert_eq!(run(&c, &crate::LOCAL, "unit", json!({ "unitId": unit })).unwrap()["orga"], 1);
        // Orga-Einheit zählt als fertig, liefert aber keine Lektion
        assert_eq!(run(&c, &crate::LOCAL, "courses", Value::Null).unwrap()[0]["ready"], 2);
        assert_eq!(run(&c, &crate::LOCAL, "start_lesson", json!({ "unitId": 31 })).unwrap()["ready"], false);
        // „Doch lernen“: nur der Besitzer; danach ohne Filter neu analysieren
        run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "share": "all" })).unwrap();
        assert_eq!(run(&c, &U2, "include_orga", json!({ "unitId": 31 })).unwrap_err(), DENIED);
        run(&c, &crate::LOCAL, "include_orga", json!({ "unitId": 31 })).unwrap();
        let (status, orga): (String, Option<String>) = c.db.lock().unwrap().query_row("SELECT status, orga FROM units WHERE id = 31", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((status.as_str(), orga), ("pending", None));
        assert!(matches!(db::next_task(&c.db.lock().unwrap()).unwrap(), Some(db::Task::Analyze { unit_id: 31, filter: false, .. })));

        // Schwerpunkte setzen: Einheiten werden neu zugeordnet; Fragen warten, bis die Einheit geprüft ist
        let exam = |who: &Who| run(&c, who, "courses", Value::Null).unwrap()[0]["exam"].as_i64().unwrap();
        let training = |who: &Who| -> Vec<i64> {
            let l = run(&c, who, "start_lesson", json!({ "examCourse": course })).unwrap();
            let mut ids: Vec<i64> = l["exercises"].as_array().unwrap().iter().filter_map(|e| e["id"].as_i64()).collect();
            ids.sort();
            ids
        };
        c.db.lock().unwrap().execute("UPDATE units SET status = 'ready', exam = '[]', q_state = 1 WHERE id = 30", []).unwrap();
        run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "examFocus": "1. Nachfrage\n2. Marktformen" })).unwrap();
        assert_eq!(run(&c, &crate::LOCAL, "courses", Value::Null).unwrap()[0]["exam_focus"], "1. Nachfrage\n2. Marktformen");
        match db::next_task(&c.db.lock().unwrap()).unwrap() {
            Some(db::Task::Focus { unit_id, topics, .. }) => assert_eq!((unit_id, topics), (30, vec!["Nachfrage".to_string(), "Marktformen".to_string()])),
            _ => panic!("erst die Prüfungsthemen der wartenden Einheit"),
        }
        assert_eq!((exam(&crate::LOCAL), training(&crate::LOCAL)), (0, vec![]));

        // Lesezeichen: nur für mich
        run(&c, &U2, "mark_question", json!({ "questionId": q, "marked": true })).unwrap();
        assert_eq!((exam(&U2), training(&U2), exam(&crate::LOCAL)), (1, vec![q], 0));
        assert_eq!(run(&c, &U2, "question", json!({ "questionId": q })).unwrap()["marked"], true);

        // Einheit passt zu den Schwerpunkten → alle ihre Fragen gehören zum Training, die Einheit ist markiert
        c.db.lock().unwrap().execute("UPDATE units SET exam = '[\"Nachfrage\"]' WHERE id = 30", []).unwrap();
        assert_eq!((exam(&crate::LOCAL), training(&crate::LOCAL)), (2, vec![q, q2]));
        assert_eq!(run(&c, &crate::LOCAL, "course_path", json!({ "courseId": course })).unwrap()[0]["exam"], true);
        assert!(matches!(db::next_task(&c.db.lock().unwrap()).unwrap(), Some(db::Task::Questions { unit_id: 30, ref topics, .. }) if topics == &["Nachfrage".to_string()]));
    }

    #[test]
    fn leave_and_rejoin() {
        use serde_json::json;
        let dir = tmp("leave");
        let c = core(&dir);
        let (course, unit, _, _) = seed(&c);
        let sees = |who: &Who| run(&c, who, "courses", Value::Null).unwrap().as_array().unwrap().len() == 1;
        assert!(run(&c, &crate::LOCAL, "leave_course", json!({ "courseId": course })).is_err()); // Besitzer löscht, verlässt nicht

        // Für alle geteilt: verlassen blendet aus, wieder anzeigen holt zurück
        run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "share": "all" })).unwrap();
        run(&c, &U2, "leave_course", json!({ "courseId": course })).unwrap();
        assert!(!sees(&U2) && sees(&U3));
        assert_eq!(run(&c, &U2, "course_path", json!({ "courseId": course })).unwrap_err(), DENIED);
        assert_eq!(run(&c, &U2, "left_courses", Value::Null).unwrap(), json!([{ "id": course, "name": "Mikro" }]));
        run(&c, &U2, "rejoin_course", json!({ "courseId": course })).unwrap();
        assert!(sees(&U2) && run(&c, &U2, "start_lesson", json!({ "unitId": unit })).is_ok());

        // Ausgewählte Personen: verlassen löscht die Mitgliedschaft, neue Einladung holt zurück
        run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "share": "some", "members": [2] })).unwrap();
        run(&c, &U2, "leave_course", json!({ "courseId": course })).unwrap();
        assert!(!sees(&U2));
        run(&c, &crate::LOCAL, "update_course", json!({ "id": course, "name": "Mikro", "share": "some", "members": [2] })).unwrap();
        assert!(sees(&U2));
    }

    #[test]
    fn pair_up_matches_kind_and_size() {
        let k = |s: &str| s.to_string();
        let a = [(1, k("mc:4")), (2, k("sort:6:2")), (3, k("true_false:2")), (4, k("mc:4"))];
        let b = [(9, k("true_false:2")), (8, k("mc:4")), (7, k("sort:5:2"))];
        // sort mit 6 statt 5 Aussagen ist mehr Arbeit → bleibt draußen; zweite mc findet keinen Partner mehr
        assert_eq!(pair_up(&a, &b, 8), vec![(1, 8), (3, 9)]);
    }

    #[test]
    fn battle_flow() {
        let dir = tmp("battle");
        let c = core(&dir);
        let (course, ..) = seed(&c);
        c.db.lock().unwrap().execute_batch(
            "INSERT INTO questions(id, unit_id, kind, prompt, options, answer) VALUES
               (42, 30, 'mc', 'Was verschiebt die Kurve?', '[\"Einkommen\",\"Preis\",\"Menge\",\"Zeit\"]', 'Einkommen');
             INSERT INTO courses(id, name, created_at, owner_id) VALUES(11, 'Makro', 0, 2);
             INSERT INTO documents(id, course_id, name, pages, created_at) VALUES(21, 11, 'm.pdf', 1, 0);
             INSERT INTO units(id, course_id, document_id, idx, page_from, page_to, title, source_text, status, q_state)
               VALUES(31, 11, 21, 0, 1, 1, 'BIP', 'Text', 'ready', 2);
             INSERT INTO questions(id, unit_id, kind, prompt, options, answer) VALUES
               (50, 31, 'mc', 'BIP misst?', '[\"Wertschöpfung\",\"Preise\",\"Zinsen\",\"Löhne\"]', 'Wertschöpfung'),
               (51, 31, 'true_false', 'Inflation senkt die Kaufkraft.', '[\"Richtig\",\"Falsch\"]', 'Richtig'),
               (52, 31, 'mc', 'Wer macht Geldpolitik?', '[\"EZB\",\"Bund\",\"Länder\",\"IWF\"]', 'EZB'),
               (53, 31, 'order', 'Ordne', '[\"a\",\"b\",\"c\"]', '');",
        )
        .unwrap();
        let me = crate::LOCAL;
        let id = run(&c, &U2, "battle_create", json!({ "courseId": 11 })).unwrap();
        // Lobby: fremder Kurs ohne Namen; U3 darf mit einem Kurs, den er nicht sieht, nicht annehmen
        let lobby = run(&c, &me, "battle_lobby", json!({})).unwrap();
        assert_eq!(lobby["open"][0]["name"], "Kommilitone");
        assert!(lobby["open"][0]["course"].is_null());
        assert_eq!(run(&c, &U3, "battle_join", json!({ "battleId": id, "courseId": course })).unwrap_err(), DENIED);
        run(&c, &me, "battle_join", json!({ "battleId": id, "courseId": course })).unwrap();
        assert!(run(&c, &U3, "battle_join", json!({ "battleId": id, "courseId": course })).is_err(), "schon vergeben");
        assert_eq!(run(&c, &U3, "battle", json!({ "battleId": id })).unwrap_err(), DENIED);
        let st = run(&c, &me, "battle", json!({ "battleId": id, "exercises": true })).unwrap();
        assert_eq!((st["state"].as_str(), st["exercises"].as_array().unwrap().len()), (Some("countdown"), 0), "Aufgaben erst nach dem Countdown");
        assert!(run(&c, &me, "battle_answer", json!({ "battleId": id, "idx": 0, "given": "x" })).is_err());

        c.db.lock().unwrap().execute("UPDATE battles SET started_ms = ?1", [now_ms() - 1000]).unwrap();
        let ex = |who: &Who| run(&c, who, "battle", json!({ "battleId": id, "exercises": true })).unwrap()["exercises"].as_array().unwrap().clone();
        let (mine, theirs) = (ex(&me), ex(&U2));
        assert_eq!(mine.len(), 3, "tf + 2 mc passen, die Reihenfolge-Aufgabe hat keinen Partner");
        for (x, y) in mine.iter().zip(&theirs) {
            assert_eq!((&x["kind"], x["options"].as_array().unwrap().len()), (&y["kind"], y["options"].as_array().unwrap().len()), "an jeder Stelle dieselbe Art");
        }
        assert_eq!(run(&c, &me, "battle_answer", json!({ "battleId": id, "idx": 1, "given": "x" })).unwrap_err(), "Diese Aufgabe ist schon beantwortet.", "nur der Reihe nach");
        for i in 0..3 {
            run(&c, &me, "battle_answer", json!({ "battleId": id, "idx": i, "given": "falsch" })).unwrap();
            let q = theirs[i]["id"].as_i64().unwrap();
            let right: String = c.db.lock().unwrap().query_row("SELECT answer FROM questions WHERE id = ?1", [q], |r| r.get(0)).unwrap();
            assert!(run(&c, &U2, "battle_answer", json!({ "battleId": id, "idx": i, "given": right })).unwrap()["correct"].as_bool().unwrap());
        }
        let (a, b) = (run(&c, &me, "battle", json!({ "battleId": id })).unwrap(), run(&c, &U2, "battle", json!({ "battleId": id })).unwrap());
        assert_eq!((a["state"].as_str(), a["result"].as_i64(), b["result"].as_i64()), (Some("done"), Some(-1), Some(1)));
        assert_eq!(b["me"]["correct"], 3);
        assert!(run(&c, &me, "battle_lobby", json!({})).unwrap()["active"].is_null(), "beendet ist nicht mehr laufend");

        let board = run(&c, &U2, "leaderboard", json!({ "courseId": 11 })).unwrap();
        assert_eq!((board[0]["name"].as_str(), board[0]["wins"].as_i64(), board.as_array().unwrap().len()), (Some("Kommilitone"), Some(1), 1));
        assert_eq!(run(&c, &me, "leaderboard", json!({ "courseId": course })).unwrap()[0]["wins"], 0);
        assert_eq!(run(&c, &U3, "leaderboard", json!({ "courseId": course })).unwrap_err(), DENIED);
        // Ganzer Server: beide Seiten zählen, auch für Konten ohne Zugriff auf die Kurse
        let all = run(&c, &U3, "leaderboard", json!({})).unwrap();
        assert_eq!((all[0]["name"].as_str(), all[0]["wins"].as_i64(), all[1]["name"].as_str(), all.as_array().unwrap().len()), (Some("Kommilitone"), Some(1), Some("Ich"), 2));
        // Fortschritt (Leitner-Boxen) bleibt vom Battle unberührt
        let n: i64 = c.db.lock().unwrap().query_row("SELECT COUNT(*) FROM progress", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
