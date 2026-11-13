# Lernpfad

Lern-App für die Prüfungsvorbereitung. Aus deinen PDF-Skripten entstehen kurze Lektionen mit Übungsfragen, ein Lernpfad, Wiederholungen und Streaks. Die KI läuft **auf deinem eigenen Gerät oder deinem eigenen Server**: keine Cloud, kein fremder Dienst.

## Vier Varianten, eine Codebasis

| Variante | Was | Daten & KI |
| --- | --- | --- |
| **PC-App** (Linux, Windows, macOS) | Tauri-App | auf diesem Gerät, offline |
| **Handy-App** (Android, iOS, iPadOS) | Tauri-App | auf diesem Gerät, offline |
| **Server** | `lernpfad-server`, selbst gehostet | auf deinem Server, mit Konten |
| **Web-App** | dieselbe Oberfläche im Browser, vom Server ausgeliefert | auf deinem Server |

Die Apps laufen **immer lokal**. Optional verbinden sie sich mit deinem Server (Profil → Eigener Server) und kommen mit „Zurück zu lokal“ jederzeit zurück. Die lokalen Daten bleiben dabei unberührt.

## So funktioniert es

1. **Hinzufügen:** PDF auswählen. Der Text wird im Gerät bzw. Browser gelesen (pdf.js) und in Abschnitte von etwa 5000 Zeichen geteilt. Dabei werden auch Grafiken gesucht (siehe unten).
2. **Vorbereiten (KI, im Hintergrund):** Pro Abschnitt sortiert die KI zuerst Seiten mit reinem Organisatorischem aus (siehe unten). Dann erzeugt sie Titel, Zusammenfassung und die wichtigsten Begriffe. Fragen entstehen, wenn eine Einheit dran ist; die nächste wird vorab vorbereitet. Danach schreibt die KI 1–2 Strukturaufgaben, die zum Stoff passen.
3. **Lernen (ohne KI):** Die Antworten prüft der Code, nicht die KI.
   - Multiple Choice, Richtig/Falsch, Lückentext (eine oder mehrere Lücken), Begriffe zuordnen
   - **Matrix füllen:** Aussagen in Felder ziehen oder antippen. Vier Felder werden zur 2×2-Matrix, eine SWOT bekommt die Achsen intern/extern und hilfreich/schädlich. Ab Stufe 2 als Szenario („Erstelle die SWOT für ein Café“) mit Aussagen, die nirgends hingehören.
   - **Reihenfolge:** Schritte eines Ablaufs zur Prozesskette ordnen.
   - **Grafik beschriften:** eine Grafik aus den Folien mit nummerierten Stellen, die Begriffe stehen zur Auswahl.
   
   Nach dem Prüfen gleiten falsch abgelegte Begriffe an ihren richtigen Platz.
4. **Wiederholen:** Leitner-Boxen (1, 3, 7, 14, 30 Tage). Gemerkte und prüfungsrelevante Fragen kommen zuerst.
5. **Motivation:** XP, Tagesziel, Streak, Lernstufe je Einheit. In den Apps gibt es eine **Lern-Erinnerung** (Profil → Einstellungen):
   - **Handy:** einmal am Tag zur gewählten Uhrzeit, aber nur, wenn das Tagesziel noch offen ist.
   - **PC:** nur, solange die App läuft.

### Strukturaufgaben: nur wenn der Stoff sie hergibt

- **Matrix:** Die KI legt sie nur an, wenn das Material selbst in Kategorien einteilt. Der Code prüft zusätzlich, ob jeder Feldname im Abschnitt vorkommt.
- **Selbstprüfung:** Jede Strukturaufgabe löst die KI ein zweites Mal, ohne die Lösung zu sehen.
  - Matrix: Nur Aussagen, bei denen beide Durchläufe übereinstimmen, bleiben in der Aufgabe.
  - Reihenfolge und Lückentext: Bei der kleinsten Abweichung wird die Aufgabe verworfen.
- **Grafiken** brauchen keine KI. pdf.js liefert jedes Textstück mit seiner Position.
  - Kurze Texte an einem Bild oder an Formen gelten als Beschriftung. Der Code schneidet die Grafik aus, übermalt die Beschriftungen in der Umgebungsfarbe und fragt sie ab.
  - Bilder ohne Textebene (Karte, Foto) werden mitgespeichert; ihre Stellen setzt man über „+ Aufgabe“ selbst.
  - Für schon eingelesene PDFs: Kurs bearbeiten → „Grafiken zum Beschriften“ → dieselbe Datei wählen.

