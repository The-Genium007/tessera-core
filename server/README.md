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
   `server/docker-compose.yml`. Ce compose décrit la topologie réelle : un **Gateway** et
   **10 Shards** (`shard-a` … `shard-j`, une cellule Voronoï par shard depuis le 2026-07-14),
   tous construits depuis la même image mais lancés avec des `command:` différents (voir
   `## Détails techniques`).

   **Quelle branche suivre ?** `release.yml` pousse le bundle serveur publié sur une branche
   portant le nom du canal — `dev`, `playtest`, `main`. Pointer Dokploy sur `playtest` +
   `server/docker-compose.yml` + auto-deploy suffit : chaque promotion vers ce canal redéploie
   toute seule, avec une image **épinglée par version** (jamais `:latest`, pour ne pas rejouer
   l'incident platform-api du 2026-07-05 : un redeploy qui réutilise l'image en cache).

   **Vérifier ce qui tourne vraiment** — au boot, le Gateway logge sa bannière de version :

   ```text
   INFO gateway: Gateway TesseraSynth — serveur v0.1.1 · modset client requis v0.1.1
     server_version="0.1.1" required_modset=0.1.1 channel=Playtest server_name=...
   ```

   Elle sort **avant** tout ce qui peut échouer (topologie, Postgres, bind UDP) : si le boot
   casse, elle est souvent la seule preuve de *quel* binaire tourne. `required_modset` est la
   version du modset **client** que ce serveur exige ; le launcher la lit via l'annuaire et
   résout le modset à installer. Les deux sortent en lockstep du même run de release, donc un
   écart entre `server_version` et le tag de l'image = déploiement incohérent (à ceci près
   qu'un *hollow re-tag* — côté serveur inchangé — réutilise l'image d'origine sans rebuild :
   `server_version` peut alors afficher légitimement moins que le tag, le binaire étant
   bit-pour-bit celui de cette version-là).

   `server_version` vient du build (`--build-arg TESSERA_RELEASE_VERSION`, gravé via
   `option_env!`), **pas** de `CARGO_PKG_VERSION` — le crate est figé à `0.0.1` et ne suit pas
   les releases. Hors CI, la bannière affiche `dev-build (non publié)`.
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

## Persistance : Postgres + Redis

Le Gateway a besoin de deux stores :

- **Redis** (`hot_state_cache.rs`) — état chaud (position/reprise rapide), jamais la source de
  vérité durable. Toujours consulté, indépendamment de `identity.public`.
- **Postgres** (`postgres_store.rs`) — données joueur durables (position, résidence), consulté
  **uniquement** quand le manifeste déclare `identity.public = true` (serveur avec authentification
  ZITADEL réelle). Un serveur privé (`identity.public = false`, défaut) utilise `FileStore`
  (fichier JSON local) à la place et n'a besoin ni de Postgres ni de ces variables.

### Déploiement standard (colocalisé, un seul VPS)

`docker-compose.yml` inclut deux services supplémentaires, **`postgres`** et **`redis`**,
colocalisés sur la même machine que le Gateway — un choix d'échelle, pas une contrainte
d'architecture : à la taille visée par les playtests (une dizaine de joueurs simultanés), une
instance managée dédiée par service serait un coût et une étape de configuration superflus. Aucun
des deux n'est exposé hors du réseau Docker interne (pas de `ports:` publié) — seul le Gateway
leur parle, par nom de service DNS Compose (`postgres`, `redis`).

