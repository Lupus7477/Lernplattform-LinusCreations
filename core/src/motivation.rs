//! Motivation ohne Druck. Erfolge vergibt der Code. Titel, Erfolgsnamen und Motivationstexte schreibt die KI
//! pro Kurs passend zum Kursnamen (für alle im Kurs gleich). Der Code prüft jeden Text, entscheidet, wann welcher
//! Anlass kommt, und setzt die echten Werte ein. Die Bedingung eines Erfolgs ist immer fester Code-Text.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::{Ai, AiError};
use crate::db::Db;
use crate::learn::{self, normalize};

// ---------- Erfolge ----------

#[derive(Clone, Copy, Debug)]
pub struct Badge {
    pub id: &'static str,
    /// Fester Name, gilt solange die KI für den Kurs nichts Passendes geschrieben hat (und immer für allgemeine Erfolge)
    pub title: &'static str,
    /// Bedingung, immer vom Code
    pub desc: &'static str,
    pub icon: &'static str,
    /// Allgemein (kursübergreifende Lernserie) statt pro Kurs
    pub global: bool,
}

const fn b(id: &'static str, title: &'static str, desc: &'static str, icon: &'static str, global: bool) -> Badge {
    Badge { id, title, desc, icon, global }
}

pub const BADGES: [Badge; 14] = [
    b("first", "Los geht's", "Schließe deine erste Lektion in diesem Kurs ab.", "star-fill", false),
    b("ten", "Dranbleiben", "Schließe 10 Lektionen in diesem Kurs ab.", "cards", false),
    b("fifty", "Ausdauer", "Schließe 50 Lektionen in diesem Kurs ab.", "trophy", false),
    b("tasks100", "100 Aufgaben", "Löse 100 Aufgaben in diesem Kurs.", "check-circle-fill", false),
    b("tasks500", "500 Aufgaben", "Löse 500 Aufgaben in diesem Kurs.", "seal-check-fill", false),
    b("perfect", "Fehlerfrei", "Schaffe eine Lektion mit mindestens 5 Aufgaben ohne Fehler.", "sparkle", false),
    b("run10", "Im Fluss", "Beantworte 10 Aufgaben hintereinander richtig.", "fire-fill", false),
    b("master", "Gemeistert", "Bring eine Einheit auf die Krone (Lernstand 4 von 5).", "crown-fill", false),
    b("script", "Skript durch", "Übe jede Einheit eines Skripts mindestens einmal.", "book-open-text", false),
    b("exam", "Prüfungsbereit", "Meistere alle prüfungsrelevanten Einheiten des Kurses.", "target", false),
    b("level10", "Level 10", "Erreiche Level 10 in diesem Kurs.", "trophy", false),
    b("week", "Eine Woche", "Lerne 7 Tage in Folge.", "calendar-blank", true),
    b("month", "Ein Monat", "Lerne 30 Tage in Folge.", "crown-fill", true),
    b("comeback", "Wieder da", "Lerne nach ein paar Tagen Pause weiter. Pausen gehören dazu.", "arrow-clockwise", true),
];

/// Was der Code über den Lernstand weiß. Kurs-Erfolge bekommen die Werte des Kurses, Serien-Erfolge die allgemeinen.
#[derive(Default, Debug)]
pub struct Facts {
    pub lessons: i64,
    pub tasks: i64,
    pub perfect: bool,
    pub best_run: i64,
    pub best_streak: i64,
    pub mastered: bool,
    pub script: bool,
    pub exam: bool,
    pub level: i64,
    pub comeback: bool,
}

/// Alle Erfolge, deren Bedingung erfüllt ist (auch schon früher freigeschaltete).
pub fn earned(f: &Facts) -> Vec<&'static Badge> {
    let ok = |id: &str| match id {
        "first" => f.lessons >= 1,
        "ten" => f.lessons >= 10,
        "fifty" => f.lessons >= 50,
        "tasks100" => f.tasks >= 100,
        "tasks500" => f.tasks >= 500,
        "perfect" => f.perfect,
        "run10" => f.best_run >= 10,
        "master" => f.mastered,
        "script" => f.script,
        "exam" => f.exam,
        "level10" => f.level >= 10,
        "week" => f.best_streak >= 7,
        "month" => f.best_streak >= 30,
        "comeback" => f.comeback,
        _ => false,
    };
    BADGES.iter().filter(|b| ok(b.id)).collect()
}

