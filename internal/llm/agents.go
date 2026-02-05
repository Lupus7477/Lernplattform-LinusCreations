package llm

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"lernplattform/internal/models"
)

// AgentTask repräsentiert eine Aufgabe für einen Mini-Agenten
type AgentTask struct {
	ID       int
	Type     string // "analyze_doc", "extract_topics", "create_questions"
	Document models.Document
	Prompt   string
}

// AgentResult ist das Ergebnis eines Mini-Agenten
type AgentResult struct {
	TaskID  int
	Success bool
	Topics  []models.Topic
	Error   error
	Duration time.Duration
}

// ParallelAgentConfig konfiguriert den Agenten-Pool
type ParallelAgentConfig struct {
	MaxWorkers    int    // Anzahl paralleler Agenten
	FastModel     string // Schnelles Modell für Agenten (z.B. llama3.2:3b)
	TimeoutPerTask time.Duration
}

// AgentPool verwaltet parallele Mini-Agenten
type AgentPool struct {
	provider Provider
	config   ParallelAgentConfig
	mu       sync.Mutex
}

// NewAgentPool erstellt einen neuen Agenten-Pool
func NewAgentPool(provider Provider, config ParallelAgentConfig) *AgentPool {
	// WICHTIG: Ollama kann nur 1 Anfrage gleichzeitig effizient verarbeiten
	// Mehr parallele Worker führen zu Speicherüberlauf!
	config.MaxWorkers = 1 // Erzwinge sequentielle Verarbeitung
	if config.TimeoutPerTask == 0 {
		config.TimeoutPerTask = 2 * time.Minute
	}
	return &AgentPool{
		provider: provider,
		config:   config,
	}
}

// AnalyzeDocumentsParallel analysiert Dokumente sequentiell (Ollama-Limit)
func (ap *AgentPool) AnalyzeDocumentsParallel(ctx context.Context, documents []models.Document) ([]models.Topic, error) {
	startTime := time.Now()
	
	log.Println("   🤖 SMART-ANALYSE-MODUS aktiviert")
	log.Printf("   🚀 Schnelles Modell: %s", ap.config.FastModel)
	log.Println("   ⚡ Sequentielle Verarbeitung (Ollama-optimiert)")
	log.Println("")

	// Dedupliziere und filtere Dokumente
	uniqueDocs := deduplicateDocuments(documents)
	mainDocs, examDocs := categorizeDocuments(uniqueDocs)
	
	log.Printf("   📚 %d Hauptdokumente + %d Klausuren/Übungen", len(mainDocs), len(examDocs))

	// Phase 1: Analysiere Hauptdokumente sequentiell
	log.Println("")
	log.Println("   ═══════════════════════════════════════════════")
	log.Println("   📖 PHASE 1: Hauptdokumente analysieren")
	log.Println("   ═══════════════════════════════════════════════")
	
	mainTopics := ap.analyzeDocumentsSequentially(ctx, mainDocs)
	
	// Phase 2: Extrahiere wichtige Themen aus Klausuren (optional, schnell)
	if len(examDocs) > 0 && len(mainTopics) > 0 {
		log.Println("")
		log.Println("   ═══════════════════════════════════════════════")
		log.Println("   📝 PHASE 2: Klausurthemen priorisieren")
		log.Println("   ═══════════════════════════════════════════════")
		
		mainTopics = ap.prioritizeWithExams(ctx, mainTopics, examDocs)
	}

	// Dedupliziere und sortiere Themen
	finalTopics := deduplicateTopics(mainTopics)
	
	log.Println("")
	log.Printf("   ✅ Analyse abgeschlossen in %v", time.Since(startTime))
	log.Printf("   📊 %d eindeutige Themen gefunden", len(finalTopics))
	
	return finalTopics, nil
}

