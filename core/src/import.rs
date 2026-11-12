//! Import: PDF-Seiten → Abschnitte → Einheiten (Titel, Zusammenfassung, Begriffe) → Übungsfragen.
//! Dazu Strukturaufgaben (Matrix, Reihenfolge, Lücken) mit Selbstprüfung der KI und Grafiken zum Beschriften.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ai::{Ai, AiError, Gen, Msg};
use crate::db::{self, Db, Task};
use crate::learn::{self, normalize, Group, Spot};

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

/// Eine Aufgabe in Speicherform. `groups` nur bei `sort`, `spots` nur bei `label`.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Question {
    pub kind: String,
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub spots: Vec<Spot>,
}

#[derive(Deserialize)]
struct Questions {
    questions: Vec<Question>,
}

#[derive(Deserialize)]
struct Tasks {
    tasks: Vec<Question>,
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

/// Strukturaufgaben in derselben Form wie die Speicherung (siehe `fix`).
const TASK_SCHEMA: &str = r#"{"type":"object","properties":{
 "tasks":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"object","properties":{
   "kind":{"enum":["sort","order","cloze"]},
   "prompt":{"type":"string"},
   "groups":{"type":"array","maxItems":4,"items":{"type":"object","properties":{
     "name":{"type":"string"},"items":{"type":"array","minItems":1,"maxItems":3,"items":{"type":"string"}}},"required":["name","items"]}},
   "options":{"type":"array","maxItems":6,"items":{"type":"string"}},
   "why":{"type":"string"}},
  "required":["kind","prompt","groups","options","why"]}}},
 "required":["tasks"]}"#;

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
    let focus = focus(level);
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

fn focus(level: i64) -> &'static str {
    match level {
        1 => "Grundbegriffe und Definitionen",
        2 => "Zusammenhänge und Abläufe",
        _ => "Anwendung auf neue Beispiele und typische Denkfehler",
    }
}

/// 1–2 Aufgaben, deren Art die KI nach dem Inhalt wählt. Eine Matrix nur, wenn ihre Felder im Material stehen.
pub fn generate_tasks(ai: &Ai, text: &str, level: i64) -> Result<Vec<Question>, AiError> {
    static G: OnceLock<String> = OnceLock::new();
    let scenario = if level >= 2 {
        "\nBei sort: Wende die Kategorien auf ein neues, konkretes Beispiel an (etwa ein Café, ein Fahrradladen oder eine App). \
         prompt beschreibt das Beispiel in 1 bis 2 Sätzen und sagt, was zu tun ist. items sind Aussagen über dieses Beispiel, \
         options = 1 bis 2 Aussagen über das Beispiel, die in keine Kategorie gehören."
    } else {
        ""
    };
    let user = format!(
        "Lernmaterial:\n<<<\n{text}\n>>>\n\n\
         Erstelle 2 Übungsaufgaben verschiedener Art, die zur Struktur dieses Materials passen. Schwerpunkt: {}.\n\
         Teilt das Material Dinge in feste Kategorien ein, ist eine davon sort. Beschreibt es einen Ablauf, ist eine davon order.\n\
         - sort: NUR wenn das Material selbst Dinge in feste Kategorien einteilt (z. B. SWOT-Analyse: Stärken, Schwächen, Chancen, Risiken). \
         groups = 2 bis 4 Kategorien, name genau wie im Material, items = je 1 bis 3 kurze Aussagen (höchstens 8 Wörter). \
         options = 0 bis 2 Aussagen, die in keine Kategorie gehören. prompt = kurze Arbeitsanweisung.\n\
         - order: NUR wenn das Material einen Ablauf, Phasen oder Schritte beschreibt. options = 3 bis 6 Schritte in der richtigen Reihenfolge, \
         je höchstens 8 Wörter, ohne Nummern. groups = [].\n\
         - cloze: sonst. prompt = 1 bis 3 Sätze zum Inhalt, in denen 2 bis 4 Fachbegriffe in eckigen Klammern stehen, \
         z. B. „Die [Nachfrage] sinkt, wenn der [Preis] steigt.“ options = 2 bis 3 ähnliche, aber falsche Begriffe. groups = [].{scenario}\n\
         why = ein kurzer Satz, worauf es ankommt. Keine Verweise auf Seiten, Kapitel, Folien oder das Skript.",
        focus(level)
    );
    let raw = ai.generate(
        Gen { messages: vec![Msg { role: "system", content: SYSTEM.into() }, Msg { role: "user", content: user }], grammar: Some(grammar(&G, TASK_SCHEMA)), max_tokens: 900, temperature: 0.4 },
        |_| {},
    )?;
    let parsed: Tasks = serde_json::from_str(&raw).map_err(|e| AiError::Failed(format!("Antwort unlesbar: {e}")))?;
    Ok(parsed.tasks.into_iter().filter_map(|q| fix(q, true)).filter(|q| q.kind != "sort" || grounded(q, text)).collect())
}

