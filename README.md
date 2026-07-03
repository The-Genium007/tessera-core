# tessera-core

Moteur multijoueur open source pour **Cyberpunk 2077**, dans l'esprit de FiveM : le jeu
reste le client visuel (moddé au runtime), et un **serveur Rust autoritaire** héberge le
monde partagé. Modèle **self-host** : chaque opérateur fait tourner son propre serveur,
il n'y a pas d'infrastructure centrale.

**Licence : à déterminer.**

## Ce que fait le moteur

- Un serveur autoritaire suit les positions des joueurs et les diffuse aux clients
  connectés (tick 20 Hz), avec **sharding** du monde (un Gateway qui répartit les joueurs
  vers des Shards) et gestion d'aire d'intérêt (AoI).
- Un mod client (RED4ext + redscript) fait apparaître les autres joueurs dans le jeu
  sous forme de proxies interpolés.
- Le tout communique en **UDP fiable** via GameNetworkingSockets (Valve), avec des
  messages **FlatBuffers** définis dans un schéma partagé.

## Architecture en un coup d'œil

| Dossier | Rôle | Stack | Plateforme |
|---|---|---|---|
| `server/` | Serveur de jeu autoritaire : Gateway + Shards, tick 20 Hz, AoI, persistance joueurs | Rust (tokio + ECS), GameNetworkingSockets | macOS / Linux / Windows |
| `client-mod/` | Mod du jeu : proxies d'entités distantes, état monde, UI overlay. Forké du client [Cyberverse](https://github.com/TDUniverse/Cyberverse) | C++ (RED4ext) + redscript + Codeware | **Windows uniquement** |
| `protocol/` | Contrat réseau partagé, source de vérité des messages client↔serveur | Schémas FlatBuffers (`schema/*.fbs`) | — |
| `directory/` | Outil `tessera-directory` : dérive et signe (Ed25519) le `servers.json` public d'un opérateur depuis son manifeste serveur | Rust | macOS / Linux / Windows |
| `voip/` | Voix de proximité spatiale (les joueurs n'entendent que les voix proches en jeu) | Mumble/Murmur | macOS / Linux / Windows |

Décisions d'architecture détaillées dans [`docs/`](docs/) (ADR 0001 à 0005) : version de
jeu épinglée, réutilisation de Cyberverse, binding GNS, chaîne de modding Windows,
port du client vers 2.31.

## Builder

Le serveur est un workspace Rust standard, à la racine de ce dépôt :

```bash
cargo build
cargo test
```

Par défaut (sans feature), le serveur compile et les tests tournent **sans réseau réel** —
aucune dépendance native requise. C'est le mode recommandé pour développer et tester.

### Transport réseau réel (feature `gns`)

Le vrai transport UDP passe par GameNetworkingSockets, une bibliothèque C++ de Valve.
Sa compilation exige des prérequis natifs (cmake, **protobuf 3.21.x** — protobuf ≥ 4
casse le build —, openssl, flatc) :

```bash
cargo build --features gns
```

Détails complets (prérequis macOS, variables d'environnement, choix du binding Rust) :
[`docs/0003-gns-binding.md`](docs/0003-gns-binding.md). Alternative sans prérequis
locaux : `server/docker-compose.yml` reproduit la topologie Gateway + 2 Shards en Docker
(voir [`server/README.md`](server/README.md)).

### Schémas FlatBuffers

Les schémas vivent dans `protocol/schema/`. Génération du code
(`brew install flatbuffers` sur macOS) :

```bash
# Rust (serveur)
flatc --rust -o server/src/generated protocol/schema/<x>.fbs
# C++ (client-mod, Windows)
flatc --cpp -o client-mod/generated protocol/schema/<x>.fbs
```

## Plateformes

- **Serveur, protocole, directory, voip** : développement et exécution sur
  macOS / Linux / Windows.
- **Client-mod** : **Windows uniquement** — RED4ext et redscript n'existent que sur
  Windows, et le jeu lui-même est requis pour tester. Version de jeu cible : **v2.31**
  (voir [`docs/0001-pinned-game-version.md`](docs/0001-pinned-game-version.md)).

## Branches & canaux

Trois branches long-cours, chacune correspondant à un canal de déploiement (même logique
que les canaux `playtest`/`stable` du launcher) :

| Branche | Rôle |
| --- | --- |
| `dev` | Développement actif — code destiné au serveur de test utilisé pour valider les changements en jeu |
| `playtest` | Test public — build candidate en cours de validation par les joueurs |
| `main` | Stable — versions publiées |

`dev` est synchronisée automatiquement depuis le monorepo privé de développement
(Tessera). La promotion vers `playtest` puis `main` est un geste délibéré (pas
automatique) — typiquement une PR ou un merge dans ce dépôt une fois une étape validée.

À chaque push sur l'une de ces branches, la CI (`.github/workflows/ci.yml`) build et teste
le workspace, et (`.github/workflows/docker-image.yml`) publie une image Docker sur GHCR
taguée par canal : `ghcr.io/the-genium007/tessera-server:dev`, `:playtest`, `:stable`.

**Ce qui n'est pas automatisé** : le déploiement de cette image sur un serveur hébergé
(dev/playtest/stable). Il n'existe aujourd'hui qu'un seul serveur hébergé (VPS de
production) — aucune infrastructure séparée pour `dev`/`playtest` n'est provisionnée, donc
aucune CI ne peut y déployer automatiquement. Procédure manuelle en attendant : sur le
serveur cible, `docker pull ghcr.io/the-genium007/tessera-server:<canal>` puis relancer le
`docker-compose.yml` du canal concerné.

## Ce que ce dépôt ne contient pas

Aucun asset de CD Projekt Red n'est présent ni redistribué : le mod fonctionne
exclusivement en **modding runtime** sur une installation légitime du jeu.

## Contribuer

Voir [CONTRIBUTING.md](CONTRIBUTING.md) : conventions de commits, TDD, formatage,
et politique vis-à-vis des assets CDPR.
