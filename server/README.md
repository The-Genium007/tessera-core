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

1. Sur Dokploy, crée une app de type **Compose** (ou Docker), source = ce dépôt, fichier
   `server/docker-compose.yml`.
2. Dans le compose, utilise l'**image GHCR** au lieu de `build:` (voir variante commentée du fichier) :
   ```yaml
   image: ghcr.io/the-genium007/tessera-server:latest
   ```
3. **Ouvre le port `27020/udp`** sur le pare-feu de ton serveur.
4. Déploie.

Les joueurs (via le launcher) se connecteront à **`<IP publique du serveur>:27020`**.

> **Alternative** : laisser Dokploy **construire** l'image lui-même depuis le dépôt (section `build:`
> du compose). Ça marche aussi, mais le build (compilation de GNS en C++) prend plusieurs minutes
> à chaque déploiement. L'image GHCR est plus rapide.

## Construire / lancer en local (développeurs)

```bash
docker build -f server/Dockerfile -t tessera-server .
docker run -d -p 27020:27020/udp --name tess tessera-server
docker logs tess     # doit afficher « GnsTransport — écoute activée addr=0.0.0.0:27020 »
```

Sans Docker (build natif, nécessite les prérequis GNS de l'ADR 0003) :
```bash
cargo run -p server --features gns -- 0.0.0.0:27020
```

## Détails techniques

- Image : multi-stage (build `rust:bookworm` → runtime `debian:bookworm-slim`, ~108 Mo).
- Le serveur écoute sur l'adresse passée en argument (défaut conteneur : `0.0.0.0:27020`).
- Protocole : FlatBuffers (voir `protocol/`). Contrat client : `client-mod/INTEGRATION-server-contract.md`.
- Build GNS : cmake + protobuf 3.21 + openssl + flatc 25.12.19 (voir `Dockerfile` et ADR 0003).
- Sans la feature `gns`, `cargo build -p server` tourne « à vide » (pas de réseau) — utile pour les tests.
