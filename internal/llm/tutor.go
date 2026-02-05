package llm

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"time"

	"lernplattform/internal/models"
)

// Tutor verwaltet die didaktische KI-Logik
type Tutor struct {
	provider   Provider
	agentPool  *AgentPool
	useAgents  bool
}

// NewTutor erstellt einen neuen Tutor
func NewTutor(provider Provider) *Tutor {
	return &Tutor{
		provider:  provider,
		useAgents: true, // Standard: Agenten-Modus aktiviert
	}
}

// NewTutorWithAgents erstellt einen Tutor mit Agent-Pool
func NewTutorWithAgents(provider Provider, fastModel string, numAgents int) *Tutor {
	config := ParallelAgentConfig{
		MaxWorkers:     numAgents,
		FastModel:      fastModel,
		TimeoutPerTask: 2 * time.Minute,
	}
	return &Tutor{
		provider:   provider,
		agentPool:  NewAgentPool(provider, config),
		useAgents:  true,
	}
}

// SetAgentMode aktiviert/deaktiviert den Agenten-Modus
func (t *Tutor) SetAgentMode(enabled bool, fastModel string, numAgents int) {
	t.useAgents = enabled
	if enabled && t.agentPool == nil {
		config := ParallelAgentConfig{
			MaxWorkers:     numAgents,
			FastModel:      fastModel,
			TimeoutPerTask: 2 * time.Minute,
		}
		t.agentPool = NewAgentPool(t.provider, config)
	}
}

// AnalyzeDocuments analysiert Dokumente und extrahiert Themen
func (t *Tutor) AnalyzeDocuments(ctx context.Context, documents []models.Document) ([]models.Topic, error) {
	// Verwende Agenten-Modus wenn aktiviert
	if t.useAgents && t.agentPool != nil {
		return t.agentPool.AnalyzeDocumentsParallel(ctx, documents)
	}
	
	// Fallback: Sequentielle Analyse
	log.Println("   [Tutor] Sequentieller Modus (ohne Agenten)")
	log.Println("   [Tutor] Bereite Dokumenteninhalt vor...")
	
	// Dedupliziere Dokumente nach Name
	seen := make(map[string]bool)
	var uniqueDocs []models.Document
	for _, doc := range documents {
		if !seen[doc.Name] {
			seen[doc.Name] = true
			uniqueDocs = append(uniqueDocs, doc)
		}
	}
	log.Printf("   [Tutor] %d eindeutige Dokumente (von %d)", len(uniqueDocs), len(documents))
	
	// Priorisiere Hauptskripte (keine Klausuren/Übungsblätter für Analyse)
	var mainDocs []models.Document
	var otherDocs []models.Document
	for _, doc := range uniqueDocs {
		nameLower := strings.ToLower(doc.Name)
		if strings.Contains(nameLower, "klausur") || strings.Contains(nameLower, "übung") {
			otherDocs = append(otherDocs, doc)
		} else {
			mainDocs = append(mainDocs, doc)
		}
	}
	
	// Verwende hauptsächlich die Skripte für die Themenanalyse
	docsToAnalyze := mainDocs
	if len(docsToAnalyze) == 0 {
		docsToAnalyze = uniqueDocs
	}
	log.Printf("   [Tutor] Analysiere %d Hauptdokumente", len(docsToAnalyze))
	
	// Kombiniere Dokumenteninhalte mit striktem Limit
	var allContent strings.Builder
	maxTotalChars := 30000 // Max 30k Zeichen gesamt für den Prompt
	charsPerDoc := maxTotalChars / max(len(docsToAnalyze), 1)
	if charsPerDoc > 8000 {
		charsPerDoc = 8000
	}
	if charsPerDoc < 2000 {
		charsPerDoc = 2000
	}
	
	for _, doc := range docsToAnalyze {
		allContent.WriteString(fmt.Sprintf("\n=== Dokument: %s ===\n", doc.Name))
		content := doc.Content
		if len(content) > charsPerDoc {
			log.Printf("   [Tutor] Dokument '%s' gekürzt (von %d auf %d Zeichen)", doc.Name, len(content), charsPerDoc)
			content = content[:charsPerDoc] + "\n[... gekürzt ...]"
		}
		allContent.WriteString(content)
		
		if allContent.Len() > maxTotalChars {
			log.Printf("   [Tutor] Maximale Prompt-Größe erreicht, stoppe bei %d Dokumenten", len(docsToAnalyze))
			break
		}
	}

	log.Printf("   [Tutor] Gesamte Prompt-Länge: %d Zeichen", allContent.Len())
	log.Println("   [Tutor] Sende Anfrage an LLM...")

	prompt := fmt.Sprintf(`Analysiere die folgenden Lernmaterialien und identifiziere die Hauptthemen/Kapitel.
Erstelle eine strukturierte Liste der Themen, die für eine Prüfungsvorbereitung relevant sind.

Antworte NUR im folgenden JSON-Format:
{
  "topics": [
    {
      "name": "Themenname",
      "description": "Kurze Beschreibung des Themas",
      "difficulty": 1-5,
      "est_minutes": geschätzte Lernzeit in Minuten
    }
  ]
}

Materialien:
%s`, allContent.String())

	resp, err := t.provider.Generate(ctx, prompt, &GenerateOptions{
		Temperature: 0.3,
		System:      "Du bist ein erfahrener Dozent, der Lernmaterialien analysiert und strukturiert. Antworte immer auf Deutsch und nur im angeforderten JSON-Format.",
	})
	if err != nil {
		log.Printf("   [Tutor] ❌ LLM-Fehler: %v", err)
		return nil, err
	}

	log.Printf("   [Tutor] ✓ LLM-Antwort erhalten (%d Zeichen)", len(resp.Content))
	log.Println("   [Tutor] Parse JSON-Antwort...")

	// JSON aus Antwort extrahieren
	topics, err := parseTopicsFromResponse(resp.Content)
	if err != nil {
		log.Printf("   [Tutor] ❌ JSON-Parse-Fehler: %v", err)
		log.Printf("   [Tutor] Rohe Antwort: %s", resp.Content[:min(500, len(resp.Content))])
		return nil, fmt.Errorf("konnte Themen nicht parsen: %w", err)
	}

	log.Printf("   [Tutor] ✓ %d Themen erfolgreich geparst", len(topics))
	return topics, nil
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}

