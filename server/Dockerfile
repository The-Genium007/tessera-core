# syntax=docker/dockerfile:1
# Image runtime unique TesseraSynth : Gateway (GNS) + Shard (TCP interne) + directory (CLI).
# Build multi-stage. CONTEXTE DE BUILD = racine du dépôt.
#
#   docker build -f tessera-core/server/Dockerfile -t tessera-server .

# ---------- étape build ----------
FROM rust:1-bookworm AS build

# Dépendances natives de GNS : cmake + protobuf 3.21 (bookworm) + openssl + clang. Voir ADR 0003.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake pkg-config clang git libssl-dev \
        protobuf-compiler libprotobuf-dev \
        curl unzip ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# flatc EXACTEMENT à la version du crate flatbuffers (25.12.19).
RUN curl -fsSL -o /tmp/flatc.zip \
      https://github.com/google/flatbuffers/releases/download/v25.12.19/Linux.flatc.binary.g++-13.zip \
    && unzip -o /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip \
    && flatc --version

WORKDIR /src
COPY . .

# Gateway seul a besoin de la feature `gns` (seul binaire qui lie GNS/protobuf au runtime).
RUN cargo build --release --features gns -p server --bin gateway
# Shard et directory n'en ont pas besoin — builds séparés (invocations distinctes) pour ne pas
# leur imposer la feature activée par la ligne ci-dessus.
RUN cargo build --release -p server --bin shard
RUN cargo build --release -p directory

# ---------- étape runtime ----------
FROM debian:bookworm-slim AS runtime
# libprotobuf32 : GNS (gateway) lie protobuf 3.21 dynamiquement. libssl3 : openssl (gateway).
RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl3 libprotobuf32 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/gateway /usr/local/bin/tessera-gateway
COPY --from=build /src/target/release/shard /usr/local/bin/tessera-shard
COPY --from=build /src/target/release/tessera-directory /usr/local/bin/tessera-directory

ENV RUST_LOG=info
# Port de jeu (UDP), utilisé uniquement par le service gateway.
EXPOSE 27020/udp
# Pas d'ENTRYPOINT/CMD fixe : docker-compose choisit le binaire par service via `command:`.
