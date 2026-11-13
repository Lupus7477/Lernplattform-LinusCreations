//! Fehler melden: Systemdaten, Vorsortieren (Programmfehler oder normaler Hinweis), KI-Beschreibung und Issue-Text für GitHub.
//! Der Server legt das Issue an (`post_issue`); ohne Server öffnet die Oberfläche ein vorausgefülltes Issue im Browser.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::{Ai, AiError, Gen, Msg};
use crate::learn::normalize;
use crate::Core;

pub const REPO: &str = "Lupus7477/Lernplattform-LinusCreations";

/// Teile von Meldungen, die der Code selbst für Nutzer schreibt: kein Programmfehler (Vergleich nach `normalize`).
pub const NORMAL: &[&str] = &[
    "kein zugriff",
    "eingescannt",
    "melde dich an",
    "nicht erreichbar",
    "verbindung zum server ist abgebrochen",
    "zu viele",
    "wird noch vorbereitet",
    "zu gross",
    "passwort",
    "der name braucht",
    "im namen gehen nur",
    "gibt es auf dem server schon",
    "ungueltig, abgelaufen oder schon benutzt",
    "einrichtungslink ist ungueltig",
    "zertifikat",
    "gib dem kurs einen namen",
    "so geht die aufgabe nicht",
    "die aufgabe ist zu lang",
    "das bild geht so nicht",
    "waehle zuerst",
    "kannst du nur loeschen",
    "nicht selbst sperren",
    "server faehrt herunter",
    "einziger admin",
];

/// Teile von Meldungen, die sicher auf einen Programmfehler zeigen.
const CODE: &[&str] = &[
    "typeerror",
    "referenceerror",
    "syntaxerror",
    "rangeerror",
    "is not a function",
    "is not defined",
    "cannot read properties",
    "undefined",
    "unbekannter befehl",
    "missing field",
    "unknown field",
    "invalid type",
    "no such column",
    "no such table",
    "sqlite",
    "database",
    "ki-fehler",
    "panicked",
    "antwort unlesbar",
    "grammatik",
    "unbekannte freigabe",
    "fehler 500",
    "fehler 502",
    "fehler 503",
];

// ---------- Systemdaten ----------

/// `/proc/meminfo`-Wert in MB (Linux, Android).
// ponytail: RAM nur unter Linux/Android, sonst null. `sysinfo`-Crate, falls Meldungen von Windows/macOS ihn oft brauchen.
fn mem_mb(key: &str) -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|l| l.starts_with(key))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb / 1024)
}

fn os_name() -> Option<String> {
    let rel = std::fs::read_to_string("/etc/os-release").ok()?;
    let line = rel.lines().find(|l| l.starts_with("PRETTY_NAME="))?;
    Some(line["PRETTY_NAME=".len()..].trim_matches('"').to_string())
}

/// Die Maschine, auf der dieser Kern läuft. Keine Namen, IPs, Hostnamen, Pfade oder Kursinhalte.
pub fn system_info(c: &Core) -> Value {
    let db = {
        let conn = c.db.lock().unwrap();
        let n = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(-1);
        json!({
            "schema": n("PRAGMA user_version"),
            "courses": n("SELECT COUNT(*) FROM courses"),
            "units": n("SELECT COUNT(*) FROM units"),
            "questions": n("SELECT COUNT(*) FROM questions"),
            "units_pending": n("SELECT COUNT(*) FROM units WHERE status = 'pending'"),
            "units_orga": n("SELECT COUNT(*) FROM units WHERE status = 'orga'"),
            "units_failed": n("SELECT COUNT(*) FROM units WHERE q_state = 3"),
        })
    };
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "os_name": os_name(),
        "cpu_threads": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "ram_total_mb": mem_mb("MemTotal:"),
        "ram_free_mb": mem_mb("MemAvailable:"),
        "ki": c.ai.info(),
        "db": db,
    })
}

// ---------- KI ----------

const SYSTEM: &str = "Du hilfst den Entwicklern einer Lern-App, Rückmeldungen zu ordnen. Antworte sachlich und auf Deutsch.";