/// Feldnamen müssen im Material stehen, sonst hat die KI ein Raster erfunden.
// ponytail: Wortstamm = erstes Wort ohne die letzten 2 Zeichen („Stärken“ findet „Stärke“). Echte Lemmatisierung, falls zu oft daneben.
fn grounded(q: &Question, text: &str) -> bool {
    let hay = normalize(text);
    q.groups.iter().all(|g| {
        let first = normalize(&g.name).split(' ').next().unwrap_or("").to_string();
        let stem: String = first.chars().take(first.chars().count().saturating_sub(2).max(4)).collect();
        !stem.is_empty() && hay.contains(&stem)
    })
}

/// JSON-Antwort mit einer Grammatik, die erst zur Laufzeit feststeht (Enums aus der Aufgabe).
fn ask_json<T: DeserializeOwned>(ai: &Ai, user: String, schema: &Value, max_tokens: usize) -> Result<T, AiError> {
    let g = llama_cpp_2::json_schema_to_grammar(&schema.to_string()).map_err(|e| AiError::Failed(format!("Grammatik: {e}")))?;
    let raw = ai.generate(
        Gen { messages: vec![Msg { role: "system", content: SYSTEM.into() }, Msg { role: "user", content: user }], grammar: Some(&g), max_tokens, temperature: 0.1 },
        |_| {},
    )?;
    serde_json::from_str(&raw).map_err(|e| AiError::Failed(format!("Antwort unlesbar: {e}")))
}

