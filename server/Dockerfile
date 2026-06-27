# syntax=docker/dockerfile:1
# Serveur autoritaire TesseraSynth — Rust + GameNetworkingSockets (UDP fiable).
# Build multi-stage. CONTEXTE DE BUILD = racine du dépôt (le build a besoin du workspace :
# Cargo.toml/Cargo.lock + protocol/ + server/ + tools/ pour résoudre le workspace).
#
#   docker build -f server/Dockerfile -t tessera-server .

# ---------- étape build ----------
FROM rust:1-bookworm AS build

# Dépendances natives de GNS : cmake + protobuf 3.21 (bookworm) + openssl + clang.
# (protobuf >= 4 casse le build GNS — bookworm fournit 3.21.x, parfait. Voir ADR 0003.)
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake pkg-config clang git libssl-dev \
        protobuf-compiler libprotobuf-dev \
        curl unzip ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# flatc EXACTEMENT à la version du crate flatbuffers (25.12.19), sinon code généré incompatible.
RUN curl -fsSL -o /tmp/flatc.zip \
      https://github.com/google/flatbuffers/releases/download/v25.12.19/Linux.flatc.binary.g++-13.zip \
    && unzip -o /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip \
    && flatc --version

WORKDIR /src
COPY . .
RUN cargo build --release -p server --features gns

# ---------- étape runtime ----------
FROM debian:bookworm-slim AS runtime
# libprotobuf32 : GNS lie protobuf 3.21 dynamiquement (libprotobuf.so.32). libssl3 : openssl.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl3 libprotobuf32 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/server /usr/local/bin/tessera-server

ENV RUST_LOG=info
# Port de jeu (UDP) — les clients moddés s'y connectent.
EXPOSE 27020/udp
# Écoute sur toutes les interfaces pour accepter les clients externes.
ENTRYPOINT ["tessera-server"]
CMD ["0.0.0.0:27020"]