fn ask<T: for<'de> Deserialize<'de>>(ai: &Ai, user: String, schema: &Value, max_tokens: usize) -> Result<T, AiError> {
    let g = llama_cpp_2::json_schema_to_grammar(&schema.to_string()).map_err(|e| AiError::Failed(format!("Grammatik: {e}")))?;
    let raw = ai.generate(
        Gen { messages: vec![Msg { role: "system", content: SYSTEM.into() }, Msg { role: "user", content: user }], grammar: Some(&g), max_tokens, temperature: 0.1 },
        |_| {},
    )?;
    serde_json::from_str(&raw).map_err(|e| AiError::Failed(format!("Antwort unlesbar: {e}")))
}

/// Einschätzung einer Fehlermeldung. `by`: „regel“ (sicher), „ki“ oder „ohne_ki“ (KI nicht bereit → im Zweifel melden lassen).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Triage {
    pub code: bool,
    pub hint: String,
    pub by: String,
}

/// Sichere Fälle per Regel, nur der Rest geht an die KI.
pub fn triage_rule(message: &str) -> Option<bool> {
    let m = normalize(message);
    if CODE.iter().any(|p| m.contains(p)) {
        Some(true)
    } else if NORMAL.iter().any(|p| m.contains(p)) {
        Some(false)
    } else {
        None
    }
}

pub fn triage(ai: &Ai, message: &str, cmd: &str) -> Triage {
    if let Some(code) = triage_rule(message) {
        return Triage { code, hint: String::new(), by: "regel".into() };
    }
    #[derive(Deserialize)]
    struct R {
        art: String,
        hinweis: String,
    }
    let schema = json!({"type": "object", "required": ["art", "hinweis"], "properties": {
        "art": {"enum": ["code", "normal"]}, "hinweis": {"type": "string", "maxLength": 240}}});
    let at = if cmd.is_empty() { String::new() } else { format!(" (bei „{cmd}“)") };
    let user = format!(
        "Die App zeigt diese Fehlermeldung{at}:\n\"{}\"\n\n\
         Ist das ein Programmfehler, den die Entwickler beheben müssen (art = code), \
         oder ein normaler Hinweis wie falsche Eingabe, fehlende Verbindung, ungeeignete Datei oder Bedienung (art = normal)?\n\
         hinweis: 1 bis 2 kurze Sätze. Bei normal: was die Person tun kann. Bei code: was vermutlich kaputt ist.",
        head(message, 600)
    );
    match ask::<R>(ai, user, &schema, 160) {
        Ok(r) => Triage { code: r.art == "code", hint: r.hinweis.trim().to_string(), by: "ki".into() },
        Err(_) => Triage { code: true, hint: String::new(), by: "ohne_ki".into() },
    }
}

/// Titel und Beschreibung des Issues. Ohne KI: erste Zeile als Titel, Beschreibung bleibt leer (der Originaltext steht ohnehin drin).
pub fn draft(ai: &Ai, kind: &str, text: &str, error: &str) -> (String, String, bool) {
    #[derive(Deserialize)]
    struct R {
        title: String,
        description: String,
    }
    let schema = json!({"type": "object", "required": ["title", "description"], "properties": {
        "title": {"type": "string", "maxLength": 90}, "description": {"type": "string", "maxLength": 700}}});
    let art = if kind == "idea" { "Idee oder Wunsch" } else { "Fehler" };
    let err = if error.is_empty() { String::new() } else { format!("\nFehlermeldung:\n\"\"\"\n{}\n\"\"\"", head(error, 1500)) };
    let user = format!(
        "Schreibe aus dieser Rückmeldung zu einer Lern-App ein GitHub-Issue.\nArt: {art}\nText der Person:\n\"\"\"\n{}\n\"\"\"{err}\n\n\
         title: höchstens 10 Wörter, sachlich.\n\
         description: 2 bis 5 Sätze: was passiert ist oder gewünscht wird, wobei, und was zu erwarten wäre. Erfinde nichts, was nicht oben steht.",
        head(text, 3000)
    );
    match ask::<R>(ai, user, &schema, 400) {
        Ok(r) if !r.title.trim().is_empty() => (r.title.trim().to_string(), r.description.trim().to_string(), true),
        _ => {
            let first = text.lines().chain(error.lines()).find(|l| !l.trim().is_empty()).unwrap_or(art);
            (head(first.trim(), 70).to_string(), String::new(), false)
        }
    }
}