### Nur Lerninhalt: Organisatorisches fliegt raus

Folien wie „Klausur am 14.02.“, „Sprechstunde Mi 10 Uhr“ oder die Literaturliste sind kein Lernstoff.

- Die KI bekommt beim Analysieren jede Seite nummeriert und markiert die, auf denen **nur** Organisatorisches steht. Diese Seiten gehen nicht in Fragen, Begriffe, Erklärung und „Frag die KI“. Das PDF selbst bleibt vollständig gespeichert.
- Besteht ein Abschnitt nur aus Organisatorischem, steht er im Lernpfad als leise Zeile „Nur Organisatorisches · übersprungen“.
- Falsch aussortiert? Lernkarte oder Zeile → **Doch einbeziehen** (nur der Besitzer des Kurses). Der Abschnitt wird dann ohne Filter neu analysiert.
- Zusätzlich verwirft der Code jede KI-Frage mit Wörtern wie „Klausur“, „Dozent“, „Sprechstunde“ oder „Vorlesung“.
- Schon eingelesene Kurse werden nicht nachträglich gefiltert. Alte Orga-Fragen blendest du aus.

### Prüfungsschwerpunkte

Kurs bearbeiten → **Prüfungsschwerpunkte**: ein Thema pro Zeile. Drei Wege dorthin:

- **Selbst eintragen**, z. B. aus Stoffplan, Lernzielen oder einer Altklausur.
- **Aus PDF übernehmen:** Der Text kommt ins Feld, du streichst, was nicht drankommt.
- **Vorschläge aus deinen Folien:** Was auf den aussortierten Orga-Folien zur Prüfung steht („Klausurrelevant: Kapitel 1–3“), bietet die App zum Antippen an. Ein Vorschlag zählt nur, wenn seine Wörter wirklich im Material stehen. Nichts wird automatisch übernommen.

Danach ordnet die KI jede Einheit den Themen zu (kurzer Aufruf mit Titel, Zusammenfassung und Begriffen).

- Prüfungsrelevante Einheiten tragen im Pfad eine kleine Zielscheibe. Ihre Fragen schreibt die KI schon vorab und mit Blick auf den Schwerpunkt. Die Erklärung geht besonders darauf ein.
- **Für die Klausur merken:** In der Lektion über `⋯` oder auf der Lernkarte. Das gilt nur für dich.
- **Prüfungstraining:** Lernpfad → „Prüfungsthemen üben“ oder Wiederholen → „Für die Klausur“. Es nimmt die Fragen der relevanten Einheiten und deine gemerkten, fällige zuerst.

### Eigene Aufgaben

Auf der Lernkarte unter „Fragen dieser Einheit“ → **Aufgabe hinzufügen**:
- Wählbar sind Multiple Choice, Richtig/Falsch, Lückentext (`[Lücke]` in eckigen Klammern), Zuordnen/Matrix, Reihenfolge und Bild beschriften.
- Für „Bild beschriften“ nimmst du eine Grafik aus dem Skript oder ein eigenes Foto, auch direkt mit der Kamera. Dann tippst du ins Bild und beschriftest jede Stelle.
- Die Aufgabe gilt, wie Bearbeiten, für alle im Kurs.

### Fragen bearbeiten oder ausblenden

Unklar formuliert oder nicht prüfungsrelevant? In der Lektion über `⋯` oder auf der Lernkarte unter „Fragen dieser Einheit“:

- **Bearbeiten:** gilt für alle, die den Kurs nutzen.
- **Ausblenden:** nur für dich, mit „Rückgängig“. Sind alle Fragen einer Einheit ausgeblendet, schreibt die KI eine neue Runde.
- **Für alle löschen:** nur der Besitzer eines geteilten Kurses.

## Fehler melden

Profil → **Über Lernpfad** → „Fehler melden“ oder „Idee vorschlagen“, in der Lektion über `⋯` → „Fehler in dieser Frage melden“, oder direkt am Fehler-Toast über **Melden**. Jede Meldung wird ein Issue auf GitHub.

