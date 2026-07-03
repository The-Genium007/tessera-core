# ADR 0002 : Réutilisation de Cyberverse (fork client + réécriture serveur)

- **Statut :** accepté (ré-audité 2026-07-02, voir addendum en fin de document)
- **Date :** 2026-06-26

## Contexte

Ce projet vise à construire un système multijoueur type FiveM pour Cyberpunk 2077. Deux
grandes options existent : partir de zéro, ou réutiliser le travail existant de
[Cyberverse](https://github.com/TDUniverse/Cyberverse), un framework multiplayer
open source pour Cyberpunk 2077 déjà en développement.

La décision de principe du projet est de forker le client Cyberverse et d'écrire un
nouveau serveur en Rust. Cette ADR documente la réalité de la structure de Cyberverse et
les implications concrètes de cette décision.

> **Note :** l'analyse a été menée sur un clone local du dépôt Cyberverse.
> Dernier commit au moment de l'analyse : **2026-05-02**.

## Structure réelle de Cyberverse

Cyberverse est organisé en **4 projets** + un répertoire de protocole partagé :

```
Cyberverse/
├── client/
│   ├── red4ext/          ← C++ RED4ext plugin (networking, RTTI, bridges redscript↔jeu)
│   │   ├── src/
│   │   │   ├── NetworkGameSystem.cpp/h  ← cœur du réseau client
│   │   │   ├── PlayerActionTracker.cpp/h
│   │   │   └── PlayerSync/InterpolationData.h
│   │   ├── CMakeLists.txt
│   │   └── vcpkg.json    ← dépendances : gamenetworkingsockets
│   └── RedscriptModule/
│       └── src/
│           ├── Cyberverse.reds
│           └── Network/
│               ├── NetworkGameSystem.reds   ← logique haut niveau (UI, events jeu)
│               └── PlayerActionTracker.reds
├── server/
│   ├── Native/           ← C++ DLL serveur (networking bas niveau, sérialisation)
│   │   ├── src/GameServer.cpp/h
│   │   └── vcpkg.json    ← dépendances : gamenetworkingsockets
│   └── Managed/          ← C# .NET serveur (logique haut niveau, handlers de paquets)
│       ├── Program.cs
│       ├── GameServer.cs
│       ├── NativeLayer/Protocol/  ← types de paquets (clientbound + serverbound)
│       ├── PacketHandling/        ← AuthPacketHandler, PlayerPacketHandler
│       └── Services/              ← EntityService, EntityTracker, PlayerService
└── shared/
    └── protocol/         ← headers C++ partagés (MessageFrame, paquets)
        ├── MessageFrame.h
        ├── clientbound/  ← AuthPacketsClientBound.h, WorldPacketsClientBound.h
        └── serverbound/  ← AuthPacketsServerBound.h, WorldPacketsServerBound.h
```

### Langages et stacks

| Composant | Langage | Rôle |
|---|---|---|
| `client/red4ext` | C++ (C++20) | Plugin RED4ext : réseau client, interpolation, bridges RTTI |
| `client/RedscriptModule` | redscript | Logique jeu : écoute d'events, spawn proxies, UI overlay |
| `server/Native` | C++ | DLL serveur : transport réseau bas niveau, sérialisation |
| `server/Managed` | C# (.NET) | Logique serveur : auth, entity sync, gestion joueurs |
| `shared/protocol` | C++ headers | Contrat réseau : MessageFrame + tous les types de paquets |

### Transport réseau : Valve GameNetworkingSockets

**Cyberverse n'utilise PAS cp2077-red-socket.** Il embarque son propre transport réseau
via **Valve GameNetworkingSockets** (bibliothèque open source Steam, indépendante de
Steam lui-même) :

- `vcpkg.json` client et serveur : dépendance `gamenetworkingsockets`
- `NetworkGameSystem.h` : `#include <steam/isteamnetworkingsockets.h>`, membres
  `HSteamNetConnection`, `ISteamNetworkingSockets*`
- `NetworkGameSystem.cpp` : `GameNetworkingSockets_Init()`, `SteamNetworkingSockets()`,
  `ConnectByIPAddress()`, `SendMessageToConnection()` avec flag `k_nSteamNetworkingSend_Reliable`
- `server/Native/GameServer.h` : `HSteamListenSocket`, `ISteamNetworkingSockets*`, `ListenOn()`

GameNetworkingSockets fournit une couche de transport fiable sur UDP (similaire à QUIC),
pas du TCP brut. **cp2077-red-socket est donc hors sujet pour ce projet.**

### Protocole de sérialisation

Cyberverse utilise **zpp_bits** (bibliothèque C++ header-only de sérialisation binaire
rapide, licence MIT) pour sérialiser/désérialiser les paquets côté client ET côté serveur
Native. La structure de paquet est une `MessageFrame` (channel_id u8, message_type u16,
reserved u8) suivie du payload sérialisé. Ce n'est pas FlatBuffers.

### État de maintenance

- Dernier commit au moment de l'analyse initiale : **2026-05-02** (bump de dépendances
  zpp_bits et GameNetworkingSockets).
