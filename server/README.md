# server/ — Serveur de jeu TesseraSynth (Rust + GameNetworkingSockets)

Le **serveur autoritaire** : il héberge les joueurs, suit leurs positions et les diffuse. Écrit
en **Rust**, réseau **UDP fiable** via GameNetworkingSockets (GNS). C'est l'« arbitre/hôte » du
monde partagé ; les jeux moddés (clients) s'y connectent.

> Validé : l'image Docker **se construit et démarre** (écoute `0.0.0.0:27020`, tick 20 Hz).

## ⚠️ Différence importante avec le site web

Le site web passe par le **tunnel Cloudflare** (HTTP). Le jeu, lui, parle en **UDP** → on **publie
directement un port UDP public** (`27020/udp`). Ce port doit être **ouvert sur le pare-feu** de la
machine qui héberge, et joignable depuis internet (le serveur a besoin d'une **IP publique**).

## Déploiement sur Dokploy — recommandé : image pré-construite (GHCR)

Le cloud GitHub construit l'image automatiquement et la publie sur **GHCR** (le registre d'images
de GitHub) à chaque mise à jour. Tu n'as donc **rien à compiler** : Dokploy **tire** l'image finie.

1. Sur Dokploy, crée une app de type **Compose**, source = ce dépôt, fichier
   `server/docker-compose.yml`. Ce compose décrit la topologie réelle : un **Gateway** et deux
   **Shards** (`shard-a`, `shard-b`), tous les trois construits depuis la même image mais lancés
   avec des `command:` différents (voir `## Détails techniques`).
2. Le compose construit l'image depuis le dépôt (`build:`) par défaut. Pour utiliser l'image
   **déjà construite** par le cloud sur GHCR à la place (plus rapide, pas de compilation GNS à
   chaque déploiement), positionne les variables d'environnement du compose, par exemple :

   ```yaml
   environment:
     IMAGE_NAME: ghcr.io/the-genium007/tessera-server
     IMAGE_TAG: latest
   ```

   ou remplace directement `image: ${IMAGE_NAME:-tessera-server}:${IMAGE_TAG:-latest}` par
   `image: ghcr.io/the-genium007/tessera-server:latest` dans chaque service.
3. Monte un manifeste (voir `server.docker.toml`) via `MANIFEST_PATH`, ou adapte
   `server.docker.toml` directement pour ton déploiement (adresses, spawn points, rayons AoI...).
4. **Ouvre le port `27020/udp`** (Gateway) sur le pare-feu de ton serveur. Le port `9100` (métriques
   Prometheus) est aussi publié par défaut — à restreindre/retirer en prod publique.
5. Déploie.

Les joueurs (via le launcher) se connecteront à **`<IP publique du serveur>:27020`**.

## Construire / lancer en local (développeurs)

Avec Docker Compose (recommandé — reproduit la topologie Gateway + 2 Shards) :

```bash
cd server
docker compose up --build
docker compose logs -f gateway   # doit afficher « Gateway handoff : écoute GNS sur ... »
```

Ça utilise `server/docker-compose.yml` et le manifeste `server/server.docker.toml` (monté en
volume, pas embarqué dans l'image).

Sans Docker (build natif, nécessite les prérequis GNS de l'ADR 0003) :

```bash
# Shards d'abord (pas de feature gns requise pour compiler, mais nécessaire à l'exécution réseau) :
cargo run -p server --bin shard -- 127.0.0.1:27030 --manifest server/server.example.toml
cargo run -p server --bin shard -- 127.0.0.1:27031 --manifest server/server.example.toml

# Puis le Gateway (feature gns obligatoire) :
cargo run -p server --features gns --bin gateway -- --manifest server/server.example.toml
```

Le manifeste (`server.example.toml`) porte la topologie, les adresses d'écoute des shards, les
spawn points et le chemin de sauvegarde des joueurs — voir `server.example.toml` pour un exemple
complet, et les doc-comments de `src/bin/shard.rs` / `src/bin/gateway.rs` pour l'usage CLI exact.

## Détails techniques

- Image : multi-stage (build `rust:bookworm` → runtime `debian:bookworm-slim`, ~108 Mo), contient
  trois binaires (`tessera-gateway`, `tessera-shard`, `tessera-directory`) — pas d'`ENTRYPOINT`/`CMD`
  fixe, chaque service du compose fournit son propre `command:`.
- Topologie réelle : un **Gateway** (parle GNS aux joueurs, feature `gns` requise) qui répartit vers
  des **Shards** selon la config `[runtime.topology]` du manifeste ; chaque Shard n'a pas besoin de
  la feature `gns` pour compiler mais reçoit ses connexions internes du Gateway.
- Protocole : FlatBuffers (voir `protocol/`). Contrat client : `client-mod/INTEGRATION-server-contract.md`.
- Build GNS : cmake + protobuf 3.21 + openssl + flatc 25.12.19 (voir `Dockerfile` et ADR 0003).
- Sans la feature `gns`, `cargo build -p server` tourne « à vide » (pas de réseau) — utile pour les tests.
