package pdf

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/ledongthuc/pdf"
	"lernplattform/internal/models"
)

// Parser extrahiert Text aus PDF-Dokumenten
type Parser struct {
	documentsPath string
}

// NewParser erstellt einen neuen PDF-Parser
func NewParser(documentsPath string) *Parser {
	return &Parser{documentsPath: documentsPath}
}

// ParseFile parst eine einzelne PDF-Datei
func (p *Parser) ParseFile(filePath string) (*models.Document, error) {
	f, r, err := pdf.Open(filePath)
	if err != nil {
		return nil, fmt.Errorf("fehler beim Öffnen der PDF: %w", err)
	}
	defer f.Close()

	var content strings.Builder
	totalPages := r.NumPage()

	for pageNum := 1; pageNum <= totalPages; pageNum++ {
		page := r.Page(pageNum)
		if page.V.IsNull() {
			continue
		}

		text, err := page.GetPlainText(nil)
		if err != nil {
			continue
		}

		content.WriteString(fmt.Sprintf("\n--- Seite %d ---\n", pageNum))
		content.WriteString(text)
	}

	doc := &models.Document{
		ID:          generateID(),
		Name:        filepath.Base(filePath),
		Path:        filePath,
		Content:     content.String(),
		PageCount:   totalPages,
		UploadedAt:  time.Now(),
		ProcessedAt: time.Now(),
	}

	return doc, nil
}

// ParseDirectory parst alle PDF-Dateien in einem Verzeichnis
func (p *Parser) ParseDirectory(dirPath string) ([]models.Document, error) {
	var documents []models.Document

	err := filepath.Walk(dirPath, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}

		if info.IsDir() || !strings.HasSuffix(strings.ToLower(info.Name()), ".pdf") {
			return nil
		}

		doc, err := p.ParseFile(path)
		if err != nil {
			// Fehler loggen, aber fortfahren
			fmt.Printf("Warnung: Konnte %s nicht parsen: %v\n", path, err)
			return nil
		}

		documents = append(documents, *doc)
		return nil
	})

	if err != nil {
		return nil, err
	}

	return documents, nil
}

// ParseFromReader parst PDF aus einem io.Reader (für Uploads)
func (p *Parser) ParseFromReader(reader io.Reader, filename string) (*models.Document, error) {
	// In temporäre Datei schreiben
	data, err := io.ReadAll(reader)
	if err != nil {
		return nil, err
	}

	// PDF parsen
	r, err := pdf.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return nil, fmt.Errorf("fehler beim Lesen der PDF: %w", err)
	}

	var content strings.Builder
	totalPages := r.NumPage()

	for pageNum := 1; pageNum <= totalPages; pageNum++ {
		page := r.Page(pageNum)
		if page.V.IsNull() {
			continue
		}

		text, err := page.GetPlainText(nil)
		if err != nil {
			continue
		}

		content.WriteString(fmt.Sprintf("\n--- Seite %d ---\n", pageNum))
		content.WriteString(text)
	}

	doc := &models.Document{
		ID:          generateID(),
		Name:        filename,
		Content:     content.String(),
		PageCount:   totalPages,
		UploadedAt:  time.Now(),
		ProcessedAt: time.Now(),
	}

	return doc, nil
}

// ExtractChunks teilt den Text in Chunks für die LLM-Verarbeitung
func ExtractChunks(content string, chunkSize int, overlap int) []string {
	if chunkSize <= 0 {
		chunkSize = 2000
	}
	if overlap < 0 {
		overlap = 200
	}

	var chunks []string
	runes := []rune(content)
	length := len(runes)

	for i := 0; i < length; i += chunkSize - overlap {
		end := i + chunkSize
		if end > length {
			end = length
		}

		chunk := string(runes[i:end])
		chunks = append(chunks, chunk)

		if end >= length {
			break
		}
	}

	return chunks
}

// ExtractSections versucht, Abschnitte/Kapitel zu identifizieren
func ExtractSections(content string) []Section {
	lines := strings.Split(content, "\n")
	var sections []Section
	var currentSection *Section

	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}

		// Heuristik für Überschriften
		isHeading := false
		if strings.HasPrefix(trimmed, "Kapitel") ||
			strings.HasPrefix(trimmed, "Abschnitt") ||
			strings.HasPrefix(trimmed, "Teil") ||
			isNumberedHeading(trimmed) ||
			(len(trimmed) < 80 && strings.ToUpper(trimmed) == trimmed && len(trimmed) > 3) {
			isHeading = true
		}

		if isHeading {
			if currentSection != nil {
				sections = append(sections, *currentSection)
			}
			currentSection = &Section{
				Title:   trimmed,
				Content: "",
			}
		} else if currentSection != nil {
			currentSection.Content += trimmed + "\n"
		}
	}

	if currentSection != nil {
		sections = append(sections, *currentSection)
	}

	return sections
}

// Section repräsentiert einen erkannten Abschnitt
type Section struct {
	Title   string
	Content string
}

func isNumberedHeading(line string) bool {
	// Prüft auf Muster wie "1.", "1.1", "1.1.1", etc.
	if len(line) < 2 {
		return false
	}

	dotCount := 0
	for i, r := range line {
		if r >= '0' && r <= '9' {
			continue
		}
		if r == '.' {
			dotCount++
			continue
		}
		if r == ' ' && dotCount > 0 && i < len(line)/2 {
			return true
		}
		break
	}
	return false
}

func generateID() string {
	return fmt.Sprintf("%d", time.Now().UnixNano())
}