- Le projet semblait alors **actif mais au ralenti** : uniquement des bumps de dépendances
  Dependabot depuis début 2026. (Voir addendum : l'amont a repris depuis.)
- Version de jeu cible documentée : **v2.1** (README).
- Licence : **MIT** (vérifiée le 2026-07-02, voir addendum).

### Dépendances client requises (documentées dans le README Cyberverse)

- RED4ext
- redscript
- Codeware

## Décision

**Forker `client/red4ext` et `client/RedscriptModule` de Cyberverse** comme base du
répertoire `client-mod/` de ce projet, et **réécrire entièrement le serveur en Rust**.

### Ce qu'on reprend (client)

1. **`client/red4ext` (C++)** — Le plugin RED4ext avec la couche réseau
   GameNetworkingSockets, le système d'interpolation, le tracking des actions joueur.
   Base pour `client-mod/`.
2. **`client/RedscriptModule` (redscript)** — La logique haut niveau du côté jeu (events,
   spawn, UI). À étendre pour le RP.
3. **`shared/protocol/` (headers C++)** — Les types de paquets existants comme référence
   de conception ; ils sont remplacés par le schéma propre à ce projet.

### Ce qu'on réécrit (serveur)

Le serveur **`server/Native` (C++) + `server/Managed` (C#)** de Cyberverse est
**entièrement remplacé** par un nouveau serveur Rust dans `server/`.

Raisons :
- Le projet exige un serveur Rust autoritaire (tokio + ECS).
- La stack C# + C++ DLL du serveur Cyberverse est lourde et non portable macOS/Linux
  sans effort.
- La logique serveur visée (comptes, RP, whitelist, monitoring) dépasse le scope de
  Cyberverse.

### Protocole : nouveau schéma FlatBuffers, pas celui de Cyberverse

Le protocole `shared/protocol` de Cyberverse (headers C++ + zpp_bits) **n'est pas adopté
tel quel**. Ce projet définit son propre contrat réseau en **FlatBuffers** dans
`protocol/schema/`.

Raisons :
- zpp_bits est C++-only ; le serveur est en Rust.
- FlatBuffers génère du code pour C++ (client-mod) ET Rust (serveur) depuis une source
  unique.
- Le schéma Cyberverse sert de **référence de conception** pour les types de paquets
  (MessageFrame, auth, spawn, position, teleport, destroy, actions joueur).

### Transport réseau

Le client Cyberverse utilise GameNetworkingSockets ; ce transport est **conservé dans le
fork client**. Un point ouvert initial (le serveur devait-il parler TCP dans un premier
temps ?) a été résolu par la pratique : le serveur Rust a adopté GNS bout-en-bout, les
deux côtés parlent le même transport (voir addendum et ADR 0003).

## Build et exécution

La build de `client/red4ext` exige **Windows** (MSVC/CMake, vcpkg, Visual Studio 2022).
Le CI de Cyberverse lui-même utilise `runs-on: windows-latest` pour le client.

Le serveur Cyberverse (`server/Managed`) est C#/.NET et se build sur Linux, mais n'est
pas utilisé dans ce projet (remplacé par Rust).

## Conséquences

### Positives
- Le client Cyberverse fournit une base C++ validée pour le networking in-game (RED4ext,
  interpolation, RTTI bridges vers redscript).
- Les types de paquets existants (spawn, position, teleport, auth) accélèrent la
  conception du schéma FlatBuffers.
- Le serveur Rust est indépendant et développable sur macOS/Linux dès le premier jour.

### Négatives / risques
- Cyberverse cible v2.1 alors que le projet cible v2.31 (voir ADR 0001) : un port du
  client est nécessaire (voir ADR 0005).
- Le fork client introduit une dette vis-à-vis du projet amont (divergence croissante).
- Build Windows-only pour le client-mod : bloque les contributeurs sans Windows sur
  cette partie.

## Alternatives considérées

- **Partir de zéro (pas de fork Cyberverse) :** écarté — implique de réécrire la couche
  RED4ext/redscript + networking client from scratch, travail considérable.
- **Adopter le protocole zpp_bits de Cyberverse côté serveur Rust :** écarté — pas de
  binding Rust de qualité, va à l'encontre du choix FlatBuffers.
- **Utiliser cp2077-red-socket comme transport client :** écarté — Cyberverse n'en dépend
  pas et utilise GameNetworkingSockets ; remplacer le transport serait une refonte majeure.
- **Forker aussi le serveur C# de Cyberverse :** écarté — le projet exige un serveur Rust
  autoritaire, la stack C# n'est pas portable et n'apporte pas la logique RP nécessaire.

## Sources

- README Cyberverse : https://github.com/TDUniverse/Cyberverse
- Code analysé localement (clone du 2026-06-26) :
  - `client/red4ext/src/NetworkGameSystem.h` — transport GameNetworkingSockets
  - `client/red4ext/CMakeLists.txt` — dépendances (GNS, zpp_bits, red-lib)
  - `client/red4ext/vcpkg.json` — dépendance gamenetworkingsockets
  - `shared/protocol/MessageFrame.h` — structure du frame réseau
  - `server/Native/src/GameServer.h` — côté serveur GNS
  - `server/Managed/Program.cs` — serveur C# (remplacé par Rust)
  - `.github/workflows/build.yml` — CI Windows pour le client
- GameNetworkingSockets (Valve) : https://github.com/ValveSoftware/GameNetworkingSockets
- zpp_bits : https://github.com/eyalz800/zpp_bits

---

## Addendum 2026-07-02 — Ré-audit de fiabilité (ambition 1000+ joueurs)

Suite à l'élargissement de l'ambition du projet (supporter un grand nombre de joueurs
par serveur via le sharding), la décision « garder le fork ou reconstruire » a été
ré-auditée franchement. **Verdict : on garde le fork, confiance élevée (~90 %).**
Ce ré-audit résout aussi les deux points ouverts de cette ADR (licence, transport).

- **Licence — résolue.** Le fichier `LICENSE` amont a été lu : **MIT** (Copyright
  MeFisto94, TDUniverse). Usage commercial/modification/redistribution permis, seule
  obligation : conserver la notice de copyright dans les distributions (y compris
  `client/red4ext/LICENSE.red4ext.md`). Aucun blocage pour un écosystème self-host public.
- **Transport GNS — résolu par la pratique.** Le serveur Rust a adopté GNS bout-en-bout,
  donc le point ouvert « TCP côté serveur vs GNS côté client » ne se pose plus : les deux
  côtés parlent GNS.
- **Amont réactivé.** Contrairement à l'état « au ralenti » constaté le 26/06,
  `TDUniverse/Cyberverse` a repoussé le 2026-06-30 (bump SDK/red-lib + nettoyage).
  L'amont a convergé **indépendamment** vers la même migration SDK v1/red-lib HEAD que
  notre fork — validation croisée des choix techniques. Réserve : bus factor ≈ 1, pas de
  release taguée — un prototype de développeur, pas un produit, mais on ne dépend pas de
  sa roadmap (le code est déjà forké).
- **Volume et coût de reconstruction.** Le code repris ≈ **1 700 lignes**
  (`client/red4ext` + `client/RedscriptModule` + `shared/protocol`). Reconstruire
  aboutirait à la même architecture (plugin RED4ext + GNS + pont redscript) sans le
  bénéfice de la validation in-game déjà obtenue — plusieurs semaines pour revenir au
  point actuel, risque égal ou supérieur.
- **Preuve déjà apportée.** Le port vers 2.31 compile en CI et **charge en jeu sans
  crash** (RTTI enregistré, ~19 appels tiennent sur 2.31) — le risque de
  couplage-version, principal risque identifié par cette ADR, est levé. Non couvert :
  l'échange réseau réel avec 2+ joueurs sur 2.31 (couture FlatBuffers committée, pas
  encore prouvée en jeu).
- **Cadrage du risque à 1000 joueurs.** L'architecture exclut « 1000 joueurs dans une
  même scène » — 32-64 par scène, le nombre global passe par le sharding serveur + AoI.
  Le client forké ne verra jamais que quelques dizaines de proxies : **il n'est pas sur
  le chemin critique du scaling**.

### Dette technique identifiée dans le fork (à corriger sur place, pas une raison de reconstruire)

- Fuite mémoire : `new char[]` jamais libéré dans `ConnectToServer`
  (`NetworkGameSystem.cpp`).
- Bug probable : le handler `eDestroyEntity` fait `erase()` sur
  `m_networkedEntitiesLookup` avec l'ID d'entité **locale** au lieu de l'ID **réseau** —
  l'entrée n'est jamais purgée.
- Tous les envois en `k_nSteamNetworkingSend_Reliable` (pas d'unreliable pour les
  positions) — head-of-line blocking possible sous charge.
- `InterpolatePuppets` fait plusieurs appels script (`Red::CallVirtual`) par proxy par
  frame — à optimiser/batcher au-delà de ~30-50 proxies simultanés (limite moteur, pas
  limite serveur).
- Interpolation mono-slot, cadence 10 Hz codée en dur, zéro test dans le dépôt amont.

### Signaux de re-bascule (si un jour reconsidérer ce fork)

Un test 2+ joueurs sur le port 2.31 qui révèle une rupture **structurelle** (pas un bug)
de la sync d'entités RTTI, ou un échec **architectural** (pas un bug de framing) de la
couture FlatBuffers, seraient les seuls scénarios réalistes de réouverture de ce dossier.
