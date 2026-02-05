package api

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"strconv"
	"sync"
	"time"

	"github.com/gorilla/mux"
	"github.com/gorilla/websocket"
	"lernplattform/internal/config"
	"lernplattform/internal/llm"
	"lernplattform/internal/models"
	"lernplattform/internal/pdf"
	"lernplattform/internal/storage"
)

// Handler verwaltet alle API-Endpunkte
type Handler struct {
	store      storage.Storage
	llm        llm.Provider
	tutor      *llm.Tutor
	pdfParser  *pdf.Parser
	config     *config.Config
	upgrader   websocket.Upgrader
}

// NewHandler erstellt einen neuen API-Handler
func NewHandler(store storage.Storage, llmProvider llm.Provider, cfg *config.Config) *Handler {
	// Schnelles Modell für Dokumentenanalyse, Hauptmodell für Chat/Quiz
	fastModel := "llama3.2:3b" // Schnell für Analyse
	numAgents := 1             // Sequentiell (Ollama-Limit)
	
	return &Handler{
		store:     store,
		llm:       llmProvider,
		tutor:     llm.NewTutorWithAgents(llmProvider, fastModel, numAgents),
		pdfParser: pdf.NewParser(cfg.DocumentsPath),
		config:    cfg,
		upgrader: websocket.Upgrader{
			CheckOrigin: func(r *http.Request) bool { return true },
		},
	}
}

// Response-Helper
func jsonResponse(w http.ResponseWriter, data interface{}, status int) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(data)
}

func errorResponse(w http.ResponseWriter, message string, status int) {
	jsonResponse(w, map[string]string{"error": message}, status)
}

// === System Endpoints ===

func (h *Handler) HealthCheck(w http.ResponseWriter, r *http.Request) {
	ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
	defer cancel()

	llmAvailable := h.llm.IsAvailable(ctx)

	jsonResponse(w, map[string]interface{}{
		"status":        "ok",
		"llm_available": llmAvailable,
		"llm_provider":  h.llm.GetName(),
		"timestamp":     time.Now(),
	}, http.StatusOK)
}

func (h *Handler) GetStatus(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()

	docs, _ := h.store.GetAllDocuments()
	plans, _ := h.store.GetAllStudyPlans()
	llmAvailable := h.llm.IsAvailable(ctx)

	var activePlan *models.StudyPlan
	for _, p := range plans {
		if p.Status == "active" {
			activePlan = &p
			break
		}
	}

	jsonResponse(w, map[string]interface{}{
		"documents_count":   len(docs),
		"study_plans_count": len(plans),
		"active_plan":       activePlan,
		"llm_available":     llmAvailable,
		"llm_provider":      h.llm.GetName(),
		"documents_path":    h.config.DocumentsPath,
	}, http.StatusOK)
}

func (h *Handler) GetModels(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()

	models, err := h.llm.GetModels(ctx)
	if err != nil {
		errorResponse(w, fmt.Sprintf("Konnte Modelle nicht abrufen: %v", err), http.StatusServiceUnavailable)
		return
	}

	jsonResponse(w, map[string]interface{}{
		"models":        models,
		"current_model": h.llm.GetCurrentModel(),
	}, http.StatusOK)
}

