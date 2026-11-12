# Lernpfad

Lern-App für die Prüfungsvorbereitung. Aus deinen PDF-Skripten entstehen kurze Lektionen mit Übungsfragen, ein Lernpfad, Wiederholungen und Streaks. Die KI läuft **komplett auf dem Gerät**: keine Cloud, kein Konto, nach dem ersten Start offline.

Läuft auf Linux, Windows, macOS, Android, iOS und iPadOS (eine Codebasis, [Tauri 2](https://v2.tauri.app)).

## So funktioniert es

1. **Hinzufügen:** PDF auswählen. Der Text wird auf dem Gerät gelesen (pdf.js) und in Abschnitte von etwa 5000 Zeichen geteilt. So wird das ganze Dokument abgedeckt.
2. **Vorbereiten (KI, im Hintergrund):** Pro Abschnitt erzeugt die KI einen Titel, eine kurze Zusammenfassung und die wichtigsten Begriffe. Fragen entstehen erst, wenn eine Einheit dran ist; die nächste wird vorab vorbereitet.
3. **Lernen (ohne KI):** Multiple Choice, Richtig/Falsch, Lückentext und Begriffe zuordnen. Die Antworten werden im Code geprüft, nicht von der KI. Falsche Aufgaben kommen am Ende der Lektion erneut.
4. **Wiederholen:** Leitner-Boxen (1, 3, 7, 14, 30 Tage). Fällige Fragen landen im Tab „Wiederholen“.
5. **Motivation:** XP, Tagesziel, Streak, Lernstufe je Einheit.

Die KI wird nur gebraucht für: Import, „Ausführlich erklären“ (einmal erzeugt, danach gespeichert), „Frag die KI“ und „Warum?“ nach einer falschen Antwort.

## KI auf dem Gerät

| Stufe | Modell | Größe | Standard auf |
| --- | --- | --- | --- |
| Kompakt | Qwen3.5 2B (Q4_K_M) | 1,3 GB | Android, iOS |
| Ausgewogen | Qwen3.5 4B (Q4_K_M) | 2,7 GB | Linux, Windows, macOS |

- llama.cpp ist als Bibliothek in die App eingebaut (`llama-cpp-2`). Nur so läuft KI auch auf iPhone und iPad, weil iOS keine nachgeladenen Programme erlaubt.
- Bei jedem Start prüft die App das Modell und lädt es bei Bedarf herunter, auch abgebrochene Downloads werden fortgesetzt. Die URLs sind auf eine feste Hugging-Face-Revision gepinnt und die Dateigröße wird geprüft.
- Das Modell wird erst geladen, wenn es gebraucht wird, und nach 3 Minuten Leerlauf wieder aus dem Speicher entfernt.
- Sparsame Einstellungen: 4096 Token Kontext, KV-Cache in 8 Bit, alle Schichten auf der GPU (Vulkan unter Linux/Windows, Metal auf Apple-Geräten, CPU auf Android).
- Strukturierte Ausgabe per JSON-Schema-Grammatik: Auch kleine Modelle liefern immer gültiges JSON.
- Die Stufe lässt sich unter **Profil → KI auf diesem Gerät** wechseln.

Speicherort der Modelle und der Datenbank (`lernen.db`): der App-Datenordner, unter Linux `~/.local/share/de.lernpfad.app/`.

## Entwickeln

### Voraussetzungen

- Rust über [rustup](https://rustup.rs) und die Tauri-CLI: `cargo install tauri-cli --version "^2" --locked`
- Linux (Debian/Ubuntu/Mint):
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev \
    libxdo-dev cmake libclang-dev libvulkan-dev glslc
  ```
- macOS: Xcode (für iOS zusätzlich die iOS-Plattform in Xcode)
- Windows: Visual Studio Build Tools mit C++, CMake und das Vulkan SDK
- Android: Android Studio mit SDK und NDK, JDK 17

### Starten

```bash
cd src-tauri
cargo tauri dev          # Desktop-App mit Neuladen
cargo test               # Tests für Lernlogik und Import
cargo tauri build        # Installationspaket für dieses System
```

Mobil:

```bash
cargo tauri android init && cargo tauri android dev
cargo tauri ios init && cargo tauri ios dev      # nur auf einem Mac
```

Für iPhone/iPad brauchst du eine Apple-ID in Xcode (kostenlos: App läuft 7 Tage, Apple Developer Program: dauerhaft und TestFlight). Auf iOS läuft der Import nur, solange die App geöffnet ist.

### Oberfläche ohne Tauri

Die Oberfläche in `web/` kommt ohne Build-Schritt aus. Im Browser ersetzt `web/dev/mock-tauri.js` den Rust-Kern durch Beispieldaten:

```bash
python3 -m http.server 8765 -d web
# http://127.0.0.1:8765/            KI lädt simuliert
# http://127.0.0.1:8765/?ai=ready   KI sofort bereit
# http://127.0.0.1:8765/?empty      ohne Kurse
```

## Aufbau

```
src-tauri/src/
  lib.rs      Tauri-Befehle für die Oberfläche
  ai.rs       Modell-Download, Laden/Entladen, Text erzeugen (llama.cpp)
  import.rs   Abschnitte, Prompts, Qualitätsfilter für Fragen, Hintergrund-Worker
  learn.rs    Antwortprüfung, Leitner-Boxen, XP, Streak (ohne KI)
  db.rs       SQLite-Schema
web/
  index.html, css/app.css, js/*.js   Oberfläche (Vanilla-JS-Module)
  vendor/                            pdf.js, marked
  fonts/, icons.svg                  Bricolage Grotesque, Figtree, Phosphor Icons
```

## Datenschutz

Alles bleibt auf dem Gerät: Dokumente, Fortschritt und Chats. Die App geht nur ins Internet, um einmalig das KI-Modell von Hugging Face zu laden.
