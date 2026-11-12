//! Lernlogik ohne KI: Antwortprüfung, Wiederholungs-Boxen (Leitner), XP, Streak.

/// Tage bis zur nächsten Wiederholung je Box (Box 0 = neu).
const INTERVAL_DAYS: [i64; 6] = [0, 1, 3, 7, 14, 30];
pub const MAX_BOX: i64 = 5;
const DAY: i64 = 86_400;

/// Vergleichsform: klein, Umlaute ausgeschrieben, Satzzeichen am Rand weg, Leerraum zusammengefasst.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            _ => out.push(c),
        }
    }
    let out = out.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Prüft eine Antwort. Lückentext toleriert ab 5 Buchstaben einen Tippfehler.
pub fn check(kind: &str, expected: &str, given: &str) -> bool {
    let (e, g) = (normalize(expected), normalize(given));
    if e.is_empty() || g.is_empty() {
        return false;
    }
    e == g || (kind == "fill_blank" && e.chars().count() >= 5 && levenshtein(&e, &g) <= 1)
}

/// Neue Box nach einer Antwort: richtig → eine Box höher, falsch → zurück auf 1.
pub fn next_box(current: i64, correct: bool) -> i64 {
    if correct {
        (current + 1).min(MAX_BOX)
    } else {
        1
    }
}

/// Zeitpunkt (Unix-Sekunden), ab dem die Frage wieder fällig ist.
pub fn due_at(boxed: i64, now: i64) -> i64 {
    now + INTERVAL_DAYS[boxed.clamp(0, MAX_BOX) as usize] * DAY
}

/// XP einer Lektion: +2 je richtiger Aufgabe, +5 fürs Abschließen, +5 wenn fehlerfrei.
pub fn lesson_xp(correct: i64, total: i64) -> i64 {
    let perfect = if total > 0 && correct >= total { 5 } else { 0 };
    2 * correct + 5 + perfect
}

/// Streak = Anzahl aufeinanderfolgender Lerntage, die heute oder gestern endet.
/// `days` = Tagesnummern (beliebige Reihenfolge, Duplikate erlaubt).
pub fn streak(days: &[i64], today: i64) -> i64 {
    let mut d: Vec<i64> = days.to_vec();
    d.sort_unstable_by(|a, b| b.cmp(a));
    d.dedup();
    let Some(&first) = d.first() else { return 0 };
    if first < today - 1 {
        return 0;
    }
    let mut n = 1;
    for w in d.windows(2) {
        if w[0] - w[1] != 1 {
            break;
        }
        n += 1;
    }
    n
}

/// Kleiner Zufallsgenerator zum Mischen (kein Krypto, nur Reihenfolge).
// ponytail: xorshift statt rand-Crate; reicht zum Mischen von 10 Aufgaben.
pub struct Shuffler(u64);

impl Shuffler {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9);
        Shuffler(seed | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_answers() {
        assert!(check("mc", "Angebot steigt", "  angebot steigt. "));
        assert!(check("true_false", "Richtig", "richtig"));
        assert!(!check("mc", "Angebot steigt", "Angebot sinkt"));
        assert!(check("fill_blank", "Größe", "groesse"));
        assert!(!check("fill_blank", "Variable", "Varaible")); // Buchstabendreher = Distanz 2
        assert!(check("fill_blank", "Variable", "Varible"));
        assert!(!check("fill_blank", "Bit", "Bot")); // kurze Wörter: exakt
        assert!(!check("mc", "", ""));
    }

    #[test]
    fn leitner_boxes() {
        assert_eq!(next_box(0, true), 1);
        assert_eq!(next_box(5, true), 5);
        assert_eq!(next_box(4, false), 1);
        assert_eq!(due_at(0, 100), 100);
        assert_eq!(due_at(3, 0), 7 * DAY);
        assert_eq!(due_at(9, 0), 30 * DAY);
    }

    #[test]
    fn xp_and_streak() {
        assert_eq!(lesson_xp(10, 10), 30);
        assert_eq!(lesson_xp(7, 10), 19);
        assert_eq!(streak(&[], 100), 0);
        assert_eq!(streak(&[100, 99, 99, 98, 96], 100), 3);
        assert_eq!(streak(&[99, 98], 100), 2); // gestern zählt noch
        assert_eq!(streak(&[98, 97], 100), 0); // vorgestern = gerissen
    }

    #[test]
    fn shuffle_keeps_items() {
        let mut v: Vec<i32> = (0..20).collect();
        Shuffler::new().shuffle(&mut v);
        v.sort();
        assert_eq!(v, (0..20).collect::<Vec<_>>());
    }
}
