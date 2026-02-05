@echo off
echo ========================================
echo    Lokale KI-Lernplattform
echo ========================================
echo.

:: Pruefe ob Go installiert ist
where go >nul 2>nul
if %errorlevel% neq 0 (
    echo [FEHLER] Go ist nicht installiert!
    echo Bitte installiere Go von: https://go.dev/dl/
    pause
    exit /b 1
)

:: Pruefe ob Ollama laeuft
curl -s http://localhost:11434/api/tags >nul 2>nul
if %errorlevel% neq 0 (
    echo [WARNUNG] Ollama scheint nicht zu laufen!
    echo Bitte starte Ollama mit: ollama serve
    echo.
)

echo [INFO] Installiere Abhaengigkeiten...
go mod tidy

echo.
echo [INFO] Starte Server...
echo.
echo Oeffne im Browser: http://localhost:8080
echo.
echo Druecke Strg+C zum Beenden
echo ========================================
echo.

go run ./cmd/server