/// Erfolg, wie ihn die Oberfläche bekommt: mit dem Namen des Kurses.
#[derive(Serialize, Debug)]
pub struct BadgeOut {
    pub id: &'static str,
    pub title: String,
    pub desc: &'static str,
    pub icon: &'static str,
    pub global: bool,
    /// Freigeschaltet (Unix-Sekunden), sonst null
    pub at: Option<i64>,
}

// ---------- Titel und Erfolgsnamen pro Kurs ----------

/// Teil-Name für den Worker: Titel und Erfolgsnamen (sonst ist der Teil ein Anlass für Motivationstexte).
pub const NAMES: &str = "names";

#[derive(Serialize, Deserialize, Default)]
struct Stored {
    /// 10 Titel, leer = ungültig, der Ersatz gilt
    titles: Vec<String>,
    badges: HashMap<String, String>,
}

/// Titel und Erfolgsnamen eines Kurses, `{course}` schon eingesetzt.
pub struct Names {
    pub titles: Vec<String>,
    badges: HashMap<String, String>,
}

impl Names {
    /// Titel zum Level: je 10 Level ein neuer.
    pub fn title(&self, level: i64) -> String {
        self.titles[((level.clamp(1, learn::MAX_LEVEL) - 1) / 10).min(9) as usize].clone()
    }

    /// Nächster Titel auf der Leiter: (ab Level, Titel), None auf der letzten Stufe.
    pub fn next_title(&self, level: i64) -> Option<(i64, String)> {
        let at = ((level.clamp(1, learn::MAX_LEVEL) - 1) / 10 + 1) * 10 + 1;
        (at <= 91).then(|| (at, self.title(at)))
    }

    pub fn out(&self, b: &Badge, at: Option<i64>) -> BadgeOut {
        let title = if b.global { b.title.to_string() } else { self.badges.get(b.id).cloned().unwrap_or_else(|| b.title.to_string()) };
        BadgeOut { id: b.id, title, desc: b.desc, icon: b.icon, global: b.global, at }
    }
}

fn course_name(conn: &Connection, course: i64) -> Option<String> {
    conn.query_row("SELECT name FROM courses WHERE id = ?1", [course], |r| r.get(0)).optional().ok().flatten()
}