// CreateStudyPlan erstellt einen Lernplan basierend auf Prüfungsdatum
func (t *Tutor) CreateStudyPlan(ctx context.Context, topics []models.Topic, examDate time.Time, documentsContent string) (*models.StudyPlan, error) {
	daysUntilExam := int(time.Until(examDate).Hours() / 24)
	if daysUntilExam < 1 {
		daysUntilExam = 1
	}

	// Berechne verfügbare Lernzeit
	totalMinutes := 0
	for _, topic := range topics {
		totalMinutes += topic.EstMinutes
	}

	// Erstelle Lernplan
	plan := &models.StudyPlan{
		ID:           fmt.Sprintf("plan_%d", time.Now().UnixNano()),
		Name:         fmt.Sprintf("Lernplan für Prüfung am %s", examDate.Format("02.01.2006")),
		ExamDate:     examDate,
		CreatedAt:    time.Now(),
		TotalMinutes: totalMinutes,
		Status:       "active",
		Progress:     0,
	}

	// Verteile Themen über verfügbare Tage
	minutesPerDay := totalMinutes / daysUntilExam
	if minutesPerDay < 30 {
		minutesPerDay = 30
	}

	for i := range topics {
		topics[i].ID = fmt.Sprintf("topic_%d_%d", time.Now().UnixNano(), i)
		topics[i].StudyPlanID = plan.ID
		topics[i].Order = i + 1
		topics[i].Status = "pending"
		topics[i].Progress = 0
	}

	plan.Topics = topics
	return plan, nil
}