/// Selbstprüfung: Die KI löst ihre Aufgabe ein zweites Mal, ohne die Lösung zu sehen.
/// Matrix: nur übereinstimmende Aussagen bleiben. Reihenfolge und Lücken: jede Abweichung verwirft die Aufgabe.
pub fn verify_task(ai: &Ai, text: &str, q: Question) -> Result<Option<Question>, AiError> {
    let mut rng = learn::Shuffler::new();
    let material = format!("Lernmaterial:\n<<<\n{text}\n>>>\n\nAufgabe: {}\n", q.prompt);
    let list = |v: &[String]| v.iter().map(|x| format!("- {x}")).collect::<Vec<_>>().join("\n");
    match q.kind.as_str() {
        "sort" => {
            let mut items: Vec<String> = q.groups.iter().flat_map(|g| g.items.clone()).chain(q.options.clone()).collect();
            rng.shuffle(&mut items);
            let mut names: Vec<String> = q.groups.iter().map(|g| g.name.clone()).collect();
            let fields = names.join(", ");
            names.push("keins".into());
            let schema = json!({"type": "object", "required": ["assign"], "properties": {"assign": {"type": "array", "minItems": items.len(), "maxItems": items.len(),
                "items": {"type": "object", "required": ["item", "group"], "properties": {"item": {"enum": items}, "group": {"enum": names}}}}}});
            #[derive(Deserialize)]
            struct A {
                item: String,
                group: String,
            }
            #[derive(Deserialize)]
            struct R {
                assign: Vec<A>,
            }
            let user = format!("{material}Kategorien: {fields}\nOrdne jede Aussage genau einer Kategorie zu, oder „keins“, wenn sie in keine passt:\n{}", list(&items));
            let r: R = ask_json(ai, user, &schema, 60 + 40 * items.len())?;
            let said: HashMap<String, String> = r.assign.into_iter().map(|a| (a.item, a.group)).collect();
            let mut q = q;
            for g in &mut q.groups {
                let name = g.name.clone();
                g.items.retain(|i| said.get(i) == Some(&name));
            }
            q.options.retain(|o| said.get(o).is_some_and(|g| g == "keins"));
            Ok(fix(q, true))
        }
        "order" => {
            let mut steps = q.options.clone();
            while steps == q.options {
                rng.shuffle(&mut steps);
            }
            let schema = json!({"type": "object", "required": ["order"], "properties": {"order": {"type": "array", "minItems": steps.len(), "maxItems": steps.len(), "items": {"enum": steps}}}});
            #[derive(Deserialize)]
            struct R {
                order: Vec<String>,
            }
            let user = format!("{material}Bringe diese Schritte in die richtige Reihenfolge:\n{}", list(&steps));
            let r: R = ask_json(ai, user, &schema, 40 + 30 * steps.len())?;
            Ok((r.order == q.options).then_some(q))
        }
        "cloze" => {
            let (gapped, gaps) = learn::cloze_parts(&q.prompt);
            let mut bank: Vec<String> = gaps.iter().chain(&q.options).cloned().collect::<HashSet<_>>().into_iter().collect();
            rng.shuffle(&mut bank);
            let mut n = 0;
            let numbered = gapped.split("___").enumerate().map(|(i, part)| if i == 0 { part.to_string() } else { n += 1; format!("({n}) ___{part}") }).collect::<String>();
            let schema = json!({"type": "object", "required": ["gaps"], "properties": {"gaps": {"type": "array", "minItems": gaps.len(), "maxItems": gaps.len(), "items": {"enum": bank}}}});
            #[derive(Deserialize)]
            struct R {
                gaps: Vec<String>,
            }
            let user = format!("Lernmaterial:\n<<<\n{text}\n>>>\n\nText mit Lücken: {numbered}\nWörter: {}\nGib für jede Lücke der Reihe nach das passende Wort an.", bank.join(", "));
            let r: R = ask_json(ai, user, &schema, 40 + 20 * gaps.len())?;
            Ok((learn::seq_mistakes(&gaps, &r.gaps) == 0).then_some(q))
        }
        _ => Ok(Some(q)),
    }
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
        .filter_map(|q| fix(q, true))
        .filter(|q| seen.insert(normalize(&q.prompt)))
        .collect()
}