- **Vorsortieren:** Bei einer Fehlermeldung prüft die App zuerst, ob es ein Programmfehler ist. Bekannte Hinweise wie „Kein Zugriff“, falsches Passwort, Server nicht erreichbar oder ein eingescanntes PDF erkennt der Code selbst. Diese Toasts bekommen gar kein „Melden“. Alles Unklare schätzt die KI ein. Bei einem normalen Hinweis zeigt die App, was du tun kannst („Trotzdem melden“ geht immer).
- **Inhalt des Issues:**
  - eine Beschreibung, die die KI schreibt
  - die Einschätzung
  - dein Originaltext
  - die Fehlermeldung mit den letzten Fehlern
  - die Frage, wenn sie aus dem Fragen-Menü kommt
  - **immer die Systemdaten des Geräts und des Servers**: Version, Betriebssystem, CPU, RAM, KI-Stufe, GPU-Backend und -Gerät, letzte Geschwindigkeit, Größe der Datenbank
  - Nicht enthalten sind Name, IP, Hostname und Pfade. Was mitgeht, lässt sich vor dem Senden aufklappen.
- **Mit eigenem Server:** Der Server legt das Issue direkt an, niemand braucht ein GitHub-Konto. Seine Systemdaten und die Einschätzung hängt er selbst an. Höchstens 5 Meldungen pro Konto und Stunde. Dafür braucht der Server einen fine-grained Token, nur für dieses Repo, mit „Issues: Read and write“:
  ```yaml
  # compose.override.yaml
  services:
    lernpfad:
      environment:
        LERNPFAD_GITHUB_TOKEN: github_pat_…
  ```
- **Ohne Server (oder ohne Token):** Die App öffnet ein vorausgefülltes Issue im Browser. Zum Absenden braucht man dort ein GitHub-Konto. Wird der Text zu lang für den Link, kürzt die App zuerst Fehlerliste und Frage, nie die Systemdaten.
- Nutzertext steht im Issue in Code-Blöcken, @-Erwähnungen, Links und Bilder werden also nicht dargestellt.

## KI

| Stufe | Modell | Größe | Standard auf |
| --- | --- | --- | --- |
| Kompakt | Qwen3.5 2B (Q4_K_M) | 1,3 GB | Android, iOS |
| Ausgewogen | Qwen3.5 4B (Q4_K_M) | 2,7 GB | PC, Server |

llama.cpp ist als Bibliothek eingebaut (`llama-cpp-2`). Das Modell wird beim ersten Start einmal von Hugging Face geladen (gepinnte Revision, Größe geprüft), danach läuft alles offline. Es wird erst geladen, wenn es gebraucht wird, und nach 3 Minuten Leerlauf entladen. GPU: Vulkan (Linux/Windows), Metal (Apple), CPU auf Android.

## Eigener Server

```bash
cargo build --release -p lernpfad-server
./target/release/lernpfad-server --data ./data --web ./web
```

Beim ersten Start schreibt der Server ins Protokoll:

```
Lernpfad-Server läuft: https://192.168.1.20:8443
Eigenes Zertifikat, SHA-256-Fingerabdruck:
  B2:53:41:…
Noch kein Konto. Einrichten (einmalig, nur mit diesem Link):
  https://192.168.1.20:8443/#/setup/908236…
```

1. **Einrichten:** Link öffnen, Name und Passwort festlegen. Dieses erste Konto ist Admin. Nur wer das Server-Protokoll sieht, kann den Server einrichten.
2. **Einladen:** Profil → Mitglieder → „Jemanden einladen“. Der Link gilt 7 Tage und für ein Konto. Eine freie Registrierung gibt es nicht.
3. **Teilen:** Kurs bearbeiten → „Mit allen auf diesem Server teilen“. Jeder behält seinen eigenen Fortschritt.
   - **Verlassen:** Mitglieder können einen geteilten Kurs unter Hinzufügen verlassen. Ihr Fortschritt bleibt erhalten. Kurse für alle holen sie unter „Ausgeblendete Kurse“ zurück. Bei ausgewählten Personen holt eine neue Einladung sie zurück.
   - **Konto löschen:** Profil → Konto → „Konto löschen“ (mit Passwort). Private Kurse werden gelöscht, geteilte gehen an einen Admin über, damit die anderen sie behalten. Der letzte Admin kann sich nicht löschen.
