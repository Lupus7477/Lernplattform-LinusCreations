//! Anmeldung: Argon2id-Passwörter, Sitzungstokens (in der DB nur als SHA-256), Passwortregeln und Sperr-Timer.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use lernpfad_core::db::now;

// OWASP-Empfehlung für Argon2id: 19 MiB Speicher, 2 Durchläufe, 1 Thread.
const M_COST: u32 = 19 * 1024;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

/// Ohne Nutzung abgemeldet nach 14 Tagen, spätestens nach 60 Tagen neu anmelden.
pub const SESSION_IDLE: i64 = 14 * 86_400;
pub const SESSION_MAX: i64 = 60 * 86_400;
pub const INVITE_TTL: i64 = 7 * 86_400;

fn argon() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(M_COST, T_COST, P_COST, None).expect("Argon2-Parameter"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `n` Zufallsbytes aus dem Zufallsgenerator des Betriebssystems, als Hex.
pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("Zufallsgenerator des Betriebssystems");
    hex(&b)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub fn hash_password(pw: &str) -> String {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("Zufallsgenerator des Betriebssystems");
    let salt = SaltString::encode_b64(&salt).expect("Salt");
    argon().hash_password(pw.as_bytes(), &salt).expect("Argon2").to_string()
}

static DUMMY: OnceLock<String> = OnceLock::new();

/// Beim Start einmal rechnen, damit schon die erste Anmeldung ohne Konto gleich lange dauert.
pub fn warm_up() {
    DUMMY.get_or_init(|| hash_password("kein Konto: Vergleichswert für gleiche Rechenzeit"));
}

/// Prüft ein Passwort. Ohne Konto (`None`) wird gegen einen festen Hash gerechnet:
/// gleiche Antwortzeit, also kein Hinweis darauf, ob es den Namen gibt.
pub fn verify_password(pw: &str, phc: Option<&str>) -> bool {
    warm_up();
    let (phc, real) = match phc {
        Some(p) => (p, true),
        None => (DUMMY.get().unwrap().as_str(), false),
    };
    let ok = PasswordHash::new(phc).map(|h| argon().verify_password(pw.as_bytes(), &h).is_ok()).unwrap_or(false);
    ok && real
}

/// Gespeicherter Hash mit älteren Parametern → beim nächsten Login neu hashen.
pub fn needs_rehash(phc: &str) -> bool {
    let Ok(h) = PasswordHash::new(phc) else { return true };
    let Ok(p) = Params::try_from(&h) else { return true };
    h.algorithm != Algorithm::Argon2id.ident() || p.m_cost() != M_COST || p.t_cost() != T_COST || p.p_cost() != P_COST
}

// ---------- Regeln für Namen und Passwörter ----------

pub fn check_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    let n = name.chars().count();
    if !(2..=32).contains(&n) {
        return Err("Der Name braucht 2 bis 32 Zeichen.");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || " .-_".contains(c)) {
        return Err("Im Namen gehen nur Buchstaben, Ziffern, Leerzeichen und . - _");
    }
    Ok(name)
}

fn common() -> &'static HashSet<&'static str> {
    static LIST: OnceLock<HashSet<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| include_str!("common-passwords.txt").lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect())
}

/// NIST 800-63B: Länge statt Sonderzeichen-Pflicht, dafür bekannte Passwörter sperren (offline, eingebaute Liste).
pub fn check_password(pw: &str, name: &str) -> Result<(), &'static str> {
    let n = pw.chars().count();
    if n < 12 {
        return Err("Das Passwort braucht mindestens 12 Zeichen.");
    }
    if n > 256 {
        return Err("Das Passwort darf höchstens 256 Zeichen haben.");
    }
    let low = pw.to_lowercase();
    if low == name.trim().to_lowercase() {
        return Err("Das Passwort darf nicht dein Name sein.");
    }
    if low.chars().collect::<HashSet<_>>().len() < 4 {
        return Err("Das Passwort wiederholt zu wenige Zeichen. Nimm lieber ein paar Wörter.");
    }
    // „password123456“, „!!sommer2026“: bekanntes Wort mit Zahlen/Zeichen davor oder dahinter zählt auch.
    let filler = |c: char| c.is_ascii_digit() || "!?.,;:#*_-+=@$%&/".contains(c);
    let core = low.trim_matches(filler);
    if common().contains(low.as_str()) || (!core.is_empty() && common().contains(core)) {
        return Err("Dieses Passwort steht auf einer Liste geleakter Passwörter. Bitte wähle ein anderes.");
    }
    Ok(())
}

// ---------- Sitzungen ----------

pub struct Session {
    pub id: i64,
    pub user_id: i64,
    pub admin: bool,
    pub name: String,
}

