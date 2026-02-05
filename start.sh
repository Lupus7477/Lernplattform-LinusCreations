#!/bin/bash

echo "========================================"
echo "   Lokale KI-Lernplattform"
echo "========================================"
echo ""

# Prüfe ob Go installiert ist
if ! command -v go &> /dev/null; then
    echo "[FEHLER] Go ist nicht installiert!"
    echo "Bitte installiere Go von: https://go.dev/dl/"
    exit 1
fi

# Prüfe ob Ollama läuft
if ! curl -s http://localhost:11434/api/tags &> /dev/null; then
    echo "[WARNUNG] Ollama scheint nicht zu laufen!"
    echo "Bitte starte Ollama mit: ollama serve"
    echo ""
fi

echo "[INFO] Installiere Abhängigkeiten..."
go mod tidy

echo ""
echo "[INFO] Starte Server..."
echo ""
echo "Öffne im Browser: http://localhost:8080"
echo ""
echo "Drücke Strg+C zum Beenden"
echo "========================================"
echo ""

go run ./cmd/server
