#!/bin/sh
# Läuft auf dem Server in /home/Lernpfad, ausgelöst von .github/workflows/deploy.yml.
# Bekommt das fertige Image (gzip) per stdin, holt compose.yaml von main und startet neu.
# Gebaut wird auf GitHub, damit der Server keinen RAM fürs Kompilieren braucht.
# Serverspezifisches (IP, RAM-Grenze) steht in compose.override.yaml, nicht im Repo.
set -eu
cd "$(dirname "$0")"

# ponytail: holt den neuesten main-Stand, nicht exakt den Commit des Images.
# Der Unterschied betrifft nur compose.yaml und dieses Skript.
git fetch -q origin main
git reset -q --hard origin/main

# Die laufende Version als lernpfad:prev behalten (sonst löscht sie `image prune`).
# Startet die neue nicht gesund, geht es automatisch zurück; von Hand: docker tag lernpfad:prev lernpfad && docker compose up -d
docker tag lernpfad lernpfad:prev 2>/dev/null || true
gunzip | docker load
if ! docker compose up -d --wait --wait-timeout 180 --remove-orphans; then
  echo "Die neue Version startet nicht gesund, zurück zur vorherigen." >&2
  docker compose logs --tail 50 >&2 || true
  docker tag lernpfad:prev lernpfad && docker compose up -d --wait --wait-timeout 180
  exit 1
fi
docker image prune -f >/dev/null
docker compose ps