/// Neue Sitzung; zurück kommt das Token für Cookie/Bearer. In der DB steht nur sein SHA-256.
pub fn create_session(conn: &Connection, user_id: i64, label: &str, ip: &str) -> rusqlite::Result<String> {
    let token = random_hex(32);
    let now = now();
    conn.execute("DELETE FROM sessions WHERE expires_at <= ?1 OR last_seen <= ?1 - ?2", params![now, SESSION_IDLE])?;
    conn.execute(
        "INSERT INTO sessions(token_hash, user_id, created_at, last_seen, expires_at, label, ip) VALUES(?1, ?2, ?3, ?3, ?4, ?5, ?6)",
        params![sha256_hex(token.as_bytes()), user_id, now, now + SESSION_MAX, label, ip],
    )?;
    Ok(token)
}

/// Gültige Sitzung zum Token: nicht abgelaufen, nicht zu lange ungenutzt, Konto nicht gesperrt.
pub fn lookup(conn: &Connection, token: &str) -> Option<Session> {
    let now = now();
    let (s, last_seen): (Session, i64) = conn
        .query_row(
            "SELECT s.id, s.user_id, u.admin, u.name, s.last_seen FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?1 AND s.expires_at > ?2 AND s.last_seen > ?2 - ?3 AND u.disabled = 0 AND u.pw_hash IS NOT NULL",
            params![sha256_hex(token.as_bytes()), now, SESSION_IDLE],
            |r| Ok((Session { id: r.get(0)?, user_id: r.get(1)?, admin: r.get(2)?, name: r.get(3)? }, r.get(4)?)),
        )
        .optional()
        .ok()
        .flatten()?;
    if now - last_seen > 3600 {
        let _ = conn.execute("UPDATE sessions SET last_seen = ?1 WHERE id = ?2", params![now, s.id]);
    }
    Some(s)
}

pub fn needs_setup(conn: &Connection) -> bool {
    conn.query_row("SELECT COUNT(*) FROM users WHERE pw_hash IS NOT NULL", [], |r| r.get::<_, i64>(0)).map(|n| n == 0).unwrap_or(true)
}

/// „Firefox · Linux“, „Lernpfad-App · Android“: genug, um die eigenen Geräte wiederzuerkennen.
pub fn device_label(ua: &str) -> String {
    let browser = if ua.contains("Lernpfad-App") {
        "Lernpfad-App"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome/") || ua.contains("Chromium/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = [("Android", "Android"), ("iPhone", "iPhone"), ("iPad", "iPad"), ("Windows", "Windows"), ("Mac OS X", "macOS"), ("Macintosh", "macOS"), ("Linux", "Linux")]
        .iter()
        .find(|(k, _)| ua.contains(k))
        .map(|(_, v)| *v);
    match os {
        Some(os) => format!("{browser} · {os}"),
        None => browser.into(),
    }
}

// ---------- Sperr-Timer ----------

const HOUR: Duration = Duration::from_secs(3600);
const MAX_ENTRIES: usize = 10_000;

#[derive(Default)]
struct Entry {
    fails: u32,
    since: Option<Instant>,
    last: Option<Instant>,
    until: Option<Instant>,
}

/// Fehlversuche pro (IP, Name), pro IP und pro Name. Gesperrt wird vor dem Hash-Vergleich.
// ponytail: im Speicher, ein Neustart setzt die Zähler zurück (neu starten kann nur der Admin). DB-Tabelle, falls das je nicht reicht.
#[derive(Default)]
pub struct Limiter {
    map: HashMap<String, Entry>,
}

/// IPv6: ganze /64 zählt als eine Adresse, sonst würde jede neue Adresse aus dem eigenen Netz neu anfangen.
fn ip_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
    }
}

impl Limiter {
    fn keys(ip: IpAddr, name: &str) -> [String; 3] {
        let (ip, name) = (ip_key(ip), name.trim().to_lowercase());
        [format!("p|{ip}|{name}"), format!("i|{ip}"), format!("n|{name}")]
    }

    /// Sekunden bis zum nächsten erlaubten Versuch, wenn gerade gesperrt.
    pub fn locked(&self, ip: IpAddr, name: &str, now: Instant) -> Option<u64> {
        Self::keys(ip, name)
            .iter()
            .filter_map(|k| self.map.get(k)?.until)
            .filter(|u| *u > now)
            .map(|u| u.saturating_duration_since(now).as_secs_f64().ceil() as u64)
            .max()
    }

