//! Tauri-Hülle. Standard: jeder Befehl geht an den Kern auf diesem Gerät (`lernpfad_core::api::call`).
//! Optional mit eigenem Server verbunden: dann werden die Befehle dorthin weitergeleitet.
//! Lokale Daten werden dabei nie angefasst; „Zurück zu lokal“ geht immer, auch wenn der Server weg ist.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lernpfad_core::{api, db, Core, LOCAL};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, State};

/// Verbindung zum eigenen Server.
#[derive(Clone)]
struct Remote {
    url: String,
    token: String,
    /// Selbst erzeugtes Server-Zertifikat (gegen den Fingerabdruck im Verbindungslink geprüft). `None` = öffentlich gültiges.
    cert: Option<String>,
    agent: ureq::Agent,
}

struct App {
    core: Arc<Core>,
    remote: Mutex<Option<Remote>>,
    /// Solange verbunden: Statusmeldungen der lokalen KI nicht an die Oberfläche geben
    online: Arc<AtomicBool>,
    /// Neue Verbindung oder Trennen zählt hoch; ein alter Ereignis-Thread beendet sich dann.
    epoch: Arc<AtomicU64>,
    handle: AppHandle,
}

type R = Result<Value, Value>;

fn msg(e: impl std::fmt::Display) -> Value {
    Value::String(e.to_string())
}

const OFFLINE: &str = "Der Server ist gerade nicht erreichbar.";
const BAD_CERT: &str = "Das Zertifikat des Servers passt nicht (mehr) zu dieser Verbindung. Hat sich der Server geändert, verbinde dich mit einem neuen Verbindungslink. \
                        Ändert es sich unerwartet, könnte jemand die Verbindung abfangen.";

fn user_agent() -> String {
    let os = match std::env::consts::OS {
        "linux" => "Linux",
        "windows" => "Windows",
        "macos" => "macOS",
        "android" => "Android",
        "ios" => "iOS",
        o => o,
    };
    format!("Lernpfad-App ({os})")
}

/// HTTP-Client, der nur dem gepinnten Zertifikat vertraut (oder, ohne Pin, den üblichen Stammzertifikaten).
fn agent(cert: Option<&str>, verify: bool) -> Result<ureq::Agent, Value> {
    let mut tls = ureq::tls::TlsConfig::builder();
    if let Some(pem) = cert {
        let c = ureq::tls::Certificate::from_pem(pem.as_bytes()).map_err(msg)?;
        tls = tls.root_certs(ureq::tls::RootCerts::new_with_certs(&[c]));
    }
    if !verify {
        tls = tls.disable_verification(true);
    }
    Ok(ureq::Agent::config_builder()
        .tls_config(tls.build())
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(6)))
        .user_agent(user_agent())
        .build()
        .into())
}

/// Zertifikatsfehler kommen von rustls als `Io(InvalidData)`; alles andere heißt: Server nicht erreichbar.
fn net_err(e: ureq::Error) -> Value {
    let detail = format!("{e:?}");
    match e {
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidData && detail.contains("NotValidForName") => {
            msg("Unter diesem Namen kennt das Server-Zertifikat den Server nicht. Starte den Server mit --host <Name> und hol dir einen neuen Verbindungslink.")
        }
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidData => msg(BAD_CERT),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => msg(BAD_CERT),
        _ => msg(OFFLINE),
    }
}

fn post(r: &Remote, cmd: &str, args: &Value) -> Result<ureq::http::Response<ureq::Body>, Value> {
    r.agent
        .post(format!("{}/api/rpc/{cmd}", r.url))
        .header("Authorization", format!("Bearer {}", r.token))
        .header("X-Lernpfad-Client", "app")
        .header("Content-Type", "application/json")
        .send(args.to_string())
        .map_err(net_err)
}