/// Bringt eine Frage in Form oder verwirft sie. `strict` = KI-Ausgabe; Nutzer-Bearbeitungen prüfen nur die Form.
/// Bei den Strukturaufgaben schreibt `fix` auch die lesbare `answer` („Stärken: a, b · Risiken: c“).
pub fn fix(mut q: Question, strict: bool) -> Option<Question> {
    let trim = |v: &[String]| v.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect::<Vec<_>>();
    q.prompt = q.prompt.trim().to_string();
    q.answer = q.answer.trim().to_string();
    q.why = q.why.trim().to_string();
    q.options = trim(&q.options);
    let groups: Vec<Group> = std::mem::take(&mut q.groups).into_iter().map(|g| Group { name: g.name.trim().to_string(), items: trim(&g.items) }).collect();
    let mut spots = std::mem::take(&mut q.spots);
    let all = format!("{} {} {} {}", q.prompt, q.answer, q.options.join(" "), groups.iter().map(|g| format!("{} {}", g.name, g.items.join(" "))).collect::<Vec<_>>().join(" "));
    let w = words(&all);
    if q.prompt.chars().count() < 8 || strict && w.iter().any(|x| BANNED_WORDS.contains(&x.as_str())) {
        return None;
    }
    // „Was ist X und welche Arten gibt es?“ = zwei Fragen in einer. Nur bei echten Fragen, Lückentext-Prosa enthält oft „wie“.
    let classic = matches!(q.kind.as_str(), "mc" | "true_false" | "fill_blank");
    if strict && classic && words(&q.prompt).iter().filter(|x| ASKS.contains(&x.as_str())).count() >= 2 {
        return None;
    }
    let mut seen = HashSet::new();
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
        // Felder mit Aussagen; options = Ablenker, die nirgends hingehören
        "sort" => {
            let mut names = HashSet::new();
            for g in groups {
                if g.name.is_empty() || !names.insert(normalize(&g.name)) {
                    continue;
                }
                let items: Vec<String> = g.items.into_iter().filter(|i| i.chars().count() <= 160 && seen.insert(normalize(i))).collect();
                if !items.is_empty() {
                    q.groups.push(Group { name: g.name, items });
                }
            }
            q.options.retain(|o| seen.insert(normalize(o)));
            q.options.truncate(4);
            let n: usize = q.groups.iter().map(|g| g.items.len()).sum();
            if !(2..=4).contains(&q.groups.len()) || !(3..=12).contains(&n) {
                return None;
            }
            q.answer = q.groups.iter().map(|g| format!("{}: {}", g.name, g.items.join(", "))).collect::<Vec<_>>().join(" · ");
        }
        // options = Schritte in der richtigen Reihenfolge; Nummern („1. …“) würden sie verraten
        "order" => {
            q.options = q.options.iter().map(|o| strip_number(o)).filter(|o| !o.is_empty()).collect();
            q.options.retain(|o| seen.insert(normalize(o)));
            if !(3..=7).contains(&q.options.len()) {
                return None;
            }
            q.answer = q.options.join(" → ");
        }
        // Lücken in [Klammern] im Text; options = Ablenker
        "cloze" => {
            // „__[Polypol]__“ → „[Polypol]“: die KI markiert Lücken gern doppelt
            while q.prompt.contains("_[") || q.prompt.contains("]_") {
                q.prompt = q.prompt.replace("_[", "[").replace("]_", "]");
            }
            let (text, gaps) = learn::cloze_parts(&q.prompt);
            if text.contains(['[', ']']) || !(1..=5).contains(&gaps.len()) || gaps.iter().any(|g| g.is_empty() || g.chars().count() > 60) {
                return None;
            }
            seen.extend(gaps.iter().map(|g| normalize(g)));
            q.options.retain(|o| seen.insert(normalize(o)));
            q.options.truncate(5);
            q.answer = gaps.join(" · ");
        }
        // Stellen im Bild (0–1); options = Ablenker
        "label" => {
            for s in &mut spots {
                s.label = s.label.trim().to_string();
            }
            let ok = |s: &Spot| {
                [s.x, s.y, s.w, s.h].iter().all(|v| v.is_finite() && *v >= 0.0)
                    && s.w > 0.005
                    && s.h > 0.005
                    && s.x + s.w <= 1.001
                    && s.y + s.h <= 1.001
                    && (1..=60).contains(&s.label.chars().count())
            };
            if !(1..=12).contains(&spots.len()) || !spots.iter().all(ok) {
                return None;
            }
            seen.extend(spots.iter().map(|s| normalize(&s.label)));
            q.options.retain(|o| seen.insert(normalize(o)));
            q.options.truncate(5);
            q.answer = spots.iter().enumerate().map(|(i, s)| format!("{} {}", i + 1, s.label)).collect::<Vec<_>>().join(" · ");
            q.spots = spots;
        }
        _ => return None,
    }
    Some(q)
}

