package llm

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"time"
)

// ollamaSemaphore limitiert gleichzeitige Ollama-Anfragen (verhindert Speicherüberlauf)
var ollamaSemaphore = make(chan struct{}, 1) // Nur 1 gleichzeitige Anfrage

func acquireOllama() {
	ollamaSemaphore <- struct{}{}
}

func releaseOllama() {
	<-ollamaSemaphore
}

// Provider definiert das Interface für LLM-Backends
type Provider interface {
	// Generate erzeugt eine Antwort basierend auf dem Prompt
	Generate(ctx context.Context, prompt string, options *GenerateOptions) (*GenerateResponse, error)

	// GenerateStream erzeugt eine Streaming-Antwort
	GenerateStream(ctx context.Context, prompt string, options *GenerateOptions) (<-chan StreamChunk, error)

	// Chat führt einen Chat mit Nachrichtenverlauf
	Chat(ctx context.Context, messages []ChatMessage, options *GenerateOptions) (*GenerateResponse, error)

	// GetModels gibt verfügbare Modelle zurück
	GetModels(ctx context.Context) ([]ModelInfo, error)

	// IsAvailable prüft, ob das Backend erreichbar ist
	IsAvailable(ctx context.Context) bool

	// GetName gibt den Namen des Providers zurück
	GetName() string

	// SetModel ändert das verwendete Modell
	SetModel(model string)

	// GetCurrentModel gibt das aktuelle Modell zurück
	GetCurrentModel() string
}

// GenerateOptions enthält optionale Parameter für die Generierung
type GenerateOptions struct {
	Model       string  `json:"model,omitempty"`
	Temperature float64 `json:"temperature,omitempty"`
	MaxTokens   int     `json:"max_tokens,omitempty"`
	TopP        float64 `json:"top_p,omitempty"`
	TopK        int     `json:"top_k,omitempty"`
	System      string  `json:"system,omitempty"`
}

// GenerateResponse enthält die Antwort des LLM
type GenerateResponse struct {
	Content      string `json:"content"`
	Model        string `json:"model"`
	TotalTokens  int    `json:"total_tokens"`
	PromptTokens int    `json:"prompt_tokens"`
	Done         bool   `json:"done"`
}

// ChatMessage repräsentiert eine Chat-Nachricht
type ChatMessage struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

// ModelInfo enthält Informationen über ein Modell
type ModelInfo struct {
	Name       string    `json:"name"`
	ModifiedAt time.Time `json:"modified_at"`
	Size       int64     `json:"size"`
}

// StreamChunk repräsentiert einen Chunk im Streaming-Modus
type StreamChunk struct {
	Content string `json:"content"`
	Done    bool   `json:"done"`
	Error   error  `json:"error,omitempty"`
}

// OllamaProvider implementiert den Provider für Ollama
type OllamaProvider struct {
	baseURL      string
	defaultModel string
	client       *http.Client
}

// SetModel ändert das Standard-Modell
func (o *OllamaProvider) SetModel(model string) {
	if model != "" {
		o.defaultModel = model
	}
}

// GetCurrentModel gibt das aktuelle Modell zurück
func (o *OllamaProvider) GetCurrentModel() string {
	return o.defaultModel
}

// NewOllamaProvider erstellt einen neuen Ollama-Provider
func NewOllamaProvider(baseURL, defaultModel string) *OllamaProvider {
	if baseURL == "" {
		baseURL = "http://localhost:11434"
	}
	if defaultModel == "" {
		defaultModel = "qwen2.5:7b"
	}

	provider := &OllamaProvider{
		baseURL:      strings.TrimSuffix(baseURL, "/"),
		defaultModel: defaultModel,
		client: &http.Client{
			Timeout: 15 * time.Minute, // Erhöht für große Prompts
		},
	}

	// Prüfe ob das Modell existiert, sonst erstes verfügbares nehmen
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	
	models, err := provider.GetModels(ctx)
	if err == nil && len(models) > 0 {
		found := false
		for _, m := range models {
			if m.Name == defaultModel {
				found = true
				break
			}
		}
		if !found {
			log.Printf("⚠️  Modell '%s' nicht gefunden, verwende '%s'", defaultModel, models[0].Name)
			provider.defaultModel = models[0].Name
		}
	}

	return provider
}