    /// Fehlversuch zählen. Gibt die Sperrzeit zurück, falls jetzt eine gilt.
    pub fn fail(&mut self, ip: IpAddr, name: &str, now: Instant) -> Option<u64> {
        if self.map.len() > MAX_ENTRIES {
            self.map.retain(|_, e| e.until.is_some_and(|u| u > now) || e.last.is_some_and(|l| now.saturating_duration_since(l) < HOUR));
        }
        let [pair, by_ip, by_name] = Self::keys(ip, name);
        // (IP, Name): 3 Tippfehler frei, dann 30 s, verdoppelt je Fehler, höchstens 15 min. Nach 1 h Ruhe von vorn.
        let e = self.map.entry(pair).or_default();
        if e.last.is_some_and(|l| now.saturating_duration_since(l) > HOUR) {
            *e = Entry::default();
        }
        e.fails += 1;
        e.last = Some(now);
        if e.fails >= 4 {
            e.until = Some(now + Duration::from_secs((30u64 << (e.fails - 4).min(5)).min(900)));
        }
        // IP: 20 Fehler pro Stunde (viele Namen durchprobieren) → 1 h
        Self::windowed(self.map.entry(by_ip).or_default(), now, 20, 3600);
        // Name: 50 Fehler pro Stunde von überall (verteilt) → 15 min. Hoch angesetzt, damit niemand fremde Konten einfach aussperrt.
        Self::windowed(self.map.entry(by_name).or_default(), now, 50, 900);
        self.locked(ip, name, now)
    }

    fn windowed(e: &mut Entry, now: Instant, limit: u32, lock_secs: u64) {
        if e.since.map_or(true, |s| now.saturating_duration_since(s) > HOUR) {
            e.since = Some(now);
            e.fails = 0;
        }
        e.fails += 1;
        e.last = Some(now);
        if e.fails >= limit {
            e.until = Some(now + Duration::from_secs(lock_secs));
        }
    }

    /// Erfolgreiche Anmeldung: Zähler für (IP, Name) und Name zurücksetzen. Der IP-Zähler bleibt (Durchprobieren).
    pub fn success(&mut self, ip: IpAddr, name: &str) {
        let [pair, _, by_name] = Self::keys(ip, name);
        self.map.remove(&pair);
        self.map.remove(&by_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockout_timer() {
        let mut l = Limiter::default();
        let ip: IpAddr = "10.0.0.7".parse().unwrap();
        let t = Instant::now();
        for _ in 0..3 {
            assert_eq!(l.fail(ip, "anna", t), None, "drei Tippfehler sind frei");
        }
        assert_eq!(l.fail(ip, "anna", t), Some(30));
        assert_eq!(l.locked(ip, " Anna ", t), Some(30), "Name ohne Groß/Klein und Leerzeichen");
        assert_eq!(l.locked(ip, "ben", t), None, "anderer Name, gleiche IP: frei");
        let later = t + Duration::from_secs(31);
        assert_eq!(l.locked(ip, "anna", later), None);
        assert_eq!(l.fail(ip, "anna", later), Some(60), "verdoppelt");
        l.success(ip, "anna");
        assert_eq!(l.locked(ip, "anna", later), None);
        assert_eq!(l.fail(ip, "anna", later), None, "Zähler nach Erfolg wieder bei 0");

        // Viele Namen von einer IP: nach 20 Fehlern in der Stunde ist die IP gesperrt.
        let spray: IpAddr = "2001:db8::1".parse().unwrap();
        for i in 0..19 {
            assert_eq!(l.fail(spray, &format!("nutzer{i}"), t), None);
        }
        assert_eq!(l.fail(spray, "nutzer19", t), Some(3600));
        let same_net: IpAddr = "2001:db8::ffff".parse().unwrap();
        assert_eq!(l.locked(same_net, "wer-anders", t), Some(3600), "gleiches /64");
    }

    #[test]
    fn password_rules() {
        assert!(check_password("kurz", "anna").is_err());
        assert!(check_password("Password123456", "anna").is_err(), "bekanntes Wort + Ziffern");
        assert!(check_password("qwerty123456", "anna").is_err());
        assert!(check_password("aaaaaaaaaaaaaaaa", "anna").is_err());
        assert!(check_password("annaannaanna12", "annaannaanna12").is_err());
        assert!(check_password("!!sommer2026!!", "anna").is_err());
        assert!(check_password("Kaffee ist mein Lernmotor", "anna").is_ok());
        assert!(check_name("A").is_err());
        assert!(check_name("Robert'); DROP TABLE users;--").is_err());
        assert_eq!(check_name("  Jürgen M. ").unwrap(), "Jürgen M.");
    }

    #[test]
    fn hashes_and_tokens() {
        let h = hash_password("Kaffee ist mein Lernmotor");
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
        assert!(!needs_rehash(&h));
        assert!(verify_password("Kaffee ist mein Lernmotor", Some(&h)));
        assert!(!verify_password("Kaffee ist mein Lernmotor!", Some(&h)));
        assert!(!verify_password("Kaffee ist mein Lernmotor", None), "ohne Konto nie erfolgreich");
        let t = random_hex(32);
        assert_eq!(t.len(), 64);
        assert_ne!(sha256_hex(t.as_bytes()), t);
        assert_eq!(device_label("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"), "Firefox · Linux");
        assert_eq!(device_label("Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 Chrome/140 Mobile Safari/537.36"), "Chrome · Android");
        assert_eq!(device_label("Lernpfad-App (Android)"), "Lernpfad-App · Android");
    }
}