// ---------- Issue-Text ----------

fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map(|(i, _)| &s[..i]).unwrap_or(s)
}

/// Text von Personen (oder der KI) außerhalb von Code-Blöcken: keine @-Erwähnungen, Links, Bilder oder HTML.
fn plain(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('@', "@\u{200B}").replace('[', "\\[").replace('`', "'")
}

/// Code-Block, aus dem nichts ausbrechen kann (Backticks entschärft).
fn block(lang: &str, s: &str) -> String {
    format!("```{lang}\n{}\n```\n", s.replace('`', "'"))
}

#[derive(Deserialize, Default)]
pub struct Input {
    /// „bug“ oder „idea“
    pub kind: String,
    #[serde(default)]
    pub text: String,
    /// Fehlermeldung samt Befehl und letzten Fehlern, als Text
    #[serde(default)]
    pub error: String,
    /// Frage aus dem Fragen-Menü, als Text
    #[serde(default)]
    pub question: String,
    /// Systemdaten des Geräts (Pflicht)
    #[serde(default)]
    pub device: Value,
    /// Einschätzung, wenn die Oberfläche sie schon geholt hat (der Server rechnet sie selbst)
    pub triage: Option<Triage>,
    /// Systemdaten des Servers: setzt nur der Server selbst
    #[serde(default)]
    pub server: Value,
    /// Meldung, die die Oberfläche beim Fehler gezeigt hat (für die Einschätzung)
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub cmd: String,
}

#[derive(Serialize, Debug)]
pub struct Draft {
    pub title: String,
    pub body: String,
    pub label: &'static str,
}

/// Prüft die Eingabe einer Meldung. Systemdaten des Geräts sind Pflicht.
pub fn check(i: &Input) -> Result<(), String> {
    let n = |s: &str| s.chars().count();
    if i.kind != "bug" && i.kind != "idea" {
        return Err("Unbekannte Art der Meldung".into());
    }
    if !i.device.is_object() || i.device.as_object().is_some_and(|o| o.is_empty()) {
        return Err("Die Systemdaten des Geräts fehlen.".into());
    }
    if i.error.is_empty() && n(i.text.trim()) < 10 {
        return Err("Beschreib kurz in ein paar Worten, worum es geht.".into());
    }
    if n(&i.text) > 4000 || n(&i.error) > 8000 || n(&i.question) > 8000 || i.device.to_string().len() > 8000 {
        return Err("Die Meldung ist zu lang.".into());
    }
    Ok(())
}

/// Issue-Text: KI-Beschreibung, Einschätzung, Originaltext, Fehlermeldung, Frage, Gerät und Server.
pub fn body(i: &Input, description: &str, by_ai: bool, triage: Option<&Triage>) -> String {
    let mut b = String::from("## Beschreibung (KI)\n\n");
    b.push_str(&if by_ai && !description.is_empty() { plain(description) } else { "_(ohne KI: siehe Originaltext)_".into() });
    b.push_str("\n\n");
    if let Some(t) = triage {
        let what = if t.code { "Programmfehler" } else { "normaler Hinweis der App (trotzdem gemeldet)" };
        let how = match t.by.as_str() {
            "regel" => " (nach Regel)",
            "ki" => " (laut KI)",
            _ => " (ohne KI eingeschätzt)",
        };
        b.push_str(&format!("**Einschätzung:** {what}{how}"));
        if !t.hint.is_empty() {
            b.push_str(&format!(": {}", plain(&t.hint)));
        }
        b.push_str("\n\n");
    }
    if !i.text.trim().is_empty() {
        b.push_str("## Originaltext\n\n");
        b.push_str(&block("text", i.text.trim()));
    }
    if !i.error.is_empty() {
        b.push_str("\n## Fehlermeldung\n\n");
        b.push_str(&block("text", &i.error));
    }
    if !i.question.is_empty() {
        b.push_str("\n## Frage\n\n");
        b.push_str(&block("text", &i.question));
    }
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    b.push_str(&format!("\n<details><summary>Gerät</summary>\n\n{}</details>\n", block("json", &pretty(&i.device))));
    if !i.server.is_null() {
        b.push_str(&format!("\n<details><summary>Server</summary>\n\n{}</details>\n", block("json", &pretty(&i.server))));
    }
    b
}