// ExplainTopic erklärt ein Thema basierend auf den Dokumenten
func (t *Tutor) ExplainTopic(ctx context.Context, topic *models.Topic, documentContent string) (*models.Explanation, error) {
	prompt := fmt.Sprintf(`Du bist ein geduldiger, sehr klar erklärender Tutor.
Dein Ziel ist es, einer Person mit Lernschwierigkeiten das Thema wirklich verständlich zu machen.

Erkläre nicht nur das Offensichtliche, sondern auch wichtige Zusammenhänge,
typische Denkfehler und Grundlagen, die oft stillschweigend vorausgesetzt werden.

Thema: %s
Beschreibung: %s

Material (nutze es als Hauptquelle, aber erkläre bei Bedarf Grundlagen):
%s

WICHTIG:
- Schreibe **einfach**, **klar** und **schrittweise**
- Gehe davon aus, dass die Person wenig Vorwissen hat
- Erkläre implizite Annahmen (Dinge, die oft "einfach bekannt" sein sollen)
- Wenn ein Begriff zum Verständnis notwendig ist, erkläre ihn – auch wenn er im Material nur kurz vorkommt
- Keine unnötige Fachsprache

**REGELN – UNBEDINGT EINHALTEN**

1. **ALLE Fachbegriffe IMMER fett markieren**
2. **Kurze Absätze** (max. 2–3 Sätze)
3. **Bullet Points** für Aufzählungen
4. **Keine Emojis in Überschriften**
5. Wichtige Merksätze als Blockquote: > **Merke:** …
6. Keine langen Textblöcke
7. Keine Abschweifungen
8. Keine Annahmen über Vorwissen

---

## Worum geht's?
- Erkläre in **1–2 einfachen Sätzen**, was das Thema ist
- Sag klar, **warum das Thema wichtig ist**

## Die wichtigsten Begriffe
- **Begriff 1**: Sehr einfache Erklärung
- **Begriff 2**: Sehr einfache Erklärung
- Erkläre Begriffe so, als hätte man sie noch nie gehört

## Grundlagen, die man dafür verstehen muss
- Welche **Grundideen** braucht man, um das Thema zu kapieren?
- Erkläre diese kurz und verständlich

## So funktioniert es – Schritt für Schritt
- Erkläre den Ablauf logisch und langsam
- Nutze **fette Fachbegriffe**
- Ein Gedanke pro Absatz

## Typische Denkfehler
- Was wird häufig falsch verstanden?
- Warum ist das falsch?

## Beispiel aus der Praxis
- Ein **konkretes, einfaches Beispiel**
- Bezug auf Alltag oder Praxis

## Zusammenfassung
- Wichtigster Punkt
- Zweitwichtigster Punkt
- Drittwichtigster Punkt

> **Merke:** Ein zentraler Satz, den man sich merken sollte

Antworte **nur auf Deutsch**.
Halte alles **übersichtlich, ruhig und lernfreundlich**.`, topic.Name, topic.Description, limitContent(documentContent, 8000))

	resp, err := t.provider.Generate(ctx, prompt, &GenerateOptions{
		Temperature: 0.5,
		System:      "Du bist ein geduldiger Tutor für Menschen mit Lernschwierigkeiten. Erkläre alles von Grund auf. Keine Annahmen über Vorwissen. Fachbegriffe immer fett und erklären. Kurze Absätze. Typische Denkfehler aufzeigen.",
	})
	if err != nil {
		return nil, err
	}

	explanation := &models.Explanation{
		TopicID: topic.ID,
		Title:   topic.Name,
		Content: resp.Content,
	}

	return explanation, nil
}

