//! Lernpfad-Server: liefert die Web-App aus und gibt angemeldeten Konten Zugriff auf den Lernpfad-Kern.
//! Grundsatz: Wer nur die Adresse kennt, bekommt nichts außer der Anmeldeseite.

mod auth;

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::{to_bytes, Body};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::stream;
use lernpfad_core::{api, db, Core, Who};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, Semaphore};
use tower_http::services::ServeDir;

const COOKIE: &str = "__Host-lernpfad";
const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; worker-src 'self' blob:; \
                   script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
const USAGE: &str = "lernpfad-server [Optionen]

  --data DIR         Datenordner (Datenbank, KI-Modell, Zertifikat). Standard: ./data
  --web DIR          Ordner der Oberfläche. Standard: ./web
  --bind ADRESSE     Standard: 0.0.0.0:8443 (HTTPS), mit --behind-proxy/--http 127.0.0.1:8080
  --tls-cert DATEI   Eigenes Zertifikat (PEM), z. B. von Let's Encrypt oder `tailscale cert`
  --tls-key DATEI    Schlüssel dazu (PEM)
  --host NAME        Weiterer Name/IP für das eigene Zertifikat (mehrfach möglich), z. B. pc.tailnet.ts.net
  --behind-proxy     HTTPS macht ein Reverse-Proxy davor (z. B. Caddy); lauscht nur auf 127.0.0.1
  --http             Ohne HTTPS, nur zum Entwickeln auf 127.0.0.1

Ohne --tls-cert erzeugt der Server beim ersten Start ein eigenes Zertifikat im Datenordner.";

struct Srv {
    core: Arc<Core>,
    events: broadcast::Sender<Ev>,
    limiter: Mutex<auth::Limiter>,
    /// Höchstens 2 Passwort-Prüfungen gleichzeitig: Argon2 braucht je 19 MiB.
    hashing: Semaphore,
    setup_token: Mutex<Option<String>>,
    scheme: &'static str,
    behind_proxy: bool,
    hsts: bool,
    /// SHA-256 des selbst erzeugten Zertifikats, für den Verbindungslink der Apps
    pin: Option<String>,
    /// Das selbst erzeugte Zertifikat (öffentlich): Apps holen es einmal und prüfen es gegen `pin`
    cert_pem: Option<String>,
}

#[derive(Clone)]
struct Ev {
    name: &'static str,
    data: String,
    course: Option<i64>,
}