/// „1. Ziel festlegen“, „Schritt 2: …“, „- …“ → nur der Schritt.
fn strip_number(s: &str) -> String {
    let t = s.trim().trim_start_matches(['-', '•', '*', ' ']);
    let t = t.strip_prefix("Schritt ").unwrap_or(t);
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    match t[digits..].strip_prefix(['.', ')', ':']) {
        Some(rest) if digits > 0 => rest.trim().to_string(),
        _ => s.trim().trim_start_matches(['-', '•', '*', ' ']).to_string(),
    }
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
        match run_task(&db, &ai, task, &notify) {
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

fn run_task(db: &Db, ai: &Ai, task: Task, notify: &dyn Fn(i64)) -> Result<i64, AiError> {
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
            let course_id = {
                let conn = db.lock().unwrap();
                let course_id: i64 = conn.query_row("SELECT course_id FROM units WHERE id = ?1", [unit_id], |r| r.get(0)).unwrap_or(0);
                match result {
                    Ok(qs) if qs.len() >= 3 => {
                        for q in &qs {
                            let _ = insert_question(&conn, unit_id, q, level, None);
                        }
                        let _ = conn.execute("UPDATE units SET q_state = 2, q_level = ?1 WHERE id = ?2", params![level, unit_id]);
                        course_id
                    }
                    Err(AiError::NotReady) => return Err(AiError::NotReady),
                    other => {
                        let _ = conn.execute("UPDATE units SET q_state = 3 WHERE id = ?1", [unit_id]);
                        return match other {
                            Err(e) => Err(e),
                            Ok(qs) => Err(AiError::Failed(format!("nur {} brauchbare Fragen für Einheit {unit_id}", qs.len()))),
                        };
                    }
                }
            };
            // Die Lektion kann schon starten; Strukturaufgaben kommen ohne Datenbank-Sperre hinterher.
            notify(course_id);
            let mut tasks = Vec::new();
            match generate_tasks(ai, &text, level) {
                Ok(ts) => {
                    for t in ts {
                        let prompt = t.prompt.clone();
                        match verify_task(ai, &text, t) {
                            Ok(Some(v)) => tasks.push(v),
                            Ok(None) => eprintln!("[import] Selbstprüfung verworfen: {prompt}"),
                            Err(e) => eprintln!("[import] Selbstprüfung Einheit {unit_id}: {e}"),
                        }
                    }
                }
                Err(e) => eprintln!("[import] Strukturaufgaben Einheit {unit_id}: {e}"),
            }
            let conn = db.lock().unwrap();
            for t in &tasks {
                let _ = insert_question(&conn, unit_id, t, level, None);
            }
            Ok(course_id)
        }
    }
}

pub fn insert_question(conn: &Connection, unit_id: i64, q: &Question, level: i64, figure_id: Option<i64>) -> rusqlite::Result<i64> {
    let (options, groups, spots) = (serde_json::to_string(&q.options).unwrap(), serde_json::to_string(&q.groups).unwrap(), serde_json::to_string(&q.spots).unwrap());
    conn.execute(
        "INSERT INTO questions(unit_id, kind, prompt, options, answer, why, level, groups, spots, figure_id) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![unit_id, q.kind, q.prompt, options, q.answer, q.why, level, groups, spots, figure_id],
    )?;
    Ok(conn.last_insert_rowid())
}

// ---------- Grafiken aus dem PDF ----------

/// Eine Grafik, wie sie die Oberfläche beim Einlesen findet: Ausschnitt mit übermalten Beschriftungen und deren Stellen.
#[derive(Deserialize)]
pub struct FigureIn {
    pub page: i64,
    #[serde(default)]
    pub title: String,
    pub image: String,
    #[serde(default)]
    pub spots: Vec<Spot>,
}

pub const MAX_FIGURES: usize = 30;
const MAX_IMAGE: usize = 2_000_000;

/// Nur Rasterbilder als data:-URL mit gültigem Base64 (kein SVG, kein HTML).
pub fn valid_image(s: &str) -> bool {
    s.len() <= MAX_IMAGE
        && ["data:image/jpeg;base64,", "data:image/png;base64,", "data:image/webp;base64,"]
            .iter()
            .filter_map(|p| s.strip_prefix(p))
            .any(|b64| !b64.is_empty() && b64.bytes().all(|c| c.is_ascii_alphanumeric() || b"+/=".contains(&c)))
}