func (o *OllamaProvider) GetName() string {
	return "Ollama"
}

func (o *OllamaProvider) IsAvailable(ctx context.Context) bool {
	req, err := http.NewRequestWithContext(ctx, "GET", o.baseURL+"/api/tags", nil)
	if err != nil {
		return false
	}

	resp, err := o.client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()

	return resp.StatusCode == http.StatusOK
}

func (o *OllamaProvider) GetModels(ctx context.Context) ([]ModelInfo, error) {
	req, err := http.NewRequestWithContext(ctx, "GET", o.baseURL+"/api/tags", nil)
	if err != nil {
		return nil, err
	}

	resp, err := o.client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("ollama nicht erreichbar: %w", err)
	}
	defer resp.Body.Close()

	var result struct {
		Models []struct {
			Name       string    `json:"name"`
			ModifiedAt time.Time `json:"modified_at"`
			Size       int64     `json:"size"`
		} `json:"models"`
	}

	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
		return nil, err
	}

	var models []ModelInfo
	for _, m := range result.Models {
		models = append(models, ModelInfo{
			Name:       m.Name,
			ModifiedAt: m.ModifiedAt,
			Size:       m.Size,
		})
	}

	return models, nil
}

func (o *OllamaProvider) Generate(ctx context.Context, prompt string, options *GenerateOptions) (*GenerateResponse, error) {
	// Semaphore: Nur eine Anfrage gleichzeitig an Ollama
	acquireOllama()
	defer releaseOllama()
	
	return o.generateWithRetry(ctx, prompt, options, 3) // Max 3 Versuche
}

func (o *OllamaProvider) generateWithRetry(ctx context.Context, prompt string, options *GenerateOptions, maxRetries int) (*GenerateResponse, error) {
	model := o.defaultModel
	if options != nil && options.Model != "" {
		model = options.Model
	}

	var lastErr error
	for attempt := 1; attempt <= maxRetries; attempt++ {
		if attempt > 1 {
			log.Printf("   [Ollama] 🔄 Retry %d/%d...", attempt, maxRetries)
			time.Sleep(time.Duration(attempt) * 2 * time.Second) // Exponential backoff
		}
		
		resp, err := o.doGenerate(ctx, prompt, model, options)
		if err == nil {
			return resp, nil
		}
		
		lastErr = err
		
		// Bei "runner terminated" warte und versuche erneut
		if strings.Contains(err.Error(), "terminated") || strings.Contains(err.Error(), "500") {
			log.Printf("   [Ollama] ⚠️ Ollama-Prozess abgestürzt, warte 5s...")
			time.Sleep(5 * time.Second)
			continue
		}
		
		// Bei Context-Abbruch sofort aufhören
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
	}
	
	return nil, lastErr
}

