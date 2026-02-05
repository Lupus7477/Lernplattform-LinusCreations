package models

import "time"

// Document repräsentiert ein hochgeladenes PDF-Dokument
type Document struct {
	ID          string    `json:"id"`
	Name        string    `json:"name"`
	Path        string    `json:"path"`
	Content     string    `json:"content,omitempty"`
	PageCount   int       `json:"page_count"`
	UploadedAt  time.Time `json:"uploaded_at"`
	ProcessedAt time.Time `json:"processed_at,omitempty"`
}

// Topic repräsentiert ein Lernthema/Kapitel
type Topic struct {
	ID          string     `json:"id"`
	StudyPlanID string     `json:"study_plan_id"`
	Name        string     `json:"name"`
	Description string     `json:"description"`
	Content     string     `json:"content,omitempty"`
	Order       int        `json:"order"`
	Difficulty  int        `json:"difficulty"` // 1-5
	EstMinutes  int        `json:"est_minutes"`
	Status      string     `json:"status"` // pending, in_progress, completed
	Progress    float64    `json:"progress"`
	Questions   []Question `json:"questions,omitempty"`
}

// Question repräsentiert eine Lernfrage
type Question struct {
	ID            string   `json:"id"`
	TopicID       string   `json:"topic_id"`
	Question      string   `json:"question"`
	ExpectedAnswer string  `json:"expected_answer"`
	Hints         []string `json:"hints,omitempty"`
	Difficulty    int      `json:"difficulty"` // 1-5
	Type          string   `json:"type"`       // multiple_choice, open, true_false
	Options       []string `json:"options,omitempty"`
	UserAnswer    string   `json:"user_answer,omitempty"`
	IsCorrect     *bool    `json:"is_correct,omitempty"`
	Feedback      string   `json:"feedback,omitempty"`
	AnsweredAt    *time.Time `json:"answered_at,omitempty"`
}

// StudyPlan repräsentiert einen Lernplan
type StudyPlan struct {
	ID           string    `json:"id"`
	Name         string    `json:"name"`
	ExamDate     time.Time `json:"exam_date"`
	CreatedAt    time.Time `json:"created_at"`
	TotalMinutes int       `json:"total_minutes"`
	Topics       []Topic   `json:"topics,omitempty"`
	Documents    []string  `json:"document_ids"`
	Status       string    `json:"status"` // active, completed, paused
	Progress     float64   `json:"progress"`
}

// StudySession repräsentiert eine Lernsitzung
type StudySession struct {
	ID          string    `json:"id"`
	StudyPlanID string    `json:"study_plan_id"`
	TopicID     string    `json:"topic_id"`
	StartedAt   time.Time `json:"started_at"`
	EndedAt     *time.Time `json:"ended_at,omitempty"`
	Duration    int       `json:"duration_minutes"`
	QuestionsAnswered int `json:"questions_answered"`
	CorrectAnswers    int `json:"correct_answers"`
}

// LearningProgress repräsentiert den Gesamtfortschritt
type LearningProgress struct {
	TotalTopics      int     `json:"total_topics"`
	CompletedTopics  int     `json:"completed_topics"`
	TotalQuestions   int     `json:"total_questions"`
	AnsweredQuestions int    `json:"answered_questions"`
	CorrectAnswers   int     `json:"correct_answers"`
	TotalStudyTime   int     `json:"total_study_time_minutes"`
	AverageScore     float64 `json:"average_score"`
	DaysUntilExam    int     `json:"days_until_exam"`
	OnTrack          bool    `json:"on_track"`
}

// ChatMessage repräsentiert eine Nachricht im Lern-Chat
type ChatMessage struct {
	ID        string    `json:"id"`
	SessionID string    `json:"session_id"`
	Role      string    `json:"role"` // user, assistant, system
	Content   string    `json:"content"`
	Timestamp time.Time `json:"timestamp"`
	TopicID   string    `json:"topic_id,omitempty"`
}

// Explanation repräsentiert eine Themenerklärung
type Explanation struct {
	TopicID     string   `json:"topic_id"`
	Title       string   `json:"title"`
	Content     string   `json:"content"`
	KeyPoints   []string `json:"key_points"`
	Examples    []string `json:"examples,omitempty"`
	SourcePages []int    `json:"source_pages,omitempty"`
}

// GlossaryItem repräsentiert einen Glossar-Eintrag
type GlossaryItem struct {
	ID         string   `json:"id"`
	Term       string   `json:"term"`
	Category   string   `json:"category"` // definition, formula, concept, abbreviation, other
	Definition string   `json:"definition"`
	Details    string   `json:"details,omitempty"`
	Related    []string `json:"related,omitempty"`
	CreatedAt  time.Time `json:"created_at"`
	UpdatedAt  time.Time `json:"updated_at"`
}