struct Opts {
    data: PathBuf,
    web: PathBuf,
    bind: Option<SocketAddr>,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    hosts: Vec<String>,
    behind_proxy: bool,
    http: bool,
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts { data: "data".into(), web: "web".into(), bind: None, tls_cert: None, tls_key: None, hosts: vec![], behind_proxy: false, http: false };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} braucht einen Wert"));
        match a.as_str() {
            "--data" => o.data = val()?.into(),
            "--web" => o.web = val()?.into(),
            "--bind" => o.bind = Some(val()?.parse().map_err(|_| "--bind erwartet z. B. 0.0.0.0:8443".to_string())?),
            "--tls-cert" => o.tls_cert = Some(val()?.into()),
            "--tls-key" => o.tls_key = Some(val()?.into()),
            "--host" => o.hosts.push(val()?),
            "--behind-proxy" => o.behind_proxy = true,
            "--http" => o.http = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(format!("Unbekannte Option: {a}\n\n{USAGE}")),
        }
    }
    Ok(o)
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Fehler: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let o = parse_args()?;
    let plain = o.behind_proxy || o.http;
    let bind = o.bind.unwrap_or_else(|| if plain { "127.0.0.1:8080" } else { "0.0.0.0:8443" }.parse().unwrap());
    if plain && !bind.ip().is_loopback() {
        return Err("Ohne HTTPS lauscht der Server nur auf 127.0.0.1, sonst gingen Passwörter und Sitzungen unverschlüsselt durchs Netz. \
                    Lass --http weg (dann nutzt der Server HTTPS) oder setze einen Reverse-Proxy mit --behind-proxy davor."
            .into());
    }
    if o.tls_cert.is_some() != o.tls_key.is_some() {
        return Err("--tls-cert und --tls-key gehören zusammen.".into());
    }
    if !o.web.join("index.html").exists() {
        return Err(format!("Oberfläche nicht gefunden: {} (mit --web angeben)", o.web.join("index.html").display()));
    }
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (tls, pin, cert_pem, hsts) = match (&o.tls_cert, &o.tls_key) {
        _ if plain => (None, None, None, false),
        (Some(c), Some(k)) => (Some(RustlsConfig::from_pem_file(c, k).await.map_err(|e| format!("Zertifikat: {e}"))?), None, None, true),
        _ => {
            let (c, k, fp) = self_signed(&o.data.join("tls"), &o.hosts)?;
            let pem = std::fs::read_to_string(&c).map_err(|e| format!("Zertifikat: {e}"))?;
            (Some(RustlsConfig::from_pem_file(c, k).await.map_err(|e| format!("Zertifikat: {e}"))?), Some(fp), Some(pem), false)
        }
    };

    let (events, _) = broadcast::channel::<Ev>(256);
    let (ai_tx, course_tx, data) = (events.clone(), events.clone(), o.data.clone());
    let core = tokio::task::spawn_blocking(move || {
        Core::open(
            &data,
            move |s| {
                let _ = ai_tx.send(Ev { name: "ai-status", data: serde_json::to_string(s).unwrap_or_default(), course: None });
            },
            move |id| {
                let _ = course_tx.send(Ev { name: "course-updated", data: id.to_string(), course: Some(id) });
            },
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    let setup = auth::needs_setup(&core.db.lock().unwrap()).then(|| auth::random_hex(16));
    auth::warm_up();

    let scheme = if o.http { "http" } else { "https" };
    let shown = if bind.ip().is_unspecified() { lan_ip().map(|ip| SocketAddr::new(ip, bind.port())).unwrap_or(bind) } else { bind };
    // Mit --host (z. B. im Container) zeigt der Link die Adresse, unter der man den Server wirklich erreicht.
    let base = match o.hosts.first() {
        _ if o.behind_proxy => "https://<deine Domain>".to_string(),
        Some(h) => format!("{scheme}://{h}:{}", bind.port()),
        None => format!("{scheme}://{shown}"),
    };
    println!("Lernpfad-Server läuft: {base}");
    if let Some(fp) = &pin {
        println!("Eigenes Zertifikat, SHA-256-Fingerabdruck:\n  {}", colon_hex(fp));
        println!("Browser warnen beim ersten Aufruf. Vergleiche den Fingerabdruck, dann fortfahren.");
    }
    if let Some(t) = &setup {
        println!("\nNoch kein Konto. Einrichten (einmalig, nur mit diesem Link):\n  {base}/#/setup/{t}\n");
    }

    let srv = Arc::new(Srv {
        core: Arc::new(core),
        events,
        limiter: Mutex::new(auth::Limiter::default()),
        hashing: Semaphore::new(2),
        setup_token: Mutex::new(setup),
        scheme,
        behind_proxy: o.behind_proxy,
        hsts,
        pin,
        cert_pem,
    });
    let app = Router::new()
        .route("/api/rpc/{cmd}", post(rpc))
        .route("/api/events", get(events_stream))
        .route("/api/cert", get(cert))
        .fallback_service(ServeDir::new(&o.web))
        .layer(middleware::from_fn_with_state(srv.clone(), secure_headers))
        .with_state(srv)
        .into_make_service_with_connect_info::<SocketAddr>();
    let served = match tls {
        Some(cfg) => axum_server::bind_rustls(bind, cfg).serve(app).await,
        None => axum_server::bind(bind).serve(app).await,
    };
    served.map_err(|e| format!("{bind}: {e}"))
}

// ---------- Zertifikat ----------

/// Eigenes Zertifikat im Datenordner (einmal erzeugt, danach wiederverwendet) + SHA-256 seiner DER-Form.
/// Neu erzeugt wird nur, wenn mit `--host` ein Name dazukommt (dann müssen sich Apps neu verbinden).
fn self_signed(dir: &FsPath, hosts: &[String]) -> Result<(PathBuf, PathBuf, String), String> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    let err = |e: &dyn std::fmt::Display| format!("Zertifikat in {}: {e}", dir.display());
    let (cert, key, names_file) = (dir.join("cert.pem"), dir.join("key.pem"), dir.join("names.txt"));
    let known = std::fs::read_to_string(&names_file).unwrap_or_default();
    let missing = hosts.iter().any(|h| !known.lines().any(|l| l == h));
    if !cert.exists() || !key.exists() || missing {
        std::fs::create_dir_all(dir).map_err(|e| err(&e))?;
        let mut names = vec!["localhost".to_string(), "127.0.0.1".into(), "::1".into()];
        names.extend(lan_ip().map(|ip| ip.to_string()));
        if let Some(h) = std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string()).filter(|h| !h.is_empty()) {
            names.push(format!("{h}.local"));
            names.push(h);
        }
        names.extend(hosts.iter().cloned());
        names.dedup();
        let ck = rcgen::generate_simple_self_signed(names.clone()).map_err(|e| err(&e))?;
        write_private(&key, ck.key_pair.serialize_pem().as_bytes()).map_err(|e| err(&e))?;
        std::fs::write(&cert, ck.cert.pem()).map_err(|e| err(&e))?;
        std::fs::write(&names_file, names.join("\n")).map_err(|e| err(&e))?;
        println!("Neues Zertifikat für: {}", names.join(", "));
    }
    let der = CertificateDer::from_pem_file(&cert).map_err(|e| err(&e))?;
    Ok((cert, key, auth::sha256_hex(&der)))
}