/// Namen eines Kurses aus `courses.motivation`. Fehlt etwas (KI noch nicht fertig oder Text verworfen),
/// gilt „{Kurs} · Stufenwort“ bzw. der feste Erfolgsname. Ohne Kurs: nur die festen Namen.
pub fn names(conn: &Connection, course: Option<i64>) -> Names {
    let row: Option<(String, Option<String>)> =
        course.and_then(|id| conn.query_row("SELECT name, motivation FROM courses WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional().ok().flatten());
    let Some((name, json)) = row else {
        return Names { titles: learn::TITLES.iter().map(|t| t.to_string()).collect(), badges: HashMap::new() };
    };
    let s: Stored = json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    let set = |t: &str| t.replace("{course}", &name);
    Names {
        titles: (0..10)
            .map(|i| s.titles.get(i).filter(|t| !t.is_empty()).map(|t| set(t)).unwrap_or_else(|| format!("{name} · {}", learn::TITLES[i])))
            .collect(),
        badges: s.badges.iter().map(|(k, v)| (k.clone(), set(v))).collect(),
    }
}

/// Prüft einen Namen der KI (Titel oder Erfolg): kurz, ohne Ziffern, nur `{course}` als Platzhalter, kein Druck.
pub fn fit_name(text: &str) -> Option<String> {
    let t = clean(text);
    let n = t.chars().count();
    (3..=40).contains(&n).then_some(())?;
    placeholders_ok(t, &["course"]).then(|| t.to_string())
}

fn clean(text: &str) -> &str {
    text.trim().trim_matches(|c: char| c == '"' || c == '„' || c == '“').trim()
}

/// Keine Ziffern (Zahlen kommen nur über Platzhalter), keine Druckwörter, nur erlaubte Platzhalter, Klammern sauber.
fn placeholders_ok(t: &str, allowed: &[&str]) -> bool {
    let low = t.to_lowercase();
    if t.chars().any(|c| c.is_ascii_digit()) || t.matches('{').count() != t.matches('}').count() || PRESSURE.iter().any(|w| low.contains(w)) {
        return false;
    }
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        let Some(j) = rest[i..].find('}') else { return false };
        if !allowed.contains(&&rest[i + 1..i + j]) {
            return false;
        }
        rest = &rest[i + j + 1..];
    }
    true
}

/// Die KI schreibt Titel und Erfolgsnamen für einen Kurs. Ungültige Einträge bleiben leer, dort gilt der Ersatz.
pub fn write_names(db: &Db, ai: &Ai, course: i64) -> Result<usize, AiError> {
    let Some(name) = course_name(&db.lock().unwrap(), course) else { return Ok(0) };
    let slots: Vec<&Badge> = BADGES.iter().filter(|b| !b.global).collect();
    let list: Vec<String> = slots.iter().map(|b| format!("- {}: {}", b.id, b.desc)).collect();
    let user = format!(
        "Ein Lernkurs heißt „{name}“. Erfinde dafür Namen, die zum Thema passen und motivieren. \
         Schreibe statt des Kursnamens immer {{course}}, die App setzt ihn ein.\n\n\
         titles: 10 Titel für die Lernstufen, aufsteigend vom Anfänger bis zur höchsten Meisterschaft, \
         z. B. „{{course}}-Starter“ … „{{course}}-Gott“. Jeder Titel anders.\n\
         badges: je ein kurzer Name für diese Erfolge:\n{}\n\n\
         Regeln: Deutsch, höchstens 40 Zeichen je Name, keine Zahlen und keine Ziffern, freundlich und gern mit Witz, nie abwertend.",
        list.join("\n")
    );
    let name_schema = json!({"type": "string", "maxLength": 40});
    let props: serde_json::Map<String, Value> = slots.iter().map(|b| (b.id.to_string(), name_schema.clone())).collect();
    let schema = json!({"type": "object", "required": ["titles", "badges"], "additionalProperties": false, "properties": {
        "titles": {"type": "array", "minItems": 10, "maxItems": 10, "items": name_schema},
        "badges": {"type": "object", "required": slots.iter().map(|b| b.id).collect::<Vec<_>>(), "additionalProperties": false, "properties": props}
    }});
    let res = crate::import::ask_json::<Stored>(ai, user, &schema, 700);
    if matches!(res, Err(AiError::NotReady)) {
        return Err(AiError::NotReady);
    }
    let conn = db.lock().unwrap();
    bump(&conn, course, NAMES);
    let raw = res?;
    let mut seen = HashSet::new();
    let titles: Vec<String> = raw.titles.iter().take(10).map(|t| fit_name(t).filter(|t| seen.insert(normalize(t))).unwrap_or_default()).collect();
    let badges: HashMap<String, String> = raw.badges.iter().filter(|(k, _)| slots.iter().any(|b| b.id == k.as_str())).filter_map(|(k, v)| Some((k.clone(), fit_name(v)?))).collect();
    let valid = titles.iter().filter(|t| !t.is_empty()).count() + badges.len();
    // Nichts Brauchbares: NULL lassen, der Worker versucht es noch einmal (höchstens ROUNDS Mal)
    if valid > 0 {
        let json = serde_json::to_string(&Stored { titles, badges }).map_err(|e| AiError::Failed(e.to_string()))?;
        conn.execute("UPDATE courses SET motivation = ?1 WHERE id = ?2", params![json, course]).map_err(|e| AiError::Failed(e.to_string()))?;
    }
    Ok(valid)
}

/// Nächster Kurs ohne Titel und Erfolgsnamen (für den Worker, gleich nach den gewünschten Fragen).
pub fn wanted_names(conn: &Connection) -> Option<i64> {
    conn.query_row(
        "SELECT c.id FROM courses c WHERE c.motivation IS NULL
           AND COALESCE((SELECT n FROM motivation_rounds r WHERE r.course_id = c.id AND r.part = ?1), 0) < ?2 ORDER BY c.id LIMIT 1",
        params![NAMES, ROUNDS],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

// ---------- Motivationstexte pro Kurs ----------

pub struct Occasion {
    pub id: &'static str,
    /// Anlass, so wie ihn die KI beschrieben bekommt
    about: &'static str,
    /// Erlaubte Platzhalter (dazu immer {course}), der Code setzt die echten Werte ein
    vars: &'static [&'static str],
    /// Gilt, solange die KI für diesen Kurs und Anlass noch nichts Brauchbares geschrieben hat
    fallback: &'static str,
    max: usize,
}

const fn o(id: &'static str, about: &'static str, vars: &'static [&'static str], fallback: &'static str, max: usize) -> Occasion {
    Occasion { id, about, vars, fallback, max }
}

pub const OCCASIONS: [Occasion; 10] = [
    o("level_up", "Jemand hat beim Lernen ein neues Level erreicht und dafür einen neuen Titel bekommen.", &["level", "title"], "Level {level} erreicht: {title}. Stark!", 120),
    o("badge", "Jemand hat gerade einen Erfolg freigeschaltet.", &["badge"], "Neuer Erfolg: {badge}. Verdient!", 120),
    o("record", "Jemand hat einen neuen persönlichen Rekord an Lerntagen in Folge aufgestellt.", &["streak"], "Neuer Rekord: {streak} Tage in Folge!", 120),
    o("freeze", "Jemand hat einen Tag nicht gelernt. Ein Pausentag hat die Lernserie gerettet. Pausen sind in Ordnung.", &["streak"], "Ein Pausentag hat deine Serie von {streak} Tagen gerettet. Pausen gehören dazu.", 120),
    o("comeback", "Jemand lernt nach ein paar Tagen Pause wieder. Schön, dass die Person zurück ist, ohne Vorwurf.", &[], "Schön, dass du wieder da bist. Genau da machst du weiter.", 120),
    o("improved", "Jemand hat eine Einheit besser gelöst als beim letzten Mal.", &["unit"], "Besser als beim letzten Mal in „{unit}“. Das ist echter Fortschritt.", 120),
    o("perfect", "Jemand hat eine ganze Lektion ohne einen einzigen Fehler geschafft.", &[], "Fehlerfrei. Das sitzt!", 120),
    o("tough", "Jemand hat eine schwere Lektion mit vielen Fehlern trotzdem zu Ende gebracht. Aufmuntern, nicht bewerten: Fehler gehören zum Lernen.", &[], "Drangeblieben, obwohl es schwer war. Genau so lernt man.", 120),
    o("done", "Jemand hat eine Lektion abgeschlossen.", &[], "Lektion geschafft. Gut gemacht!", 120),
    o("boost", "Jemand hatte gerade ein paar Fehler und beantwortet jetzt eine Frage richtig, die schon sitzt. Ganz kurz bestärken.", &[], "Siehst du, das sitzt!", 60),
];

/// Texte je Kurs und Anlass, höchstens so viele KI-Runden je Teil (auch gescheiterte zählen, damit der Worker nie hängt).
const KEEP: i64 = 6;
const ROUNDS: i64 = 3;
/// Wörter, die Druck oder Schuld machen. Texte damit verwirft der Code.
const PRESSURE: [&str; 7] = ["musst", "müsstest", "enttäusch", "faul", "versag", "schäm", "peinlich"];

pub fn occasion(id: &str) -> &'static Occasion {
    OCCASIONS.iter().find(|o| o.id == id).unwrap_or(&OCCASIONS[8])
}