// SetModel ändert das aktive LLM-Modell
func (h *Handler) SetModel(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Model string `json:"model"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	if req.Model == "" {
		errorResponse(w, "Kein Modell angegeben", http.StatusBadRequest)
		return
	}

	// Prüfe ob das Modell existiert
	ctx := r.Context()
	models, err := h.llm.GetModels(ctx)
	if err != nil {
		errorResponse(w, "Konnte Modelle nicht abrufen", http.StatusServiceUnavailable)
		return
	}

	found := false
	for _, m := range models {
		if m.Name == req.Model {
			found = true
			break
		}
	}

	if !found {
		errorResponse(w, fmt.Sprintf("Modell '%s' nicht gefunden", req.Model), http.StatusBadRequest)
		return
	}

	// Setze das neue Modell
	h.llm.SetModel(req.Model)
	h.config.DefaultModel = req.Model

	jsonResponse(w, map[string]interface{}{
		"message":       "Modell geändert",
		"current_model": req.Model,
	}, http.StatusOK)
}

// === Dokument Endpoints ===

func (h *Handler) GetDocuments(w http.ResponseWriter, r *http.Request) {
	docs, err := h.store.GetAllDocuments()
	if err != nil {
		errorResponse(w, "Fehler beim Laden der Dokumente", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, map[string]interface{}{
		"documents": docs,
		"count":     len(docs),
	}, http.StatusOK)
}

func (h *Handler) UploadDocument(w http.ResponseWriter, r *http.Request) {
	// Max 50MB
	r.ParseMultipartForm(50 << 20)

	file, header, err := r.FormFile("file")
	if err != nil {
		errorResponse(w, "Keine Datei gefunden", http.StatusBadRequest)
		return
	}
	defer file.Close()

	doc, err := h.pdfParser.ParseFromReader(file, header.Filename)
	if err != nil {
		errorResponse(w, fmt.Sprintf("Fehler beim Parsen: %v", err), http.StatusBadRequest)
		return
	}

	if err := h.store.SaveDocument(doc); err != nil {
		errorResponse(w, "Fehler beim Speichern", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, doc, http.StatusCreated)
}

func (h *Handler) ScanDocumentsFolder(w http.ResponseWriter, r *http.Request) {
	path := h.config.DocumentsPath

	// Optional: Pfad aus Request
	var req struct {
		Path string `json:"path"`
	}
	json.NewDecoder(r.Body).Decode(&req)
	if req.Path != "" {
		path = req.Path
	}

	docs, err := h.pdfParser.ParseDirectory(path)
	if err != nil {
		errorResponse(w, fmt.Sprintf("Fehler beim Scannen: %v", err), http.StatusInternalServerError)
		return
	}

	// Dokumente speichern
	for _, doc := range docs {
		h.store.SaveDocument(&doc)
	}

	jsonResponse(w, map[string]interface{}{
		"message":   fmt.Sprintf("%d Dokumente gefunden und verarbeitet", len(docs)),
		"documents": docs,
	}, http.StatusOK)
}

func (h *Handler) GetDocument(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	doc, err := h.store.GetDocument(id)
	if err != nil {
		errorResponse(w, "Dokument nicht gefunden", http.StatusNotFound)
		return
	}

	jsonResponse(w, doc, http.StatusOK)
}

func (h *Handler) DeleteDocument(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	if err := h.store.DeleteDocument(id); err != nil {
		errorResponse(w, "Fehler beim Löschen", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, map[string]string{"message": "Dokument gelöscht"}, http.StatusOK)
}

// === Lernplan Endpoints ===

func (h *Handler) GetStudyPlans(w http.ResponseWriter, r *http.Request) {
	plans, err := h.store.GetAllStudyPlans()
	if err != nil {
		errorResponse(w, "Fehler beim Laden", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, plans, http.StatusOK)
}

// studyPlanMutex verhindert parallele Lernplan-Erstellung
var studyPlanMutex sync.Mutex
var studyPlanInProgress bool

func (h *Handler) CreateStudyPlan(w http.ResponseWriter, r *http.Request) {
	// Verhindere parallele Requests
	studyPlanMutex.Lock()
	if studyPlanInProgress {
		studyPlanMutex.Unlock()
		log.Println("⚠️ Lernplan-Erstellung läuft bereits, ignoriere Anfrage")
		errorResponse(w, "Lernplan wird bereits erstellt, bitte warten", http.StatusTooManyRequests)
		return
	}
	studyPlanInProgress = true
	studyPlanMutex.Unlock()
	
	defer func() {
		studyPlanMutex.Lock()
		studyPlanInProgress = false
		studyPlanMutex.Unlock()
	}()

	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
	log.Println("📋 LERNPLAN ERSTELLEN - Start")
	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
	
	var req struct {
		ExamDate    string   `json:"exam_date"`
		DocumentIDs []string `json:"document_ids"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		log.Printf("❌ Fehler: Ungültige Anfrage - %v", err)
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	log.Printf("📅 Prüfungsdatum: %s", req.ExamDate)
	log.Printf("📄 Dokument-IDs: %v", req.DocumentIDs)

	examDate, err := time.Parse("2006-01-02", req.ExamDate)
	if err != nil {
		log.Printf("❌ Fehler: Ungültiges Datum - %v", err)
		errorResponse(w, "Ungültiges Datum (Format: YYYY-MM-DD)", http.StatusBadRequest)
		return
	}

	// Dokumente laden
	log.Println("📚 Lade Dokumente...")
	var docs []models.Document
	var allContent string
	for _, id := range req.DocumentIDs {
		doc, err := h.store.GetDocument(id)
		if err == nil {
			log.Printf("   ✓ Geladen: %s (%d Zeichen)", doc.Name, len(doc.Content))
			docs = append(docs, *doc)
			allContent += doc.Content + "\n"
		} else {
			log.Printf("   ✗ Fehler bei ID %s: %v", id, err)
		}
	}

	if len(docs) == 0 {
		log.Println("❌ Fehler: Keine gültigen Dokumente gefunden")
		errorResponse(w, "Keine gültigen Dokumente gefunden", http.StatusBadRequest)
		return
	}

	log.Printf("✓ %d Dokumente geladen, Gesamtinhalt: %d Zeichen", len(docs), len(allContent))

	// Eigener Context mit langem Timeout (nicht abhängig vom HTTP-Request)
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Minute)
	defer cancel()

	// Themen analysieren
	log.Println("")
	log.Println("🤖 SCHRITT 1: Analysiere Dokumente mit KI...")
	log.Printf("   Verwende Modell: %s", h.llm.GetCurrentModel())
	log.Println("   ⏳ Dies kann einige Minuten dauern (max. 15 Min)...")
	
	startAnalyze := time.Now()
	topics, err := h.tutor.AnalyzeDocuments(ctx, docs)
	if err != nil {
		log.Printf("❌ Fehler bei der Analyse: %v", err)
		errorResponse(w, fmt.Sprintf("Fehler bei der Analyse: %v", err), http.StatusInternalServerError)
		return
	}
	log.Printf("✓ Analyse abgeschlossen in %v", time.Since(startAnalyze))
	log.Printf("   Gefundene Themen: %d", len(topics))
	for i, t := range topics {
		log.Printf("   %d. %s", i+1, t.Name)
	}

	// Lernplan erstellen
	log.Println("")
	log.Println("📝 SCHRITT 2: Erstelle Lernplan...")
	plan, err := h.tutor.CreateStudyPlan(ctx, topics, examDate, allContent)
	if err != nil {
		log.Printf("❌ Fehler beim Erstellen des Lernplans: %v", err)
		errorResponse(w, fmt.Sprintf("Fehler beim Erstellen des Lernplans: %v", err), http.StatusInternalServerError)
		return
	}
	log.Printf("✓ Lernplan erstellt: %s", plan.Name)

	plan.Documents = req.DocumentIDs

	// Speichern
	log.Println("")
	log.Println("💾 SCHRITT 3: Speichere in Datenbank...")
	if err := h.store.SaveStudyPlan(plan); err != nil {
		log.Printf("❌ Fehler beim Speichern des Lernplans: %v", err)
		errorResponse(w, "Fehler beim Speichern", http.StatusInternalServerError)
		return
	}
	log.Println("   ✓ Lernplan gespeichert")

	// Themen speichern
	for _, topic := range plan.Topics {
		if err := h.store.SaveTopic(&topic); err != nil {
			log.Printf("   ✗ Fehler beim Speichern von Thema '%s': %v", topic.Name, err)
		} else {
			log.Printf("   ✓ Thema gespeichert: %s", topic.Name)
		}
	}

	log.Println("")
	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
	log.Println("✅ LERNPLAN ERFOLGREICH ERSTELLT!")
	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

	jsonResponse(w, plan, http.StatusCreated)
}