/// Schlüsseldatei nur für den eigenen Nutzer lesbar.
fn write_private(path: &FsPath, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(data)
}

/// Adresse im eigenen Netz (für den Link im Log). Es wird nichts gesendet, nur die Route nachgeschlagen.
fn lan_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:80").ok()?;
    s.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_unspecified())
}

fn colon_hex(hex: &str) -> String {
    hex.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().to_uppercase()).collect::<Vec<_>>().join(":")
}

// ---------- Hilfen ----------

fn text(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, msg.into()).into_response()
}

fn locked(secs: u64) -> Response {
    let body = Json(json!({ "error": "Zu viele Fehlversuche. Bitte warte kurz.", "retry_after": secs }));
    (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, secs.to_string())], body).into_response()
}

fn core_result(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) if e == api::DENIED => text(StatusCode::FORBIDDEN, e),
        Err(e) => text(StatusCode::BAD_REQUEST, e),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("Hintergrundaufgabe abgestürzt")
}

/// Client-IP vom Socket. `X-Forwarded-For` nur, wenn ein Proxy auf demselben Rechner davor sitzt.
fn client_ip(s: &Srv, peer: SocketAddr, h: &HeaderMap) -> IpAddr {
    let ip = peer.ip().to_canonical();
    if s.behind_proxy && ip.is_loopback() {
        let fwd = h.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit(',').next()).and_then(|v| v.trim().parse::<IpAddr>().ok());
        if let Some(f) = fwd {
            return f.to_canonical();
        }
    }
    ip
}

fn host(h: &HeaderMap, uri: &Uri) -> String {
    let v = h.get(header::HOST).and_then(|v| v.to_str().ok()).map(String::from);
    v.or_else(|| uri.authority().map(|a| a.to_string())).unwrap_or_else(|| "localhost".into())
}

/// Token aus dem Cookie (Browser) oder `Authorization: Bearer` (Apps).
fn token_of(h: &HeaderMap) -> Option<String> {
    let bearer = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    let cookie = || {
        h.get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .find_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('='))
    };
    bearer.or_else(cookie).filter(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())).map(String::from)
}

async fn current(s: &Arc<Srv>, h: &HeaderMap) -> Option<(auth::Session, String)> {
    let token = token_of(h)?;
    let s = s.clone();
    blocking(move || auth::lookup(&s.core.db.lock().unwrap(), &token).map(|x| (x, token))).await
}

