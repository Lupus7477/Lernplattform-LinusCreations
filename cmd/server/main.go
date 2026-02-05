package main

import (
	"context"
	"flag"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"lernplattform/internal/api"
	"lernplattform/internal/config"
	"lernplattform/internal/llm"
	"lernplattform/internal/storage"
)

func main() {
	log.SetFlags(log.Ltime | log.Lmsgprefix)
	log.SetPrefix("")

	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
	log.Println("🎓 LOKALE LERNPLATTFORM - Start")
	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

	// Kommandozeilen-Flags
	configPath := flag.String("config", "config.json", "Pfad zur Konfigurationsdatei")
	port := flag.String("port", "8080", "Server-Port")
	flag.Parse()

	// Konfiguration laden
	log.Println("📋 Lade Konfiguration...")
	cfg, err := config.Load(*configPath)
	if err != nil {
		log.Printf("⚠️  Konnte Konfiguration nicht laden, verwende Standardwerte: %v", err)
		cfg = config.Default()
	}
	log.Printf("   ✓ Konfiguration geladen")

	// Storage initialisieren
	log.Println("💾 Initialisiere Datenbank...")
	store, err := storage.NewSQLiteStorage(cfg.DatabasePath)
	if err != nil {
		log.Fatalf("❌ Fehler beim Initialisieren der Datenbank: %v", err)
	}
	defer store.Close()
	log.Printf("   ✓ Datenbank: %s", cfg.DatabasePath)

	// LLM-Provider initialisieren
	log.Println("🤖 Initialisiere LLM-Provider...")
	llmProvider := llm.NewOllamaProvider(cfg.OllamaURL, cfg.DefaultModel)
	
	// Prüfe LLM-Verbindung
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	if llmProvider.IsAvailable(ctx) {
		log.Printf("   ✓ Ollama erreichbar: %s", cfg.OllamaURL)
		models, err := llmProvider.GetModels(ctx)
		if err == nil {
			log.Printf("   ✓ Verfügbare Modelle: %d", len(models))
			for _, m := range models {
				log.Printf("      - %s", m.Name)
			}
		}
	} else {
		log.Printf("   ⚠️  Ollama NICHT erreichbar unter %s", cfg.OllamaURL)
		log.Println("      Starte Ollama mit: ollama serve")
	}
	cancel()
	log.Printf("   ✓ Standard-Modell: %s", cfg.DefaultModel)

	// API-Handler erstellen
	handler := api.NewHandler(store, llmProvider, cfg)

	// Router erstellen
	router := api.NewRouter(handler)

	// Server starten
	server := &http.Server{
		Addr:    ":" + *port,
		Handler: router,
	}

	// Graceful Shutdown
	go func() {
		sigChan := make(chan os.Signal, 1)
		signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)
		<-sigChan
		log.Println("")
		log.Println("⏹️  Server wird heruntergefahren...")
		server.Close()
	}()

	log.Println("")
	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
	log.Printf("✅ Server läuft auf: http://localhost:%s", *port)
	log.Println("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
	log.Println("📚 Dokumente-Ordner:", cfg.DocumentsPath)
	log.Println("💡 Drücke Strg+C zum Beenden")
	log.Println("")

	if err := server.ListenAndServe(); err != http.ErrServerClosed {
		log.Fatalf("Server-Fehler: %v", err)
	}
}
