//! Motivation ohne Druck. Erfolge vergibt der Code. Motivationstexte schreibt die KI im Hintergrund,
//! der Code prüft jeden Text, entscheidet, wann welcher Anlass kommt, und setzt die echten Zahlen ein.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ai::{Ai, AiError};
use crate::db::{self, Db};
use crate::learn::normalize;

// ---------- Erfolge ----------

#[derive(Serialize, Clone, Copy, Debug)]
pub struct Badge {
    pub id: &'static str,
    pub title: &'static str,
    pub desc: &'static str,
    pub icon: &'static str,
}

const fn b(id: &'static str, title: &'static str, desc: &'static str, icon: &'static str) -> Badge {
    Badge { id, title, desc, icon }
}

pub const BADGES: [Badge; 14] = [
    b("first", "Los geht's", "Schließe deine erste Lektion ab.", "star-fill"),
    b("ten", "Dranbleiben", "Schließe 10 Lektionen ab.", "cards"),
    b("fifty", "Ausdauer", "Schließe 50 Lektionen ab.", "trophy"),
    b("tasks100", "100 Aufgaben", "Löse 100 Aufgaben in Lektionen.", "check-circle-fill"),
    b("tasks500", "500 Aufgaben", "Löse 500 Aufgaben in Lektionen.", "seal-check-fill"),
    b("perfect", "Fehlerfrei", "Schaffe eine Lektion mit mindestens 5 Aufgaben ohne Fehler.", "sparkle"),
    b("run10", "Im Fluss", "Beantworte 10 Aufgaben hintereinander richtig.", "fire-fill"),
    b("week", "Eine Woche", "Lerne 7 Tage in Folge.", "calendar-blank"),
    b("month", "Ein Monat", "Lerne 30 Tage in Folge.", "crown-fill"),
    b("master", "Gemeistert", "Bring eine Einheit auf die Krone (Lernstand 4 von 5).", "crown-fill"),
    b("script", "Skript durch", "Übe jede Einheit eines Skripts mindestens einmal.", "book-open-text"),
    b("exam", "Prüfungsbereit", "Meistere alle prüfungsrelevanten Einheiten eines Kurses.", "target"),
    b("level10", "Level 10", "Erreiche Level 10.", "trophy"),
    b("comeback", "Wieder da", "Lerne nach ein paar Tagen Pause weiter. Pausen gehören dazu.", "arrow-clockwise"),
];

/// Was der Code über den Lernstand weiß, um Erfolge zu vergeben (sammelt `api::finish_lesson`).
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
pub fn earned(f: &Facts) -> Vec<&'static str> {
    let ok = |id: &str| match id {
        "first" => f.lessons >= 1,
        "ten" => f.lessons >= 10,
        "fifty" => f.lessons >= 50,
        "tasks100" => f.tasks >= 100,
        "tasks500" => f.tasks >= 500,
        "perfect" => f.perfect,
        "run10" => f.best_run >= 10,
        "week" => f.best_streak >= 7,
        "month" => f.best_streak >= 30,
        "master" => f.mastered,
        "script" => f.script,
        "exam" => f.exam,
        "level10" => f.level >= 10,
        "comeback" => f.comeback,
        _ => false,
    };
    BADGES.iter().map(|b| b.id).filter(|id| ok(id)).collect()
}

pub fn badge(id: &str) -> Option<&'static Badge> {
    BADGES.iter().find(|b| b.id == id)
}

// ---------- Motivationstexte ----------

pub struct Occasion {
    pub id: &'static str,
    /// Anlass, so wie ihn die KI beschrieben bekommt
    about: &'static str,
    /// Erlaubte Platzhalter, der Code setzt die echten Werte ein
    vars: &'static [&'static str],
    /// Gilt, solange die KI für diesen Anlass noch nichts Brauchbares geschrieben hat
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

/// Texte je Anlass, höchstens so viele KI-Runden je Anlass (auch gescheiterte zählen, damit der Worker nie hängt).
const KEEP: i64 = 6;
const ROUNDS: i64 = 3;
/// Wörter, die Druck oder Schuld machen. Texte damit verwirft der Code.
const PRESSURE: [&str; 7] = ["musst", "müsstest", "enttäusch", "faul", "versag", "schäm", "peinlich"];