fn session_cookie(token: &str) -> String {
    format!("{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={}", auth::SESSION_MAX)
}

// ---------- Wächter & Befehle ----------

async fn rpc(State(s): State<Arc<Srv>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, Path(cmd): Path<String>, req: Request) -> Response {
    let (h, uri) = (req.headers().clone(), req.uri().clone());
    // Gegen Cross-Site-Anfragen: nur JSON (Formulare können das nicht senden) und, falls angegeben, eigene Herkunft.
    let is_json = h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return text(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Erwartet: Content-Type application/json");
    }
    let origin_ok = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()).map_or(true, |o| {
        o.split_once("://").is_some_and(|(_, rest)| rest == host(&h, &uri))
    });
    if !origin_ok {
        return text(StatusCode::FORBIDDEN, "Anfrage von fremder Herkunft");
    }
    let ip = client_ip(&s, peer, &h);
    let public = matches!(cmd.as_str(), "session" | "login" | "setup" | "join" | "reset_password");

    // Ohne Sitzung wird nur ein kleiner Body gelesen; große PDFs nur von angemeldeten Konten.
    let sess = if public { None } else { current(&s, &h).await };
    if !public && sess.is_none() {
        return text(StatusCode::UNAUTHORIZED, "Bitte melde dich an.");
    }
    let limit = match (public, cmd.as_str()) {
        (true, _) => 16 * 1024,
        (_, "import_document" | "replace_document") => 64 * 1024 * 1024,
        // eigene Aufgabe mit Foto (data:-URL, bis 2 MB im Kern)
        (_, "create_question" | "update_question") => 4 * 1024 * 1024,
        _ => 1024 * 1024,
    };
    let Ok(body) = to_bytes(req.into_body(), limit).await else {
        return text(StatusCode::PAYLOAD_TOO_LARGE, "Die Anfrage ist zu groß.");
    };
    let args: Value = if body.is_empty() { json!({}) } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return text(StatusCode::BAD_REQUEST, "Ungültige Anfrage"),
        }
    };

    if cmd == "session" {
        let user = current(&s, &h).await.map(|(x, _)| json!({ "name": x.name, "admin": x.admin }));
        let setup = s.setup_token.lock().unwrap().is_some();
        return Json(json!({ "mode": "server", "user": user, "setup": setup })).into_response();
    }
    if public {
        return sign_in(&s, &cmd, args, ip, &h).await;
    }
    let (sess, _) = sess.unwrap();
    let who = Who { id: sess.user_id, admin: sess.admin };
    match cmd.as_str() {
        "logout" | "change_password" | "sessions_list" | "session_revoke" | "sessions_revoke_others" | "members" | "member_disable" | "invite_create"
        | "connect_link" => account(&s, &cmd, args, sess, ip, &h, &uri).await,
        "explain" | "ask" => stream_call(&s, who, cmd, args),
        _ => {
            let core = s.core.clone();
            core_result(blocking(move || api::call(&core, &who, &cmd, args, &mut |_| {})).await)
        }
    }
}

/// `explain`/`ask`: NDJSON, je Zeile `{"t": Stück}`, zum Schluss `{"ok": Ergebnis}` oder `{"err": Meldung}`.
fn stream_call(s: &Arc<Srv>, who: Who, cmd: String, args: Value) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let core = s.core.clone();
    tokio::task::spawn_blocking(move || {
        let res = api::call(&core, &who, &cmd, args, &mut |p| {
            let _ = tx.send(format!("{}\n", json!({ "t": p })));
        });
        let last = match res {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "err": e }),
        };
        let _ = tx.send(format!("{last}\n"));
    });
    let body = Body::from_stream(stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|l| (Ok::<_, Infallible>(l), rx)) }));
    ([(header::CONTENT_TYPE, "application/x-ndjson")], body).into_response()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SignIn {
    name: String,
    password: String,
    code: String,
}

