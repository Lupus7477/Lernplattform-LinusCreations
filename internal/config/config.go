package config

import (
	"encoding/json"
	"os"
	"path/filepath"
)

// Config enthält alle Konfigurationseinstellungen
type Config struct {
	// Server-Einstellungen
	ServerPort string `json:"server_port"`

	// Pfade
	DocumentsPath string `json:"documents_path"`
	DatabasePath  string `json:"database_path"`

	// LLM-Einstellungen
	OllamaURL    string `json:"ollama_url"`
	DefaultModel string `json:"default_model"`

	// Lern-Einstellungen
	MinStudySessionMinutes int `json:"min_study_session_minutes"`
	MaxQuestionsPerTopic   int `json:"max_questions_per_topic"`
}

// Default gibt die Standardkonfiguration zurück
func Default() *Config {
	homeDir, _ := os.UserHomeDir()
	return &Config{
		ServerPort:             "8080",
		DocumentsPath:          filepath.Join(homeDir, "Lernmaterial"),
		DatabasePath:           "lernplattform.db",
		OllamaURL:              "http://localhost:11434",
		DefaultModel:           "qwen2.5:7b",
		MinStudySessionMinutes: 30,
		MaxQuestionsPerTopic:   10,
	}
}

// Load lädt die Konfiguration aus einer Datei
func Load(path string) (*Config, error) {
	cfg := Default()

	data, err := os.ReadFile(path)
	if err != nil {
		return cfg, err
	}

	if err := json.Unmarshal(data, cfg); err != nil {
		return cfg, err
	}

	return cfg, nil
}

// Save speichert die Konfiguration in eine Datei
func (c *Config) Save(path string) error {
	data, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0644)
}
