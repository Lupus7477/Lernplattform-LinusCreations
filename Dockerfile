# Lernpfad-Server im Container.
#   docker compose up -d --build
#   docker compose logs     → Einrichtungslink und Zertifikats-Fingerabdruck
# Daten (Datenbank, KI-Modell ~2,7 GB, Zertifikat) liegen im Volume /data.

FROM rust:1-trixie AS chef
RUN apt-get update && apt-get install -y --no-install-recommends \
      cmake clang libclang-dev libvulkan-dev glslc spirv-headers \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install cargo-chef --locked
WORKDIR /src

# Nur die Abhängigkeiten (u. a. llama.cpp mit Vulkan) als eigene Schicht bauen:
# Solange Cargo.toml/Cargo.lock gleich bleiben, kommt sie aus dem Cache.
FROM chef AS plan
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=plan /src/recipe.json .
RUN cargo chef cook --release -p lernpfad-server --recipe-path recipe.json
COPY . .
# Erst die Tests: Schlägt einer fehl, bricht der Build ab, und auf dem Server läuft die alte Version weiter
RUN cargo test --release -p lernpfad-core -p lernpfad-server
RUN cargo build --release -p lernpfad-server

FROM debian:trixie-slim
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates libvulkan1 mesa-vulkan-drivers libgomp1 tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /data lernpfad \
    && mkdir -p /data && chown lernpfad:lernpfad /data
# Tagesgrenzen (Streak, Tagesziel, Kalender) in deutscher Zeit statt UTC; ändern über TZ in compose.yaml
ENV TZ=Europe/Berlin
COPY --from=build /src/target/release/lernpfad-server /usr/local/bin/lernpfad-server
COPY web /app/web
USER lernpfad
VOLUME /data
EXPOSE 8443
# Der Port öffnet erst, wenn Datenbank und Migrationen bereit sind. `compose up --wait` (deploy.sh) wartet darauf.
# Anderer Port per --bind? Dann in compose.override.yaml `healthcheck:` anpassen.
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 CMD ["bash", "-c", ": > /dev/tcp/127.0.0.1/8443"]
ENTRYPOINT ["lernpfad-server", "--data", "/data", "--web", "/app/web"]