/// Antwort des Servers wie im Browser deuten: 401 → Anmeldung, 429 → Sperre mit Countdown.
fn reply(resp: ureq::http::Response<ureq::Body>) -> R {
    let status = resp.status().as_u16();
    let body = resp.into_body().read_to_string().map_err(|_| msg(OFFLINE))?;
    match status {
        200 => serde_json::from_str(&body).map_err(msg),
        401 => Err(json!({ "auth": true, "message": "Bitte melde dich an." })),
        429 => {
            let v: Value = serde_json::from_str(&body).unwrap_or_default();
            Err(json!({ "locked": v["retry_after"].as_u64().unwrap_or(30), "message": v["error"] }))
        }
        _ => Err(msg(body)),
    }
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// `https://host:8443/#pin=<sha256>` → (Basis-URL, Pin). Ohne Pin: Server mit öffentlich gültigem Zertifikat.
fn parse_link(link: &str) -> Result<(String, Option<String>), Value> {
    let (base, frag) = link.trim().split_once('#').unwrap_or((link.trim(), ""));
    let pin = frag.strip_prefix("pin=").map(str::to_lowercase);
    if pin.as_ref().is_some_and(|p| p.len() != 64 || !p.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(msg("Der Fingerabdruck im Verbindungslink ist unvollständig. Kopiere den Link noch einmal."));
    }
    let base = base.trim_end_matches('/').to_string();
    let local_http = ["http://127.0.0.1", "http://localhost"].iter().any(|p| base.starts_with(p));
    if !base.starts_with("https://") && !local_http {
        return Err(msg("Der Verbindungslink muss mit https:// beginnen."));
    }
    Ok((base, pin))
}

/// Holt das Zertifikat und vertraut ihm nur, wenn es zum Fingerabdruck aus dem Link passt (wie bei SSH).
/// Die ungeprüfte Verbindung überträgt nichts Geheimes; alles Weitere läuft nur noch mit genau diesem Zertifikat.
fn fetch_cert(url: &str, pin: &str) -> Result<String, Value> {
    let resp = agent(None, false)?.get(format!("{url}/api/cert")).call().map_err(net_err)?;
    let pem = resp.into_body().read_to_string().map_err(|_| msg(OFFLINE))?;
    let cert = ureq::tls::Certificate::from_pem(pem.as_bytes()).map_err(|_| msg("Der Server hat kein gültiges Zertifikat geschickt."))?;
    if sha256_hex(cert.der()) != pin {
        return Err(msg("Der Fingerabdruck des Servers passt nicht zum Verbindungslink. Prüf den Link, sonst fängt womöglich jemand die Verbindung ab."));
    }
    Ok(pem)
}

/// Anmelden/Einrichten/Beitreten am Server; das Token kommt nur für Apps in der Antwort (Browser: HttpOnly-Cookie).
fn sign_in(r: &Remote, cmd: &str, args: &Value) -> Result<String, Value> {
    let v = reply(post(r, cmd, args)?)?;
    v["token"].as_str().map(String::from).ok_or_else(|| msg("Der Server hat keine Sitzung geschickt."))
}

impl App {
    fn remote(&self) -> Option<Remote> {
        self.remote.lock().unwrap().clone()
    }

    // ponytail: Token liegt in der App-Datenbank (im geschützten App-Ordner). Schlüsselbund des Systems, falls das nicht reicht.
    fn save(&self, r: Option<&Remote>) -> Result<(), Value> {
        let conn = self.core.db.lock().unwrap();
        let set = |k: &str, v: &str| db::set_setting(&conn, k, v).map_err(msg);
        if let Some(r) = r {
            set("server_url", &r.url)?;
            set("server_token", &r.token)?;
            set("server_cert", r.cert.as_deref().unwrap_or(""))?;
        }
        set("server_active", if r.is_some() { "1" } else { "0" })
    }

    fn activate(&self, r: Option<Remote>) {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.online.store(r.is_some(), Ordering::SeqCst);
        if let Some(r) = &r {
            listen(self.handle.clone(), r.clone(), self.epoch.clone(), epoch);
        }
        *self.remote.lock().unwrap() = r;
    }

    /// Gespeicherte Verbindung beim Start wiederherstellen (nur wenn sie aktiv war).
    fn restore(&self) {
        let conn = self.core.db.lock().unwrap();
        let get = |k: &str| db::setting(&conn, k).unwrap_or_default();
        if get("server_active") != "1" || get("server_token").is_empty() {
            return;
        }
        let cert = Some(get("server_cert")).filter(|c| !c.is_empty());
        let (url, token) = (get("server_url"), get("server_token"));
        drop(conn);
        if let Ok(agent) = agent(cert.as_deref(), true) {
            self.activate(Some(Remote { url, token, cert, agent }));
        }
    }

    fn connect(&self, args: &Value) -> R {
        let (url, pin) = parse_link(args["link"].as_str().unwrap_or(""))?;
        let cert = pin.map(|p| fetch_cert(&url, &p)).transpose()?;
        let mut r = Remote { agent: agent(cert.as_deref(), true)?, url, token: String::new(), cert };
        r.token = sign_in(&r, "login", &json!({ "name": args["name"], "password": args["password"] }))?;
        self.save(Some(&r))?;
        self.activate(Some(r));
        Ok(Value::Null)
    }

    fn call(&self, cmd: &str, args: Value, on_token: &mut dyn FnMut(&str)) -> R {
        match cmd {
            "server_connect" => return self.connect(&args),
            "server_disconnect" => {
                self.save(None)?;
                self.activate(None);
                return Ok(Value::Null);
            }
            _ => {}
        }
        let Some(mut r) = self.remote() else {
            return match cmd {
                "session" => Ok(json!({ "mode": "local", "user": null })),
                _ => api::call(&self.core, &LOCAL, cmd, args, on_token).map_err(msg),
            };
        };
        match cmd {
            "session" => {
                let mut v = reply(post(&r, cmd, &args)?)?;
                v["remote"] = json!({ "url": r.url });
                Ok(v)
            }
            // Sitzung abgelaufen → neu anmelden; neues Token merken
            "login" | "setup" | "join" | "reset_password" => {
                r.token = sign_in(&r, cmd, &args)?;
                self.save(Some(&r))?;
                self.activate(Some(r));
                Ok(json!({ "ok": true }))
            }
            "explain" | "ask" => {
                let resp = post(&r, cmd, &args)?;
                if resp.status() != 200 {
                    return reply(resp);
                }
                // NDJSON: {"t": Stück} … {"ok": Ergebnis} | {"err": Meldung}
                for line in BufReader::new(resp.into_body().into_reader()).lines() {
                    let line = line.map_err(|_| msg(OFFLINE))?;
                    let v: Value = serde_json::from_str(&line).unwrap_or_default();
                    if let Some(t) = v["t"].as_str() {
                        on_token(t);
                    } else if v.get("ok").is_some() {
                        return Ok(v["ok"].clone());
                    } else if v.get("err").is_some() {
                        return Err(v["err"].clone());
                    }
                }
                Err(msg(OFFLINE))
            }
            _ => reply(post(&r, cmd, &args)?),
        }
    }
}

/// Ereignisse des Servers (SSE) als Tauri-Ereignisse weitergeben; bei Abbruch alle 5 s neu verbinden.
fn listen(handle: AppHandle, r: Remote, epoch: Arc<AtomicU64>, mine: u64) {
    std::thread::spawn(move || {
        let alive = || epoch.load(Ordering::SeqCst) == mine;
        while alive() {
            let resp = r.agent.get(format!("{}/api/events", r.url)).header("Authorization", format!("Bearer {}", r.token)).call();
            if let Some(resp) = resp.ok().filter(|x| x.status() == 200) {
                let mut event = String::new();
                for line in BufReader::new(resp.into_body().into_reader()).lines() {
                    let Ok(line) = line else { break };
                    if !alive() {
                        return;
                    }
                    if let Some(e) = line.strip_prefix("event:") {
                        event = e.trim().to_string();
                    } else if let Some(d) = line.strip_prefix("data:") {
                        let _ = handle.emit(&event, serde_json::from_str::<Value>(d.trim()).unwrap_or(Value::Null));
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

#[tauri::command]
async fn rpc(app: State<'_, Arc<App>>, cmd: String, args: Value) -> R {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || app.call(&cmd, args, &mut |_| {})).await.map_err(msg)?
}

/// Wie `rpc`, aber der Text kommt stückweise über `on_token` (Erklären, Frag die KI).
#[tauri::command]
async fn rpc_stream(app: State<'_, Arc<App>>, cmd: String, args: Value, on_token: Channel<String>) -> R {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.call(&cmd, args, &mut |p| {
            let _ = on_token.send(p.to_string());
        })
    })
    .await
    .map_err(msg)?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let online = Arc::new(AtomicBool::new(false));
            let (ai_handle, course_handle) = (app.handle().clone(), app.handle().clone());
            let (quiet_ai, quiet_course) = (online.clone(), online.clone());
            let core = Core::open(
                &dir,
                move |s| {
                    if !quiet_ai.load(Ordering::SeqCst) {
                        let _ = ai_handle.emit("ai-status", s);
                    }
                },
                move |course_id| {
                    if !quiet_course.load(Ordering::SeqCst) {
                        let _ = course_handle.emit("course-updated", course_id);
                    }
                },
            )?;
            let shell = Arc::new(App { core: Arc::new(core), remote: Mutex::new(None), online, epoch: Arc::new(AtomicU64::new(0)), handle: app.handle().clone() });
            shell.restore();
            app.manage(shell);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc, rpc_stream])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der App");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_parsing() {
        let pin = "ab".repeat(32);
        assert_eq!(parse_link(&format!("https://pc.local:8443/#pin={pin}")).unwrap(), ("https://pc.local:8443".into(), Some(pin)));
        assert_eq!(parse_link("https://lernen.example.org/").unwrap(), ("https://lernen.example.org".into(), None));
        assert!(parse_link("http://192.168.1.5:8080/").is_err(), "kein Klartext übers Netz");
        assert!(parse_link("https://pc.local:8443/#pin=abc").is_err(), "abgeschnittener Fingerabdruck");
        assert!(parse_link("http://127.0.0.1:8080/").is_ok(), "Entwicklung auf dem eigenen Rechner");
    }

    /// Gegen einen laufenden Server:
    /// LERNPFAD_LINK='https://127.0.0.1:8443/#pin=…' LERNPFAD_NAME=… LERNPFAD_PW=… cargo test -p lernplattform -- --ignored
    #[test]
    #[ignore]
    fn pinned_connection() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} fehlt"));
        let (url, pin) = parse_link(&env("LERNPFAD_LINK")).unwrap();
        let pin = pin.expect("Link mit Fingerabdruck");
        assert!(fetch_cert(&url, &"0".repeat(64)).is_err(), "falscher Fingerabdruck wird abgelehnt");
        let cert = fetch_cert(&url, &pin).unwrap();
        let mut r = Remote { agent: agent(Some(&cert), true).unwrap(), url: url.clone(), token: String::new(), cert: Some(cert) };
        assert!(sign_in(&r, "login", &json!({ "name": env("LERNPFAD_NAME"), "password": "falsch" })).is_err());
        r.token = sign_in(&r, "login", &json!({ "name": env("LERNPFAD_NAME"), "password": env("LERNPFAD_PW") })).unwrap();
        let session = reply(post(&r, "session", &json!({})).unwrap()).unwrap();
        assert_eq!(session["user"]["name"], env("LERNPFAD_NAME"));
        let courses = reply(post(&r, "courses", &json!({})).unwrap()).unwrap();
        assert!(courses.is_array());
        // Ein anderes (z. B. untergeschobenes) Zertifikat: Verbindung scheitert hart.
        let other = std::fs::read_to_string(env("LERNPFAD_OTHER_CERT")).unwrap();
        let evil = Remote { agent: agent(Some(&other), true).unwrap(), ..r.clone() };
        assert_eq!(post(&evil, "session", &json!({})).unwrap_err(), msg(BAD_CERT));
    }
}