pub fn occasion(id: &str) -> &'static Occasion {
    OCCASIONS.iter().find(|o| o.id == id).unwrap_or(&OCCASIONS[8])
}

/// Prüft einen KI-Text: kurz, ohne Ziffern (Zahlen kommen nur über Platzhalter), nur erlaubte Platzhalter, kein Druck.
pub fn fit(o: &Occasion, text: &str) -> Option<String> {
    let t = text.trim().trim_matches(|c: char| c == '"' || c == '„' || c == '“').trim();
    let n = t.chars().count();
    if n < 8 || n > o.max || t.chars().any(|c| c.is_ascii_digit()) || t.matches('{').count() != t.matches('}').count() {
        return None;
    }
    let low = t.to_lowercase();
    if PRESSURE.iter().any(|w| low.contains(w)) {
        return None;
    }
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        let j = rest[i..].find('}')?;
        if !o.vars.contains(&&rest[i + 1..i + j]) {
            return None;
        }
        rest = &rest[i + j + 1..];
    }
    Some(t.to_string())
}

fn texts(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Vec<String> {
    let Ok(mut st) = conn.prepare(sql) else { return vec![] };
    let rows = st.query_map(p, |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect());
    rows.unwrap_or_default()
}

fn fill(text: String, vars: &[(&str, String)]) -> String {
    vars.iter().fold(text, |t, (k, v)| t.replace(&format!("{{{k}}}"), v))
}

/// Ein zufälliger geprüfter Text zum Anlass (sonst der feste) mit den echten Werten.
pub fn pick(conn: &Connection, id: &str, vars: &[(&str, String)]) -> String {
    let o = occasion(id);
    let t = conn
        .query_row("SELECT text FROM cheers WHERE occasion = ?1 ORDER BY RANDOM() LIMIT 1", [o.id], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
        .unwrap_or_else(|| o.fallback.to_string());
    fill(t, vars)
}

/// Bis zu `n` verschiedene Texte ohne Platzhalter (für die Lektion, z. B. Lob nach einer Erfolgsfrage).
pub fn picks(conn: &Connection, id: &str, n: usize) -> Vec<String> {
    let o = occasion(id);
    let mut out = texts(conn, "SELECT text FROM cheers WHERE occasion = ?1 ORDER BY RANDOM() LIMIT ?2", params![o.id, n as i64]);
    if out.is_empty() {
        out.push(o.fallback.to_string());
    }
    out
}

/// Was am Ende einer Lektion passiert ist. Der Code wählt daraus genau einen Anlass.
#[derive(Default)]
pub struct Moment {
    pub level_up: Option<(i64, &'static str)>,
    pub badge: Option<&'static str>,
    pub record: Option<i64>,
    pub comeback: bool,
    pub improved: Option<String>,
    pub perfect: bool,
    /// Trefferquote in Prozent
    pub acc: i64,
}

/// Feste Reihenfolge: Level > Erfolg > Rekord > Comeback > besser als letztes Mal > fehlerfrei > schwer (< 50 %) > geschafft.
pub fn cheer_for(conn: &Connection, m: &Moment) -> String {
    let (id, vars): (&str, Vec<(&str, String)>) = if let Some((lvl, title)) = m.level_up {
        ("level_up", vec![("level", lvl.to_string()), ("title", title.to_string())])
    } else if let Some(b) = m.badge {
        ("badge", vec![("badge", b.to_string())])
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
    pick(conn, id, &vars)
}

/// Nächster Anlass, für den die KI Texte schreiben soll (für den Hintergrund-Worker, ganz zuletzt).
pub fn wanted(conn: &Connection) -> Option<&'static str> {
    OCCASIONS.iter().map(|o| o.id).find(|id| {
        let have: i64 = conn.query_row("SELECT COUNT(*) FROM cheers WHERE occasion = ?1", [id], |r| r.get(0)).unwrap_or(KEEP);
        have < KEEP && rounds(conn, id) < ROUNDS
    })
}

fn rounds(conn: &Connection, id: &str) -> i64 {
    db::setting(conn, &format!("cheers_round:{id}")).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Die KI schreibt eine Runde Texte für einen Anlass, nur geprüfte kommen in die Datenbank.
pub fn write(db: &Db, ai: &Ai, id: &str) -> Result<usize, AiError> {
    let o = occasion(id);
    let vars = if o.vars.is_empty() {
        "Ohne Platzhalter.".to_string()
    } else {
        let list: Vec<String> = o.vars.iter().map(|v| format!("{{{v}}}")).collect();
        format!("Nutze diese Platzhalter genau so geschrieben, die App setzt später die echten Werte ein: {}.", list.join(", "))
    };
    let user = format!(
        "Schreibe 6 verschiedene Motivationstexte für eine Lern-App.\nAnlass: {}\n{vars}\n\n\
         Regeln:\n- Deutsch, du-Form, ein Satz, höchstens {} Zeichen\n- keine Zahlen und keine Ziffern\n\
         - warm und ehrlich, kein Druck, keine Schuld, keine Übertreibung\n- jeder Text fängt anders an",
        o.about, o.max
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
    db::set_setting(&conn, &format!("cheers_round:{id}"), &(rounds(&conn, id) + 1).to_string()).map_err(|e| AiError::Failed(e.to_string()))?;
    let mut have: HashSet<String> = texts(&conn, "SELECT text FROM cheers WHERE occasion = ?1", [o.id]).iter().map(|t| normalize(t)).collect();
    let mut added = 0;
    for t in res?.texts.iter().filter_map(|t| fit(o, t)) {
        if have.len() < KEEP as usize && have.insert(normalize(&t)) {
            conn.execute("INSERT INTO cheers(occasion, text) VALUES(?1, ?2)", params![o.id, t]).map_err(|e| AiError::Failed(e.to_string()))?;
            added += 1;
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_ai_texts() {
        let lvl = occasion("level_up");
        assert_eq!(fit(lvl, " „Willkommen auf Level {level}, {title}!“ ").as_deref(), Some("Willkommen auf Level {level}, {title}!"));
        assert!(fit(lvl, "Schon 12 Lektionen, Level {level}!").is_none()); // erfundene Zahl
        assert!(fit(lvl, "Hallo {name}, weiter so!").is_none()); // fremder Platzhalter
        assert!(fit(lvl, "Level {level erreicht!").is_none()); // kaputte Klammer
        assert!(fit(lvl, "Du musst jetzt dranbleiben!").is_none()); // Druck
        assert!(fit(lvl, "Super").is_none()); // zu kurz
        assert!(fit(occasion("boost"), &"Richtig! ".repeat(8)).is_none()); // zu lang für den Anlass
        assert!(fit(occasion("done"), "Nicht schlecht, weiter geht's!").is_some());
        // Die festen Texte halten sich an dieselben Regeln
        for o in &OCCASIONS {
            assert!(fit(o, o.fallback).is_some(), "{}", o.id);
        }
    }

    #[test]
    fn picks_and_fills() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE cheers(id INTEGER PRIMARY KEY, occasion TEXT NOT NULL, text TEXT NOT NULL);").unwrap();
        assert_eq!(pick(&conn, "record", &[("streak", "9".into())]), "Neuer Rekord: 9 Tage in Folge!");
        conn.execute("INSERT INTO cheers(occasion, text) VALUES('record', '{streak} Tage am Stück, dein Bestwert!')", []).unwrap();
        assert_eq!(pick(&conn, "record", &[("streak", "9".into())]), "9 Tage am Stück, dein Bestwert!");
        assert_eq!(picks(&conn, "boost", 3), vec!["Siehst du, das sitzt!".to_string()]);
        // Reihenfolge der Anlässe: Level schlägt alles, schwere Lektion nur ohne anderen Anlass
        let m = Moment { level_up: Some((5, "Neugierig")), perfect: true, ..Default::default() };
        assert_eq!(cheer_for(&conn, &m), "Level 5 erreicht: Neugierig. Stark!");
        assert_eq!(cheer_for(&conn, &Moment { acc: 30, ..Default::default() }), occasion("tough").fallback);
        assert_eq!(cheer_for(&conn, &Moment { acc: 90, ..Default::default() }), occasion("done").fallback);
    }

    #[test]
    fn badges_from_facts() {
        assert!(earned(&Facts::default()).is_empty());
        let f = Facts { lessons: 10, tasks: 120, best_streak: 7, level: 3, ..Default::default() };
        assert_eq!(earned(&f), vec!["first", "ten", "tasks100", "week"]);
        assert!(BADGES.iter().all(|b| badge(b.id).is_some()));
    }
}