func (h *Handler) GetActiveStudyPlan(w http.ResponseWriter, r *http.Request) {
	plan, err := h.store.GetActiveStudyPlan()
	if err != nil {
		errorResponse(w, "Kein aktiver Lernplan", http.StatusNotFound)
		return
	}

	jsonResponse(w, plan, http.StatusOK)
}

func (h *Handler) GetStudyPlan(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	plan, err := h.store.GetStudyPlan(id)
	if err != nil {
		errorResponse(w, "Lernplan nicht gefunden", http.StatusNotFound)
		return
	}

	jsonResponse(w, plan, http.StatusOK)
}

func (h *Handler) UpdateStudyPlan(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	var req struct {
		Status   string  `json:"status"`
		Progress float64 `json:"progress"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	if req.Status != "" {
		// Status-Update würde hier implementiert
	}
	if req.Progress > 0 {
		h.store.UpdateStudyPlanProgress(id, req.Progress)
	}

	plan, _ := h.store.GetStudyPlan(id)
	jsonResponse(w, plan, http.StatusOK)
}

func (h *Handler) DeleteStudyPlan(w http.ResponseWriter, r *http.Request) {
	// Implementierung
	jsonResponse(w, map[string]string{"message": "Lernplan gelöscht"}, http.StatusOK)
}

// === Themen Endpoints ===

func (h *Handler) GetTopic(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	topic, err := h.store.GetTopic(id)
	if err != nil {
		errorResponse(w, "Thema nicht gefunden", http.StatusNotFound)
		return
	}

	jsonResponse(w, topic, http.StatusOK)
}

func (h *Handler) ExplainTopic(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	topic, err := h.store.GetTopic(id)
	if err != nil {
		errorResponse(w, "Thema nicht gefunden", http.StatusNotFound)
		return
	}

	// Dokumentinhalt für Kontext laden
	plan, _ := h.store.GetStudyPlan(topic.StudyPlanID)
	var content string
	if plan != nil {
		for _, docID := range plan.Documents {
			doc, _ := h.store.GetDocument(docID)
			if doc != nil {
				content += doc.Content + "\n"
			}
		}
	}

	ctx := r.Context()
	explanation, err := h.tutor.ExplainTopic(ctx, topic, content)
	if err != nil {
		errorResponse(w, fmt.Sprintf("Fehler bei der Erklärung: %v", err), http.StatusInternalServerError)
		return
	}

	jsonResponse(w, explanation, http.StatusOK)
}

func (h *Handler) GetQuestions(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]
	
	// Optional: Nach Schwierigkeit filtern
	difficultyStr := r.URL.Query().Get("difficulty")

	questions, err := h.store.GetQuestionsByTopic(id)
	if err != nil {
		errorResponse(w, "Fehler beim Laden", http.StatusInternalServerError)
		return
	}
	
	// Filtere nach Schwierigkeit wenn angegeben
	if difficultyStr != "" {
		difficulty := 0
		fmt.Sscanf(difficultyStr, "%d", &difficulty)
		if difficulty > 0 {
			filtered := make([]models.Question, 0)
			for _, q := range questions {
				if q.Difficulty == difficulty {
					filtered = append(filtered, q)
				}
			}
			questions = filtered
		}
	}

	jsonResponse(w, questions, http.StatusOK)
}

func (h *Handler) GenerateQuestions(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	var req struct {
		Difficulty int `json:"difficulty"`
		Count      int `json:"count"`
	}
	json.NewDecoder(r.Body).Decode(&req)
	if req.Difficulty < 1 || req.Difficulty > 5 {
		req.Difficulty = 1
	}
	if req.Count <= 0 || req.Count > 10 {
		req.Count = 3 // Standard: 3 Fragen
	}

	topic, err := h.store.GetTopic(id)
	if err != nil {
		errorResponse(w, "Thema nicht gefunden", http.StatusNotFound)
		return
	}

	// Dokumentinhalt laden
	plan, _ := h.store.GetStudyPlan(topic.StudyPlanID)
	var content string
	if plan != nil {
		for _, docID := range plan.Documents {
			doc, _ := h.store.GetDocument(docID)
			if doc != nil {
				content += doc.Content + "\n"
			}
		}
	}

	ctx := r.Context()
	questions, err := h.tutor.GenerateQuestions(ctx, topic, content, req.Difficulty, req.Count)
	if err != nil {
		errorResponse(w, fmt.Sprintf("Fehler bei der Generierung: %v", err), http.StatusInternalServerError)
		return
	}

	// Fragen speichern
	for _, q := range questions {
		h.store.SaveQuestion(&q)
	}

	jsonResponse(w, questions, http.StatusCreated)
}

func (h *Handler) UpdateTopicStatus(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	var req struct {
		Status   string  `json:"status"`
		Progress float64 `json:"progress"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	if err := h.store.UpdateTopicStatus(id, req.Status, req.Progress); err != nil {
		errorResponse(w, "Fehler beim Update", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, map[string]string{"message": "Status aktualisiert"}, http.StatusOK)
}

// === Fragen Endpoints ===

func (h *Handler) GetQuestion(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	question, err := h.store.GetQuestion(id)
	if err != nil {
		errorResponse(w, "Frage nicht gefunden", http.StatusNotFound)
		return
	}

	jsonResponse(w, question, http.StatusOK)
}

func (h *Handler) SubmitAnswer(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	var req struct {
		Answer string `json:"answer"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	question, err := h.store.GetQuestion(id)
	if err != nil {
		errorResponse(w, "Frage nicht gefunden", http.StatusNotFound)
		return
	}

	// Dokumentinhalt für Bewertung laden
	topic, _ := h.store.GetTopic(question.TopicID)
	var content string
	if topic != nil {
		plan, _ := h.store.GetStudyPlan(topic.StudyPlanID)
		if plan != nil {
			for _, docID := range plan.Documents {
				doc, _ := h.store.GetDocument(docID)
				if doc != nil {
					content += doc.Content + "\n"
				}
			}
		}
	}

	ctx := r.Context()
	isCorrect, feedback, err := h.tutor.EvaluateAnswer(ctx, question, req.Answer, content)
	if err != nil {
		errorResponse(w, fmt.Sprintf("Fehler bei der Bewertung: %v", err), http.StatusInternalServerError)
		return
	}

	// Antwort speichern
	h.store.SaveQuestionAnswer(id, req.Answer, isCorrect, feedback)

	jsonResponse(w, map[string]interface{}{
		"is_correct": isCorrect,
		"feedback":   feedback,
		"expected":   question.ExpectedAnswer,
	}, http.StatusOK)
}

// === Chat Endpoints ===

func (h *Handler) Chat(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Message   string `json:"message"`
		TopicID   string `json:"topic_id"`
		SessionID string `json:"session_id"`
	}

	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	// Topic und Kontext laden
	topic, _ := h.store.GetTopic(req.TopicID)
	if topic == nil {
		topic = &models.Topic{Name: "Allgemein", Description: "Allgemeine Lernfragen"}
	}

	var content string
	if topic.StudyPlanID != "" {
		plan, _ := h.store.GetStudyPlan(topic.StudyPlanID)
		if plan != nil {
			for _, docID := range plan.Documents {
				doc, _ := h.store.GetDocument(docID)
				if doc != nil {
					content += doc.Content + "\n"
				}
			}
		}
	}

	// Chat-Historie laden
	var messages []llm.ChatMessage
	if req.SessionID != "" {
		history, _ := h.store.GetChatHistory(req.SessionID)
		for _, msg := range history {
			messages = append(messages, llm.ChatMessage{
				Role:    msg.Role,
				Content: msg.Content,
			})
		}
	}

	// Neue Nachricht hinzufügen
	messages = append(messages, llm.ChatMessage{
		Role:    "user",
		Content: req.Message,
	})

	ctx := r.Context()
	resp, err := h.tutor.ChatWithContext(ctx, messages, content, topic)
	if err != nil {
		errorResponse(w, fmt.Sprintf("Chat-Fehler: %v", err), http.StatusInternalServerError)
		return
	}

	// Nachrichten speichern
	if req.SessionID != "" {
		h.store.SaveChatMessage(&models.ChatMessage{
			ID:        fmt.Sprintf("msg_%d", time.Now().UnixNano()),
			SessionID: req.SessionID,
			Role:      "user",
			Content:   req.Message,
			Timestamp: time.Now(),
			TopicID:   req.TopicID,
		})
		h.store.SaveChatMessage(&models.ChatMessage{
			ID:        fmt.Sprintf("msg_%d", time.Now().UnixNano()+1),
			SessionID: req.SessionID,
			Role:      "assistant",
			Content:   resp.Content,
			Timestamp: time.Now(),
			TopicID:   req.TopicID,
		})
	}

	jsonResponse(w, map[string]interface{}{
		"response": resp.Content,
		"model":    resp.Model,
	}, http.StatusOK)
}

func (h *Handler) ChatStream(w http.ResponseWriter, r *http.Request) {
	// WebSocket für Streaming
	conn, err := h.upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	defer conn.Close()

	// Nachricht empfangen
	var req struct {
		Message   string `json:"message"`
		TopicID   string `json:"topic_id"`
	}

	if err := conn.ReadJSON(&req); err != nil {
		return
	}

	// Streaming-Antwort
	ctx := r.Context()
	chunks, err := h.llm.GenerateStream(ctx, req.Message, nil)
	if err != nil {
		conn.WriteJSON(map[string]string{"error": err.Error()})
		return
	}

	for chunk := range chunks {
		if chunk.Error != nil {
			conn.WriteJSON(map[string]string{"error": chunk.Error.Error()})
			return
		}
		conn.WriteJSON(map[string]interface{}{
			"content": chunk.Content,
			"done":    chunk.Done,
		})
	}
}

func (h *Handler) GetChatHistory(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	sessionID := vars["sessionId"]

	messages, err := h.store.GetChatHistory(sessionID)
	if err != nil {
		errorResponse(w, "Fehler beim Laden", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, messages, http.StatusOK)
}

// === Fortschritt Endpoints ===

func (h *Handler) GetProgress(w http.ResponseWriter, r *http.Request) {
	plan, err := h.store.GetActiveStudyPlan()
	if err != nil {
		errorResponse(w, "Kein aktiver Lernplan", http.StatusNotFound)
		return
	}

	topics := plan.Topics
	var completed, totalQuestions, answeredQuestions, correctAnswers int

	for _, topic := range topics {
		if topic.Status == "completed" {
			completed++
		}
		questions, _ := h.store.GetQuestionsByTopic(topic.ID)
		totalQuestions += len(questions)
		for _, q := range questions {
			if q.AnsweredAt != nil {
				answeredQuestions++
				if q.IsCorrect != nil && *q.IsCorrect {
					correctAnswers++
				}
			}
		}
	}

	daysUntilExam := int(time.Until(plan.ExamDate).Hours() / 24)
	if daysUntilExam < 0 {
		daysUntilExam = 0
	}

	var avgScore float64
	if answeredQuestions > 0 {
		avgScore = float64(correctAnswers) / float64(answeredQuestions) * 100
	}

	progress := models.LearningProgress{
		TotalTopics:       len(topics),
		CompletedTopics:   completed,
		TotalQuestions:    totalQuestions,
		AnsweredQuestions: answeredQuestions,
		CorrectAnswers:    correctAnswers,
		AverageScore:      avgScore,
		DaysUntilExam:     daysUntilExam,
		OnTrack:           float64(completed)/float64(len(topics))*100 >= float64(100-daysUntilExam),
	}

	jsonResponse(w, progress, http.StatusOK)
}

func (h *Handler) GetSessions(w http.ResponseWriter, r *http.Request) {
	planID := r.URL.Query().Get("plan_id")
	if planID == "" {
		plan, _ := h.store.GetActiveStudyPlan()
		if plan != nil {
			planID = plan.ID
		}
	}

	sessions, err := h.store.GetSessionsByPlan(planID)
	if err != nil {
		errorResponse(w, "Fehler beim Laden", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, sessions, http.StatusOK)
}

func (h *Handler) StartSession(w http.ResponseWriter, r *http.Request) {
	var req struct {
		TopicID string `json:"topic_id"`
	}
	json.NewDecoder(r.Body).Decode(&req)

	plan, _ := h.store.GetActiveStudyPlan()
	planID := ""
	if plan != nil {
		planID = plan.ID
	}

	session := &models.StudySession{
		ID:          fmt.Sprintf("session_%d", time.Now().UnixNano()),
		StudyPlanID: planID,
		TopicID:     req.TopicID,
		StartedAt:   time.Now(),
	}

	h.store.SaveSession(session)
	jsonResponse(w, session, http.StatusCreated)
}

func (h *Handler) EndSession(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	var req struct {
		QuestionsAnswered int `json:"questions_answered"`
		CorrectAnswers    int `json:"correct_answers"`
	}
	json.NewDecoder(r.Body).Decode(&req)

	// Session aktualisieren (vereinfacht)
	_ = id
	_ = req

	jsonResponse(w, map[string]string{"message": "Session beendet"}, http.StatusOK)
}

// Hilfsfunktion für optionale Query-Parameter
func getQueryInt(r *http.Request, key string, defaultVal int) int {
	val := r.URL.Query().Get(key)
	if val == "" {
		return defaultVal
	}
	i, err := strconv.Atoi(val)
	if err != nil {
		return defaultVal
	}
	return i
}

// === Glossar Handlers ===

func (h *Handler) GetGlossary(w http.ResponseWriter, r *http.Request) {
	items, err := h.store.GetAllGlossaryItems()
	if err != nil {
		jsonResponse(w, []models.GlossaryItem{}, http.StatusOK)
		return
	}
	jsonResponse(w, items, http.StatusOK)
}

func (h *Handler) CreateGlossaryItem(w http.ResponseWriter, r *http.Request) {
	var item models.GlossaryItem
	if err := json.NewDecoder(r.Body).Decode(&item); err != nil {
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	item.ID = fmt.Sprintf("%d", time.Now().UnixNano())
	item.CreatedAt = time.Now()
	item.UpdatedAt = time.Now()

	if err := h.store.SaveGlossaryItem(&item); err != nil {
		errorResponse(w, "Fehler beim Speichern", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, item, http.StatusCreated)
}

func (h *Handler) GetGlossaryItem(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	item, err := h.store.GetGlossaryItem(id)
	if err != nil {
		errorResponse(w, "Begriff nicht gefunden", http.StatusNotFound)
		return
	}

	jsonResponse(w, item, http.StatusOK)
}

func (h *Handler) UpdateGlossaryItem(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	var item models.GlossaryItem
	if err := json.NewDecoder(r.Body).Decode(&item); err != nil {
		errorResponse(w, "Ungültige Anfrage", http.StatusBadRequest)
		return
	}

	item.ID = id
	item.UpdatedAt = time.Now()

	if err := h.store.SaveGlossaryItem(&item); err != nil {
		errorResponse(w, "Fehler beim Aktualisieren", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, item, http.StatusOK)
}

func (h *Handler) DeleteGlossaryItem(w http.ResponseWriter, r *http.Request) {
	vars := mux.Vars(r)
	id := vars["id"]

	if err := h.store.DeleteGlossaryItem(id); err != nil {
		errorResponse(w, "Fehler beim Löschen", http.StatusInternalServerError)
		return
	}

	jsonResponse(w, map[string]string{"message": "Gelöscht"}, http.StatusOK)
}

// Placeholder für io import
var _ = io.EOF
