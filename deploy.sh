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

gunzip | docker load
docker compose up -d --wait --wait-timeout 180 --remove-orphans
docker image prune -f >/dev/null
docker compose ps