/// Fehlschlag beim Anmelden: `Wrong` zählt für den Sperr-Timer, `Invalid` (z. B. zu kurzes neues Passwort) nicht.
enum Fail {
    Wrong(&'static str),
    Invalid(String),
}

/// Anmelden, Einrichten, Beitreten, neues Passwort. Ablauf:
/// 1. kommt per POST über HTTPS (der Server spricht kein Klartext-HTTP nach außen)
/// 2. Sperre für (IP, Name), IP oder Name aktiv? → sofort 429, ohne Hash-Rechnung
/// 3. Argon2id-Abgleich (ohne Konto gegen einen Dummy-Hash: gleiche Zeit, gleiche Meldung)
/// 4. Erfolg: Zähler zurück, neue Sitzung, Cookie. Fehler: Zähler hoch, Timer, Log ohne Passwort.
async fn sign_in(s: &Arc<Srv>, cmd: &str, args: Value, ip: IpAddr, h: &HeaderMap) -> Response {
    let a: SignIn = serde_json::from_value(args).unwrap_or_default();
    // Codes aus Links werden pro IP gezählt, Passwörter pro (IP, Name)
    let key = if cmd == "login" { a.name.clone() } else { format!("#{cmd}") };
    if let Some(secs) = s.limiter.lock().unwrap().locked(ip, &key, Instant::now()) {
        return locked(secs);
    }
    let Ok(_permit) = s.hashing.acquire().await else { return text(StatusCode::SERVICE_UNAVAILABLE, "Server fährt herunter") };
    let label = auth::device_label(h.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or(""));
    let setup = s.setup_token.lock().unwrap().clone();
    let (s2, cmd2, ip_s) = (s.clone(), cmd.to_string(), ip.to_string());
    let res = blocking(move || -> Result<(i64, String), Fail> {
        let db = &s2.core.db;
        let user_id = match cmd2.as_str() {
            "login" => {
                let row: Option<(i64, Option<String>, bool)> = db
                    .lock()
                    .unwrap()
                    .query_row("SELECT id, pw_hash, disabled FROM users WHERE name = ?1", [a.name.trim()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .optional()
                    .unwrap_or(None);
                let stored = row.as_ref().filter(|r| !r.2).and_then(|r| r.1.clone());
                if !auth::verify_password(&a.password, stored.as_deref()) {
                    return Err(Fail::Wrong("Name oder Passwort stimmt nicht."));
                }
                let (id, ..) = row.unwrap();
                if stored.as_deref().is_some_and(auth::needs_rehash) {
                    let _ = db.lock().unwrap().execute("UPDATE users SET pw_hash = ?1 WHERE id = ?2", params![auth::hash_password(&a.password), id]);
                }
                id
            }
            "setup" => {
                let ok = setup.is_some_and(|t| auth::sha256_hex(t.as_bytes()) == auth::sha256_hex(a.code.as_bytes()));
                if !ok {
                    return Err(Fail::Wrong("Dieser Einrichtungslink ist ungültig oder schon benutzt."));
                }
                let name = auth::check_name(&a.name).map_err(|e| Fail::Invalid(e.into()))?;
                auth::check_password(&a.password, name).map_err(|e| Fail::Invalid(e.into()))?;
                let hash = auth::hash_password(&a.password);
                // Nutzer 1 ist der Besitzer aller Kurse aus einer mitgebrachten lokalen Datenbank.
                db.lock()
                    .unwrap()
                    .execute(
                        "INSERT INTO users(id, name, pw_hash, admin, created_at) VALUES(1, ?1, ?2, 1, ?3)
                         ON CONFLICT(id) DO UPDATE SET name = excluded.name, pw_hash = excluded.pw_hash, admin = 1, disabled = 0",
                        params![name, hash, db::now()],
                    )
                    .map_err(|e| Fail::Invalid(e.to_string()))?;
                *s2.setup_token.lock().unwrap() = None;
                1
            }
            _ => {
                // join (neues Konto) und reset_password (Passwort-Link für ein bestehendes Konto)
                let code_hash = auth::sha256_hex(a.code.as_bytes());
                let invite: Option<Option<i64>> = db
                    .lock()
                    .unwrap()
                    .query_row("SELECT user_id FROM invites WHERE code_hash = ?1 AND used_at IS NULL AND expires_at > ?2", params![code_hash, db::now()], |r| r.get(0))
                    .optional()
                    .unwrap_or(None);
                let target = match (cmd2.as_str(), invite) {
                    ("join", Some(None)) => None,
                    ("reset_password", Some(Some(id))) => Some(id),
                    _ => return Err(Fail::Wrong("Dieser Link ist ungültig, abgelaufen oder schon benutzt.")),
                };
                let name = match target {
                    None => auth::check_name(&a.name).map_err(|e| Fail::Invalid(e.into()))?.to_string(),
                    Some(id) => db.lock().unwrap().query_row("SELECT name FROM users WHERE id = ?1", [id], |r| r.get(0)).map_err(|e| Fail::Invalid(e.to_string()))?,
                };
                auth::check_password(&a.password, &name).map_err(|e| Fail::Invalid(e.into()))?;
                let hash = auth::hash_password(&a.password);
                let mut conn = db.lock().unwrap();
                let tx = conn.transaction().map_err(|e| Fail::Invalid(e.to_string()))?;
                let id = match target {
                    None => {
                        tx.execute("INSERT INTO users(name, pw_hash, created_at) VALUES(?1, ?2, ?3)", params![name, hash, db::now()])
                            .map_err(|_| Fail::Invalid("Diesen Namen gibt es auf dem Server schon.".into()))?;
                        tx.last_insert_rowid()
                    }
                    Some(id) => {
                        tx.execute("UPDATE users SET pw_hash = ?1 WHERE id = ?2", params![hash, id]).map_err(|e| Fail::Invalid(e.to_string()))?;
                        tx.execute("DELETE FROM sessions WHERE user_id = ?1", [id]).map_err(|e| Fail::Invalid(e.to_string()))?;
                        id
                    }
                };
                tx.execute("UPDATE invites SET used_at = ?1 WHERE code_hash = ?2", params![db::now(), code_hash]).map_err(|e| Fail::Invalid(e.to_string()))?;
                tx.commit().map_err(|e| Fail::Invalid(e.to_string()))?;
                id
            }
        };
        let token = auth::create_session(&db.lock().unwrap(), user_id, &label, &ip_s).map_err(|e| Fail::Invalid(e.to_string()))?;
        Ok((user_id, token))
    })
    .await;

    match res {
        Ok((_, token)) => {
            s.limiter.lock().unwrap().success(ip, &key);
            // Das Token selbst bekommt nur die App (für den Bearer-Header); der Browser hat es nur im HttpOnly-Cookie.
            let app = h.get("x-lernpfad-client").is_some_and(|v| v == "app");
            let body = if app { json!({ "ok": true, "token": token }) } else { json!({ "ok": true }) };
            ([(header::SET_COOKIE, session_cookie(&token))], Json(body)).into_response()
        }
        Err(Fail::Wrong(msg)) => {
            eprintln!("[anmeldung] fehlgeschlagen: {cmd} name={:?} ip={ip}", head(&key, 64));
            match s.limiter.lock().unwrap().fail(ip, &key, Instant::now()) {
                Some(secs) => locked(secs),
                None => text(StatusCode::BAD_REQUEST, msg),
            }
        }
        Err(Fail::Invalid(msg)) => text(StatusCode::BAD_REQUEST, msg),
    }
}

fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map(|(i, _)| &s[..i]).unwrap_or(s)
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AccountArgs {
    id: i64,
    disabled: bool,
    user_id: Option<i64>,
    current: String,
    password: String,
}

/// Konto, Geräte und (nur Admin) Mitglieder.
async fn account(s: &Arc<Srv>, cmd: &str, args: Value, sess: auth::Session, ip: IpAddr, h: &HeaderMap, uri: &Uri) -> Response {
    let a: AccountArgs = serde_json::from_value(args).unwrap_or_default();
    let admin_only = matches!(cmd, "members" | "member_disable" | "invite_create");
    if admin_only && !sess.admin {
        return text(StatusCode::FORBIDDEN, api::DENIED);
    }
    let base = format!("{}://{}", s.scheme, host(h, uri));

    if cmd == "change_password" {
        if let Some(secs) = s.limiter.lock().unwrap().locked(ip, &sess.name, Instant::now()) {
            return locked(secs);
        }
        let Ok(_permit) = s.hashing.acquire().await else { return text(StatusCode::SERVICE_UNAVAILABLE, "") };
        let s2 = s.clone();
        let (uid, sid, name) = (sess.user_id, sess.id, sess.name.clone());
        let res = blocking(move || -> Result<(), Fail> {
            let db = &s2.core.db;
            let stored: Option<String> = db.lock().unwrap().query_row("SELECT pw_hash FROM users WHERE id = ?1", [uid], |r| r.get(0)).unwrap_or(None);
            if !auth::verify_password(&a.current, stored.as_deref()) {
                return Err(Fail::Wrong("Das aktuelle Passwort stimmt nicht."));
            }
            auth::check_password(&a.password, &name).map_err(|e| Fail::Invalid(e.into()))?;
            let hash = auth::hash_password(&a.password);
            let conn = db.lock().unwrap();
            conn.execute("UPDATE users SET pw_hash = ?1 WHERE id = ?2", params![hash, uid]).map_err(|e| Fail::Invalid(e.to_string()))?;
            conn.execute("DELETE FROM sessions WHERE user_id = ?1 AND id != ?2", params![uid, sid]).map_err(|e| Fail::Invalid(e.to_string()))?;
            Ok(())
        })
        .await;
        return match res {
            Ok(()) => {
                s.limiter.lock().unwrap().success(ip, &sess.name);
                Json(Value::Null).into_response()
            }
            Err(Fail::Wrong(msg)) => match s.limiter.lock().unwrap().fail(ip, &sess.name, Instant::now()) {
                Some(secs) => locked(secs),
                None => text(StatusCode::BAD_REQUEST, msg),
            },
            Err(Fail::Invalid(msg)) => text(StatusCode::BAD_REQUEST, msg),
        };
    }

    let logout = cmd == "logout";
    let (s2, cmd, pin) = (s.clone(), cmd.to_string(), s.pin.clone());
    let res = blocking(move || -> Result<Value, String> {
        let conn = s2.core.db.lock().unwrap();
        let e = |e: rusqlite::Error| e.to_string();
        let me = sess.user_id;
        Ok(match cmd.as_str() {
            "logout" => {
                conn.execute("DELETE FROM sessions WHERE id = ?1", [sess.id]).map_err(e)?;
                Value::Null
            }
            "sessions_list" => {
                let mut st = conn
                    .prepare("SELECT id, label, ip, last_seen, id = ?2 FROM sessions WHERE user_id = ?1 AND expires_at > ?3 ORDER BY id = ?2 DESC, last_seen DESC")
                    .map_err(e)?;
                let rows = st
                    .query_map(params![me, sess.id, db::now()], |r| {
                        Ok(json!({ "id": r.get::<_, i64>(0)?, "label": r.get::<_, String>(1)?, "ip": r.get::<_, String>(2)?, "last_seen": r.get::<_, i64>(3)?, "current": r.get::<_, bool>(4)? }))
                    })
                    .map_err(e)?;
                Value::Array(rows.collect::<Result<_, _>>().map_err(e)?)
            }
            "session_revoke" => json!(conn.execute("DELETE FROM sessions WHERE id = ?1 AND user_id = ?2", params![a.id, me]).map_err(e)?),
            "sessions_revoke_others" => json!(conn.execute("DELETE FROM sessions WHERE user_id = ?1 AND id != ?2", params![me, sess.id]).map_err(e)?),
            "members" => {
                let mut st = conn.prepare("SELECT id, name, admin, disabled FROM users WHERE pw_hash IS NOT NULL ORDER BY name COLLATE NOCASE").map_err(e)?;
                let rows = st
                    .query_map([], |r| {
                        let id: i64 = r.get(0)?;
                        Ok(json!({ "id": id, "name": r.get::<_, String>(1)?, "admin": r.get::<_, bool>(2)?, "disabled": r.get::<_, bool>(3)?, "me": id == me }))
                    })
                    .map_err(e)?;
                Value::Array(rows.collect::<Result<_, _>>().map_err(e)?)
            }
            "member_disable" => {
                if a.id == me {
                    return Err("Du kannst dich nicht selbst sperren.".into());
                }
                conn.execute("UPDATE users SET disabled = ?1 WHERE id = ?2", params![a.disabled, a.id]).map_err(e)?;
                if a.disabled {
                    conn.execute("DELETE FROM sessions WHERE user_id = ?1", [a.id]).map_err(e)?;
                }
                Value::Null
            }
            "invite_create" => {
                let code = auth::random_hex(32);
                let exists = a.user_id.map(|id| conn.query_row("SELECT COUNT(*) FROM users WHERE id = ?1 AND pw_hash IS NOT NULL", [id], |r| r.get::<_, i64>(0)));
                if let Some(Ok(0)) | Some(Err(_)) = exists {
                    return Err(api::DENIED.into());
                }
                conn.execute(
                    "INSERT INTO invites(code_hash, created_by, user_id, expires_at) VALUES(?1, ?2, ?3, ?4)",
                    params![auth::sha256_hex(code.as_bytes()), me, a.user_id, db::now() + auth::INVITE_TTL],
                )
                .map_err(e)?;
                let route = if a.user_id.is_some() { "reset" } else { "join" };
                json!({ "link": format!("{base}/#/{route}/{code}"), "expires_at": db::now() + auth::INVITE_TTL })
            }
            "connect_link" => json!({ "link": match pin { Some(p) => format!("{base}/#pin={p}"), None => format!("{base}/") } }),
            _ => unreachable!(),
        })
    })
    .await;
    match res {
        Ok(v) if logout => ([(header::SET_COOKIE, format!("{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"))], Json(v)).into_response(),
        r => core_result(r),
    }
}

/// Öffentliches Zertifikat für die Apps. Vertraut wird ihm dort erst, wenn es zum Fingerabdruck im Verbindungslink passt.
async fn cert(State(s): State<Arc<Srv>>) -> Response {
    match &s.cert_pem {
        Some(pem) => ([(header::CONTENT_TYPE, "application/x-pem-file")], pem.clone()).into_response(),
        None => text(StatusCode::NOT_FOUND, "Dieser Server nutzt ein öffentlich gültiges Zertifikat."),
    }
}

// ---------- Ereignisse (Server-Sent Events) ----------

enum Check {
    Send,
    Skip,
    End,
}

async fn events_stream(State(s): State<Arc<Srv>>, h: HeaderMap) -> Response {
    let Some((_, token)) = current(&s, &h).await else { return text(StatusCode::UNAUTHORIZED, "Bitte melde dich an.") };
    let rx = s.events.subscribe();
    let st = stream::unfold((rx, s, token), |(mut rx, s, token)| async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            // Bei jedem Ereignis neu prüfen: abgemeldet oder gesperrt → Strom endet; fremder Kurs → nicht senden.
            let (s2, t2, course) = (s.clone(), token.clone(), ev.course);
            let check = blocking(move || {
                let conn = s2.core.db.lock().unwrap();
                let Some(x) = auth::lookup(&conn, &t2) else { return Check::End };
                match course {
                    Some(c) if c != 0 && api::role(&conn, &Who { id: x.user_id, admin: x.admin }, c).is_err() => Check::Skip,
                    _ => Check::Send,
                }
            })
            .await;
            match check {
                Check::End => return None,
                Check::Skip => continue,
                Check::Send => return Some((Ok::<_, Infallible>(Event::default().event(ev.name).data(ev.data)), (rx, s, token))),
            }
        }
    });
    Sse::new(st).keep_alive(KeepAlive::default()).into_response()
}

// ---------- Sicherheitsheader ----------

async fn secure_headers(State(s): State<Arc<Srv>>, req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    // Nur mit echtem Zertifikat: nach einer Warnung beim eigenen Zertifikat käme man sonst nie mehr auf die Seite.
    if s.hsts {
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    }
    res
}
