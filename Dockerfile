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
RUN cargo build --release -p lernpfad-server

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates libvulkan1 mesa-vulkan-drivers libgomp1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /data lernpfad \
    && mkdir -p /data && chown lernpfad:lernpfad /data
COPY --from=build /src/target/release/lernpfad-server /usr/local/bin/lernpfad-server
COPY web /app/web
USER lernpfad
VOLUME /data
EXPOSE 8443
ENTRYPOINT ["lernpfad-server", "--data", "/data", "--web", "/app/web"]