4. **Handy:** Seite öffnen, „Zum Startbildschirm hinzufügen“. Oder die App über den Verbindungslink aus Profil → Konto verbinden.

**Lokale Daten mitnehmen:** Die `lernen.db` der App vor dem ersten Start in den Datenordner kopieren. Dein Konto aus der Einrichtung übernimmt dann alle Kurse und den Fortschritt.

**Docker:**

```bash
docker compose up -d --build
docker compose logs lernpfad     # Einrichtungslink
```

Im Container kennt der Server die Adresse deines Rechners nicht. Trag sie in `compose.override.yaml` ein (wird nicht eingecheckt): `command: ["--host", "192.168.1.20"]`.

### Automatisch aktualisieren

Jeder Push auf `main` startet [.github/workflows/deploy.yml](.github/workflows/deploy.yml): GitHub baut das Image und schickt es per SSH an den Server. Dort lädt [deploy.sh](deploy.sh) es und startet den Container neu. Der Server kompiliert nichts, er braucht also keinen zusätzlichen RAM. Schlägt der Build fehl, läuft die alte Version weiter.

Einmalig einrichten:

1. Repo auf den Server klonen, z. B. nach `/home/Lernpfad`. Dort eine `compose.override.yaml` mit `command: ["--host", "<Server-IP>"]` anlegen.
2. Einen eigenen SSH-Schlüssel ohne Passphrase nur für GitHub erzeugen und auf dem Server mit Zwangsbefehl in `authorized_keys` eintragen. Dieser Schlüssel kann dann nur `deploy.sh` starten:
   `command="/home/Lernpfad/deploy.sh",restrict ssh-ed25519 AAAA… lernpfad-github-deploy`
3. In GitHub unter Settings → Secrets and variables → Actions anlegen: `DEPLOY_HOST` (Server-IP), `DEPLOY_KEY` (privater Teil des Schlüssels), `DEPLOY_KNOWN_HOSTS` (Ausgabe von `ssh-keygen -F <Server-IP>`).

Von Hand neu ausrollen: Actions → Deploy → „Run workflow“.

### Optionen

| Option | Wirkung |
| --- | --- |
| `--data DIR` | Datenbank, Modell, Zertifikat (Standard `./data`) |
| `--web DIR` | Oberfläche (Standard `./web`) |
| `--bind ADR` | Standard `0.0.0.0:8443` |
| `--host NAME` | weiterer Name/IP fürs eigene Zertifikat, mehrfach möglich (erzeugt es neu) |
| `--tls-cert`, `--tls-key` | echtes Zertifikat, z. B. Let's Encrypt oder `tailscale cert` |
| `--behind-proxy` | HTTPS übernimmt ein Proxy davor (z. B. Caddy), Server lauscht nur auf 127.0.0.1 |
| `--http` | ohne HTTPS, nur zum Entwickeln auf 127.0.0.1 |

## Sicherheit des Servers

Wer nur die Adresse kennt, bekommt die Anmeldeseite und sonst nichts. Jede Datenanfrage braucht eine gültige Sitzung, und bei jeder Kurs-, Einheiten- oder Fragen-ID wird geprüft, ob das Konto sie sehen darf. Fremde und nicht vorhandene IDs bekommen dieselbe Antwort.

- **Passwörter:** Argon2id (19 MiB, 2 Durchläufe), mindestens 12 Zeichen. Rund 30 000 bekannte, geleakte Passwörter sind offline gesperrt, auch mit angehängten Ziffern („password123456“).
- **Anmeldung:** Unbekannte Namen werden gegen einen Dummy-Hash geprüft. Das kostet gleich viel Zeit und ergibt dieselbe Meldung, so verrät die Antwort nicht, welche Namen es gibt.
- **Sperr-Timer:**
  - Pro (IP, Name) sind 3 Fehlversuche frei, danach 30 s Sperre, die sich bis auf 15 min verdoppelt.
  - Pro IP: 20 Fehler pro Stunde sperren die IP für 1 h.
  - Pro Name: 50 Fehler pro Stunde sperren den Namen für 15 min.
  - Während einer Sperre rechnet der Server keinen Hash.
