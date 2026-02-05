package storage

import (
	"database/sql"
	"encoding/json"
	"time"

	"lernplattform/internal/models"

	_ "modernc.org/sqlite"
)

// Storage definiert das Interface für Datenpersistenz
type Storage interface {
	// Dokumente
	SaveDocument(doc *models.Document) error
	GetDocument(id string) (*models.Document, error)
	GetAllDocuments() ([]models.Document, error)
	DeleteDocument(id string) error

	// Lernpläne
	SaveStudyPlan(plan *models.StudyPlan) error
	GetStudyPlan(id string) (*models.StudyPlan, error)
	GetActiveStudyPlan() (*models.StudyPlan, error)
	GetAllStudyPlans() ([]models.StudyPlan, error)
	UpdateStudyPlanProgress(id string, progress float64) error

	// Themen
	SaveTopic(topic *models.Topic) error
	GetTopic(id string) (*models.Topic, error)
	GetTopicsByPlan(planID string) ([]models.Topic, error)
	UpdateTopicStatus(id string, status string, progress float64) error

	// Fragen
	SaveQuestion(q *models.Question) error
	GetQuestion(id string) (*models.Question, error)
	GetQuestionsByTopic(topicID string) ([]models.Question, error)
	SaveQuestionAnswer(id string, answer string, isCorrect bool, feedback string) error

	// Sitzungen
	SaveSession(session *models.StudySession) error
	GetSessionsByPlan(planID string) ([]models.StudySession, error)

	// Chat
	SaveChatMessage(msg *models.ChatMessage) error
	GetChatHistory(sessionID string) ([]models.ChatMessage, error)

	// Glossar
	SaveGlossaryItem(item *models.GlossaryItem) error
	GetGlossaryItem(id string) (*models.GlossaryItem, error)
	GetAllGlossaryItems() ([]models.GlossaryItem, error)
	DeleteGlossaryItem(id string) error

	Close() error
}

// SQLiteStorage implementiert Storage mit SQLite
type SQLiteStorage struct {
	db *sql.DB
}

// NewSQLiteStorage erstellt eine neue SQLite-Storage-Instanz
func NewSQLiteStorage(dbPath string) (*SQLiteStorage, error) {
	db, err := sql.Open("sqlite", dbPath)
	if err != nil {
		return nil, err
	}

	storage := &SQLiteStorage{db: db}
	if err := storage.initSchema(); err != nil {
		return nil, err
	}

	return storage, nil
}