**Avec ce compose, tu n'as rien à configurer manuellement** : le Gateway assemble lui-même l'URL
Postgres depuis des variables composants (`TESSERA_PG_HOST=postgres`, `TESSERA_PG_PORT=5432`,
`TESSERA_PG_USER=tessera`, `TESSERA_PG_DATABASE=tessera`, déjà positionnées dans le compose) si
aucune URL complète (`TESSERA_POSTGRES_URL`) n'est fournie — voir
`manifest::assemble_postgres_url_from_components`. Le mot de passe (`TESSERA_POSTGRES_PASSWORD`,
partagé entre le service `postgres` et le Gateway via la même variable d'environnement) a une
valeur par défaut versionnée dans le compose ; acceptable seulement parce que Postgres n'est
**jamais** joignable depuis l'extérieur du réseau Docker — à changer explicitement si ce n'est
plus vrai pour ton déploiement (ex. tu ajoutes un `ports:` sur le service `postgres`).

Les **migrations** (`migrations/0001_create_player_records.sql`) tournent automatiquement au
démarrage du Gateway (`sqlx::migrate!(...).run(&pool)` dans `bin/gateway.rs`), avant de servir des
joueurs. Idempotent : sans effet sur un Postgres déjà à jour, donc pas d'étape manuelle même au
tout premier déploiement (volume Postgres vide).

### Variables d'environnement (à poser sur Dokploy si tu veux surcharger les défauts)

| Variable | Rôle | Obligatoire ? |
|---|---|---|
| `TESSERA_REDIS_URL` | URL Redis complète — l'emporte sur tout | Non (défaut : service `redis` colocalisé) |
| `TESSERA_POSTGRES_URL` | URL Postgres complète — l'emporte sur les composants `TESSERA_PG_*` et sur le manifeste | Non (défaut : assemblée depuis `TESSERA_PG_*`) |
| `TESSERA_POSTGRES_PASSWORD` | Mot de passe Postgres, partagé entre le service `postgres` et le Gateway | Non (défaut versionné, voir avertissement ci-dessus) |
| `TESSERA_PG_HOST`/`_PORT`/`_USER`/`_DATABASE` | Composants d'URL Postgres, utilisés si `TESSERA_POSTGRES_URL` est absente | Non (défauts déjà dans `docker-compose.yml`) |

**Ordre de priorité** (du plus fort au plus faible) : `TESSERA_POSTGRES_URL` (env) >
`runtime.postgres_url` (manifeste) > composants `TESSERA_PG_*` (env). Même principe pour Redis
sans les composants (`TESSERA_REDIS_URL` > `runtime.redis_url` du manifeste).

### Bascule vers un Postgres/Redis managé externe

Si le trafic dépasse un jour ce qu'une seule machine encaisse : poser `TESSERA_POSTGRES_URL`/
`TESSERA_REDIS_URL` avec l'URL du service managé suffit — rien d'autre à changer côté code. Les
services `postgres`/`redis` du compose peuvent alors être retirés (ou laissés inutilisés).

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

## Journal de session (playtest)

Le Gateway écrit un journal JSONL des événements de session (connexions, handoffs, zones
tampons, stalls) — spec `docs/superpowers/specs/2026-07-05-playtest-shards-design.md` §#4.

- `TESSERA_SESSION_LOG_PATH` (défaut `session.jsonl`) — fichier JSONL, une ligne par événement
  (`{"ts_ms":…,"event":"handoff","client":…,"from":…,"to":…,…}`).
- `TESSERA_GATEWAY_SESSIONLOG_ADDR` (défaut `127.0.0.1:9102`) — endpoint HTTP qui sert le
  fichier brut. Comme 9100, **non publié** hors du réseau Docker.

Récupérer le rapport après une session (depuis le VPS) :

    docker compose cp gateway:/data/session.jsonl ./rapport-session-$(date +%Y%m%d).jsonl

## Présence serveur (registre Platform API)

Le service `heartbeat` du compose (`tessera-directory heartbeat`) enregistre le serveur puis
bat toutes les 30 s vers `PLATFORM_URL` (défaut `https://platform.tesserasynth.net`). La clé
d'identité Ed25519 est créée au premier lancement dans le volume (`/data/server_identity.b64`)
— la sauvegarder : c'est elle qui prouve l'identité du serveur auprès du registre. Sans
heartbeat depuis 90 s, le serveur disparaît de la liste publique (`GET /v1/servers`).

## Pipeline de release (canaux dev/playtest/main)

Depuis l'introduction du pipeline unifié (`.github/workflows/release.yml`, voir
`docs/superpowers/specs/2026-07-08-server-release-channels-design.md`), le serveur est publié
par version/canal signée sur `The-Genium007/tessera-core`, synchronisée avec le modset client.

**Prérequis opérateur (une fois) :**

- Secret de dépôt `CORE_RELEASE_TOKEN` : token avec droits push + Releases sur
  `The-Genium007/tessera-core` (équivalent de `DISTRIBUTION_TOKEN` pour l'ancien pipeline modset).
- Le canal `main` publie une image `ghcr.io/the-genium007/tessera-server:main` (tag flottant,
  repoussé uniquement à la promotion `playtest→main`). **Reconfigurer Dokploy** pour tirer ce tag
  au lieu de `:latest` — Dokploy n'est pas piloté depuis ce dépôt, ce changement se fait dans son
  interface/état, hors CI.

## Rebase 0.2.0 (2026-07-20)

Le passage au format de version `-dev.N` / `-pts.N` (action `reset-to-new-format` de
`release.yml`) a semé le ledger sur `0.2.0-dev.1` **sans publier d'artefacts**, et toutes les
releases `client-v*` / `server-v*` ont été purgées dans la foulée. Les deux côtés étaient donc
« inchangés » depuis le SHA du reset et partaient en *hollow re-tag*, qui va chercher une release
de base désormais inexistante.

Deux pièges vérifiés au passage, à ne pas réapprendre :

- **`force: true` ne force pas un rebuild.** Il outrepasse le garde « aucun côté n'a changé, rien
  à publier », pas la décision rebuild-vs-re-tag. Un run `force` sur deux côtés inchangés part
  quand même en hollow re-tag, et meurt sur `manifeste client introuvable`.
- **Après un reset ou une purge de releases, il faut que les DEUX côtés changent réellement**
  (cf. CLAUDE.md). D'où ce paragraphe, pendant serveur de l'entrée équivalente dans
  `distribution/modset.packages.toml` — `tessera-core/server` est dans `SERVER_PATHS`, donc y
  toucher suffit à déclencher un vrai build.
