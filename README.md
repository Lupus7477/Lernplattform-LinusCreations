# 🎓 Lokale KI-Lernplattform

Eine vollständig lokal laufende KI-gestützte Lernplattform für die Prüfungsvorbereitung. Die Anwendung analysiert deine PDF-Dokumente (z.B. Vorlesungsskripte) und erstellt daraus einen personalisierten Lernplan mit Erklärungen und Quizfragen.

## ✨ Features

- **100% Lokal**: Keine Cloud, keine externen APIs - alle Daten bleiben auf deinem Computer
- **PDF-Analyse**: Automatisches Einlesen und Analysieren von PDF-Dokumenten
- **Intelligenter Lernplan**: KI erstellt basierend auf Prüfungsdatum einen strukturierten Lernplan
- **Adaptive Fragen**: Schwierigkeitsgrad steigt schrittweise an
- **Fortschrittsverfolgung**: Dashboard mit Übersicht über deinen Lernfortschritt
- **Interaktiver Chat**: Stelle Fragen zu deinen Lernmaterialien

## 🏗️ Architektur

```
lernplattform/
├── cmd/
│   └── server/          # Haupteinstiegspunkt
├── internal/
│   ├── api/             # REST-API Handler & Router
│   ├── config/          # Konfigurationsverwaltung
│   ├── llm/             # LLM-Provider (Ollama) & Tutor-Logik
│   ├── models/          # Datenmodelle
│   ├── pdf/             # PDF-Parser
│   └── storage/         # SQLite-Datenpersistenz
├── web/
│   └── static/          # Frontend (HTML, CSS, JS)
└── config.json          # Konfigurationsdatei
```

## 🚀 Installation

### Voraussetzungen

1. **Go 1.21+** installieren: https://go.dev/dl/
2. **Ollama** installieren: https://ollama.ai/download

### Ollama einrichten

```bash
# Ollama starten (läuft im Hintergrund)
ollama serve

# Ein Modell herunterladen (z.B. Llama 3.2)
ollama pull llama3.2
```

### Lernplattform starten

```bash
# In das Projektverzeichnis wechseln
cd lernplattform

# Abhängigkeiten installieren
go mod tidy

# Server starten
go run ./cmd/server
```

Die Anwendung ist dann unter **http://localhost:8080** erreichbar.

## 📖 Verwendung

### Schritt 1: Dokumente hochladen

1. Öffne die Anwendung im Browser
2. Gehe zu **📚 Dokumente**
3. Lade deine PDF-Dateien hoch oder klicke auf "Ordner scannen"

### Schritt 2: Lernplan erstellen

1. Gehe zu **📅 Lernplan**
2. Wähle dein Prüfungsdatum
3. Wähle die relevanten Dokumente aus
4. Klicke auf "Lernplan erstellen"

Die KI analysiert deine Dokumente und erstellt automatisch Themen/Kapitel.

### Schritt 3: Lernen

1. Gehe zu **📖 Lernen**
2. Wähle ein Thema aus
3. Die KI erklärt dir das Thema basierend auf deinen Materialien
4. Markiere Themen als abgeschlossen

### Schritt 4: Quiz

1. Gehe zu **❓ Quiz**
2. Wähle ein Thema und Schwierigkeitsgrad
3. Beantworte die Fragen
4. Erhalte sofortiges Feedback

### Chat

Im **💬 Chat** kannst du jederzeit Fragen zu deinen Lernmaterialien stellen.

## ⚙️ Konfiguration

Bearbeite `config.json`:

```json
{
  "server_port": "8080",
  "documents_path": "./dokumente",
  "database_path": "lernplattform.db",
  "ollama_url": "http://localhost:11434",
  "default_model": "llama3.2",
  "min_study_session_minutes": 30,
  "max_questions_per_topic": 10
}
```

### Unterstützte Modelle

Die Plattform ist kompatibel mit allen Ollama-Modellen:

- `llama3.2` (empfohlen)
- `mistral`
- `codellama`
- `phi`
- Und viele mehr...

## 🔒 Datenschutz

- **Alle Daten bleiben lokal**: Dokumente, Lernfortschritt und Chat-Verläufe
- **Keine Internetverbindung nötig** (nach Installation von Ollama)
- **SQLite-Datenbank**: Alle Daten in einer lokalen Datei
- **Keine Telemetrie**: Kein Tracking, keine Analytics

## 🛠️ Entwicklung

### Projektstruktur erweitern

**Neuen LLM-Provider hinzufügen:**

Implementiere das `llm.Provider`-Interface:

```go
type Provider interface {
    Generate(ctx context.Context, prompt string, options *GenerateOptions) (*GenerateResponse, error)
    GenerateStream(ctx context.Context, prompt string, options *GenerateOptions) (<-chan StreamChunk, error)
    Chat(ctx context.Context, messages []ChatMessage, options *GenerateOptions) (*GenerateResponse, error)
    GetModels(ctx context.Context) ([]ModelInfo, error)
    IsAvailable(ctx context.Context) bool
    GetName() string
}
```

### API-Endpoints

| Methode | Endpoint | Beschreibung |
|---------|----------|--------------|
| GET | `/api/v1/health` | Systemstatus |
| GET | `/api/v1/documents` | Alle Dokumente |
| POST | `/api/v1/documents` | Dokument hochladen |
| POST | `/api/v1/documents/scan` | Ordner scannen |
| GET | `/api/v1/plans` | Alle Lernpläne |
| POST | `/api/v1/plans` | Neuen Lernplan erstellen |
| GET | `/api/v1/plans/active` | Aktiver Lernplan |
| GET | `/api/v1/topics/{id}/explain` | Themenerklärung |
| POST | `/api/v1/topics/{id}/questions/generate` | Fragen generieren |
| POST | `/api/v1/questions/{id}/answer` | Antwort einreichen |
| POST | `/api/v1/chat` | Chat-Nachricht senden |
| GET | `/api/v1/progress` | Lernfortschritt |

## 📋 Roadmap

- [ ] Export von Lernfortschritt (PDF/CSV)
- [ ] Spaced Repetition Algorithmus
- [ ] Mehrere Benutzerprofile
- [ ] Dark Mode
- [ ] Mobile App (PWA)
- [ ] Weitere LLM-Provider (LocalAI, etc.)

## 🤝 Lizenz

Dieses Projekt ist für den persönlichen Bildungsgebrauch gedacht.

---

**Hinweis**: Diese Anwendung nutzt ausschließlich die von dir bereitgestellten Materialien als Wissensquelle. Die KI kann keine Informationen erfinden, die nicht in deinen Dokumenten enthalten sind.