// GenerateQuestions generiert Fragen zu einem Thema
func (t *Tutor) GenerateQuestions(ctx context.Context, topic *models.Topic, documentContent string, difficulty int, count int) ([]models.Question, error) {
	if count <= 0 {
		count = 3 // Standard: 3 Fragen
	}
	
	difficultyDesc := map[int]string{
		1: "einfache Verständnisfragen",
		2: "grundlegende Wissensfragen",
		3: "Anwendungsfragen",
		4: "Analyse- und Verknüpfungsfragen",
		5: "komplexe Transfer- und Synthesefragen",
	}

	prompt := fmt.Sprintf(`Erstelle %s zum Thema "%s".

Material:
%s

Erstelle genau %d Fragen mit Schwierigkeitsgrad %d.
Schwierigkeitstyp: %s

Antworte NUR im JSON-Format:
{
  "questions": [
    {
      "question": "Die Frage",
      "expected_answer": "Die direkte Antwort",
      "hints": ["Inhaltlicher Denkansatz", "Weiterer inhaltlicher Hinweis"],
      "type": "open"
    }
  ]
}

**WICHTIGE REGELN:**

1. **expected_answer:**
   - DIREKTE inhaltliche Antwort
   - NIEMALS "Siehe Kapitel X" oder "Seite Y"
   - Die tatsächliche Definition/Erklärung

2. **hints (SEHR WICHTIG!):**
   - NIEMALS "Schauen Sie auf Seite X" oder "Siehe Kapitel Y"
   - IMMER inhaltliche Denkhilfen!
   - GUTE Beispiele:
     * "Denke an die drei Bereiche: Einkauf, Herstellung, Transport"
     * "Welche Gruppen beeinflussen Nachhaltigkeit? Firmen, Staat, Menschen..."
     * "Überlege: Was kommt rein, was passiert damit, was kommt raus?"
   - SCHLECHTE Beispiele (VERBOTEN!):
     * "Siehe Seite 5"
     * "Kapitel 2.3 behandelt das"
     * "Im Skript steht..."`, difficultyDesc[difficulty], topic.Name, limitContent(documentContent, 6000), count, difficulty, difficultyDesc[difficulty])

	resp, err := t.provider.Generate(ctx, prompt, &GenerateOptions{
		Temperature: 0.4,
		System:      "Du bist ein Prüfer. Fragen prüfen WISSEN, nicht wo es steht. Hinweise sind INHALTLICHE Denkhilfen, NIEMALS Seitenverweise. JSON-Format.",
	})
	if err != nil {
		return nil, err
	}

	questions, err := parseQuestionsFromResponse(resp.Content, topic.ID, difficulty)
	if err != nil {
		return nil, err
	}

	return questions, nil
}

// EvaluateAnswer bewertet eine Antwort des Studenten
func (t *Tutor) EvaluateAnswer(ctx context.Context, question *models.Question, userAnswer string, documentContent string) (bool, string, error) {
	// Leere oder zu kurze Antworten sofort als falsch werten
	if len(strings.TrimSpace(userAnswer)) < 3 {
		return false, "💡 Du hast keine richtige Antwort eingegeben. Versuch es nochmal!", nil
	}
	
	prompt := fmt.Sprintf(`Bewerte diese Antwort FAIR aber nicht zu großzügig:

Frage: %s
Erwartete Kernpunkte: %s
Antwort des Studenten: %s

Antworte im JSON-Format:
{
  "is_correct": true/false,
  "feedback": "Kurzes Feedback",
  "score": 0-100
}

**BEWERTUNGSREGELN:**

1. **is_correct = TRUE wenn:**
   - Mindestens 70-80%% der Kernpunkte inhaltlich genannt wurden
   - Tippfehler vorhanden sind ("Diputive" statt "Dispositive")
   - Synonyme verwendet werden
   - Die Formulierung anders aber inhaltlich korrekt ist

2. **is_correct = FALSE wenn:**
   - Die Antwort komplett falsch oder am Thema vorbei ist
   - Wichtige Kernbegriffe fehlen (z.B. nur 1 von 3 genannt)
   - Die Antwort zu vage/allgemein ist ohne konkrete Punkte
   - Die Antwort nur 1-2 Wörter enthält ohne echten Inhalt
   - Nur "ja", "nein", "keine", "weiß nicht" etc.

3. **Feedback-Regeln:**
   - Bei TRUE: "✅ Richtig! [kurzes Lob, max 1 Satz]"
   - Bei FALSE: "💡 [Was konkret fehlt] - Die richtige Antwort ist: [Antwort]"
   - KURZ halten! Max 2 Sätze.

BEISPIELE:
- "Unternehmen, Politik, Gesellschaft" -> TRUE (Kernpunkte genannt)
- "Die drei Ebenen sind Güter, Finanzen und Disposition" -> TRUE
- "keine" oder "weiß nicht" -> FALSE
- "Wirtschaft" (zu vage) -> FALSE`, question.Question, question.ExpectedAnswer, userAnswer)

	resp, err := t.provider.Generate(ctx, prompt, &GenerateOptions{
		Temperature: 0.1,
		System:      "Du bist ein FAIRER Prüfer. Akzeptiere Antworten wenn die Kernidee stimmt. ABER: Leere, zu kurze oder völlig falsche Antworten sind FALSCH. Tippfehler ignorieren. JSON-Format.",
	})
	if err != nil {
		return false, "", err
	}

	var result struct {
		IsCorrect bool   `json:"is_correct"`
		Feedback  string `json:"feedback"`
	}

	// JSON aus Antwort extrahieren
	jsonStr := extractJSON(resp.Content)
	if err := json.Unmarshal([]byte(jsonStr), &result); err != nil {
		// Fallback: Einfache Heuristik
		return strings.Contains(strings.ToLower(resp.Content), "richtig"), resp.Content, nil
	}

	return result.IsCorrect, result.Feedback, nil
}

