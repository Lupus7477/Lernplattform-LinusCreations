//! Import: PDF-Seiten → Abschnitte → Einheiten (Titel, Zusammenfassung, Begriffe) → Übungsfragen.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rusqlite::params;
use serde::Deserialize;

use crate::ai::{Ai, AiError, Gen, Msg};
use crate::db::{self, Db, Task};
use crate::learn::normalize;

/// ~5000 Zeichen ≈ 1600 Tokens: passt mit Prompt und Antwort sicher in 4096 Kontext.
pub const SECTION_CHARS: usize = 5000;

#[derive(Debug, PartialEq)]
pub struct Section {
    pub from: usize,
    pub to: usize,
    pub text: String,
}

fn clean(page: &str) -> String {
    page.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Teilt Seiten (1-basiert) an Seitengrenzen in Abschnitte ≤ `max` Zeichen; zu lange Seiten werden an Wortgrenzen geteilt.
pub fn sections(pages: &[String], max: usize) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut cur: Option<Section> = None;
    for (i, raw) in pages.iter().enumerate() {
        let page = i + 1;
        let text = clean(raw);
        if text.is_empty() {
            continue;
        }
        for piece in split_long(&text, max) {
            match cur.as_mut() {
                Some(s) if s.text.len() + piece.len() < max => {
                    s.text.push('\n');
                    s.text.push_str(&piece);
                    s.to = page;
                }
                _ => {
                    out.extend(cur.take());
                    cur = Some(Section { from: page, to: page, text: piece });
                }
            }
        }
    }
    out.extend(cur);
    // Winzigen Rest an den Vorgänger hängen statt eine Mini-Einheit zu bauen.
    if out.len() > 1 && out.last().map(|s| s.text.len() < 400).unwrap_or(false) {
        let last = out.pop().unwrap();
        let prev = out.last_mut().unwrap();
        prev.text.push('\n');
        prev.text.push_str(&last.text);
        prev.to = last.to;
    }
    out
}

fn split_long(text: &str, max: usize) -> Vec<String> {
    let mut parts = vec![String::new()];
    for word in text.split(' ') {
        let p = parts.last_mut().unwrap();
        if !p.is_empty() && p.len() + word.len() + 1 > max {
            parts.push(word.to_string());
        } else {
            if !p.is_empty() {
                p.push(' ');
            }
            p.push_str(word);
        }
    }
    parts
}

// ---------- KI-Ausgabe-Formate ----------

#[derive(Deserialize)]
pub struct UnitInfo {
    pub title: String,
    pub summary: String,
    pub terms: Vec<Term>,
}

#[derive(Deserialize)]
pub struct Term {
    pub term: String,
    pub definition: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Question {
    pub kind: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub answer: String,
    pub why: String,
}

#[derive(Deserialize)]
struct Questions {
    questions: Vec<Question>,
}

const UNIT_SCHEMA: &str = r#"{"type":"object","properties":{
 "title":{"type":"string"},
 "summary":{"type":"string"},
 "terms":{"type":"array","minItems":2,"maxItems":6,"items":{"type":"object","properties":{
   "term":{"type":"string"},"definition":{"type":"string"}},"required":["term","definition"]}}},
 "required":["title","summary","terms"]}"#;

const QUESTION_SCHEMA: &str = r#"{"type":"object","properties":{
 "questions":{"type":"array","minItems":5,"maxItems":8,"items":{"type":"object","properties":{
   "kind":{"enum":["mc","true_false","fill_blank"]},
   "prompt":{"type":"string"},
   "options":{"type":"array","minItems":2,"maxItems":4,"items":{"type":"string"}},
   "answer":{"type":"string"},
   "why":{"type":"string"}},
  "required":["kind","prompt","options","answer","why"]}}},
 "required":["questions"]}"#;