/// Titel, Text und Label eines Issues. Bei einer Fehlermeldung ohne Einschätzung schätzt die KI sie hier ein.
pub fn make(ai: &Ai, mut i: Input) -> Result<Draft, String> {
    check(&i)?;
    let triage = match (i.triage.take(), i.message.is_empty()) {
        (Some(t), _) => Some(t),
        (None, false) => Some(triage(ai, &i.message, &i.cmd)),
        (None, true) => None,
    };
    let (title, description, by_ai) = draft(ai, &i.kind, &i.text, &i.error);
    let label = if i.kind == "idea" { "enhancement" } else { "bug" };
    Ok(Draft { title: head(&title, 120).to_string(), body: body(&i, &description, by_ai, triage.as_ref()), label })
}

/// Legt das Issue auf GitHub an (nur der Server, mit Token). Gibt Nummer und Link zurück.
pub fn post_issue(token: &str, d: &Draft) -> Result<(i64, String), String> {
    let payload = json!({ "title": d.title, "body": d.body, "labels": [d.label] }).to_string();
    let resp = ureq::post(format!("https://api.github.com/repos/{REPO}/issues"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "lernpfad-server")
        .header("Content-Type", "application/json")
        .send(payload)
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => "GitHub kennt den Token nicht (abgelaufen?).".to_string(),
            ureq::Error::StatusCode(403 | 404) => "Der GitHub-Token darf in diesem Repo keine Issues anlegen.".to_string(),
            ureq::Error::StatusCode(c) => format!("GitHub hat die Meldung abgelehnt (Fehler {c})."),
            _ => "GitHub ist gerade nicht erreichbar.".to_string(),
        })?;
    let v: Value = serde_json::from_reader(resp.into_body().into_reader()).map_err(|_| "Antwort von GitHub unlesbar".to_string())?;
    Ok((v["number"].as_i64().unwrap_or(0), v["html_url"].as_str().unwrap_or_default().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triage_rules() {
        assert_eq!(triage_rule(crate::api::DENIED), Some(false));
        assert_eq!(triage_rule("In diesem PDF wurde kein Text gefunden. Ist es eingescannt?"), Some(false));
        assert_eq!(triage_rule("Der Server ist gerade nicht erreichbar."), Some(false));
        assert_eq!(triage_rule("TypeError: x is undefined"), Some(true));
        assert_eq!(triage_rule("no such column: u.exam"), Some(true));
        assert_eq!(triage_rule("Irgendwas Seltsames"), None);
    }

    #[test]
    fn body_has_system_data_and_no_mentions() {
        let i = Input {
            kind: "bug".into(),
            text: "Klappt nicht, frag @octocat ```rm```".into(),
            device: json!({"os": "linux"}),
            server: json!({"version": "0.3.0"}),
            ..Default::default()
        };
        check(&i).unwrap();
        let b = body(&i, "Beschreibung mit @jemand und ![bild](x)", true, Some(&Triage { code: true, hint: "Absturz".into(), by: "ki".into() }));
        assert!(b.contains("<summary>Gerät</summary>") && b.contains("<summary>Server</summary>"));
        assert!(b.contains("\"os\": \"linux\"") && b.contains("Programmfehler (laut KI): Absturz"));
        assert!(!b.contains("@jemand") && !b.contains("![bild]"));
        // Originaltext im Code-Block, dessen Zaun sich nicht schließen lässt
        assert!(b.contains("```text\nKlappt nicht, frag @octocat '''rm'''\n```"));
        // Ohne Gerät keine Meldung
        assert!(check(&Input { device: Value::Null, ..i }).is_err());
    }
}