// ChatWithContext ermöglicht einen kontextbezogenen Chat
func (t *Tutor) ChatWithContext(ctx context.Context, messages []ChatMessage, documentContext string, topic *models.Topic) (*GenerateResponse, error) {
	systemPrompt := fmt.Sprintf(`Du bist ein hilfreicher Lernassistent. 
Du hilfst dem Studenten beim Lernen und beantwortest Fragen.

WICHTIG: Du darfst NUR Informationen aus dem folgenden Kontext verwenden.
Wenn eine Frage nicht aus dem Kontext beantwortet werden kann, sage das ehrlich.

Aktuelles Thema: %s
Beschreibung: %s

Verfügbarer Kontext aus den Lernmaterialien:
%s`, topic.Name, topic.Description, limitContent(documentContext, 6000))

	// Füge System-Nachricht hinzu
	allMessages := append([]ChatMessage{{Role: "system", Content: systemPrompt}}, messages...)

	return t.provider.Chat(ctx, allMessages, &GenerateOptions{
		Temperature: 0.5,
	})
}

// Helper-Funktionen

func limitContent(content string, maxLen int) string {
	if len(content) <= maxLen {
		return content
	}
	return content[:maxLen] + "\n[... gekürzt ...]"
}

func extractJSON(text string) string {
	start := strings.Index(text, "{")
	end := strings.LastIndex(text, "}")
	if start == -1 || end == -1 || start >= end {
		return "{}"
	}
	return text[start : end+1]
}

func parseTopicsFromResponse(response string) ([]models.Topic, error) {
	jsonStr := extractJSON(response)

	var result struct {
		Topics []struct {
			Name        string `json:"name"`
			Description string `json:"description"`
			Difficulty  int    `json:"difficulty"`
			EstMinutes  int    `json:"est_minutes"`
		} `json:"topics"`
	}

	if err := json.Unmarshal([]byte(jsonStr), &result); err != nil {
		return nil, err
	}

	var topics []models.Topic
	for _, t := range result.Topics {
		topics = append(topics, models.Topic{
			Name:        t.Name,
			Description: t.Description,
			Difficulty:  t.Difficulty,
			EstMinutes:  t.EstMinutes,
		})
	}

	return topics, nil
}

func parseQuestionsFromResponse(response string, topicID string, difficulty int) ([]models.Question, error) {
	jsonStr := extractJSON(response)

	var result struct {
		Questions []struct {
			Question       string   `json:"question"`
			ExpectedAnswer string   `json:"expected_answer"`
			Hints          []string `json:"hints"`
			Type           string   `json:"type"`
		} `json:"questions"`
	}

	if err := json.Unmarshal([]byte(jsonStr), &result); err != nil {
		return nil, err
	}

	var questions []models.Question
	for i, q := range result.Questions {
		qType := q.Type
		if qType == "" {
			qType = "open"
		}

		questions = append(questions, models.Question{
			ID:             fmt.Sprintf("q_%d_%d", time.Now().UnixNano(), i),
			TopicID:        topicID,
			Question:       q.Question,
			ExpectedAnswer: q.ExpectedAnswer,
			Hints:          q.Hints,
			Difficulty:     difficulty,
			Type:           qType,
		})
	}

	return questions, nil
}