fn grammar(cell: &'static OnceLock<String>, schema: &str) -> &'static str {
    cell.get_or_init(|| llama_cpp_2::json_schema_to_grammar(schema).expect("gültiges JSON-Schema"))
}

const SYSTEM: &str = "Du bist ein geduldiger Tutor. Du arbeitest ausschließlich mit dem gegebenen Lernmaterial und antwortest auf Deutsch.";

pub fn analyze(ai: &Ai, text: &str) -> Result<UnitInfo, AiError> {
    static G: OnceLock<String> = OnceLock::new();
    let user = format!(
        "Lernmaterial:\n<<<\n{text}\n>>>\n\n\
         Gib zurück:\n\
         - title: kurzer Thementitel (höchstens 6 Wörter)\n\
         - summary: 2–3 einfache Sätze, worum es in diesem Material geht\n\
         - terms: die 3–6 wichtigsten Fachbegriffe aus dem Material, je mit einer kurzen Definition in einem Satz"
    );
    let raw = ai.generate(
        Gen { messages: vec![Msg { role: "system", content: SYSTEM.into() }, Msg { role: "user", content: user }], grammar: Some(grammar(&G, UNIT_SCHEMA)), max_tokens: 600, temperature: 0.3 },
        |_| {},
    )?;
    serde_json::from_str(&raw).map_err(|e| AiError::Failed(format!("Antwort unlesbar: {e}")))
}

pub fn generate_questions(ai: &Ai, text: &str, level: i64) -> Result<Vec<Question>, AiError> {
    static G: OnceLock<String> = OnceLock::new();
    let focus = match level {
        1 => "Grundbegriffe und Definitionen",
        2 => "Zusammenhänge und Abläufe",
        _ => "Anwendung auf neue Beispiele und typische Denkfehler",
    };
    let user = format!(
        "Lernmaterial:\n<<<\n{text}\n>>>\n\n\
         Erstelle 8 Übungsfragen zu diesem Material. Schwerpunkt: {focus}.\n\
         Verteilung: 4 Fragen vom Typ mc, 2 vom Typ true_false, 2 vom Typ fill_blank.\n\
         - mc: Frage mit genau 4 Antwortoptionen, genau eine ist richtig. answer = exakter Text der richtigen Option. Falsche Optionen klingen plausibel.\n\
         - true_false: eine Aussage, die stimmt oder nicht stimmt. options = [\"Richtig\", \"Falsch\"], answer = \"Richtig\" oder \"Falsch\".\n\
         - fill_blank: ein Satz mit genau einer Lücke ___ für einen Fachbegriff. answer = das fehlende Wort, options = answer plus 3 ähnliche falsche Wörter.\n\
         Regeln: Jede Frage prüft genau einen Punkt. why = ein kurzer Satz, warum die Antwort stimmt. \
         Keine Verweise auf Seiten, Kapitel, Folien oder das Skript."
    );
    let raw = ai.generate(
        Gen { messages: vec![Msg { role: "system", content: SYSTEM.into() }, Msg { role: "user", content: user }], grammar: Some(grammar(&G, QUESTION_SCHEMA)), max_tokens: 1700, temperature: 0.5 },
        |_| {},
    )?;
    let parsed: Questions = serde_json::from_str(&raw).map_err(|e| AiError::Failed(format!("Antwort unlesbar: {e}")))?;
    Ok(validate(parsed.questions))
}

// ---------- Qualitätsfilter im Code statt langer Prompt-Regeln ----------

const BANNED_WORDS: [&str; 6] = ["seite", "seiten", "kapitel", "skript", "folie", "folien"];
const ASKS: [&str; 10] = ["was", "welche", "welcher", "wie", "warum", "wer", "wo", "wann", "nenne", "erkläre"];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

pub fn validate(qs: Vec<Question>) -> Vec<Question> {
    let mut seen = std::collections::HashSet::new();
    qs.into_iter()
        .filter_map(fix)
        .filter(|q| seen.insert(normalize(&q.prompt)))
        .collect()
}

fn fix(mut q: Question) -> Option<Question> {
    q.prompt = q.prompt.trim().to_string();
    q.answer = q.answer.trim().to_string();
    q.why = q.why.trim().to_string();
    q.options = q.options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
    let all = format!("{} {} {}", q.prompt, q.answer, q.options.join(" "));
    let w = words(&all);
    if q.prompt.chars().count() < 8 || w.iter().any(|x| BANNED_WORDS.contains(&x.as_str())) {
        return None;
    }
    // „Was ist X und welche Arten gibt es?“ = zwei Fragen in einer
    if words(&q.prompt).iter().filter(|x| ASKS.contains(&x.as_str())).count() >= 2 {
        return None;
    }
    match q.kind.as_str() {
        "true_false" => {
            q.answer = match normalize(&q.answer).as_str() {
                "richtig" | "wahr" | "true" | "stimmt" => "Richtig".into(),
                "falsch" | "false" | "stimmt nicht" => "Falsch".into(),
                _ => return None,
            };
            q.options = vec!["Richtig".into(), "Falsch".into()];
        }
        "mc" | "fill_blank" => {
            if q.kind == "fill_blank" {
                q.prompt = collapse_blanks(&q.prompt)?;
                if !q.options.iter().any(|o| normalize(o) == normalize(&q.answer)) {
                    q.options.insert(0, q.answer.clone());
                }
            }
            let mut uniq = std::collections::HashSet::new();
            q.options.retain(|o| uniq.insert(normalize(o)));
            // Antwort exakt wie die passende Option schreiben
            q.answer = q.options.iter().find(|o| normalize(o) == normalize(&q.answer))?.clone();
            let need = if q.kind == "mc" { 4 } else { 2 };
            if q.options.len() < need {
                return None;
            }
            q.options.truncate(4);
            if !q.options.contains(&q.answer) {
                q.options[3] = q.answer.clone();
            }
        }
        _ => return None,
    }
    Some(q)
}

/// Lücke vereinheitlichen: genau eine Folge aus ≥2 Unterstrichen → „___“, sonst ungültig.
fn collapse_blanks(p: &str) -> Option<String> {
    let parts: Vec<&str> = p.split("__").collect();
    if parts.len() < 2 {
        return None;
    }
    let before = parts[0];
    let after = parts[1..].join("__");
    let after = after.trim_start_matches('_');
    (!after.contains("__")).then(|| format!("{before}___{after}"))
}

// ---------- Hintergrund-Worker ----------

/// Arbeitet Analysen und Fragen nacheinander ab. `notify(course_id)` nach jeder fertigen Aufgabe.
pub fn spawn_worker(db: Db, ai: Arc<Ai>, wake: Receiver<()>, notify: impl Fn(i64) + Send + 'static) {
    std::thread::spawn(move || loop {
        let task = db::next_task(&db.lock().unwrap()).ok().flatten();
        let Some(task) = task else {
            let _ = wake.recv_timeout(Duration::from_secs(60));
            continue;
        };
        let started = std::time::Instant::now();
        let what = match &task {
            Task::Analyze { unit_id, .. } => format!("Analyse Einheit {unit_id}"),
            Task::Questions { unit_id, level, .. } => format!("Fragen Einheit {unit_id} (Stufe {level})"),
        };
        match run_task(&db, &ai, task) {
            Ok(course_id) => {
                eprintln!("[import] {what}: {:.1} s", started.elapsed().as_secs_f32());
                notify(course_id);
            }
            Err(AiError::NotReady) => {
                let _ = wake.recv_timeout(Duration::from_secs(5));
            }
            // auch bei Fehlern melden, damit wartende Ansichten den Zustand neu lesen
            Err(AiError::Failed(e)) => {
                eprintln!("[import] {e}");
                notify(0);
            }
        }
    });
}

fn run_task(db: &Db, ai: &Ai, task: Task) -> Result<i64, AiError> {
    match task {
        Task::Analyze { unit_id, course_id, idx, page_from, page_to, text } => {
            let info = match analyze(ai, &text) {
                Ok(i) => i,
                Err(AiError::NotReady) => return Err(AiError::NotReady),
                // Analyse kaputt → Einheit trotzdem nutzbar machen (Fragen kommen aus dem Text)
                Err(AiError::Failed(e)) => {
                    eprintln!("[import] Analyse Einheit {unit_id}: {e}");
                    UnitInfo { title: format!("Seiten {page_from}-{page_to}"), summary: String::new(), terms: vec![] }
                }
            };
            let conn = db.lock().unwrap();
            let base = info.title.trim().trim_end_matches('.').to_string();
            let earlier: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM units WHERE course_id = ?1 AND idx < ?2 AND (title = ?3 OR title LIKE ?3 || ' (Teil %')",
                    params![course_id, idx, base],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let title = if earlier > 0 { format!("{base} (Teil {})", earlier + 1) } else { base };
            let _ = conn.execute(
                "UPDATE units SET title = ?1, summary = ?2, status = 'ready' WHERE id = ?3",
                params![title, info.summary.trim(), unit_id],
            );
            for t in info.terms.iter().filter(|t| !t.term.trim().is_empty()) {
                let _ = conn.execute(
                    "INSERT INTO terms(unit_id, term, definition) VALUES(?1, ?2, ?3)",
                    params![unit_id, t.term.trim(), t.definition.trim()],
                );
            }
            // Die ersten 2 Einheiten eines Kurses bekommen sofort Fragen.
            let _ = conn.execute(
                "UPDATE units SET q_state = 1 WHERE id = ?1 AND (SELECT COUNT(*) FROM units WHERE course_id = ?2 AND q_state > 0) < 2",
                params![unit_id, course_id],
            );
            Ok(course_id)
        }
        Task::Questions { unit_id, level, text } => {
            let result = generate_questions(ai, &text, level);
            let conn = db.lock().unwrap();
            let course_id: i64 = conn.query_row("SELECT course_id FROM units WHERE id = ?1", [unit_id], |r| r.get(0)).unwrap_or(0);
            match result {
                Ok(qs) if qs.len() >= 3 => {
                    for q in &qs {
                        let _ = conn.execute(
                            "INSERT INTO questions(unit_id, kind, prompt, options, answer, why, level) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![unit_id, q.kind, q.prompt, serde_json::to_string(&q.options).unwrap(), q.answer, q.why, level],
                        );
                    }
                    let _ = conn.execute("UPDATE units SET q_state = 2, q_level = ?1 WHERE id = ?2", params![level, unit_id]);
                    Ok(course_id)
                }
                Err(AiError::NotReady) => Err(AiError::NotReady),
                other => {
                    let _ = conn.execute("UPDATE units SET q_state = 3 WHERE id = ?1", [unit_id]);
                    drop(conn);
                    match other {
                        Err(e) => Err(e),
                        Ok(qs) => Err(AiError::Failed(format!("nur {} brauchbare Fragen für Einheit {unit_id}", qs.len()))),
                    }
                }
            }
        }
    }
}

/// Speichert ein neues Dokument und legt pro Abschnitt eine wartende Einheit an.
pub fn store_document(db: &Db, course_id: i64, name: &str, pages: &[String]) -> rusqlite::Result<(i64, usize)> {
    let secs = sections(pages, SECTION_CHARS);
    let mut conn = db.lock().unwrap();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO documents(course_id, name, pages, created_at) VALUES(?1, ?2, ?3, ?4)",
        params![course_id, name, pages.len() as i64, db::now()],
    )?;
    let doc_id = tx.last_insert_rowid();
    let mut idx: i64 = tx.query_row("SELECT COALESCE(MAX(idx) + 1, 0) FROM units WHERE course_id = ?1", [course_id], |r| r.get(0))?;
    for s in &secs {
        tx.execute(
            "INSERT INTO units(course_id, document_id, idx, page_from, page_to, title, source_text) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![course_id, doc_id, idx, s.from as i64, s.to as i64, format!("Seiten {}-{}", s.from, s.to), s.text],
        )?;
        idx += 1;
    }
    tx.commit()?;
    Ok((doc_id, secs.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(kind: &str, prompt: &str, options: &[&str], answer: &str) -> Question {
        Question { kind: kind.into(), prompt: prompt.into(), options: options.iter().map(|s| s.to_string()).collect(), answer: answer.into(), why: "weil".into() }
    }

    #[test]
    fn splits_into_sections() {
        let pages: Vec<String> = vec!["a ".repeat(1000), "  ".into(), "b ".repeat(1000), "c ".repeat(1000), "kurz".into()];
        let s = sections(&pages, 5000);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].from, s[0].to), (1, 3)); // leere Seite 2 übersprungen
        assert_eq!((s[1].from, s[1].to), (4, 5));
    }

    #[test]
    fn sections_respect_limit_and_pages() {
        let pages: Vec<String> = (0..6).map(|i| format!("Seite{i} ").repeat(300)).collect(); // je ~2100 Zeichen
        let s = sections(&pages, 5000);
        assert!(s.iter().all(|x| x.text.len() <= 5000 + 400));
        assert_eq!((s[0].from, s[0].to), (1, 2));
        assert_eq!(s.last().unwrap().to, 6);
        let long = vec!["wort ".repeat(3000)]; // 15000 Zeichen auf einer Seite
        let s = sections(&long, 5000);
        assert_eq!(s.len(), 3);
        assert!(s.iter().all(|x| x.from == 1 && x.text.len() <= 5000));
    }

    /// Echter Lauf mit dem heruntergeladenen Modell (dauert, braucht die GGUF-Datei):
    /// `cargo test real_model -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_model() {
        let dir = std::env::var("LERNPFAD_MODELS").map(std::path::PathBuf::from).unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/de.lernpfad.app/models")
        });
        let tier = std::env::var("LERNPFAD_TIER").unwrap_or_else(|_| "ausgewogen".into());
        let ai = Ai::new(dir, &tier, Box::new(|_| {}));
        ai.ensure_model();
        let text = include_str!("../testdata/marktformen.txt");
        let secs = sections(&[text.to_string()], SECTION_CHARS);
        println!("{} Abschnitt(e), Modell {tier}", secs.len());

        let t = std::time::Instant::now();
        let info = analyze(&ai, &secs[0].text).expect("Analyse");
        println!("\nAnalyse in {:.1} s\n  Titel: {}\n  Zusammenfassung: {}", t.elapsed().as_secs_f32(), info.title, info.summary);
        for term in &info.terms {
            println!("  - {}: {}", term.term, term.definition);
        }

        let t = std::time::Instant::now();
        let qs = generate_questions(&ai, &secs[0].text, 1).expect("Fragen");
        println!("\nFragen in {:.1} s ({} gültig)", t.elapsed().as_secs_f32(), qs.len());
        for q in &qs {
            println!("  [{}] {}\n      {:?} -> {}\n      warum: {}", q.kind, q.prompt, q.options, q.answer, q.why);
        }
        assert!(!info.title.is_empty() && info.terms.len() >= 2);
        assert!(qs.len() >= 3, "zu wenige brauchbare Fragen");
    }

    #[test]
    fn filters_bad_questions() {
        let qs = vec![
            q("mc", "Was beschreibt die Nachfrage?", &["A", "B", "C", "d"], "D"),          // ok, Antwort wird zu "d"
            q("mc", "Was steht auf Seite 5?", &["A", "B", "C", "D"], "A"),                  // Seitenverweis
            q("mc", "Was ist X und welche Arten gibt es?", &["A", "B", "C", "D"], "A"),     // Doppelfrage
            q("mc", "Welche Option fehlt hier?", &["A", "B", "C", "D"], "E"),               // Antwort nicht in Optionen
            q("true_false", "Die Erde ist rund.", &["x"], "wahr"),                          // ok → Richtig
            q("fill_blank", "Ein ____ speichert Daten.", &["Puffer", "Register"], "Speicher"), // ok, Antwort ergänzt
            q("fill_blank", "Ohne Lücke im Satz.", &["A", "B"], "A"),                        // keine Lücke
            q("mc", "Was beschreibt die Nachfrage?", &["A", "B", "C", "D"], "A"),           // Duplikat
        ];
        let out = validate(qs);
        assert_eq!(out.len(), 3, "{out:#?}");
        assert_eq!(out[0].answer, "d");
        assert_eq!(out[1].answer, "Richtig");
        assert_eq!(out[1].options, vec!["Richtig", "Falsch"]);
        assert_eq!(out[2].prompt, "Ein ___ speichert Daten.");
        assert!(out[2].options.contains(&"Speicher".to_string()));
    }
}
