package api

import (
	"net/http"

	"github.com/gorilla/mux"
	"github.com/rs/cors"
)

// NewRouter erstellt den HTTP-Router mit allen Endpoints
func NewRouter(h *Handler) http.Handler {
	r := mux.NewRouter()

	// API-Version
	api := r.PathPrefix("/api/v1").Subrouter()

	// System
	api.HandleFunc("/health", h.HealthCheck).Methods("GET")
	api.HandleFunc("/status", h.GetStatus).Methods("GET")
	api.HandleFunc("/models", h.GetModels).Methods("GET")
	api.HandleFunc("/models", h.SetModel).Methods("POST")

	// Dokumente
	api.HandleFunc("/documents", h.GetDocuments).Methods("GET")
	api.HandleFunc("/documents", h.UploadDocument).Methods("POST")
	api.HandleFunc("/documents/scan", h.ScanDocumentsFolder).Methods("POST")
	api.HandleFunc("/documents/{id}", h.GetDocument).Methods("GET")
	api.HandleFunc("/documents/{id}", h.DeleteDocument).Methods("DELETE")

	// Lernpläne
	api.HandleFunc("/plans", h.GetStudyPlans).Methods("GET")
	api.HandleFunc("/plans", h.CreateStudyPlan).Methods("POST")
	api.HandleFunc("/plans/active", h.GetActiveStudyPlan).Methods("GET")
	api.HandleFunc("/plans/{id}", h.GetStudyPlan).Methods("GET")
	api.HandleFunc("/plans/{id}", h.UpdateStudyPlan).Methods("PUT")
	api.HandleFunc("/plans/{id}", h.DeleteStudyPlan).Methods("DELETE")

	// Themen
	api.HandleFunc("/topics/{id}", h.GetTopic).Methods("GET")
	api.HandleFunc("/topics/{id}/explain", h.ExplainTopic).Methods("GET")
	api.HandleFunc("/topics/{id}/questions", h.GetQuestions).Methods("GET")
	api.HandleFunc("/topics/{id}/questions/generate", h.GenerateQuestions).Methods("POST")
	api.HandleFunc("/topics/{id}/status", h.UpdateTopicStatus).Methods("PUT")

	// Fragen
	api.HandleFunc("/questions/{id}", h.GetQuestion).Methods("GET")
	api.HandleFunc("/questions/{id}/answer", h.SubmitAnswer).Methods("POST")

	// Chat
	api.HandleFunc("/chat", h.Chat).Methods("POST")
	api.HandleFunc("/chat/stream", h.ChatStream).Methods("POST")
	api.HandleFunc("/chat/history/{sessionId}", h.GetChatHistory).Methods("GET")

	// Fortschritt
	api.HandleFunc("/progress", h.GetProgress).Methods("GET")
	api.HandleFunc("/sessions", h.GetSessions).Methods("GET")
	api.HandleFunc("/sessions", h.StartSession).Methods("POST")
	api.HandleFunc("/sessions/{id}/end", h.EndSession).Methods("POST")

	// Glossar
	api.HandleFunc("/glossary", h.GetGlossary).Methods("GET")
	api.HandleFunc("/glossary", h.CreateGlossaryItem).Methods("POST")
	api.HandleFunc("/glossary/{id}", h.GetGlossaryItem).Methods("GET")
	api.HandleFunc("/glossary/{id}", h.UpdateGlossaryItem).Methods("PUT")
	api.HandleFunc("/glossary/{id}", h.DeleteGlossaryItem).Methods("DELETE")

	// Statische Dateien (Frontend)
	r.PathPrefix("/").Handler(http.FileServer(http.Dir("./web/static")))

	// CORS für lokale Entwicklung
	c := cors.New(cors.Options{
		AllowedOrigins:   []string{"*"},
		AllowedMethods:   []string{"GET", "POST", "PUT", "DELETE", "OPTIONS"},
		AllowedHeaders:   []string{"Content-Type", "Authorization"},
		AllowCredentials: true,
	})

	return c.Handler(r)
}
