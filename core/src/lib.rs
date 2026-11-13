//! Lernpfad-Kern: Datenbank, KI, Import und Lernlogik. Tauri-App und Server sind nur Hüllen um `api::call`.

pub mod ai;
pub mod api;
pub mod db;
pub mod import;
pub mod learn;
pub mod report;

use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};

pub use ai::Ai;

/// Wer einen Befehl ausführt. Lokal immer [`LOCAL`]; auf dem Server der angemeldete Nutzer.
#[derive(Clone, Copy, Debug)]
pub struct Who {
    pub id: i64,
    pub admin: bool,
}

pub const LOCAL: Who = Who { id: 1, admin: true };

pub struct Core {
    pub db: db::Db,
    pub ai: Arc<Ai>,
    wake: mpsc::Sender<()>,
}

impl Core {
    /// Öffnet `dir/lernen.db`, prüft das Modell (lädt es bei Bedarf herunter) und startet den Hintergrund-Worker.
    pub fn open(dir: &Path, on_ai: impl Fn(&ai::Status) + Send + Sync + 'static, on_course: impl Fn(i64) + Send + 'static) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let conn = db::open(&dir.join("lernen.db")).map_err(|e| e.to_string())?;
        let tier = db::setting(&conn, "tier").unwrap_or_else(|| ai::default_tier().into());
        let db: db::Db = Arc::new(Mutex::new(conn));
        let (wake, wake_rx) = mpsc::channel();
        let wake_on_ready = wake.clone();
        let ai = Ai::new(
            dir.join("models"),
            &tier,
            Box::new(move |s| {
                on_ai(s);
                if s.state == ai::State::Ready {
                    let _ = wake_on_ready.send(());
                }
            }),
        );
        ai.ensure_model();
        import::spawn_worker(db.clone(), ai.clone(), wake_rx, on_course);
        Ok(Core { db, ai, wake })
    }

    pub fn wake(&self) {
        let _ = self.wake.send(());
    }
}