/// Prüft einen KI-Text: kurz, ohne Ziffern, nur die Platzhalter des Anlasses und {course}, kein Druck.
pub fn fit(o: &Occasion, text: &str) -> Option<String> {
    let t = clean(text);
    let n = t.chars().count();
    let allowed: Vec<&str> = o.vars.iter().copied().chain(["course"]).collect();
    (n >= 8 && n <= o.max && placeholders_ok(t, &allowed)).then(|| t.to_string())
}

/// Erste Spalte aller Zeilen (Fehler = leer: Motivation darf nie eine Lektion blockieren).
fn texts<T: rusqlite::types::FromSql>(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Vec<T> {
    let Ok(mut st) = conn.prepare(sql) else { return vec![] };
    let rows = st.query_map(p, |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect());
    rows.unwrap_or_default()
}

fn fill(text: String, vars: &[(&str, String)]) -> String {
    vars.iter().fold(text, |t, (k, v)| t.replace(&format!("{{{k}}}"), v))
}

/// Ein zufälliger geprüfter Text des Kurses zum Anlass (sonst der feste) mit den echten Werten.
pub fn pick(conn: &Connection, course: Option<i64>, id: &str, vars: &[(&str, String)]) -> String {
    picks(conn, course, id, 1).pop().map(|t| fill(t, vars)).unwrap_or_default()
}

/// Bis zu `n` verschiedene Texte des Kurses, {course} schon eingesetzt (für die Lektion: Lob nach einer Erfolgsfrage).
pub fn picks(conn: &Connection, course: Option<i64>, id: &str, n: usize) -> Vec<String> {
    let o = occasion(id);
    let mut out = match course {
        Some(c) => texts(conn, "SELECT text FROM cheers WHERE course_id = ?1 AND occasion = ?2 ORDER BY RANDOM() LIMIT ?3", params![c, o.id, n as i64]),
        None => vec![],
    };
    if out.is_empty() {
        out.push(o.fallback.to_string());
    }
    let name = course.and_then(|c| course_name(conn, c)).unwrap_or_default();
    out.into_iter().map(|t| t.replace("{course}", &name)).collect()
}

/// Was am Ende einer Lektion passiert ist. Der Code wählt daraus genau einen Anlass.
#[derive(Default)]
pub struct Moment {
    pub level_up: Option<(i64, String)>,
    pub badge: Option<String>,
    pub record: Option<i64>,
    pub comeback: bool,
    pub improved: Option<String>,
    pub perfect: bool,
    /// Trefferquote in Prozent
    pub acc: i64,
}

/// Feste Reihenfolge: Level > Erfolg > Rekord > Comeback > besser als letztes Mal > fehlerfrei > schwer (< 50 %) > geschafft.
pub fn cheer_for(conn: &Connection, course: Option<i64>, m: &Moment) -> String {
    let (id, vars): (&str, Vec<(&str, String)>) = if let Some((lvl, title)) = &m.level_up {
        ("level_up", vec![("level", lvl.to_string()), ("title", title.clone())])
    } else if let Some(b) = &m.badge {
        ("badge", vec![("badge", b.clone())])
    } else if let Some(s) = m.record {
        ("record", vec![("streak", s.to_string())])
    } else if m.comeback {
        ("comeback", vec![])
    } else if let Some(u) = &m.improved {
        ("improved", vec![("unit", u.clone())])
    } else if m.perfect {
        ("perfect", vec![])
    } else if m.acc < 50 {
        ("tough", vec![])
    } else {
        ("done", vec![])
    };
    pick(conn, course, id, &vars)
}

fn rounds(conn: &Connection, course: i64, part: &str) -> i64 {
    conn.query_row("SELECT n FROM motivation_rounds WHERE course_id = ?1 AND part = ?2", params![course, part], |r| r.get(0)).optional().ok().flatten().unwrap_or(0)
}

fn bump(conn: &Connection, course: i64, part: &str) {
    let _ = conn.execute(
        "INSERT INTO motivation_rounds(course_id, part, n) VALUES(?1, ?2, 1) ON CONFLICT(course_id, part) DO UPDATE SET n = n + 1",
        params![course, part],
    );
}

/// Nächster Kurs und Anlass, für den die KI Texte schreiben soll (für den Worker, ganz zuletzt).
// ponytail: je Kurs und Anlass eine Zählabfrage; eine Abfrage mit GROUP BY, falls ein Server sehr viele Kurse hat.
pub fn wanted_cheers(conn: &Connection) -> Option<(i64, &'static str)> {
    let courses: Vec<i64> = texts(conn, "SELECT id FROM courses ORDER BY id", []);
    courses.into_iter().find_map(|c| {
        OCCASIONS.iter().map(|o| o.id).find(|id| {
            let have: i64 = conn.query_row("SELECT COUNT(*) FROM cheers WHERE course_id = ?1 AND occasion = ?2", params![c, id], |r| r.get(0)).unwrap_or(KEEP);
            have < KEEP && rounds(conn, c, id) < ROUNDS
        })
        .map(|id| (c, id))
    })
}

/// Die KI schreibt eine Runde Texte für einen Kurs und Anlass, nur geprüfte kommen in die Datenbank.
pub fn write(db: &Db, ai: &Ai, course: i64, id: &str) -> Result<usize, AiError> {
    let o = occasion(id);
    let Some(name) = course_name(&db.lock().unwrap(), course) else { return Ok(0) };
    let list: Vec<String> = o.vars.iter().chain(&["course"]).map(|v| format!("{{{v}}}")).collect();
    let user = format!(
        "Schreibe 6 verschiedene Motivationstexte für eine Lern-App, Kurs „{name}“. Gern mit Bezug zum Thema des Kurses.\n\
         Anlass: {}\n\
         Nutze diese Platzhalter genau so geschrieben, die App setzt später die echten Werte ein: {}. \
         Schreibe statt des Kursnamens immer {{course}}.\n\n\
         Regeln:\n- Deutsch, du-Form, ein Satz, höchstens {} Zeichen\n- keine Zahlen und keine Ziffern\n\
         - warm und ehrlich, kein Druck, keine Schuld, keine Übertreibung\n- jeder Text fängt anders an",
        o.about,
        list.join(", "),
        o.max
    );
    let schema = json!({"type": "object", "required": ["texts"], "properties": {"texts": {"type": "array", "minItems": 6, "maxItems": 6, "items": {"type": "string", "maxLength": o.max}}}});
    #[derive(Deserialize)]
    struct R {
        texts: Vec<String>,
    }
    let res = crate::import::ask_json::<R>(ai, user, &schema, 520);
    if matches!(res, Err(AiError::NotReady)) {
        return Err(AiError::NotReady);
    }
    let conn = db.lock().unwrap();
    bump(&conn, course, o.id);
    let mut have: HashSet<String> =
        texts::<String>(&conn, "SELECT text FROM cheers WHERE course_id = ?1 AND occasion = ?2", params![course, o.id]).iter().map(|t| normalize(t)).collect();
    let mut added = 0;
    for t in res?.texts.iter().filter_map(|t| fit(o, t)) {
        if have.len() < KEEP as usize && have.insert(normalize(&t)) {
            conn.execute("INSERT INTO cheers(course_id, occasion, text) VALUES(?1, ?2, ?3)", params![course, o.id, t]).map_err(|e| AiError::Failed(e.to_string()))?;
            added += 1;
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE courses(id INTEGER PRIMARY KEY, name TEXT NOT NULL, motivation TEXT);
             CREATE TABLE cheers(id INTEGER PRIMARY KEY, course_id INTEGER NOT NULL, occasion TEXT NOT NULL, text TEXT NOT NULL);
             INSERT INTO courses(id, name) VALUES(1, 'Mathe 2');",
        )
        .unwrap();
        c
    }

    #[test]
    fn checks_ai_texts() {
        let lvl = occasion("level_up");
        assert_eq!(fit(lvl, " „Willkommen auf Level {level}, {title}!“ ").as_deref(), Some("Willkommen auf Level {level}, {title}!"));
        assert!(fit(lvl, "Level {level} in {course}, stark!").is_some()); // Kursname über den Platzhalter
        assert!(fit(lvl, "Schon 12 Lektionen, Level {level}!").is_none()); // erfundene Zahl
        assert!(fit(lvl, "Hallo {name}, weiter so!").is_none()); // fremder Platzhalter
        assert!(fit(lvl, "Level {level erreicht!").is_none()); // kaputte Klammer
        assert!(fit(lvl, "Du musst jetzt dranbleiben!").is_none()); // Druck
        assert!(fit(lvl, "Super").is_none()); // zu kurz
        assert!(fit(occasion("boost"), &"Richtig! ".repeat(8)).is_none()); // zu lang für den Anlass
        assert!(fit(occasion("done"), "Nicht schlecht, weiter geht's!").is_some());
        for o in &OCCASIONS {
            assert!(fit(o, o.fallback).is_some(), "{}", o.id); // die festen Texte halten sich an dieselben Regeln
        }
        assert_eq!(fit_name("„{course}-Gott“").as_deref(), Some("{course}-Gott"));
        assert!(fit_name("Zahlenzauberer").is_some());
        assert!(fit_name("{course} Level 10").is_none());
        assert!(fit_name("{title}-Profi").is_none());
        assert!(fit_name("Der {course}-Meister aller Klassen und Zeiten, ehrlich").is_none());
        assert!(fit_name("Versager-Stufe").is_none());
    }

    #[test]
    fn names_fill_and_fall_back() {
        let c = conn();
        // Ohne KI: Kursname + Stufenwort, feste Erfolgsnamen
        let n = names(&c, Some(1));
        assert_eq!((n.title(1), n.title(95)), ("Mathe 2 · Erste Schritte".to_string(), "Mathe 2 · Legendär".to_string()));
        assert_eq!(n.next_title(1), Some((11, "Mathe 2 · Neugierig".to_string())));
        assert_eq!(n.next_title(95), None);
        // KI-Namen: {course} eingesetzt (auch mit Ziffer im Kursnamen), leere Stelle = Ersatz
        let mut titles = vec!["{course}-Starter".to_string(), String::new()];
        titles.resize(10, "{course}-Gott".into());
        let json = serde_json::to_string(&Stored { titles, badges: HashMap::from([("first".to_string(), "Erste {course}-Schritte".to_string())]) }).unwrap();
        c.execute("UPDATE courses SET motivation = ?1 WHERE id = 1", [json]).unwrap();
        let n = names(&c, Some(1));
        assert_eq!((n.title(5), n.title(15), n.title(100)), ("Mathe 2-Starter".to_string(), "Mathe 2 · Neugierig".to_string(), "Mathe 2-Gott".to_string()));
        let first = BADGES.iter().find(|b| b.id == "first").unwrap();
        let week = BADGES.iter().find(|b| b.id == "week").unwrap();
        let ten = BADGES.iter().find(|b| b.id == "ten").unwrap();
        assert_eq!((n.out(first, None).title, n.out(ten, None).title, n.out(week, Some(1)).title), ("Erste Mathe 2-Schritte".to_string(), "Dranbleiben".to_string(), "Eine Woche".to_string()));
        assert_eq!(names(&c, None).title(1), "Erste Schritte");
    }

    #[test]
    fn picks_and_fills() {
        let c = conn();
        assert_eq!(pick(&c, Some(1), "record", &[("streak", "9".into())]), "Neuer Rekord: 9 Tage in Folge!");
        c.execute("INSERT INTO cheers(course_id, occasion, text) VALUES(1, 'record', '{streak} Tage {course} am Stück!')", []).unwrap();
        assert_eq!(pick(&c, Some(1), "record", &[("streak", "9".into())]), "9 Tage Mathe 2 am Stück!");
        assert_eq!(pick(&c, None, "record", &[("streak", "9".into())]), "Neuer Rekord: 9 Tage in Folge!"); // ohne Kurs: fester Text
        assert_eq!(picks(&c, Some(1), "boost", 3), vec!["Siehst du, das sitzt!".to_string()]);
        let m = Moment { level_up: Some((5, "Mathe 2-Starter".into())), perfect: true, ..Default::default() };
        assert_eq!(cheer_for(&c, Some(1), &m), "Level 5 erreicht: Mathe 2-Starter. Stark!");
        assert_eq!(cheer_for(&c, Some(1), &Moment { acc: 30, ..Default::default() }), occasion("tough").fallback);
        assert_eq!(cheer_for(&c, Some(1), &Moment { acc: 90, ..Default::default() }), occasion("done").fallback);
    }

    #[test]
    fn badges_from_facts() {
        assert!(earned(&Facts::default()).is_empty());
        let f = Facts { lessons: 10, tasks: 120, best_streak: 7, level: 3, ..Default::default() };
        let ids: Vec<&str> = earned(&f).iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["first", "ten", "tasks100", "week"]);
        assert_eq!(BADGES.iter().filter(|b| b.global).count(), 3);
    }
}