- **Sitzungen:**
  - Das Token besteht aus 32 Zufallsbytes, in der Datenbank steht nur sein SHA-256.
  - Der Browser bekommt es als Cookie, das JavaScript nicht lesen kann (`__Host-…`, Secure, HttpOnly, SameSite=Strict).
  - Ungenutzt abgemeldet nach 14 Tagen, spätestens nach 60 Tagen.
  - Unter Profil → Angemeldete Geräte kannst du einzelne Geräte oder alle anderen abmelden.
- **Transport:**
  - Der Server spricht nur HTTPS. Klartext gibt es nur auf 127.0.0.1, sonst startet er nicht.
  - Ohne eigenes Zertifikat erzeugt er eines und zeigt dessen Fingerabdruck. Die Apps pinnen genau dieses Zertifikat.
  - Mit echtem Zertifikat sendet er auch HSTS.
- **Verwaltung (Admin):** Konten sperren (sofort auf allen Geräten abgemeldet) und Passwort-Links ausstellen, wenn jemand sein Passwort vergessen hat.
- Fehlermeldungen gehen nur mit `LERNPFAD_GITHUB_TOKEN` an GitHub. Der Token bleibt auf dem Server und erreicht keine App.
- Die Dateien selbst sind nicht verschlüsselt. Für den Fall, dass die Platte gestohlen wird, ist die Festplattenverschlüsselung des Systems da (LUKS, BitLocker, FileVault).

## Entwickeln

### Voraussetzungen

- Rust über [rustup](https://rustup.rs) und die Tauri-CLI: `cargo install tauri-cli --version "^2" --locked`
- Linux (Debian/Ubuntu/Mint):
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev \
    libxdo-dev cmake libclang-dev libvulkan-dev glslc spirv-headers
  ```
- macOS: Xcode (für iOS zusätzlich die iOS-Plattform)
- Windows: Visual Studio Build Tools mit C++, CMake und das Vulkan SDK
- Android: Android Studio mit SDK und NDK, JDK 17

### Starten

```bash
cargo test --workspace                     # Kern (Lernlogik, Rechte, Migration), Server (Anmeldung), App
cd src-tauri && cargo tauri dev            # PC-App
cargo run -p lernpfad-server -- --http     # Server zum Entwickeln: http://127.0.0.1:8080
cargo tauri android dev                    # Android (vorher einmal: cargo tauri android init)
cargo tauri ios dev                        # iOS, nur auf einem Mac
```

Oberfläche ohne Rust: `python3 -m http.server 8765 -d web`. Dann ersetzt `web/dev/mock-tauri.js` den Kern durch Beispieldaten (`?ai=ready`, `?empty`, `?server`, `?server&login`).

## Aufbau

```
core/src/            Crate lernpfad-core: alles, was App und Server teilen
  api.rs             alle Befehle der Oberfläche + Rechteprüfung (Besitzer, Mitglied)
  ai.rs              Modell-Download, Laden/Entladen, Text erzeugen (llama.cpp)
  import.rs          Abschnitte, Themenfilter, Prüfungsthemen, Prompts, Qualitätsfilter, Strukturaufgaben + Selbstprüfung, Grafiken, Hintergrund-Worker
  report.rs          Fehler melden: Systemdaten, Vorsortieren (Regel + KI), Issue-Text, GitHub-Post (nur Server)
  learn.rs           Antwortprüfung (auch Matrix, Reihenfolge, Lücken, Grafik), Leitner-Boxen, XP, Streak
  db.rs              SQLite-Schema und Migration
src-tauri/src/lib.rs Tauri-Hülle: lokal oder an den eigenen Server weiterleiten (Zertifikat gepinnt)
server/src/
  main.rs            HTTPS, Befehle, Ereignisse (SSE), Sicherheitsheader
  auth.rs            Argon2id, Sitzungen, Passwortregeln, Sperr-Timer
web/                 Oberfläche (Vanilla-JS-Module, kein Build-Schritt), für alle Varianten gleich
```

## Ideen
 
Quiz Duell wenn auf Server und es Kommonitonen gibt, ...


## Datenschutz

In den Apps bleibt alles auf dem Gerät: Dokumente, Fortschritt und Chats. Auf dem eigenen Server liegt alles auf deinem Server. Ins Internet geht es nur, um einmalig das KI-Modell von Hugging Face zu laden, und wenn du selbst einen Fehler oder eine Idee an GitHub meldest.