func (s *SQLiteStorage) initSchema() error {
	schema := `
	CREATE TABLE IF NOT EXISTS documents (
		id TEXT PRIMARY KEY,
		name TEXT NOT NULL,
		path TEXT NOT NULL,
		content TEXT,
		page_count INTEGER,
		uploaded_at DATETIME,
		processed_at DATETIME
	);

	CREATE TABLE IF NOT EXISTS study_plans (
		id TEXT PRIMARY KEY,
		name TEXT NOT NULL,
		exam_date DATETIME NOT NULL,
		created_at DATETIME NOT NULL,
		total_minutes INTEGER,
		document_ids TEXT,
		status TEXT DEFAULT 'active',
		progress REAL DEFAULT 0
	);

	CREATE TABLE IF NOT EXISTS topics (
		id TEXT PRIMARY KEY,
		study_plan_id TEXT NOT NULL,
		name TEXT NOT NULL,
		description TEXT,
		content TEXT,
		topic_order INTEGER,
		difficulty INTEGER DEFAULT 1,
		est_minutes INTEGER,
		status TEXT DEFAULT 'pending',
		progress REAL DEFAULT 0,
		FOREIGN KEY (study_plan_id) REFERENCES study_plans(id)
	);

	CREATE TABLE IF NOT EXISTS questions (
		id TEXT PRIMARY KEY,
		topic_id TEXT NOT NULL,
		question TEXT NOT NULL,
		expected_answer TEXT,
		hints TEXT,
		difficulty INTEGER DEFAULT 1,
		type TEXT DEFAULT 'open',
		options TEXT,
		user_answer TEXT,
		is_correct INTEGER,
		feedback TEXT,
		answered_at DATETIME,
		FOREIGN KEY (topic_id) REFERENCES topics(id)
	);

	CREATE TABLE IF NOT EXISTS study_sessions (
		id TEXT PRIMARY KEY,
		study_plan_id TEXT NOT NULL,
		topic_id TEXT,
		started_at DATETIME NOT NULL,
		ended_at DATETIME,
		duration_minutes INTEGER,
		questions_answered INTEGER DEFAULT 0,
		correct_answers INTEGER DEFAULT 0,
		FOREIGN KEY (study_plan_id) REFERENCES study_plans(id)
	);

	CREATE TABLE IF NOT EXISTS chat_messages (
		id TEXT PRIMARY KEY,
		session_id TEXT NOT NULL,
		role TEXT NOT NULL,
		content TEXT NOT NULL,
		timestamp DATETIME NOT NULL,
		topic_id TEXT
	);

	CREATE INDEX IF NOT EXISTS idx_topics_plan ON topics(study_plan_id);
	CREATE INDEX IF NOT EXISTS idx_questions_topic ON questions(topic_id);
	CREATE INDEX IF NOT EXISTS idx_sessions_plan ON study_sessions(study_plan_id);
	CREATE INDEX IF NOT EXISTS idx_chat_session ON chat_messages(session_id);

	CREATE TABLE IF NOT EXISTS glossary (
		id TEXT PRIMARY KEY,
		term TEXT NOT NULL,
		category TEXT DEFAULT 'definition',
		definition TEXT NOT NULL,
		details TEXT,
		related TEXT,
		created_at DATETIME NOT NULL,
		updated_at DATETIME NOT NULL
	);
	CREATE INDEX IF NOT EXISTS idx_glossary_term ON glossary(term);
	`

	_, err := s.db.Exec(schema)
	return err
}

func (s *SQLiteStorage) Close() error {
	return s.db.Close()
}

// Dokumente

func (s *SQLiteStorage) SaveDocument(doc *models.Document) error {
	_, err := s.db.Exec(`
		INSERT OR REPLACE INTO documents (id, name, path, content, page_count, uploaded_at, processed_at)
		VALUES (?, ?, ?, ?, ?, ?, ?)
	`, doc.ID, doc.Name, doc.Path, doc.Content, doc.PageCount, doc.UploadedAt, doc.ProcessedAt)
	return err
}

func (s *SQLiteStorage) GetDocument(id string) (*models.Document, error) {
	var doc models.Document
	err := s.db.QueryRow(`
		SELECT id, name, path, content, page_count, uploaded_at, processed_at
		FROM documents WHERE id = ?
	`, id).Scan(&doc.ID, &doc.Name, &doc.Path, &doc.Content, &doc.PageCount, &doc.UploadedAt, &doc.ProcessedAt)
	if err != nil {
		return nil, err
	}
	return &doc, nil
}