// analyzeDocumentsSequentially analysiert Dokumente nacheinander (Ollama-freundlich)
func (ap *AgentPool) analyzeDocumentsSequentially(ctx context.Context, docs []models.Document) []models.Topic {
	if len(docs) == 0 {
		return nil
	}

	var allTopics []models.Topic
	successCount := 0
	
	for i, doc := range docs {
		docName := doc.Name
		if len(docName) > 35 {
			docName = docName[:32] + "..."
		}
		
		log.Printf("   [%d/%d] 🔍 Analysiere: %s", i+1, len(docs), docName)
		startTime := time.Now()
		
		topics, err := ap.analyzeOneDocument(ctx, doc)
		duration := time.Since(startTime)
		
		if err != nil {
			log.Printf("   [%d/%d] ❌ Fehler nach %v: %v", i+1, len(docs), duration, err)
			continue
		}
		
		successCount++
		allTopics = append(allTopics, topics...)
		log.Printf("   [%d/%d] ✓ Fertig in %v (%d Themen)", i+1, len(docs), duration, len(topics))
	}
	
	log.Printf("   ✓ %d/%d Dokumente erfolgreich analysiert", successCount, len(docs))
	return allTopics
}

// analyzeDocumentsInParallel führt parallele Dokumentenanalyse durch (Legacy)
func (ap *AgentPool) analyzeDocumentsInParallel(ctx context.Context, docs []models.Document) []models.Topic {
	// Verwende jetzt sequentielle Verarbeitung
	return ap.analyzeDocumentsSequentially(ctx, docs)
}

// documentWorker ist ein Worker-Goroutine für Dokumentenanalyse (nicht mehr verwendet)
func (ap *AgentPool) documentWorker(ctx context.Context, workerID int, tasks <-chan AgentTask, results chan<- AgentResult) {
	for task := range tasks {
		startTime := time.Now()
		docName := task.Document.Name
		if len(docName) > 30 {
			docName = docName[:27] + "..."
		}
		
		log.Printf("   [Agent %d] 🔍 Starte: %s", workerID, docName)
		
		// Timeout pro Task
		taskCtx, cancel := context.WithTimeout(ctx, ap.config.TimeoutPerTask)
		
		topics, err := ap.analyzeOneDocument(taskCtx, task.Document)
		cancel()
		
		duration := time.Since(startTime)
		
		if err != nil {
			log.Printf("   [Agent %d] ❌ Fehler nach %v: %s - %v", workerID, duration, docName, err)
			results <- AgentResult{
				TaskID:   task.ID,
				Success:  false,
				Error:    err,
				Duration: duration,
			}
		} else {
			log.Printf("   [Agent %d] ✓ Fertig in %v: %s (%d Themen)", workerID, duration, docName, len(topics))
			results <- AgentResult{
				TaskID:   task.ID,
				Success:  true,
				Topics:   topics,
				Duration: duration,
			}
		}
	}
}

// analyzeOneDocument analysiert ein einzelnes Dokument
func (ap *AgentPool) analyzeOneDocument(ctx context.Context, doc models.Document) ([]models.Topic, error) {
	// Kürze Inhalt für schnelle Analyse
	content := doc.Content
	maxChars := 4000 // Kurz für schnelle Verarbeitung
	if len(content) > maxChars {
		content = content[:maxChars]
	}

	prompt := fmt.Sprintf(`Analysiere dieses Dokument und liste die 3-5 wichtigsten Lernthemen auf.

Dokument: %s
---
%s
---

Antworte NUR im JSON-Format:
{"topics": [{"name": "Thema", "description": "Kurzbeschreibung", "difficulty": 1-5, "est_minutes": 30}]}`, 
		doc.Name, content)

	// Verwende schnelles Modell
	oldModel := ap.provider.GetCurrentModel()
	if ap.config.FastModel != "" && ap.config.FastModel != oldModel {
		ap.provider.SetModel(ap.config.FastModel)
		defer ap.provider.SetModel(oldModel)
	}

	resp, err := ap.provider.Generate(ctx, prompt, &GenerateOptions{
		Temperature: 0.3,
		System:      "Du bist ein Lernassistent. Antworte kurz und nur im JSON-Format.",
	})
	if err != nil {
		return nil, err
	}

	return parseTopicsFromResponse(resp.Content)
}