func (o *OllamaProvider) doGenerate(ctx context.Context, prompt string, model string, options *GenerateOptions) (*GenerateResponse, error) {
	log.Printf("   [Ollama] Sende Anfrage an %s/api/generate", o.baseURL)
	log.Printf("   [Ollama] Modell: %s", model)
	log.Printf("   [Ollama] Prompt-Länge: %d Zeichen", len(prompt))

	reqBody := map[string]interface{}{
		"model":  model,
		"prompt": prompt,
		"stream": false,
	}

	if options != nil {
		if options.Temperature > 0 {
			reqBody["options"] = map[string]interface{}{
				"temperature": options.Temperature,
			}
		}
		if options.System != "" {
			reqBody["system"] = options.System
		}
	}

	jsonData, err := json.Marshal(reqBody)
	if err != nil {
		log.Printf("   [Ollama] ❌ JSON-Marshal Fehler: %v", err)
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, "POST", o.baseURL+"/api/generate", bytes.NewReader(jsonData))
	if err != nil {
		log.Printf("   [Ollama] ❌ Request-Erstellung Fehler: %v", err)
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json")

	log.Println("   [Ollama] Warte auf Antwort... (kann dauern bei großen Prompts)")
	start := time.Now()

	resp, err := o.client.Do(req)
	if err != nil {
		log.Printf("   [Ollama] ❌ Netzwerk-Fehler nach %v: %v", time.Since(start), err)
		return nil, fmt.Errorf("ollama-anfrage fehlgeschlagen: %w", err)
	}
	defer resp.Body.Close()

	log.Printf("   [Ollama] Antwort erhalten nach %v (Status: %d)", time.Since(start), resp.StatusCode)

	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(resp.Body)
		log.Printf("   [Ollama] ❌ Fehler-Antwort: %s", string(body))
		return nil, fmt.Errorf("ollama-fehler (%d): %s", resp.StatusCode, string(body))
	}

	var result struct {
		Response string `json:"response"`
		Model    string `json:"model"`
		Done     bool   `json:"done"`
	}

	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
		log.Printf("   [Ollama] ❌ JSON-Decode Fehler: %v", err)
		return nil, err
	}

	log.Printf("   [Ollama] ✓ Erfolgreich! Antwort: %d Zeichen", len(result.Response))

	return &GenerateResponse{
		Content: result.Response,
		Model:   result.Model,
		Done:    result.Done,
	}, nil
}

func (o *OllamaProvider) GenerateStream(ctx context.Context, prompt string, options *GenerateOptions) (<-chan StreamChunk, error) {
	model := o.defaultModel
	if options != nil && options.Model != "" {
		model = options.Model
	}

	reqBody := map[string]interface{}{
		"model":  model,
		"prompt": prompt,
		"stream": true,
	}

	if options != nil && options.System != "" {
		reqBody["system"] = options.System
	}

	jsonData, err := json.Marshal(reqBody)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, "POST", o.baseURL+"/api/generate", bytes.NewReader(jsonData))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json")

	resp, err := o.client.Do(req)
	if err != nil {
		return nil, err
	}

	ch := make(chan StreamChunk, 100)

	go func() {
		defer close(ch)
		defer resp.Body.Close()

		decoder := json.NewDecoder(resp.Body)
		for {
			var chunk struct {
				Response string `json:"response"`
				Done     bool   `json:"done"`
			}

			if err := decoder.Decode(&chunk); err != nil {
				if err != io.EOF {
					ch <- StreamChunk{Error: err}
				}
				return
			}

			ch <- StreamChunk{
				Content: chunk.Response,
				Done:    chunk.Done,
			}

			if chunk.Done {
				return
			}
		}
	}()

	return ch, nil
}

func (o *OllamaProvider) Chat(ctx context.Context, messages []ChatMessage, options *GenerateOptions) (*GenerateResponse, error) {
	model := o.defaultModel
	if options != nil && options.Model != "" {
		model = options.Model
	}

	reqBody := map[string]interface{}{
		"model":    model,
		"messages": messages,
		"stream":   false,
	}

	jsonData, err := json.Marshal(reqBody)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, "POST", o.baseURL+"/api/chat", bytes.NewReader(jsonData))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json")

	resp, err := o.client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("ollama-chat fehlgeschlagen: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("ollama-fehler (%d): %s", resp.StatusCode, string(body))
	}

	var result struct {
		Message struct {
			Content string `json:"content"`
		} `json:"message"`
		Model string `json:"model"`
		Done  bool   `json:"done"`
	}

	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
		return nil, err
	}

	return &GenerateResponse{
		Content: result.Message.Content,
		Model:   result.Model,
		Done:    result.Done,
	}, nil
}