/// Grafiken eines Dokuments der passenden Einheit zuordnen (Seite liegt darin oder am nächsten dran).
/// Ab 3 Beschriftungen entsteht direkt eine Beschriftungsaufgabe, Ablenker kommen aus den anderen Grafiken.
pub fn store_figures(conn: &Connection, document_id: i64, figures: &[FigureIn]) -> rusqlite::Result<usize> {
    let mut rng = learn::Shuffler::new();
    let mut stored = 0;
    for (i, f) in figures.iter().take(MAX_FIGURES).enumerate() {
        if !valid_image(&f.image) {
            continue;
        }
        let unit: Option<i64> = conn
            .query_row(
                "SELECT id FROM units WHERE document_id = ?1
                 ORDER BY MAX(page_from - ?2, 0) + MAX(?2 - page_to, 0), idx LIMIT 1",
                params![document_id, f.page],
                |r| r.get(0),
            )
            .optional()?;
        let Some(unit_id) = unit else { continue };
        conn.execute("INSERT INTO figures(unit_id, page, image) VALUES(?1, ?2, ?3)", params![unit_id, f.page, f.image])?;
        let figure_id = conn.last_insert_rowid();
        stored += 1;
        if f.spots.len() < 3 {
            continue;
        }
        let mut others: Vec<String> = figures.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, o)| o.spots.iter().map(|s| s.label.clone())).collect();
        rng.shuffle(&mut others);
        others.truncate(2);
        let title: String = f.title.trim().chars().take(80).collect();
        let prompt = if title.is_empty() { "Beschrifte die Grafik.".to_string() } else { format!("Beschrifte die Grafik „{title}“.") };
        let q = Question { kind: "label".into(), prompt, options: others, spots: f.spots.clone(), ..Default::default() };
        if let Some(q) = fix(q, false) {
            insert_question(conn, unit_id, &q, 1, Some(figure_id))?;
        }
    }
    Ok(stored)
}

