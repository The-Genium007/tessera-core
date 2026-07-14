# Architecture Decision Records

Décisions d'architecture notables du moteur, dans l'ordre chronologique.

| ADR | Titre | Statut |
|---|---|---|
| [0001](0001-pinned-game-version.md) | Version de Cyberpunk 2077 à figer | accepté (v2.31) |
| [0002](0002-cyberverse-reuse.md) | Réutilisation de Cyberverse (fork client + réécriture serveur) | accepté |
| [0003](0003-gns-binding.md) | Binding Rust pour GameNetworkingSockets (GNS) | accepté |
| [0004](0004-windows-modding-toolchain.md) | Chaîne de modding Windows pour le baseline 2.31 | accepté |
| [0005](0005-cyberverse-port-vs-rebuild.md) | Porter le client Cyberverse vers 2.31 (vs reconstruction) | proposé |
| [0006](0006-distribution-and-signing.md) | Distribution des modsets & signature des manifestes | accepté |
| [0009](0009-world-time-server-authority.md) | Heure du monde (jour/nuit) — autorité serveur | proposé (partie client préparée, sync réseau différée) |

Les numéros 0007 et 0008 n'apparaissent pas ici : ce sont des décisions sur la
plateforme web (site, CMS, IdP) qui vivent dans `tessera-administration/`, hors du
périmètre de ce moteur open source.

Ces ADR sont mirroirés depuis le monorepo de développement (`docs/architecture/` à la
racine du dépôt privé) — en cas de contribution qui change une décision, voir
[CONTRIBUTING.md](../CONTRIBUTING.md#décisions-darchitecture).