func (s *SQLiteStorage) GetAllDocuments() ([]models.Document, error) {
	rows, err := s.db.Query(`SELECT id, name, path, page_count, uploaded_at, processed_at FROM documents`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var docs []models.Document
	for rows.Next() {
		var doc models.Document
		if err := rows.Scan(&doc.ID, &doc.Name, &doc.Path, &doc.PageCount, &doc.UploadedAt, &doc.ProcessedAt); err != nil {
			return nil, err
		}
		docs = append(docs, doc)
	}
	return docs, nil
}

func (s *SQLiteStorage) DeleteDocument(id string) error {
	_, err := s.db.Exec(`DELETE FROM documents WHERE id = ?`, id)
	return err
}

// Lernpläne

func (s *SQLiteStorage) SaveStudyPlan(plan *models.StudyPlan) error {
	docIDs, _ := json.Marshal(plan.Documents)
	_, err := s.db.Exec(`
		INSERT OR REPLACE INTO study_plans (id, name, exam_date, created_at, total_minutes, document_ids, status, progress)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?)
	`, plan.ID, plan.Name, plan.ExamDate, plan.CreatedAt, plan.TotalMinutes, string(docIDs), plan.Status, plan.Progress)
	return err
}

func (s *SQLiteStorage) GetStudyPlan(id string) (*models.StudyPlan, error) {
	var plan models.StudyPlan
	var docIDs string
	err := s.db.QueryRow(`
		SELECT id, name, exam_date, created_at, total_minutes, document_ids, status, progress
		FROM study_plans WHERE id = ?
	`, id).Scan(&plan.ID, &plan.Name, &plan.ExamDate, &plan.CreatedAt, &plan.TotalMinutes, &docIDs, &plan.Status, &plan.Progress)
	if err != nil {
		return nil, err
	}
	json.Unmarshal([]byte(docIDs), &plan.Documents)

	// Themen laden
	plan.Topics, _ = s.GetTopicsByPlan(plan.ID)
	return &plan, nil
}

func (s *SQLiteStorage) GetActiveStudyPlan() (*models.StudyPlan, error) {
	var plan models.StudyPlan
	var docIDs string
	err := s.db.QueryRow(`
		SELECT id, name, exam_date, created_at, total_minutes, document_ids, status, progress
		FROM study_plans WHERE status = 'active' ORDER BY created_at DESC LIMIT 1
	`).Scan(&plan.ID, &plan.Name, &plan.ExamDate, &plan.CreatedAt, &plan.TotalMinutes, &docIDs, &plan.Status, &plan.Progress)
	if err != nil {
		return nil, err
	}
	json.Unmarshal([]byte(docIDs), &plan.Documents)
	plan.Topics, _ = s.GetTopicsByPlan(plan.ID)
	return &plan, nil
}

func (s *SQLiteStorage) GetAllStudyPlans() ([]models.StudyPlan, error) {
	rows, err := s.db.Query(`
		SELECT id, name, exam_date, created_at, total_minutes, document_ids, status, progress
		FROM study_plans ORDER BY created_at DESC
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var plans []models.StudyPlan
	for rows.Next() {
		var plan models.StudyPlan
		var docIDs string
		if err := rows.Scan(&plan.ID, &plan.Name, &plan.ExamDate, &plan.CreatedAt, &plan.TotalMinutes, &docIDs, &plan.Status, &plan.Progress); err != nil {
			return nil, err
		}
		json.Unmarshal([]byte(docIDs), &plan.Documents)
		plans = append(plans, plan)
	}
	return plans, nil
}

func (s *SQLiteStorage) UpdateStudyPlanProgress(id string, progress float64) error {
	_, err := s.db.Exec(`UPDATE study_plans SET progress = ? WHERE id = ?`, progress, id)
	return err
}

// Themen

func (s *SQLiteStorage) SaveTopic(topic *models.Topic) error {
	_, err := s.db.Exec(`
		INSERT OR REPLACE INTO topics (id, study_plan_id, name, description, content, topic_order, difficulty, est_minutes, status, progress)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, topic.ID, topic.StudyPlanID, topic.Name, topic.Description, topic.Content, topic.Order, topic.Difficulty, topic.EstMinutes, topic.Status, topic.Progress)
	return err
}

func (s *SQLiteStorage) GetTopic(id string) (*models.Topic, error) {
	var topic models.Topic
	err := s.db.QueryRow(`
		SELECT id, study_plan_id, name, description, content, topic_order, difficulty, est_minutes, status, progress
		FROM topics WHERE id = ?
	`, id).Scan(&topic.ID, &topic.StudyPlanID, &topic.Name, &topic.Description, &topic.Content, &topic.Order, &topic.Difficulty, &topic.EstMinutes, &topic.Status, &topic.Progress)
	if err != nil {
		return nil, err
	}
	topic.Questions, _ = s.GetQuestionsByTopic(topic.ID)
	return &topic, nil
}

func (s *SQLiteStorage) GetTopicsByPlan(planID string) ([]models.Topic, error) {
	rows, err := s.db.Query(`
		SELECT id, study_plan_id, name, description, topic_order, difficulty, est_minutes, status, progress
		FROM topics WHERE study_plan_id = ? ORDER BY topic_order
	`, planID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var topics []models.Topic
	for rows.Next() {
		var topic models.Topic
		if err := rows.Scan(&topic.ID, &topic.StudyPlanID, &topic.Name, &topic.Description, &topic.Order, &topic.Difficulty, &topic.EstMinutes, &topic.Status, &topic.Progress); err != nil {
			return nil, err
		}
		topics = append(topics, topic)
	}
	return topics, nil
}

func (s *SQLiteStorage) UpdateTopicStatus(id string, status string, progress float64) error {
	_, err := s.db.Exec(`UPDATE topics SET status = ?, progress = ? WHERE id = ?`, status, progress, id)
	return err
}

// Fragen

func (s *SQLiteStorage) SaveQuestion(q *models.Question) error {
	hints, _ := json.Marshal(q.Hints)
	options, _ := json.Marshal(q.Options)
	_, err := s.db.Exec(`
		INSERT OR REPLACE INTO questions (id, topic_id, question, expected_answer, hints, difficulty, type, options, user_answer, is_correct, feedback, answered_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, q.ID, q.TopicID, q.Question, q.ExpectedAnswer, string(hints), q.Difficulty, q.Type, string(options), q.UserAnswer, q.IsCorrect, q.Feedback, q.AnsweredAt)
	return err
}

func (s *SQLiteStorage) GetQuestion(id string) (*models.Question, error) {
	var q models.Question
	var hints, options string
	var isCorrect sql.NullInt64
	var answeredAt sql.NullTime
	err := s.db.QueryRow(`
		SELECT id, topic_id, question, expected_answer, hints, difficulty, type, options, user_answer, is_correct, feedback, answered_at
		FROM questions WHERE id = ?
	`, id).Scan(&q.ID, &q.TopicID, &q.Question, &q.ExpectedAnswer, &hints, &q.Difficulty, &q.Type, &options, &q.UserAnswer, &isCorrect, &q.Feedback, &answeredAt)
	if err != nil {
		return nil, err
	}
	json.Unmarshal([]byte(hints), &q.Hints)
	json.Unmarshal([]byte(options), &q.Options)
	if isCorrect.Valid {
		val := isCorrect.Int64 == 1
		q.IsCorrect = &val
	}
	if answeredAt.Valid {
		q.AnsweredAt = &answeredAt.Time
	}
	return &q, nil
}

func (s *SQLiteStorage) GetQuestionsByTopic(topicID string) ([]models.Question, error) {
	rows, err := s.db.Query(`
		SELECT id, topic_id, question, expected_answer, hints, difficulty, type, options, user_answer, is_correct, feedback, answered_at
		FROM questions WHERE topic_id = ? ORDER BY difficulty
	`, topicID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var questions []models.Question
	for rows.Next() {
		var q models.Question
		var hints, options string
		var isCorrect sql.NullInt64
		var answeredAt sql.NullTime
		if err := rows.Scan(&q.ID, &q.TopicID, &q.Question, &q.ExpectedAnswer, &hints, &q.Difficulty, &q.Type, &options, &q.UserAnswer, &isCorrect, &q.Feedback, &answeredAt); err != nil {
			return nil, err
		}
		json.Unmarshal([]byte(hints), &q.Hints)
		json.Unmarshal([]byte(options), &q.Options)
		if isCorrect.Valid {
			val := isCorrect.Int64 == 1
			q.IsCorrect = &val
		}
		if answeredAt.Valid {
			q.AnsweredAt = &answeredAt.Time
		}
		questions = append(questions, q)
	}
	return questions, nil
}

func (s *SQLiteStorage) SaveQuestionAnswer(id string, answer string, isCorrect bool, feedback string) error {
	_, err := s.db.Exec(`
		UPDATE questions SET user_answer = ?, is_correct = ?, feedback = ?, answered_at = ? WHERE id = ?
	`, answer, isCorrect, feedback, time.Now(), id)
	return err
}

// Sitzungen

func (s *SQLiteStorage) SaveSession(session *models.StudySession) error {
	_, err := s.db.Exec(`
		INSERT OR REPLACE INTO study_sessions (id, study_plan_id, topic_id, started_at, ended_at, duration_minutes, questions_answered, correct_answers)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?)
	`, session.ID, session.StudyPlanID, session.TopicID, session.StartedAt, session.EndedAt, session.Duration, session.QuestionsAnswered, session.CorrectAnswers)
	return err
}

func (s *SQLiteStorage) GetSessionsByPlan(planID string) ([]models.StudySession, error) {
	rows, err := s.db.Query(`
		SELECT id, study_plan_id, topic_id, started_at, ended_at, duration_minutes, questions_answered, correct_answers
		FROM study_sessions WHERE study_plan_id = ? ORDER BY started_at DESC
	`, planID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var sessions []models.StudySession
	for rows.Next() {
		var session models.StudySession
		var endedAt sql.NullTime
		if err := rows.Scan(&session.ID, &session.StudyPlanID, &session.TopicID, &session.StartedAt, &endedAt, &session.Duration, &session.QuestionsAnswered, &session.CorrectAnswers); err != nil {
			return nil, err
		}
		if endedAt.Valid {
			session.EndedAt = &endedAt.Time
		}
		sessions = append(sessions, session)
	}
	return sessions, nil
}

// Chat

func (s *SQLiteStorage) SaveChatMessage(msg *models.ChatMessage) error {
	_, err := s.db.Exec(`
		INSERT INTO chat_messages (id, session_id, role, content, timestamp, topic_id)
		VALUES (?, ?, ?, ?, ?, ?)
	`, msg.ID, msg.SessionID, msg.Role, msg.Content, msg.Timestamp, msg.TopicID)
	return err
}

func (s *SQLiteStorage) GetChatHistory(sessionID string) ([]models.ChatMessage, error) {
	rows, err := s.db.Query(`
		SELECT id, session_id, role, content, timestamp, topic_id
		FROM chat_messages WHERE session_id = ? ORDER BY timestamp
	`, sessionID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var messages []models.ChatMessage
	for rows.Next() {
		var msg models.ChatMessage
		if err := rows.Scan(&msg.ID, &msg.SessionID, &msg.Role, &msg.Content, &msg.Timestamp, &msg.TopicID); err != nil {
			return nil, err
		}
		messages = append(messages, msg)
	}
	return messages, nil
}

// Glossar

func (s *SQLiteStorage) SaveGlossaryItem(item *models.GlossaryItem) error {
	relatedJSON, _ := json.Marshal(item.Related)
	
	_, err := s.db.Exec(`
		INSERT OR REPLACE INTO glossary (id, term, category, definition, details, related, created_at, updated_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?)
	`, item.ID, item.Term, item.Category, item.Definition, item.Details, string(relatedJSON), item.CreatedAt, item.UpdatedAt)
	return err
}

func (s *SQLiteStorage) GetGlossaryItem(id string) (*models.GlossaryItem, error) {
	var item models.GlossaryItem
	var relatedJSON string
	
	err := s.db.QueryRow(`
		SELECT id, term, category, definition, details, related, created_at, updated_at
		FROM glossary WHERE id = ?
	`, id).Scan(&item.ID, &item.Term, &item.Category, &item.Definition, &item.Details, &relatedJSON, &item.CreatedAt, &item.UpdatedAt)
	
	if err != nil {
		return nil, err
	}
	
	if relatedJSON != "" {
		json.Unmarshal([]byte(relatedJSON), &item.Related)
	}
	
	return &item, nil
}

func (s *SQLiteStorage) GetAllGlossaryItems() ([]models.GlossaryItem, error) {
	rows, err := s.db.Query(`
		SELECT id, term, category, definition, details, related, created_at, updated_at
		FROM glossary ORDER BY term
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var items []models.GlossaryItem
	for rows.Next() {
		var item models.GlossaryItem
		var relatedJSON string
		
		if err := rows.Scan(&item.ID, &item.Term, &item.Category, &item.Definition, &item.Details, &relatedJSON, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		
		if relatedJSON != "" {
			json.Unmarshal([]byte(relatedJSON), &item.Related)
		}
		
		items = append(items, item)
	}
	return items, nil
}

func (s *SQLiteStorage) DeleteGlossaryItem(id string) error {
	_, err := s.db.Exec(`DELETE FROM glossary WHERE id = ?`, id)
	return err
}