/// Speichert ein neues Dokument und legt pro Abschnitt eine wartende Einheit an.
pub fn store_document(db: &Db, course_id: i64, name: &str, pages: &[String], figures: &[FigureIn]) -> rusqlite::Result<(i64, usize, usize)> {
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
    let figs = store_figures(&tx, doc_id, figures)?;
    tx.commit()?;
    Ok((doc_id, secs.len(), figs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(kind: &str, prompt: &str, options: &[&str], answer: &str) -> Question {
        Question { kind: kind.into(), prompt: prompt.into(), options: options.iter().map(|s| s.to_string()).collect(), answer: answer.into(), why: "weil".into(), ..Default::default() }
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

        // Strukturaufgaben mit Selbstprüfung: SWOT-Text soll eine Matrix ergeben, Marktformen keine erfundene.
        for (name, text, level) in [("swot", include_str!("../testdata/swot.txt"), 1), ("swot", include_str!("../testdata/swot.txt"), 2), ("marktformen", text, 1)] {
            let t = std::time::Instant::now();
            let tasks = generate_tasks(&ai, text, level).expect("Strukturaufgaben");
            println!("\nStrukturaufgaben {name} Stufe {level} in {:.1} s", t.elapsed().as_secs_f32());
            for task in tasks {
                println!("  [{}] {}\n      {}\n      Ablenker: {:?}", task.kind, task.prompt, task.answer, task.options);
                let t = std::time::Instant::now();
                match verify_task(&ai, text, task).expect("Selbstprüfung") {
                    Some(v) => println!("      geprüft ({:.1} s): {}", t.elapsed().as_secs_f32(), v.answer),
                    None => println!("      verworfen ({:.1} s)", t.elapsed().as_secs_f32()),
                }
            }
        }
    }

    #[test]
    fn filters_bad_tasks() {
        let g = |n: &str, i: &[&str]| Group { name: n.into(), items: i.iter().map(|s| s.to_string()).collect() };
        let task = |kind: &str, prompt: &str, options: &[&str], groups: Vec<Group>| Question { groups, ..q(kind, prompt, options, "") };

        // Matrix: Dubletten über Felder hinweg und Ablenker, die einer Aussage gleichen, fliegen raus
        let t = fix(task("sort", "Ordne die Aussagen der SWOT zu.", &["treue kunden", "Schönes Wetter"], vec![g("Stärken", &["Treue Kunden", " "]), g("Risiken", &["Neue Konkurrenz", "treue Kunden", "Hohe Mieten"])]), true).unwrap();
        assert_eq!(t.groups[1].items, vec!["Neue Konkurrenz", "Hohe Mieten"]);
        assert_eq!(t.options, vec!["Schönes Wetter"]);
        assert_eq!(t.answer, "Stärken: Treue Kunden · Risiken: Neue Konkurrenz, Hohe Mieten");
        assert!(fix(task("sort", "Ordne die Aussagen zu.", &[], vec![g("Stärken", &["a", "b", "c"])]), true).is_none()); // nur ein Feld

        // Verankerung: Felder müssen im Material stehen
        let swot = "Die SWOT-Analyse stellt jede Stärke und Schwäche des Unternehmens den Chancen und dem Risiko am Markt gegenüber.";
        assert!(grounded(&task("sort", "x", &[], vec![g("Stärken", &[]), g("Schwächen", &[]), g("Chancen", &[]), g("Risiken", &[])]), swot));
        assert!(!grounded(&task("sort", "x", &[], vec![g("Stärken", &[]), g("Marktmacht", &[])]), swot));

        // Reihenfolge
        assert_eq!(fix(task("order", "Bringe die Schritte in Reihenfolge.", &["Ziel", "Analyse", "Strategie"], vec![]), true).unwrap().answer, "Ziel → Analyse → Strategie");
        assert!(fix(task("order", "Bringe die Schritte in Reihenfolge.", &["Ziel", "ziel", "Analyse"], vec![]), true).is_none());
        let t = fix(task("order", "Bringe die Schritte in Reihenfolge.", &["1. Ziel festlegen", "Schritt 2: Intern analysieren", "- 3) Strategie"], vec![]), true).unwrap();
        assert_eq!(t.options, vec!["Ziel festlegen", "Intern analysieren", "Strategie"]);

        // Lückentext: „wie“ zweimal ist Prosa, keine Doppelfrage; Ablenker gleich einer Lücke fliegt raus
        let t = fix(task("cloze", "Wie gut die [Stärken] sind, zeigt sich, wie die [Konkurrenz] reagiert.", &["stärken", "Kunden"], vec![]), true).unwrap();
        assert_eq!((t.answer.as_str(), t.options.clone()), ("Stärken · Konkurrenz", vec!["Kunden".to_string()]));
        assert!(fix(task("cloze", "Ganz ohne Lücke geschrieben.", &[], vec![]), true).is_none());
        assert_eq!(fix(task("cloze", "Im __[Polypol]__ gibt es viele Anbieter.", &[], vec![]), true).unwrap().prompt, "Im [Polypol] gibt es viele Anbieter.");
        assert!(fix(task("cloze", "Eine [Lücke und eine offene [Klammer.", &[], vec![]), true).is_none());

        // Grafik: Stellen im Bild, ungültige Koordinaten verwerfen
        let spot = |x: f64, label: &str| Spot { x, y: 0.1, w: 0.2, h: 0.1, label: label.into() };
        let t = Question { spots: vec![spot(0.1, " Wurzel "), spot(0.5, "Blatt")], ..q("label", "Beschrifte die Grafik.", &["blatt", "Blüte"], "") };
        let t = fix(t, false).unwrap();
        assert_eq!((t.answer.as_str(), t.options.clone()), ("1 Wurzel · 2 Blatt", vec!["Blüte".to_string()]));
        assert!(fix(Question { spots: vec![spot(0.9, "zu weit rechts")], ..q("label", "Beschrifte die Grafik.", &[], "") }, false).is_none());
        assert!(valid_image("data:image/png;base64,iVBORw0KGgo=") && !valid_image("data:image/svg+xml;base64,PHN2Zz4=") && !valid_image("data:image/png;base64,<script>"));
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