// prioritizeWithExams gewichtet Themen basierend auf Klausuren
func (ap *AgentPool) prioritizeWithExams(ctx context.Context, topics []models.Topic, examDocs []models.Document) []models.Topic {
	if len(examDocs) == 0 || len(topics) == 0 {
		return topics
	}

	// Sammle alle Klausur-Inhalte
	var examContent strings.Builder
	for _, doc := range examDocs {
		content := doc.Content
		if len(content) > 2000 {
			content = content[:2000]
		}
		examContent.WriteString(content)
		examContent.WriteString("\n")
		if examContent.Len() > 10000 {
			break
		}
	}

	// Erstelle Liste der Themennamen
	var topicNames []string
	for _, t := range topics {
		topicNames = append(topicNames, t.Name)
	}

	prompt := fmt.Sprintf(`Basierend auf diesen Klausurfragen, welche der folgenden Themen sind am wichtigsten?
Sortiere sie nach Prüfungsrelevanz (wichtigste zuerst).

Themen: %s

Klausurinhalt:
%s

Antworte NUR mit der sortierten Liste als JSON:
{"priority": ["Wichtigstes Thema", "Zweitwichtigstes", ...]}`,
		strings.Join(topicNames, ", "), examContent.String())

	// Schnelle Anfrage
	taskCtx, cancel := context.WithTimeout(ctx, 1*time.Minute)
	defer cancel()
	
	oldModel := ap.provider.GetCurrentModel()
	if ap.config.FastModel != "" {
		ap.provider.SetModel(ap.config.FastModel)
		defer ap.provider.SetModel(oldModel)
	}

	resp, err := ap.provider.Generate(taskCtx, prompt, &GenerateOptions{
		Temperature: 0.2,
		System:      "Du bist ein Prüfungsexperte. Antworte nur im JSON-Format.",
	})
	if err != nil {
		log.Printf("   ⚠️ Priorisierung übersprungen: %v", err)
		return topics
	}

	// Parse Priorität und sortiere Themen
	var priorityResult struct {
		Priority []string `json:"priority"`
	}
	
	// Extrahiere JSON
	jsonStr := resp.Content
	if start := strings.Index(jsonStr, "{"); start != -1 {
		if end := strings.LastIndex(jsonStr, "}"); end != -1 {
			jsonStr = jsonStr[start : end+1]
		}
	}
	
	if err := json.Unmarshal([]byte(jsonStr), &priorityResult); err != nil {
		return topics
	}

	// Sortiere Themen nach Priorität
	priorityMap := make(map[string]int)
	for i, name := range priorityResult.Priority {
		priorityMap[strings.ToLower(name)] = i
	}

	sortedTopics := make([]models.Topic, len(topics))
	copy(sortedTopics, topics)
	
	// Einfache Bubble-Sort nach Priorität
	for i := 0; i < len(sortedTopics); i++ {
		for j := i + 1; j < len(sortedTopics); j++ {
			pi, oki := priorityMap[strings.ToLower(sortedTopics[i].Name)]
			pj, okj := priorityMap[strings.ToLower(sortedTopics[j].Name)]
			
			if !oki {
				pi = 999
			}
			if !okj {
				pj = 999
			}
			
			if pj < pi {
				sortedTopics[i], sortedTopics[j] = sortedTopics[j], sortedTopics[i]
			}
		}
	}

	log.Printf("   ✓ Themen nach Klausurrelevanz sortiert")
	return sortedTopics
}

// === Hilfsfunktionen ===

func deduplicateDocuments(docs []models.Document) []models.Document {
	seen := make(map[string]bool)
	var result []models.Document
	for _, doc := range docs {
		if !seen[doc.Name] {
			seen[doc.Name] = true
			result = append(result, doc)
		}
	}
	return result
}

func categorizeDocuments(docs []models.Document) (main, exams []models.Document) {
	for _, doc := range docs {
		nameLower := strings.ToLower(doc.Name)
		if strings.Contains(nameLower, "klausur") || strings.Contains(nameLower, "übung") {
			exams = append(exams, doc)
		} else {
			main = append(main, doc)
		}
	}
	return
}

func deduplicateTopics(topics []models.Topic) []models.Topic {
	seen := make(map[string]bool)
	var result []models.Topic
	for _, t := range topics {
		key := strings.ToLower(t.Name)
		if !seen[key] {
			seen[key] = true
			result = append(result, t)
		}
	}
	return result
}
